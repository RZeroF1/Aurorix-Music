//! Deterministic orchestration for a local library scan.
//!
//! The types in this module describe scan work and its result without reading
//! the filesystem or depending on persistence, providers, or a platform API.
//! A platform scanner supplies directory and asset observations; this module
//! orders that input, creates bounded batches, classifies changes, and applies
//! the shared [`ScanState`] transition policy.

use std::{collections::BTreeMap, error::Error, fmt};

use crate::scan_state::{InvalidTransition, ScanObservation, ScanState};

/// A directory root requested for a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectorySpec {
    /// Opaque platform path. No filesystem operation is performed here.
    pub path: String,
    /// Whether the platform scanner should include descendants.
    pub recursive: bool,
}

impl DirectorySpec {
    /// Creates a directory specification.
    ///
    /// Empty paths are rejected so sorting and deduplication do not conceal a
    /// malformed scanner input.
    ///
    /// # Errors
    ///
    /// Returns [`ScanInputError::EmptyPath`] when `path` is empty.
    pub fn new(path: impl Into<String>, recursive: bool) -> Result<Self, ScanInputError> {
        let path = path.into();
        if path.is_empty() {
            return Err(ScanInputError::EmptyPath);
        }
        Ok(Self { path, recursive })
    }
}

/// A bounded, deterministic directory scan plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPlan {
    directories: Vec<DirectorySpec>,
    batch_size: usize,
}

impl DirectoryPlan {
    /// Builds a plan, sorting directories by path and then recursion mode.
    /// Duplicate directory requests are collapsed deterministically.
    ///
    /// # Errors
    ///
    /// Returns [`ScanInputError::ZeroBatchSize`] when `batch_size` is zero.
    pub fn new(
        mut directories: Vec<DirectorySpec>,
        batch_size: usize,
    ) -> Result<Self, ScanInputError> {
        if batch_size == 0 {
            return Err(ScanInputError::ZeroBatchSize);
        }

        directories.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.recursive.cmp(&right.recursive))
        });
        directories.dedup();

        Ok(Self {
            directories,
            batch_size,
        })
    }

    /// Returns directories in their canonical order.
    #[must_use]
    pub fn directories(&self) -> &[DirectorySpec] {
        &self.directories
    }

    /// Returns the configured maximum number of directories per batch.
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Splits the plan into stable, bounded batches.
    #[must_use]
    pub fn batches(&self) -> Vec<DirectoryBatch> {
        self.directories
            .chunks(self.batch_size)
            .enumerate()
            .map(|(index, directories)| DirectoryBatch {
                index,
                directories: directories.to_vec(),
            })
            .collect()
    }
}

/// One bounded portion of a [`DirectoryPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryBatch {
    /// Zero-based position in the plan.
    pub index: usize,
    /// Directory roots assigned to this batch.
    pub directories: Vec<DirectorySpec>,
}

/// An asset known from a previous local scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedAsset {
    /// Opaque locator supplied by a platform scanner.
    pub locator: String,
    /// Scanner-defined content or metadata fingerprint.
    pub fingerprint: Option<String>,
    /// Persisted state before this scan run.
    pub state: ScanState,
}

impl TrackedAsset {
    /// Creates a tracked asset record.
    ///
    /// # Errors
    ///
    /// Returns [`ScanInputError::EmptyLocator`] when `locator` is empty.
    pub fn new(
        locator: impl Into<String>,
        fingerprint: Option<String>,
        state: ScanState,
    ) -> Result<Self, ScanInputError> {
        let locator = locator.into();
        if locator.is_empty() {
            return Err(ScanInputError::EmptyLocator);
        }
        Ok(Self {
            locator,
            fingerprint,
            state,
        })
    }
}

/// An asset observed during the current scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAsset {
    /// Opaque locator supplied by a platform scanner.
    pub locator: String,
    /// Scanner-defined content or metadata fingerprint.
    pub fingerprint: Option<String>,
}

impl ObservedAsset {
    /// Creates a current scan observation.
    ///
    /// # Errors
    ///
    /// Returns [`ScanInputError::EmptyLocator`] when `locator` is empty.
    pub fn new(
        locator: impl Into<String>,
        fingerprint: Option<String>,
    ) -> Result<Self, ScanInputError> {
        let locator = locator.into();
        if locator.is_empty() {
            return Err(ScanInputError::EmptyLocator);
        }
        Ok(Self {
            locator,
            fingerprint,
        })
    }
}

/// Input collected by a platform scanner for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRunInput {
    /// Directory roots to scan.
    pub directories: Vec<DirectorySpec>,
    /// Assets from the previous completed run.
    pub tracked_assets: Vec<TrackedAsset>,
    /// Assets observed by the current run.
    pub observed_assets: Vec<ObservedAsset>,
    /// Maximum number of directory roots assigned to one batch.
    pub batch_size: usize,
}

/// The resulting deterministic work description and change summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRun {
    directory_plan: DirectoryPlan,
    events: Vec<ScanEvent>,
    summary: ScanSummary,
}

impl ScanRun {
    /// Builds a scan run from platform-provided observations.
    ///
    /// Inputs are keyed by locator and sorted before comparison. Duplicate
    /// asset locators are rejected because silently choosing one observation
    /// would make a scan result dependent on enumeration order.
    ///
    /// # Errors
    ///
    /// Returns an input error for an empty path or locator, duplicate locator,
    /// zero batch size, or an invalid [`ScanState`] transition.
    pub fn build(input: ScanRunInput) -> Result<Self, ScanInputError> {
        let directory_plan = DirectoryPlan::new(input.directories, input.batch_size)?;
        let tracked = unique_tracked(input.tracked_assets)?;
        let observed = unique_observed(input.observed_assets)?;

        let mut events = Vec::with_capacity(tracked.len().max(observed.len()));
        for (locator, tracked_asset) in &tracked {
            if let Some(current) = observed.get(locator) {
                let kind = if tracked_asset.fingerprint == current.fingerprint {
                    ChangeKind::Unchanged
                } else {
                    ChangeKind::Changed
                };
                let next_state = next_state(kind, Some(tracked_asset.state))?;
                events.push(ScanEvent {
                    locator: locator.clone(),
                    kind,
                    from: Some(tracked_asset.state),
                    to: next_state,
                });
            } else {
                let kind = ChangeKind::Missing;
                let next_state = next_state(kind, Some(tracked_asset.state))?;
                events.push(ScanEvent {
                    locator: locator.clone(),
                    kind,
                    from: Some(tracked_asset.state),
                    to: next_state,
                });
            }
        }
        for (locator, _) in observed {
            if !tracked.contains_key(&locator) {
                let kind = ChangeKind::Added;
                events.push(ScanEvent {
                    locator,
                    kind,
                    from: None,
                    to: next_state(kind, None)?,
                });
            }
        }
        events.sort_by(|left, right| left.locator.cmp(&right.locator));
        let summary = ScanSummary::from_events(&events);

        Ok(Self {
            directory_plan,
            events,
            summary,
        })
    }

    /// Returns the canonical directory plan.
    #[must_use]
    pub const fn directory_plan(&self) -> &DirectoryPlan {
        &self.directory_plan
    }

    /// Returns change events ordered by locator.
    #[must_use]
    pub fn events(&self) -> &[ScanEvent] {
        &self.events
    }

    /// Returns aggregate change counts.
    #[must_use]
    pub const fn summary(&self) -> &ScanSummary {
        &self.summary
    }
}

/// Classification of one locator between two scan runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// Present only in the current scan.
    Added,
    /// Present in both scans, but its fingerprint changed.
    Changed,
    /// Present in both scans with the same fingerprint.
    Unchanged,
    /// Present in the previous scan but absent from the current scan.
    Missing,
}

/// A state-aware change event for one asset locator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEvent {
    /// Opaque asset locator.
    pub locator: String,
    /// Difference between the previous and current observations.
    pub kind: ChangeKind,
    /// State before this run, if the asset was previously tracked.
    pub from: Option<ScanState>,
    /// State after applying the scan observation policy.
    pub to: ScanState,
}

/// Aggregate counts for one scan run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub added: usize,
    pub changed: usize,
    pub unchanged: usize,
    pub missing: usize,
}

impl ScanSummary {
    fn from_events(events: &[ScanEvent]) -> Self {
        let mut summary = Self::default();
        for event in events {
            match event.kind {
                ChangeKind::Added => summary.added += 1,
                ChangeKind::Changed => summary.changed += 1,
                ChangeKind::Unchanged => summary.unchanged += 1,
                ChangeKind::Missing => summary.missing += 1,
            }
        }
        summary
    }
}

/// Invalid input supplied to the orchestration boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanInputError {
    EmptyPath,
    EmptyLocator,
    ZeroBatchSize,
    DuplicateTrackedLocator,
    DuplicateObservedLocator,
    InvalidTransition(InvalidTransition),
}

impl fmt::Display for ScanInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("scan directory path must not be empty"),
            Self::EmptyLocator => formatter.write_str("scan asset locator must not be empty"),
            Self::ZeroBatchSize => formatter.write_str("scan batch size must be greater than zero"),
            Self::DuplicateTrackedLocator => {
                formatter.write_str("tracked asset locator is duplicated")
            }
            Self::DuplicateObservedLocator => {
                formatter.write_str("observed asset locator is duplicated")
            }
            Self::InvalidTransition(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScanInputError {}

impl From<InvalidTransition> for ScanInputError {
    fn from(error: InvalidTransition) -> Self {
        Self::InvalidTransition(error)
    }
}

fn unique_tracked(
    assets: Vec<TrackedAsset>,
) -> Result<BTreeMap<String, TrackedAsset>, ScanInputError> {
    let mut result = BTreeMap::new();
    for asset in assets {
        if result.insert(asset.locator.clone(), asset).is_some() {
            return Err(ScanInputError::DuplicateTrackedLocator);
        }
    }
    Ok(result)
}

fn unique_observed(
    assets: Vec<ObservedAsset>,
) -> Result<BTreeMap<String, ObservedAsset>, ScanInputError> {
    let mut result = BTreeMap::new();
    for asset in assets {
        if result.insert(asset.locator.clone(), asset).is_some() {
            return Err(ScanInputError::DuplicateObservedLocator);
        }
    }
    Ok(result)
}

fn next_state(kind: ChangeKind, previous: Option<ScanState>) -> Result<ScanState, ScanInputError> {
    match kind {
        ChangeKind::Added => Ok(ScanState::Discovered.observe(ScanObservation::ProbeStarted)?),
        ChangeKind::Changed => {
            let previous = previous.expect("changed assets always have a previous state");
            match previous {
                ScanState::Missing | ScanState::RelinkCandidate => Ok(ScanState::RelinkCandidate),
                _ => Ok(previous
                    .observe(ScanObservation::ContentChanged)?
                    .observe(ScanObservation::ProbeStarted)?),
            }
        }
        ChangeKind::Unchanged => {
            Ok(previous.expect("unchanged assets always have a previous state"))
        }
        ChangeKind::Missing => Ok(previous
            .expect("missing assets always have a previous state")
            .observe(ScanObservation::Missing)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(path: &str, recursive: bool) -> DirectorySpec {
        DirectorySpec::new(path, recursive).unwrap()
    }

    fn tracked(locator: &str, fingerprint: &str) -> TrackedAsset {
        TrackedAsset::new(locator, Some(fingerprint.to_owned()), ScanState::Active).unwrap()
    }

    fn observed(locator: &str, fingerprint: &str) -> ObservedAsset {
        ObservedAsset::new(locator, Some(fingerprint.to_owned())).unwrap()
    }

    #[test]
    fn directory_order_and_batches_are_deterministic() {
        let plan = DirectoryPlan::new(
            vec![
                directory("z", false),
                directory("a", true),
                directory("a", false),
                directory("z", false),
            ],
            2,
        )
        .unwrap();

        assert_eq!(
            plan.directories(),
            &[
                directory("a", false),
                directory("a", true),
                directory("z", false)
            ]
        );
        assert_eq!(
            plan.batches(),
            vec![
                DirectoryBatch {
                    index: 0,
                    directories: vec![directory("a", false), directory("a", true)],
                },
                DirectoryBatch {
                    index: 1,
                    directories: vec![directory("z", false)],
                },
            ]
        );
    }

    #[test]
    fn changes_are_classified_and_sorted() {
        let run = ScanRun::build(ScanRunInput {
            directories: Vec::new(),
            tracked_assets: vec![
                tracked("changed", "old"),
                tracked("missing", "gone"),
                tracked("same", "v1"),
            ],
            observed_assets: vec![
                observed("added", "new"),
                observed("changed", "new"),
                observed("same", "v1"),
            ],
            batch_size: 3,
        })
        .unwrap();

        assert_eq!(
            run.events()
                .iter()
                .map(|event| (&event.locator, event.kind, event.to))
                .collect::<Vec<_>>(),
            vec![
                (&"added".to_owned(), ChangeKind::Added, ScanState::Probing),
                (
                    &"changed".to_owned(),
                    ChangeKind::Changed,
                    ScanState::Probing
                ),
                (
                    &"missing".to_owned(),
                    ChangeKind::Missing,
                    ScanState::Missing
                ),
                (&"same".to_owned(), ChangeKind::Unchanged, ScanState::Active),
            ]
        );
        assert_eq!(
            *run.summary(),
            ScanSummary {
                added: 1,
                changed: 1,
                unchanged: 1,
                missing: 1,
            }
        );
    }

    #[test]
    fn empty_directory_plan_has_no_batches() {
        let plan = DirectoryPlan::new(Vec::new(), 4).unwrap();
        assert!(plan.batches().is_empty());
    }
}

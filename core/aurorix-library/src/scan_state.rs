//! Pure state machine for local filesystem scan observations.
//!
//! This module does not touch the filesystem or database. It only validates
//! transitions so platform scanners can share one deterministic state policy.

use std::{error::Error, fmt};

/// Persisted state of one discovered local asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanState {
    /// A scanner found a locator but has not probed it.
    Discovered,
    /// Metadata probing is in progress.
    Probing,
    /// The asset is readable and usable.
    Active,
    /// A previously active asset changed and needs re-probing.
    Changed,
    /// The locator was not found during a scan.
    Missing,
    /// The locator could not be read due to permissions.
    PermissionDenied,
    /// The file type or codec is unsupported.
    Unsupported,
    /// Probing failed for an unspecified reason.
    Error,
    /// A missing asset has a possible replacement candidate.
    RelinkCandidate,
    /// The asset was explicitly retired after relink resolution.
    Tombstoned,
}

/// Why a scan observation was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanObservation {
    /// A new locator was discovered.
    Discovered,
    /// Probing started.
    ProbeStarted,
    /// Probing completed successfully.
    ProbeSucceeded,
    /// Metadata or bytes changed since the last active observation.
    ContentChanged,
    /// The locator was absent.
    Missing,
    /// Access was denied.
    PermissionDenied,
    /// The asset format is unsupported.
    Unsupported,
    /// Probing failed.
    Failed,
    /// A possible replacement was found for a missing asset.
    CandidateFound,
    /// Relink resolution retired the old asset.
    Tombstone,
}

/// Invalid local scan state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    /// State before the observation.
    pub from: ScanState,
    /// Observation that was rejected.
    pub observation: ScanObservation,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid scan transition from {:?} via {:?}",
            self.from, self.observation
        )
    }
}

impl Error for InvalidTransition {}

impl ScanState {
    /// Applies one observation to the current state.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTransition`] when the observation is not permitted by
    /// the local scan state contract.
    pub fn observe(self, observation: ScanObservation) -> Result<Self, InvalidTransition> {
        let next = match (self, observation) {
            (Self::Discovered | Self::Changed, ScanObservation::ProbeStarted) => Self::Probing,
            (Self::Probing | Self::RelinkCandidate, ScanObservation::ProbeSucceeded) => {
                Self::Active
            }
            (Self::Active, ScanObservation::ContentChanged) => Self::Changed,
            (Self::Active, ScanObservation::Missing) => Self::Missing,
            (Self::Active, ScanObservation::PermissionDenied) => Self::PermissionDenied,
            (Self::Active, ScanObservation::Unsupported) => Self::Unsupported,
            (Self::Active | Self::Changed | Self::Probing, ScanObservation::Failed) => Self::Error,
            (Self::Missing, ScanObservation::CandidateFound) => Self::RelinkCandidate,
            (Self::RelinkCandidate, ScanObservation::Tombstone) => Self::Tombstoned,
            _ => {
                return Err(InvalidTransition {
                    from: self,
                    observation,
                });
            }
        };
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanObservation, ScanState};

    #[test]
    fn normal_asset_lifecycle_is_deterministic() {
        let state = ScanState::Discovered
            .observe(ScanObservation::ProbeStarted)
            .unwrap()
            .observe(ScanObservation::ProbeSucceeded)
            .unwrap()
            .observe(ScanObservation::ContentChanged)
            .unwrap()
            .observe(ScanObservation::ProbeStarted)
            .unwrap()
            .observe(ScanObservation::ProbeSucceeded)
            .unwrap();
        assert_eq!(state, ScanState::Active);
    }

    #[test]
    fn missing_asset_can_relink_or_be_tombstoned() {
        assert_eq!(
            ScanState::Active
                .observe(ScanObservation::Missing)
                .unwrap()
                .observe(ScanObservation::CandidateFound)
                .unwrap()
                .observe(ScanObservation::Tombstone)
                .unwrap(),
            ScanState::Tombstoned
        );
    }

    #[test]
    fn illegal_resurrection_is_rejected() {
        let error = ScanState::Tombstoned
            .observe(ScanObservation::ProbeSucceeded)
            .unwrap_err();
        assert_eq!(error.from, ScanState::Tombstoned);
    }
}

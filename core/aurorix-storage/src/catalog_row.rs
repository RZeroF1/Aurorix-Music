//! Storage-neutral rows for the device-local music catalog.
//!
//! These values are the boundary between the validated music graph and a
//! persistence adapter. They contain no SQL types, filesystem locators, scan
//! state, Provider identity, or replicated identity. Conversion preserves the
//! model's values exactly; in particular, a missing local asset remains an
//! asset row with [`LocalAssetAvailability::Missing`] rather than becoming a
//! delete operation.

use aurorix_model::{
    ids::LocalCatalogEntityId,
    music::{
        AssetSegment, DurationMs, LocalAsset, LocalAssetAvailability, Recording, Release,
        ReleaseMedium, ReleaseTrack, Work,
    },
};

/// A persistence row for a local musical work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkRow {
    id: LocalCatalogEntityId,
    title: String,
}

impl From<&Work> for WorkRow {
    fn from(work: &Work) -> Self {
        Self {
            id: work.id(),
            title: work.title().to_owned(),
        }
    }
}

impl WorkRow {
    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the original display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// A persistence row for a local recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingRow {
    id: LocalCatalogEntityId,
    title: String,
    duration_ms: Option<DurationMs>,
    work_ids: Vec<LocalCatalogEntityId>,
}

impl From<&Recording> for RecordingRow {
    fn from(recording: &Recording) -> Self {
        Self {
            id: recording.id(),
            title: recording.title().to_owned(),
            duration_ms: recording.duration_ms(),
            work_ids: recording.work_ids().to_vec(),
        }
    }
}

impl RecordingRow {
    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the original display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the optional known duration.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }

    /// Returns the linked local work identifiers in model order.
    #[must_use]
    pub fn work_ids(&self) -> &[LocalCatalogEntityId] {
        &self.work_ids
    }
}

/// A persistence row for a local release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseRow {
    id: LocalCatalogEntityId,
    title: String,
}

impl From<&Release> for ReleaseRow {
    fn from(release: &Release) -> Self {
        Self {
            id: release.id(),
            title: release.title().to_owned(),
        }
    }
}

impl ReleaseRow {
    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the original display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// A persistence row for one numbered medium belonging to a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReleaseMediumRow {
    id: LocalCatalogEntityId,
    release_id: LocalCatalogEntityId,
    number: u32,
}

impl ReleaseMediumRow {
    /// Creates a medium row while retaining its containing release identifier.
    #[must_use]
    pub const fn from_model(release_id: LocalCatalogEntityId, medium: &ReleaseMedium) -> Self {
        Self {
            id: medium.id(),
            release_id,
            number: medium.number(),
        }
    }

    /// Returns the medium identifier.
    #[must_use]
    pub const fn id(self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the containing release identifier.
    #[must_use]
    pub const fn release_id(self) -> LocalCatalogEntityId {
        self.release_id
    }

    /// Returns the one-based medium number.
    #[must_use]
    pub const fn number(self) -> u32 {
        self.number
    }
}

/// A persistence row for one track position belonging to a release medium.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTrackRow {
    id: LocalCatalogEntityId,
    medium_id: LocalCatalogEntityId,
    medium_number: u32,
    track_number: u32,
    track_order: u32,
    title: String,
    primary_recording_id: Option<LocalCatalogEntityId>,
    duration_ms: Option<DurationMs>,
}

impl ReleaseTrackRow {
    /// Creates a track row while retaining its containing medium identifier.
    #[must_use]
    pub fn from_model(medium_id: LocalCatalogEntityId, track: &ReleaseTrack) -> Self {
        Self {
            id: track.id(),
            medium_id,
            medium_number: track.medium_number(),
            track_number: track.track_number(),
            track_order: track.track_order(),
            title: track.title().to_owned(),
            primary_recording_id: track.primary_recording_id(),
            duration_ms: track.duration_ms(),
        }
    }

    /// Returns the release-track identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the containing medium identifier.
    #[must_use]
    pub const fn medium_id(&self) -> LocalCatalogEntityId {
        self.medium_id
    }

    /// Returns the model's medium number.
    #[must_use]
    pub const fn medium_number(&self) -> u32 {
        self.medium_number
    }

    /// Returns the release-assigned track number.
    #[must_use]
    pub const fn track_number(&self) -> u32 {
        self.track_number
    }

    /// Returns the stable order within the medium.
    #[must_use]
    pub const fn track_order(&self) -> u32 {
        self.track_order
    }

    /// Returns the original display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the linked local recording, when known.
    #[must_use]
    pub const fn primary_recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.primary_recording_id
    }

    /// Returns the optional known duration.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }
}

/// A persistence row for device-local media bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAssetRow {
    id: LocalCatalogEntityId,
    primary_recording_id: Option<LocalCatalogEntityId>,
    duration_ms: Option<DurationMs>,
    availability: LocalAssetAvailability,
}

impl From<&LocalAsset> for LocalAssetRow {
    fn from(asset: &LocalAsset) -> Self {
        Self {
            id: asset.id(),
            primary_recording_id: asset.primary_recording_id(),
            duration_ms: asset.duration_ms(),
            availability: asset.availability(),
        }
    }
}

impl LocalAssetRow {
    /// Returns the local asset identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the linked local recording, when known.
    #[must_use]
    pub const fn primary_recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.primary_recording_id
    }

    /// Returns the optional known duration.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }

    /// Returns the observed availability, including [`LocalAssetAvailability::Missing`].
    #[must_use]
    pub const fn availability(&self) -> LocalAssetAvailability {
        self.availability
    }
}

/// A persistence row for one logical segment within a local asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSegmentRow {
    asset_id: LocalCatalogEntityId,
    segment_order: u32,
    title: String,
    start_ms: u64,
    duration_ms: DurationMs,
    recording_id: Option<LocalCatalogEntityId>,
}

impl AssetSegmentRow {
    /// Creates a segment row while retaining its containing asset identifier.
    #[must_use]
    pub fn from_model(asset_id: LocalCatalogEntityId, segment: &AssetSegment) -> Self {
        Self {
            asset_id,
            segment_order: segment.order(),
            title: segment.title().to_owned(),
            start_ms: segment.start_ms(),
            duration_ms: segment.duration_ms(),
            recording_id: segment.recording_id(),
        }
    }

    /// Returns the containing local asset identifier.
    #[must_use]
    pub const fn asset_id(&self) -> LocalCatalogEntityId {
        self.asset_id
    }

    /// Returns the one-based segment order.
    #[must_use]
    pub const fn segment_order(&self) -> u32 {
        self.segment_order
    }

    /// Returns the original display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the segment start offset in milliseconds.
    #[must_use]
    pub const fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// Returns the positive segment duration.
    #[must_use]
    pub const fn duration_ms(&self) -> DurationMs {
        self.duration_ms
    }

    /// Returns the linked local recording, when known.
    #[must_use]
    pub const fn recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.recording_id
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        AssetSegmentRow, LocalAssetRow, RecordingRow, ReleaseMediumRow, ReleaseRow,
        ReleaseTrackRow, WorkRow,
    };
    use aurorix_model::{
        ids::LocalCatalogEntityId,
        music::{
            AssetSegment, DurationMs, LocalAsset, LocalAssetAvailability, Recording, Release,
            ReleaseMedium, ReleaseTrack, Work,
        },
    };

    fn id(value: u128) -> LocalCatalogEntityId {
        LocalCatalogEntityId::from_uuid(Uuid::from_u128(value))
    }

    #[test]
    fn work_and_recording_rows_preserve_ids_text_links_and_optional_duration() {
        let work = Work::new(id(1), "  Theme  ").expect("valid work");
        let recording = Recording::new(id(2), "  Track  ", Some(182_000), vec![work.id(), id(3)])
            .expect("valid recording");

        let work_row = WorkRow::from(&work);
        assert_eq!(work_row.id(), id(1));
        assert_eq!(work_row.title(), "  Theme  ");

        let recording_row = RecordingRow::from(&recording);
        assert_eq!(recording_row.id(), id(2));
        assert_eq!(recording_row.title(), "  Track  ");
        assert_eq!(
            recording_row.duration_ms().map(DurationMs::get),
            Some(182_000)
        );
        assert_eq!(recording_row.work_ids(), [id(1), id(3)]);
    }

    #[test]
    fn release_rows_retain_parent_ids_positions_and_track_metadata() {
        let recording_id = id(20);
        let track = ReleaseTrack::new(id(22), 1, 4, 2, "Track", Some(recording_id), None)
            .expect("valid track");
        let medium = ReleaseMedium::new(id(21), 1, vec![track.clone()]).expect("valid medium");
        let release = Release::new(id(19), "Release", vec![medium.clone()]).expect("valid release");

        let release_row = ReleaseRow::from(&release);
        assert_eq!(release_row.id(), id(19));
        assert_eq!(release_row.title(), "Release");

        let medium_row = ReleaseMediumRow::from_model(release.id(), &medium);
        assert_eq!(medium_row.id(), id(21));
        assert_eq!(medium_row.release_id(), id(19));
        assert_eq!(medium_row.number(), 1);

        let track_row = ReleaseTrackRow::from_model(medium.id(), &track);
        assert_eq!(track_row.id(), id(22));
        assert_eq!(track_row.medium_id(), id(21));
        assert_eq!(track_row.medium_number(), 1);
        assert_eq!(track_row.track_number(), 4);
        assert_eq!(track_row.track_order(), 2);
        assert_eq!(track_row.title(), "Track");
        assert_eq!(track_row.primary_recording_id(), Some(recording_id));
        assert_eq!(track_row.duration_ms(), None);
    }

    #[test]
    fn missing_asset_is_retained_as_availability_without_delete_semantics() {
        let segment = AssetSegment::new(1, "Side A", 0, 30_000, None).expect("valid segment");
        let asset = LocalAsset::new(
            id(30),
            Some(id(20)),
            Some(30_000),
            LocalAssetAvailability::Missing,
            vec![segment.clone()],
        )
        .expect("valid missing asset");

        let asset_row = LocalAssetRow::from(&asset);
        assert_eq!(asset_row.id(), id(30));
        assert_eq!(asset_row.primary_recording_id(), Some(id(20)));
        assert_eq!(asset_row.duration_ms().map(DurationMs::get), Some(30_000));
        assert_eq!(asset_row.availability(), LocalAssetAvailability::Missing);

        let segment_row = AssetSegmentRow::from_model(asset.id(), &segment);
        assert_eq!(segment_row.asset_id(), id(30));
        assert_eq!(segment_row.segment_order(), 1);
        assert_eq!(segment_row.title(), "Side A");
        assert_eq!(segment_row.start_ms(), 0);
        assert_eq!(segment_row.duration_ms().get(), 30_000);
        assert_eq!(segment_row.recording_id(), None);
    }
}

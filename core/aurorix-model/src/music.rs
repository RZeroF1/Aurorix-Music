//! Local music-catalog graph value objects.
//!
//! These types deliberately use only [`LocalCatalogEntityId`] values. They do
//! not define portable or replicated media identity.

use std::{
    collections::HashSet,
    error::Error,
    fmt::{self, Display},
};

use crate::ids::LocalCatalogEntityId;

/// Validation failure for a music graph value object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicGraphError {
    /// A required human-readable field has no non-whitespace characters.
    EmptyText {
        /// The rejected field.
        field: &'static str,
    },
    /// A duration was zero milliseconds.
    ZeroDuration,
    /// A release medium number was zero.
    ZeroMediumNumber,
    /// A release track number was zero.
    ZeroTrackNumber,
    /// A release track order was zero.
    ZeroTrackOrder,
    /// An asset segment order was zero.
    ZeroSegmentOrder,
    /// A credit sort order was zero.
    ZeroCreditOrder,
    /// A release did not contain any media.
    EmptyRelease,
    /// A release medium did not contain any tracks.
    EmptyReleaseMedium,
    /// A local asset did not contain any segments.
    EmptyAssetSegments,
    /// A recording referenced the same work more than once.
    DuplicateWorkReference,
    /// A release contained the same medium number more than once.
    DuplicateMediumNumber {
        /// The repeated medium number.
        number: u32,
    },
    /// Release media were not stored in ascending medium-number order.
    MediumOrder {
        /// The preceding medium number.
        preceding: u32,
        /// The following medium number.
        following: u32,
    },
    /// A track was placed in a release medium with a different number.
    TrackMediumMismatch {
        /// The containing medium number.
        expected: u32,
        /// The track's medium number.
        actual: u32,
    },
    /// A release medium reused a track number.
    DuplicateTrackNumber {
        /// The containing medium number.
        medium_number: u32,
        /// The repeated track number.
        track_number: u32,
    },
    /// A release medium reused a track order.
    DuplicateTrackOrder {
        /// The containing medium number.
        medium_number: u32,
        /// The repeated track order.
        track_order: u32,
    },
    /// Release tracks were not stored in ascending track-order order.
    TrackOrder {
        /// The preceding track order.
        preceding: u32,
        /// The following track order.
        following: u32,
    },
    /// A local asset reused an asset segment order.
    DuplicateSegmentOrder {
        /// The repeated segment order.
        order: u32,
    },
    /// Asset segments were not stored in ascending segment-order order.
    SegmentOrder {
        /// The preceding segment order.
        preceding: u32,
        /// The following segment order.
        following: u32,
    },
    /// Segment start plus duration overflowed a millisecond value.
    SegmentEndOverflow,
    /// A segment extended past its asset's known duration.
    SegmentExceedsAssetDuration,
}

impl Display for MusicGraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyText { field } => write!(formatter, "{field} must not be empty"),
            Self::ZeroDuration => formatter.write_str("duration must be greater than zero"),
            Self::ZeroMediumNumber => {
                formatter.write_str("release medium number must be greater than zero")
            }
            Self::ZeroTrackNumber => {
                formatter.write_str("release track number must be greater than zero")
            }
            Self::ZeroTrackOrder => {
                formatter.write_str("release track order must be greater than zero")
            }
            Self::ZeroSegmentOrder => {
                formatter.write_str("asset segment order must be greater than zero")
            }
            Self::ZeroCreditOrder => {
                formatter.write_str("credit sort order must be greater than zero")
            }
            Self::EmptyRelease => formatter.write_str("release must contain at least one medium"),
            Self::EmptyReleaseMedium => {
                formatter.write_str("release medium must contain at least one track")
            }
            Self::EmptyAssetSegments => {
                formatter.write_str("local asset must contain at least one segment")
            }
            Self::DuplicateWorkReference => {
                formatter.write_str("recording must not reference the same work twice")
            }
            Self::DuplicateMediumNumber { number } => {
                write!(formatter, "release medium number {number} is duplicated")
            }
            Self::MediumOrder {
                preceding,
                following,
            } => write!(
                formatter,
                "release medium number {following} must follow {preceding}"
            ),
            Self::TrackMediumMismatch { expected, actual } => write!(
                formatter,
                "track medium number {actual} does not match containing medium {expected}"
            ),
            Self::DuplicateTrackNumber {
                medium_number,
                track_number,
            } => write!(
                formatter,
                "track number {track_number} is duplicated in medium {medium_number}"
            ),
            Self::DuplicateTrackOrder {
                medium_number,
                track_order,
            } => write!(
                formatter,
                "track order {track_order} is duplicated in medium {medium_number}"
            ),
            Self::TrackOrder {
                preceding,
                following,
            } => write!(formatter, "track order {following} must follow {preceding}"),
            Self::DuplicateSegmentOrder { order } => {
                write!(formatter, "asset segment order {order} is duplicated")
            }
            Self::SegmentOrder {
                preceding,
                following,
            } => write!(
                formatter,
                "asset segment order {following} must follow {preceding}"
            ),
            Self::SegmentEndOverflow => {
                formatter.write_str("asset segment start and duration overflowed")
            }
            Self::SegmentExceedsAssetDuration => {
                formatter.write_str("asset segment extends past the known asset duration")
            }
        }
    }
}

impl Error for MusicGraphError {}

/// A known, positive duration measured in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurationMs(u64);

impl DurationMs {
    /// Creates a duration from a positive millisecond count.
    ///
    /// # Errors
    ///
    /// Returns [`MusicGraphError::ZeroDuration`] when `milliseconds` is zero.
    pub fn new(milliseconds: u64) -> Result<Self, MusicGraphError> {
        if milliseconds == 0 {
            return Err(MusicGraphError::ZeroDuration);
        }

        Ok(Self(milliseconds))
    }

    /// Returns the duration in milliseconds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for DurationMs {
    type Error = MusicGraphError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A local musical work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    id: LocalCatalogEntityId,
    title: String,
}

impl Work {
    /// Creates a local work with a required title.
    ///
    /// # Errors
    ///
    /// Returns [`MusicGraphError::EmptyText`] when `title` is blank.
    pub fn new(
        id: LocalCatalogEntityId,
        title: impl Into<String>,
    ) -> Result<Self, MusicGraphError> {
        Ok(Self {
            id,
            title: required_text(title.into(), "work title")?,
        })
    }

    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// A local recording, which may be linked to zero or more works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recording {
    id: LocalCatalogEntityId,
    title: String,
    duration_ms: Option<DurationMs>,
    work_ids: Vec<LocalCatalogEntityId>,
}

impl Recording {
    /// Creates a recording with optional known duration and local work links.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank title, zero duration, or duplicate work link.
    pub fn new(
        id: LocalCatalogEntityId,
        title: impl Into<String>,
        duration_ms: Option<u64>,
        work_ids: Vec<LocalCatalogEntityId>,
    ) -> Result<Self, MusicGraphError> {
        ensure_unique_work_ids(&work_ids)?;

        Ok(Self {
            id,
            title: required_text(title.into(), "recording title")?,
            duration_ms: optional_duration(duration_ms)?,
            work_ids,
        })
    }

    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the known duration, when catalog metadata provides one.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }

    /// Returns the linked local works.
    #[must_use]
    pub fn work_ids(&self) -> &[LocalCatalogEntityId] {
        &self.work_ids
    }
}

/// A release containing one or more ordered media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    id: LocalCatalogEntityId,
    title: String,
    media: Vec<ReleaseMedium>,
}

impl Release {
    /// Creates a release whose media are in ascending medium-number order.
    ///
    /// # Errors
    ///
    /// Returns an error for a blank title, an empty release, duplicate medium
    /// numbers, or media out of order.
    pub fn new(
        id: LocalCatalogEntityId,
        title: impl Into<String>,
        media: Vec<ReleaseMedium>,
    ) -> Result<Self, MusicGraphError> {
        ensure_release_media(&media)?;

        Ok(Self {
            id,
            title: required_text(title.into(), "release title")?,
            media,
        })
    }

    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the display title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the ordered media.
    #[must_use]
    pub fn media(&self) -> &[ReleaseMedium] {
        &self.media
    }
}

/// One numbered medium of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseMedium {
    id: LocalCatalogEntityId,
    number: u32,
    tracks: Vec<ReleaseTrack>,
}

impl ReleaseMedium {
    /// Creates a release medium with ordered tracks.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero medium number, empty medium, track assigned
    /// to another medium, duplicate track position, or unordered tracks.
    pub fn new(
        id: LocalCatalogEntityId,
        number: u32,
        tracks: Vec<ReleaseTrack>,
    ) -> Result<Self, MusicGraphError> {
        if number == 0 {
            return Err(MusicGraphError::ZeroMediumNumber);
        }

        ensure_release_tracks(number, &tracks)?;

        Ok(Self { id, number, tracks })
    }

    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the one-based medium number.
    #[must_use]
    pub const fn number(&self) -> u32 {
        self.number
    }

    /// Returns the tracks in release order.
    #[must_use]
    pub fn tracks(&self) -> &[ReleaseTrack] {
        &self.tracks
    }
}

/// A position on a release medium, optionally linked to its primary recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTrack {
    id: LocalCatalogEntityId,
    medium_number: u32,
    track_number: u32,
    track_order: u32,
    title: String,
    primary_recording_id: Option<LocalCatalogEntityId>,
    duration_ms: Option<DurationMs>,
}

impl ReleaseTrack {
    /// Creates a release track position.
    ///
    /// `primary_recording_id` is optional because a catalog may know a release
    /// position before it has established a recording link.
    ///
    /// # Errors
    ///
    /// Returns an error for zero position fields, a blank title, or zero known
    /// duration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: LocalCatalogEntityId,
        medium_number: u32,
        track_number: u32,
        track_order: u32,
        title: impl Into<String>,
        primary_recording_id: Option<LocalCatalogEntityId>,
        duration_ms: Option<u64>,
    ) -> Result<Self, MusicGraphError> {
        if medium_number == 0 {
            return Err(MusicGraphError::ZeroMediumNumber);
        }
        if track_number == 0 {
            return Err(MusicGraphError::ZeroTrackNumber);
        }
        if track_order == 0 {
            return Err(MusicGraphError::ZeroTrackOrder);
        }

        Ok(Self {
            id,
            medium_number,
            track_number,
            track_order,
            title: required_text(title.into(), "release track title")?,
            primary_recording_id,
            duration_ms: optional_duration(duration_ms)?,
        })
    }

    /// Returns the local catalog identifier for this release position.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the one-based medium number.
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

    /// Returns the display title for this release position.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the linked local recording, when known.
    #[must_use]
    pub const fn primary_recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.primary_recording_id
    }

    /// Returns the known duration, when catalog metadata provides one.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }
}

/// Availability observed for a local asset without changing its music links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalAssetAvailability {
    /// The asset is currently available to the local catalog.
    Available,
    /// The asset was known previously but is not currently available.
    Missing,
}

/// Device-local media bytes and their ordered logical segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAsset {
    id: LocalCatalogEntityId,
    primary_recording_id: Option<LocalCatalogEntityId>,
    duration_ms: Option<DurationMs>,
    availability: LocalAssetAvailability,
    segments: Vec<AssetSegment>,
}

impl LocalAsset {
    /// Creates a local asset with at least one ordered segment.
    ///
    /// # Errors
    ///
    /// Returns an error for zero known duration, no segments, unordered
    /// segments, arithmetic overflow, or a segment beyond a known duration.
    pub fn new(
        id: LocalCatalogEntityId,
        primary_recording_id: Option<LocalCatalogEntityId>,
        duration_ms: Option<u64>,
        availability: LocalAssetAvailability,
        segments: Vec<AssetSegment>,
    ) -> Result<Self, MusicGraphError> {
        let duration_ms = optional_duration(duration_ms)?;
        ensure_asset_segments(&segments, duration_ms)?;

        Ok(Self {
            id,
            primary_recording_id,
            duration_ms,
            availability,
            segments,
        })
    }

    /// Returns the local catalog identifier.
    #[must_use]
    pub const fn id(&self) -> LocalCatalogEntityId {
        self.id
    }

    /// Returns the primary linked recording, when known.
    #[must_use]
    pub const fn primary_recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.primary_recording_id
    }

    /// Returns the known duration, when probing has produced one.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<DurationMs> {
        self.duration_ms
    }

    /// Returns the current local availability without changing any music link.
    #[must_use]
    pub const fn availability(&self) -> LocalAssetAvailability {
        self.availability
    }

    /// Returns the segments in their declared order.
    #[must_use]
    pub fn segments(&self) -> &[AssetSegment] {
        &self.segments
    }

    /// Returns this asset with a different observed availability.
    #[must_use]
    pub fn with_availability(mut self, availability: LocalAssetAvailability) -> Self {
        self.availability = availability;
        self
    }
}

/// An ordered logical segment within one local asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSegment {
    order: u32,
    title: String,
    start_ms: u64,
    duration_ms: DurationMs,
    recording_id: Option<LocalCatalogEntityId>,
}

impl AssetSegment {
    /// Creates a positive-duration segment in a local asset.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero order, blank title, or zero duration.
    pub fn new(
        order: u32,
        title: impl Into<String>,
        start_ms: u64,
        duration_ms: u64,
        recording_id: Option<LocalCatalogEntityId>,
    ) -> Result<Self, MusicGraphError> {
        if order == 0 {
            return Err(MusicGraphError::ZeroSegmentOrder);
        }

        Ok(Self {
            order,
            title: required_text(title.into(), "asset segment title")?,
            start_ms,
            duration_ms: DurationMs::new(duration_ms)?,
            recording_id,
        })
    }

    /// Returns the one-based segment order.
    #[must_use]
    pub const fn order(&self) -> u32 {
        self.order
    }

    /// Returns the display title.
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

    /// Returns the local recording associated with this segment, when known.
    #[must_use]
    pub const fn recording_id(&self) -> Option<LocalCatalogEntityId> {
        self.recording_id
    }

    fn end_ms(&self) -> Result<u64, MusicGraphError> {
        self.start_ms
            .checked_add(self.duration_ms.get())
            .ok_or(MusicGraphError::SegmentEndOverflow)
    }
}

/// The graph entity to which a credit applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreditTarget {
    /// A local work.
    Work(LocalCatalogEntityId),
    /// A local recording.
    Recording(LocalCatalogEntityId),
    /// A local release.
    Release(LocalCatalogEntityId),
    /// A local release track position.
    ReleaseTrack(LocalCatalogEntityId),
}

/// An ordered, target-scoped credit relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credit {
    target: CreditTarget,
    participant: String,
    role: String,
    credited_name: String,
    sort_order: u32,
    join_phrase: Option<String>,
}

impl Credit {
    /// Creates a credit relation for one graph target.
    ///
    /// # Errors
    ///
    /// Returns an error for blank participant, role, credited name, or join
    /// phrase, or for zero sort order.
    pub fn new(
        target: CreditTarget,
        participant: impl Into<String>,
        role: impl Into<String>,
        credited_name: impl Into<String>,
        sort_order: u32,
        join_phrase: Option<String>,
    ) -> Result<Self, MusicGraphError> {
        if sort_order == 0 {
            return Err(MusicGraphError::ZeroCreditOrder);
        }

        let join_phrase = join_phrase
            .map(|value| required_text(value, "credit join phrase"))
            .transpose()?;

        Ok(Self {
            target,
            participant: required_text(participant.into(), "credit participant")?,
            role: required_text(role.into(), "credit role")?,
            credited_name: required_text(credited_name.into(), "credited name")?,
            sort_order,
            join_phrase,
        })
    }

    /// Returns the graph target for this credit.
    #[must_use]
    pub const fn target(&self) -> CreditTarget {
        self.target
    }

    /// Returns the credited participant.
    #[must_use]
    pub fn participant(&self) -> &str {
        &self.participant
    }

    /// Returns the participant's role.
    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    /// Returns the displayed credited name.
    #[must_use]
    pub fn credited_name(&self) -> &str {
        &self.credited_name
    }

    /// Returns the one-based display order for this target's credits.
    #[must_use]
    pub const fn sort_order(&self) -> u32 {
        self.sort_order
    }

    /// Returns the optional text used to join this credit to the next one.
    #[must_use]
    pub fn join_phrase(&self) -> Option<&str> {
        self.join_phrase.as_deref()
    }
}

fn required_text(value: String, field: &'static str) -> Result<String, MusicGraphError> {
    if value.trim().is_empty() {
        return Err(MusicGraphError::EmptyText { field });
    }

    Ok(value)
}

fn optional_duration(value: Option<u64>) -> Result<Option<DurationMs>, MusicGraphError> {
    value.map(DurationMs::new).transpose()
}

fn ensure_unique_work_ids(work_ids: &[LocalCatalogEntityId]) -> Result<(), MusicGraphError> {
    let mut seen = HashSet::with_capacity(work_ids.len());
    for work_id in work_ids {
        if !seen.insert(*work_id) {
            return Err(MusicGraphError::DuplicateWorkReference);
        }
    }

    Ok(())
}

fn ensure_release_media(media: &[ReleaseMedium]) -> Result<(), MusicGraphError> {
    if media.is_empty() {
        return Err(MusicGraphError::EmptyRelease);
    }

    let mut numbers = HashSet::with_capacity(media.len());
    let mut previous = None;
    for medium in media {
        if !numbers.insert(medium.number) {
            return Err(MusicGraphError::DuplicateMediumNumber {
                number: medium.number,
            });
        }
        if let Some(preceding) = previous
            && medium.number <= preceding
        {
            return Err(MusicGraphError::MediumOrder {
                preceding,
                following: medium.number,
            });
        }
        previous = Some(medium.number);
    }

    Ok(())
}

fn ensure_release_tracks(
    medium_number: u32,
    tracks: &[ReleaseTrack],
) -> Result<(), MusicGraphError> {
    if tracks.is_empty() {
        return Err(MusicGraphError::EmptyReleaseMedium);
    }

    let mut numbers = HashSet::with_capacity(tracks.len());
    let mut orders = HashSet::with_capacity(tracks.len());
    let mut previous_order = None;

    for track in tracks {
        if track.medium_number != medium_number {
            return Err(MusicGraphError::TrackMediumMismatch {
                expected: medium_number,
                actual: track.medium_number,
            });
        }
        if !numbers.insert(track.track_number) {
            return Err(MusicGraphError::DuplicateTrackNumber {
                medium_number,
                track_number: track.track_number,
            });
        }
        if !orders.insert(track.track_order) {
            return Err(MusicGraphError::DuplicateTrackOrder {
                medium_number,
                track_order: track.track_order,
            });
        }
        if let Some(preceding) = previous_order
            && track.track_order <= preceding
        {
            return Err(MusicGraphError::TrackOrder {
                preceding,
                following: track.track_order,
            });
        }
        previous_order = Some(track.track_order);
    }

    Ok(())
}

fn ensure_asset_segments(
    segments: &[AssetSegment],
    asset_duration: Option<DurationMs>,
) -> Result<(), MusicGraphError> {
    if segments.is_empty() {
        return Err(MusicGraphError::EmptyAssetSegments);
    }

    let mut orders = HashSet::with_capacity(segments.len());
    let mut previous_order = None;

    for segment in segments {
        if !orders.insert(segment.order) {
            return Err(MusicGraphError::DuplicateSegmentOrder {
                order: segment.order,
            });
        }
        if let Some(preceding) = previous_order
            && segment.order <= preceding
        {
            return Err(MusicGraphError::SegmentOrder {
                preceding,
                following: segment.order,
            });
        }
        if let Some(asset_duration) = asset_duration
            && segment.end_ms()? > asset_duration.get()
        {
            return Err(MusicGraphError::SegmentExceedsAssetDuration);
        }
        previous_order = Some(segment.order);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{
        AssetSegment, Credit, CreditTarget, DurationMs, LocalAsset, LocalAssetAvailability,
        MusicGraphError, Recording, Release, ReleaseMedium, ReleaseTrack, Work,
    };
    use crate::ids::LocalCatalogEntityId;

    fn id(value: u128) -> LocalCatalogEntityId {
        LocalCatalogEntityId::from_uuid(Uuid::from_u128(value))
    }

    fn track(
        id_value: u128,
        medium_number: u32,
        track_number: u32,
        track_order: u32,
    ) -> ReleaseTrack {
        ReleaseTrack::new(
            id(id_value),
            medium_number,
            track_number,
            track_order,
            "Track",
            None,
            Some(180_000),
        )
        .unwrap()
    }

    fn segment(order: u32, start_ms: u64, duration_ms: u64) -> AssetSegment {
        AssetSegment::new(order, "Segment", start_ms, duration_ms, None).unwrap()
    }

    #[test]
    fn blank_titles_are_rejected() {
        assert_eq!(
            Work::new(id(1), " \t "),
            Err(MusicGraphError::EmptyText {
                field: "work title"
            })
        );
        assert_eq!(
            Recording::new(id(2), "\n", None, Vec::new()),
            Err(MusicGraphError::EmptyText {
                field: "recording title"
            })
        );
        assert_eq!(
            ReleaseTrack::new(id(3), 1, 1, 1, "", None, None),
            Err(MusicGraphError::EmptyText {
                field: "release track title"
            })
        );
        assert_eq!(
            AssetSegment::new(1, "", 0, 1, None),
            Err(MusicGraphError::EmptyText {
                field: "asset segment title"
            })
        );
    }

    #[test]
    fn zero_durations_are_rejected() {
        assert_eq!(DurationMs::new(0), Err(MusicGraphError::ZeroDuration));
        assert_eq!(
            Recording::new(id(1), "Recording", Some(0), Vec::new()),
            Err(MusicGraphError::ZeroDuration)
        );
        assert_eq!(
            ReleaseTrack::new(id(2), 1, 1, 1, "Track", None, Some(0)),
            Err(MusicGraphError::ZeroDuration)
        );
        assert_eq!(
            AssetSegment::new(1, "Segment", 0, 0, None),
            Err(MusicGraphError::ZeroDuration)
        );
    }

    #[test]
    fn release_tracks_are_positions_not_recordings() {
        let recording_id = id(1);
        let release_track =
            ReleaseTrack::new(id(2), 1, 3, 1, "Release edit", None, Some(200_000)).unwrap();
        let medium = ReleaseMedium::new(id(3), 1, vec![release_track.clone()]).unwrap();
        let release = Release::new(id(4), "Release", vec![medium]).unwrap();

        assert_ne!(release_track.id(), recording_id);
        assert_eq!(release_track.primary_recording_id(), None);
        assert_eq!(release_track.medium_number(), 1);
        assert_eq!(release_track.track_number(), 3);
        assert_eq!(release.media()[0].tracks()[0], release_track);
    }

    #[test]
    fn release_rejects_invalid_or_inconsistent_positions() {
        assert_eq!(
            ReleaseTrack::new(id(1), 0, 1, 1, "Track", None, None),
            Err(MusicGraphError::ZeroMediumNumber)
        );
        assert_eq!(
            ReleaseTrack::new(id(1), 1, 0, 1, "Track", None, None),
            Err(MusicGraphError::ZeroTrackNumber)
        );
        assert_eq!(
            ReleaseTrack::new(id(1), 1, 1, 0, "Track", None, None),
            Err(MusicGraphError::ZeroTrackOrder)
        );

        let incorrect_medium_track = track(2, 2, 1, 1);
        assert_eq!(
            ReleaseMedium::new(id(3), 1, vec![incorrect_medium_track]),
            Err(MusicGraphError::TrackMediumMismatch {
                expected: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn release_rejects_unordered_media_and_tracks() {
        let first = ReleaseMedium::new(id(1), 1, vec![track(2, 1, 1, 1)]).unwrap();
        let second = ReleaseMedium::new(id(3), 2, vec![track(4, 2, 1, 1)]).unwrap();
        assert_eq!(
            Release::new(id(5), "Release", vec![second, first]),
            Err(MusicGraphError::MediumOrder {
                preceding: 2,
                following: 1,
            })
        );

        let second_track = track(6, 1, 2, 2);
        let first_track = track(7, 1, 1, 1);
        assert_eq!(
            ReleaseMedium::new(id(8), 1, vec![second_track, first_track]),
            Err(MusicGraphError::TrackOrder {
                preceding: 2,
                following: 1,
            })
        );
    }

    #[test]
    fn local_asset_can_be_missing_without_losing_its_recording_link() {
        let recording_id = id(1);
        let asset = LocalAsset::new(
            id(2),
            Some(recording_id),
            Some(180_000),
            LocalAssetAvailability::Available,
            vec![AssetSegment::new(1, "Track", 0, 180_000, Some(recording_id)).unwrap()],
        )
        .unwrap()
        .with_availability(LocalAssetAvailability::Missing);

        assert_eq!(asset.availability(), LocalAssetAvailability::Missing);
        assert_eq!(asset.primary_recording_id(), Some(recording_id));
        assert_eq!(asset.segments()[0].recording_id(), Some(recording_id));
    }

    #[test]
    fn local_asset_rejects_unordered_or_out_of_bounds_segments() {
        assert_eq!(
            LocalAsset::new(
                id(1),
                None,
                None,
                LocalAssetAvailability::Available,
                vec![segment(2, 0, 1), segment(1, 1, 1)],
            ),
            Err(MusicGraphError::SegmentOrder {
                preceding: 2,
                following: 1,
            })
        );
        assert_eq!(
            LocalAsset::new(
                id(2),
                None,
                Some(100),
                LocalAssetAvailability::Available,
                vec![segment(1, 80, 21)],
            ),
            Err(MusicGraphError::SegmentExceedsAssetDuration)
        );
    }

    #[test]
    fn credits_have_explicit_target_scope_and_order() {
        let target = CreditTarget::ReleaseTrack(id(1));
        let credit = Credit::new(
            target,
            "Artist",
            "performer",
            "Artist",
            2,
            Some(" feat. ".to_owned()),
        )
        .unwrap();

        assert_eq!(credit.target(), target);
        assert_eq!(credit.sort_order(), 2);
        assert_eq!(credit.join_phrase(), Some(" feat. "));
        assert_eq!(
            Credit::new(target, "Artist", "performer", "Artist", 0, None),
            Err(MusicGraphError::ZeroCreditOrder)
        );
    }
}

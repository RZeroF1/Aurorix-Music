//! Versioned, bounded transport projections for the public FFI boundary.
//!
//! The C ABI still exposes the bootstrap envelope from G3-05. Home is a
//! typed, public-safe payload carried by the existing Extension arm so this
//! contract can be reviewed without changing the ABI lifetime or dispatch
//! surface. No Core query, reducer, provider runtime, or UI code belongs here.

/// The only schema major accepted by this transport.
pub const SCHEMA_MAJOR: u32 = 1;
/// Maximum ordinary request or response envelope size.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// Maximum event envelope size reserved by the transport contract.
pub const MAX_EVENT_BYTES: usize = 256 * 1024;
/// Namespace used when a Home payload is carried by the bootstrap extension.
pub const HOME_EXTENSION_NAMESPACE: &str = "aurorix.home";
/// Payload schema version used by the Home projection.
pub const HOME_EXTENSION_SCHEMA_VERSION: &str = "1";
/// Home value projection version within transport schema major 1.
pub const HOME_PROJECTION_VERSION: u32 = 1;
/// Maximum number of sections in one Home snapshot.
pub const MAX_HOME_SECTIONS: usize = 4;
/// Maximum number of cards in one Home section or recommendation set.
pub const MAX_HOME_CARDS: usize = 64;
/// Maximum number of quick entries in one Home snapshot.
pub const MAX_HOME_QUICK_ENTRIES: usize = 32;
/// Maximum number of bytes in a Home card identity.
pub const MAX_HOME_CARD_ID_BYTES: usize = 128;
/// Maximum number of bytes in a Home route identifier.
pub const MAX_HOME_ROUTE_ID_BYTES: usize = 256;
/// Maximum number of bytes in a Home search query.
pub const MAX_HOME_QUERY_BYTES: usize = 512;
/// Maximum number of bytes in a Home display string.
pub const MAX_HOME_TEXT_BYTES: usize = 512;
/// Maximum number of bytes in a Home icon or artwork key.
pub const MAX_HOME_ASSET_KEY_BYTES: usize = 256;
/// Maximum number of bytes in a Home source identifier.
pub const MAX_HOME_SOURCE_ID_BYTES: usize = 128;
/// Maximum number of bytes in a Home command identifier.
pub const MAX_HOME_COMMAND_ID_BYTES: usize = 128;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_NAMESPACE_BYTES: usize = 128;
const MAX_EXTENSION_SCHEMA_BYTES: usize = 64;
const MAX_EXTENSION_PAYLOAD_BYTES: usize = MAX_MESSAGE_BYTES - 256;

const FIELD_SCHEMA_VERSION: u32 = 1;
const FIELD_REQUEST_ID: u32 = 2;
const FIELD_PING: u32 = 10;
const FIELD_EXTENSION: u32 = 11;
const FIELD_PONG: u32 = 10;
const FIELD_ERROR: u32 = 11;
const MAX_ERROR_CODE_BYTES: usize = 64;
const MAX_ERROR_MESSAGE_BYTES: usize = 1024;

const FIELD_HOME_PROJECTION_VERSION: u32 = 1;
const FIELD_HOME_OBSERVED_EVENT_SEQUENCE: u32 = 2;
const FIELD_HOME_SOURCE: u32 = 3;
const FIELD_HOME_STATUS: u32 = 4;
const FIELD_HOME_SECTIONS: u32 = 5;
const FIELD_HOME_QUICK_ENTRIES: u32 = 6;
const FIELD_HOME_DISCOVER: u32 = 7;

const FIELD_HOME_COMMAND_VERSION: u32 = 1;
const FIELD_HOME_COMMAND_ID: u32 = 2;
const FIELD_HOME_COMMAND_OBSERVED_EVENT_SEQUENCE: u32 = 3;
const FIELD_HOME_OPEN_QUICK_ENTRY: u32 = 10;
const FIELD_HOME_PLAY_TRACK: u32 = 11;
const FIELD_HOME_OPEN_RECOMMENDATION: u32 = 12;
const FIELD_HOME_OPEN_ALL_RECENT: u32 = 13;
const FIELD_HOME_CUSTOMIZE: u32 = 14;
const FIELD_HOME_SEARCH: u32 = 15;
const FIELD_HOME_TOGGLE_FAVORITE: u32 = 16;
const FIELD_HOME_OPEN_EXTENSION: u32 = 17;

const MAX_HOME_COMMAND_MESSAGE_BYTES: usize = 1024;
const MAX_HOME_SOURCE_MESSAGE_BYTES: usize = 1024;
const MAX_HOME_STATUS_MESSAGE_BYTES: usize = 1024;
const MAX_HOME_CARD_MESSAGE_BYTES: usize = 4096;
const MAX_HOME_SECTION_MESSAGE_BYTES: usize = 300 * 1024;
const MAX_HOME_QUICK_ENTRY_MESSAGE_BYTES: usize = 2048;
const MAX_HOME_RECOMMENDATION_MESSAGE_BYTES: usize = 4096;
const MAX_HOME_RECOMMENDATION_SET_MESSAGE_BYTES: usize = 300 * 1024;

/// One public-safe request body arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestBody {
    /// A harmless liveness request used by generation and ABI smoke tests.
    Ping,
    /// A bounded, namespaced payload reserved for reviewed extensions.
    Extension(ExtensionRequest),
}

/// One public-safe response body arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseBody {
    /// The response to a liveness request.
    Pong,
    /// A bounded, display-safe transport error.
    Error(FfiError),
}

/// A namespaced extension transport projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRequest {
    namespace: String,
    schema_version: String,
    payload: Vec<u8>,
}

impl ExtensionRequest {
    /// Creates a bounded extension request.
    ///
    /// # Errors
    ///
    /// Returns an error when a text field is empty or exceeds its bound, or
    /// when the payload exceeds the extension limit.
    pub fn new(
        namespace: impl Into<String>,
        schema_version: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self, TransportError> {
        let request = Self {
            namespace: namespace.into(),
            schema_version: schema_version.into(),
            payload: payload.into(),
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the registered extension namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the extension payload schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    /// Returns the bounded payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Creates a bootstrap extension containing a Home snapshot payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot or resulting extension exceeds its
    /// declared bounds.
    pub fn from_home_snapshot(snapshot: &HomeSnapshot) -> Result<Self, TransportError> {
        Self::new(
            HOME_EXTENSION_NAMESPACE,
            HOME_EXTENSION_SCHEMA_VERSION,
            snapshot.encode()?,
        )
    }

    /// Decodes a Home snapshot from the reviewed Home extension namespace.
    ///
    /// # Errors
    ///
    /// Returns an error for a different namespace/version, malformed payload,
    /// unknown fields, invalid values, or exceeded limits.
    pub fn decode_home_snapshot(&self) -> Result<HomeSnapshot, TransportError> {
        if self.namespace != HOME_EXTENSION_NAMESPACE {
            return Err(TransportError::InvalidValue(
                "unexpected Home extension namespace",
            ));
        }
        if self.schema_version != HOME_EXTENSION_SCHEMA_VERSION {
            return Err(TransportError::UnsupportedProjection {
                expected: HOME_PROJECTION_VERSION,
                actual: parse_projection_version(&self.schema_version)?,
            });
        }
        HomeSnapshot::decode(&self.payload)
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text("extension namespace", &self.namespace, MAX_NAMESPACE_BYTES)?;
        validate_text(
            "extension schema version",
            &self.schema_version,
            MAX_EXTENSION_SCHEMA_BYTES,
        )?;
        if self.payload.len() > MAX_EXTENSION_PAYLOAD_BYTES {
            return Err(TransportError::FieldTooLarge {
                field: "extension payload",
                max: MAX_EXTENSION_PAYLOAD_BYTES,
                actual: self.payload.len(),
            });
        }
        Ok(())
    }
}

/// A typed transport error projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiError {
    code: String,
    message: String,
    retryable: bool,
}

impl FfiError {
    /// Creates a bounded transport error projection.
    ///
    /// # Errors
    ///
    /// Returns an error when the code or message is empty or exceeds its
    /// declared bound.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Result<Self, TransportError> {
        let error = Self {
            code: code.into(),
            message: message.into(),
            retryable,
        };
        validate_text("error code", &error.code, MAX_ERROR_CODE_BYTES)?;
        validate_text("error message", &error.message, MAX_ERROR_MESSAGE_BYTES)?;
        Ok(error)
    }

    /// Returns the machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the bounded display-safe error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether retrying may be appropriate.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }
}

/// The display state of a Home projection or one of its sections/cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HomeState {
    /// The projection contains usable data.
    Ready = 1,
    /// Data is being loaded and may be absent temporarily.
    Loading = 2,
    /// The source completed successfully but has no data.
    Empty = 3,
    /// The source is not currently reachable and cached data may be stale.
    Offline = 4,
    /// The source cannot provide this projection.
    Unavailable = 5,
}

impl HomeState {
    fn from_wire(value: u32) -> Result<Self, TransportError> {
        match value {
            1 => Ok(Self::Ready),
            2 => Ok(Self::Loading),
            3 => Ok(Self::Empty),
            4 => Ok(Self::Offline),
            5 => Ok(Self::Unavailable),
            _ => Err(TransportError::InvalidEnum {
                field: "home state",
                value,
            }),
        }
    }
}

/// A canonical Home section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HomeSectionKind {
    /// Tracks the user played most recently.
    RecentlyPlayed = 1,
    /// Tracks recently introduced to the local catalog.
    RecentlyAdded = 2,
    /// Tracks marked as favorites.
    Favorites = 3,
    /// The resumable playback item(s).
    ContinuePlayback = 4,
}

impl HomeSectionKind {
    fn from_wire(value: u32) -> Result<Self, TransportError> {
        match value {
            1 => Ok(Self::RecentlyPlayed),
            2 => Ok(Self::RecentlyAdded),
            3 => Ok(Self::Favorites),
            4 => Ok(Self::ContinuePlayback),
            _ => Err(TransportError::InvalidEnum {
                field: "home section kind",
                value,
            }),
        }
    }
}

/// The ownership/source class of a Home value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HomeSourceKind {
    /// Data owned by local Core state.
    Local = 1,
    /// Data supplied by a configured Provider.
    Provider = 2,
    /// Data supplied by a reviewed extension.
    Extension = 3,
    /// Data supplied by the Core/system projection itself.
    System = 4,
}

impl HomeSourceKind {
    fn from_wire(value: u32) -> Result<Self, TransportError> {
        match value {
            1 => Ok(Self::Local),
            2 => Ok(Self::Provider),
            3 => Ok(Self::Extension),
            4 => Ok(Self::System),
            _ => Err(TransportError::InvalidEnum {
                field: "home source kind",
                value,
            }),
        }
    }
}

/// Availability of the source that produced a Home value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HomeSourceStatus {
    /// The source responded successfully.
    Available = 1,
    /// The source is known to be offline.
    Offline = 2,
    /// A Provider did not answer within its bounded request window.
    TimedOut = 3,
    /// The source exists but cannot provide the requested data.
    Unavailable = 4,
}

impl HomeSourceStatus {
    fn from_wire(value: u32) -> Result<Self, TransportError> {
        match value {
            1 => Ok(Self::Available),
            2 => Ok(Self::Offline),
            3 => Ok(Self::TimedOut),
            4 => Ok(Self::Unavailable),
            _ => Err(TransportError::InvalidEnum {
                field: "home source status",
                value,
            }),
        }
    }
}

/// Bounded status metadata attached to a Home snapshot, section, or card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeStatus {
    state: HomeState,
    message: Option<String>,
    retryable: bool,
}

impl HomeStatus {
    /// Creates status metadata with no user-facing detail.
    #[must_use]
    pub const fn new(state: HomeState) -> Self {
        Self {
            state,
            message: None,
            retryable: false,
        }
    }

    /// Adds bounded status detail for a host to display or log.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Marks a status as eligible for a later retry.
    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Returns the projection state.
    #[must_use]
    pub const fn state(&self) -> HomeState {
        self.state
    }

    /// Returns optional bounded status detail.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Returns whether a retry is appropriate.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_optional_text(
            "home status message",
            self.message.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )
    }
}

/// Bounded source metadata shared by Home values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSourceMetadata {
    kind: HomeSourceKind,
    source_id: String,
    display_name: Option<String>,
    status: HomeSourceStatus,
    detail: Option<String>,
}

impl HomeSourceMetadata {
    /// Creates source metadata without exposing provider handles or secrets.
    ///
    /// # Errors
    ///
    /// Returns an error when the source ID is empty or any bounded field is
    /// too large.
    pub fn new(
        kind: HomeSourceKind,
        source_id: impl Into<String>,
        status: HomeSourceStatus,
    ) -> Result<Self, TransportError> {
        let source = Self {
            kind,
            source_id: source_id.into(),
            display_name: None,
            status,
            detail: None,
        };
        source.validate()?;
        Ok(source)
    }

    /// Adds a bounded display name.
    #[must_use]
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    /// Adds bounded non-secret source detail.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Returns the source class.
    #[must_use]
    pub const fn kind(&self) -> HomeSourceKind {
        self.kind
    }

    /// Returns the stable, non-secret source identifier.
    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the optional display name.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// Returns source availability metadata.
    #[must_use]
    pub const fn status(&self) -> HomeSourceStatus {
        self.status
    }

    /// Returns optional bounded source detail.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text("home source ID", &self.source_id, MAX_HOME_SOURCE_ID_BYTES)?;
        validate_optional_text(
            "home source display name",
            self.display_name.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home source detail",
            self.detail.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )
    }
}

/// A stable Home media/recommendation card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeCard {
    card_id: String,
    title: String,
    subtitle: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    artwork_key: Option<String>,
    quality_label: Option<String>,
    duration_ms: Option<u32>,
    route_id: Option<String>,
    is_favorite: bool,
    source: HomeSourceMetadata,
    status: HomeStatus,
}

impl HomeCard {
    /// Creates a card with its required stable identity, title, source, and
    /// status. Optional presentation fields can be added with `with_*`.
    ///
    /// # Errors
    ///
    /// Returns an error when a required value or nested metadata is invalid.
    pub fn new(
        card_id: impl Into<String>,
        title: impl Into<String>,
        source: HomeSourceMetadata,
        status: HomeStatus,
    ) -> Result<Self, TransportError> {
        let card = Self {
            card_id: card_id.into(),
            title: title.into(),
            subtitle: None,
            artist: None,
            album: None,
            artwork_key: None,
            quality_label: None,
            duration_ms: None,
            route_id: None,
            is_favorite: false,
            source,
            status,
        };
        card.validate()?;
        Ok(card)
    }

    /// Adds an optional subtitle.
    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Adds an optional artist.
    #[must_use]
    pub fn with_artist(mut self, artist: impl Into<String>) -> Self {
        self.artist = Some(artist.into());
        self
    }

    /// Adds an optional album.
    #[must_use]
    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    /// Adds an artwork cache key, never a filesystem path.
    #[must_use]
    pub fn with_artwork_key(mut self, artwork_key: impl Into<String>) -> Self {
        self.artwork_key = Some(artwork_key.into());
        self
    }

    /// Adds an optional source quality label.
    #[must_use]
    pub fn with_quality_label(mut self, quality_label: impl Into<String>) -> Self {
        self.quality_label = Some(quality_label.into());
        self
    }

    /// Adds an optional duration in milliseconds.
    #[must_use]
    pub const fn with_duration_ms(mut self, duration_ms: u32) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    /// Adds the route used by an open-card command.
    #[must_use]
    pub fn with_route_id(mut self, route_id: impl Into<String>) -> Self {
        self.route_id = Some(route_id.into());
        self
    }

    /// Sets the projected favorite marker.
    #[must_use]
    pub const fn with_favorite(mut self, is_favorite: bool) -> Self {
        self.is_favorite = is_favorite;
        self
    }

    /// Returns the stable card identity used for reconciliation and routing.
    #[must_use]
    pub fn card_id(&self) -> &str {
        &self.card_id
    }

    /// Returns the card title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns optional card subtitle.
    #[must_use]
    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// Returns optional artist.
    #[must_use]
    pub fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    /// Returns optional album.
    #[must_use]
    pub fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    /// Returns optional artwork cache key.
    #[must_use]
    pub fn artwork_key(&self) -> Option<&str> {
        self.artwork_key.as_deref()
    }

    /// Returns optional source quality label.
    #[must_use]
    pub fn quality_label(&self) -> Option<&str> {
        self.quality_label.as_deref()
    }

    /// Returns optional duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u32> {
        self.duration_ms
    }

    /// Returns the optional route identifier.
    #[must_use]
    pub fn route_id(&self) -> Option<&str> {
        self.route_id.as_deref()
    }

    /// Returns the projected favorite marker.
    #[must_use]
    pub const fn is_favorite(&self) -> bool {
        self.is_favorite
    }

    /// Returns source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns card status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text("home card ID", &self.card_id, MAX_HOME_CARD_ID_BYTES)?;
        validate_text("home card title", &self.title, MAX_HOME_TEXT_BYTES)?;
        validate_optional_text(
            "home card subtitle",
            self.subtitle.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home card artist",
            self.artist.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home card album",
            self.album.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home card artwork key",
            self.artwork_key.as_deref(),
            MAX_HOME_ASSET_KEY_BYTES,
        )?;
        validate_optional_text(
            "home card quality label",
            self.quality_label.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home card route ID",
            self.route_id.as_deref(),
            MAX_HOME_ROUTE_ID_BYTES,
        )?;
        self.source.validate()?;
        self.status.validate()
    }
}

/// One canonical Home section and its source/status metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSection {
    kind: HomeSectionKind,
    source: HomeSourceMetadata,
    status: HomeStatus,
    cards: Vec<HomeCard>,
}

impl HomeSection {
    /// Creates a bounded section.
    ///
    /// # Errors
    ///
    /// Returns an error when nested metadata, cards, or card identities are
    /// invalid or exceed their bounds.
    pub fn new(
        kind: HomeSectionKind,
        source: HomeSourceMetadata,
        status: HomeStatus,
        cards: impl Into<Vec<HomeCard>>,
    ) -> Result<Self, TransportError> {
        let section = Self {
            kind,
            source,
            status,
            cards: cards.into(),
        };
        section.validate()?;
        Ok(section)
    }

    /// Returns the canonical section kind.
    #[must_use]
    pub const fn kind(&self) -> HomeSectionKind {
        self.kind
    }

    /// Returns section source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns section status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    /// Returns cards in deterministic source order.
    #[must_use]
    pub fn cards(&self) -> &[HomeCard] {
        &self.cards
    }

    fn validate(&self) -> Result<(), TransportError> {
        self.source.validate()?;
        self.status.validate()?;
        validate_cards(&self.cards, "home section cards")
    }
}

/// A bounded quick-entry card routed by stable identity and route ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeQuickEntry {
    card_id: String,
    title: String,
    subtitle: Option<String>,
    icon_ref: Option<String>,
    route_id: String,
    count: Option<u32>,
    customizable: bool,
    source: HomeSourceMetadata,
    status: HomeStatus,
}

impl HomeQuickEntry {
    /// Creates a quick entry with its stable identity and route.
    ///
    /// # Errors
    ///
    /// Returns an error when a required value, nested metadata, or route is
    /// invalid or exceeds its bound.
    pub fn new(
        card_id: impl Into<String>,
        title: impl Into<String>,
        route_id: impl Into<String>,
        source: HomeSourceMetadata,
        status: HomeStatus,
    ) -> Result<Self, TransportError> {
        let entry = Self {
            card_id: card_id.into(),
            title: title.into(),
            subtitle: None,
            icon_ref: None,
            route_id: route_id.into(),
            count: None,
            customizable: false,
            source,
            status,
        };
        entry.validate()?;
        Ok(entry)
    }

    /// Adds a display subtitle.
    #[must_use]
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Adds an icon reference or glyph identifier.
    #[must_use]
    pub fn with_icon_ref(mut self, icon_ref: impl Into<String>) -> Self {
        self.icon_ref = Some(icon_ref.into());
        self
    }

    /// Adds a bounded count displayed by the host.
    #[must_use]
    pub const fn with_count(mut self, count: u32) -> Self {
        self.count = Some(count);
        self
    }

    /// Marks this entry as a host-customizable slot.
    #[must_use]
    pub const fn with_customizable(mut self, customizable: bool) -> Self {
        self.customizable = customizable;
        self
    }

    /// Returns the stable quick-entry identity.
    #[must_use]
    pub fn card_id(&self) -> &str {
        &self.card_id
    }

    /// Returns the title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns optional subtitle.
    #[must_use]
    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// Returns optional icon reference.
    #[must_use]
    pub fn icon_ref(&self) -> Option<&str> {
        self.icon_ref.as_deref()
    }

    /// Returns the route identifier.
    #[must_use]
    pub fn route_id(&self) -> &str {
        &self.route_id
    }

    /// Returns optional count.
    #[must_use]
    pub const fn count(&self) -> Option<u32> {
        self.count
    }

    /// Returns whether the host may customize this entry.
    #[must_use]
    pub const fn customizable(&self) -> bool {
        self.customizable
    }

    /// Returns source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text("home quick-entry ID", &self.card_id, MAX_HOME_CARD_ID_BYTES)?;
        validate_text("home quick-entry title", &self.title, MAX_HOME_TEXT_BYTES)?;
        validate_optional_text(
            "home quick-entry subtitle",
            self.subtitle.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home quick-entry icon ref",
            self.icon_ref.as_deref(),
            MAX_HOME_ASSET_KEY_BYTES,
        )?;
        validate_text(
            "home quick-entry route ID",
            &self.route_id,
            MAX_HOME_ROUTE_ID_BYTES,
        )?;
        self.source.validate()?;
        self.status.validate()
    }
}

/// An optional discover/recommendation card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeRecommendation {
    card_id: String,
    title: String,
    description: Option<String>,
    artwork_key: Option<String>,
    route_id: String,
    source: HomeSourceMetadata,
    status: HomeStatus,
}

impl HomeRecommendation {
    /// Creates a recommendation with a stable identity and route.
    ///
    /// # Errors
    ///
    /// Returns an error when a required value, nested metadata, or route is
    /// invalid or exceeds its bound.
    pub fn new(
        card_id: impl Into<String>,
        title: impl Into<String>,
        route_id: impl Into<String>,
        source: HomeSourceMetadata,
        status: HomeStatus,
    ) -> Result<Self, TransportError> {
        let recommendation = Self {
            card_id: card_id.into(),
            title: title.into(),
            description: None,
            artwork_key: None,
            route_id: route_id.into(),
            source,
            status,
        };
        recommendation.validate()?;
        Ok(recommendation)
    }

    /// Adds optional recommendation description text.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds an artwork cache key.
    #[must_use]
    pub fn with_artwork_key(mut self, artwork_key: impl Into<String>) -> Self {
        self.artwork_key = Some(artwork_key.into());
        self
    }

    /// Returns the stable recommendation identity.
    #[must_use]
    pub fn card_id(&self) -> &str {
        &self.card_id
    }

    /// Returns the title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns optional description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns optional artwork cache key.
    #[must_use]
    pub fn artwork_key(&self) -> Option<&str> {
        self.artwork_key.as_deref()
    }

    /// Returns the route identifier.
    #[must_use]
    pub fn route_id(&self) -> &str {
        &self.route_id
    }

    /// Returns source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    fn validate(&self) -> Result<(), TransportError> {
        validate_text(
            "home recommendation ID",
            &self.card_id,
            MAX_HOME_CARD_ID_BYTES,
        )?;
        validate_text(
            "home recommendation title",
            &self.title,
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home recommendation description",
            self.description.as_deref(),
            MAX_HOME_TEXT_BYTES,
        )?;
        validate_optional_text(
            "home recommendation artwork key",
            self.artwork_key.as_deref(),
            MAX_HOME_ASSET_KEY_BYTES,
        )?;
        validate_text(
            "home recommendation route ID",
            &self.route_id,
            MAX_HOME_ROUTE_ID_BYTES,
        )?;
        self.source.validate()?;
        self.status.validate()
    }
}

/// The optional discover/recommendation set on a Home snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeRecommendationSet {
    source: HomeSourceMetadata,
    status: HomeStatus,
    cards: Vec<HomeRecommendation>,
}

impl HomeRecommendationSet {
    /// Creates a bounded recommendation set.
    ///
    /// # Errors
    ///
    /// Returns an error when nested metadata, cards, or card identities are
    /// invalid or exceed their bounds.
    pub fn new(
        source: HomeSourceMetadata,
        status: HomeStatus,
        cards: impl Into<Vec<HomeRecommendation>>,
    ) -> Result<Self, TransportError> {
        let set = Self {
            source,
            status,
            cards: cards.into(),
        };
        set.validate()?;
        Ok(set)
    }

    /// Returns source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns set status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    /// Returns recommendation cards in deterministic source order.
    #[must_use]
    pub fn cards(&self) -> &[HomeRecommendation] {
        &self.cards
    }

    fn validate(&self) -> Result<(), TransportError> {
        self.source.validate()?;
        self.status.validate()?;
        validate_recommendations(&self.cards)
    }
}

/// A complete Core-backed Home value projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeSnapshot {
    projection_version: u32,
    observed_event_sequence: u64,
    source: HomeSourceMetadata,
    status: HomeStatus,
    sections: Vec<HomeSection>,
    quick_entries: Vec<HomeQuickEntry>,
    discover: Option<HomeRecommendationSet>,
}

impl HomeSnapshot {
    /// Creates a Home snapshot. Querying, ordering, and aggregation are owned
    /// by Core callers; this value only validates and maps supplied values.
    ///
    /// # Errors
    ///
    /// Returns an error when sections, quick entries, recommendations, or
    /// nested metadata exceed their bounds or contain duplicate identities.
    pub fn new(
        observed_event_sequence: u64,
        source: HomeSourceMetadata,
        status: HomeStatus,
        sections: impl Into<Vec<HomeSection>>,
        quick_entries: impl Into<Vec<HomeQuickEntry>>,
        discover: Option<HomeRecommendationSet>,
    ) -> Result<Self, TransportError> {
        let snapshot = Self {
            projection_version: HOME_PROJECTION_VERSION,
            observed_event_sequence,
            source,
            status,
            sections: sections.into(),
            quick_entries: quick_entries.into(),
            discover,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Returns the Home projection version.
    #[must_use]
    pub const fn projection_version(&self) -> u32 {
        self.projection_version
    }

    /// Returns the event sequence observed when this snapshot was assembled.
    #[must_use]
    pub const fn observed_event_sequence(&self) -> u64 {
        self.observed_event_sequence
    }

    /// Returns aggregate source metadata.
    #[must_use]
    pub const fn source(&self) -> &HomeSourceMetadata {
        &self.source
    }

    /// Returns aggregate status metadata.
    #[must_use]
    pub const fn status(&self) -> &HomeStatus {
        &self.status
    }

    /// Returns aggregate display state.
    #[must_use]
    pub const fn state(&self) -> HomeState {
        self.status.state()
    }

    /// Returns canonical sections.
    #[must_use]
    pub fn sections(&self) -> &[HomeSection] {
        &self.sections
    }

    /// Returns quick entries.
    #[must_use]
    pub fn quick_entries(&self) -> &[HomeQuickEntry] {
        &self.quick_entries
    }

    /// Returns the optional discover/recommendation set.
    #[must_use]
    pub fn discover(&self) -> Option<&HomeRecommendationSet> {
        self.discover.as_ref()
    }

    /// Encodes the snapshot message under the ordinary transport limit.
    ///
    /// # Errors
    ///
    /// Returns an error when validation fails or the encoded message exceeds
    /// the ordinary transport limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        ensure_message_size(self.encode_message()?)
    }

    /// Encodes the snapshot as an event payload under the smaller event cap.
    ///
    /// # Errors
    ///
    /// Returns an error when validation fails or the encoded event exceeds
    /// `MAX_EVENT_BYTES`.
    pub fn encode_event(&self) -> Result<Vec<u8>, TransportError> {
        let output = self.encode()?;
        ensure_event_size(output)
    }

    /// Decodes a snapshot message and rejects unknown fields and enum values.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed input, unknown fields, invalid enum or
    /// projection values, duplicate fields, or exceeded limits.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        decode_home_snapshot(input, MAX_MESSAGE_BYTES)
    }

    /// Decodes a snapshot event payload under the event cap.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed input, unknown fields, invalid enum or
    /// projection values, duplicate fields, or exceeded event limits.
    pub fn decode_event(input: &[u8]) -> Result<Self, TransportError> {
        decode_home_snapshot(input, MAX_EVENT_BYTES)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.projection_version != HOME_PROJECTION_VERSION {
            return Err(TransportError::UnsupportedProjection {
                expected: HOME_PROJECTION_VERSION,
                actual: self.projection_version,
            });
        }
        self.source.validate()?;
        self.status.validate()?;
        if self.sections.len() > MAX_HOME_SECTIONS {
            return Err(TransportError::FieldTooLarge {
                field: "home sections",
                max: MAX_HOME_SECTIONS,
                actual: self.sections.len(),
            });
        }
        let mut section_kinds = Vec::with_capacity(self.sections.len());
        for section in &self.sections {
            section.validate()?;
            if section_kinds.contains(&section.kind) {
                return Err(TransportError::InvalidValue("duplicate home section kind"));
            }
            section_kinds.push(section.kind);
        }
        if self.quick_entries.len() > MAX_HOME_QUICK_ENTRIES {
            return Err(TransportError::FieldTooLarge {
                field: "home quick entries",
                max: MAX_HOME_QUICK_ENTRIES,
                actual: self.quick_entries.len(),
            });
        }
        validate_unique_ids(
            self.quick_entries.iter().map(HomeQuickEntry::card_id),
            "home quick-entry ID",
        )?;
        for entry in &self.quick_entries {
            entry.validate()?;
        }
        if let Some(discover) = &self.discover {
            discover.validate()?;
        }
        Ok(())
    }

    fn encode_message(&self) -> Result<Vec<u8>, TransportError> {
        let mut output = Vec::with_capacity(1024);
        put_varint_field(
            &mut output,
            FIELD_HOME_PROJECTION_VERSION,
            u64::from(self.projection_version),
        );
        put_varint_field(
            &mut output,
            FIELD_HOME_OBSERVED_EVENT_SEQUENCE,
            self.observed_event_sequence,
        );
        put_message_field(
            &mut output,
            FIELD_HOME_SOURCE,
            &encode_source(&self.source)?,
        );
        put_message_field(
            &mut output,
            FIELD_HOME_STATUS,
            &encode_status(&self.status)?,
        );
        for section in &self.sections {
            put_message_field(&mut output, FIELD_HOME_SECTIONS, &encode_section(section)?);
        }
        for entry in &self.quick_entries {
            put_message_field(
                &mut output,
                FIELD_HOME_QUICK_ENTRIES,
                &encode_quick_entry(entry)?,
            );
        }
        if let Some(discover) = &self.discover {
            put_message_field(
                &mut output,
                FIELD_HOME_DISCOVER,
                &encode_recommendation_set(discover)?,
            );
        }
        Ok(output)
    }
}

/// A semantic command routed by a host to the Core facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeCommand {
    /// Opens a quick entry route.
    OpenQuickEntry { card_id: String, route_id: String },
    /// Starts playback for a stable media card identity.
    PlayTrack { card_id: String },
    /// Opens a recommendation route.
    OpenRecommendation { card_id: String, route_id: String },
    /// Opens the full recent-played section.
    OpenAllRecent { section_id: String },
    /// Opens host customization for Home.
    CustomizeHome,
    /// Routes a bounded global search query.
    Search { query: String },
    /// Toggles the favorite state for a stable card identity.
    ToggleFavorite { card_id: String },
    /// Opens a reviewed extension route.
    OpenExtension { card_id: String, route_id: String },
}

/// A bounded Home command request with an observed snapshot sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeCommandRequest {
    command_version: u32,
    command_id: String,
    observed_event_sequence: u64,
    command: HomeCommand,
}

impl HomeCommandRequest {
    /// Creates a command DTO. It does not execute or enqueue the command.
    ///
    /// # Errors
    ///
    /// Returns an error when the command ID, route target, query, or command
    /// version is invalid or exceeds its bound.
    pub fn new(
        command_id: impl Into<String>,
        observed_event_sequence: u64,
        command: HomeCommand,
    ) -> Result<Self, TransportError> {
        let request = Self {
            command_version: HOME_PROJECTION_VERSION,
            command_id: command_id.into(),
            observed_event_sequence,
            command,
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the command DTO version.
    #[must_use]
    pub const fn command_version(&self) -> u32 {
        self.command_version
    }

    /// Returns the caller correlation identifier.
    #[must_use]
    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    /// Returns the snapshot sequence observed by the caller.
    #[must_use]
    pub const fn observed_event_sequence(&self) -> u64 {
        self.observed_event_sequence
    }

    /// Returns the semantic command.
    #[must_use]
    pub const fn command(&self) -> &HomeCommand {
        &self.command
    }

    /// Encodes this command DTO under the ordinary transport limit.
    ///
    /// # Errors
    ///
    /// Returns an error when validation fails or the encoded command exceeds
    /// the ordinary transport limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        let mut output = Vec::with_capacity(128);
        put_varint_field(
            &mut output,
            FIELD_HOME_COMMAND_VERSION,
            u64::from(self.command_version),
        );
        put_bytes_field(
            &mut output,
            FIELD_HOME_COMMAND_ID,
            self.command_id.as_bytes(),
        );
        put_varint_field(
            &mut output,
            FIELD_HOME_COMMAND_OBSERVED_EVENT_SEQUENCE,
            self.observed_event_sequence,
        );
        encode_command(&mut output, &self.command)?;
        ensure_message_size(output)
    }

    /// Decodes this command DTO and rejects unknown oneof arms.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed input, unknown fields or oneof arms,
    /// invalid values, duplicate fields, or exceeded limits.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: input.len(),
            });
        }
        decode_home_command(input)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.command_version != HOME_PROJECTION_VERSION {
            return Err(TransportError::UnsupportedProjection {
                expected: HOME_PROJECTION_VERSION,
                actual: self.command_version,
            });
        }
        validate_text(
            "home command ID",
            &self.command_id,
            MAX_HOME_COMMAND_ID_BYTES,
        )?;
        validate_command(&self.command)
    }
}

/// A public-safe request envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiRequest {
    schema_version: u32,
    request_id: String,
    body: RequestBody,
}

impl FfiRequest {
    /// Creates a request envelope after validating its contract fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the request ID or extension body is invalid.
    pub fn new(request_id: impl Into<String>, body: RequestBody) -> Result<Self, TransportError> {
        let request = Self {
            schema_version: SCHEMA_MAJOR,
            request_id: request_id.into(),
            body,
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the schema major carried by this envelope.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the caller-generated correlation identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected oneof body arm.
    #[must_use]
    pub const fn body(&self) -> &RequestBody {
        &self.body
    }

    /// Encodes this envelope using the protobuf wire format defined by the
    /// public bootstrap schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope is invalid or exceeds the message
    /// limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        let mut output = Vec::with_capacity(64);
        put_varint_field(
            &mut output,
            FIELD_SCHEMA_VERSION,
            u64::from(self.schema_version),
        );
        put_bytes_field(&mut output, FIELD_REQUEST_ID, self.request_id.as_bytes());
        match &self.body {
            RequestBody::Ping => put_message_field(&mut output, FIELD_PING, &[]),
            RequestBody::Extension(extension) => {
                let mut message = Vec::with_capacity(64 + extension.payload.len());
                put_bytes_field(&mut message, 1, extension.namespace.as_bytes());
                put_bytes_field(&mut message, 2, extension.schema_version.as_bytes());
                put_bytes_field(&mut message, 3, &extension.payload);
                put_message_field(&mut output, FIELD_EXTENSION, &message);
            }
        }
        ensure_message_size(output)
    }

    /// Decodes and validates one protobuf envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed wire data, unknown fields, unsupported
    /// schema versions, invalid oneof arms, or exceeded bounds.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: input.len(),
            });
        }

        let mut cursor = Cursor::new(input);
        let mut schema_version = None;
        let mut request_id = None;
        let mut body = None;
        while !cursor.is_empty() {
            let (field, wire_type) = cursor.key()?;
            match field {
                FIELD_SCHEMA_VERSION if wire_type == WireType::Varint => {
                    if schema_version.is_some() {
                        return Err(TransportError::DuplicateField("schema_version"));
                    }
                    schema_version = Some(cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("schema_version exceeds uint32")
                    })?);
                }
                FIELD_REQUEST_ID if wire_type == WireType::LengthDelimited => {
                    if request_id.is_some() {
                        return Err(TransportError::DuplicateField("request_id"));
                    }
                    request_id = Some(cursor.utf8("request_id", MAX_REQUEST_ID_BYTES)?);
                }
                FIELD_PING if wire_type == WireType::LengthDelimited => {
                    select_body(&mut body, RequestBody::Ping)?;
                    let payload = cursor.bytes("ping", 0)?;
                    if !payload.is_empty() {
                        return Err(TransportError::InvalidValue("ping must be empty"));
                    }
                }
                FIELD_EXTENSION if wire_type == WireType::LengthDelimited => {
                    let payload = cursor.bytes("extension", MAX_EXTENSION_PAYLOAD_BYTES + 256)?;
                    if body.is_some() {
                        return Err(TransportError::InvalidValue("multiple body oneof arms"));
                    }
                    select_body(
                        &mut body,
                        RequestBody::Extension(decode_extension(&payload)?),
                    )?;
                }
                FIELD_PING | FIELD_EXTENSION => {
                    return Err(TransportError::InvalidWireType { field });
                }
                _ => return Err(TransportError::UnknownField { field }),
            }
        }

        let request = Self {
            schema_version: schema_version.ok_or(TransportError::MissingField("schema_version"))?,
            request_id: request_id.ok_or(TransportError::MissingField("request_id"))?,
            body: body.ok_or(TransportError::MissingOneof("body"))?,
        };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.schema_version != SCHEMA_MAJOR {
            return Err(TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: self.schema_version,
            });
        }
        validate_text("request_id", &self.request_id, MAX_REQUEST_ID_BYTES)?;
        if let RequestBody::Extension(extension) = &self.body {
            extension.validate()?;
        }
        Ok(())
    }
}

/// A public-safe response envelope paired with one request correlation ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiResponse {
    schema_version: u32,
    request_id: String,
    body: ResponseBody,
}

impl FfiResponse {
    /// Creates a response envelope after validating its contract fields.
    ///
    /// # Errors
    ///
    /// Returns an error when the request ID or error body is invalid.
    pub fn new(request_id: impl Into<String>, body: ResponseBody) -> Result<Self, TransportError> {
        let response = Self {
            schema_version: SCHEMA_MAJOR,
            request_id: request_id.into(),
            body,
        };
        response.validate()?;
        Ok(response)
    }

    /// Returns the schema major carried by this envelope.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the request correlation identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected oneof body arm.
    #[must_use]
    pub const fn body(&self) -> &ResponseBody {
        &self.body
    }

    /// Encodes this response using the protobuf wire format.
    ///
    /// # Errors
    ///
    /// Returns an error when the envelope is invalid or exceeds the message
    /// limit.
    pub fn encode(&self) -> Result<Vec<u8>, TransportError> {
        self.validate()?;
        let mut output = Vec::with_capacity(64);
        put_varint_field(
            &mut output,
            FIELD_SCHEMA_VERSION,
            u64::from(self.schema_version),
        );
        put_bytes_field(&mut output, FIELD_REQUEST_ID, self.request_id.as_bytes());
        match &self.body {
            ResponseBody::Pong => put_message_field(&mut output, FIELD_PONG, &[]),
            ResponseBody::Error(error) => {
                let mut message = Vec::with_capacity(64 + error.message.len());
                put_bytes_field(&mut message, 1, error.code.as_bytes());
                put_bytes_field(&mut message, 2, error.message.as_bytes());
                put_varint_field(&mut message, 3, u64::from(error.retryable));
                put_message_field(&mut output, FIELD_ERROR, &message);
            }
        }
        ensure_message_size(output)
    }

    /// Decodes and validates one protobuf response envelope.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed wire data, unknown fields, unsupported
    /// schema versions, invalid oneof arms, or exceeded bounds.
    pub fn decode(input: &[u8]) -> Result<Self, TransportError> {
        if input.len() > MAX_MESSAGE_BYTES {
            return Err(TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: input.len(),
            });
        }

        let mut cursor = Cursor::new(input);
        let mut schema_version = None;
        let mut request_id = None;
        let mut body = None;
        while !cursor.is_empty() {
            let (field, wire_type) = cursor.key()?;
            match field {
                FIELD_SCHEMA_VERSION if wire_type == WireType::Varint => {
                    if schema_version.is_some() {
                        return Err(TransportError::DuplicateField("schema_version"));
                    }
                    schema_version = Some(cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("schema_version exceeds uint32")
                    })?);
                }
                FIELD_REQUEST_ID if wire_type == WireType::LengthDelimited => {
                    if request_id.is_some() {
                        return Err(TransportError::DuplicateField("request_id"));
                    }
                    request_id = Some(cursor.utf8("request_id", MAX_REQUEST_ID_BYTES)?);
                }
                FIELD_PONG if wire_type == WireType::LengthDelimited => {
                    select_response_body(&mut body, ResponseBody::Pong)?;
                    let payload = cursor.bytes("pong", 0)?;
                    if !payload.is_empty() {
                        return Err(TransportError::InvalidValue("pong must be empty"));
                    }
                }
                FIELD_ERROR if wire_type == WireType::LengthDelimited => {
                    let payload = cursor.bytes("error", MAX_ERROR_MESSAGE_BYTES + 256)?;
                    if body.is_some() {
                        return Err(TransportError::InvalidValue(
                            "multiple response body oneof arms",
                        ));
                    }
                    select_response_body(&mut body, ResponseBody::Error(decode_error(&payload)?))?;
                }
                FIELD_PONG | FIELD_ERROR => {
                    return Err(TransportError::InvalidWireType { field });
                }
                _ => return Err(TransportError::UnknownField { field }),
            }
        }

        let response = Self {
            schema_version: schema_version.ok_or(TransportError::MissingField("schema_version"))?,
            request_id: request_id.ok_or(TransportError::MissingField("request_id"))?,
            body: body.ok_or(TransportError::MissingOneof("body"))?,
        };
        response.validate()?;
        Ok(response)
    }

    fn validate(&self) -> Result<(), TransportError> {
        if self.schema_version != SCHEMA_MAJOR {
            return Err(TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: self.schema_version,
            });
        }
        validate_text("request_id", &self.request_id, MAX_REQUEST_ID_BYTES)?;
        if let ResponseBody::Error(error) = &self.body {
            validate_text("error code", &error.code, MAX_ERROR_CODE_BYTES)?;
            validate_text("error message", &error.message, MAX_ERROR_MESSAGE_BYTES)?;
        }
        Ok(())
    }
}

/// Errors returned when a transport envelope cannot be accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The complete wire message exceeded the ordinary envelope limit.
    MessageTooLarge { max: usize, actual: usize },
    /// A Home event payload exceeded the event envelope limit.
    EventTooLarge { max: usize, actual: usize },
    /// A text or bytes field exceeded its declared limit.
    FieldTooLarge {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    /// The schema major is not understood by this client.
    UnsupportedSchema { expected: u32, actual: u32 },
    /// A known projection or command version is not understood by this client.
    UnsupportedProjection { expected: u32, actual: u32 },
    /// A required field was absent.
    MissingField(&'static str),
    /// The required body oneof was absent.
    MissingOneof(&'static str),
    /// The same singular field occurred more than once.
    DuplicateField(&'static str),
    /// A field number was not part of the closed public schema.
    UnknownField { field: u32 },
    /// A known field used an invalid protobuf wire type.
    InvalidWireType { field: u32 },
    /// A scalar or nested message contained an invalid value.
    InvalidValue(&'static str),
    /// An enum value is not part of the closed public contract.
    InvalidEnum { field: &'static str, value: u32 },
    /// A length-delimited value was malformed or truncated.
    Malformed(&'static str),
}

impl core::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MessageTooLarge { max, actual } => {
                write!(formatter, "message is {actual} bytes; limit is {max}")
            }
            Self::EventTooLarge { max, actual } => {
                write!(formatter, "event is {actual} bytes; limit is {max}")
            }
            Self::FieldTooLarge { field, max, actual } => {
                write!(formatter, "{field} is {actual} bytes; limit is {max}")
            }
            Self::UnsupportedSchema { expected, actual } => {
                write!(
                    formatter,
                    "schema major {actual} is unsupported; expected {expected}"
                )
            }
            Self::UnsupportedProjection { expected, actual } => {
                write!(
                    formatter,
                    "projection version {actual} is unsupported; expected {expected}"
                )
            }
            Self::MissingField(field) => write!(formatter, "missing required field {field}"),
            Self::MissingOneof(field) => write!(formatter, "missing required oneof {field}"),
            Self::DuplicateField(field) => write!(formatter, "duplicate singular field {field}"),
            Self::UnknownField { field } => write!(formatter, "unknown field {field}"),
            Self::InvalidWireType { field } => {
                write!(formatter, "invalid wire type for field {field}")
            }
            Self::InvalidValue(value) | Self::Malformed(value) => formatter.write_str(value),
            Self::InvalidEnum { field, value } => {
                write!(formatter, "invalid {field} value {value}")
            }
        }
    }
}

impl std::error::Error for TransportError {}

fn parse_projection_version(value: &str) -> Result<u32, TransportError> {
    value
        .parse()
        .map_err(|_| TransportError::InvalidValue("Home extension schema version"))
}

fn encode_status(status: &HomeStatus) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(32);
    put_varint_field(&mut output, 1, status.state as u64);
    if let Some(message) = &status.message {
        put_bytes_field(&mut output, 2, message.as_bytes());
    }
    put_varint_field(&mut output, 3, u64::from(status.retryable));
    ensure_nested_size(output, MAX_HOME_STATUS_MESSAGE_BYTES, "home status")
}

fn decode_status(input: &[u8]) -> Result<HomeStatus, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut state = None;
    let mut message = None;
    let mut retryable = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::Varint => {
                if state.is_some() {
                    return Err(TransportError::DuplicateField("home status state"));
                }
                state = Some(HomeState::from_wire(cursor.varint()?.try_into().map_err(
                    |_| TransportError::InvalidValue("home status state exceeds uint32"),
                )?)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if message.is_some() {
                    return Err(TransportError::DuplicateField("home status message"));
                }
                message = Some(cursor.utf8("home status message", MAX_HOME_TEXT_BYTES)?);
            }
            3 if wire_type == WireType::Varint => {
                if retryable.is_some() {
                    return Err(TransportError::DuplicateField("home status retryable"));
                }
                retryable = Some(cursor.varint()? != 0);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let status = HomeStatus {
        state: state.ok_or(TransportError::MissingField("home status state"))?,
        message,
        retryable: retryable.ok_or(TransportError::MissingField("home status retryable"))?,
    };
    status.validate()?;
    Ok(status)
}

fn encode_source(source: &HomeSourceMetadata) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(64);
    put_varint_field(&mut output, 1, source.kind as u64);
    put_bytes_field(&mut output, 2, source.source_id.as_bytes());
    if let Some(display_name) = &source.display_name {
        put_bytes_field(&mut output, 3, display_name.as_bytes());
    }
    put_varint_field(&mut output, 4, source.status as u64);
    if let Some(detail) = &source.detail {
        put_bytes_field(&mut output, 5, detail.as_bytes());
    }
    ensure_nested_size(output, MAX_HOME_SOURCE_MESSAGE_BYTES, "home source")
}

fn decode_source(input: &[u8]) -> Result<HomeSourceMetadata, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut kind = None;
    let mut source_id = None;
    let mut display_name = None;
    let mut status = None;
    let mut detail = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::Varint => {
                if kind.is_some() {
                    return Err(TransportError::DuplicateField("home source kind"));
                }
                kind = Some(HomeSourceKind::from_wire(
                    cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("home source kind exceeds uint32")
                    })?,
                )?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if source_id.is_some() {
                    return Err(TransportError::DuplicateField("home source ID"));
                }
                source_id = Some(cursor.utf8("home source ID", MAX_HOME_SOURCE_ID_BYTES)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if display_name.is_some() {
                    return Err(TransportError::DuplicateField("home source display name"));
                }
                display_name = Some(cursor.utf8("home source display name", MAX_HOME_TEXT_BYTES)?);
            }
            4 if wire_type == WireType::Varint => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home source status"));
                }
                status = Some(HomeSourceStatus::from_wire(
                    cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("home source status exceeds uint32")
                    })?,
                )?);
            }
            5 if wire_type == WireType::LengthDelimited => {
                if detail.is_some() {
                    return Err(TransportError::DuplicateField("home source detail"));
                }
                detail = Some(cursor.utf8("home source detail", MAX_HOME_TEXT_BYTES)?);
            }
            1..=5 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let source = HomeSourceMetadata {
        kind: kind.ok_or(TransportError::MissingField("home source kind"))?,
        source_id: source_id.ok_or(TransportError::MissingField("home source ID"))?,
        display_name,
        status: status.ok_or(TransportError::MissingField("home source status"))?,
        detail,
    };
    source.validate()?;
    Ok(source)
}

fn encode_card(card: &HomeCard) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(128);
    put_bytes_field(&mut output, 1, card.card_id.as_bytes());
    put_bytes_field(&mut output, 2, card.title.as_bytes());
    put_optional_text_field(&mut output, 3, card.subtitle.as_deref());
    put_optional_text_field(&mut output, 4, card.artist.as_deref());
    put_optional_text_field(&mut output, 5, card.album.as_deref());
    put_optional_text_field(&mut output, 6, card.artwork_key.as_deref());
    put_optional_text_field(&mut output, 7, card.quality_label.as_deref());
    if let Some(duration_ms) = card.duration_ms {
        put_varint_field(&mut output, 8, u64::from(duration_ms));
    }
    put_optional_text_field(&mut output, 9, card.route_id.as_deref());
    put_varint_field(&mut output, 10, u64::from(card.is_favorite));
    put_message_field(&mut output, 11, &encode_source(&card.source)?);
    put_message_field(&mut output, 12, &encode_status(&card.status)?);
    ensure_nested_size(output, MAX_HOME_CARD_MESSAGE_BYTES, "home card")
}

#[allow(clippy::too_many_lines)]
fn decode_card(input: &[u8]) -> Result<HomeCard, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut card_id = None;
    let mut title = None;
    let mut subtitle = None;
    let mut artist = None;
    let mut album = None;
    let mut artwork_key = None;
    let mut quality_label = None;
    let mut duration_ms = None;
    let mut route_id = None;
    let mut is_favorite = None;
    let mut source = None;
    let mut status = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if card_id.is_some() {
                    return Err(TransportError::DuplicateField("home card ID"));
                }
                card_id = Some(cursor.utf8("home card ID", MAX_HOME_CARD_ID_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if title.is_some() {
                    return Err(TransportError::DuplicateField("home card title"));
                }
                title = Some(cursor.utf8("home card title", MAX_HOME_TEXT_BYTES)?);
            }
            3..=7 | 9 if wire_type == WireType::LengthDelimited => {
                let slot = match field {
                    3 => &mut subtitle,
                    4 => &mut artist,
                    5 => &mut album,
                    6 => &mut artwork_key,
                    7 => &mut quality_label,
                    9 => &mut route_id,
                    _ => unreachable!("field is constrained by the match arm"),
                };
                if slot.is_some() {
                    return Err(TransportError::DuplicateField("home card optional field"));
                }
                let (name, max) = match field {
                    3 => ("home card subtitle", MAX_HOME_TEXT_BYTES),
                    4 => ("home card artist", MAX_HOME_TEXT_BYTES),
                    5 => ("home card album", MAX_HOME_TEXT_BYTES),
                    6 => ("home card artwork key", MAX_HOME_ASSET_KEY_BYTES),
                    7 => ("home card quality label", MAX_HOME_TEXT_BYTES),
                    9 => ("home card route ID", MAX_HOME_ROUTE_ID_BYTES),
                    _ => unreachable!("field is constrained by the match arm"),
                };
                *slot = Some(cursor.utf8(name, max)?);
            }
            8 if wire_type == WireType::Varint => {
                if duration_ms.is_some() {
                    return Err(TransportError::DuplicateField("home card duration"));
                }
                duration_ms = Some(cursor.varint()?.try_into().map_err(|_| {
                    TransportError::InvalidValue("home card duration exceeds uint32")
                })?);
            }
            10 if wire_type == WireType::Varint => {
                if is_favorite.is_some() {
                    return Err(TransportError::DuplicateField("home card favorite"));
                }
                is_favorite = Some(cursor.varint()? != 0);
            }
            11 if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField("home card source"));
                }
                source = Some(decode_source(
                    &cursor.bytes("home card source", MAX_HOME_SOURCE_MESSAGE_BYTES)?,
                )?);
            }
            12 if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home card status"));
                }
                status = Some(decode_status(
                    &cursor.bytes("home card status", MAX_HOME_STATUS_MESSAGE_BYTES)?,
                )?);
            }
            1..=12 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let card = HomeCard {
        card_id: card_id.ok_or(TransportError::MissingField("home card ID"))?,
        title: title.ok_or(TransportError::MissingField("home card title"))?,
        subtitle,
        artist,
        album,
        artwork_key,
        quality_label,
        duration_ms,
        route_id,
        is_favorite: is_favorite.ok_or(TransportError::MissingField("home card favorite"))?,
        source: source.ok_or(TransportError::MissingField("home card source"))?,
        status: status.ok_or(TransportError::MissingField("home card status"))?,
    };
    card.validate()?;
    Ok(card)
}

fn encode_section(section: &HomeSection) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(256);
    put_varint_field(&mut output, 1, section.kind as u64);
    put_message_field(&mut output, 2, &encode_source(&section.source)?);
    put_message_field(&mut output, 3, &encode_status(&section.status)?);
    for card in &section.cards {
        put_message_field(&mut output, 4, &encode_card(card)?);
    }
    ensure_nested_size(output, MAX_HOME_SECTION_MESSAGE_BYTES, "home section")
}

fn decode_section(input: &[u8]) -> Result<HomeSection, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut kind = None;
    let mut source = None;
    let mut status = None;
    let mut cards = Vec::new();
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::Varint => {
                if kind.is_some() {
                    return Err(TransportError::DuplicateField("home section kind"));
                }
                kind = Some(HomeSectionKind::from_wire(
                    cursor.varint()?.try_into().map_err(|_| {
                        TransportError::InvalidValue("home section kind exceeds uint32")
                    })?,
                )?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField("home section source"));
                }
                source = Some(decode_source(
                    &cursor.bytes("home section source", MAX_HOME_SOURCE_MESSAGE_BYTES)?,
                )?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home section status"));
                }
                status = Some(decode_status(
                    &cursor.bytes("home section status", MAX_HOME_STATUS_MESSAGE_BYTES)?,
                )?);
            }
            4 if wire_type == WireType::LengthDelimited => {
                if cards.len() >= MAX_HOME_CARDS {
                    return Err(TransportError::FieldTooLarge {
                        field: "home section cards",
                        max: MAX_HOME_CARDS,
                        actual: cards.len() + 1,
                    });
                }
                cards.push(decode_card(
                    &cursor.bytes("home section card", MAX_HOME_CARD_MESSAGE_BYTES)?,
                )?);
            }
            1..=4 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    HomeSection::new(
        kind.ok_or(TransportError::MissingField("home section kind"))?,
        source.ok_or(TransportError::MissingField("home section source"))?,
        status.ok_or(TransportError::MissingField("home section status"))?,
        cards,
    )
}

fn encode_quick_entry(entry: &HomeQuickEntry) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(128);
    put_bytes_field(&mut output, 1, entry.card_id.as_bytes());
    put_bytes_field(&mut output, 2, entry.title.as_bytes());
    put_optional_text_field(&mut output, 3, entry.subtitle.as_deref());
    put_optional_text_field(&mut output, 4, entry.icon_ref.as_deref());
    put_bytes_field(&mut output, 5, entry.route_id.as_bytes());
    if let Some(count) = entry.count {
        put_varint_field(&mut output, 6, u64::from(count));
    }
    put_varint_field(&mut output, 7, u64::from(entry.customizable));
    put_message_field(&mut output, 8, &encode_source(&entry.source)?);
    put_message_field(&mut output, 9, &encode_status(&entry.status)?);
    ensure_nested_size(
        output,
        MAX_HOME_QUICK_ENTRY_MESSAGE_BYTES,
        "home quick entry",
    )
}

fn decode_quick_entry(input: &[u8]) -> Result<HomeQuickEntry, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut card_id = None;
    let mut title = None;
    let mut subtitle = None;
    let mut icon_ref = None;
    let mut route_id = None;
    let mut count = None;
    let mut customizable = None;
    let mut source = None;
    let mut status = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if card_id.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry ID"));
                }
                card_id = Some(cursor.utf8("home quick-entry ID", MAX_HOME_CARD_ID_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if title.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry title"));
                }
                title = Some(cursor.utf8("home quick-entry title", MAX_HOME_TEXT_BYTES)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if subtitle.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry subtitle"));
                }
                subtitle = Some(cursor.utf8("home quick-entry subtitle", MAX_HOME_TEXT_BYTES)?);
            }
            4 if wire_type == WireType::LengthDelimited => {
                if icon_ref.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry icon ref"));
                }
                icon_ref =
                    Some(cursor.utf8("home quick-entry icon ref", MAX_HOME_ASSET_KEY_BYTES)?);
            }
            5 if wire_type == WireType::LengthDelimited => {
                if route_id.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry route ID"));
                }
                route_id = Some(cursor.utf8("home quick-entry route ID", MAX_HOME_ROUTE_ID_BYTES)?);
            }
            6 if wire_type == WireType::Varint => {
                if count.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry count"));
                }
                count = Some(cursor.varint()?.try_into().map_err(|_| {
                    TransportError::InvalidValue("home quick-entry count exceeds uint32")
                })?);
            }
            7 if wire_type == WireType::Varint => {
                if customizable.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home quick-entry customizable",
                    ));
                }
                customizable = Some(cursor.varint()? != 0);
            }
            8 if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry source"));
                }
                source = Some(decode_source(
                    &cursor.bytes("home quick-entry source", MAX_HOME_SOURCE_MESSAGE_BYTES)?,
                )?);
            }
            9 if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home quick-entry status"));
                }
                status = Some(decode_status(
                    &cursor.bytes("home quick-entry status", MAX_HOME_STATUS_MESSAGE_BYTES)?,
                )?);
            }
            1..=9 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let entry = HomeQuickEntry {
        card_id: card_id.ok_or(TransportError::MissingField("home quick-entry ID"))?,
        title: title.ok_or(TransportError::MissingField("home quick-entry title"))?,
        subtitle,
        icon_ref,
        route_id: route_id.ok_or(TransportError::MissingField("home quick-entry route ID"))?,
        count,
        customizable: customizable.ok_or(TransportError::MissingField(
            "home quick-entry customizable",
        ))?,
        source: source.ok_or(TransportError::MissingField("home quick-entry source"))?,
        status: status.ok_or(TransportError::MissingField("home quick-entry status"))?,
    };
    entry.validate()?;
    Ok(entry)
}

fn encode_recommendation(recommendation: &HomeRecommendation) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(128);
    put_bytes_field(&mut output, 1, recommendation.card_id.as_bytes());
    put_bytes_field(&mut output, 2, recommendation.title.as_bytes());
    put_optional_text_field(&mut output, 3, recommendation.description.as_deref());
    put_optional_text_field(&mut output, 4, recommendation.artwork_key.as_deref());
    put_bytes_field(&mut output, 5, recommendation.route_id.as_bytes());
    put_message_field(&mut output, 6, &encode_source(&recommendation.source)?);
    put_message_field(&mut output, 7, &encode_status(&recommendation.status)?);
    ensure_nested_size(
        output,
        MAX_HOME_RECOMMENDATION_MESSAGE_BYTES,
        "home recommendation",
    )
}

fn decode_recommendation(input: &[u8]) -> Result<HomeRecommendation, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut card_id = None;
    let mut title = None;
    let mut description = None;
    let mut artwork_key = None;
    let mut route_id = None;
    let mut source = None;
    let mut status = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if card_id.is_some() {
                    return Err(TransportError::DuplicateField("home recommendation ID"));
                }
                card_id = Some(cursor.utf8("home recommendation ID", MAX_HOME_CARD_ID_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if title.is_some() {
                    return Err(TransportError::DuplicateField("home recommendation title"));
                }
                title = Some(cursor.utf8("home recommendation title", MAX_HOME_TEXT_BYTES)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if description.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home recommendation description",
                    ));
                }
                description =
                    Some(cursor.utf8("home recommendation description", MAX_HOME_TEXT_BYTES)?);
            }
            4 if wire_type == WireType::LengthDelimited => {
                if artwork_key.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home recommendation artwork key",
                    ));
                }
                artwork_key =
                    Some(cursor.utf8("home recommendation artwork key", MAX_HOME_ASSET_KEY_BYTES)?);
            }
            5 if wire_type == WireType::LengthDelimited => {
                if route_id.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home recommendation route ID",
                    ));
                }
                route_id =
                    Some(cursor.utf8("home recommendation route ID", MAX_HOME_ROUTE_ID_BYTES)?);
            }
            6 if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField("home recommendation source"));
                }
                source = Some(decode_source(&cursor.bytes(
                    "home recommendation source",
                    MAX_HOME_SOURCE_MESSAGE_BYTES,
                )?)?);
            }
            7 if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home recommendation status"));
                }
                status = Some(decode_status(&cursor.bytes(
                    "home recommendation status",
                    MAX_HOME_STATUS_MESSAGE_BYTES,
                )?)?);
            }
            1..=7 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let recommendation = HomeRecommendation {
        card_id: card_id.ok_or(TransportError::MissingField("home recommendation ID"))?,
        title: title.ok_or(TransportError::MissingField("home recommendation title"))?,
        description,
        artwork_key,
        route_id: route_id.ok_or(TransportError::MissingField("home recommendation route ID"))?,
        source: source.ok_or(TransportError::MissingField("home recommendation source"))?,
        status: status.ok_or(TransportError::MissingField("home recommendation status"))?,
    };
    recommendation.validate()?;
    Ok(recommendation)
}

fn encode_recommendation_set(set: &HomeRecommendationSet) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(256);
    put_message_field(&mut output, 1, &encode_source(&set.source)?);
    put_message_field(&mut output, 2, &encode_status(&set.status)?);
    for card in &set.cards {
        put_message_field(&mut output, 3, &encode_recommendation(card)?);
    }
    ensure_nested_size(
        output,
        MAX_HOME_RECOMMENDATION_SET_MESSAGE_BYTES,
        "home recommendation set",
    )
}

fn decode_recommendation_set(input: &[u8]) -> Result<HomeRecommendationSet, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut source = None;
    let mut status = None;
    let mut cards = Vec::new();
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home recommendation set source",
                    ));
                }
                source = Some(decode_source(&cursor.bytes(
                    "home recommendation set source",
                    MAX_HOME_SOURCE_MESSAGE_BYTES,
                )?)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home recommendation set status",
                    ));
                }
                status = Some(decode_status(&cursor.bytes(
                    "home recommendation set status",
                    MAX_HOME_STATUS_MESSAGE_BYTES,
                )?)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if cards.len() >= MAX_HOME_CARDS {
                    return Err(TransportError::FieldTooLarge {
                        field: "home recommendations",
                        max: MAX_HOME_CARDS,
                        actual: cards.len() + 1,
                    });
                }
                cards.push(decode_recommendation(&cursor.bytes(
                    "home recommendation",
                    MAX_HOME_RECOMMENDATION_MESSAGE_BYTES,
                )?)?);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    HomeRecommendationSet::new(
        source.ok_or(TransportError::MissingField(
            "home recommendation set source",
        ))?,
        status.ok_or(TransportError::MissingField(
            "home recommendation set status",
        ))?,
        cards,
    )
}

#[allow(clippy::too_many_lines)]
fn decode_home_snapshot(input: &[u8], max: usize) -> Result<HomeSnapshot, TransportError> {
    ensure_input_size(input, max)?;
    let mut cursor = Cursor::new(input);
    let mut projection_version = None;
    let mut observed_event_sequence = None;
    let mut source = None;
    let mut status = None;
    let mut sections = Vec::new();
    let mut quick_entries = Vec::new();
    let mut discover = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            FIELD_HOME_PROJECTION_VERSION if wire_type == WireType::Varint => {
                if projection_version.is_some() {
                    return Err(TransportError::DuplicateField("home projection version"));
                }
                let version: u32 = cursor.varint()?.try_into().map_err(|_| {
                    TransportError::InvalidValue("home projection version exceeds uint32")
                })?;
                projection_version = Some(version);
            }
            FIELD_HOME_OBSERVED_EVENT_SEQUENCE if wire_type == WireType::Varint => {
                if observed_event_sequence.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home observed event sequence",
                    ));
                }
                observed_event_sequence = Some(cursor.varint()?);
            }
            FIELD_HOME_SOURCE if wire_type == WireType::LengthDelimited => {
                if source.is_some() {
                    return Err(TransportError::DuplicateField("home source"));
                }
                source = Some(decode_source(
                    &cursor.bytes("home source", MAX_HOME_SOURCE_MESSAGE_BYTES)?,
                )?);
            }
            FIELD_HOME_STATUS if wire_type == WireType::LengthDelimited => {
                if status.is_some() {
                    return Err(TransportError::DuplicateField("home status"));
                }
                status = Some(decode_status(
                    &cursor.bytes("home status", MAX_HOME_STATUS_MESSAGE_BYTES)?,
                )?);
            }
            FIELD_HOME_SECTIONS if wire_type == WireType::LengthDelimited => {
                if sections.len() >= MAX_HOME_SECTIONS {
                    return Err(TransportError::FieldTooLarge {
                        field: "home sections",
                        max: MAX_HOME_SECTIONS,
                        actual: sections.len() + 1,
                    });
                }
                sections.push(decode_section(
                    &cursor.bytes("home section", MAX_HOME_SECTION_MESSAGE_BYTES)?,
                )?);
            }
            FIELD_HOME_QUICK_ENTRIES if wire_type == WireType::LengthDelimited => {
                if quick_entries.len() >= MAX_HOME_QUICK_ENTRIES {
                    return Err(TransportError::FieldTooLarge {
                        field: "home quick entries",
                        max: MAX_HOME_QUICK_ENTRIES,
                        actual: quick_entries.len() + 1,
                    });
                }
                quick_entries.push(decode_quick_entry(
                    &cursor.bytes("home quick entry", MAX_HOME_QUICK_ENTRY_MESSAGE_BYTES)?,
                )?);
            }
            FIELD_HOME_DISCOVER if wire_type == WireType::LengthDelimited => {
                if discover.is_some() {
                    return Err(TransportError::DuplicateField("home discover"));
                }
                discover = Some(decode_recommendation_set(
                    &cursor.bytes("home discover", MAX_HOME_RECOMMENDATION_SET_MESSAGE_BYTES)?,
                )?);
            }
            FIELD_HOME_PROJECTION_VERSION
            | FIELD_HOME_OBSERVED_EVENT_SEQUENCE
            | FIELD_HOME_SOURCE
            | FIELD_HOME_STATUS
            | FIELD_HOME_SECTIONS
            | FIELD_HOME_QUICK_ENTRIES
            | FIELD_HOME_DISCOVER => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let projection_version =
        projection_version.ok_or(TransportError::MissingField("home projection version"))?;
    if projection_version != HOME_PROJECTION_VERSION {
        return Err(TransportError::UnsupportedProjection {
            expected: HOME_PROJECTION_VERSION,
            actual: projection_version,
        });
    }
    HomeSnapshot::new(
        observed_event_sequence
            .ok_or(TransportError::MissingField("home observed event sequence"))?,
        source.ok_or(TransportError::MissingField("home source"))?,
        status.ok_or(TransportError::MissingField("home status"))?,
        sections,
        quick_entries,
        discover,
    )
}

fn encode_command(output: &mut Vec<u8>, command: &HomeCommand) -> Result<(), TransportError> {
    match command {
        HomeCommand::OpenQuickEntry { card_id, route_id } => {
            let mut message = Vec::with_capacity(64);
            put_bytes_field(&mut message, 1, card_id.as_bytes());
            put_bytes_field(&mut message, 2, route_id.as_bytes());
            put_message_field(
                output,
                FIELD_HOME_OPEN_QUICK_ENTRY,
                &ensure_nested_size(message, MAX_HOME_COMMAND_MESSAGE_BYTES, "home command")?,
            );
        }
        HomeCommand::PlayTrack { card_id } => {
            put_message_field(
                output,
                FIELD_HOME_PLAY_TRACK,
                &encode_single_id_command(card_id, "home command")?,
            );
        }
        HomeCommand::OpenRecommendation { card_id, route_id } => {
            let mut message = Vec::with_capacity(64);
            put_bytes_field(&mut message, 1, card_id.as_bytes());
            put_bytes_field(&mut message, 2, route_id.as_bytes());
            put_message_field(
                output,
                FIELD_HOME_OPEN_RECOMMENDATION,
                &ensure_nested_size(message, MAX_HOME_COMMAND_MESSAGE_BYTES, "home command")?,
            );
        }
        HomeCommand::OpenAllRecent { section_id } => {
            put_message_field(
                output,
                FIELD_HOME_OPEN_ALL_RECENT,
                &encode_single_id_command(section_id, "home command")?,
            );
        }
        HomeCommand::CustomizeHome => put_message_field(output, FIELD_HOME_CUSTOMIZE, &[]),
        HomeCommand::Search { query } => {
            let mut message = Vec::with_capacity(query.len() + 8);
            put_bytes_field(&mut message, 1, query.as_bytes());
            put_message_field(
                output,
                FIELD_HOME_SEARCH,
                &ensure_nested_size(message, MAX_HOME_COMMAND_MESSAGE_BYTES, "home command")?,
            );
        }
        HomeCommand::ToggleFavorite { card_id } => {
            put_message_field(
                output,
                FIELD_HOME_TOGGLE_FAVORITE,
                &encode_single_id_command(card_id, "home command")?,
            );
        }
        HomeCommand::OpenExtension { card_id, route_id } => {
            let mut message = Vec::with_capacity(64);
            put_bytes_field(&mut message, 1, card_id.as_bytes());
            put_bytes_field(&mut message, 2, route_id.as_bytes());
            put_message_field(
                output,
                FIELD_HOME_OPEN_EXTENSION,
                &ensure_nested_size(message, MAX_HOME_COMMAND_MESSAGE_BYTES, "home command")?,
            );
        }
    }
    Ok(())
}

fn encode_single_id_command(id: &str, field: &'static str) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::with_capacity(id.len() + 8);
    put_bytes_field(&mut output, 1, id.as_bytes());
    ensure_nested_size(output, MAX_HOME_COMMAND_MESSAGE_BYTES, field)
}

fn decode_single_id_command(input: &[u8], field: &'static str) -> Result<String, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut id = None;
    while !cursor.is_empty() {
        let (number, wire_type) = cursor.key()?;
        match number {
            1 if wire_type == WireType::LengthDelimited => {
                if id.is_some() {
                    return Err(TransportError::DuplicateField(field));
                }
                id = Some(cursor.utf8(field, MAX_HOME_CARD_ID_BYTES)?);
            }
            1 => return Err(TransportError::InvalidWireType { field: number }),
            _ => return Err(TransportError::UnknownField { field: number }),
        }
    }
    let id = id.ok_or(TransportError::MissingField(field))?;
    validate_text(field, &id, MAX_HOME_CARD_ID_BYTES)?;
    Ok(id)
}

fn decode_route_command(
    input: &[u8],
    id_field: &'static str,
    route_field: &'static str,
) -> Result<(String, String), TransportError> {
    let mut cursor = Cursor::new(input);
    let mut id = None;
    let mut route = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if id.is_some() {
                    return Err(TransportError::DuplicateField(id_field));
                }
                id = Some(cursor.utf8(id_field, MAX_HOME_CARD_ID_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if route.is_some() {
                    return Err(TransportError::DuplicateField(route_field));
                }
                route = Some(cursor.utf8(route_field, MAX_HOME_ROUTE_ID_BYTES)?);
            }
            1..=2 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let id = id.ok_or(TransportError::MissingField(id_field))?;
    let route = route.ok_or(TransportError::MissingField(route_field))?;
    validate_text(id_field, &id, MAX_HOME_CARD_ID_BYTES)?;
    validate_text(route_field, &route, MAX_HOME_ROUTE_ID_BYTES)?;
    Ok((id, route))
}

fn decode_search_command(input: &[u8]) -> Result<String, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut query = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if query.is_some() {
                    return Err(TransportError::DuplicateField("home search query"));
                }
                query = Some(cursor.utf8("home search query", MAX_HOME_QUERY_BYTES)?);
            }
            1 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let query = query.ok_or(TransportError::MissingField("home search query"))?;
    validate_bounded_text("home search query", &query, MAX_HOME_QUERY_BYTES)?;
    Ok(query)
}

#[allow(clippy::too_many_lines)]
fn decode_home_command(input: &[u8]) -> Result<HomeCommandRequest, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut command_version = None;
    let mut command_id = None;
    let mut observed_event_sequence = None;
    let mut command = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            FIELD_HOME_COMMAND_VERSION if wire_type == WireType::Varint => {
                if command_version.is_some() {
                    return Err(TransportError::DuplicateField("home command version"));
                }
                let version: u32 = cursor.varint()?.try_into().map_err(|_| {
                    TransportError::InvalidValue("home command version exceeds uint32")
                })?;
                command_version = Some(version);
            }
            FIELD_HOME_COMMAND_ID if wire_type == WireType::LengthDelimited => {
                if command_id.is_some() {
                    return Err(TransportError::DuplicateField("home command ID"));
                }
                command_id = Some(cursor.utf8("home command ID", MAX_HOME_COMMAND_ID_BYTES)?);
            }
            FIELD_HOME_COMMAND_OBSERVED_EVENT_SEQUENCE if wire_type == WireType::Varint => {
                if observed_event_sequence.is_some() {
                    return Err(TransportError::DuplicateField(
                        "home command observed event sequence",
                    ));
                }
                observed_event_sequence = Some(cursor.varint()?);
            }
            FIELD_HOME_OPEN_QUICK_ENTRY if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                let value = decode_route_command(
                    &cursor.bytes("home open quick entry", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                    "home quick-entry ID",
                    "home quick-entry route ID",
                )?;
                command = Some(HomeCommand::OpenQuickEntry {
                    card_id: value.0,
                    route_id: value.1,
                });
            }
            FIELD_HOME_PLAY_TRACK if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                command = Some(HomeCommand::PlayTrack {
                    card_id: decode_single_id_command(
                        &cursor.bytes("home play track", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                        "home track ID",
                    )?,
                });
            }
            FIELD_HOME_OPEN_RECOMMENDATION if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                let value = decode_route_command(
                    &cursor.bytes("home open recommendation", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                    "home recommendation ID",
                    "home recommendation route ID",
                )?;
                command = Some(HomeCommand::OpenRecommendation {
                    card_id: value.0,
                    route_id: value.1,
                });
            }
            FIELD_HOME_OPEN_ALL_RECENT if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                command = Some(HomeCommand::OpenAllRecent {
                    section_id: decode_single_id_command(
                        &cursor.bytes("home open all recent", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                        "home section ID",
                    )?,
                });
            }
            FIELD_HOME_CUSTOMIZE if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                let payload = cursor.bytes("home customize", 0)?;
                if !payload.is_empty() {
                    return Err(TransportError::InvalidValue("home customize must be empty"));
                }
                command = Some(HomeCommand::CustomizeHome);
            }
            FIELD_HOME_SEARCH if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                command = Some(HomeCommand::Search {
                    query: decode_search_command(
                        &cursor.bytes("home search", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                    )?,
                });
            }
            FIELD_HOME_TOGGLE_FAVORITE if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                command = Some(HomeCommand::ToggleFavorite {
                    card_id: decode_single_id_command(
                        &cursor.bytes("home toggle favorite", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                        "home track ID",
                    )?,
                });
            }
            FIELD_HOME_OPEN_EXTENSION if wire_type == WireType::LengthDelimited => {
                if command.is_some() {
                    return Err(TransportError::InvalidValue(
                        "multiple Home command oneof arms",
                    ));
                }
                let value = decode_route_command(
                    &cursor.bytes("home open extension", MAX_HOME_COMMAND_MESSAGE_BYTES)?,
                    "home extension ID",
                    "home extension route ID",
                )?;
                command = Some(HomeCommand::OpenExtension {
                    card_id: value.0,
                    route_id: value.1,
                });
            }
            FIELD_HOME_COMMAND_VERSION
            | FIELD_HOME_COMMAND_ID
            | FIELD_HOME_COMMAND_OBSERVED_EVENT_SEQUENCE
            | FIELD_HOME_OPEN_QUICK_ENTRY
            | FIELD_HOME_PLAY_TRACK
            | FIELD_HOME_OPEN_RECOMMENDATION
            | FIELD_HOME_OPEN_ALL_RECENT
            | FIELD_HOME_CUSTOMIZE
            | FIELD_HOME_SEARCH
            | FIELD_HOME_TOGGLE_FAVORITE
            | FIELD_HOME_OPEN_EXTENSION => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    let command_version =
        command_version.ok_or(TransportError::MissingField("home command version"))?;
    if command_version != HOME_PROJECTION_VERSION {
        return Err(TransportError::UnsupportedProjection {
            expected: HOME_PROJECTION_VERSION,
            actual: command_version,
        });
    }
    HomeCommandRequest::new(
        command_id.ok_or(TransportError::MissingField("home command ID"))?,
        observed_event_sequence.ok_or(TransportError::MissingField(
            "home command observed event sequence",
        ))?,
        command.ok_or(TransportError::MissingOneof("home command"))?,
    )
}

fn validate_command(command: &HomeCommand) -> Result<(), TransportError> {
    match command {
        HomeCommand::OpenQuickEntry { card_id, route_id }
        | HomeCommand::OpenRecommendation { card_id, route_id }
        | HomeCommand::OpenExtension { card_id, route_id } => {
            validate_text("home command card ID", card_id, MAX_HOME_CARD_ID_BYTES)?;
            validate_text("home command route ID", route_id, MAX_HOME_ROUTE_ID_BYTES)
        }
        HomeCommand::PlayTrack { card_id } | HomeCommand::ToggleFavorite { card_id } => {
            validate_text("home command card ID", card_id, MAX_HOME_CARD_ID_BYTES)
        }
        HomeCommand::OpenAllRecent { section_id } => validate_text(
            "home command section ID",
            section_id,
            MAX_HOME_CARD_ID_BYTES,
        ),
        HomeCommand::CustomizeHome => Ok(()),
        HomeCommand::Search { query } => {
            validate_bounded_text("home search query", query, MAX_HOME_QUERY_BYTES)
        }
    }
}

fn validate_cards(cards: &[HomeCard], field: &'static str) -> Result<(), TransportError> {
    if cards.len() > MAX_HOME_CARDS {
        return Err(TransportError::FieldTooLarge {
            field,
            max: MAX_HOME_CARDS,
            actual: cards.len(),
        });
    }
    for card in cards {
        card.validate()?;
    }
    validate_unique_ids(cards.iter().map(HomeCard::card_id), "home card ID")
}

fn validate_recommendations(cards: &[HomeRecommendation]) -> Result<(), TransportError> {
    if cards.len() > MAX_HOME_CARDS {
        return Err(TransportError::FieldTooLarge {
            field: "home recommendations",
            max: MAX_HOME_CARDS,
            actual: cards.len(),
        });
    }
    for card in cards {
        card.validate()?;
    }
    validate_unique_ids(
        cards.iter().map(HomeRecommendation::card_id),
        "home recommendation ID",
    )
}

fn validate_unique_ids<'a, I>(ids: I, field: &'static str) -> Result<(), TransportError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(&id) {
            return Err(TransportError::InvalidValue(field));
        }
        seen.push(id);
    }
    Ok(())
}

fn validate_optional_text(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), TransportError> {
    if let Some(value) = value {
        validate_text(field, value, max)?;
    }
    Ok(())
}

fn validate_bounded_text(
    field: &'static str,
    value: &str,
    max: usize,
) -> Result<(), TransportError> {
    if value.len() > max {
        return Err(TransportError::FieldTooLarge {
            field,
            max,
            actual: value.len(),
        });
    }
    Ok(())
}

fn put_optional_text_field(output: &mut Vec<u8>, field: u32, value: Option<&str>) {
    if let Some(value) = value {
        put_bytes_field(output, field, value.as_bytes());
    }
}

fn ensure_nested_size(
    output: Vec<u8>,
    max: usize,
    field: &'static str,
) -> Result<Vec<u8>, TransportError> {
    if output.len() > max {
        return Err(TransportError::FieldTooLarge {
            field,
            max,
            actual: output.len(),
        });
    }
    Ok(output)
}

fn ensure_input_size(input: &[u8], max: usize) -> Result<(), TransportError> {
    if input.len() > max {
        if max == MAX_EVENT_BYTES {
            return Err(TransportError::EventTooLarge {
                max,
                actual: input.len(),
            });
        }
        return Err(TransportError::MessageTooLarge {
            max,
            actual: input.len(),
        });
    }
    Ok(())
}

fn ensure_event_size(output: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    ensure_input_size(&output, MAX_EVENT_BYTES)?;
    Ok(output)
}

fn decode_extension(input: &[u8]) -> Result<ExtensionRequest, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut namespace = None;
    let mut schema_version = None;
    let mut payload = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if namespace.is_some() {
                    return Err(TransportError::DuplicateField("extension namespace"));
                }
                namespace = Some(cursor.utf8("extension namespace", MAX_NAMESPACE_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if schema_version.is_some() {
                    return Err(TransportError::DuplicateField("extension schema version"));
                }
                schema_version =
                    Some(cursor.utf8("extension schema version", MAX_EXTENSION_SCHEMA_BYTES)?);
            }
            3 if wire_type == WireType::LengthDelimited => {
                if payload.is_some() {
                    return Err(TransportError::DuplicateField("extension payload"));
                }
                payload = Some(cursor.bytes("extension payload", MAX_EXTENSION_PAYLOAD_BYTES)?);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    ExtensionRequest::new(
        namespace.ok_or(TransportError::MissingField("extension namespace"))?,
        schema_version.ok_or(TransportError::MissingField("extension schema version"))?,
        payload.ok_or(TransportError::MissingField("extension payload"))?,
    )
}

fn decode_error(input: &[u8]) -> Result<FfiError, TransportError> {
    let mut cursor = Cursor::new(input);
    let mut code = None;
    let mut message = None;
    let mut retryable = None;
    while !cursor.is_empty() {
        let (field, wire_type) = cursor.key()?;
        match field {
            1 if wire_type == WireType::LengthDelimited => {
                if code.is_some() {
                    return Err(TransportError::DuplicateField("error code"));
                }
                code = Some(cursor.utf8("error code", MAX_ERROR_CODE_BYTES)?);
            }
            2 if wire_type == WireType::LengthDelimited => {
                if message.is_some() {
                    return Err(TransportError::DuplicateField("error message"));
                }
                message = Some(cursor.utf8("error message", MAX_ERROR_MESSAGE_BYTES)?);
            }
            3 if wire_type == WireType::Varint => {
                if retryable.is_some() {
                    return Err(TransportError::DuplicateField("error retryable"));
                }
                retryable = Some(cursor.varint()? != 0);
            }
            1..=3 => return Err(TransportError::InvalidWireType { field }),
            _ => return Err(TransportError::UnknownField { field }),
        }
    }
    FfiError::new(
        code.ok_or(TransportError::MissingField("error code"))?,
        message.ok_or(TransportError::MissingField("error message"))?,
        retryable.ok_or(TransportError::MissingField("error retryable"))?,
    )
}

fn select_body(
    body: &mut Option<RequestBody>,
    selected: RequestBody,
) -> Result<(), TransportError> {
    if body.is_some() {
        return Err(TransportError::InvalidValue("multiple body oneof arms"));
    }
    *body = Some(selected);
    Ok(())
}

fn select_response_body(
    body: &mut Option<ResponseBody>,
    selected: ResponseBody,
) -> Result<(), TransportError> {
    if body.is_some() {
        return Err(TransportError::InvalidValue(
            "multiple response body oneof arms",
        ));
    }
    *body = Some(selected);
    Ok(())
}

fn validate_text(field: &'static str, value: &str, max: usize) -> Result<(), TransportError> {
    if value.is_empty() {
        return Err(TransportError::InvalidValue(field));
    }
    if value.len() > max {
        return Err(TransportError::FieldTooLarge {
            field,
            max,
            actual: value.len(),
        });
    }
    Ok(())
}

fn ensure_message_size(output: Vec<u8>) -> Result<Vec<u8>, TransportError> {
    if output.len() > MAX_MESSAGE_BYTES {
        return Err(TransportError::MessageTooLarge {
            max: MAX_MESSAGE_BYTES,
            actual: output.len(),
        });
    }
    Ok(output)
}

fn put_varint_field(output: &mut Vec<u8>, field: u32, value: u64) {
    put_key(output, field, WireType::Varint);
    put_varint(output, value);
}

fn put_bytes_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    put_message_field(output, field, value);
}

fn put_message_field(output: &mut Vec<u8>, field: u32, value: &[u8]) {
    put_key(output, field, WireType::LengthDelimited);
    put_varint(output, value.len() as u64);
    output.extend_from_slice(value);
}

fn put_key(output: &mut Vec<u8>, field: u32, wire_type: WireType) {
    put_varint(output, (u64::from(field) << 3) | u64::from(wire_type as u8));
}

fn put_varint(output: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        let byte = u8::try_from(value & 0x7f).expect("varint byte is masked to seven bits");
        output.push(byte | 0x80);
        value >>= 7;
    }
    output.push(u8::try_from(value).expect("final varint byte fits in u8"));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireType {
    Varint = 0,
    LengthDelimited = 2,
}

impl TryFrom<u8> for WireType {
    type Error = TransportError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Varint),
            2 => Ok(Self::LengthDelimited),
            _ => Err(TransportError::Malformed("unsupported protobuf wire type")),
        }
    }
}

struct Cursor<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, position: 0 }
    }

    fn is_empty(&self) -> bool {
        self.position == self.input.len()
    }

    fn key(&mut self) -> Result<(u32, WireType), TransportError> {
        let key = self.varint()?;
        let field = u32::try_from(key >> 3)
            .map_err(|_| TransportError::Malformed("field number exceeds uint32"))?;
        if field == 0 {
            return Err(TransportError::Malformed("field number must not be zero"));
        }
        let wire_type = WireType::try_from((key & 0x07) as u8)?;
        Ok((field, wire_type))
    }

    fn varint(&mut self) -> Result<u64, TransportError> {
        let mut value = 0_u64;
        for shift in (0..70).step_by(7) {
            let byte = *self
                .input
                .get(self.position)
                .ok_or(TransportError::Malformed("truncated varint"))?;
            self.position += 1;
            if shift == 63 && byte > 1 {
                return Err(TransportError::Malformed("varint overflows uint64"));
            }
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(TransportError::Malformed("varint is too long"))
    }

    fn bytes(&mut self, field: &'static str, max: usize) -> Result<Vec<u8>, TransportError> {
        let length = self.varint()?;
        let length = usize::try_from(length)
            .map_err(|_| TransportError::Malformed("length exceeds usize"))?;
        if length > max {
            return Err(TransportError::FieldTooLarge {
                field,
                max,
                actual: length,
            });
        }
        let end = self
            .position
            .checked_add(length)
            .ok_or(TransportError::Malformed("length overflows input"))?;
        let value = self
            .input
            .get(self.position..end)
            .ok_or(TransportError::Malformed("truncated bytes field"))?;
        self.position = end;
        Ok(value.to_vec())
    }

    fn utf8(&mut self, field: &'static str, max: usize) -> Result<String, TransportError> {
        let value = self.bytes(field, max)?;
        String::from_utf8(value).map_err(|_| TransportError::InvalidValue(field))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExtensionRequest, FfiError, FfiRequest, FfiResponse, HomeCard, HomeCommand,
        HomeCommandRequest, HomeQuickEntry, HomeRecommendation, HomeRecommendationSet, HomeSection,
        HomeSectionKind, HomeSnapshot, HomeSourceKind, HomeSourceMetadata, HomeSourceStatus,
        HomeState, HomeStatus, MAX_EVENT_BYTES, MAX_HOME_ASSET_KEY_BYTES, MAX_MESSAGE_BYTES,
        RequestBody, ResponseBody, SCHEMA_MAJOR, TransportError, put_bytes_field,
        put_message_field, put_varint_field,
    };

    fn local_source() -> HomeSourceMetadata {
        HomeSourceMetadata::new(HomeSourceKind::Local, "local", HomeSourceStatus::Available)
            .expect("local source is valid")
    }

    fn ready_status() -> HomeStatus {
        HomeStatus::new(HomeState::Ready)
    }

    fn complete_home_snapshot() -> HomeSnapshot {
        let source = local_source();
        let status = ready_status();
        let track = HomeCard::new("track-1", "Track 1", source.clone(), status.clone())
            .expect("track is valid")
            .with_subtitle("Artist")
            .with_artist("Artist")
            .with_album("Album")
            .with_artwork_key("artwork-1")
            .with_quality_label("FLAC")
            .with_duration_ms(245_000)
            .with_route_id("track:track-1")
            .with_favorite(true);
        let section = HomeSection::new(
            HomeSectionKind::RecentlyPlayed,
            source.clone(),
            status.clone(),
            vec![track],
        )
        .expect("section is valid");
        let quick_entry = HomeQuickEntry::new(
            "favorites",
            "Favorites",
            "favorites",
            source.clone(),
            status.clone(),
        )
        .expect("quick entry is valid")
        .with_subtitle("1 track")
        .with_icon_ref("heart")
        .with_count(1)
        .with_customizable(false);
        let recommendation = HomeRecommendation::new(
            "discover-1",
            "Daily mix",
            "playlist:daily-mix",
            HomeSourceMetadata::new(
                HomeSourceKind::Provider,
                "provider-1",
                HomeSourceStatus::Available,
            )
            .expect("provider source is valid"),
            status.clone(),
        )
        .expect("recommendation is valid")
        .with_description("A bounded recommendation")
        .with_artwork_key("daily-mix");
        let discover = HomeRecommendationSet::new(
            recommendation.source().clone(),
            status.clone(),
            vec![recommendation],
        )
        .expect("recommendation set is valid");
        HomeSnapshot::new(
            42,
            source,
            status,
            vec![section],
            vec![quick_entry],
            Some(discover),
        )
        .expect("snapshot is valid")
    }

    #[test]
    fn ping_round_trips_through_the_public_wire_shape() {
        let request = FfiRequest::new("request-1", RequestBody::Ping).expect("request is valid");
        let encoded = request.encode().expect("request encodes");
        let decoded = FfiRequest::decode(&encoded).expect("request decodes");
        assert_eq!(decoded, request);
        assert_eq!(decoded.schema_version(), SCHEMA_MAJOR);
    }

    #[test]
    fn extension_round_trip_preserves_only_bounded_transport_fields() {
        let extension = ExtensionRequest::new("org.example.tool", "1", vec![1, 2, 3])
            .expect("extension is valid");
        let request = FfiRequest::new("request-2", RequestBody::Extension(extension))
            .expect("request is valid");
        let encoded = request.encode().expect("request encodes");
        assert_eq!(
            FfiRequest::decode(&encoded).expect("request decodes"),
            request
        );
    }

    #[test]
    fn unknown_schema_major_fails_closed() {
        let error = FfiRequest::decode(&[0x08, 0x02, 0x12, 0x01, b'x', 0x52, 0x00])
            .expect_err("unknown major must fail");
        assert_eq!(
            error,
            TransportError::UnsupportedSchema {
                expected: SCHEMA_MAJOR,
                actual: 2
            }
        );
    }

    #[test]
    fn unknown_oneof_arm_fails_closed() {
        let error = FfiRequest::decode(&[
            0x08, 0x01, // schema_version
            0x12, 0x01, b'x', // request_id
            0x62, 0x00, // field 12: unknown body arm
        ])
        .expect_err("unknown oneof must fail");
        assert_eq!(error, TransportError::UnknownField { field: 12 });
    }

    #[test]
    fn over_limit_message_fails_before_allocation() {
        let input = vec![0_u8; MAX_MESSAGE_BYTES + 1];
        assert_eq!(
            FfiRequest::decode(&input).expect_err("oversized input must fail"),
            TransportError::MessageTooLarge {
                max: MAX_MESSAGE_BYTES,
                actual: MAX_MESSAGE_BYTES + 1
            }
        );
    }

    #[test]
    fn duplicate_body_arms_fail_closed() {
        let error = FfiRequest::decode(&[0x08, 0x01, 0x12, 0x01, b'x', 0x52, 0x00, 0x5a, 0x00])
            .expect_err("two oneof arms must fail");
        assert_eq!(
            error,
            TransportError::InvalidValue("multiple body oneof arms")
        );
    }

    #[test]
    fn extension_payload_limit_is_enforced() {
        let payload = vec![0_u8; super::MAX_EXTENSION_PAYLOAD_BYTES + 1];
        assert!(matches!(
            ExtensionRequest::new("org.example.tool", "1", payload),
            Err(TransportError::FieldTooLarge {
                field: "extension payload",
                ..
            })
        ));
    }

    #[test]
    fn pong_response_round_trips_with_request_correlation() {
        let response =
            FfiResponse::new("request-1", ResponseBody::Pong).expect("response is valid");
        let encoded = response.encode().expect("response encodes");
        let decoded = FfiResponse::decode(&encoded).expect("response decodes");
        assert_eq!(decoded, response);
    }

    #[test]
    fn typed_error_response_round_trips_and_preserves_retryability() {
        let error = FfiError::new("invalid_request", "the request was rejected", true)
            .expect("error is valid");
        let response =
            FfiResponse::new("request-3", ResponseBody::Error(error)).expect("response is valid");
        let encoded = response.encode().expect("response encodes");
        assert_eq!(
            FfiResponse::decode(&encoded).expect("response decodes"),
            response
        );
    }

    #[test]
    fn unknown_response_oneof_arm_fails_closed() {
        let error = FfiResponse::decode(&[
            0x08, 0x01, // schema_version
            0x12, 0x01, b'x', // request_id
            0x62, 0x00, // field 12: unknown body arm
        ])
        .expect_err("unknown response oneof must fail");
        assert_eq!(error, TransportError::UnknownField { field: 12 });
    }

    #[test]
    fn home_snapshot_round_trips_through_the_existing_extension_arm() {
        let snapshot = complete_home_snapshot();
        let extension = ExtensionRequest::from_home_snapshot(&snapshot)
            .expect("Home extension is bounded and valid");
        let decoded = extension
            .decode_home_snapshot()
            .expect("Home extension decodes");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.observed_event_sequence(), 42);
        assert_eq!(decoded.sections()[0].cards()[0].card_id(), "track-1");
        assert_eq!(decoded.quick_entries()[0].route_id(), "favorites");
        assert_eq!(
            decoded.discover().expect("discover exists").cards().len(),
            1
        );
    }

    #[test]
    fn home_empty_offline_and_provider_timeout_states_round_trip() {
        let cases = [
            (
                HomeSourceKind::Local,
                HomeSourceStatus::Available,
                HomeState::Loading,
                true,
            ),
            (
                HomeSourceKind::Local,
                HomeSourceStatus::Available,
                HomeState::Empty,
                false,
            ),
            (
                HomeSourceKind::Local,
                HomeSourceStatus::Offline,
                HomeState::Offline,
                false,
            ),
            (
                HomeSourceKind::Provider,
                HomeSourceStatus::TimedOut,
                HomeState::Unavailable,
                true,
            ),
        ];
        for (kind, source_status, state, retryable) in cases {
            let source = HomeSourceMetadata::new(kind, "source", source_status)
                .expect("source is valid")
                .with_detail("bounded status detail");
            let status = HomeStatus::new(state)
                .with_message("no data is currently available")
                .with_retryable(retryable);
            let snapshot = HomeSnapshot::new(7, source, status, Vec::new(), Vec::new(), None)
                .expect("state snapshot is valid");
            let decoded = HomeSnapshot::decode(&snapshot.encode().expect("snapshot encodes"))
                .expect("state snapshot decodes");
            assert_eq!(decoded.state(), state);
            assert_eq!(decoded.source().status(), source_status);
            assert_eq!(decoded.status().retryable(), retryable);
            assert!(decoded.sections().is_empty());
            assert!(decoded.quick_entries().is_empty());
            assert!(decoded.discover().is_none());
        }
    }

    #[test]
    fn home_command_routes_round_trip_without_executing_business_logic() {
        let commands = vec![
            HomeCommand::OpenQuickEntry {
                card_id: "favorites".to_owned(),
                route_id: "favorites".to_owned(),
            },
            HomeCommand::PlayTrack {
                card_id: "track-1".to_owned(),
            },
            HomeCommand::OpenRecommendation {
                card_id: "discover-1".to_owned(),
                route_id: "playlist:daily-mix".to_owned(),
            },
            HomeCommand::OpenAllRecent {
                section_id: "recently-played".to_owned(),
            },
            HomeCommand::CustomizeHome,
            HomeCommand::Search {
                query: "artist name".to_owned(),
            },
            HomeCommand::ToggleFavorite {
                card_id: "track-1".to_owned(),
            },
            HomeCommand::OpenExtension {
                card_id: "extension-card".to_owned(),
                route_id: "extension:open".to_owned(),
            },
        ];
        for (index, command) in commands.into_iter().enumerate() {
            let request = HomeCommandRequest::new(format!("home-command-{index}"), 42, command)
                .expect("command is valid");
            let encoded = request.encode().expect("command encodes");
            assert_eq!(
                HomeCommandRequest::decode(&encoded).expect("command decodes"),
                request
            );
        }
    }

    #[test]
    fn home_unknown_field_and_invalid_enum_fail_closed() {
        let snapshot = complete_home_snapshot();
        let mut unknown = snapshot.encode().expect("snapshot encodes");
        unknown.extend([0x40, 0x00]); // field 8 is outside HomeSnapshotV1.
        assert_eq!(
            HomeSnapshot::decode(&unknown).expect_err("unknown Home field must fail"),
            TransportError::UnknownField { field: 8 }
        );

        let mut source = Vec::new();
        put_varint_field(&mut source, 1, 99);
        put_bytes_field(&mut source, 2, b"source");
        put_varint_field(&mut source, 4, 1);
        let mut status = Vec::new();
        put_varint_field(&mut status, 1, 1);
        put_varint_field(&mut status, 3, 0);
        let mut invalid = Vec::new();
        put_varint_field(&mut invalid, 1, 1);
        put_varint_field(&mut invalid, 2, 0);
        put_message_field(&mut invalid, 3, &source);
        put_message_field(&mut invalid, 4, &status);
        assert_eq!(
            HomeSnapshot::decode(&invalid).expect_err("invalid enum must fail"),
            TransportError::InvalidEnum {
                field: "home source kind",
                value: 99,
            }
        );
    }

    #[test]
    fn home_duplicate_card_identity_is_rejected() {
        let source = local_source();
        let status = ready_status();
        let card = HomeCard::new("same-id", "Track", source.clone(), status.clone())
            .expect("card is valid");
        let section = HomeSection::new(
            HomeSectionKind::Favorites,
            source.clone(),
            status.clone(),
            vec![card.clone(), card],
        )
        .expect_err("duplicate card identity must fail");
        assert_eq!(section, TransportError::InvalidValue("home card ID"));
    }

    #[test]
    fn home_event_size_limit_is_enforced_before_decode() {
        let input = vec![0_u8; MAX_EVENT_BYTES + 1];
        assert_eq!(
            HomeSnapshot::decode_event(&input).expect_err("oversized event must fail"),
            TransportError::EventTooLarge {
                max: MAX_EVENT_BYTES,
                actual: MAX_EVENT_BYTES + 1,
            }
        );
    }

    #[test]
    fn home_card_field_limit_is_enforced_before_encoding() {
        let card = HomeCard::new("track-1", "Track", local_source(), ready_status())
            .expect("card is valid")
            .with_artwork_key("a".repeat(MAX_HOME_ASSET_KEY_BYTES + 1));
        assert!(matches!(
            card.validate(),
            Err(TransportError::FieldTooLarge {
                field: "home card artwork key",
                ..
            })
        ));
    }

    #[test]
    fn home_command_unknown_oneof_arm_fails_closed() {
        let error = HomeCommandRequest::decode(&[
            0x08, 0x01, // command_version
            0x12, 0x01, b'x', // command_id
            0x18, 0x00, // observed_event_sequence
            0x92, 0x01, 0x00, // field 18: unknown command arm
        ])
        .expect_err("unknown Home command oneof must fail");
        assert_eq!(error, TransportError::UnknownField { field: 18 });
    }

    #[test]
    fn home_extension_namespace_and_projection_version_are_checked() {
        let snapshot = complete_home_snapshot();
        let payload = snapshot.encode().expect("snapshot encodes");
        let wrong_namespace = ExtensionRequest::new("other.namespace", "1", payload.clone())
            .expect("extension is valid");
        assert_eq!(
            wrong_namespace
                .decode_home_snapshot()
                .expect_err("wrong namespace must fail"),
            TransportError::InvalidValue("unexpected Home extension namespace")
        );
        let wrong_version =
            ExtensionRequest::new("aurorix.home", "2", payload).expect("extension is valid");
        assert_eq!(
            wrong_version
                .decode_home_snapshot()
                .expect_err("wrong Home version must fail"),
            TransportError::UnsupportedProjection {
                expected: 1,
                actual: 2,
            }
        );
    }
}

//! Runtime source resolution without durable resource leakage.
//!
//! The request and durable intent types below carry only Core identities. A
//! resolver may use a platform locator internally, but paths, descriptors,
//! URLs, credentials, and leases never enter these values or their errors.

use std::{error::Error, fmt};

use crate::command::PlaybackItemId;

/// The catalog entity kind that can be resolved for local playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogSourceKind {
    /// A device-local asset containing encoded bytes.
    LocalAsset,
    /// A catalog recording resolved through one or more local assets.
    Recording,
}

/// Availability observed for a catalog source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CatalogSourceAvailability {
    /// A source is eligible for runtime resolution.
    Available,
    /// The identity remains valid but no local bytes are currently available.
    Missing,
}

/// A catalog-only source reference used as a resolver request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSourceRef {
    item_id: PlaybackItemId,
    kind: CatalogSourceKind,
    availability: CatalogSourceAvailability,
}

impl CatalogSourceRef {
    /// Creates a local-asset request with an observed availability.
    #[must_use]
    pub fn local_asset(item_id: PlaybackItemId, availability: CatalogSourceAvailability) -> Self {
        Self {
            item_id,
            kind: CatalogSourceKind::LocalAsset,
            availability,
        }
    }

    /// Creates a recording request. The resolver determines whether an asset
    /// for the recording is currently available.
    #[must_use]
    pub fn recording(item_id: PlaybackItemId) -> Self {
        Self {
            item_id,
            kind: CatalogSourceKind::Recording,
            availability: CatalogSourceAvailability::Available,
        }
    }

    /// Creates a request with an explicit kind and observed availability.
    #[must_use]
    pub const fn new(
        item_id: PlaybackItemId,
        kind: CatalogSourceKind,
        availability: CatalogSourceAvailability,
    ) -> Self {
        Self {
            item_id,
            kind,
            availability,
        }
    }

    /// Returns the Core identity used for playback and history.
    #[must_use]
    pub fn item_id(&self) -> &PlaybackItemId {
        &self.item_id
    }

    /// Returns the catalog source kind.
    #[must_use]
    pub const fn kind(&self) -> CatalogSourceKind {
        self.kind
    }

    /// Returns the latest catalog availability observation.
    #[must_use]
    pub const fn availability(&self) -> CatalogSourceAvailability {
        self.availability
    }
}

/// Durable playback intent containing no runtime resource data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSourceIntent {
    item_id: PlaybackItemId,
    kind: CatalogSourceKind,
}

impl DurableSourceIntent {
    /// Creates durable intent from a catalog-only source reference.
    #[must_use]
    pub fn from_catalog(source: &CatalogSourceRef) -> Self {
        Self {
            item_id: source.item_id.clone(),
            kind: source.kind,
        }
    }

    /// Returns the identity that may be persisted or synchronized.
    #[must_use]
    pub fn item_id(&self) -> &PlaybackItemId {
        &self.item_id
    }

    /// Returns the catalog kind.
    #[must_use]
    pub const fn kind(&self) -> CatalogSourceKind {
        self.kind
    }
}

/// An opaque identifier for one active runtime capability.
///
/// This identifier has meaning only inside the process and active worker
/// lifetime that created it. It is intentionally not a catalog or sync ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeCapabilityId(u64);

impl RuntimeCapabilityId {
    /// Creates a runtime-only capability identifier supplied by an adapter.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the opaque value for an in-process adapter.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A short-lived capability handed from resolution to a decoder worker.
///
/// The type has no serialization implementation and contains no path, file
/// descriptor, URL, credential, or provider lease. Dropping it retires the
/// capability at the owning adapter boundary.
#[derive(Debug, PartialEq, Eq)]
pub struct RuntimeSourceCapability {
    item_id: PlaybackItemId,
    kind: CatalogSourceKind,
    capability_id: RuntimeCapabilityId,
}

impl RuntimeSourceCapability {
    /// Creates a capability from an adapter-owned opaque runtime handle.
    #[must_use]
    pub const fn new(
        item_id: PlaybackItemId,
        kind: CatalogSourceKind,
        capability_id: RuntimeCapabilityId,
    ) -> Self {
        Self {
            item_id,
            kind,
            capability_id,
        }
    }

    /// Returns the Core identity associated with the active capability.
    #[must_use]
    pub fn item_id(&self) -> &PlaybackItemId {
        &self.item_id
    }

    /// Returns the catalog kind associated with the capability.
    #[must_use]
    pub const fn kind(&self) -> CatalogSourceKind {
        self.kind
    }

    /// Returns the opaque adapter-local capability ID.
    #[must_use]
    pub const fn capability_id(&self) -> RuntimeCapabilityId {
        self.capability_id
    }
}

/// A typed failure from source resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceResolutionError {
    /// The catalog source was marked missing before opening.
    MissingCatalogSource,
    /// The catalog identity is not known to the resolver.
    UnknownCatalogIdentity,
    /// A valid identity has no currently openable local source.
    RuntimeSourceUnavailable,
    /// The adapter rejected the source without exposing private details.
    OpenFailed,
    /// The adapter returned a capability for another Core identity or kind.
    CapabilityMismatch,
}

impl fmt::Display for SourceResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCatalogSource => formatter.write_str("catalog source is missing"),
            Self::UnknownCatalogIdentity => formatter.write_str("catalog identity is unknown"),
            Self::RuntimeSourceUnavailable => formatter.write_str("runtime source is unavailable"),
            Self::OpenFailed => formatter.write_str("runtime source could not be opened"),
            Self::CapabilityMismatch => {
                formatter.write_str("runtime capability does not match catalog source")
            }
        }
    }
}

impl Error for SourceResolutionError {}

/// Adapter boundary that resolves catalog intent into a short-lived capability.
pub trait RuntimeSourceResolver {
    /// Resolves one catalog source on a worker/control path.
    ///
    /// Implementations may inspect their private platform locator store here.
    /// They must not return that locator, a raw descriptor, credentials, or a
    /// URL in the capability or error value.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the runtime source cannot be resolved.
    fn resolve(
        &self,
        source: &CatalogSourceRef,
    ) -> Result<RuntimeSourceCapability, SourceResolutionError>;
}

/// Resolves one source while enforcing the catalog availability boundary.
///
/// # Errors
///
/// Returns a typed error when the source is missing, resolution fails, or the
/// returned capability does not match the requested identity.
pub fn resolve_source<R: RuntimeSourceResolver>(
    resolver: &R,
    source: &CatalogSourceRef,
) -> Result<RuntimeSourceCapability, SourceResolutionError> {
    if source.availability == CatalogSourceAvailability::Missing {
        return Err(SourceResolutionError::MissingCatalogSource);
    }
    let capability = resolver.resolve(source)?;
    if capability.item_id() != source.item_id() || capability.kind() != source.kind() {
        return Err(SourceResolutionError::CapabilityMismatch);
    }
    Ok(capability)
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogSourceAvailability, CatalogSourceKind, CatalogSourceRef, DurableSourceIntent,
        RuntimeCapabilityId, RuntimeSourceCapability, RuntimeSourceResolver, SourceResolutionError,
        resolve_source,
    };
    use crate::command::PlaybackItemId;

    struct FixtureResolver;

    impl RuntimeSourceResolver for FixtureResolver {
        fn resolve(
            &self,
            source: &CatalogSourceRef,
        ) -> Result<RuntimeSourceCapability, SourceResolutionError> {
            Ok(RuntimeSourceCapability::new(
                source.item_id().clone(),
                source.kind(),
                RuntimeCapabilityId::new(7),
            ))
        }
    }

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("test identity is valid")
    }

    #[test]
    fn local_asset_and_recording_requests_keep_only_identity() {
        let asset =
            CatalogSourceRef::local_asset(item("asset-1"), CatalogSourceAvailability::Available);
        let recording = CatalogSourceRef::recording(item("recording-1"));

        assert_eq!(asset.kind(), CatalogSourceKind::LocalAsset);
        assert_eq!(recording.kind(), CatalogSourceKind::Recording);
        assert_eq!(
            DurableSourceIntent::from_catalog(&asset).item_id(),
            &item("asset-1")
        );
    }

    #[test]
    fn missing_asset_preserves_identity_and_never_calls_resolver() {
        let missing = CatalogSourceRef::local_asset(
            item("missing-asset"),
            CatalogSourceAvailability::Missing,
        );
        let result = resolve_source(&FixtureResolver, &missing);

        assert_eq!(result, Err(SourceResolutionError::MissingCatalogSource));
        assert_eq!(missing.item_id(), &item("missing-asset"));
    }

    #[test]
    fn resolution_returns_opaque_runtime_capability() {
        let source = CatalogSourceRef::recording(item("recording-1"));
        let capability = resolve_source(&FixtureResolver, &source).expect("fixture resolves");

        assert_eq!(capability.item_id(), source.item_id());
        assert_eq!(capability.capability_id(), RuntimeCapabilityId::new(7));
        assert!(!format!("{capability:?}").contains("file://"));
        assert!(!format!("{capability:?}").contains("credential"));
    }

    #[test]
    fn durable_intent_does_not_change_when_runtime_capability_is_created() {
        let source = CatalogSourceRef::recording(item("recording-1"));
        let durable = DurableSourceIntent::from_catalog(&source);
        let _runtime = resolve_source(&FixtureResolver, &source).expect("fixture resolves");

        assert_eq!(durable.item_id(), source.item_id());
        assert_eq!(durable.kind(), CatalogSourceKind::Recording);
    }

    #[test]
    fn mismatched_runtime_capability_is_rejected() {
        struct MismatchedResolver;

        impl RuntimeSourceResolver for MismatchedResolver {
            fn resolve(
                &self,
                _source: &CatalogSourceRef,
            ) -> Result<RuntimeSourceCapability, SourceResolutionError> {
                Ok(RuntimeSourceCapability::new(
                    item("another-recording"),
                    CatalogSourceKind::Recording,
                    RuntimeCapabilityId::new(9),
                ))
            }
        }

        let source = CatalogSourceRef::recording(item("recording-1"));
        assert_eq!(
            resolve_source(&MismatchedResolver, &source),
            Err(SourceResolutionError::CapabilityMismatch)
        );
    }
}

//! Immutable portable media identity types and structural validation.

use core::fmt;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::ids::ProviderPackageId;

/// Declares whether an external namespace can stand alone or needs a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NamespaceScope {
    /// A registered external catalogue namespace with globally stable IDs.
    Canonical,
    /// An identity whose meaning depends on a portable provider binding.
    BindingRequired,
}

/// The type of entity identified by an [`ExternalIdentity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExternalEntityType {
    Recording,
    Release,
    Artist,
    Playlist,
    Episode,
}

impl ExternalEntityType {
    /// Returns whether this entity can identify a provider media reference.
    #[must_use]
    pub const fn is_recording(self) -> bool {
        matches!(self, Self::Recording)
    }
}

/// A portable, non-secret provider configuration identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderBindingRef {
    package_id: ProviderPackageId,
    binding_key: String,
    configuration_version: String,
}

impl ProviderBindingRef {
    /// Creates a binding after rejecting non-portable or credential-like values.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when a field is blank or contains an
    /// obvious credential, absolute path, or local/LAN endpoint.
    pub fn new(
        package_id: ProviderPackageId,
        binding_key: impl Into<String>,
        configuration_version: impl Into<String>,
    ) -> Result<Self, MediaReferenceError> {
        let binding_key = binding_key.into();
        let configuration_version = configuration_version.into();

        validate_portable_binding_field("binding_key", &binding_key)?;
        validate_portable_binding_field("configuration_version", &configuration_version)?;

        Ok(Self {
            package_id,
            binding_key,
            configuration_version,
        })
    }

    #[must_use]
    pub fn package_id(&self) -> &ProviderPackageId {
        &self.package_id
    }

    #[must_use]
    pub fn binding_key(&self) -> &str {
        &self.binding_key
    }

    #[must_use]
    pub fn configuration_version(&self) -> &str {
        &self.configuration_version
    }
}

/// A typed identifier from an external media catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalIdentity {
    namespace: String,
    namespace_scope: NamespaceScope,
    entity_type: ExternalEntityType,
    external_id: String,
    binding: Option<ProviderBindingRef>,
}

impl ExternalIdentity {
    /// Creates an external identity with a scope-consistent optional binding.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when a required field is blank or the
    /// binding does not agree with the declared namespace scope.
    pub fn new(
        namespace: impl Into<String>,
        namespace_scope: NamespaceScope,
        entity_type: ExternalEntityType,
        external_id: impl Into<String>,
        binding: Option<ProviderBindingRef>,
    ) -> Result<Self, MediaReferenceError> {
        let namespace = namespace.into();
        let external_id = external_id.into();

        validate_text_field("namespace", &namespace)?;
        validate_text_field("external_id", &external_id)?;

        match (namespace_scope, binding.is_some()) {
            (NamespaceScope::Canonical, true) => {
                return Err(MediaReferenceError::CanonicalIdentityHasBinding);
            }
            (NamespaceScope::BindingRequired, false) => {
                return Err(MediaReferenceError::BindingRequiredIdentityHasNoBinding);
            }
            _ => {}
        }

        Ok(Self {
            namespace,
            namespace_scope,
            entity_type,
            external_id,
            binding,
        })
    }

    /// Creates a canonical identity, which cannot have a provider binding.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when the namespace or external ID is blank.
    pub fn canonical(
        namespace: impl Into<String>,
        entity_type: ExternalEntityType,
        external_id: impl Into<String>,
    ) -> Result<Self, MediaReferenceError> {
        Self::new(
            namespace,
            NamespaceScope::Canonical,
            entity_type,
            external_id,
            None,
        )
    }

    /// Creates an identity that requires the supplied portable provider binding.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when the namespace or external ID is blank.
    pub fn binding_required(
        namespace: impl Into<String>,
        entity_type: ExternalEntityType,
        external_id: impl Into<String>,
        binding: ProviderBindingRef,
    ) -> Result<Self, MediaReferenceError> {
        Self::new(
            namespace,
            NamespaceScope::BindingRequired,
            entity_type,
            external_id,
            Some(binding),
        )
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub const fn namespace_scope(&self) -> NamespaceScope {
        self.namespace_scope
    }

    #[must_use]
    pub const fn entity_type(&self) -> ExternalEntityType {
        self.entity_type
    }

    #[must_use]
    pub fn external_id(&self) -> &str {
        &self.external_id
    }

    #[must_use]
    pub fn binding(&self) -> Option<&ProviderBindingRef> {
        self.binding.as_ref()
    }
}

/// A provider portable media reference whose identity is guaranteed to be a recording.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderRecordingRef {
    identity: ExternalIdentity,
}

impl ProviderRecordingRef {
    /// Creates a provider recording reference from a recording external identity.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError::ProviderReferenceMustIdentifyRecording`]
    /// when the identity does not identify a recording.
    pub fn new(identity: ExternalIdentity) -> Result<Self, MediaReferenceError> {
        if !identity.entity_type().is_recording() {
            return Err(
                MediaReferenceError::ProviderReferenceMustIdentifyRecording {
                    actual: identity.entity_type(),
                },
            );
        }

        Ok(Self { identity })
    }

    #[must_use]
    pub fn identity(&self) -> &ExternalIdentity {
        &self.identity
    }
}

/// A versioned fingerprint used as a portable media matching input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FingerprintRef {
    algorithm: String,
    algorithm_version: String,
    namespace: String,
    fingerprint: String,
}

impl FingerprintRef {
    /// Creates a fingerprint reference with documented algorithm and namespace fields.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when any field is blank.
    pub fn new(
        algorithm: impl Into<String>,
        algorithm_version: impl Into<String>,
        namespace: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Result<Self, MediaReferenceError> {
        let algorithm = algorithm.into();
        let algorithm_version = algorithm_version.into();
        let namespace = namespace.into();
        let fingerprint = fingerprint.into();

        validate_text_field("algorithm", &algorithm)?;
        validate_text_field("algorithm_version", &algorithm_version)?;
        validate_text_field("fingerprint_namespace", &namespace)?;
        validate_text_field("fingerprint", &fingerprint)?;

        Ok(Self {
            algorithm,
            algorithm_version,
            namespace,
            fingerprint,
        })
    }

    #[must_use]
    pub fn algorithm(&self) -> &str {
        &self.algorithm
    }

    #[must_use]
    pub fn algorithm_version(&self) -> &str {
        &self.algorithm_version
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// An untrusted portable payload understood only by a compatible provider package.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpaquePortableRef {
    package_id: ProviderPackageId,
    schema_version: String,
    payload: Vec<u8>,
}

impl OpaquePortableRef {
    /// Creates an opaque provider reference.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when the schema version or payload is empty.
    pub fn new(
        package_id: ProviderPackageId,
        schema_version: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, MediaReferenceError> {
        let schema_version = schema_version.into();

        validate_text_field("schema_version", &schema_version)?;
        if payload.is_empty() {
            return Err(MediaReferenceError::EmptyField { field: "payload" });
        }

        Ok(Self {
            package_id,
            schema_version,
            payload,
        })
    }

    #[must_use]
    pub fn package_id(&self) -> &ProviderPackageId {
        &self.package_id
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// A replicated playable-media reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PortableMediaRef {
    ProviderRecording(ProviderRecordingRef),
    Fingerprint(FingerprintRef),
    OpaqueProvider(OpaquePortableRef),
}

impl PortableMediaRef {
    /// Creates a provider media reference only when the identity names a recording.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError::ProviderReferenceMustIdentifyRecording`]
    /// when the identity does not identify a recording.
    pub fn provider_recording(identity: ExternalIdentity) -> Result<Self, MediaReferenceError> {
        ProviderRecordingRef::new(identity).map(Self::ProviderRecording)
    }

    /// Creates a versioned fingerprint media reference.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when any fingerprint field is blank.
    pub fn fingerprint(
        algorithm: impl Into<String>,
        algorithm_version: impl Into<String>,
        namespace: impl Into<String>,
        fingerprint: impl Into<String>,
    ) -> Result<Self, MediaReferenceError> {
        FingerprintRef::new(algorithm, algorithm_version, namespace, fingerprint)
            .map(Self::Fingerprint)
    }

    /// Creates an opaque provider media reference.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when the schema version or payload is empty.
    pub fn opaque_provider(
        package_id: ProviderPackageId,
        schema_version: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, MediaReferenceError> {
        OpaquePortableRef::new(package_id, schema_version, payload).map(Self::OpaqueProvider)
    }
}

/// Optional metadata that can assist matching but cannot establish identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortableMediaHints {
    title: Option<String>,
    artists: Option<Vec<String>>,
    release_title: Option<String>,
    duration_ms: Option<u64>,
    isrc: Option<String>,
    musicbrainz_recording_id: Option<String>,
}

impl PortableMediaHints {
    /// Creates validated optional media hints.
    ///
    /// # Errors
    ///
    /// Returns [`MediaReferenceError`] when an optional string is blank, the
    /// supplied artist collection is empty, or an artist is blank.
    pub fn new(
        title: Option<String>,
        artists: Option<Vec<String>>,
        release_title: Option<String>,
        duration_ms: Option<u64>,
        isrc: Option<String>,
        musicbrainz_recording_id: Option<String>,
    ) -> Result<Self, MediaReferenceError> {
        validate_optional_text_field("title", title.as_deref())?;
        validate_optional_text_field("release_title", release_title.as_deref())?;
        validate_optional_text_field("isrc", isrc.as_deref())?;
        validate_optional_text_field(
            "musicbrainz_recording_id",
            musicbrainz_recording_id.as_deref(),
        )?;

        if let Some(artists) = artists.as_deref() {
            if artists.is_empty() {
                return Err(MediaReferenceError::EmptyCollection { field: "artists" });
            }
            for artist in artists {
                validate_text_field("artist", artist)?;
            }
        }

        Ok(Self {
            title,
            artists,
            release_title,
            duration_ms,
            isrc,
            musicbrainz_recording_id,
        })
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn artists(&self) -> Option<&[String]> {
        self.artists.as_deref()
    }

    #[must_use]
    pub fn release_title(&self) -> Option<&str> {
        self.release_title.as_deref()
    }

    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    #[must_use]
    pub fn isrc(&self) -> Option<&str> {
        self.isrc.as_deref()
    }

    #[must_use]
    pub fn musicbrainz_recording_id(&self) -> Option<&str> {
        self.musicbrainz_recording_id.as_deref()
    }
}

/// Structural validation errors for portable media identity values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaReferenceError {
    EmptyField { field: &'static str },
    EmptyCollection { field: &'static str },
    CanonicalIdentityHasBinding,
    BindingRequiredIdentityHasNoBinding,
    ProviderReferenceMustIdentifyRecording { actual: ExternalEntityType },
    BindingContainsCredential { field: &'static str },
    BindingContainsAbsolutePath { field: &'static str },
    BindingContainsLocalEndpoint { field: &'static str },
}

impl fmt::Display for MediaReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } | Self::EmptyCollection { field } => {
                write!(formatter, "{field} must not be empty")
            }
            Self::CanonicalIdentityHasBinding => {
                formatter.write_str("a canonical identity must not carry a provider binding")
            }
            Self::BindingRequiredIdentityHasNoBinding => {
                formatter.write_str("a binding-required identity must carry a provider binding")
            }
            Self::ProviderReferenceMustIdentifyRecording { actual } => {
                write!(
                    formatter,
                    "a provider media reference must identify a recording, not {actual:?}"
                )
            }
            Self::BindingContainsCredential { field } => {
                write!(formatter, "{field} must not contain credential-like data")
            }
            Self::BindingContainsAbsolutePath { field } => {
                write!(
                    formatter,
                    "{field} must not contain an absolute filesystem path"
                )
            }
            Self::BindingContainsLocalEndpoint { field } => {
                write!(
                    formatter,
                    "{field} must not contain a local or LAN endpoint"
                )
            }
        }
    }
}

impl Error for MediaReferenceError {}

fn validate_text_field(field: &'static str, value: &str) -> Result<(), MediaReferenceError> {
    if value.trim().is_empty() {
        return Err(MediaReferenceError::EmptyField { field });
    }
    Ok(())
}

fn validate_optional_text_field(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), MediaReferenceError> {
    if let Some(value) = value {
        validate_text_field(field, value)?;
    }
    Ok(())
}

fn validate_portable_binding_field(
    field: &'static str,
    value: &str,
) -> Result<(), MediaReferenceError> {
    validate_text_field(field, value)?;

    if contains_obvious_credential(value) {
        return Err(MediaReferenceError::BindingContainsCredential { field });
    }
    if is_absolute_filesystem_path(value) {
        return Err(MediaReferenceError::BindingContainsAbsolutePath { field });
    }
    if contains_local_or_lan_endpoint(value) {
        return Err(MediaReferenceError::BindingContainsLocalEndpoint { field });
    }
    Ok(())
}

fn contains_obvious_credential(value: &str) -> bool {
    const CREDENTIAL_MARKERS: &[&str] = &[
        "authorization:",
        "authorization=",
        "bearer ",
        "cookie=",
        "cookie:",
        "set-cookie",
        "password=",
        "password:",
        "passwd=",
        "passwd:",
        "secret=",
        "secret:",
        "token=",
        "token:",
        "access_token",
        "refresh_token",
        "api_key",
        "apikey=",
        "client_secret",
        "private_key",
        "credential=",
    ];

    let normalized = value.to_ascii_lowercase();
    CREDENTIAL_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
        || endpoint_has_user_info(&normalized)
}

fn endpoint_has_user_info(value: &str) -> bool {
    let Some((_, after_scheme)) = value.split_once("://") else {
        return false;
    };
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    authority.contains('@')
}

fn is_absolute_filesystem_path(value: &str) -> bool {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let bytes = value.as_bytes();

    value.starts_with('/')
        || value.starts_with('\\')
        || value.starts_with("~/")
        || value.starts_with("~\\")
        || lower.starts_with("file:")
        || matches!(bytes, [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic())
}

fn contains_local_or_lan_endpoint(value: &str) -> bool {
    let value = value.trim();
    let host = endpoint_host(value);
    host.is_some_and(is_local_or_lan_host)
}

fn endpoint_host(value: &str) -> Option<&str> {
    let endpoint = value
        .split_once("://")
        .map_or(value, |(_, after_scheme)| after_scheme);
    let authority = endpoint.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);

    if let Some(bracketed_host) = authority
        .strip_prefix('[')
        .and_then(|authority| authority.split_once(']').map(|(host, _)| host))
    {
        return Some(bracketed_host);
    }

    if authority.parse::<IpAddr>().is_ok() {
        return Some(authority);
    }

    Some(
        authority
            .split_once(':')
            .map_or(authority, |(host, _)| host),
    )
}

fn is_local_or_lan_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    let normalized = host.to_ascii_lowercase();
    if normalized == "localhost"
        || normalized.ends_with(".localhost")
        || normalized
            .rsplit_once('.')
            .is_some_and(|(_, label)| label == "local")
        || normalized == "host.docker.internal"
    {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => is_local_or_lan_ipv4(address),
        Ok(IpAddr::V6(address)) => is_local_or_lan_ipv6(address),
        Err(_) => false,
    }
}

fn is_local_or_lan_ipv4(address: Ipv4Addr) -> bool {
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
}

fn is_local_or_lan_ipv6(address: Ipv6Addr) -> bool {
    address.is_loopback()
        || address.is_unspecified()
        || address.is_unique_local()
        || address.is_unicast_link_local()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_id() -> ProviderPackageId {
        ProviderPackageId::new("com.example.provider").expect("valid package ID")
    }

    fn binding() -> ProviderBindingRef {
        ProviderBindingRef::new(package_id(), "public-config", "v1").expect("valid binding")
    }

    #[test]
    fn canonical_identity_rejects_a_binding() {
        let result = ExternalIdentity::new(
            "music.example",
            NamespaceScope::Canonical,
            ExternalEntityType::Recording,
            "track-1",
            Some(binding()),
        );

        assert_eq!(
            result,
            Err(MediaReferenceError::CanonicalIdentityHasBinding)
        );
    }

    #[test]
    fn binding_required_identity_rejects_a_missing_binding() {
        let result = ExternalIdentity::new(
            "bridge.example",
            NamespaceScope::BindingRequired,
            ExternalEntityType::Recording,
            "track-1",
            None,
        );

        assert_eq!(
            result,
            Err(MediaReferenceError::BindingRequiredIdentityHasNoBinding)
        );
    }

    #[test]
    fn binding_required_identity_keeps_its_portable_binding() {
        let expected_binding = binding();
        let identity = ExternalIdentity::binding_required(
            "bridge.example",
            ExternalEntityType::Recording,
            "track-1",
            expected_binding.clone(),
        )
        .expect("scope-consistent identity");

        assert_eq!(identity.binding(), Some(&expected_binding));
    }

    #[test]
    fn binding_rejects_obvious_credentials() {
        let result = ProviderBindingRef::new(package_id(), "access_token=secret", "v1");

        assert_eq!(
            result,
            Err(MediaReferenceError::BindingContainsCredential {
                field: "binding_key"
            })
        );
    }

    #[test]
    fn binding_rejects_absolute_paths() {
        let result = ProviderBindingRef::new(package_id(), r"C:\Users\example\config", "v1");

        assert_eq!(
            result,
            Err(MediaReferenceError::BindingContainsAbsolutePath {
                field: "binding_key"
            })
        );
    }

    #[test]
    fn binding_rejects_local_or_lan_endpoints() {
        let localhost = ProviderBindingRef::new(package_id(), "https://localhost:8080", "v1");
        let lan = ProviderBindingRef::new(package_id(), "http://192.168.1.4", "v1");
        let loopback_v6 = ProviderBindingRef::new(package_id(), "[::1]", "v1");

        assert!(matches!(
            localhost,
            Err(MediaReferenceError::BindingContainsLocalEndpoint { .. })
        ));
        assert!(matches!(
            lan,
            Err(MediaReferenceError::BindingContainsLocalEndpoint { .. })
        ));
        assert!(matches!(
            loopback_v6,
            Err(MediaReferenceError::BindingContainsLocalEndpoint { .. })
        ));
    }

    #[test]
    fn provider_media_references_require_recordings() {
        let release =
            ExternalIdentity::canonical("music.example", ExternalEntityType::Release, "release-1")
                .expect("valid release identity");

        let result = PortableMediaRef::provider_recording(release);

        assert_eq!(
            result,
            Err(
                MediaReferenceError::ProviderReferenceMustIdentifyRecording {
                    actual: ExternalEntityType::Release,
                }
            )
        );
    }

    #[test]
    fn provider_media_references_accept_recordings() {
        let recording = ExternalIdentity::canonical(
            "music.example",
            ExternalEntityType::Recording,
            "recording-1",
        )
        .expect("valid recording identity");

        let reference =
            PortableMediaRef::provider_recording(recording).expect("recording reference");

        assert!(matches!(reference, PortableMediaRef::ProviderRecording(_)));
    }

    #[test]
    fn portable_reference_fields_reject_blank_or_empty_values() {
        let blank_identity =
            ExternalIdentity::canonical("   ", ExternalEntityType::Recording, "recording-1");
        let blank_fingerprint = FingerprintRef::new("algorithm", "v1", "scope", " ");
        let empty_opaque = OpaquePortableRef::new(package_id(), "v1", Vec::new());

        assert_eq!(
            blank_identity,
            Err(MediaReferenceError::EmptyField { field: "namespace" })
        );
        assert_eq!(
            blank_fingerprint,
            Err(MediaReferenceError::EmptyField {
                field: "fingerprint"
            })
        );
        assert_eq!(
            empty_opaque,
            Err(MediaReferenceError::EmptyField { field: "payload" })
        );
    }

    #[test]
    fn hints_reject_blank_values() {
        let result = PortableMediaHints::new(Some("   ".to_owned()), None, None, None, None, None);

        assert_eq!(
            result,
            Err(MediaReferenceError::EmptyField { field: "title" })
        );
    }
}

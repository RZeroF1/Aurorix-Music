//! Opaque identifiers used by the portable domain model.

use std::{error::Error, fmt, str::FromStr};

use uuid::Uuid;

macro_rules! uuid_identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a new identifier with a UUID version 7 value.
            #[must_use]
            pub fn new_v7() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wraps a UUID without requiring a particular UUID version.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID wrapped by this identifier.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.as_uuid()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

uuid_identifier!(
    LocalCatalogEntityId,
    "Identifier for an entity in the local catalog."
);
uuid_identifier!(
    ReplicatedEntityId,
    "Identifier for an entity replicated between devices."
);
uuid_identifier!(
    ProviderInstallId,
    "Identifier for one installation of a Provider package."
);

/// An invalid [`ProviderPackageId`] value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPackageIdError {
    /// The supplied value has no non-whitespace characters.
    Empty,
}

impl fmt::Display for ProviderPackageIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("provider package ID must not be empty"),
        }
    }
}

impl Error for ProviderPackageIdError {}

/// A non-empty identifier for a Provider package.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderPackageId(String);

impl ProviderPackageId {
    /// Creates a package identifier after removing leading and trailing whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderPackageIdError::Empty`] when the value contains only whitespace.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ProviderPackageIdError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(ProviderPackageIdError::Empty);
        }

        Ok(Self(value.to_owned()))
    }

    /// Returns the normalized package identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderPackageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for ProviderPackageId {
    type Err = ProviderPackageIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ProviderPackageId {
    type Error = ProviderPackageIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ProviderPackageId {
    type Error = ProviderPackageIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use std::{any::TypeId, str::FromStr};

    use uuid::Uuid;

    use super::{
        LocalCatalogEntityId, ProviderInstallId, ProviderPackageId, ProviderPackageIdError,
        ReplicatedEntityId,
    };

    #[test]
    fn uuid_identifier_types_are_distinct() {
        assert_ne!(
            TypeId::of::<LocalCatalogEntityId>(),
            TypeId::of::<ReplicatedEntityId>()
        );
        assert_ne!(
            TypeId::of::<LocalCatalogEntityId>(),
            TypeId::of::<ProviderInstallId>()
        );
        assert_ne!(
            TypeId::of::<ReplicatedEntityId>(),
            TypeId::of::<ProviderInstallId>()
        );
    }

    #[test]
    fn uuid_identifier_strings_round_trip() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        let local = LocalCatalogEntityId::from_uuid(uuid);
        let replicated = ReplicatedEntityId::from_uuid(uuid);
        let provider_install = ProviderInstallId::from_uuid(uuid);

        assert_eq!(
            local.to_string().parse::<LocalCatalogEntityId>().unwrap(),
            local
        );
        assert_eq!(
            replicated
                .to_string()
                .parse::<ReplicatedEntityId>()
                .unwrap(),
            replicated
        );
        assert_eq!(
            provider_install
                .to_string()
                .parse::<ProviderInstallId>()
                .unwrap(),
            provider_install
        );
    }

    #[test]
    fn uuid_identifiers_accept_valid_non_v7_values() {
        let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();

        assert_eq!(
            LocalCatalogEntityId::from_str(&uuid.to_string())
                .unwrap()
                .as_uuid(),
            uuid
        );
        assert_eq!(ReplicatedEntityId::from_uuid(uuid).as_uuid(), uuid);
        assert_eq!(ProviderInstallId::from_uuid(uuid).as_uuid(), uuid);
    }

    #[test]
    fn uuid_identifier_constructors_generate_v7_values() {
        assert_eq!(
            LocalCatalogEntityId::new_v7().as_uuid().get_version_num(),
            7
        );
        assert_eq!(ReplicatedEntityId::new_v7().as_uuid().get_version_num(), 7);
        assert_eq!(ProviderInstallId::new_v7().as_uuid().get_version_num(), 7);
    }

    #[test]
    fn uuid_identifiers_reject_invalid_strings() {
        assert!("not-a-uuid".parse::<LocalCatalogEntityId>().is_err());
        assert!("not-a-uuid".parse::<ReplicatedEntityId>().is_err());
        assert!("not-a-uuid".parse::<ProviderInstallId>().is_err());
    }

    #[test]
    fn provider_package_id_trims_and_round_trips() {
        let package_id = ProviderPackageId::new("  org.aurorix.example  ").unwrap();

        assert_eq!(package_id.as_str(), "org.aurorix.example");
        assert_eq!(package_id.to_string(), "org.aurorix.example");
        assert_eq!(
            package_id.to_string().parse::<ProviderPackageId>().unwrap(),
            package_id
        );
    }

    #[test]
    fn provider_package_id_rejects_whitespace_only_values() {
        assert_eq!(
            ProviderPackageId::new(" \t\n "),
            Err(ProviderPackageIdError::Empty)
        );
    }
}

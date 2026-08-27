//! Deterministic, in-memory Sync v2 reducer oracle.
//!
//! This is a contract-level reducer for local tests. It accepts typed intents,
//! assigns monotonically increasing canonical revisions, preserves tombstones,
//! and records operation digests for idempotent retries. It has no transport,
//! credentials, or database ownership.

#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use uuid::Uuid;

use crate::outbox::OperationDigest;

/// A typed Sync v2 mutation supported by the reducer oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    PlaylistCreate {
        name: String,
    },
    PlaylistRename {
        name: String,
    },
    PlaylistDelete,
    FavoriteSet {
        is_favorite: bool,
    },
    SettingPatch {
        key: String,
        value: String,
    },
    PlayFactFinalize,
    PlaylistItemAdd {
        item_id: Uuid,
        media_key: String,
    },
    PlaylistItemRemove {
        item_id: Uuid,
    },
    PlaylistItemMove {
        item_id: Uuid,
        before: Option<Uuid>,
        after: Option<Uuid>,
    },
}

/// One reducer input. `bytes` must be the exact serialized operation bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub operation_id: Uuid,
    pub entity_id: Uuid,
    pub bytes: Vec<u8>,
    pub base_entity_version: Option<u64>,
    pub mutation: Mutation,
}

/// Canonical result status for one operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultStatus {
    Accepted,
    Duplicate,
}

/// A compact canonical result exposed by the oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalResult {
    pub operation_id: Uuid,
    pub status: ResultStatus,
    pub revision: u64,
    pub entity_id: Uuid,
    pub version: u64,
    pub deleted: bool,
}

/// Reducer rejection or idempotency failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReducerError {
    IdempotencyKeyReused { operation_id: Uuid },
    EntityDeleted { entity_id: Uuid },
    MissingPlaylist { playlist_id: Uuid },
    MissingItem { item_id: Uuid },
    MoveRequiresBaseVersion,
    InvalidName,
}

impl fmt::Display for ReducerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdempotencyKeyReused { operation_id } => {
                write!(f, "operation {operation_id} reused with different bytes")
            }
            Self::EntityDeleted { entity_id } => write!(f, "entity {entity_id} is deleted"),
            Self::MissingPlaylist { playlist_id } => {
                write!(f, "playlist {playlist_id} does not exist")
            }
            Self::MissingItem { item_id } => write!(f, "playlist item {item_id} does not exist"),
            Self::MoveRequiresBaseVersion => {
                f.write_str("playlist item move requires a base entity version")
            }
            Self::InvalidName => f.write_str("playlist name must not be empty"),
        }
    }
}
impl Error for ReducerError {}

#[derive(Debug, Clone)]
struct Playlist {
    version: u64,
    name: String,
    deleted: bool,
    items: Vec<Uuid>,
    item_media: BTreeMap<Uuid, String>,
}

/// A deterministic materialized Sync state.
#[derive(Debug, Default)]
pub struct Reducer {
    revision: u64,
    operations: BTreeMap<Uuid, (OperationDigest, CanonicalResult)>,
    playlists: BTreeMap<Uuid, Playlist>,
    favorites: BTreeMap<Uuid, bool>,
    settings: BTreeMap<String, String>,
    play_facts: BTreeSet<Uuid>,
}

impl Reducer {
    /// Applies one operation, assigning a new canonical revision on success.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection when the operation reuses an ID with different
    /// bytes, targets a tombstone, or violates a mutation precondition.
    pub fn apply(&mut self, operation: Operation) -> Result<CanonicalResult, ReducerError> {
        let digest = OperationDigest::compute(&operation.bytes);
        if let Some((retained, result)) = self.operations.get(&operation.operation_id) {
            if *retained == digest {
                return Ok(CanonicalResult {
                    status: ResultStatus::Duplicate,
                    ..result.clone()
                });
            }
            return Err(ReducerError::IdempotencyKeyReused {
                operation_id: operation.operation_id,
            });
        }

        self.validate(&operation)?;
        self.revision = self.revision.saturating_add(1);
        let result = self.mutate(operation.entity_id, &operation.mutation, self.revision)?;
        let result = CanonicalResult {
            operation_id: operation.operation_id,
            status: ResultStatus::Accepted,
            revision: self.revision,
            entity_id: operation.entity_id,
            version: result.0,
            deleted: result.1,
        };
        self.operations
            .insert(operation.operation_id, (digest, result.clone()));
        Ok(result)
    }

    /// Returns the latest canonical revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    /// Returns a playlist's canonical item order.
    #[must_use]
    pub fn playlist_items(&self, id: Uuid) -> Option<&[Uuid]> {
        self.playlists.get(&id).map(|p| p.items.as_slice())
    }
    /// Returns a cloud setting value.
    #[must_use]
    pub fn setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(String::as_str)
    }

    fn validate(&self, op: &Operation) -> Result<(), ReducerError> {
        if matches!(op.mutation, Mutation::PlaylistItemMove { .. })
            && op.base_entity_version.is_none()
        {
            return Err(ReducerError::MoveRequiresBaseVersion);
        }
        let entity_deleted = self
            .playlists
            .get(&op.entity_id)
            .is_some_and(|p| p.deleted && !matches!(op.mutation, Mutation::PlaylistCreate { .. }));
        if entity_deleted {
            return Err(ReducerError::EntityDeleted {
                entity_id: op.entity_id,
            });
        }
        if matches!(&op.mutation, Mutation::PlaylistRename { name } | Mutation::PlaylistCreate { name } if name.trim().is_empty())
        {
            return Err(ReducerError::InvalidName);
        }
        Ok(())
    }

    fn mutate(
        &mut self,
        entity_id: Uuid,
        mutation: &Mutation,
        revision: u64,
    ) -> Result<(u64, bool), ReducerError> {
        match mutation {
            Mutation::PlaylistCreate { name } => {
                self.playlists.entry(entity_id).or_insert_with(|| Playlist {
                    version: 0,
                    name: name.clone(),
                    deleted: false,
                    items: Vec::new(),
                    item_media: BTreeMap::new(),
                });
                let p = self.playlists.get_mut(&entity_id).expect("inserted");
                p.version += 1;
                p.name.clone_from(name);
                Ok((p.version, p.deleted))
            }
            Mutation::PlaylistRename { name } => {
                let p =
                    self.playlists
                        .get_mut(&entity_id)
                        .ok_or(ReducerError::MissingPlaylist {
                            playlist_id: entity_id,
                        })?;
                p.version += 1;
                p.name.clone_from(name);
                Ok((p.version, p.deleted))
            }
            Mutation::PlaylistDelete => {
                let p =
                    self.playlists
                        .get_mut(&entity_id)
                        .ok_or(ReducerError::MissingPlaylist {
                            playlist_id: entity_id,
                        })?;
                p.version += 1;
                p.deleted = true;
                Ok((p.version, true))
            }
            Mutation::FavoriteSet { is_favorite } => {
                self.favorites.insert(entity_id, *is_favorite);
                Ok((revision, false))
            }
            Mutation::SettingPatch { key, value } => {
                self.settings.insert(key.clone(), value.clone());
                Ok((revision, false))
            }
            Mutation::PlayFactFinalize => {
                self.play_facts.insert(entity_id);
                Ok((revision, false))
            }
            Mutation::PlaylistItemAdd { item_id, media_key } => {
                let p =
                    self.playlists
                        .get_mut(&entity_id)
                        .ok_or(ReducerError::MissingPlaylist {
                            playlist_id: entity_id,
                        })?;
                p.items.push(*item_id);
                p.item_media.insert(*item_id, media_key.clone());
                p.version += 1;
                Ok((p.version, false))
            }
            Mutation::PlaylistItemRemove { item_id } => {
                let p =
                    self.playlists
                        .get_mut(&entity_id)
                        .ok_or(ReducerError::MissingPlaylist {
                            playlist_id: entity_id,
                        })?;
                if !p.item_media.contains_key(item_id) {
                    return Err(ReducerError::MissingItem { item_id: *item_id });
                }
                p.items.retain(|id| id != item_id);
                p.item_media.remove(item_id);
                p.version += 1;
                Ok((p.version, false))
            }
            Mutation::PlaylistItemMove {
                item_id,
                before,
                after,
            } => {
                let p =
                    self.playlists
                        .get_mut(&entity_id)
                        .ok_or(ReducerError::MissingPlaylist {
                            playlist_id: entity_id,
                        })?;
                if !p.item_media.contains_key(item_id) {
                    return Err(ReducerError::MissingItem { item_id: *item_id });
                }
                p.items.retain(|id| id != item_id);
                let index = before
                    .and_then(|id| p.items.iter().position(|candidate| candidate == &id))
                    .or_else(|| {
                        after.and_then(|id| {
                            p.items
                                .iter()
                                .position(|candidate| candidate == &id)
                                .map(|i| i + 1)
                        })
                    })
                    .unwrap_or(p.items.len());
                p.items.insert(index, *item_id);
                p.version += 1;
                Ok((p.version, false))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn op(id: u128, entity: u128, bytes: &[u8], mutation: Mutation) -> Operation {
        Operation {
            operation_id: Uuid::from_u128(id),
            entity_id: Uuid::from_u128(entity),
            bytes: bytes.to_vec(),
            base_entity_version: None,
            mutation,
        }
    }
    #[test]
    fn duplicate_is_idempotent_and_reuse_is_rejected() {
        let mut r = Reducer::default();
        let first = op(1, 2, b"a", Mutation::FavoriteSet { is_favorite: true });
        let accepted = r.apply(first.clone()).unwrap();
        assert_eq!(r.apply(first).unwrap().status, ResultStatus::Duplicate);
        let reused = op(1, 2, b"b", Mutation::FavoriteSet { is_favorite: false });
        assert!(matches!(
            r.apply(reused),
            Err(ReducerError::IdempotencyKeyReused { .. })
        ));
        assert_eq!(accepted.revision, 1);
    }
    #[test]
    fn delete_tombstone_rejects_later_rename() {
        let mut r = Reducer::default();
        r.apply(op(
            1,
            2,
            b"create",
            Mutation::PlaylistCreate { name: "A".into() },
        ))
        .unwrap();
        r.apply(op(2, 2, b"delete", Mutation::PlaylistDelete))
            .unwrap();
        assert!(matches!(
            r.apply(op(
                3,
                2,
                b"rename",
                Mutation::PlaylistRename { name: "B".into() }
            )),
            Err(ReducerError::EntityDeleted { .. })
        ));
    }
    #[test]
    fn playlist_move_uses_surviving_anchor_then_end() {
        let mut r = Reducer::default();
        r.apply(op(
            1,
            2,
            b"create",
            Mutation::PlaylistCreate { name: "A".into() },
        ))
        .unwrap();
        r.apply(op(
            2,
            2,
            b"add-a",
            Mutation::PlaylistItemAdd {
                item_id: Uuid::from_u128(10),
                media_key: "a".into(),
            },
        ))
        .unwrap();
        r.apply(op(
            3,
            2,
            b"add-b",
            Mutation::PlaylistItemAdd {
                item_id: Uuid::from_u128(11),
                media_key: "b".into(),
            },
        ))
        .unwrap();
        let mut move_op = op(
            4,
            2,
            b"move",
            Mutation::PlaylistItemMove {
                item_id: Uuid::from_u128(10),
                before: Some(Uuid::from_u128(11)),
                after: None,
            },
        );
        move_op.base_entity_version = Some(3);
        r.apply(move_op).unwrap();
        assert_eq!(
            r.playlist_items(Uuid::from_u128(2)).unwrap(),
            &[Uuid::from_u128(10), Uuid::from_u128(11)]
        );
    }
}

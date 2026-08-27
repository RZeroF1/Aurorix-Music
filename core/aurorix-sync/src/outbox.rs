//! Deterministic, in-memory ownership of Sync v2 outbox operations.
//!
//! This module deliberately stores validated operation bytes without parsing or
//! transforming them. Schema validation and Sync reducers belong to their own
//! boundaries; preserving these bytes is what makes an `operation_id` a safe
//! idempotency key.

#![allow(clippy::many_single_char_names, clippy::too_many_lines)]

use std::{collections::BTreeMap, error::Error, fmt, sync::Arc};

/// The byte length of a SHA-256 operation digest.
pub const OPERATION_DIGEST_LEN: usize = 32;

/// A deterministic SHA-256 digest of the exact bytes sent for an operation.
///
/// The digest is an identity check, not a signature or a replacement for
/// transport authentication.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationDigest([u8; OPERATION_DIGEST_LEN]);

impl OperationDigest {
    /// Computes the SHA-256 digest of `operation_bytes`.
    #[must_use]
    pub fn compute(operation_bytes: &[u8]) -> Self {
        Self(sha256(operation_bytes))
    }

    /// Returns the raw SHA-256 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; OPERATION_DIGEST_LEN] {
        &self.0
    }

    /// Returns the lowercase hexadecimal representation used by Sync v2
    /// digest-bearing contracts.
    #[must_use]
    pub fn to_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(OPERATION_DIGEST_LEN * 2);
        for byte in self.0 {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        output
    }
}

impl fmt::Debug for OperationDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("OperationDigest")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for OperationDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// One immutable operation accepted into the local outbox.
///
/// `operation_id` is opaque here because the Sync operation schema owns UUID
/// validation. Its bytes and computed digest cannot be changed after creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxOperation {
    operation_id: String,
    operation_bytes: Arc<[u8]>,
    digest: OperationDigest,
}

impl OutboxOperation {
    /// Captures the exact serialized bytes associated with `operation_id`.
    #[must_use]
    pub fn new(operation_id: impl Into<String>, operation_bytes: impl Into<Vec<u8>>) -> Self {
        let operation_bytes: Arc<[u8]> = Arc::from(operation_bytes.into());
        let digest = OperationDigest::compute(&operation_bytes);

        Self {
            operation_id: operation_id.into(),
            operation_bytes,
            digest,
        }
    }

    /// Returns the schema-owned idempotency key.
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Returns the immutable bytes whose digest is recorded for this operation.
    #[must_use]
    pub fn operation_bytes(&self) -> &[u8] {
        &self.operation_bytes
    }

    /// Returns the digest of [`Self::operation_bytes`].
    #[must_use]
    pub const fn digest(&self) -> OperationDigest {
        self.digest
    }
}

/// Durable lifecycle state for an outbox operation.
///
/// Archived entries retain their original operation bytes and digest. They are
/// deliberately not removed by this in-memory model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// The operation has not yet received a canonical acknowledgement.
    Pending,
    /// The operation's canonical result was committed locally.
    Acknowledged,
    /// The operation is retained as immutable local archive evidence.
    Archived,
}

/// Alias for callers that describe the lifecycle as a status.
pub type OutboxStatus = OutboxState;

/// An outbox operation together with its current local lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    operation: OutboxOperation,
    state: OutboxState,
}

impl OutboxEntry {
    /// Returns the immutable operation retained by this entry.
    #[must_use]
    pub fn operation(&self) -> &OutboxOperation {
        &self.operation
    }

    /// Returns this entry's lifecycle state.
    #[must_use]
    pub const fn state(&self) -> OutboxState {
        self.state
    }
}

/// Result of adding an operation to an [`Outbox`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    /// A new pending operation was retained.
    Enqueued,
    /// The exact same operation was already retained and was not duplicated.
    AlreadyPresent { state: OutboxState },
}

/// Failure while retaining or transitioning an outbox operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboxError {
    /// A caller reused an idempotency key for different exact operation bytes.
    IdempotencyKeyReused {
        /// The immutable idempotency key that was reused.
        operation_id: String,
        /// Digest of the operation first retained under the key.
        retained_digest: OperationDigest,
        /// Digest of the later, incompatible operation.
        supplied_digest: OperationDigest,
    },
    /// The requested idempotency key is not present in this outbox.
    UnknownOperationId { operation_id: String },
    /// An archived operation cannot be marked acknowledged again.
    ArchivedOperationCannotBeAcknowledged { operation_id: String },
}

impl fmt::Display for OutboxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdempotencyKeyReused { operation_id, .. } => write!(
                formatter,
                "operation ID {operation_id:?} was reused with different immutable bytes"
            ),
            Self::UnknownOperationId { operation_id } => {
                write!(formatter, "unknown operation ID {operation_id:?}")
            }
            Self::ArchivedOperationCannotBeAcknowledged { operation_id } => write!(
                formatter,
                "archived operation ID {operation_id:?} cannot be acknowledged"
            ),
        }
    }
}

impl Error for OutboxError {}

/// An insertion-ordered outbox with permanent idempotency-key records.
///
/// `entries` preserves local command order for deterministic send batches,
/// while `positions` provides idempotency-key lookup without exposing mutable
/// operation bytes.
#[derive(Debug, Default)]
pub struct Outbox {
    entries: Vec<OutboxEntry>,
    positions: BTreeMap<String, usize>,
}

impl Outbox {
    /// Creates an empty outbox.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of retained entries, including archived operations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether no operations have been retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Retains `operation_bytes` under `operation_id` as a pending operation.
    ///
    /// Repeating the exact same bytes is a successful idempotent no-op. A key
    /// reused with any different bytes returns
    /// [`OutboxError::IdempotencyKeyReused`] and leaves the first bytes intact.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::IdempotencyKeyReused`] when an existing operation
    /// ID is paired with different immutable bytes.
    pub fn enqueue(
        &mut self,
        operation_id: impl Into<String>,
        operation_bytes: impl Into<Vec<u8>>,
    ) -> Result<EnqueueResult, OutboxError> {
        let operation = OutboxOperation::new(operation_id, operation_bytes);
        if let Some(&position) = self.positions.get(operation.operation_id()) {
            let retained = &self.entries[position];
            if retained.operation.digest == operation.digest
                && retained.operation.operation_bytes == operation.operation_bytes
            {
                return Ok(EnqueueResult::AlreadyPresent {
                    state: retained.state,
                });
            }

            return Err(OutboxError::IdempotencyKeyReused {
                operation_id: operation.operation_id,
                retained_digest: retained.operation.digest,
                supplied_digest: operation.digest,
            });
        }

        let position = self.entries.len();
        self.positions
            .insert(operation.operation_id.clone(), position);
        self.entries.push(OutboxEntry {
            operation,
            state: OutboxState::Pending,
        });
        Ok(EnqueueResult::Enqueued)
    }

    /// Returns the retained entry for `operation_id`.
    #[must_use]
    pub fn entry(&self, operation_id: &str) -> Option<&OutboxEntry> {
        self.positions
            .get(operation_id)
            .map(|&position| &self.entries[position])
    }

    /// Iterates every retained entry in its original local-command order.
    #[must_use]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &OutboxEntry> {
        self.entries.iter()
    }

    /// Iterates pending entries in their original local-command order.
    pub fn pending(&self) -> impl Iterator<Item = &OutboxEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.state == OutboxState::Pending)
    }

    /// Marks a pending entry acknowledged after its canonical result commits.
    ///
    /// A repeated acknowledgement is an idempotent no-op and returns `false`.
    /// Returning `true` means the state changed from pending to acknowledged.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::UnknownOperationId`] when no matching entry is
    /// retained, or [`OutboxError::ArchivedOperationCannotBeAcknowledged`] when
    /// the operation has already been archived.
    pub fn acknowledge(&mut self, operation_id: &str) -> Result<bool, OutboxError> {
        let entry = self.entry_mut(operation_id)?;
        match entry.state {
            OutboxState::Pending => {
                entry.state = OutboxState::Acknowledged;
                Ok(true)
            }
            OutboxState::Acknowledged => Ok(false),
            OutboxState::Archived => Err(OutboxError::ArchivedOperationCannotBeAcknowledged {
                operation_id: operation_id.to_owned(),
            }),
        }
    }

    /// Retains an operation as archived evidence and removes it from send work.
    ///
    /// Pending operations may be archived for an explicit rebase outcome, and
    /// acknowledged operations may be archived after the relevant retention
    /// policy permits it. No operation bytes are deleted. Repeating the same
    /// archive transition is an idempotent no-op and returns `false`.
    ///
    /// # Errors
    ///
    /// Returns [`OutboxError::UnknownOperationId`] when no matching entry is
    /// retained.
    pub fn archive(&mut self, operation_id: &str) -> Result<bool, OutboxError> {
        let entry = self.entry_mut(operation_id)?;
        if entry.state == OutboxState::Archived {
            return Ok(false);
        }

        entry.state = OutboxState::Archived;
        Ok(true)
    }

    fn entry_mut(&mut self, operation_id: &str) -> Result<&mut OutboxEntry, OutboxError> {
        let Some(&position) = self.positions.get(operation_id) else {
            return Err(OutboxError::UnknownOperationId {
                operation_id: operation_id.to_owned(),
            });
        };
        Ok(&mut self.entries[position])
    }
}

fn sha256(input: &[u8]) -> [u8; OPERATION_DIGEST_LEN] {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const ROUND_CONSTANTS: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_length = u64::try_from(input.len())
        .expect("usize must fit into u64 on supported Rust targets")
        .checked_mul(8)
        .expect("SHA-256 input length exceeds the format limit");
    let padding_len = (64 - ((input.len() + 1 + 8) % 64)) % 64;
    let mut padded = Vec::with_capacity(input.len() + 1 + padding_len + 8);
    padded.extend_from_slice(input);
    padded.push(0x80);
    padded.resize(input.len() + 1 + padding_len, 0);
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let mut hash = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0_u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let small_sigma_0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let small_sigma_1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(small_sigma_0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(small_sigma_1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let big_sigma_1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temporary_1 = h
                .wrapping_add(big_sigma_1)
                .wrapping_add(choose)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(schedule[index]);
            let big_sigma_0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary_2 = big_sigma_0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary_1);
            d = c;
            c = b;
            b = a;
            a = temporary_1.wrapping_add(temporary_2);
        }

        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut output = [0_u8; OPERATION_DIGEST_LEN];
    for (index, word) in hash.iter().enumerate() {
        output[index * 4..(index + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{EnqueueResult, OperationDigest, Outbox, OutboxError, OutboxState};

    #[test]
    fn digest_is_sha256_of_exact_operation_bytes() {
        assert_eq!(
            OperationDigest::compute(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            OperationDigest::compute(b"abc").to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            OperationDigest::compute(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")
                .to_hex(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_ne!(
            OperationDigest::compute(br#"{"a":1}"#),
            OperationDigest::compute(br#"{ "a": 1 }"#)
        );
    }

    #[test]
    fn repeated_identical_operation_is_not_enqueued_twice() {
        let mut outbox = Outbox::new();
        let bytes = br#"{"operation_id":"op-1","payload":{"name":"A"}}"#;

        assert_eq!(
            outbox.enqueue("op-1", bytes.as_slice()),
            Ok(EnqueueResult::Enqueued)
        );
        assert_eq!(
            outbox.enqueue("op-1", bytes.as_slice()),
            Ok(EnqueueResult::AlreadyPresent {
                state: OutboxState::Pending,
            })
        );
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox.pending().count(), 1);
    }

    #[test]
    fn reused_id_with_different_bytes_is_rejected_without_replacing_original() {
        let mut outbox = Outbox::new();
        let first = br#"{"operation_id":"op-1","payload":{"name":"A"}}"#;
        let replacement = br#"{"operation_id":"op-1","payload":{"name":"B"}}"#;
        outbox.enqueue("op-1", first.as_slice()).unwrap();

        let error = outbox.enqueue("op-1", replacement.as_slice()).unwrap_err();
        assert!(matches!(error, OutboxError::IdempotencyKeyReused { .. }));
        let retained = outbox.entry("op-1").unwrap();
        assert_eq!(retained.operation().operation_bytes(), first);
        assert_eq!(retained.state(), OutboxState::Pending);
    }

    #[test]
    fn lifecycle_keeps_archived_bytes_and_excludes_them_from_pending_work() {
        let mut outbox = Outbox::new();
        let first = br#"{"operation_id":"op-1"}"#;
        let second = br#"{"operation_id":"op-2"}"#;
        outbox.enqueue("op-1", first.as_slice()).unwrap();
        outbox.enqueue("op-2", second.as_slice()).unwrap();

        assert!(outbox.acknowledge("op-1").unwrap());
        assert!(outbox.archive("op-1").unwrap());
        assert!(outbox.archive("op-2").unwrap());
        assert!(!outbox.archive("op-2").unwrap());

        assert_eq!(outbox.pending().count(), 0);
        assert_eq!(outbox.entry("op-1").unwrap().state(), OutboxState::Archived);
        assert_eq!(
            outbox.entry("op-1").unwrap().operation().operation_bytes(),
            first
        );
        assert_eq!(
            outbox.entry("op-2").unwrap().operation().operation_bytes(),
            second
        );
        assert!(matches!(
            outbox.acknowledge("op-1"),
            Err(OutboxError::ArchivedOperationCannotBeAcknowledged { .. })
        ));
    }
}

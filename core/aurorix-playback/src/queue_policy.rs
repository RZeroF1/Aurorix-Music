//! Deterministic queue policy for one local playback session.
//!
//! The queue owns only Core playback identities.  It does not retain a path,
//! descriptor, URL, credential, decoder, or runtime source capability.

use crate::command::{PlaybackItemId, RepeatMode};

/// The threshold after which `previous` restarts the current item instead of
/// selecting the preceding item.
pub const PREVIOUS_RESTART_THRESHOLD_US: u64 = 3_000_000;

/// A queue transition selected by the policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueTransition {
    /// A queue item became current.
    Selected {
        /// The selected Core identity.
        item_id: PlaybackItemId,
        /// The position in the canonical queue order.
        canonical_index: usize,
    },
    /// The current item should be restarted from its beginning.
    RestartCurrent {
        /// The current Core identity.
        item_id: PlaybackItemId,
        /// The position in the canonical queue order.
        canonical_index: usize,
    },
    /// The queue has no next item under the active policy.
    EndOfQueue,
    /// The queue has no items to select.
    Empty,
}

impl QueueTransition {
    /// Returns the selected identity, including a restart transition.
    #[must_use]
    pub fn item_id(&self) -> Option<&PlaybackItemId> {
        match self {
            Self::Selected { item_id, .. } | Self::RestartCurrent { item_id, .. } => Some(item_id),
            Self::EndOfQueue | Self::Empty => None,
        }
    }

    /// Returns the canonical index selected by the transition.
    #[must_use]
    pub const fn canonical_index(&self) -> Option<usize> {
        match self {
            Self::Selected {
                canonical_index, ..
            }
            | Self::RestartCurrent {
                canonical_index, ..
            } => Some(*canonical_index),
            Self::EndOfQueue | Self::Empty => None,
        }
    }

    /// Reports whether this transition ended the queue.
    #[must_use]
    pub const fn is_end_of_queue(&self) -> bool {
        matches!(self, Self::EndOfQueue)
    }
}

/// A stable snapshot of queue policy state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePolicySnapshot {
    items: Vec<PlaybackItemId>,
    current_index: Option<usize>,
    shuffle_enabled: bool,
    shuffle_seed: u64,
    repeat_mode: RepeatMode,
}

impl QueuePolicySnapshot {
    /// Returns the canonical queue identities in insertion order.
    #[must_use]
    pub fn items(&self) -> &[PlaybackItemId] {
        &self.items
    }

    /// Returns the current canonical index, when selected.
    #[must_use]
    pub const fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Returns whether deterministic shuffle is enabled.
    #[must_use]
    pub const fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    /// Returns the seed used by the deterministic permutation.
    #[must_use]
    pub const fn shuffle_seed(&self) -> u64 {
        self.shuffle_seed
    }

    /// Returns the active repeat policy.
    #[must_use]
    pub const fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    /// Returns the canonical identities in the effective playback order.
    #[must_use]
    pub fn playback_order(&self) -> Vec<PlaybackItemId> {
        let indices = effective_indices(self.items.len(), self.shuffle_enabled, self.shuffle_seed);
        indices
            .into_iter()
            .map(|index| self.items[index].clone())
            .collect()
    }
}

/// Deterministic queue and repeat policy for one playback session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePolicy {
    items: Vec<PlaybackItemId>,
    current_index: Option<usize>,
    shuffle_enabled: bool,
    shuffle_seed: u64,
    repeat_mode: RepeatMode,
}

impl QueuePolicy {
    /// Creates an empty queue with shuffle disabled and repeat off.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            items: Vec::new(),
            current_index: None,
            shuffle_enabled: false,
            shuffle_seed: 0,
            repeat_mode: RepeatMode::Off,
        }
    }

    /// Creates a queue from canonical identities and an optional current item.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::CurrentIndexOutOfBounds`] when the supplied index
    /// is not present in `items`.
    pub fn from_items(
        items: Vec<PlaybackItemId>,
        current_index: Option<usize>,
    ) -> Result<Self, QueueError> {
        if let Some(index) = current_index
            && index >= items.len()
        {
            return Err(QueueError::CurrentIndexOutOfBounds {
                index,
                length: items.len(),
            });
        }
        Ok(Self {
            items,
            current_index,
            ..Self::new()
        })
    }

    /// Returns a bounded latest-value snapshot of the policy state.
    #[must_use]
    pub fn snapshot(&self) -> QueuePolicySnapshot {
        QueuePolicySnapshot {
            items: self.items.clone(),
            current_index: self.current_index,
            shuffle_enabled: self.shuffle_enabled,
            shuffle_seed: self.shuffle_seed,
            repeat_mode: self.repeat_mode,
        }
    }

    /// Returns the canonical queue identities in insertion order.
    #[must_use]
    pub fn items(&self) -> &[PlaybackItemId] {
        &self.items
    }

    /// Returns the current canonical index, when selected.
    #[must_use]
    pub const fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Returns the current identity, when selected.
    #[must_use]
    pub fn current_item(&self) -> Option<&PlaybackItemId> {
        self.current_index.map(|index| &self.items[index])
    }

    /// Returns the active repeat mode.
    #[must_use]
    pub const fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    /// Returns whether shuffle is enabled.
    #[must_use]
    pub const fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    /// Returns the deterministic shuffle seed.
    #[must_use]
    pub const fn shuffle_seed(&self) -> u64 {
        self.shuffle_seed
    }

    /// Replaces queue intent and optionally selects an item in the new queue.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::CurrentIndexOutOfBounds`] when the supplied index
    /// is not present in `items`.
    pub fn set_items(
        &mut self,
        items: Vec<PlaybackItemId>,
        current_index: Option<usize>,
    ) -> Result<(), QueueError> {
        if let Some(index) = current_index
            && index >= items.len()
        {
            return Err(QueueError::CurrentIndexOutOfBounds {
                index,
                length: items.len(),
            });
        }
        self.items = items;
        self.current_index = current_index;
        Ok(())
    }

    /// Selects a canonical item without changing the queue order.
    ///
    /// # Errors
    ///
    /// Returns [`QueueError::ItemNotFound`] when `item_id` is not in the queue.
    pub fn select_item(&mut self, item_id: &PlaybackItemId) -> Result<(), QueueError> {
        let Some(index) = self.items.iter().position(|item| item == item_id) else {
            return Err(QueueError::ItemNotFound);
        };
        self.current_index = Some(index);
        Ok(())
    }

    /// Enables or disables shuffle while retaining the current identity.
    pub fn set_shuffle(&mut self, enabled: bool, seed: u64) {
        self.shuffle_enabled = enabled;
        self.shuffle_seed = seed;
    }

    /// Changes repeat policy without changing the current identity.
    pub const fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat_mode = mode;
    }

    /// Returns the canonical indices in the effective deterministic order.
    #[must_use]
    pub fn effective_indices(&self) -> Vec<usize> {
        effective_indices(self.items.len(), self.shuffle_enabled, self.shuffle_seed)
    }

    /// Selects the next item for an explicit `next` command.
    ///
    /// Explicit next always advances to the following item.  At the end of a
    /// queue it wraps only for [`RepeatMode::All`]; [`RepeatMode::One`] is
    /// applied by [`Self::on_item_completed`], not by a user skip command.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> QueueTransition {
        self.advance()
    }

    /// Applies normal item completion and selects the policy-defined result.
    ///
    /// Repeat one selects the same item again.  Repeat all wraps to the first
    /// effective item.  Repeat off reports the end without removing identity.
    #[must_use]
    pub fn on_item_completed(&mut self) -> QueueTransition {
        if self.items.is_empty() || self.current_index.is_none() {
            return QueueTransition::Empty;
        }
        if self.repeat_mode == RepeatMode::One {
            return self.current_transition(true);
        }
        self.advance()
    }

    /// Selects the preceding item in effective order.
    ///
    /// At the first item, repeat all wraps to the final item. Other modes
    /// restart the first item because a previous command never ends playback.
    #[must_use]
    pub fn previous(&mut self) -> QueueTransition {
        if self.items.is_empty() {
            return QueueTransition::Empty;
        }
        let order = self.effective_indices();
        let current_position = self.current_position(&order);
        let target_position = if current_position == 0 {
            if self.repeat_mode == RepeatMode::All {
                order.len() - 1
            } else {
                return self.current_transition(true);
            }
        } else {
            current_position - 1
        };
        self.current_index = Some(order[target_position]);
        self.current_transition(false)
    }

    /// Applies the conventional previous-button restart threshold.
    #[must_use]
    pub fn previous_at(&mut self, position_us: u64) -> QueueTransition {
        if self.items.is_empty() {
            return QueueTransition::Empty;
        }
        if self.current_index.is_none() {
            return self.next();
        }
        if position_us > PREVIOUS_RESTART_THRESHOLD_US {
            return self.current_transition(true);
        }
        let transition = self.previous();
        if transition.is_end_of_queue() {
            self.current_transition(true)
        } else {
            transition
        }
    }

    fn advance(&mut self) -> QueueTransition {
        if self.items.is_empty() {
            return QueueTransition::Empty;
        }
        let order = self.effective_indices();
        if self.current_index.is_none() {
            self.current_index = Some(order[0]);
            return self.current_transition(false);
        }
        let current_position = self.current_position(&order);
        if let Some(next_position) = current_position.checked_add(1)
            && next_position < order.len()
        {
            self.current_index = Some(order[next_position]);
            return self.current_transition(false);
        }

        if self.repeat_mode == RepeatMode::All {
            self.current_index = Some(order[0]);
            return self.current_transition(false);
        }
        QueueTransition::EndOfQueue
    }

    fn current_position(&self, order: &[usize]) -> usize {
        self.current_index
            .and_then(|current| order.iter().position(|index| *index == current))
            .unwrap_or(0)
    }

    fn current_transition(&self, restart: bool) -> QueueTransition {
        let index = self.current_index.unwrap_or(0);
        let item_id = self.items[index].clone();
        if restart {
            QueueTransition::RestartCurrent {
                item_id,
                canonical_index: index,
            }
        } else {
            QueueTransition::Selected {
                item_id,
                canonical_index: index,
            }
        }
    }
}

impl Default for QueuePolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Failure while constructing or mutating queue intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    /// The current index was outside the replacement queue.
    CurrentIndexOutOfBounds {
        /// The rejected index.
        index: usize,
        /// The replacement queue length.
        length: usize,
    },
    /// A requested identity was not present in the queue.
    ItemNotFound,
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CurrentIndexOutOfBounds { index, length } => {
                write!(
                    formatter,
                    "queue current index {index} is outside length {length}"
                )
            }
            Self::ItemNotFound => formatter.write_str("queue item was not found"),
        }
    }
}

impl std::error::Error for QueueError {}

fn effective_indices(length: usize, shuffle_enabled: bool, seed: u64) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..length).collect();
    if !shuffle_enabled {
        return indices;
    }

    // Fisher-Yates with SplitMix64 makes the permutation independent of the
    // process hash seed and of the platform's random-number implementation.
    let mut state = seed;
    for index in (1..length).rev() {
        let random = splitmix64(&mut state);
        let swap_index = usize::try_from(random % (index as u64 + 1)).unwrap_or_default();
        indices.swap(index, swap_index);
    }
    indices
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::{PREVIOUS_RESTART_THRESHOLD_US, QueueError, QueuePolicy, QueueTransition};
    use crate::command::{PlaybackItemId, RepeatMode};

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("test identity is valid")
    }

    fn queue() -> QueuePolicy {
        QueuePolicy::from_items(vec![item("a"), item("b"), item("c")], Some(0))
            .expect("queue is valid")
    }

    #[test]
    fn same_seed_produces_same_order() {
        let mut left = queue();
        let mut right = queue();
        left.set_shuffle(true, 0xA11C_E5EED);
        right.set_shuffle(true, 0xA11C_E5EED);

        assert_eq!(left.effective_indices(), right.effective_indices());
        assert_eq!(
            left.snapshot().playback_order(),
            right.snapshot().playback_order()
        );
    }

    #[test]
    fn changing_seed_changes_or_can_reproduce_a_distinct_permutation() {
        let mut left = queue();
        let mut right = queue();
        left.set_shuffle(true, 1);
        right.set_shuffle(true, 2);

        assert_ne!(left.effective_indices(), right.effective_indices());
    }

    #[test]
    fn repeat_one_applies_only_to_normal_completion() {
        let mut policy = queue();
        policy.set_repeat(RepeatMode::One);

        assert_eq!(
            policy.next(),
            QueueTransition::Selected {
                item_id: item("b"),
                canonical_index: 1
            }
        );
        assert_eq!(
            policy.on_item_completed(),
            QueueTransition::RestartCurrent {
                item_id: item("b"),
                canonical_index: 1
            }
        );
    }

    #[test]
    fn repeat_all_wraps_and_repeat_off_ends_without_removing_identity() {
        let mut policy = queue();
        assert_eq!(policy.next().item_id(), Some(&item("b")));
        assert_eq!(policy.next().item_id(), Some(&item("c")));
        assert_eq!(policy.on_item_completed(), QueueTransition::EndOfQueue);
        assert_eq!(policy.current_item(), Some(&item("c")));

        policy.set_repeat(RepeatMode::All);
        assert_eq!(policy.on_item_completed().item_id(), Some(&item("a")));
        assert_eq!(policy.items(), &[item("a"), item("b"), item("c")]);
    }

    #[test]
    fn previous_uses_restart_threshold_and_wrap_policy() {
        let mut policy = queue();
        let _ = policy.next();
        assert_eq!(
            policy.previous_at(PREVIOUS_RESTART_THRESHOLD_US + 1),
            QueueTransition::RestartCurrent {
                item_id: item("b"),
                canonical_index: 1
            }
        );
        assert_eq!(policy.previous_at(0).item_id(), Some(&item("a")));
        assert_eq!(
            policy.previous(),
            QueueTransition::RestartCurrent {
                item_id: item("a"),
                canonical_index: 0
            }
        );

        policy.set_repeat(RepeatMode::All);
        assert_eq!(policy.previous().item_id(), Some(&item("c")));
    }

    #[test]
    fn missing_identity_is_not_a_queue_deletion() {
        let mut policy = queue();
        let missing = item("b");
        let result = policy.select_item(&item("missing"));

        assert_eq!(result, Err(QueueError::ItemNotFound));
        assert_eq!(policy.items(), &[item("a"), missing, item("c")]);
    }

    #[test]
    fn replacement_rejects_invalid_current_index_without_mutating() {
        let mut policy = queue();
        let before = policy.snapshot();
        let result = policy.set_items(vec![item("new")], Some(1));

        assert_eq!(
            result,
            Err(QueueError::CurrentIndexOutOfBounds {
                index: 1,
                length: 1
            })
        );
        assert_eq!(policy.snapshot(), before);
    }
}

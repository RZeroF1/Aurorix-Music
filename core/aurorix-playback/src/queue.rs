//! Core-owned queue policy for one offline playback session.
//!
//! A queue stores only opaque media identities and playback intent. It never
//! stores a path, open handle, URL, credential, or provider lease. Shuffle is
//! an explicit, reproducible permutation of logical item indexes; ordinary
//! queue mutations keep the logical order stable.

use std::{error::Error, fmt};

use crate::command::{PlaybackItemId, RepeatMode};

/// The default point at which previous changes item instead of restarting it.
pub const DEFAULT_PREVIOUS_RESTART_THRESHOLD_US: u64 = 3_000_000;

/// The reason a queue advanced to another item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdvanceReason {
    /// The caller explicitly requested the next item.
    ManualNext,
    /// The current item reached its normal end.
    Completed,
}

/// A queue operation that selected or restarted an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueTransition {
    /// A new item became current.
    Selected {
        /// The selected Core media identity.
        item_id: PlaybackItemId,
        /// Its stable logical position in the queue.
        logical_index: usize,
        /// Its position in the active playback order.
        order_position: usize,
    },
    /// The current item should restart from position zero.
    RestartCurrent {
        /// The current Core media identity.
        item_id: PlaybackItemId,
        /// Its stable logical position in the queue.
        logical_index: usize,
        /// Its position in the active playback order.
        order_position: usize,
    },
    /// The queue has no item to select.
    Empty,
    /// Repeat-off reached the end and the session should become ended.
    Ended,
}

impl QueueTransition {
    /// Returns the selected item, if this transition has one.
    #[must_use]
    pub fn item_id(&self) -> Option<&PlaybackItemId> {
        match self {
            Self::Selected { item_id, .. } | Self::RestartCurrent { item_id, .. } => Some(item_id),
            Self::Empty | Self::Ended => None,
        }
    }

    /// Reports whether this transition restarts the current item.
    #[must_use]
    pub const fn is_restart(&self) -> bool {
        matches!(self, Self::RestartCurrent { .. })
    }
}

/// A bounded projection of queue state for clients and deterministic tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    items: Vec<PlaybackItemId>,
    playback_order: Vec<usize>,
    current_index: Option<usize>,
    current_order_position: Option<usize>,
    shuffle_enabled: bool,
    shuffle_seed: u64,
    repeat_mode: RepeatMode,
    previous_restart_threshold_us: u64,
}

impl QueueSnapshot {
    /// Returns the logical queue items in insertion order.
    #[must_use]
    pub fn items(&self) -> &[PlaybackItemId] {
        &self.items
    }

    /// Returns the logical indexes in the active playback order.
    #[must_use]
    pub fn playback_order(&self) -> &[usize] {
        &self.playback_order
    }

    /// Returns the current logical item index, if one is selected.
    #[must_use]
    pub const fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Returns the current position in the active playback order.
    #[must_use]
    pub const fn current_order_position(&self) -> Option<usize> {
        self.current_order_position
    }

    /// Returns whether deterministic shuffle is enabled.
    #[must_use]
    pub const fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    /// Returns the stored shuffle seed.
    #[must_use]
    pub const fn shuffle_seed(&self) -> u64 {
        self.shuffle_seed
    }

    /// Returns the repeat policy.
    #[must_use]
    pub const fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    /// Returns the previous-item restart threshold in microseconds.
    #[must_use]
    pub const fn previous_restart_threshold_us(&self) -> u64 {
        self.previous_restart_threshold_us
    }
}

/// A failure to apply a queue mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    /// The supplied logical index is outside the queue.
    IndexOutOfBounds {
        /// The rejected index.
        index: usize,
        /// The queue length at rejection time.
        length: usize,
    },
}

impl fmt::Display for QueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IndexOutOfBounds { index, length } => {
                write!(formatter, "queue index {index} is outside length {length}")
            }
        }
    }
}

impl Error for QueueError {}

/// Core-owned ordered queue and deterministic playback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackQueue {
    items: Vec<PlaybackItemId>,
    playback_order: Vec<usize>,
    current_index: Option<usize>,
    current_order_position: Option<usize>,
    shuffle_enabled: bool,
    shuffle_seed: u64,
    repeat_mode: RepeatMode,
    previous_restart_threshold_us: u64,
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackQueue {
    /// Creates an empty, ordered queue with repeat-off behavior.
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            playback_order: Vec::new(),
            current_index: None,
            current_order_position: None,
            shuffle_enabled: false,
            shuffle_seed: 0,
            repeat_mode: RepeatMode::Off,
            previous_restart_threshold_us: DEFAULT_PREVIOUS_RESTART_THRESHOLD_US,
        }
    }

    /// Creates a queue from logical items and an optional current index.
    ///
    /// `current_index` is a logical insertion-order index, not a shuffled
    /// playback-order index.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` for an invalid current index.
    pub fn from_items(
        items: Vec<PlaybackItemId>,
        current_index: Option<usize>,
    ) -> Result<Self, QueueError> {
        let mut queue = Self::new();
        queue.items = items;
        queue.rebuild_order(current_index)?;
        Ok(queue)
    }

    /// Returns the logical queue items in stable insertion order.
    #[must_use]
    pub fn items(&self) -> &[PlaybackItemId] {
        &self.items
    }

    /// Returns the current logical item, if any.
    #[must_use]
    pub fn current_item(&self) -> Option<&PlaybackItemId> {
        self.current_index.and_then(|index| self.items.get(index))
    }

    /// Returns the current logical item index.
    #[must_use]
    pub const fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Returns the current position in the active playback order.
    #[must_use]
    pub const fn current_order_position(&self) -> Option<usize> {
        self.current_order_position
    }

    /// Returns the current queue length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Reports whether the queue contains no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns the active logical indexes in playback order.
    #[must_use]
    pub fn playback_order(&self) -> &[usize] {
        &self.playback_order
    }

    /// Returns the current repeat policy.
    #[must_use]
    pub const fn repeat_mode(&self) -> RepeatMode {
        self.repeat_mode
    }

    /// Changes repeat policy without changing queue order or current item.
    pub const fn set_repeat_mode(&mut self, repeat_mode: RepeatMode) {
        self.repeat_mode = repeat_mode;
    }

    /// Returns whether deterministic shuffle is active.
    #[must_use]
    pub const fn shuffle_enabled(&self) -> bool {
        self.shuffle_enabled
    }

    /// Returns the explicit seed used to generate the shuffle permutation.
    #[must_use]
    pub const fn shuffle_seed(&self) -> u64 {
        self.shuffle_seed
    }

    /// Enables or disables deterministic shuffle and stores its seed.
    ///
    /// Rebuilding the permutation preserves the current logical item when it
    /// exists, so changing policy does not silently change the current item.
    ///
    /// # Errors
    ///
    /// Returns a typed error if the current queue selection is invalid.
    pub fn set_shuffle(&mut self, enabled: bool, seed: u64) -> Result<(), QueueError> {
        self.shuffle_enabled = enabled;
        self.shuffle_seed = seed;
        self.rebuild_order(self.current_index)
    }

    /// Returns the previous-item restart threshold in microseconds.
    #[must_use]
    pub const fn previous_restart_threshold_us(&self) -> u64 {
        self.previous_restart_threshold_us
    }

    /// Sets the threshold used by previous.
    pub const fn set_previous_restart_threshold_us(&mut self, threshold_us: u64) {
        self.previous_restart_threshold_us = threshold_us;
    }

    /// Returns a bounded clone suitable for a facade snapshot.
    #[must_use]
    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            items: self.items.clone(),
            playback_order: self.playback_order.clone(),
            current_index: self.current_index,
            current_order_position: self.current_order_position,
            shuffle_enabled: self.shuffle_enabled,
            shuffle_seed: self.shuffle_seed,
            repeat_mode: self.repeat_mode,
            previous_restart_threshold_us: self.previous_restart_threshold_us,
        }
    }

    /// Replaces the logical queue, preserving policy settings.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` for an invalid current index.
    pub fn replace(
        &mut self,
        items: Vec<PlaybackItemId>,
        current_index: Option<usize>,
    ) -> Result<(), QueueError> {
        self.items = items;
        self.rebuild_order(current_index)
    }

    /// Appends one identity and preserves the current logical item.
    ///
    /// # Errors
    ///
    /// Returns a typed error if the current queue selection cannot be
    /// preserved.
    pub fn append(&mut self, item_id: PlaybackItemId) -> Result<(), QueueError> {
        let new_index = self.items.len();
        self.items.push(item_id);
        if self.shuffle_enabled {
            self.playback_order.push(new_index);
            self.refresh_current_position();
            Ok(())
        } else {
            self.rebuild_order(self.current_index)
        }
    }

    /// Inserts an identity at a logical index.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` unless index is at most `len`.
    pub fn insert(&mut self, index: usize, item_id: PlaybackItemId) -> Result<(), QueueError> {
        if index > self.items.len() {
            return Err(QueueError::IndexOutOfBounds {
                index,
                length: self.items.len(),
            });
        }
        self.items.insert(index, item_id);
        let current = self.current_index.map(|current| {
            if current >= index {
                current.saturating_add(1)
            } else {
                current
            }
        });
        if self.shuffle_enabled {
            for item in &mut self.playback_order {
                if *item >= index {
                    *item += 1;
                }
            }
            self.playback_order.push(index);
            self.current_index = current;
            self.refresh_current_position();
            Ok(())
        } else {
            self.rebuild_order(current)
        }
    }

    /// Removes a logical item and selects the nearest remaining item when the
    /// removed item was current.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` for an invalid index.
    pub fn remove(&mut self, index: usize) -> Result<PlaybackItemId, QueueError> {
        if index >= self.items.len() {
            return Err(QueueError::IndexOutOfBounds {
                index,
                length: self.items.len(),
            });
        }
        let removed = self.items.remove(index);
        let current = match self.current_index {
            None => None,
            Some(_) if self.items.is_empty() => None,
            Some(current) if current > index => Some(current - 1),
            Some(current) if current == index => Some(current.min(self.items.len() - 1)),
            Some(current) => Some(current),
        };
        if self.shuffle_enabled {
            self.playback_order.retain(|&item| item != index);
            for item in &mut self.playback_order {
                if *item > index {
                    *item -= 1;
                }
            }
            self.current_index = current;
            self.refresh_current_position();
        } else {
            self.rebuild_order(current)?;
        }
        Ok(removed)
    }

    /// Moves one logical item while preserving all other logical identities.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` for an invalid source or target.
    pub fn move_item(&mut self, from: usize, to: usize) -> Result<(), QueueError> {
        let length = self.items.len();
        if from >= length {
            return Err(QueueError::IndexOutOfBounds {
                index: from,
                length,
            });
        }
        if to >= length {
            return Err(QueueError::IndexOutOfBounds { index: to, length });
        }
        if from == to {
            return Ok(());
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        let current = self.current_index.map(|current| {
            if current == from {
                to
            } else if from < current && current <= to {
                current - 1
            } else if to <= current && current < from {
                current.saturating_add(1)
            } else {
                current
            }
        });
        if self.shuffle_enabled {
            for item in &mut self.playback_order {
                *item = remap_index_after_move(*item, from, to);
            }
            self.current_index = current;
            self.refresh_current_position();
            Ok(())
        } else {
            self.rebuild_order(current)
        }
    }

    /// Selects a logical item without changing the active playback policy.
    ///
    /// # Errors
    ///
    /// Returns `QueueError::IndexOutOfBounds` for an invalid index.
    pub fn select(&mut self, index: usize) -> Result<QueueTransition, QueueError> {
        if index >= self.items.len() {
            return Err(QueueError::IndexOutOfBounds {
                index,
                length: self.items.len(),
            });
        }
        self.current_index = Some(index);
        self.current_order_position = self.playback_order.iter().position(|&item| item == index);
        Ok(self
            .selected_transition(false)
            .unwrap_or(QueueTransition::Empty))
    }

    /// Selects the first item, if any, according to the active playback order.
    #[must_use]
    pub fn first(&mut self) -> QueueTransition {
        if self.playback_order.is_empty() {
            self.current_index = None;
            self.current_order_position = None;
            return QueueTransition::Empty;
        }
        self.current_order_position = Some(0);
        self.current_index = self.playback_order.first().copied();
        self.selected_transition(false)
            .unwrap_or(QueueTransition::Empty)
    }

    /// Advances according to an explicit manual/completion reason.
    #[must_use]
    pub fn advance_next(&mut self, reason: AdvanceReason) -> QueueTransition {
        let Some(current_position) = self.current_order_position else {
            return self.first();
        };
        if matches!(reason, AdvanceReason::Completed) && self.repeat_mode == RepeatMode::One {
            return self
                .selected_transition(true)
                .unwrap_or(QueueTransition::Empty);
        }

        if current_position + 1 < self.playback_order.len() {
            self.current_order_position = Some(current_position + 1);
            self.current_index = self.playback_order.get(current_position + 1).copied();
            return self
                .selected_transition(false)
                .unwrap_or(QueueTransition::Empty);
        }

        match (reason, self.repeat_mode) {
            (AdvanceReason::Completed | AdvanceReason::ManualNext, RepeatMode::All) => {
                self.current_order_position = Some(0);
                self.current_index = self.playback_order.first().copied();
                self.selected_transition(false)
                    .unwrap_or(QueueTransition::Empty)
            }
            _ => QueueTransition::Ended,
        }
    }

    /// Advances to the next item as a manual command.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> QueueTransition {
        self.advance_next(AdvanceReason::ManualNext)
    }

    /// Applies normal completion semantics to the current item.
    #[must_use]
    pub fn complete_current(&mut self) -> QueueTransition {
        self.advance_next(AdvanceReason::Completed)
    }

    /// Restarts the current item when it is beyond the threshold, otherwise
    /// selects the previous item in the active order.
    #[must_use]
    pub fn previous(&mut self, position_us: u64) -> QueueTransition {
        let Some(current_position) = self.current_order_position else {
            return QueueTransition::Empty;
        };
        if position_us > self.previous_restart_threshold_us {
            return self
                .selected_transition(true)
                .unwrap_or(QueueTransition::Empty);
        }
        if current_position > 0 {
            let previous_position = current_position - 1;
            self.current_order_position = Some(previous_position);
            self.current_index = self.playback_order.get(previous_position).copied();
            return self
                .selected_transition(false)
                .unwrap_or(QueueTransition::Empty);
        }
        if self.repeat_mode == RepeatMode::All && !self.playback_order.is_empty() {
            let previous_position = self.playback_order.len() - 1;
            self.current_order_position = Some(previous_position);
            self.current_index = self.playback_order.get(previous_position).copied();
            return self
                .selected_transition(false)
                .unwrap_or(QueueTransition::Empty);
        }
        self.selected_transition(true)
            .unwrap_or(QueueTransition::Empty)
    }

    fn selected_transition(&self, restart: bool) -> Option<QueueTransition> {
        let logical_index = self.current_index?;
        let item_id = self.items.get(logical_index)?.clone();
        let order_position = self.current_order_position?;
        Some(if restart {
            QueueTransition::RestartCurrent {
                item_id,
                logical_index,
                order_position,
            }
        } else {
            QueueTransition::Selected {
                item_id,
                logical_index,
                order_position,
            }
        })
    }

    fn rebuild_order(&mut self, current_index: Option<usize>) -> Result<(), QueueError> {
        if let Some(index) = current_index
            && index >= self.items.len()
        {
            return Err(QueueError::IndexOutOfBounds {
                index,
                length: self.items.len(),
            });
        }
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        if self.shuffle_enabled {
            let mut state = self.shuffle_seed;
            let mut index = order.len();
            while index > 1 {
                index -= 1;
                let range = u64::try_from(index + 1).unwrap_or(u64::MAX);
                let swap_with = usize::try_from(splitmix64(&mut state) % range).unwrap_or(0);
                order.swap(index, swap_with);
            }
        }
        self.playback_order = order;
        self.current_index = current_index;
        self.refresh_current_position();
        Ok(())
    }

    fn refresh_current_position(&mut self) {
        self.current_order_position = self
            .current_index
            .and_then(|current| self.playback_order.iter().position(|&item| item == current));
    }
}

fn remap_index_after_move(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < to && index > from && index <= to {
        index - 1
    } else if to < from && index >= to && index < from {
        index + 1
    } else {
        index
    }
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
    use super::{
        AdvanceReason, DEFAULT_PREVIOUS_RESTART_THRESHOLD_US, PlaybackQueue, QueueTransition,
    };
    use crate::command::{PlaybackItemId, RepeatMode};

    fn item(value: &str) -> PlaybackItemId {
        PlaybackItemId::new(value).expect("fixture identity is valid")
    }

    fn queue() -> PlaybackQueue {
        PlaybackQueue::from_items(vec![item("a"), item("b"), item("c")], Some(0))
            .expect("fixture queue is valid")
    }

    #[test]
    fn ordinary_order_and_mutations_are_stable() {
        let mut queue = queue();
        assert_eq!(queue.playback_order(), &[0, 1, 2]);
        assert_eq!(queue.current_item().map(PlaybackItemId::as_str), Some("a"));
        queue.insert(1, item("x")).expect("insert succeeds");
        assert_eq!(
            queue
                .items()
                .iter()
                .map(PlaybackItemId::as_str)
                .collect::<Vec<_>>(),
            ["a", "x", "b", "c"]
        );
        assert_eq!(queue.current_item().map(PlaybackItemId::as_str), Some("a"));
        queue.move_item(3, 1).expect("move succeeds");
        assert_eq!(
            queue
                .items()
                .iter()
                .map(PlaybackItemId::as_str)
                .collect::<Vec<_>>(),
            ["a", "c", "x", "b"]
        );
        assert_eq!(queue.remove(0).expect("remove succeeds").as_str(), "a");
        assert_eq!(queue.current_item().map(PlaybackItemId::as_str), Some("c"));
    }

    #[test]
    fn next_and_end_respect_repeat_modes() {
        let mut queue = queue();
        assert_eq!(
            queue.next().item_id().map(PlaybackItemId::as_str),
            Some("b")
        );
        assert_eq!(
            queue.next().item_id().map(PlaybackItemId::as_str),
            Some("c")
        );
        assert_eq!(queue.next(), QueueTransition::Ended);

        queue.set_repeat_mode(RepeatMode::One);
        assert!(queue.complete_current().is_restart());

        queue.set_repeat_mode(RepeatMode::All);
        assert_eq!(
            queue
                .complete_current()
                .item_id()
                .map(PlaybackItemId::as_str),
            Some("a")
        );
    }

    #[test]
    fn previous_uses_restart_threshold_and_repeat_all_wrap() {
        let mut queue = queue();
        assert!(
            queue
                .previous(DEFAULT_PREVIOUS_RESTART_THRESHOLD_US + 1)
                .is_restart()
        );
        assert!(queue.previous(0).is_restart());
        let _ = queue.next();
        assert_eq!(
            queue.previous(0).item_id().map(PlaybackItemId::as_str),
            Some("a")
        );
        queue.set_repeat_mode(RepeatMode::All);
        assert_eq!(
            queue.previous(0).item_id().map(PlaybackItemId::as_str),
            Some("c")
        );
    }

    #[test]
    fn shuffle_is_reproducible_and_preserves_current_identity() {
        let mut first = queue();
        let mut second = queue();
        first.set_shuffle(true, 42).expect("shuffle succeeds");
        second.set_shuffle(true, 42).expect("shuffle succeeds");
        assert_eq!(first.playback_order(), second.playback_order());
        assert_eq!(first.current_item(), second.current_item());
        let original = first.playback_order().to_vec();
        first
            .set_shuffle(false, 42)
            .expect("shuffle disable succeeds");
        first
            .set_shuffle(true, 42)
            .expect("shuffle re-enable succeeds");
        assert_eq!(first.playback_order(), original);
    }

    #[test]
    fn shuffled_mutations_preserve_the_relative_order_of_existing_items() {
        let mut queue = queue();
        queue.set_shuffle(true, 42).expect("shuffle succeeds");
        let original = queue.playback_order().to_vec();
        queue.append(item("d")).expect("append succeeds");
        assert_eq!(&queue.playback_order()[..3], original.as_slice());
        queue.insert(1, item("x")).expect("insert succeeds");
        let remapped = original
            .iter()
            .map(|index| if *index >= 1 { index + 1 } else { *index })
            .collect::<Vec<_>>();
        assert_eq!(
            queue
                .playback_order()
                .iter()
                .filter(|index| remapped.contains(index))
                .copied()
                .collect::<Vec<_>>(),
            remapped
        );
    }

    #[test]
    fn completion_repeat_one_does_not_apply_to_manual_next() {
        let mut queue = queue();
        queue.set_repeat_mode(RepeatMode::One);
        assert_eq!(
            queue.next().item_id().map(PlaybackItemId::as_str),
            Some("b")
        );
        assert!(queue.advance_next(AdvanceReason::Completed).is_restart());
    }

    #[test]
    fn queue_snapshot_contains_only_identity_and_policy() {
        let queue = queue();
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.items().len(), 3);
        assert_eq!(snapshot.current_index(), Some(0));
        assert!(!snapshot.shuffle_enabled());
        assert_eq!(snapshot.repeat_mode(), RepeatMode::Off);
    }
}

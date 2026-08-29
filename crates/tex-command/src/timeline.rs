//! Generation-owned reversible command stack storage.

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index, IndexMut};
use core::slice::SliceIndex;

use crate::scalar_journal::{PackedJournal, PackedJournalMark};

const PAYLOAD_SLOTS_PER_PAGE: usize = 32;
const UNDO_RECORDS_PER_CHUNK: usize = 16;

pub(crate) trait LogicalStackElement {
    type State;

    fn capture_state(&self) -> Self::State;
    fn swap_state(&mut self, state: &mut Self::State);
}

impl LogicalStackElement for i32 {
    type State = i32;

    fn capture_state(&self) -> Self::State {
        *self
    }

    fn swap_state(&mut self, state: &mut Self::State) {
        std::mem::swap(self, state);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PayloadHandle {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

struct PayloadSlot<T> {
    generation: u32,
    value: Option<T>,
    free_next: Option<u32>,
}

struct PayloadPage<T> {
    slots: Box<[PayloadSlot<T>]>,
}

pub(crate) struct PayloadSlab<T> {
    pages: Vec<PayloadPage<T>>,
    free_head: Option<u32>,
    pub(crate) live: u32,
    admissions: u64,
    pub(crate) reuses: u64,
}

impl<T> Default for PayloadSlab<T> {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            free_head: None,
            live: 0,
            admissions: 0,
            reuses: 0,
        }
    }
}

impl<T> PayloadSlab<T> {
    pub(crate) fn warm_first_page(&mut self) {
        if self.pages.is_empty() {
            self.add_page();
        }
    }

    pub(crate) fn insert(&mut self, value: T) -> PayloadHandle {
        if self.free_head.is_none() {
            self.add_page();
        }
        let slot = self.free_head.expect("payload page supplied a free slot");
        let (free_next, generation, reused) = {
            let entry = self.slot_by_index_mut(slot);
            (entry.free_next, entry.generation, entry.generation != 1)
        };
        self.free_head = free_next;
        let entry = self.slot_by_index_mut(slot);
        entry.free_next = None;
        entry.value = Some(value);
        self.live = self.live.saturating_add(1);
        self.admissions = self.admissions.saturating_add(1);
        self.reuses = self.reuses.saturating_add(u64::from(reused));
        PayloadHandle { slot, generation }
    }

    pub(crate) fn swap(&mut self, handle: PayloadHandle, value: &mut T) {
        let stored = self
            .slot_mut(handle)
            .and_then(|slot| slot.value.as_mut())
            .expect("logical-stack displaced payload remains live");
        std::mem::swap(stored, value);
    }

    pub(crate) fn release(&mut self, handle: PayloadHandle) {
        let free_head = self.free_head;
        let slot = self
            .slot_mut(handle)
            .expect("logical-stack released payload remains live");
        drop(slot.value.take());
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.free_next = free_head;
        self.free_head = Some(handle.slot);
        self.live = self.live.saturating_sub(1);
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.pages
                    .capacity()
                    .saturating_mul(std::mem::size_of::<PayloadPage<T>>()),
            )
            .saturating_add(self.pages.iter().fold(0_usize, |bytes, page| {
                bytes.saturating_add(
                    page.slots
                        .len()
                        .saturating_mul(std::mem::size_of::<PayloadSlot<T>>()),
                )
            }))
    }

    fn add_page(&mut self) {
        let start = self.pages.len().saturating_mul(PAYLOAD_SLOTS_PER_PAGE);
        assert!(u32::try_from(start.saturating_add(PAYLOAD_SLOTS_PER_PAGE)).is_ok());
        let mut free = self.free_head;
        let slots = (0..PAYLOAD_SLOTS_PER_PAGE)
            .map(|offset| {
                let slot = u32::try_from(start + offset).expect("payload slab fits u32");
                let entry = PayloadSlot {
                    generation: 1,
                    value: None,
                    free_next: free,
                };
                free = Some(slot);
                entry
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        self.pages.push(PayloadPage { slots });
        self.free_head = free;
    }

    pub(crate) fn value_mut(&mut self, handle: PayloadHandle) -> Option<&mut T> {
        self.slot_mut(handle).and_then(|slot| slot.value.as_mut())
    }

    fn slot_mut(&mut self, handle: PayloadHandle) -> Option<&mut PayloadSlot<T>> {
        let slot = self.slot_by_index_mut(handle.slot);
        (slot.generation == handle.generation && slot.value.is_some()).then_some(slot)
    }

    fn slot_by_index_mut(&mut self, slot: u32) -> &mut PayloadSlot<T> {
        let slot = slot as usize;
        &mut self.pages[slot / PAYLOAD_SLOTS_PER_PAGE].slots[slot % PAYLOAD_SLOTS_PER_PAGE]
    }
}

enum ElementUndo<S> {
    InlineState { index: u32, state: S },
    Replacement { index: u32, payload: PayloadHandle },
}

/// A stack whose payload rows are admitted once and whose history contains
/// only compact mutable-state and displaced-payload handles.
pub(crate) struct LogicalStack<T: LogicalStackElement> {
    rows: Vec<T>,
    top: usize,
    undo: PackedJournal<ElementUndo<T::State>, UNDO_RECORDS_PER_CHUNK>,
    displaced: PayloadSlab<T>,
    fork: Option<LogicalStackFork>,
    recording: bool,
    interval: u64,
    touched: Vec<u64>,
    /// The current row occupant has an inline, compact, or stored inverse in
    /// this interval. Reusing such a row must first preserve that occupant as
    /// a replacement, even though ordinary first-touch coalescing has already
    /// marked the physical row touched.
    partially_captured: Vec<u64>,
    coalesced_mutations: u64,
    payload_admissions: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct LogicalStackMark {
    pub(crate) top: u32,
    pub(crate) undo: PackedJournalMark,
}

#[derive(Debug)]
struct LogicalStackFork {
    accepted_top: usize,
}

#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct LogicalStackCounters {
    pub(crate) payload_admissions: u64,
    pub(crate) full_payload_history_clones: u64,
    pub(crate) undo_records: u64,
    pub(crate) undo_record_bytes: u64,
    pub(crate) coalesced_mutations: u64,
    pub(crate) displaced_payloads: u32,
    pub(crate) displaced_reuses: u64,
    pub(crate) stored_state_captures: u64,
    pub(crate) owner_swaps: u64,
}

impl<T: LogicalStackElement> Default for LogicalStack<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            top: 0,
            undo: PackedJournal::default(),
            displaced: PayloadSlab::default(),
            fork: None,
            recording: false,
            interval: 1,
            touched: Vec::new(),
            partially_captured: Vec::new(),
            coalesced_mutations: 0,
            payload_admissions: 0,
        }
    }
}

impl<T: LogicalStackElement + core::fmt::Debug> core::fmt::Debug for LogicalStack<T>
where
    T::State: core::fmt::Debug,
{
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LogicalStack")
            .field("rows", &&self.rows[..self.top])
            .field("top", &self.top)
            .finish_non_exhaustive()
    }
}

impl<T: LogicalStackElement + PartialEq> PartialEq for LogicalStack<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: LogicalStackElement + Eq> Eq for LogicalStack<T> {}

impl<T: LogicalStackElement + Hash> Hash for LogicalStack<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<T: LogicalStackElement> Deref for LogicalStack<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.rows[..self.top]
    }
}

impl<T: LogicalStackElement, I> Index<I> for LogicalStack<T>
where
    I: SliceIndex<[T]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl<T: LogicalStackElement> IndexMut<usize> for LogicalStack<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.record(index);
        &mut self.rows[index]
    }
}

impl<'a, T: LogicalStackElement> IntoIterator for &'a LogicalStack<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: LogicalStackElement> LogicalStack<T> {
    pub(crate) fn push(&mut self, value: T) {
        self.payload_admissions = self.payload_admissions.saturating_add(1);
        if self.top == self.rows.len() {
            self.rows.push(value);
            self.touched.push(self.interval);
            self.partially_captured.push(0);
        } else if self.recording
            && (self.touched[self.top] != self.interval
                || self.partially_captured[self.top] == self.interval)
        {
            self.displaced.warm_first_page();
            let old = std::mem::replace(&mut self.rows[self.top], value);
            let payload = self.displaced.insert(old);
            self.undo.append(ElementUndo::Replacement {
                index: u32::try_from(self.top).expect("logical stack index fits u32"),
                payload,
            });
            self.touched[self.top] = self.interval;
            self.partially_captured[self.top] = 0;
        } else {
            // This physical row was admitted after the newest observable
            // mark, or was already replaced once in its interval. No retained
            // cursor can name its current payload: a mark would have advanced
            // `interval` before making that version observable. Reuse the row
            // directly so an arbitrary number of push/pop transitions inside
            // one command operation or named-boundary interval retain only
            // the stack high water, not one displaced payload per push.
            self.rows[self.top] = value;
            self.touched[self.top] = self.interval;
            self.partially_captured[self.top] = 0;
        }
        self.top += 1;
    }

    pub(crate) fn pop_project<R>(&mut self, project: impl FnOnce(&T) -> R) -> Option<R> {
        let index = self.top.checked_sub(1)?;
        let result = project(&self.rows[index]);
        self.top = index;
        if !self.recording {
            drop(self.rows.pop());
            self.touched.pop();
            self.partially_captured.pop();
        }
        Some(result)
    }

    pub(crate) fn pop_copy(&mut self) -> Option<T>
    where
        T: Copy,
    {
        self.pop_project(|value| *value)
    }

    pub(crate) fn last_mut(&mut self) -> Option<&mut T> {
        let index = self.top.checked_sub(1)?;
        self.record(index);
        self.rows.get_mut(index)
    }

    pub(crate) fn truncate_top(&mut self, top: usize) -> bool {
        if top > self.top {
            return false;
        }
        self.top = top;
        if !self.recording {
            self.rows.truncate(top);
            self.touched.truncate(top);
            self.partially_captured.truncate(top);
        }
        true
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.top {
            return None;
        }
        self.record(index);
        self.rows.get_mut(index)
    }

    pub(crate) fn mark(&mut self) -> Option<LogicalStackMark> {
        self.recording = true;
        self.undo.warm_first_page();
        self.displaced.warm_first_page();
        let mark = LogicalStackMark {
            top: u32::try_from(self.top).ok()?,
            undo: self.undo.mark(),
        };
        self.next_interval();
        Some(mark)
    }

    pub(crate) fn validates(&self, mark: LogicalStackMark) -> bool {
        mark.top as usize <= self.rows.len() && self.undo.validates(mark.undo)
    }

    pub(crate) fn restore(&mut self, mark: LogicalStackMark) -> bool {
        if self.fork.is_some() || !self.validates(mark) {
            return false;
        }
        let (rows, displaced) = (&mut self.rows, &mut self.displaced);
        let restored = self.undo.restore_with(
            mark.undo,
            &mut (rows, displaced),
            |inverse, (rows, displaced)| inverse.swap(rows, displaced),
            |inverse, (_, displaced)| inverse.release(displaced),
        );
        if restored {
            self.top = mark.top as usize;
            self.next_interval();
        }
        restored
    }

    pub(crate) fn release_prefix(&mut self, mark: LogicalStackMark) -> Option<usize> {
        if self.fork.is_some() || !self.validates(mark) {
            return None;
        }
        let displaced = &mut self.displaced;
        self.undo.release_prefix(mark.undo, |inverse| {
            inverse.release(displaced);
        })
    }

    pub(crate) fn begin_checkpoint_candidate(&mut self, mark: LogicalStackMark) {
        assert!(
            self.fork.is_none(),
            "logical stack already owns a candidate fork"
        );
        assert!(
            self.validates(mark),
            "logical stack candidate mark was prevalidated"
        );
        let accepted_top = self.top;
        let (rows, displaced) = (&mut self.rows, &mut self.displaced);
        self.undo.begin_checkpoint_candidate(mark.undo, |inverse| {
            inverse.swap(rows, displaced);
        });
        self.top = mark.top as usize;
        self.fork = Some(LogicalStackFork { accepted_top });
        self.next_interval();
    }

    pub(crate) fn reject_checkpoint_candidate(&mut self) {
        let fork = self
            .fork
            .take()
            .expect("logical stack rejection requires a candidate fork");
        let (rows, displaced) = (&mut self.rows, &mut self.displaced);
        self.undo.reject_checkpoint_candidate_with(
            &mut (rows, displaced),
            |inverse, (rows, displaced)| inverse.swap(rows, displaced),
            |inverse, (_, displaced)| inverse.release(displaced),
        );
        self.top = fork.accepted_top;
        self.next_interval();
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        self.fork
            .take()
            .expect("logical stack acceptance requires a candidate fork");
        self.undo
            .accept_checkpoint_candidate_with(&mut &mut self.displaced, |inverse, displaced| {
                inverse.release(displaced)
            });
        self.next_interval();
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        &self.rows[..self.top]
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(
                self.rows
                    .capacity()
                    .saturating_mul(std::mem::size_of::<T>()),
            )
            .saturating_add(
                self.touched
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.partially_captured
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(self.undo.retained_bytes())
            .saturating_add(self.displaced.retained_bytes())
    }

    #[cfg(any(test, feature = "profiling"))]
    pub(crate) fn counters(&self) -> LogicalStackCounters {
        let undo = self.undo.counters();
        LogicalStackCounters {
            payload_admissions: self.payload_admissions,
            full_payload_history_clones: 0,
            undo_records: undo.records,
            undo_record_bytes: undo.record_bytes,
            coalesced_mutations: self.coalesced_mutations,
            displaced_payloads: self.displaced.live,
            displaced_reuses: self.displaced.reuses,
            stored_state_captures: 0,
            owner_swaps: 0,
        }
    }

    fn record(&mut self, index: usize) {
        if !self.recording || self.touched[index] == self.interval {
            if self.recording {
                self.coalesced_mutations = self.coalesced_mutations.saturating_add(1);
            }
            return;
        }
        self.touched[index] = self.interval;
        self.partially_captured[index] = self.interval;
        let index = u32::try_from(index).expect("logical stack index fits u32");
        let state = self.rows[index as usize].capture_state();
        self.undo.append(ElementUndo::InlineState { index, state });
    }

    fn next_interval(&mut self) {
        self.interval = self.interval.wrapping_add(1).max(1);
        if self.interval == 1 {
            self.touched.fill(0);
        }
    }
}

impl<S> ElementUndo<S> {
    fn swap<T: LogicalStackElement<State = S>>(
        &mut self,
        rows: &mut [T],
        displaced: &mut PayloadSlab<T>,
    ) {
        match self {
            Self::InlineState { index, state } => {
                rows[*index as usize].swap_state(state);
            }
            Self::Replacement { index, payload } => {
                displaced.swap(*payload, &mut rows[*index as usize]);
            }
        }
    }

    fn release<T: LogicalStackElement>(self, displaced: &mut PayloadSlab<T>) {
        match self {
            Self::Replacement { payload, .. } => displaced.release(payload),
            Self::InlineState { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogicalStack, PayloadSlab};

    #[test]
    fn displaced_payload_slab_reuses_a_warmed_slot_without_a_new_page() {
        let mut slab = PayloadSlab::default();
        slab.warm_first_page();
        let first = slab.insert(11);
        slab.release(first);
        let second = slab.insert(22);

        assert_eq!(first.slot, second.slot);
        assert_ne!(first.generation, second.generation);
        assert_eq!(slab.pages.len(), 1);
        assert_eq!(slab.reuses, 1);
        assert_eq!(slab.slot_mut(second).and_then(|slot| slot.value), Some(22));
    }

    #[test]
    fn pop_then_push_preserves_the_rollback_reachable_row_without_payload_clone() {
        let mut stack = LogicalStack::default();
        stack.push(1);
        stack.push(2);
        let mark = stack.mark().expect("small stack marks");

        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(3);
        assert_eq!(stack.as_slice(), &[1, 3]);

        assert!(stack.restore(mark));
        assert_eq!(stack.as_slice(), &[1, 2]);
        let counters = stack.counters();
        assert_eq!(counters.payload_admissions, 3);
        assert_eq!(counters.full_payload_history_clones, 0);
        assert_eq!(counters.displaced_payloads, 0);
    }

    #[test]
    fn repeated_mutation_is_one_compact_record_per_checkpoint_interval() {
        let mut stack = LogicalStack::default();
        stack.push(10);
        let mark = stack.mark().expect("small stack marks");
        for _ in 0..1_024 {
            *stack.last_mut().expect("frame remains live") += 1;
        }
        assert_eq!(stack.counters().undo_records, 1);
        assert_eq!(stack.counters().coalesced_mutations, 1_023);
        assert!(stack.restore(mark));
        assert_eq!(stack.as_slice(), &[10]);
    }

    #[test]
    fn unobserved_pop_push_reuses_one_row_without_history() {
        let mut stack = LogicalStack::default();
        let root = stack.mark().expect("empty stack marks");

        stack.push(1);
        assert_eq!(stack.pop_copy(), Some(1));
        stack.push(2);
        assert_eq!(stack.pop_copy(), Some(2));

        assert!(stack.as_slice().is_empty());
        assert_eq!(stack.counters().undo_records, 0);
        assert_eq!(stack.counters().displaced_payloads, 0);
        assert!(stack.restore(root));
        assert!(stack.as_slice().is_empty());
    }

    #[test]
    fn first_replacement_after_a_mark_preserves_exactly_one_version() {
        let mut stack = LogicalStack::default();
        stack.push(1);
        let root = stack.mark().expect("live row marks");

        assert_eq!(stack.pop_copy(), Some(1));
        stack.push(2);
        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(3);

        assert_eq!(stack.as_slice(), &[3]);
        assert_eq!(stack.counters().undo_records, 1);
        assert_eq!(stack.counters().displaced_payloads, 1);
        assert!(stack.restore(root));
        assert_eq!(stack.as_slice(), &[1]);
        assert_eq!(stack.counters().displaced_payloads, 0);
    }

    #[test]
    fn ten_million_unobserved_pushes_retain_only_one_warmed_row() {
        let mut stack = LogicalStack::default();
        let root = stack.mark().expect("empty stack marks");

        stack.push(0);
        assert_eq!(stack.pop_copy(), Some(0));
        let warmed_bytes = stack.retained_bytes();
        for value in 1..=10_000_000 {
            stack.push(value);
            assert_eq!(stack.pop_copy(), Some(value));
        }

        assert_eq!(stack.retained_bytes(), warmed_bytes);
        assert_eq!(stack.counters().payload_admissions, 10_000_001);
        assert_eq!(stack.counters().undo_records, 0);
        assert_eq!(stack.counters().displaced_payloads, 0);
        assert!(stack.restore(root));
    }

    #[test]
    fn candidate_reject_and_accept_settle_displaced_payload_slots() {
        let mut stack = LogicalStack::default();
        stack.push(1);
        stack.push(2);
        let root = stack.mark().expect("small stack marks");
        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(3);
        let accepted = stack.as_slice().to_vec();

        stack.begin_checkpoint_candidate(root);
        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(4);
        stack.reject_checkpoint_candidate();
        assert_eq!(stack.as_slice(), accepted);

        stack.begin_checkpoint_candidate(root);
        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(5);
        stack.accept_checkpoint_candidate();
        assert_eq!(stack.as_slice(), &[1, 5]);
        assert_eq!(stack.counters().displaced_payloads, 1);
    }

    #[test]
    fn released_stack_floor_drops_old_alternates_and_restores_from_the_new_base() {
        let mut stack = LogicalStack::default();
        stack.push(1);
        let root = stack.mark().expect("root mark");
        assert_eq!(stack.pop_copy(), Some(1));
        stack.push(2);
        let floor = stack.mark().expect("ordinary floor");
        assert_eq!(stack.counters().displaced_payloads, 1);

        assert_eq!(stack.release_prefix(floor), Some(1));
        assert!(!stack.validates(root));
        assert!(stack.validates(floor));
        assert_eq!(stack.counters().displaced_payloads, 0);

        assert_eq!(stack.pop_copy(), Some(2));
        stack.push(3);
        assert!(stack.restore(floor));
        assert_eq!(stack.as_slice(), [2]);
        assert_eq!(stack.counters().displaced_payloads, 0);
    }
}

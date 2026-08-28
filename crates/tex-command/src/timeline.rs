//! Generation-owned reversible command stack storage.

use core::hash::{Hash, Hasher};
use core::ops::{Deref, Index, IndexMut};
use core::slice::SliceIndex;
use std::cell::Cell;

/// A stack whose physical rows remain generation-owned after a logical pop.
#[derive(Debug)]
pub(crate) struct LogicalStack<T> {
    rows: Vec<T>,
    top: usize,
    undo: Vec<ElementUndo<T>>,
    fork: Option<LogicalStackFork>,
    detached_prior: Vec<ElementUndo<T>>,
    recording: Cell<bool>,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct LogicalStackMark {
    pub(crate) top: u32,
    pub(crate) undo: u32,
}

#[derive(Debug)]
struct ElementUndo<T> {
    index: usize,
    old: T,
}

#[derive(Debug)]
struct LogicalStackFork {
    accepted_top: usize,
    selected_undo: usize,
}

impl<T> Default for LogicalStack<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            top: 0,
            undo: Vec::new(),
            fork: None,
            detached_prior: Vec::new(),
            recording: Cell::new(false),
        }
    }
}

impl<T: PartialEq> PartialEq for LogicalStack<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: Eq> Eq for LogicalStack<T> {}

impl<T: Hash> Hash for LogicalStack<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<T> Deref for LogicalStack<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.rows[..self.top]
    }
}

impl<T, I> Index<I> for LogicalStack<T>
where
    I: SliceIndex<[T]>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl<T: Clone> IndexMut<usize> for LogicalStack<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.record(index);
        &mut self.rows[index]
    }
}

impl<'a, T> IntoIterator for &'a LogicalStack<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: Clone> LogicalStack<T> {
    pub(crate) fn push(&mut self, value: T) {
        if self.top == self.rows.len() {
            self.rows.push(value);
        } else {
            self.record(self.top);
            self.rows[self.top] = value;
        }
        self.top += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        if self.top == 0 {
            return None;
        }
        self.top -= 1;
        if self.recording.get() {
            Some(self.rows[self.top].clone())
        } else {
            Some(self.rows.remove(self.top))
        }
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
        true
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.top {
            return None;
        }
        self.record(index);
        self.rows.get_mut(index)
    }

    pub(crate) fn mark(&self) -> Option<LogicalStackMark> {
        self.recording.set(true);
        Some(LogicalStackMark {
            top: u32::try_from(self.top).ok()?,
            undo: u32::try_from(self.undo.len()).ok()?,
        })
    }

    pub(crate) fn validates(&self, mark: LogicalStackMark) -> bool {
        mark.top as usize <= self.rows.len() && mark.undo as usize <= self.undo.len()
    }

    pub(crate) fn restore(&mut self, mark: LogicalStackMark) -> bool {
        if self.fork.is_some() || !self.validates(mark) {
            return false;
        }
        while self.undo.len() > mark.undo as usize {
            let inverse = self.undo.pop().expect("validated undo suffix exists");
            self.rows[inverse.index] = inverse.old;
        }
        self.top = mark.top as usize;
        true
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
        debug_assert!(self.detached_prior.is_empty());
        while self.undo.len() > mark.undo as usize {
            let mut inverse = self.undo.pop().expect("validated undo suffix exists");
            std::mem::swap(&mut self.rows[inverse.index], &mut inverse.old);
            self.detached_prior.push(inverse);
        }
        let accepted_top = self.top;
        self.top = mark.top as usize;
        self.fork = Some(LogicalStackFork {
            accepted_top,
            selected_undo: mark.undo as usize,
        });
    }

    pub(crate) fn reject_checkpoint_candidate(&mut self) {
        let fork = self
            .fork
            .take()
            .expect("logical stack rejection requires a candidate fork");
        while self.undo.len() > fork.selected_undo {
            let mut inverse = self.undo.pop().expect("candidate undo suffix exists");
            std::mem::swap(&mut self.rows[inverse.index], &mut inverse.old);
        }
        while let Some(mut inverse) = self.detached_prior.pop() {
            std::mem::swap(&mut self.rows[inverse.index], &mut inverse.old);
            self.undo.push(inverse);
        }
        self.top = fork.accepted_top;
    }

    pub(crate) fn accept_checkpoint_candidate(&mut self) {
        let _fork = self
            .fork
            .take()
            .expect("logical stack acceptance requires a candidate fork");
        self.detached_prior.clear();
    }

    fn record(&mut self, index: usize) {
        if !self.recording.get() {
            return;
        }
        self.undo.push(ElementUndo {
            index,
            old: self.rows[index].clone(),
        });
    }
}

impl<T> LogicalStack<T> {
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
                self.undo
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ElementUndo<T>>()),
            )
            .saturating_add(
                self.detached_prior
                    .capacity()
                    .saturating_mul(std::mem::size_of::<ElementUndo<T>>()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::LogicalStack;

    #[test]
    fn pop_then_push_preserves_the_rollback_reachable_row() {
        let mut stack = LogicalStack::default();
        stack.push(1);
        stack.push(2);
        let mark = stack.mark().expect("small stack marks");

        assert_eq!(stack.pop(), Some(2));
        stack.push(3);
        assert_eq!(stack.as_slice(), &[1, 3]);

        assert!(stack.restore(mark));
        assert_eq!(stack.as_slice(), &[1, 2]);
    }
}

//! Dynamically scoped rollback for recursive execution submodes.

use tex_state::{ExpansionState, Universe};

use crate::ModeNest;

/// A live-call-stack transaction over executor-owned mode roots and semantic state.
///
/// This capability is deliberately crate-private, lifetime-bound, and not
/// cloneable. Dropping it before [`Self::commit`] restores both owned roots.
pub(crate) struct ExecutionTransaction<'a> {
    nest: &'a mut ModeNest,
    stores: &'a mut Universe,
    nest_before: Option<ModeNest>,
    group_depth_before: u32,
}

impl<'a> ExecutionTransaction<'a> {
    pub(crate) fn begin(nest: &'a mut ModeNest, stores: &'a mut Universe) -> Self {
        let nest_before = nest.clone();
        let group_depth_before = stores.execution_group_depth();
        Self {
            nest,
            stores,
            nest_before: Some(nest_before),
            group_depth_before,
        }
    }

    pub(crate) fn parts(&mut self) -> (&mut ModeNest, &mut Universe) {
        (self.nest, self.stores)
    }

    pub(crate) fn commit(mut self) {
        self.nest_before = None;
    }
}

impl Drop for ExecutionTransaction<'_> {
    fn drop(&mut self) {
        let Some(nest) = self.nest_before.take() else {
            return;
        };
        while self.stores.execution_group_depth() > self.group_depth_before {
            let _ = self.stores.leave_group();
        }
        *self.nest = nest;
    }
}

#[cfg(test)]
mod tests {
    use tex_state::Universe;

    use crate::{Mode, ModeNest};

    use super::ExecutionTransaction;

    #[test]
    fn drop_restores_mode_and_scoped_group_roots() {
        let mut nest = ModeNest::new();
        let mut stores = Universe::new();
        {
            let mut transaction = ExecutionTransaction::begin(&mut nest, &mut stores);
            let (nest, stores) = transaction.parts();
            nest.push(Mode::Horizontal);
            stores.enter_group();
            stores.set_count(4, 91);
        }
        assert_eq!(nest.current_mode(), Mode::Vertical);
        assert_eq!(stores.count(4), 0);
    }

    #[test]
    fn explicit_commit_keeps_mode_and_scoped_group_roots() {
        let mut nest = ModeNest::new();
        let mut stores = Universe::new();
        let mut transaction = ExecutionTransaction::begin(&mut nest, &mut stores);
        {
            let (nest, stores) = transaction.parts();
            nest.push(Mode::Horizontal);
            stores.enter_group();
            stores.set_count(4, 91);
        }
        transaction.commit();
        assert_eq!(nest.current_mode(), Mode::Horizontal);
        assert_eq!(stores.count(4), 91);
        let _ = stores.leave_group();
    }
}

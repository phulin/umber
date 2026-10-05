//! Disjoint admitted loans for input-preserving terminal consumption.
use super::*;

/// Immutable live meaning rows, borrowed while assignments and restoration
/// are excluded by the simultaneous restricted group loan.
pub struct CommandMeaningRows<'a, G> {
    bank: &'a MeaningBank<MeaningWord<G>>,
}

impl<G> CommandMeaningRows<'_, G> {
    #[must_use]
    #[inline(always)]
    pub fn get(&self, symbol: Symbol) -> &MeaningWord<G> {
        #[cfg(any(test, feature = "profiling"))]
        crate::meaning::record_dense_meaning_row_access();
        self.bank
            .get_ref(symbol.raw())
            .expect("command symbols are admitted")
    }
}

/// Actual mutable group bookkeeping plus immutable live integer parameters.
/// This loan cannot assign state or restore saved values. A nonempty close
/// declines before mutation so the reader can release its meaning/input loans.
pub struct ResidentGroups<'a, G> {
    parameters: &'a DenseBank<i32, PARAMETER_COUNT>,
    journal: &'a mut Option<SaveJournal<G>>,
    next_lineage: &'a mut u64,
    dependencies: &'a mut crate::DependencyRuntime,
}

impl<G> ResidentGroups<'_, G> {
    #[must_use]
    #[inline(always)]
    pub fn int_param(&self, parameter: IntParam) -> i32 {
        self.parameters
            .get(u32::from(parameter.raw()))
            .expect("validated integer parameter")
            .value
    }

    #[inline(always)]
    pub fn begin_group(&mut self, kind: GroupKind, entered_line: u32) -> Result<(), StateError> {
        self.journal
            .as_mut()
            .expect("admitted journal")
            .begin_group(kind, entered_line, self.next_lineage)
    }

    /// A decline preserves the boundary and every saved owner for real unsave.
    #[inline(always)]
    pub fn end_empty_group(&mut self, kind: GroupKind) -> bool {
        let journal = self.journal.as_mut().expect("admitted journal");
        let Some(&frame) = journal.group_frames().last() else {
            return false;
        };
        if frame.kind() != kind || journal.len() != frame.journal_start as usize {
            return false;
        }
        journal.record_group_exit_with_records(frame);
        self.dependencies
            .poison(crate::TrackedRegionBarrier::EnvironmentTimelineChange);
        true
    }
}

impl<G> DenseState<G> {
    #[inline(always)]
    pub(crate) fn resident_groups<'a>(
        &'a mut self,
        dependencies: &'a mut crate::DependencyRuntime,
    ) -> (CommandMeaningRows<'a, G>, ResidentGroups<'a, G>) {
        (
            CommandMeaningRows {
                bank: &self.meanings,
            },
            ResidentGroups {
                parameters: &self.integer_parameters,
                journal: &mut self.journal,
                next_lineage: &mut self.next_group_lineage,
                dependencies,
            },
        )
    }
}

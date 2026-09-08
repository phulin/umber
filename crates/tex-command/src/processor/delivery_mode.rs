//! Compact authority for persistent exceptional raw-delivery settlement.

/// Conditions that move a delivered token off the ordinary path.
///
/// Rich scanner and alignment values retain context needed by cold handlers.
/// This byte is nevertheless the delivery authority for persistent regime
/// state: transition sites maintain it directly, and resident delivery never
/// reconstructs it from those values. Token-local suppression and outerness
/// are supplied by the already-decoded hot command at settlement time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeliveryMode(u8);

const _: () = assert!(std::mem::size_of::<DeliveryMode>() == 1);

impl DeliveryMode {
    const SCANNER: u8 = 1 << 0;
    const OBSERVING: u8 = 1 << 1;
    const ALIGNMENT: u8 = 1 << 2;
    const TRACING: u8 = 1 << 5;
    const EPISODE: u8 = Self::OBSERVING | Self::TRACING;

    #[inline(always)]
    const fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }

    pub(crate) const fn set_scanner_active(&mut self, active: bool) {
        self.set(Self::SCANNER, active);
    }

    pub(crate) const fn set_alignment_active(&mut self, active: bool) {
        self.set(Self::ALIGNMENT, active);
    }

    pub(crate) const fn begin_episode(&mut self, observing: bool, tracing: bool) {
        self.0 &= !Self::EPISODE;
        self.set(Self::OBSERVING, observing);
        self.set(Self::TRACING, tracing);
    }

    pub(crate) const fn end_episode(&mut self) {
        self.0 &= !Self::EPISODE;
    }

    pub(crate) const fn set_observing(&mut self, observing: bool) {
        self.set(Self::OBSERVING, observing);
    }

    #[inline(always)]
    pub(crate) const fn requires_slow_settlement(
        self,
        suppresses_expandable_control_sequence: bool,
        outer: bool,
    ) -> bool {
        self.observing()
            || self.requires_semantic_settlement(suppresses_expandable_control_sequence, outer)
    }

    /// Semantic settlement only; observation is selected at delivery entry.
    #[inline(always)]
    pub(crate) const fn requires_semantic_settlement(
        self,
        suppresses_expandable_control_sequence: bool,
        outer: bool,
    ) -> bool {
        self.alignment_active()
            || suppresses_expandable_control_sequence
            || (outer && self.scanner_active())
    }

    /// Returns whether persistent delivery state needs settlement after the
    /// token-local facts have already been consumed.
    #[inline(always)]
    pub(crate) const fn requires_persistent_settlement(self) -> bool {
        self.requires_slow_settlement(false, false)
    }

    #[inline(always)]
    pub(crate) const fn allows_character_run(self) -> bool {
        self.0 & (Self::SCANNER | Self::OBSERVING | Self::TRACING) == 0
    }

    pub(crate) const fn scanner_active(self) -> bool {
        self.0 & Self::SCANNER != 0
    }

    pub(crate) const fn observing(self) -> bool {
        self.0 & Self::OBSERVING != 0
    }

    pub(crate) const fn alignment_active(self) -> bool {
        self.0 & Self::ALIGNMENT != 0
    }

    pub(crate) const fn tracing(self) -> bool {
        self.0 & Self::TRACING != 0
    }
}

#[cfg(test)]
mod tests {
    use super::DeliveryMode;
    use crate::CommandState;
    use crate::conditionals::ConditionalKind;
    use crate::processor::status::{ConditionId, ScannerWarning, SkippingContext};
    use crate::processor::{AlignmentIdentity, ScannerStatus};

    #[test]
    fn persistent_state_is_unchanged_by_token_facts() {
        let mut mode = DeliveryMode::default();
        mode.set_alignment_active(true);
        assert!(mode.alignment_active());
        let before = mode;
        assert!(mode.requires_slow_settlement(true, false));
        assert!(mode.requires_slow_settlement(false, true));
        assert_eq!(mode, before);
    }

    #[test]
    fn semantic_transition_sites_maintain_the_persistent_bits() {
        let mut state = CommandState::<()>::default();
        let scanner = state.begin_scanner_status(ScannerStatus::Skipping(SkippingContext {
            condition: ConditionId(1),
            warning: ScannerWarning(2),
            skip_line: 3,
            conditional: ConditionalKind::IfTrue,
        }));
        assert!(state.delivery_mode.scanner_active());
        state.restore_scanner_status(scanner);
        assert!(!state.delivery_mode.scanner_active());

        let alignment = AlignmentIdentity::new(1);
        state.begin_alignment(alignment);
        assert!(state.delivery_mode.alignment_active());
        state.suspend_alignment(alignment).expect("suspend");
        assert!(!state.delivery_mode.alignment_active());
        state.resume_alignment(alignment).expect("resume");
        assert!(state.delivery_mode.alignment_active());
        state.finish_alignment(alignment).expect("finish");
        assert!(!state.delivery_mode.alignment_active());
    }

    #[test]
    fn slow_settlement_matches_the_previous_boolean_rule() {
        for scanner in [false, true] {
            for observing in [false, true] {
                for alignment in [false, true] {
                    for tracing in [false, true] {
                        let mut mode = DeliveryMode::default();
                        mode.begin_episode(observing, tracing);
                        mode.set_scanner_active(scanner);
                        mode.set_alignment_active(alignment);
                        for suppresses in [false, true] {
                            for outer in [false, true] {
                                let old = observing || alignment || suppresses || scanner && outer;
                                assert_eq!(
                                    mode.requires_slow_settlement(suppresses, outer),
                                    old,
                                    "persistent_bits=(scanner={scanner}, observing={observing}, alignment={alignment}, tracing={tracing}), suppresses={suppresses}, outer={outer}"
                                );
                            }
                        }
                    }
                }
            }
        }

        let mut mode = DeliveryMode::default();
        mode.set_scanner_active(true);
        assert!(!mode.requires_persistent_settlement());
        assert!(!mode.requires_slow_settlement(false, false));
    }
}

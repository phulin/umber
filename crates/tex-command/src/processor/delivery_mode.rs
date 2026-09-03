//! Compact authority for exceptional raw-delivery settlement.

/// Conditions that move a delivered token off the ordinary path.
///
/// Rich scanner and alignment values retain context needed by cold handlers.
/// This byte is nevertheless the delivery authority: transition sites
/// maintain it directly, and resident delivery never reconstructs it from
/// those values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeliveryMode(u8);

const _: () = assert!(std::mem::size_of::<DeliveryMode>() == 1);

impl DeliveryMode {
    const SCANNER: u8 = 1 << 0;
    const OBSERVING: u8 = 1 << 1;
    const ALIGNMENT: u8 = 1 << 2;
    const SUPPRESS_NEXT: u8 = 1 << 3;
    const OUTER: u8 = 1 << 4;
    const TRACING: u8 = 1 << 5;
    const TOKEN: u8 = Self::SUPPRESS_NEXT | Self::OUTER;
    const EPISODE: u8 = Self::OBSERVING | Self::TRACING | Self::TOKEN;

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
    pub(crate) const fn begin_token(&mut self, suppress_next: bool, outer: bool) {
        self.0 &= !Self::TOKEN;
        self.set(Self::SUPPRESS_NEXT, suppress_next);
        self.set(Self::OUTER, outer);
    }

    #[inline(always)]
    pub(crate) const fn requires_slow_settlement(self) -> bool {
        self.0 != 0
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

    pub(crate) const fn suppresses_next(self) -> bool {
        self.0 & Self::SUPPRESS_NEXT != 0
    }

    pub(crate) const fn outer(self) -> bool {
        self.0 & Self::OUTER != 0
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
    fn token_conditions_replace_only_token_bits() {
        let mut mode = DeliveryMode::default();
        mode.set_alignment_active(true);
        mode.begin_token(true, false);
        assert!(mode.alignment_active());
        assert!(mode.suppresses_next());

        mode.begin_token(false, true);
        assert!(mode.alignment_active());
        assert!(!mode.suppresses_next());
        assert!(mode.outer());
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
}

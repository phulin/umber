//! Ephemeral current-command representation.

mod identity;
mod native_meaning;

pub(crate) use identity::{CommandIdentity, ConvertSelector};

use tex_state::CommandContext;
use tex_state::DefinitionRef;
use tex_state::interner::Symbol;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord, ResolvedMeaning, StaticCommandClass,
    UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

/// Profiling-only proof of how current-command ownership changes.
///
/// The counter is thread-local and absent from shipping builds. It records
/// only explicit ownership operations, never semantic command state.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CommandOwnershipCounters {
    pub(crate) backup_copies: u64,
    pub(crate) expansion_moves_in: u64,
    pub(crate) expansion_moves_out: u64,
    pub(crate) resolved_writes: u64,
}

#[cfg(test)]
thread_local! {
    static COMMAND_OWNERSHIP_COUNTERS: core::cell::Cell<CommandOwnershipCounters> =
        const { core::cell::Cell::new(CommandOwnershipCounters {
            backup_copies: 0,
            expansion_moves_in: 0,
            expansion_moves_out: 0,
            resolved_writes: 0,
        }) };
}

#[cfg(test)]
pub(crate) fn command_ownership_counters() -> CommandOwnershipCounters {
    COMMAND_OWNERSHIP_COUNTERS.with(core::cell::Cell::get)
}

#[cfg(test)]
fn update_command_ownership_counters(update: impl FnOnce(&mut CommandOwnershipCounters)) {
    COMMAND_OWNERSHIP_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        update(&mut counters);
        slot.set(counters);
    });
}

/// Compact coordinates and one-delivery policy attached to a hot token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeliverySite {
    control_sequence: Option<Symbol>,
    direct_source_line: u32,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
    delivery_flags: CommandDeliveryFlags,
}

/// The one delivered native token/site and command word.
///
/// Remaining semantic consumers and exceptional delivery retain this value.
/// Ordinary native input, expansion and scanners do not construct it. Printed
/// spelling, full meaning and diagnostic geometry are projected only by their
/// consumers. Native fields are copyable coordinates,
/// not immutable definition owners; backup retains spelling and resolves again.
#[derive(Debug)]
pub struct CurrentCommand<G> {
    word: TokenWord,
    origin: OriginId,
    site: DeliverySite,
    command: MeaningWord<G>,
}

/// Scalar delivery facts shared by the raw, resolved, and recovery phases.
///
/// Native word paths do not construct these flags. When a semantic consumer
/// requires a delivered command, its actual site facts stay attached through
/// scanning, execution and backup.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
struct CommandDeliveryFlags(u8);

impl CommandDeliveryFlags {
    const DIRECT_SOURCE: u8 = 1 << 0;
    const SUPPRESS_EXPANDABLE: u8 = 1 << 1;
    const OUTER_RECOVERY_SPACE: u8 = 1 << 2;
    const HAS_DIRECT_SOURCE_LINE: u8 = 1 << 3;
    const NOEXPAND_FROZEN_RELAX: u8 = 1 << 4;

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    const fn delivery(direct_source: bool, has_line: bool, suppress: bool) -> Self {
        Self(
            ((direct_source as u8) * Self::DIRECT_SOURCE)
                | ((has_line as u8) * Self::HAS_DIRECT_SOURCE_LINE)
                | ((suppress as u8) * Self::SUPPRESS_EXPANDABLE),
        )
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }
}

impl<G> Copy for CurrentCommand<G> {}

impl<G> Clone for CurrentCommand<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> PartialEq for CurrentCommand<G> {
    fn eq(&self, other: &Self) -> bool {
        self.word == other.word
            && self.origin == other.origin
            && self.site == other.site
            && self.command == other.command
    }
}

impl<G> Eq for CurrentCommand<G> {}

impl<G> core::hash::Hash for CurrentCommand<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.word.hash(state);
        self.origin.hash(state);
        self.site.hash(state);
        if let Some(word) = self.command.scalar_word() {
            0_u8.hash(state);
            word.hash(state);
        } else if let Some(font) = self.command.font_id() {
            1_u8.hash(state);
            font.hash(state);
        } else {
            let (flags, definition) = self.command.macro_parts().expect("native macro");
            2_u8.hash(state);
            flags.hash(state);
            definition.hash(state);
        }
    }
}

impl<G> CurrentCommand<G> {
    /// Initializes or overwrites a caller that explicitly requests raw command delivery.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_delivery_into(
        destination: &mut Option<Self>,
        word: TokenWord,
        origin: OriginId,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        state: &CommandContext<'_, G>,
    ) {
        if let Some(command) = destination {
            command.write_resolved_delivery(
                word,
                origin,
                direct_source,
                direct_source_line,
                suppress_expandable,
                state,
            )
        } else {
            let command = CurrentCommand::from_resolved_delivery(
                word,
                origin,
                direct_source,
                direct_source_line,
                suppress_expandable,
                state,
            );
            *destination = Some(command);
        }
    }
}

impl<G> CurrentCommand<G> {
    /// Resolves a resident word into a newly-owned hot command without first
    /// constructing an empty destination. The command word is initialized to
    /// a valid value before the packed resolver overwrites it; unlike
    /// [`Self::empty`], no placeholder token or delivery stamp is installed.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn from_resolved_delivery(
        word: TokenWord,
        origin: OriginId,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        state: &CommandContext<'_, G>,
    ) -> Self {
        let mut meaning = MeaningWord::scalar(Meaning::UNDEFINED_WORD);
        let control_sequence = state.write_packed_token_command_into(word, &mut meaning);
        Self::from_delivered_parts(
            word,
            origin,
            direct_source,
            direct_source_line,
            suppress_expandable,
            meaning,
            control_sequence,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn from_delivered_parts(
        word: TokenWord,
        origin: OriginId,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        meaning: MeaningWord<G>,
        control_sequence: Option<Symbol>,
    ) -> Self {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.resolved_writes = counters.resolved_writes.saturating_add(1);
        });
        Self {
            word,
            origin,
            site: DeliverySite {
                control_sequence,
                direct_source_line: direct_source_line.unwrap_or(0),
                alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                delivery_flags: CommandDeliveryFlags::delivery(
                    direct_source,
                    direct_source_line.is_some(),
                    suppress_expandable,
                ),
            },
            command: meaning,
        }
    }

    #[inline(always)]
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            word: TokenWord::pack(Token::Char {
                ch: '\0',
                cat: Catcode::Ignored,
            }),
            origin: OriginId::UNKNOWN,
            site: DeliverySite {
                control_sequence: None,
                direct_source_line: 0,
                alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                delivery_flags: CommandDeliveryFlags::default(),
            },
            command: MeaningWord::scalar(Meaning::UNDEFINED_WORD),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn write_resolved_delivery(
        &mut self,
        word: TokenWord,
        origin: OriginId,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        state: &CommandContext<'_, G>,
    ) {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.resolved_writes = counters.resolved_writes.saturating_add(1);
        });
        self.word = word;
        self.origin = origin;
        self.site = DeliverySite {
            control_sequence: None,
            direct_source_line: direct_source_line.unwrap_or(0),
            alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
            delivery_flags: CommandDeliveryFlags::delivery(
                direct_source,
                direct_source_line.is_some(),
                suppress_expandable,
            ),
        };
        self.site.control_sequence = state.write_packed_token_command_into(word, &mut self.command);
    }

    pub(crate) const fn spelling_word(&self) -> TokenWord {
        self.word
    }

    /// Whether the settled effective meaning is TeX's paragraph command.
    /// This is kept on the compact command so the matcher does not project a
    /// `ResolvedMeaning` merely to identify `\par`.
    #[inline(always)]
    pub(crate) fn is_effective_paragraph(&self) -> bool {
        self.unexpandable_primitive() == Some(UnexpandablePrimitive::Par)
    }

    /// Whether this compact delivery was replaced by TeX82 §23's temporary
    /// recovery space.  Macro matching and raw token collection consume this
    /// fact directly; a rich command is needed only at a genuine recovery
    /// boundary.
    pub(crate) const fn is_outer_recovery_space(&self) -> bool {
        self.site
            .delivery_flags
            .contains(CommandDeliveryFlags::OUTER_RECOVERY_SPACE)
    }

    pub(crate) fn set_alignment_adjustment(
        &mut self,
        adjustment: crate::processor::AlignmentDeliveryAdjustment,
    ) {
        self.site.alignment_adjustment = adjustment;
    }

    pub(crate) const fn suppresses_expandable_control_sequence(&self) -> bool {
        self.site
            .delivery_flags
            .contains(CommandDeliveryFlags::SUPPRESS_EXPANDABLE)
    }

    pub(crate) fn suppress_expandable(&mut self) {
        let suppress = self.command.is_macro()
            || self.command.scalar_word().is_some_and(|word| {
                match Meaning::runtime_word_class(word) {
                    StaticCommandClass::Undefined => true,
                    StaticCommandClass::Expandable => {
                        ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
                            != Some(ExpandablePrimitive::EndCsName)
                    }
                    _ => false,
                }
            });
        if suppress {
            self.command = MeaningWord::scalar(Meaning::RELAX_WORD);
            self.site
                .delivery_flags
                .set(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX, true);
        }
    }

    pub(crate) const fn is_outer(&self) -> bool {
        self.command.is_outer()
    }

    pub(crate) const fn macro_parts(&self) -> Option<(MeaningFlags, DefinitionRef<G>)> {
        self.command.macro_parts()
    }

    pub(crate) fn convert_end_template_to_endv(&mut self, frozen_endv: Token) {
        self.word = TokenWord::pack(frozen_endv);
        self.command = MeaningWord::scalar(Meaning::END_V_WORD);
        self.site.control_sequence = None;
        self.site.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
    }

    pub(crate) fn convert_to_end_template(&mut self) {
        self.command = MeaningWord::scalar(Meaning::END_TEMPLATE_WORD);
        self.site.control_sequence = None;
    }
}

impl<G> CurrentCommand<G> {
    /// Returns the command-owned identity selected by raw input delivery.
    pub(crate) fn identity(&self) -> CommandIdentity {
        if self
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
        {
            CommandIdentity::NoExpandFrozenRelax
        } else {
            CommandIdentity::from_meaning(&self.command)
        }
    }

    /// The active character this delivery would have been, had `\\noexpand`
    /// not replaced it with the frozen-`\\relax` command identity.
    ///
    /// TeX82 §506's `get_x_token_or_active_char` recovers exactly this from
    /// the retained `cur_tok`: `cur_cmd:=active_char` and
    /// `cur_chr:=cur_tok-cs_token_flag-active_base`. Only `\\if` and
    /// `\\ifcat` perform that reconstruction — everywhere else a `\\noexpand`
    /// delivery keeps its `relax`/`no_expand_flag` identity.
    pub(crate) fn no_expand_active_character(&self) -> Option<char> {
        if !self
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
        {
            return None;
        }
        match self.spelling().semantic_token() {
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => Some(ch),
            _ => None,
        }
    }

    /// Converts the effective current command to TeX82's recovery space
    /// while preserving the original spelling for diagnostics and exact input
    /// replay. This is the final step of `check_outer_validity`.
    pub(crate) fn recover_as_space(&mut self) {
        // TeX82 §23 has already retained the original `cur_tok` in backup
        // input. Its live current token is now the synthetic space selected
        // by `cur_cmd := spacer; cur_chr := " "`.
        self.word = TokenWord::pack(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        });
        self.origin = OriginId::UNKNOWN;
        self.command = MeaningWord::scalar(Meaning::char_token_word(' ', Catcode::Space));
        self.site.control_sequence = None;
        self.site
            .delivery_flags
            .set(CommandDeliveryFlags::DIRECT_SOURCE, false);
        self.site
            .delivery_flags
            .set(CommandDeliveryFlags::OUTER_RECOVERY_SPACE, true);
    }

    /// Completes TeX82's `get_x_token` conversion of inaccessible
    /// `end_template` to `endv`.
    ///
    /// TeX82 also replaces `cur_cs` with `frozen_endv`, so a later
    /// `back_input` replays the effective `endv` command. The two frozen
    /// control sequences have the same canonical observer spelling
    /// (`endtemplate`), while retaining distinct input semantics.
    pub(crate) const fn alignment_adjustment(
        &self,
    ) -> crate::processor::AlignmentDeliveryAdjustment {
        self.site.alignment_adjustment
    }

    /// Returns the original token spelling, including its delivery origin.
    #[must_use]
    pub const fn spelling(&self) -> TracedTokenWord {
        TracedTokenWord::from_parts(self.word, self.origin)
    }

    /// Returns the effective meaning resolved at this delivery.
    #[must_use]
    pub fn meaning(&self) -> ResolvedMeaning<G> {
        self.command.resolve()
    }

    /// Borrows the delivered packed static word for direct main-control dispatch.
    /// Macro and font capabilities remain in their actual typed command fields.
    #[must_use]
    pub const fn static_word(&self) -> Option<u64> {
        self.command.scalar_word()
    }

    /// Small native family/selector key for the sole mode/command switch.
    #[must_use]
    #[inline]
    pub fn control_code(&self) -> Option<u64> {
        self.command.control_code()
    }

    /// Returns the control-sequence identity, if this spelling resolves via
    /// a control-sequence meaning cell.
    #[must_use]
    pub const fn control_sequence(&self) -> Option<Symbol> {
        self.site.control_sequence
    }

    /// Returns the spelling's diagnostic origin.
    #[must_use]
    pub const fn origin(&self) -> tex_state::token::OriginId {
        self.origin
    }

    /// Returns the physical range only when this delivery came directly from
    /// a source level. Replayed tokens retain their range for diagnostics but
    /// must not masquerade as a second physical-source transition.
    pub(crate) const fn is_direct_source_delivery(&self) -> bool {
        self.site
            .delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
    }

    /// Physical line captured while this exact command was delivered from a
    /// source cursor. Replayed macro and backup commands deliberately carry
    /// no direct line even when their diagnostic provenance names a source.
    #[must_use]
    pub const fn direct_source_line_number(&self) -> Option<u32> {
        if self
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::HAS_DIRECT_SOURCE_LINE)
        {
            Some(self.site.direct_source_line)
        } else {
            None
        }
    }

    /// Makes a fresh copy for the input backup path. native delivery coordinates
    /// carry no immutable definition ownership.
    pub(crate) fn copy_for_backup(&self) -> Self {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.backup_copies = counters.backup_copies.saturating_add(1);
        });
        *self
    }
}

#[cfg(test)]
mod tests;

//! Ephemeral current-command representation.

mod identity;
mod native_meaning;

pub(crate) use identity::{CommandIdentity, ConvertSelector};

use tex_state::CommandContext;
use tex_state::DefinitionRef;
use tex_state::interner::Symbol;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, MeaningWord, ResolvedMeaning, UnexpandablePrimitive,
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

/// TeX's directly branchable `cur_cmd` class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandClass {
    Undefined,
    Relax,
    Character,
    Macro,
    Expandable,
    EndV,
    Unexpandable,
    Font,
    Value,
}

/// Compact coordinates and one-delivery policy attached to a hot token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeliverySite {
    control_sequence: Option<Symbol>,
    direct_source_line: u32,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
    delivery_flags: CommandDeliveryFlags,
}

/// The token-only value advanced by the selected input frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct HotToken {
    word: TokenWord,
    origin: OriginId,
    site: DeliverySite,
}

/// The one delivered native token/site and command word.
///
/// Raw input, expansion, synchronous scanners and main control use this same
/// opaque value. Printed spelling, full meaning and diagnostic geometry are
/// projected only by their consumers. Native fields are copyable coordinates,
/// not immutable definition owners; backup retains spelling and resolves again.
#[derive(Debug)]
pub struct CurrentCommand<G> {
    token: HotToken,
    command: MeaningWord<G>,
}

/// Scalar delivery facts shared by the raw, resolved, and recovery phases.
///
/// These bits replace the raw input-frame kind/flags and the final command's
/// separate booleans. The caller-owned command slot is the only delivery
/// representation, so each fact is written once and remains attached to the
/// same value through scanning, execution, backup, or suspension.
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
        self.token == other.token && self.command == other.command
    }
}

impl<G> Eq for CurrentCommand<G> {}

impl<G> core::hash::Hash for CurrentCommand<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.token.hash(state);
        match self.command {
            MeaningWord::Static(word) => {
                0_u8.hash(state);
                word.hash(state);
            }
            MeaningWord::Font(font) => {
                1_u8.hash(state);
                font.hash(state);
            }
            MeaningWord::Macro { flags, definition } => {
                2_u8.hash(state);
                flags.hash(state);
                definition.hash(state);
            }
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
    ) -> tex_state::token::PackedMeaningResolution {
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
            let (command, resolution) = CurrentCommand::from_resolved_delivery(
                word,
                origin,
                direct_source,
                direct_source_line,
                suppress_expandable,
                state,
            );
            *destination = Some(command);
            resolution
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
    ) -> (Self, tex_state::token::PackedMeaningResolution) {
        let mut meaning = MeaningWord::Static(Meaning::UNDEFINED_WORD);
        let resolution = state.write_packed_token_command_into(word, &mut meaning);
        (
            Self::from_delivered_parts(
                word,
                origin,
                direct_source,
                direct_source_line,
                suppress_expandable,
                meaning,
                resolution.control_sequence(),
            ),
            resolution,
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
            token: HotToken {
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
            },
            command: meaning,
        }
    }

    #[inline(always)]
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            token: HotToken {
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
            },
            command: MeaningWord::Static(Meaning::UNDEFINED_WORD),
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
    ) -> tex_state::token::PackedMeaningResolution {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.resolved_writes = counters.resolved_writes.saturating_add(1);
        });
        self.token = HotToken {
            word,
            origin,
            site: DeliverySite {
                control_sequence: None,
                direct_source_line: direct_source_line.unwrap_or(0),
                alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                delivery_flags: CommandDeliveryFlags::delivery(
                    direct_source,
                    direct_source_line.is_some(),
                    suppress_expandable,
                ),
            },
        };
        let resolution = state.write_packed_token_command_into(word, &mut self.command);
        self.token.site.control_sequence = resolution.control_sequence();
        resolution
    }

    pub(crate) const fn spelling_word(&self) -> TokenWord {
        self.token.word
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
        self.token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::OUTER_RECOVERY_SPACE)
    }

    pub(crate) fn set_alignment_adjustment(
        &mut self,
        adjustment: crate::processor::AlignmentDeliveryAdjustment,
    ) {
        self.token.site.alignment_adjustment = adjustment;
    }

    pub(crate) const fn suppresses_expandable_control_sequence(&self) -> bool {
        self.token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::SUPPRESS_EXPANDABLE)
    }

    pub(crate) fn suppress_expandable(&mut self) {
        if self.expandable_primitive() != Some(ExpandablePrimitive::EndCsName)
            && matches!(
                self.class(),
                CommandClass::Undefined | CommandClass::Macro | CommandClass::Expandable
            )
        {
            self.command = MeaningWord::Static(Meaning::RELAX_WORD);
            self.token
                .site
                .delivery_flags
                .set(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX, true);
        }
    }

    pub(crate) const fn is_outer(&self) -> bool {
        self.command.is_outer()
    }

    pub(crate) const fn macro_parts(&self) -> Option<(MeaningFlags, DefinitionRef<G>)> {
        match self.command {
            MeaningWord::Macro { flags, definition } => Some((flags, definition)),
            _ => None,
        }
    }

    pub(crate) fn convert_end_template_to_endv(&mut self, frozen_endv: Token) {
        self.token.word = TokenWord::pack(frozen_endv);
        self.command = MeaningWord::Static(Meaning::END_V_WORD);
        self.token.site.control_sequence = None;
        self.token.site.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
    }

    pub(crate) fn convert_to_end_template(&mut self) {
        self.command = MeaningWord::Static(Meaning::END_TEMPLATE_WORD);
        self.token.site.control_sequence = None;
    }
}

impl<G> CurrentCommand<G> {
    /// Resolves one delivered spelling into TeX's effective current command.
    ///
    /// TeX82's `get_next` preserves the input token as `cur_tok` while it
    /// obtains `cur_cmd`/`cur_chr` from either that character spelling or the
    /// current meaning of its control sequence. Active characters use their
    /// separate control-sequence namespace; escaped control sequences retain
    /// their original spelling even after their meaning changes.
    pub(crate) fn resolve(
        spelling: TracedTokenWord,
        direct_source: bool,
        direct_source_line: Option<u32>,
        state: &CommandContext<'_, G>,
    ) -> Self {
        Self::from_resolved_delivery(
            spelling.token_word(),
            spelling.origin(),
            direct_source,
            direct_source_line,
            false,
            state,
        )
        .0
    }

    /// Returns the command-owned identity selected by raw input delivery.
    pub(crate) fn identity(&self) -> CommandIdentity {
        if self
            .token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
        {
            CommandIdentity::NoExpandFrozenRelax
        } else {
            CommandIdentity::from_command(self)
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
            .token
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
        self.token.word = TokenWord::pack(Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        });
        self.token.origin = OriginId::UNKNOWN;
        self.command = MeaningWord::Static(Meaning::char_token_word(' ', Catcode::Space));
        self.token.site.control_sequence = None;
        self.token
            .site
            .delivery_flags
            .set(CommandDeliveryFlags::DIRECT_SOURCE, false);
        self.token
            .site
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
        self.token.site.alignment_adjustment
    }

    /// Returns the original token spelling, including its delivery origin.
    #[must_use]
    pub const fn spelling(&self) -> TracedTokenWord {
        TracedTokenWord::from_parts(self.token.word, self.token.origin)
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
        match self.command {
            MeaningWord::Static(word) => Some(word),
            _ => None,
        }
    }

    /// Native packed command family and modifier for mode/command dispatch.
    #[must_use]
    #[inline]
    pub fn control_code(&self) -> Option<u64> {
        match self.command {
            MeaningWord::Static(word) => Some(Meaning::runtime_word_control_code(word)),
            MeaningWord::Font(font) => Some(Meaning::runtime_word_control_code(
                Meaning::Font(font).encode(),
            )),
            MeaningWord::Macro { .. } => None,
        }
    }

    /// Returns the control-sequence identity, if this spelling resolves via
    /// a control-sequence meaning cell.
    #[must_use]
    pub const fn control_sequence(&self) -> Option<Symbol> {
        self.token.site.control_sequence
    }

    #[cfg(test)]
    pub(crate) fn clear_control_sequence_for_test(&mut self) {
        self.token.site.control_sequence = None;
    }

    /// Returns the spelling's diagnostic origin.
    #[must_use]
    pub const fn origin(&self) -> tex_state::token::OriginId {
        self.token.origin
    }

    /// Returns the physical range only when this delivery came directly from
    /// a source level. Replayed tokens retain their range for diagnostics but
    /// must not masquerade as a second physical-source transition.
    pub(crate) const fn is_direct_source_delivery(&self) -> bool {
        self.token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
    }

    /// Physical line captured while this exact command was delivered from a
    /// source cursor. Replayed macro and backup commands deliberately carry
    /// no direct line even when their diagnostic provenance names a source.
    #[must_use]
    pub const fn direct_source_line_number(&self) -> Option<u32> {
        if self
            .token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::HAS_DIRECT_SOURCE_LINE)
        {
            Some(self.token.site.direct_source_line)
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

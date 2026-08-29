//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{SourceLocation, SourceProvenance, SourceRange};

/// Profiling-only proof of how current-command ownership changes.
///
/// The counter is thread-local and absent from shipping builds. It records
/// only explicit ownership operations, never semantic command state.
#[cfg(any(test, feature = "profiling"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CommandOwnershipCounters {
    pub(crate) clones: u64,
    pub(crate) backup_copies: u64,
    pub(crate) expansion_moves_in: u64,
    pub(crate) expansion_moves_out: u64,
}

#[cfg(any(test, feature = "profiling"))]
thread_local! {
    static COMMAND_OWNERSHIP_COUNTERS: core::cell::Cell<CommandOwnershipCounters> =
        const { core::cell::Cell::new(CommandOwnershipCounters {
            clones: 0,
            backup_copies: 0,
            expansion_moves_in: 0,
            expansion_moves_out: 0,
        }) };
}

#[cfg(any(test, feature = "profiling"))]
pub(crate) fn command_ownership_counters() -> CommandOwnershipCounters {
    COMMAND_OWNERSHIP_COUNTERS.with(core::cell::Cell::get)
}

#[cfg(any(test, feature = "profiling"))]
fn update_command_ownership_counters(update: impl FnOnce(&mut CommandOwnershipCounters)) {
    COMMAND_OWNERSHIP_COUNTERS.with(|slot| {
        let mut counters = slot.get();
        update(&mut counters);
        slot.set(counters);
    });
}

pub(crate) fn record_expansion_command_move_in() {
    #[cfg(any(test, feature = "profiling"))]
    update_command_ownership_counters(|counters| {
        counters.expansion_moves_in = counters.expansion_moves_in.saturating_add(1);
    });
}

pub(crate) fn record_expansion_command_move_out() {
    #[cfg(any(test, feature = "profiling"))]
    update_command_ownership_counters(|counters| {
        counters.expansion_moves_out = counters.expansion_moves_out.saturating_add(1);
    });
}

/// One command delivery, equivalent to TeX's `cur_cmd`, `cur_chr`, `cur_cs`,
/// and `cur_tok`.
///
/// This value is normally call-local and remains absent at durable named
/// checkpoints. A resource-suspended expanded scanner may retain exactly one
/// current command as its typed continuation; its delivery stamp identifies
/// that exact live cursor transition and is never reconstructed from token
/// equality.
#[derive(Debug)]
pub struct CurrentCommand<G> {
    spelling: TracedTokenWord,
    meaning: ResolvedMeaning<G>,
    identity: CommandIdentity,
    control_sequence: Option<Symbol>,
    delivery: DeliveryStamp,
    source_provenance: Option<SourceProvenance>,
    direct_source_line: Option<u32>,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
    delivery_flags: CommandDeliveryFlags,
}

/// Exclusive proof that the caller-owned command slot is ready for raw input.
///
/// The wrapper is only one mutable reference. It adds no storage and cannot
/// outlive the caller's final [`CurrentCommand`]. Consuming it is the only
/// production route to [`RawCommand`], so cold input transitions cannot leave
/// a half-initialized command borrow live.
pub(crate) struct EmptyCommand<'slot, G>(&'slot mut CurrentCommand<G>);

/// Exclusive proof that raw spelling, provenance, and delivery coordinates
/// have been written into the caller-owned command slot.
pub(crate) struct RawCommand<'slot, G>(&'slot mut CurrentCommand<G>);

/// Exclusive proof that meaning resolution completed in the same command
/// slot. Delivery policy may now settle noexpand, outer validity, alignment,
/// and observation exactly once.
pub(crate) struct ResolvedCommand<'slot, G>(&'slot mut CurrentCommand<G>);

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

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }
}

impl<G> Clone for CurrentCommand<G> {
    fn clone(&self) -> Self {
        #[cfg(any(test, feature = "profiling"))]
        update_command_ownership_counters(|counters| {
            counters.clones = counters.clones.saturating_add(1);
        });
        Self {
            spelling: self.spelling,
            meaning: self.meaning.clone(),
            identity: self.identity,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            source_provenance: self.source_provenance,
            direct_source_line: self.direct_source_line,
            alignment_adjustment: self.alignment_adjustment,
            delivery_flags: self.delivery_flags,
        }
    }
}

impl<G> PartialEq for CurrentCommand<G> {
    fn eq(&self, other: &Self) -> bool {
        self.spelling == other.spelling
            && self.meaning == other.meaning
            && self.identity == other.identity
            && self.control_sequence == other.control_sequence
            && self.delivery == other.delivery
            && self.source_provenance == other.source_provenance
            && self.direct_source_line == other.direct_source_line
            && self.alignment_adjustment == other.alignment_adjustment
            && self.delivery_flags == other.delivery_flags
    }
}

impl<G> Eq for CurrentCommand<G> {}

impl<G> core::hash::Hash for CurrentCommand<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.spelling.hash(state);
        self.meaning.hash(state);
        self.identity.hash(state);
        self.control_sequence.hash(state);
        self.delivery.hash(state);
        self.source_provenance.hash(state);
        self.direct_source_line.hash(state);
        self.alignment_adjustment.hash(state);
        self.delivery_flags.hash(state);
    }
}

/// The command-code identity of a current delivery.
///
/// TeX82's `get_next` gives an expandable control sequence replayed by
/// `\\noexpand` the inaccessible frozen-`\\relax` command identity: its
/// effective meaning is `relax`, but its `cur_chr` is `no_expand_flag` (257),
/// rather than the ordinary `relax` value (256). Separately, TeX82 §25's
/// `\expandafter`, `\csname`, and its `\endcsname` boundary have dedicated
/// command identities rather than the generic expandable fallback. TeX82's
/// text conversions likewise share the `convert` command with a selector
/// owned by the delivered primitive. Its diagnostic commands likewise share
/// `xray`, with a selector installed by the primitive table. These remain
/// ephemeral with the current delivery and are never stored in snapshots or
/// input payloads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CommandIdentity {
    Ordinary,
    ExpandAfter,
    CsName,
    EndCsName,
    Convert(ConvertSelector),
    XRay(XRaySelector),
    NoExpandFrozenRelax,
}

/// TeX82's `convert` selectors.
///
/// TeX.web §35 installs the classic conversion primitives with these values;
/// §27's `conv_toks` then consumes the selector while retaining the original
/// `convert` current-command identity throughout the conversion episode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ConvertSelector {
    Number,
    RomanNumeral,
    String,
    Meaning,
    FontName,
    JobName,
}

impl ConvertSelector {
    const fn from_primitive(primitive: tex_state::meaning::ExpandablePrimitive) -> Option<Self> {
        use tex_state::meaning::ExpandablePrimitive;

        match primitive {
            ExpandablePrimitive::Number => Some(Self::Number),
            ExpandablePrimitive::RomanNumeral => Some(Self::RomanNumeral),
            ExpandablePrimitive::String => Some(Self::String),
            ExpandablePrimitive::Meaning => Some(Self::Meaning),
            ExpandablePrimitive::FontName => Some(Self::FontName),
            ExpandablePrimitive::JobName => Some(Self::JobName),
            _ => None,
        }
    }

    pub(crate) const fn operand(self) -> i64 {
        match self {
            Self::Number => 0,
            Self::RomanNumeral => 1,
            Self::String => 2,
            Self::Meaning => 3,
            Self::FontName => 4,
            Self::JobName => 5,
        }
    }
}

/// TeX82's `xray` selectors.
///
/// TeX.web §18 installs the diagnostic primitives under the shared `xray`
/// command: `\show`, `\showbox`, `\showthe`, and `\showlists`. Their
/// selector is current-command identity; the executor still owns each
/// primitive's distinct typed diagnostic behavior.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum XRaySelector {
    Show,
    ShowBox,
    ShowThe,
    ShowLists,
}

impl XRaySelector {
    const fn from_primitive(primitive: tex_state::meaning::UnexpandablePrimitive) -> Option<Self> {
        use tex_state::meaning::UnexpandablePrimitive;

        match primitive {
            UnexpandablePrimitive::Show => Some(Self::Show),
            UnexpandablePrimitive::ShowBox => Some(Self::ShowBox),
            UnexpandablePrimitive::ShowThe => Some(Self::ShowThe),
            UnexpandablePrimitive::ShowLists => Some(Self::ShowLists),
            _ => None,
        }
    }

    pub(crate) const fn operand(self) -> i64 {
        match self {
            Self::Show => 0,
            Self::ShowBox => 1,
            Self::ShowThe => 2,
            Self::ShowLists => 3,
        }
    }
}

impl CommandIdentity {
    fn from_meaning<G>(meaning: &ResolvedMeaning<G>) -> Self {
        match meaning {
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                tex_state::meaning::ExpandablePrimitive::ExpandAfter,
            )) => Self::ExpandAfter,
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                tex_state::meaning::ExpandablePrimitive::CsName,
            )) => Self::CsName,
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                tex_state::meaning::ExpandablePrimitive::EndCsName,
            )) => Self::EndCsName,
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(primitive)) => {
                if let Some(selector) = ConvertSelector::from_primitive(*primitive) {
                    Self::Convert(selector)
                } else {
                    Self::Ordinary
                }
            }
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => {
                if let Some(selector) = XRaySelector::from_primitive(*primitive) {
                    Self::XRay(selector)
                } else {
                    Self::Ordinary
                }
            }
            _ => Self::Ordinary,
        }
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
    #[allow(dead_code)] // invoked by the ordered canonical raw-delivery implementation
    pub(crate) fn resolve(
        spelling: TracedTokenWord,
        delivery: DeliveryStamp,
        source_provenance: Option<SourceProvenance>,
        direct_source: bool,
        direct_source_line: Option<u32>,
        state: &CommandContext<'_, G>,
    ) -> Self {
        let mut command = Self::empty();
        command
            .empty_for_raw_delivery()
            .write_raw_delivery(
                spelling,
                delivery.input_level,
                delivery.position,
                source_provenance,
                direct_source,
                direct_source_line,
                false,
            )
            .resolve_in_place(delivery.sequence, state);
        command
    }

    /// Creates the reusable destination value owned by one delivery request.
    ///
    /// Raw input overwrites this value before any semantic consumer can
    /// observe it. Keeping an initialized unresolved meaning avoids unsafe
    /// partial initialization while adding no second command representation.
    #[inline(always)]
    pub(crate) fn empty() -> Self {
        Self {
            spelling: TracedTokenWord::pack(
                Token::Char {
                    ch: '\0',
                    cat: Catcode::Other,
                },
                tex_state::token::OriginId::UNKNOWN,
            ),
            meaning: ResolvedMeaning::Static(Meaning::Undefined),
            identity: CommandIdentity::Ordinary,
            control_sequence: None,
            delivery: DeliveryStamp::new(0, 0, 0),
            source_provenance: None,
            direct_source_line: None,
            alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
            delivery_flags: CommandDeliveryFlags::default(),
        }
    }

    /// Borrows this reusable caller-owned value as an empty raw destination.
    #[inline(always)]
    pub(crate) fn empty_for_raw_delivery(&mut self) -> EmptyCommand<'_, G> {
        EmptyCommand(self)
    }

    /// Returns whether the raw spelling requires a dense meaning lookup.
    #[inline(always)]
    fn raw_requires_meaning_lookup(&self) -> bool {
        matches!(
            self.spelling.semantic_token(),
            Token::Cs(_)
                | Token::Char {
                    cat: Catcode::Active,
                    ..
                }
        )
    }

    /// Returns the input coordinate written by raw input before resolution.
    #[inline(always)]
    fn raw_delivery_coordinate(&self) -> (u64, u64) {
        (self.delivery.input_level, self.delivery.position)
    }

    /// Resolves the raw spelling already held by this final command slot.
    #[inline(always)]
    fn resolve_raw_delivery(&mut self, sequence: u64, state: &CommandContext<'_, G>) {
        self.delivery.sequence = sequence;
        let token = self.spelling.semantic_token();
        match token {
            Token::Cs(symbol) => {
                self.control_sequence = Some(symbol);
                self.meaning = state.compact_control_sequence_meaning(symbol);
            }
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = state.active_character_symbol(ch);
                self.control_sequence = symbol;
                self.meaning = symbol
                    .map(|symbol| state.compact_control_sequence_meaning(symbol))
                    .unwrap_or(ResolvedMeaning::Static(Meaning::Undefined));
            }
            Token::Char { ch, cat } => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::CharToken { ch, cat });
            }
            // `out_param` is converted to a literal replay token before
            // meaning resolution (TeX.web, get_next). A stray parameter token
            // is nevertheless represented deterministically while recovery
            // remains the responsibility of the raw delivery loop.
            Token::Param(_) => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::Undefined);
            }
            // TeX82 §222 keeps `eq_type(undefined_control_sequence)` at
            // `undefined_cs` and `equiv` at `null` for the whole run: the
            // dummy location has no meaning cell an assignment could reach.
            Token::Frozen(_) if token.is_undefined_control_sequence() => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::Undefined);
            }
            Token::Frozen(_) if token.is_frozen_end_template() => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::EndTemplate,
                ));
            }
            Token::Frozen(_) if token.is_frozen_endv() => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::EndV);
            }
            Token::Frozen(_) if token.is_frozen_relax() => {
                self.control_sequence = None;
                self.meaning = ResolvedMeaning::Static(Meaning::Relax);
            }
            // Frozen primitive slots retain their complete registered word.
            // In particular TeX82 §§1369--1371's inaccessible `\endwrite`
            // is an outer macro, not a static primitive; projecting through
            // `Meaning` alone silently reclassified it as `undefined_cs`.
            Token::Frozen(_) => {
                self.control_sequence = None;
                self.meaning = state
                    .frozen_primitive_resolved(token)
                    .unwrap_or(ResolvedMeaning::Static(Meaning::Undefined));
            }
        }
        self.identity = CommandIdentity::from_meaning(&self.meaning);
    }

    pub(crate) const fn suppresses_expandable_control_sequence(&self) -> bool {
        self.delivery_flags
            .contains(CommandDeliveryFlags::SUPPRESS_EXPANDABLE)
    }

    /// Replaces the effective meaning while retaining the exact delivered
    /// spelling and stamp. This is solely TeX82's one-delivery `\\noexpand`
    /// treatment in `get_next` (TeX82 §25). `\endcsname` is represented as
    /// an expandable primitive so the expansion loop can own its dedicated
    /// boundary, but TeX82 §15 assigns `end_cs_name` a command code at or
    /// below `max_command`; §25 therefore preserves it through `\noexpand`.
    pub(crate) fn suppress_expandable(&mut self) {
        if !matches!(self.identity, CommandIdentity::EndCsName)
            && matches!(
                self.meaning,
                ResolvedMeaning::Static(Meaning::Undefined)
                    | ResolvedMeaning::Macro { .. }
                    | ResolvedMeaning::Static(Meaning::ExpandablePrimitive(_))
            )
        {
            self.meaning = ResolvedMeaning::Static(Meaning::Relax);
            self.identity = CommandIdentity::NoExpandFrozenRelax;
        }
    }

    /// Returns the command-owned identity selected by raw input delivery.
    pub(crate) const fn identity(&self) -> CommandIdentity {
        self.identity
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
        if !matches!(self.identity, CommandIdentity::NoExpandFrozenRelax) {
            return None;
        }
        match self.spelling.semantic_token() {
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
        self.spelling = TracedTokenWord::pack(
            Token::Char {
                ch: ' ',
                cat: Catcode::Space,
            },
            tex_state::token::OriginId::UNKNOWN,
        );
        self.meaning = ResolvedMeaning::Static(Meaning::CharToken {
            ch: ' ',
            cat: Catcode::Space,
        });
        self.control_sequence = None;
        self.source_provenance = None;
        self.delivery_flags
            .set(CommandDeliveryFlags::DIRECT_SOURCE, false);
        self.delivery_flags
            .set(CommandDeliveryFlags::OUTER_RECOVERY_SPACE, true);
    }

    /// Replaces an intercepted alignment terminator's effective meaning while
    /// preserving its spelling and delivery proof (TeX.web `get_next`).
    pub(crate) fn convert_to_end_template(&mut self) {
        self.meaning = ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
            tex_state::meaning::ExpandablePrimitive::EndTemplate,
        ));
        self.control_sequence = None;
    }

    /// Whether §23 replaced this delivery by its temporary recovery space.
    ///
    /// The space is TeX's effective current command after the forbidden
    /// outer token has been backed up; it is not an input token for an active
    /// `scan_toks` collector to append before the inserted right brace closes
    /// the runaway text.
    pub(crate) const fn is_outer_recovery_space(&self) -> bool {
        self.delivery_flags
            .contains(CommandDeliveryFlags::OUTER_RECOVERY_SPACE)
    }

    /// Completes TeX82's `get_x_token` conversion of inaccessible
    /// `end_template` to `endv`.
    ///
    /// TeX82 also replaces `cur_cs` with `frozen_endv`, so a later
    /// `back_input` replays the effective `endv` command. The two frozen
    /// control sequences have the same canonical observer spelling
    /// (`endtemplate`), while retaining distinct input semantics.
    pub(crate) fn convert_end_template_to_endv(&mut self, frozen_endv: Token) {
        self.spelling = TracedTokenWord::pack(frozen_endv, self.spelling.origin());
        self.meaning = ResolvedMeaning::Static(Meaning::EndV);
        self.control_sequence = None;
        // The preceding tab/span/cr adjustment belongs to the intercepted
        // delimiter. TeX82 §343 replaces `cur_tok` with frozen end-v before
        // a possible §1131 `back_input`, so replaying end-v must not undo the
        // delimiter's already-committed alignment transition.
        self.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
    }

    pub(crate) fn set_alignment_adjustment(
        &mut self,
        adjustment: crate::processor::AlignmentDeliveryAdjustment,
    ) {
        self.alignment_adjustment = adjustment;
    }

    pub(crate) const fn alignment_adjustment(
        &self,
    ) -> crate::processor::AlignmentDeliveryAdjustment {
        self.alignment_adjustment
    }

    /// Whether this delivery is an outer macro command.
    pub(crate) const fn is_outer(&self) -> bool {
        matches!(
            self.meaning,
            ResolvedMeaning::Macro { flags, .. } if flags.contains(tex_state::meaning::MeaningFlags::OUTER)
        ) || matches!(
            self.meaning,
            ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                tex_state::meaning::ExpandablePrimitive::EndTemplate
            ))
        )
    }

    /// Returns the original token spelling, including its delivery origin.
    #[must_use]
    pub const fn spelling(&self) -> TracedTokenWord {
        self.spelling
    }

    /// Returns the effective meaning resolved at this delivery.
    #[must_use]
    pub fn meaning(&self) -> ResolvedMeaning<G> {
        self.meaning.clone()
    }

    /// Borrows the effective meaning without creating a transient alias.
    #[must_use]
    pub const fn meaning_ref(&self) -> &ResolvedMeaning<G> {
        &self.meaning
    }

    /// Returns the control-sequence identity, if this spelling resolves via
    /// a control-sequence meaning cell.
    #[must_use]
    pub const fn control_sequence(&self) -> Option<Symbol> {
        self.control_sequence
    }

    /// Returns the spelling's diagnostic origin.
    #[must_use]
    pub const fn origin(&self) -> tex_state::token::OriginId {
        self.spelling.origin()
    }

    /// Returns the execution-local proof of this exact input delivery.
    #[must_use]
    pub const fn delivery_stamp(&self) -> DeliveryStamp {
        self.delivery
    }

    /// Returns the committed source spelling range when this command first
    /// originated in a registered physical source. Backup delivery preserves
    /// this range without making it part of token identity.
    #[must_use]
    pub const fn source_range(&self) -> Option<SourceRange> {
        match self.source_provenance {
            Some(provenance) => Some(provenance.range()),
            None => None,
        }
    }

    /// Returns the physical source column of the final byte this command's
    /// spelling consumed, if it originated in registered source input.
    #[must_use]
    pub const fn source_location(&self) -> Option<SourceLocation> {
        match self.source_provenance {
            Some(provenance) => Some(provenance.location()),
            None => None,
        }
    }

    /// Returns retained physical provenance for diagnostic consumers.
    #[must_use]
    pub const fn source_provenance(&self) -> Option<SourceProvenance> {
        self.source_provenance
    }

    /// Returns the physical range only when this delivery came directly from
    /// a source level. Replayed tokens retain their range for diagnostics but
    /// must not masquerade as a second physical-source transition.
    pub(crate) const fn direct_source_provenance(&self) -> Option<SourceProvenance> {
        if self
            .delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
        {
            self.source_provenance
        } else {
            None
        }
    }

    /// Physical line captured while this exact command was delivered from a
    /// source cursor. Replayed macro and backup commands deliberately carry
    /// no direct line even when their diagnostic provenance names a source.
    #[must_use]
    pub const fn direct_source_line_number(&self) -> Option<u32> {
        if self
            .delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
        {
            self.direct_source_line
        } else {
            None
        }
    }

    /// Makes a fresh copy for the input backup path. `CurrentCommand` itself
    /// remains deliberately non-`Clone` at the public boundary.
    pub(crate) fn copy_for_backup(&self) -> Self {
        #[cfg(any(test, feature = "profiling"))]
        update_command_ownership_counters(|counters| {
            counters.backup_copies = counters.backup_copies.saturating_add(1);
        });
        Self {
            spelling: self.spelling,
            meaning: self.meaning.clone(),
            identity: self.identity,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            source_provenance: self.source_provenance,
            direct_source_line: self.direct_source_line,
            alignment_adjustment: self.alignment_adjustment,
            delivery_flags: self.delivery_flags,
        }
    }
}

impl<'slot, G> EmptyCommand<'slot, G> {
    /// Writes one raw input transition into the final caller-owned slot.
    ///
    /// Parameter substitution may discard this proof and overwrite the slot
    /// on a later input transition. No semantic consumer can obtain a
    /// [`ResolvedCommand`] until resolution consumes the resulting raw proof.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn write_raw_delivery(
        self,
        spelling: TracedTokenWord,
        input_level: u64,
        position: u64,
        source_provenance: Option<SourceProvenance>,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
    ) -> RawCommand<'slot, G> {
        let command = self.0;
        command.spelling = spelling;
        command.delivery = DeliveryStamp::new(input_level, position, 0);
        command.source_provenance = source_provenance;
        command.direct_source_line = direct_source_line;
        command.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
        command.delivery_flags = CommandDeliveryFlags::default();
        command
            .delivery_flags
            .set(CommandDeliveryFlags::DIRECT_SOURCE, direct_source);
        command.delivery_flags.set(
            CommandDeliveryFlags::SUPPRESS_EXPANDABLE,
            suppress_expandable,
        );
        RawCommand(command)
    }
}

impl<'slot, G> RawCommand<'slot, G> {
    #[inline(always)]
    pub(crate) fn spelling(&self) -> TracedTokenWord {
        self.0.spelling
    }

    #[inline(always)]
    pub(crate) fn delivery_coordinate(&self) -> (u64, u64) {
        self.0.raw_delivery_coordinate()
    }

    #[inline(always)]
    pub(crate) fn requires_meaning_lookup(&self) -> bool {
        self.0.raw_requires_meaning_lookup()
    }

    /// Completes the same caller-owned slot in place and ends the dense
    /// meaning borrow before delivery policy can mutate command state.
    #[inline(always)]
    pub(crate) fn resolve_in_place(
        self,
        sequence: u64,
        state: &CommandContext<'_, G>,
    ) -> ResolvedCommand<'slot, G> {
        self.0.resolve_raw_delivery(sequence, state);
        ResolvedCommand(self.0)
    }
}

impl<'slot, G> ResolvedCommand<'slot, G> {
    #[inline(always)]
    pub(crate) fn as_ref(&self) -> &CurrentCommand<G> {
        self.0
    }

    #[inline(always)]
    pub(crate) fn as_mut(&mut self) -> &mut CurrentCommand<G> {
        self.0
    }
}

/// Proof of one exact input transition that delivered a current command.
///
/// Position identifies the cursor slot, while `sequence` distinguishes a
/// later delivery after that slot was rewound.  It is deliberately not a
/// provenance identity and is valid only within the processor episode that
/// minted it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryStamp {
    input_level: u64,
    position: u64,
    sequence: u64,
}

impl DeliveryStamp {
    /// Constructs the stamp for the input-level position consumed by this
    /// delivery. Only the canonical raw-delivery loop may mint stamps.
    #[allow(dead_code)] // minted by the ordered canonical raw-delivery implementation
    pub(crate) const fn new(input_level: u64, position: u64, sequence: u64) -> Self {
        Self {
            input_level,
            position,
            sequence,
        }
    }

    /// Returns the stable identity of the level that delivered the token.
    #[must_use]
    pub const fn input_level(&self) -> u64 {
        self.input_level
    }

    /// Returns the exact pre-retirement cursor position within that level.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Returns the unique sequence within the live processor episode.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[cfg(test)]
mod tests;

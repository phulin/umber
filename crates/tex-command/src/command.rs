//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::DefinitionRef;
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{
    CommandOperandWord, ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning,
    StaticCommandClass, UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, PackedCommandTarget, Token, TokenWord, TracedTokenWord};

use crate::SourceProvenance;

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
    pub(crate) slot_initializations: u64,
    pub(crate) resolved_writes: u64,
    pub(crate) delivery_stamp_writes: u64,
    pub(crate) rich_materializations: u64,
}

#[cfg(any(test, feature = "profiling"))]
thread_local! {
    static COMMAND_OWNERSHIP_COUNTERS: core::cell::Cell<CommandOwnershipCounters> =
        const { core::cell::Cell::new(CommandOwnershipCounters {
            clones: 0,
            backup_copies: 0,
            expansion_moves_in: 0,
            expansion_moves_out: 0,
            slot_initializations: 0,
            resolved_writes: 0,
            delivery_stamp_writes: 0,
            rich_materializations: 0,
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

/// The fixed-width `cur_chr` payload retained by [`CommandWord`].
/// Fixed-width command equivalent to TeX's `cur_cmd`/`cur_chr`.
///
/// Static meanings retain their validated packed word. In particular, dense
/// meaning resolution does not first decode a [`ResolvedMeaning`] and then
/// rematch it to discover the expansion branch.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct CommandWord<G> {
    code: CommandClass,
    flags: MeaningFlags,
    operand: CommandOperandWord<G>,
}

impl<G> Clone for CommandWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<G> Copy for CommandWord<G> {}

impl<G> CommandWord<G> {
    fn from_static_word(word: u64) -> Self {
        let code = match Meaning::runtime_word_class(word) {
            StaticCommandClass::Undefined => CommandClass::Undefined,
            StaticCommandClass::Relax => CommandClass::Relax,
            StaticCommandClass::Character => CommandClass::Character,
            StaticCommandClass::Expandable => CommandClass::Expandable,
            StaticCommandClass::EndV => CommandClass::EndV,
            StaticCommandClass::Unexpandable => CommandClass::Unexpandable,
            StaticCommandClass::Value => CommandClass::Value,
        };
        Self {
            code,
            flags: MeaningFlags::EMPTY,
            operand: CommandOperandWord::scalar(word),
        }
    }

    fn from_meaning(meaning: ResolvedMeaning<G>) -> (Self, Option<FontId>) {
        match meaning {
            ResolvedMeaning::Static(Meaning::Font(font)) => (
                Self {
                    code: CommandClass::Font,
                    flags: MeaningFlags::EMPTY,
                    operand: CommandOperandWord::scalar(0),
                },
                Some(font),
            ),
            ResolvedMeaning::Static(meaning) => (Self::from_static_word(meaning.encode()), None),
            ResolvedMeaning::Macro { flags, definition } => (
                Self {
                    code: CommandClass::Macro,
                    flags,
                    operand: CommandOperandWord::definition(definition),
                },
                None,
            ),
        }
    }

    pub(crate) const fn class(self) -> CommandClass {
        self.code
    }

    pub(crate) const fn flags(self) -> MeaningFlags {
        self.flags
    }

    pub(crate) const fn expandable_primitive(self) -> Option<ExpandablePrimitive> {
        if !matches!(self.code, CommandClass::Expandable) {
            return None;
        }
        let word = self.operand.scalar_value();
        ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word))
    }

    pub(crate) fn character_catcode(self) -> Option<Catcode> {
        if !matches!(self.code, CommandClass::Character) {
            return None;
        }
        match Meaning::from_runtime_word(self.operand.scalar_value()) {
            Meaning::CharToken { cat, .. } => Some(cat),
            _ => None,
        }
    }

    /// Returns the character value carried by either TeX character command.
    ///
    /// TeX82 §26 accepts both a literal character token and a `\chardef`
    /// (`char_given`) wherever a numeric scanner reads an internal integer.
    /// Keep that value projection separate from [`Self::character_token`],
    /// whose callers need to distinguish a literal token from a character
    /// value defined by a control sequence.
    pub(crate) fn character_value(self) -> Option<char> {
        if !matches!(self.code, CommandClass::Character) {
            return None;
        }
        match Meaning::from_runtime_word(self.operand.scalar_value()) {
            Meaning::CharGiven(ch) | Meaning::CharToken { ch, .. } => Some(ch),
            _ => None,
        }
    }

    /// Returns only a literal character token, never a `\chardef` value.
    pub(crate) fn character_token(self) -> Option<char> {
        if !matches!(self.code, CommandClass::Character) {
            return None;
        }
        match Meaning::from_runtime_word(self.operand.scalar_value()) {
            Meaning::CharToken { ch, .. } => Some(ch),
            _ => None,
        }
    }

    /// Whether TeX82 section 1038 can continue its main-loop character run.
    pub(crate) fn is_main_loop_character(self) -> bool {
        if !matches!(self.code, CommandClass::Character) {
            return false;
        }
        matches!(
            Meaning::from_runtime_word(self.operand.scalar_value()),
            Meaning::CharGiven(_)
                | Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                }
        )
    }

    pub(crate) const fn unexpandable_primitive(self) -> Option<UnexpandablePrimitive> {
        if !matches!(self.code, CommandClass::Unexpandable) {
            return None;
        }
        UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(
            self.operand.scalar_value(),
        ))
    }

    pub(crate) fn resolved_meaning(&self, font: Option<FontId>) -> ResolvedMeaning<G> {
        match self.code {
            CommandClass::Macro => ResolvedMeaning::Macro {
                flags: self.flags,
                definition: self.operand.definition_value(),
            },
            CommandClass::Font => ResolvedMeaning::Static(Meaning::Font(
                font.expect("font command retains its opaque admitted identity"),
            )),
            _ => ResolvedMeaning::Static(Meaning::from_runtime_word(self.operand.scalar_value())),
        }
    }

    /// Decodes a static command word without constructing a rich delivery.
    ///
    /// Macro definitions and font identities are the only command meanings
    /// whose operands are opaque capabilities rather than packed static
    /// words. Scanners that only classify a terminal command therefore use
    /// this projection instead of materializing `CurrentCommand`.
    pub(crate) fn static_meaning(self) -> Option<Meaning> {
        match self.code {
            CommandClass::Macro | CommandClass::Font => None,
            _ => Some(Meaning::from_runtime_word(self.operand.scalar_value())),
        }
    }
}

const _: () = assert!(core::mem::size_of::<CommandWord<()>>() == 16);

/// Compact coordinates and one-delivery policy attached to a hot token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeliverySite {
    delivery: DeliveryStamp,
    control_sequence: Option<Symbol>,
    active_source_role: Option<crate::SourceRole>,
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

/// Sole compact command owner inside raw and expanded delivery loops.
#[derive(Debug)]
pub(crate) struct HotCommand<G> {
    token: HotToken,
    command: CommandWord<G>,
    /// Font identities carry a runtime namespace and generation that cannot
    /// be reconstructed from their dense slot. Keep that opaque capability
    /// beside the fixed-width command word only for the font command class.
    font: Option<FontId>,
}

impl<G> PartialEq for HotCommand<G> {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token
            && self.command.class() == other.command.class()
            && self.command.flags() == other.command.flags()
            && self.command.operand.scalar_value() == other.command.operand.scalar_value()
            && self.font == other.font
    }
}

impl<G> Eq for HotCommand<G> {}

impl<G> core::hash::Hash for HotCommand<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.token.hash(state);
        self.command.class().hash(state);
        self.command.flags().hash(state);
        self.command.operand.scalar_value().hash(state);
        self.font.hash(state);
    }
}

// `HotCommand` contains only compact coordinates and the manually
// copyable `CommandWord`; its generation parameter brands opaque definition
// capabilities but is not itself a runtime field. Keep the copy contract
// independent of whether the generation marker happens to implement `Copy`.
impl<G> Copy for HotCommand<G> {}

impl<G> Clone for HotCommand<G> {
    fn clone(&self) -> Self {
        *self
    }
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
    /// Compact policy projection of the external source active at delivery.
    /// Its identity and physical spelling range remain recoverable from the
    /// token origin or live input coordinate and are not copied here.
    active_source_role: Option<crate::SourceRole>,
    direct_source_line: u32,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
    delivery_flags: CommandDeliveryFlags,
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

impl<G> Clone for CurrentCommand<G> {
    fn clone(&self) -> Self {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.clones = counters.clones.saturating_add(1);
        });
        Self {
            spelling: self.spelling,
            meaning: self.meaning,
            identity: self.identity,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            active_source_role: self.active_source_role,
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
            && self.active_source_role == other.active_source_role
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
        self.active_source_role.hash(state);
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
    fn from_static_meaning(meaning: Meaning) -> Self {
        match meaning {
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::ExpandAfter) => {
                Self::ExpandAfter
            }
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::CsName) => {
                Self::CsName
            }
            Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndCsName) => {
                Self::EndCsName
            }
            Meaning::ExpandablePrimitive(primitive) => {
                if let Some(selector) = ConvertSelector::from_primitive(primitive) {
                    Self::Convert(selector)
                } else {
                    Self::Ordinary
                }
            }
            Meaning::UnexpandablePrimitive(primitive) => {
                if let Some(selector) = XRaySelector::from_primitive(primitive) {
                    Self::XRay(selector)
                } else {
                    Self::Ordinary
                }
            }
            _ => Self::Ordinary,
        }
    }
}

impl<G> PackedCommandTarget<G> for HotCommand<G> {
    #[inline(always)]
    fn write_control_sequence(&mut self, control_sequence: Option<Symbol>) {
        self.token.site.control_sequence = control_sequence;
    }

    #[inline(always)]
    fn write_static_meaning_word(&mut self, word: u64) {
        self.command = CommandWord::from_static_word(word);
        self.font = None;
    }

    #[inline(always)]
    fn write_font_meaning(&mut self, font: FontId) {
        self.command = CommandWord {
            code: CommandClass::Font,
            flags: MeaningFlags::EMPTY,
            operand: CommandOperandWord::scalar(0),
        };
        self.font = Some(font);
    }

    #[inline(always)]
    fn write_macro_meaning(&mut self, flags: MeaningFlags, definition: DefinitionRef<G>) {
        self.command = CommandWord {
            code: CommandClass::Macro,
            flags,
            operand: CommandOperandWord::definition(definition),
        };
        self.font = None;
    }
}

impl<G> HotCommand<G> {
    /// Resolves a resident word into a newly-owned hot command without first
    /// constructing an empty destination. The command word is initialized to
    /// a valid value before the packed resolver overwrites it; unlike
    /// [`Self::empty`], no placeholder token or delivery stamp is installed.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn from_resolved_delivery(
        word: TokenWord,
        origin: OriginId,
        input_level: u64,
        position: u64,
        active_source: Option<tex_state::packed_input::SourceContext>,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        state: &CommandContext<'_, G>,
    ) -> (Self, tex_state::token::PackedMeaningResolution) {
        #[cfg(any(test, feature = "profiling"))]
        update_command_ownership_counters(|counters| {
            counters.resolved_writes = counters.resolved_writes.saturating_add(1);
            counters.delivery_stamp_writes = counters.delivery_stamp_writes.saturating_add(1);
        });
        let mut command = Self {
            token: HotToken {
                word,
                origin,
                site: DeliverySite {
                    delivery: DeliveryStamp::new(input_level, position),
                    control_sequence: None,
                    active_source_role: active_source.map(|source| source.role()),
                    direct_source_line: direct_source_line.unwrap_or(0),
                    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                    delivery_flags: CommandDeliveryFlags::delivery(
                        direct_source,
                        direct_source_line.is_some(),
                        suppress_expandable,
                    ),
                },
            },
            command: CommandWord::from_static_word(Meaning::Undefined.encode()),
            font: None,
        };
        let resolution = state.write_packed_token_command_into(word, &mut command);
        (command, resolution)
    }

    #[inline(always)]
    pub(crate) fn empty() -> Self {
        Self {
            token: HotToken {
                word: TokenWord::pack(Token::Char {
                    ch: '\0',
                    cat: Catcode::Ignored,
                }),
                origin: OriginId::UNKNOWN,
                site: DeliverySite {
                    delivery: DeliveryStamp::new(0, 0),
                    control_sequence: None,
                    active_source_role: None,
                    direct_source_line: 0,
                    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                    delivery_flags: CommandDeliveryFlags::default(),
                },
            },
            command: CommandWord::from_static_word(Meaning::Undefined.encode()),
            font: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    pub(crate) fn write_resolved_delivery(
        &mut self,
        word: TokenWord,
        origin: OriginId,
        input_level: u64,
        position: u64,
        active_source: Option<tex_state::packed_input::SourceContext>,
        direct_source: bool,
        direct_source_line: Option<u32>,
        suppress_expandable: bool,
        state: &CommandContext<'_, G>,
    ) -> tex_state::token::PackedMeaningResolution {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.resolved_writes = counters.resolved_writes.saturating_add(1);
            counters.delivery_stamp_writes = counters.delivery_stamp_writes.saturating_add(1);
        });
        self.token = HotToken {
            word,
            origin,
            site: DeliverySite {
                delivery: DeliveryStamp::new(input_level, position),
                control_sequence: None,
                active_source_role: active_source.map(|source| source.role()),
                direct_source_line: direct_source_line.unwrap_or(0),
                alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
                delivery_flags: CommandDeliveryFlags::delivery(
                    direct_source,
                    direct_source_line.is_some(),
                    suppress_expandable,
                ),
            },
        };
        state.write_packed_token_command_into(word, self)
    }

    #[inline(always)]
    pub(crate) const fn command_word(&self) -> CommandWord<G> {
        self.command
    }

    /// The command identity selected by the compact delivery loop.
    pub(crate) fn identity(&self) -> CommandIdentity {
        if self
            .token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
        {
            CommandIdentity::NoExpandFrozenRelax
        } else {
            match self.command.static_meaning() {
                Some(meaning) => CommandIdentity::from_static_meaning(meaning),
                None => CommandIdentity::Ordinary,
            }
        }
    }

    #[allow(dead_code)] // profiling-only direct-delivery harness
    pub(crate) fn resolved_meaning(&self) -> ResolvedMeaning<G> {
        self.command.resolved_meaning(self.font)
    }

    /// Returns the opaque font identity carried by a compact font command.
    /// Font commands are the one static command class whose operand is not
    /// reconstructible from the packed runtime word.
    pub(crate) const fn font_id(&self) -> Option<FontId> {
        self.font
    }

    pub(crate) const fn spelling_word(&self) -> TokenWord {
        self.token.word
    }

    pub(crate) const fn spelling(&self) -> TracedTokenWord {
        TracedTokenWord::from_parts(self.token.word, self.token.origin)
    }

    pub(crate) const fn origin(&self) -> OriginId {
        self.token.origin
    }

    /// Returns a literal character token represented by a compact character
    /// command. The expanded `\csname` and `\ifcsname` drivers use this
    /// without materializing a rich `CurrentCommand` on every collected
    /// character. A `\chardef` value deliberately returns `None`, allowing
    /// the caller to enter TeX82 §25's missing-`\endcsname` recovery.
    pub(crate) fn character_token(&self) -> Option<char> {
        self.command.character_token()
    }

    /// Returns the character value carried by either a literal token or a
    /// `\chardef` command. Numeric, dimension, numeric-conditional, and PDF
    /// hot scanners use this TeX82 §26 projection; they must not use the
    /// literal-token-only `character_token` projection above.
    pub(crate) fn character_value(&self) -> Option<char> {
        self.command.character_value()
    }

    /// Returns the category attached to a compact character command.  Scalar
    /// and conditional hot lanes use this projection without materializing a
    /// `CurrentCommand` for each digit or relation token.
    pub(crate) fn character_catcode(&self) -> Option<Catcode> {
        self.command.character_catcode()
    }

    /// Returns the active character recovered by TeX82 §506 after `\noexpand`
    /// has replaced its effective command with frozen `\relax`.
    pub(crate) fn no_expand_active_character(&self) -> Option<char> {
        if !self
            .token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
        {
            return None;
        }
        match self.token.word.semantic_token() {
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => Some(ch),
            _ => None,
        }
    }

    /// TeX82 part 28's character-code projection for an `\if` operand.
    pub(crate) fn conditional_character_code(self) -> u32 {
        if let Some(ch) = self.no_expand_active_character()
            && (ch as u32) <= u32::from(u8::MAX)
        {
            return ch as u32;
        }
        match Meaning::from_runtime_word(self.command.operand.scalar_value()) {
            Meaning::CharToken { ch, .. } if (ch as u32) <= u32::from(u8::MAX) => ch as u32,
            _ => 256,
        }
    }

    /// TeX82 part 28's category-code projection for an `\ifcat` operand.
    pub(crate) fn conditional_category_code(self) -> Option<Catcode> {
        if self.no_expand_active_character().is_some() {
            return Some(Catcode::Active);
        }
        match Meaning::from_runtime_word(self.command.operand.scalar_value()) {
            Meaning::CharToken { cat, .. } => Some(cat),
            _ => None,
        }
    }

    pub(crate) const fn control_sequence(&self) -> Option<Symbol> {
        self.token.site.control_sequence
    }

    pub(crate) const fn delivery_stamp(&self) -> DeliveryStamp {
        self.token.site.delivery
    }

    pub(crate) const fn alignment_adjustment(
        &self,
    ) -> crate::processor::AlignmentDeliveryAdjustment {
        self.token.site.alignment_adjustment
    }

    pub(crate) fn set_alignment_adjustment(
        &mut self,
        adjustment: crate::processor::AlignmentDeliveryAdjustment,
    ) {
        self.token.site.alignment_adjustment = adjustment;
    }

    pub(crate) const fn is_direct_source_delivery(&self) -> bool {
        self.token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
    }

    pub(crate) const fn suppresses_expandable_control_sequence(&self) -> bool {
        self.token
            .site
            .delivery_flags
            .contains(CommandDeliveryFlags::SUPPRESS_EXPANDABLE)
    }

    pub(crate) fn suppress_expandable(&mut self) {
        if self.command.expandable_primitive() != Some(ExpandablePrimitive::EndCsName)
            && matches!(
                self.command.class(),
                CommandClass::Undefined | CommandClass::Macro | CommandClass::Expandable
            )
        {
            self.command = CommandWord::from_static_word(Meaning::Relax.encode());
            self.font = None;
            self.token
                .site
                .delivery_flags
                .set(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX, true);
        }
    }

    pub(crate) const fn is_outer(&self) -> bool {
        (matches!(self.command.class(), CommandClass::Macro)
            && self.command.flags().contains(MeaningFlags::OUTER))
            || matches!(
                self.command.expandable_primitive(),
                Some(ExpandablePrimitive::EndTemplate)
            )
    }

    pub(crate) fn macro_parts(&self) -> Option<(MeaningFlags, DefinitionRef<G>)> {
        matches!(self.command.class(), CommandClass::Macro).then(|| {
            (
                self.command.flags(),
                self.command.operand.definition_value(),
            )
        })
    }

    pub(crate) fn convert_end_template_to_endv(&mut self, frozen_endv: Token) {
        self.token.word = TokenWord::pack(frozen_endv);
        self.command = CommandWord::from_static_word(Meaning::EndV.encode());
        self.font = None;
        self.token.site.control_sequence = None;
        self.token.site.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
    }

    pub(crate) fn convert_to_end_template(&mut self) {
        self.command = CommandWord::from_static_word(
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate).encode(),
        );
        self.font = None;
        self.token.site.control_sequence = None;
    }

    pub(crate) fn materialize(&self) -> CurrentCommand<G> {
        #[cfg(any(test, feature = "profiling"))]
        update_command_ownership_counters(|counters| {
            counters.rich_materializations = counters.rich_materializations.saturating_add(1);
        });
        let meaning = self.command.resolved_meaning(self.font);
        CurrentCommand {
            spelling: TracedTokenWord::from_parts(self.token.word, self.token.origin),
            identity: if self
                .token
                .site
                .delivery_flags
                .contains(CommandDeliveryFlags::NOEXPAND_FROZEN_RELAX)
            {
                CommandIdentity::NoExpandFrozenRelax
            } else {
                match meaning {
                    ResolvedMeaning::Static(meaning) => {
                        CommandIdentity::from_static_meaning(meaning)
                    }
                    ResolvedMeaning::Macro { .. } => CommandIdentity::Ordinary,
                }
            },
            meaning,
            control_sequence: self.token.site.control_sequence,
            delivery: self.token.site.delivery,
            active_source_role: self.token.site.active_source_role,
            direct_source_line: self.token.site.direct_source_line,
            alignment_adjustment: self.token.site.alignment_adjustment,
            delivery_flags: self.token.site.delivery_flags,
        }
    }

    pub(crate) fn from_current(command: CurrentCommand<G>) -> Self {
        let (command_word, font) = CommandWord::from_meaning(command.meaning);
        Self {
            token: HotToken {
                word: command.spelling.token_word(),
                origin: command.spelling.origin(),
                site: DeliverySite {
                    delivery: command.delivery,
                    control_sequence: command.control_sequence,
                    active_source_role: command.active_source_role,
                    direct_source_line: command.direct_source_line,
                    alignment_adjustment: command.alignment_adjustment,
                    delivery_flags: command.delivery_flags,
                },
            },
            command: command_word,
            font,
        }
    }

    pub(crate) fn from_current_ref(command: &CurrentCommand<G>) -> Self {
        let (command_word, font) = CommandWord::from_meaning(command.meaning);
        Self {
            token: HotToken {
                word: command.spelling.token_word(),
                origin: command.spelling.origin(),
                site: DeliverySite {
                    delivery: command.delivery,
                    control_sequence: command.control_sequence,
                    active_source_role: command.active_source_role,
                    direct_source_line: command.direct_source_line,
                    alignment_adjustment: command.alignment_adjustment,
                    delivery_flags: command.delivery_flags,
                },
            },
            command: command_word,
            font,
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
        let mut command = HotCommand::empty();
        let active_source = source_provenance.map(|provenance| {
            tex_state::packed_input::SourceContext::new(
                provenance.range().source(),
                crate::SourceRole::GeneratedInput,
            )
        });
        let _ = command.write_resolved_delivery(
            spelling.token_word(),
            spelling.origin(),
            delivery.input_level,
            delivery.position,
            active_source,
            direct_source,
            direct_source_line,
            false,
            state,
        );
        command.materialize()
    }

    /// Creates the reusable destination value owned by one delivery request.
    ///
    /// Raw input overwrites this value before any semantic consumer can
    /// observe it. Keeping an initialized unresolved meaning avoids unsafe
    /// partial initialization while adding no second command representation.
    #[inline(always)]
    pub(crate) fn empty() -> Self {
        #[cfg(test)]
        update_command_ownership_counters(|counters| {
            counters.slot_initializations = counters.slot_initializations.saturating_add(1);
        });
        Self {
            spelling: TracedTokenWord::INITIALIZED_PLACEHOLDER,
            meaning: ResolvedMeaning::Static(Meaning::Undefined),
            identity: CommandIdentity::Ordinary,
            control_sequence: None,
            delivery: DeliveryStamp::new(0, 0),
            active_source_role: None,
            direct_source_line: 0,
            alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
            delivery_flags: CommandDeliveryFlags::default(),
        }
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
        self.delivery_flags
            .set(CommandDeliveryFlags::DIRECT_SOURCE, false);
        self.delivery_flags
            .set(CommandDeliveryFlags::OUTER_RECOVERY_SPACE, true);
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
        self.meaning
    }

    /// Borrows the effective meaning without creating a transient alias.
    #[must_use]
    pub const fn meaning_ref(&self) -> &ResolvedMeaning<G> {
        &self.meaning
    }

    /// Consumes this delivered command and returns its already-resolved
    /// meaning without acquiring another immutable-definition owner.
    #[must_use]
    pub(crate) fn into_meaning(self) -> ResolvedMeaning<G> {
        self.meaning
    }

    /// Returns the control-sequence identity, if this spelling resolves via
    /// a control-sequence meaning cell.
    #[must_use]
    pub const fn control_sequence(&self) -> Option<Symbol> {
        self.control_sequence
    }

    #[cfg(test)]
    pub(crate) fn clear_control_sequence_for_test(&mut self) {
        self.control_sequence = None;
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

    /// Host/VFS role of the external source active at this delivery.
    #[must_use]
    pub const fn active_source_role(&self) -> Option<crate::SourceRole> {
        self.active_source_role
    }

    /// Returns the physical range only when this delivery came directly from
    /// a source level. Replayed tokens retain their range for diagnostics but
    /// must not masquerade as a second physical-source transition.
    pub(crate) const fn is_direct_source_delivery(&self) -> bool {
        self.delivery_flags
            .contains(CommandDeliveryFlags::DIRECT_SOURCE)
    }

    /// Physical line captured while this exact command was delivered from a
    /// source cursor. Replayed macro and backup commands deliberately carry
    /// no direct line even when their diagnostic provenance names a source.
    #[must_use]
    pub const fn direct_source_line_number(&self) -> Option<u32> {
        if self
            .delivery_flags
            .contains(CommandDeliveryFlags::HAS_DIRECT_SOURCE_LINE)
        {
            Some(self.direct_source_line)
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
            meaning: self.meaning,
            identity: self.identity,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            active_source_role: self.active_source_role,
            direct_source_line: self.direct_source_line,
            alignment_adjustment: self.alignment_adjustment,
            delivery_flags: self.delivery_flags,
        }
    }
}

/// Proof of one exact input transition that delivered a current command.
///
/// Position identifies the pre-advance cursor slot. It is deliberately not a
/// provenance identity; the processor-local delivery authority determines
/// whether this coordinate is currently admitted for backup.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryStamp {
    input_level: u64,
    position: u64,
}

impl DeliveryStamp {
    /// Constructs the stamp for the input-level position consumed by this
    /// delivery. Only the canonical raw-delivery loop may mint stamps.
    #[allow(dead_code)] // minted by the ordered canonical raw-delivery implementation
    pub(crate) const fn new(input_level: u64, position: u64) -> Self {
        Self {
            input_level,
            position,
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
}

#[cfg(test)]
mod tests;

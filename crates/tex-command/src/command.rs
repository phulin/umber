//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::DefinitionRef;
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{
    ExpandablePrimitive, Meaning, MeaningFlags, ResolvedMeaning, StaticCommandClass,
    UnexpandablePrimitive,
};
use tex_state::token::{Catcode, OriginId, PackedCommandTarget, Token, TokenWord, TracedTokenWord};

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

/// Native final meaning equivalent to TeX's `cur_cmd`/`cur_chr`.
///
/// Static meanings retain their validated packed word. In particular, dense
/// meaning resolution does not first decode a [`ResolvedMeaning`] and then
/// rematch it to discover the expansion branch.
#[derive(Debug)]
pub(crate) enum CommandWord<G> {
    Static {
        class: CommandClass,
        word: u64,
    },
    Macro {
        flags: MeaningFlags,
        definition: DefinitionRef<G>,
    },
    Font(FontId),
}

impl<G> Clone for CommandWord<G> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<G> Copy for CommandWord<G> {}

impl<G> PartialEq for CommandWord<G> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Static { class: a, word: x }, Self::Static { class: b, word: y }) => {
                a == b && x == y
            }
            (
                Self::Macro {
                    flags: a,
                    definition: x,
                },
                Self::Macro {
                    flags: b,
                    definition: y,
                },
            ) => a == b && x == y,
            (Self::Font(a), Self::Font(b)) => a == b,
            _ => false,
        }
    }
}
impl<G> Eq for CommandWord<G> {}
impl<G> core::hash::Hash for CommandWord<G> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.class().hash(state);
        match self {
            Self::Static { word, .. } => word.hash(state),
            Self::Macro { flags, definition } => {
                flags.hash(state);
                definition.hash(state);
            }
            Self::Font(font) => font.hash(state),
        }
    }
}

impl<G> CommandWord<G> {
    pub(crate) fn from_static_word(word: u64) -> Self {
        let class = match Meaning::runtime_word_class(word) {
            StaticCommandClass::Undefined => CommandClass::Undefined,
            StaticCommandClass::Relax => CommandClass::Relax,
            StaticCommandClass::Character => CommandClass::Character,
            StaticCommandClass::Expandable => CommandClass::Expandable,
            StaticCommandClass::EndV => CommandClass::EndV,
            StaticCommandClass::Unexpandable => CommandClass::Unexpandable,
            StaticCommandClass::Value => CommandClass::Value,
        };
        Self::Static { class, word }
    }

    #[cfg(test)]
    fn from_meaning(meaning: ResolvedMeaning<G>) -> Self {
        match meaning {
            ResolvedMeaning::Static(Meaning::Font(font)) => Self::Font(font),
            ResolvedMeaning::Static(meaning) => Self::from_static_word(meaning.encode()),
            ResolvedMeaning::Macro { flags, definition } => Self::Macro { flags, definition },
        }
    }

    pub(crate) const fn class(self) -> CommandClass {
        match self {
            Self::Static { class, .. } => class,
            Self::Macro { .. } => CommandClass::Macro,
            Self::Font(_) => CommandClass::Font,
        }
    }

    pub(crate) const fn flags(self) -> MeaningFlags {
        match self {
            Self::Macro { flags, .. } => flags,
            _ => MeaningFlags::EMPTY,
        }
    }

    pub(crate) const fn expandable_primitive(self) -> Option<ExpandablePrimitive> {
        match self {
            Self::Static {
                class: CommandClass::Expandable,
                word,
            } => ExpandablePrimitive::from_operand(Meaning::runtime_word_operand(word)),
            _ => None,
        }
    }

    pub(crate) fn character_catcode(self) -> Option<Catcode> {
        match self.static_meaning() {
            Some(Meaning::CharToken { cat, .. }) => Some(cat),
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
    #[cfg(test)]
    pub(crate) fn character_value(self) -> Option<char> {
        if !matches!(self.class(), CommandClass::Character) {
            return None;
        }
        match Meaning::from_runtime_word(self.static_word()) {
            Meaning::CharGiven(ch) | Meaning::CharToken { ch, .. } => Some(ch),
            _ => None,
        }
    }

    /// Returns only a literal character token, never a `\chardef` value.
    pub(crate) fn character_token(self) -> Option<char> {
        if !matches!(self.class(), CommandClass::Character) {
            return None;
        }
        match Meaning::from_runtime_word(self.static_word()) {
            Meaning::CharToken { ch, .. } => Some(ch),
            _ => None,
        }
    }

    /// Whether TeX82 section 1038 can continue its main-loop character run.
    pub(crate) fn is_main_loop_character(self) -> bool {
        if !matches!(self.class(), CommandClass::Character) {
            return false;
        }
        matches!(
            Meaning::from_runtime_word(self.static_word()),
            Meaning::CharGiven(_)
                | Meaning::CharToken {
                    cat: Catcode::Letter | Catcode::Other,
                    ..
                }
        )
    }

    pub(crate) const fn unexpandable_primitive(self) -> Option<UnexpandablePrimitive> {
        if !matches!(self.class(), CommandClass::Unexpandable) {
            return None;
        }
        UnexpandablePrimitive::from_operand(Meaning::runtime_word_operand(self.static_word()))
    }

    pub(crate) fn resolved_meaning(&self) -> ResolvedMeaning<G> {
        match *self {
            Self::Macro { flags, definition } => ResolvedMeaning::Macro { flags, definition },
            Self::Font(font) => ResolvedMeaning::Static(Meaning::Font(font)),
            Self::Static { word, .. } => ResolvedMeaning::Static(Meaning::from_runtime_word(word)),
        }
    }

    pub(crate) const fn macro_definition(self) -> Option<(MeaningFlags, DefinitionRef<G>)> {
        match self {
            Self::Macro { flags, definition } => Some((flags, definition)),
            _ => None,
        }
    }

    pub(crate) fn is_outer(self) -> bool {
        self.flags().contains(MeaningFlags::OUTER)
            || self.expandable_primitive() == Some(ExpandablePrimitive::EndTemplate)
    }

    const fn static_word(self) -> u64 {
        match self {
            Self::Static { word, .. } => word,
            _ => 0,
        }
    }

    /// Decodes a static command word without constructing a rich delivery.
    ///
    /// Macro definitions and font identities are the only command meanings
    /// whose operands are opaque capabilities rather than packed static
    /// words. Scanners that only classify a terminal command therefore use
    /// this projection instead of materializing `CurrentCommand`.
    pub(crate) fn static_meaning(self) -> Option<Meaning> {
        match self.class() {
            CommandClass::Macro | CommandClass::Font => None,
            _ => Some(Meaning::from_runtime_word(self.static_word())),
        }
    }
}

const _: () = assert!(core::mem::size_of::<CommandWord<()>>() == 24);

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
    command: CommandWord<G>,
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
        self.command.hash(state);
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
    fn from_command_word<G>(command: CommandWord<G>) -> Self {
        if let Some(primitive) = command.expandable_primitive() {
            return match primitive {
                ExpandablePrimitive::ExpandAfter => Self::ExpandAfter,
                ExpandablePrimitive::CsName => Self::CsName,
                ExpandablePrimitive::EndCsName => Self::EndCsName,
                other => {
                    ConvertSelector::from_primitive(other).map_or(Self::Ordinary, Self::Convert)
                }
            };
        }
        command
            .unexpandable_primitive()
            .and_then(XRaySelector::from_primitive)
            .map_or(Self::Ordinary, Self::XRay)
    }
}

impl<G> PackedCommandTarget<G> for CommandWord<G> {
    #[inline(always)]
    fn write_static_meaning_word(&mut self, word: u64) {
        *self = Self::from_static_word(word);
    }
    #[inline(always)]
    fn write_font_meaning(&mut self, font: FontId) {
        *self = Self::Font(font);
    }
    #[inline(always)]
    fn write_macro_meaning(&mut self, flags: MeaningFlags, definition: DefinitionRef<G>) {
        *self = Self::Macro { flags, definition };
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
        let mut meaning = CommandWord::from_static_word(Meaning::UNDEFINED_WORD);
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
        meaning: CommandWord<G>,
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
            command: CommandWord::from_static_word(Meaning::UNDEFINED_WORD),
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

    #[inline(always)]
    pub(crate) const fn command_word(&self) -> CommandWord<G> {
        self.command
    }

    /// Classifies a delivered static command without expanding its rich
    /// recovery and diagnostic representation.
    #[inline(always)]
    pub(crate) fn static_meaning(&self) -> Option<Meaning> {
        self.command.static_meaning()
    }

    pub(crate) const fn spelling_word(&self) -> TokenWord {
        self.token.word
    }

    /// Whether the settled effective meaning is TeX's paragraph command.
    /// This is kept on the compact command so the matcher does not project a
    /// `ResolvedMeaning` merely to identify `\par`.
    #[inline(always)]
    pub(crate) fn is_effective_paragraph(&self) -> bool {
        self.command.unexpandable_primitive() == Some(UnexpandablePrimitive::Par)
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

    /// Returns a literal character token represented by a compact character
    /// command. The expanded `\csname` and `\ifcsname` drivers use this
    /// without projecting a decoded meaning on every collected
    /// character. A `\chardef` value deliberately returns `None`, allowing
    /// the caller to enter TeX82 §25's missing-`\endcsname` recovery.
    pub(crate) fn character_token(&self) -> Option<char> {
        self.command.character_token()
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
        if self.command.expandable_primitive() != Some(ExpandablePrimitive::EndCsName)
            && matches!(
                self.command.class(),
                CommandClass::Undefined | CommandClass::Macro | CommandClass::Expandable
            )
        {
            self.command = CommandWord::from_static_word(Meaning::RELAX_WORD);
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
                self.command
                    .macro_definition()
                    .expect("macro command definition")
                    .1,
            )
        })
    }

    pub(crate) fn convert_end_template_to_endv(&mut self, frozen_endv: Token) {
        self.token.word = TokenWord::pack(frozen_endv);
        self.command = CommandWord::from_static_word(Meaning::END_V_WORD);
        self.token.site.control_sequence = None;
        self.token.site.alignment_adjustment = crate::processor::AlignmentDeliveryAdjustment::None;
    }

    pub(crate) fn convert_to_end_template(&mut self) {
        self.command = CommandWord::from_static_word(Meaning::END_TEMPLATE_WORD);
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
            CommandIdentity::from_command_word(self.command)
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
        self.command = CommandWord::from_static_word(Meaning::char_token_word(' ', Catcode::Space));
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
        self.command.resolved_meaning()
    }

    /// Borrows the delivered packed static word for direct main-control dispatch.
    /// Macro and font capabilities remain in their actual typed command fields.
    #[must_use]
    pub const fn static_word(&self) -> Option<u64> {
        match self.command {
            CommandWord::Static { word, .. } => Some(word),
            _ => None,
        }
    }

    /// Native packed command family and modifier for mode/command dispatch.
    #[must_use]
    #[inline]
    pub fn control_code(&self) -> Option<u64> {
        match self.command {
            CommandWord::Static { word, .. } => Some(Meaning::runtime_word_control_code(word)),
            CommandWord::Font(font) => Some(Meaning::runtime_word_control_code(
                Meaning::Font(font).encode(),
            )),
            CommandWord::Macro { .. } => None,
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

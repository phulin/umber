//! Ephemeral current-command representation.

use tex_state::CommandContext;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TracedTokenWord};

use crate::{SourceLocation, SourceProvenance, SourceRange};

/// One command delivery, equivalent to TeX's `cur_cmd`, `cur_chr`, `cur_cs`,
/// and `cur_tok`.
///
/// This value is normally call-local and remains absent at durable named
/// checkpoints. A resource-suspended expanded scanner may retain exactly one
/// current command as its typed continuation; its delivery stamp identifies
/// that exact live cursor transition and is never reconstructed from token
/// equality.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrentCommand<G> {
    spelling: TracedTokenWord,
    meaning: ResolvedMeaning<G>,
    macro_observation_operand: Option<i64>,
    identity: CommandIdentity,
    control_sequence: Option<Symbol>,
    delivery: DeliveryStamp,
    source_provenance: Option<SourceProvenance>,
    direct_source: bool,
    alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment,
    outer_recovery_space: bool,
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
    const fn from_meaning<G>(meaning: ResolvedMeaning<G>) -> Self {
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
                if let Some(selector) = ConvertSelector::from_primitive(primitive) {
                    Self::Convert(selector)
                } else {
                    Self::Ordinary
                }
            }
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(primitive)) => {
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
        state: &CommandContext<'_, G>,
    ) -> Self {
        let token = spelling.semantic_token();
        let (control_sequence, meaning) = match token {
            Token::Cs(symbol) => (
                Some(symbol),
                state
                    .meaning(symbol)
                    .unwrap_or(ResolvedMeaning::Static(Meaning::Undefined)),
            ),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = state.active_character_symbol(ch);
                (
                    symbol,
                    symbol
                        .and_then(|symbol| state.meaning(symbol).ok())
                        .unwrap_or(ResolvedMeaning::Static(Meaning::Undefined)),
                )
            }
            Token::Char { ch, cat } => (
                None,
                ResolvedMeaning::Static(Meaning::CharToken { ch, cat }),
            ),
            // `out_param` is converted to a literal replay token before
            // meaning resolution (TeX.web, get_next). A stray parameter token
            // is nevertheless represented deterministically while recovery
            // remains the responsibility of the raw delivery loop.
            Token::Param(_) => (None, ResolvedMeaning::Static(Meaning::Undefined)),
            // TeX82 §222 keeps `eq_type(undefined_control_sequence)` at
            // `undefined_cs` and `equiv` at `null` for the whole run: the
            // dummy location has no meaning cell an assignment could reach.
            Token::Frozen(_) if token.is_undefined_control_sequence() => {
                (None, ResolvedMeaning::Static(Meaning::Undefined))
            }
            Token::Frozen(_) if token.is_frozen_end_template() => (
                None,
                ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::EndTemplate,
                )),
            ),
            Token::Frozen(_) if token.is_frozen_endv() => {
                (None, ResolvedMeaning::Static(Meaning::EndV))
            }
            Token::Frozen(_) if token.is_frozen_relax() => {
                (None, ResolvedMeaning::Static(Meaning::Relax))
            }
            Token::Frozen(_) => (
                None,
                ResolvedMeaning::Static(
                    state
                        .frozen_primitive_meaning(token)
                        .unwrap_or(Meaning::Undefined),
                ),
            ),
        };
        let macro_observation_operand = None;
        Self {
            spelling,
            meaning,
            macro_observation_operand,
            identity: CommandIdentity::from_meaning(meaning),
            control_sequence,
            delivery,
            source_provenance,
            direct_source,
            alignment_adjustment: crate::processor::AlignmentDeliveryAdjustment::None,
            outer_recovery_space: false,
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
        self.macro_observation_operand = None;
        self.control_sequence = None;
        self.source_provenance = None;
        self.direct_source = false;
        self.outer_recovery_space = true;
    }

    /// Replaces an intercepted alignment terminator's effective meaning while
    /// preserving its spelling and delivery proof (TeX.web `get_next`).
    pub(crate) fn convert_to_end_template(&mut self) {
        self.meaning = ResolvedMeaning::Static(Meaning::ExpandablePrimitive(
            tex_state::meaning::ExpandablePrimitive::EndTemplate,
        ));
        self.macro_observation_operand = None;
        self.control_sequence = None;
    }

    /// Whether §23 replaced this delivery by its temporary recovery space.
    ///
    /// The space is TeX's effective current command after the forbidden
    /// outer token has been backed up; it is not an input token for an active
    /// `scan_toks` collector to append before the inserted right brace closes
    /// the runaway text.
    pub(crate) const fn is_outer_recovery_space(&self) -> bool {
        self.outer_recovery_space
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
        self.macro_observation_operand = None;
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
    pub const fn meaning(&self) -> ResolvedMeaning<G> {
        self.meaning
    }

    pub(crate) const fn macro_observation_operand(&self) -> Option<i64> {
        self.macro_observation_operand
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
        if self.direct_source {
            self.source_provenance
        } else {
            None
        }
    }

    /// Makes a fresh copy for the input backup path. `CurrentCommand` itself
    /// remains deliberately non-`Clone` at the public boundary.
    pub(crate) fn copy_for_backup(&self) -> Self {
        Self {
            spelling: self.spelling,
            meaning: self.meaning,
            macro_observation_operand: self.macro_observation_operand,
            identity: self.identity,
            control_sequence: self.control_sequence,
            delivery: self.delivery,
            source_provenance: self.source_provenance,
            direct_source: self.direct_source,
            alignment_adjustment: self.alignment_adjustment,
            outer_recovery_space: self.outer_recovery_space,
        }
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

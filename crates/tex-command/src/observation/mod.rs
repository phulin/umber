//! Test and instrumentation-only command semantic observation.
//!
//! These records deliberately belong to `tex-command`, rather than to the
//! fixture/oracle crate.  They are built from values already available at the
//! transition seam and are delivered only after the transition has committed.
//! In particular, an observer is non-fallible and never participates in
//! command state, snapshots, delivery, expansion, or scanner control flow.

#![cfg(any(test, feature = "instrumentation"))]

use tex_state::meaning::Meaning;

use crate::command::{CommandIdentity, CurrentCommand};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{DeliveryStamp, SourceLocation, SourceProvenance, SourceRange};

mod primitive_identity;
use primitive_identity::{expandable_primitive_identity, unexpandable_primitive_identity};

/// An owned, allocation-independent spelling used by command observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedToken {
    Character { character: char, catcode: Catcode },
    ControlSequence(String),
    MacroMatch,
    MacroEndMatch,
    Parameter(u8),
    FrozenEndTemplate,
    FrozenEndV,
    FrozenPrimitive(u16),
    FrozenOther,
}

/// A canonical diagnostic argument, which can be a token spelling or a
/// diagnostic-specific symbolic value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticArgument {
    Token(ObservedToken),
    Name(String),
}

/// Exact source and aggregate delivery provenance for an observed command.
///
/// The input-level identity and cursor slot identify the aggregate input
/// transition; the processor-local sequence distinguishes a later replay of
/// the same slot. The origin stays opaque, but is retained for host-side
/// source-map resolution while the aggregate timeline is live.
#[derive(Clone, Copy, Debug, Eq)]
pub struct CommandProvenance {
    pub input_level: u64,
    pub position: u64,
    pub delivery_sequence: u64,
    pub has_origin: bool,
    pub origin: OriginId,
    /// Exact physical range for direct source delivery. Replayed and expanded
    /// commands retain it through their traced spelling origin instead.
    pub source_range: Option<SourceRange>,
    /// Source column of the final byte the spelling consumed. This differs
    /// from the raw span start for decoded caret notation.
    pub source_location: Option<SourceLocation>,
}

impl PartialEq for CommandProvenance {
    fn eq(&self, other: &Self) -> bool {
        self.input_level == other.input_level
            && self.position == other.position
            && self.delivery_sequence == other.delivery_sequence
            && self.has_origin == other.has_origin
            && self.source_range == other.source_range
            && self.source_location == other.source_location
    }
}

impl CommandProvenance {
    pub(crate) fn from_stamp(
        stamp: DeliveryStamp,
        origin: OriginId,
        source_provenance: Option<SourceProvenance>,
    ) -> Self {
        Self {
            input_level: stamp.input_level(),
            position: stamp.position(),
            delivery_sequence: stamp.sequence(),
            has_origin: origin != OriginId::UNKNOWN,
            origin,
            source_range: source_provenance.map(SourceProvenance::range),
            source_location: source_provenance.map(SourceProvenance::location),
        }
    }
}

/// The caller-visible delivery boundary which committed a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandDeliveryBoundary {
    Raw,
    Expanded,
}

/// A command delivery expressed without engine allocation identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandDeliveryRecord {
    pub boundary: CommandDeliveryBoundary,
    pub spelling: ObservedToken,
    pub command: String,
    /// Canonical TeX82 command operand when the meaning has one.
    ///
    /// This is an identity from the installed primitive registry, never a
    /// fixture-derived value. Character spellings retain their own operand.
    pub command_operand: Option<i64>,
    pub provenance: CommandProvenance,
}

/// Canonical TeX82 command identity for an already delivered meaning.
///
/// This is instrumentation-only metadata.  It is derived from the installed
/// primitive registry, not from a fixture or host replay policy.
pub(crate) fn canonical_command_identity(meaning: Meaning) -> (String, Option<i64>) {
    match meaning {
        // TeX82 §23 can replace an offending outer control sequence's
        // `cur_cmd`/`cur_chr` with a space while the original control-sequence
        // token remains backed up for rereading.  Project character commands
        // from that effective pair, never from the input spelling.
        Meaning::CharToken { ch, cat } => (
            match cat {
                Catcode::BeginGroup => "left_brace",
                Catcode::EndGroup => "right_brace",
                Catcode::MathShift => "math_shift",
                Catcode::AlignmentTab => "tab_mark",
                Catcode::EndLine => "car_ret",
                Catcode::Parameter => "mac_param",
                Catcode::Letter => "letter",
                Catcode::Space => "spacer",
                Catcode::Other => "other_char",
                _ => "character",
            }
            .into(),
            Some(i64::from(u32::from(ch))),
        ),
        // TeX.web's `relax` command has the fixed `cur_chr` value 256.  It
        // is a distinguished meaning rather than a primitive-registry entry,
        // but remains observable at both raw and expanded delivery.
        Meaning::Relax => ("relax".into(), Some(256)),
        Meaning::EndV => ("endv".into(), Some(249_988)),
        Meaning::Macro { flags, .. } => (
            if flags.contains(tex_state::meaning::MeaningFlags::LONG)
                && flags.contains(tex_state::meaning::MeaningFlags::OUTER)
            {
                "long_outer_call"
            } else if flags.contains(tex_state::meaning::MeaningFlags::LONG) {
                "long_call"
            } else if flags.contains(tex_state::meaning::MeaningFlags::OUTER) {
                "outer_call"
            } else {
                "call"
            }
            .into(),
            None,
        ),
        // Every `ExpandablePrimitive` variant (including `The`/`NoExpand`/
        // `EndTemplate`, which used to have their own arms here) has a real
        // tex.web/e-TeX/pdfTeX command identity, computed exhaustively by
        // `expandable_primitive_identity` (`docs/tex_command_core.md`
        // §33.2's dispatch-completeness invariant, applied to this
        // classifier): a variant added to the enum without a named arm there
        // is a build failure, not a silent generic fallback.
        Meaning::ExpandablePrimitive(primitive) => expandable_primitive_identity(primitive),
        Meaning::IntParam(index) => ("assign_int".into(), Some(27_167 + i64::from(index))),
        // TeX82's named glue parameters occupy the contiguous `assign_glue`
        // command range. Their selector is the glue-parameter base plus the
        // stored parameter index (for example, `\\tabskip` is 24538).
        Meaning::GlueParam(index) => ("assign_glue".into(), Some(24_527 + i64::from(index))),
        Meaning::TokParam(index) => ("assign_toks".into(), Some(25_058 + i64::from(index))),
        // Every `UnexpandablePrimitive` variant has a real tex.web/e-TeX/pdfTeX
        // command identity, computed exhaustively by `unexpandable_primitive_identity`
        // (`docs/tex_command_core.md` §33.2's dispatch-completeness invariant,
        // applied to this classifier): a variant added to the enum without a
        // named arm there is a build failure, not a silent generic fallback.
        Meaning::UnexpandablePrimitive(primitive) => unexpandable_primitive_identity(primitive),
        // TeX82 §208 gives `\chardef` and `\mathchardef` constants their own
        // command codes, `char_given=68` and `math_given=69`, and §1224's
        // `shorthand_def` stores the scanned code as the `equiv`/`cur_chr` of
        // the defined control sequence (§1222: "A `\chardef` creates a control
        // sequence whose `cmd` is `char_given`"). §413's
        // `scan_something_internal` then reads that `cur_chr` at `int_val`,
        // but that is how the constant is *scanned*, not how it is
        // *classified*: both stay distinct commands with the stored code as
        // their operand at every delivery boundary, which is also what lets
        // §935 and §1030's main control treat `char_given` like `char_num`.
        Meaning::CharGiven(character) => {
            ("char_given".into(), Some(i64::from(u32::from(character))))
        }
        Meaning::MathCharGiven(code) => ("math_given".into(), Some(i64::from(code))),
        Meaning::Undefined => ("undefined_cs".into(), Some(-268_435_455)),
        _ => ("internal".into(), None),
    }
}

/// Canonical TeX82 observer identity for one delivered current command.
///
/// Most identities derive from the effective meaning. `\\expandafter`,
/// `\\csname`, `\\endcsname`, and `\\noexpand` are the deliberate exceptions:
/// the first three have TeX82's dedicated command codes, while the latter keeps the relaxed
/// meaning but changes its current-character identity to `no_expand_flag`
/// (257). Both distinctions are carried by `CurrentCommand`, so observation
/// merely projects command state.
pub(crate) fn canonical_current_command_identity(
    command: &CurrentCommand,
) -> (String, Option<i64>) {
    match command.identity() {
        CommandIdentity::Ordinary => canonical_command_identity(command.meaning()),
        // TeX82 §25 dispatches `\expandafter` through the dedicated
        // `expand_after` command with selector zero. The current command owns
        // that identity before its two-token expansion lifecycle begins.
        CommandIdentity::ExpandAfter => ("expand_after".into(), Some(0)),
        // TeX82 §25 dispatches `\csname` through `cs_name` with selector
        // zero before collecting expanded character commands through its
        // matching `\endcsname` boundary.
        CommandIdentity::CsName => ("cs_name".into(), Some(0)),
        // The `\csname` collector observes its inaccessible boundary through
        // the same raw path, where TeX82 retains `end_cs_name` and selector zero.
        CommandIdentity::EndCsName => ("end_cs_name".into(), Some(0)),
        // TeX82 §35 installs the six classic text conversions under the
        // shared `convert` command; §27's `conv_toks` uses the retained
        // selector to choose the existing scan/render lifecycle.
        CommandIdentity::Convert(selector) => ("convert".into(), Some(selector.operand())),
        // TeX82 §18 installs the classic diagnostic primitives under the
        // shared `xray` command. The selector remains with the delivery even
        // though the executor later dispatches each typed diagnostic action.
        CommandIdentity::XRay(selector) => ("xray".into(), Some(selector.operand())),
        CommandIdentity::NoExpandFrozenRelax => {
            debug_assert_eq!(command.meaning(), Meaning::Relax);
            ("relax".into(), Some(257))
        }
    }
}

/// Logical input changes observable at the canonical raw-input seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputTransition {
    Push,
    Retire,
    Stop,
    Backup,
    Recovery,
}

/// The semantic class of one input level.
///
/// This accompanies retirement because TeX's observable input lifecycle
/// distinguishes a backed-up token from a physical source even though both
/// use the same retire transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputReason {
    Source,
    Backup,
    Macro,
    Parameter,
    AlignmentUTemplate,
    AlignmentVTemplate,
    Recovery,
    TokenList,
    Write,
}

/// One input transition with its deterministic aggregate provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRecord {
    pub transition: InputTransition,
    pub reason: InputReason,
    pub level: u64,
    pub position: u64,
}

/// Canonical class of a committed input recovery.
///
/// TeX's recovery trace classifies the recovery operation independently from
/// the concrete token that it inserts. In particular, an inserted frozen
/// control-sequence token can be an `InsertedToken` operation. Preserve that
/// semantic fact at the command observer boundary; transports must not infer
/// it from a token's spelling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryKind {
    Backup,
    InsertedToken,
    InsertedControlSequence,
}

/// One backup or canonical recovery insertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub kind: RecoveryKind,
    pub tokens: Vec<ObservedToken>,
}

/// A committed change between live scanner episodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerStatusRecord {
    pub from: String,
    pub to: String,
}

/// A completed scalar macro-match milestone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroRecord {
    pub activation: bool,
    pub definition: u64,
    pub control_sequence: Option<String>,
    pub argument: Option<u8>,
    pub token_count: u64,
    pub tokens: Vec<ObservedToken>,
}

/// A committed condition-stack transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionRecord {
    pub transition: &'static str,
    /// Stable stack identity for diagnostic context only; it is not part of
    /// the portable oracle event.
    pub identity: u64,
    /// Canonical TeX conditional name, e.g. `iftrue` or `ifcase`.
    pub condition: &'static str,
    /// Canonical TeX `if_limit` name at the transition seam.
    pub limit: &'static str,
    /// A branch selected at this seam, when applicable.
    pub branch: Option<String>,
}

/// A completed typed scanner result. Values are rendered only from the
/// scanner's owned semantic result, never from aggregate allocation state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerRecord {
    pub kind: &'static str,
    pub value: String,
    /// A frozen token-list scanner result, when the result domain is tokens.
    pub tokens: Option<Vec<ObservedToken>>,
}

/// One `scan_toks` direct splice or completed immutable collection.
///
/// `purpose` and `tokens` are captured at the command-owned collection seam,
/// before the executor takes ownership of a completed definition or general
/// text value.  They deliberately describe semantic token values rather than
/// token-list identities, so host-only schema translation never has to infer
/// a conversion from a later mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenListRecord {
    pub transition: &'static str,
    pub purpose: &'static str,
    pub tokens: Vec<ObservedToken>,
}

/// A raw-delivery alignment adjustment or template lifecycle transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlignmentRecord {
    pub transition: &'static str,
    pub alignment: Option<u64>,
    pub align_state: i32,
    /// Original spelling for an intercepted alignment delimiter.
    pub delimiter: Option<&'static str>,
    /// The raw-delivery value immediately before a state-changing transition.
    ///
    /// Lifecycle observations without a direct `align_state` mutation leave
    /// this absent. This keeps the command-owned record sufficient for a
    /// host-only canonical schema translation without giving the host an
    /// alignment-state shadow.
    pub previous_align_state: Option<i32>,
}

/// A typed command-relevant assignment seam. Assignment dispatch owns the
/// payload in later slices; retaining this record here keeps the observer
/// union complete without depending on an oracle transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationRecord {
    pub target: &'static str,
    pub value: String,
    pub key: Option<String>,
    pub tokens: Option<Vec<ObservedToken>>,
    pub global: bool,
}

/// A committed externally-visible command effect or final ordering marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectRecord {
    pub kind: &'static str,
    pub detail: String,
    /// The frozen expanded token payload for token-oriented effects such as
    /// TeX82 `\\write`; textual effects leave this absent.
    pub tokens: Option<Vec<ObservedToken>>,
}

/// A stable semantic diagnostic selected by a committed command transition.
///
/// Formatting remains executor/host policy; this record preserves only the
/// TeX82 diagnostic identity needed by detached conformance observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub severity: &'static str,
    pub diagnostic: &'static str,
    /// Canonical token arguments selected by the recovery site.
    pub arguments: Vec<DiagnosticArgument>,
}

/// One committed command-core observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandObservation {
    Command(CommandDeliveryRecord),
    Input(InputRecord),
    Recovery(RecoveryRecord),
    ScannerStatus(ScannerStatusRecord),
    Macro(MacroRecord),
    Condition(ConditionRecord),
    Scanner(ScannerRecord),
    TokenList(TokenListRecord),
    Alignment(AlignmentRecord),
    Mutation(MutationRecord),
    Diagnostic(DiagnosticRecord),
    Effect(EffectRecord),
}

/// Test/instrumentation sink for committed command-owned semantic records.
///
/// This interface is intentionally non-fallible. An instrumentation transport
/// must buffer or handle its own failures outside the command operation.
pub trait CommandObserver {
    fn committed(&mut self, observation: CommandObservation);
}

pub(crate) fn observed_token(
    token: TracedTokenWord,
    resolve: impl FnOnce(tex_state::interner::Symbol) -> String,
) -> ObservedToken {
    match token.semantic_token() {
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(resolve(symbol)),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        Token::Frozen(_) if token.semantic_token().is_frozen_end_template() => {
            ObservedToken::FrozenEndTemplate
        }
        Token::Frozen(_) if token.semantic_token().is_frozen_endv() => ObservedToken::FrozenEndV,
        Token::Frozen(frozen) => frozen
            .primitive_index()
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::meaning::{ExpandablePrimitive, UnexpandablePrimitive};

    #[test]
    fn par_uses_tex82_par_end_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)),
            ("par_end".into(), Some(256))
        );
    }

    #[test]
    fn shorthand_definitions_keep_their_own_tex82_command_codes() {
        // TeX82 §208's `char_given`/`math_given` are distinct commands whose
        // `cur_chr` is the code §1224's `shorthand_def` stored. §413 reading
        // them at `int_val` describes how they are *scanned*, not how they are
        // classified: plain.tex's `\chardef\active=13` must still deliver
        // `char_given` with operand 13.
        assert_eq!(
            canonical_command_identity(Meaning::CharGiven('\r')),
            ("char_given".into(), Some(13))
        );
        assert_eq!(
            canonical_command_identity(Meaning::MathCharGiven(10_000)),
            ("math_given".into(), Some(10_000))
        );
    }

    #[test]
    fn recovered_outer_control_projects_the_effective_space_command() {
        // TeX82 §23 backs up the outer control-sequence token, then replaces
        // the live `cur_cmd`/`cur_chr` pair with spacer/" ".
        let mut universe = tex_state::Universe::new();
        let outer = universe.intern("outer").symbol();
        let empty = universe.intern_token_list(&[]);
        let definition = universe.intern_macro(tex_state::macro_store::MacroMeaning::new(
            tex_state::meaning::MeaningFlags::OUTER,
            empty,
            empty,
        ));
        universe.set_meaning(
            outer,
            Meaning::Macro {
                flags: tex_state::meaning::MeaningFlags::OUTER,
                definition,
            },
        );
        let mut state = universe.command_context();
        let mut command = CurrentCommand::resolve(
            TracedTokenWord::pack(Token::Cs(outer), OriginId::UNKNOWN),
            DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        );

        command.recover_as_space();

        assert_eq!(
            canonical_current_command_identity(&command),
            ("spacer".into(), Some(i64::from(u32::from(' '))))
        );
    }

    #[test]
    fn csname_uses_tex82_cs_name_identity() {
        let mut universe = tex_state::Universe::new();
        let csname = universe.intern("csname").symbol();
        universe.set_meaning(
            csname,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::CsName),
        );
        let mut state = universe.command_context();
        let command = CurrentCommand::resolve(
            TracedTokenWord::pack(Token::Cs(csname), OriginId::UNKNOWN),
            DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        );

        assert_eq!(
            canonical_current_command_identity(&command),
            ("cs_name".into(), Some(0))
        );
    }

    #[test]
    fn endcsname_uses_tex82_end_cs_name_identity() {
        let mut universe = tex_state::Universe::new();
        let endcsname = universe.intern("endcsname").symbol();
        universe.set_meaning(
            endcsname,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::EndCsName),
        );
        let mut state = universe.command_context();
        let command = CurrentCommand::resolve(
            TracedTokenWord::pack(Token::Cs(endcsname), OriginId::UNKNOWN),
            DeliveryStamp::new(0, 0, 0),
            None,
            false,
            &mut state,
        );

        assert_eq!(
            canonical_current_command_identity(&command),
            ("end_cs_name".into(), Some(0))
        );
    }

    #[test]
    fn classic_conversions_use_tex82_convert_selectors() {
        let mut universe = tex_state::Universe::new();

        for (name, primitive, operand) in [
            ("number", ExpandablePrimitive::Number, 0),
            ("romannumeral", ExpandablePrimitive::RomanNumeral, 1),
            ("string", ExpandablePrimitive::String, 2),
            ("meaning", ExpandablePrimitive::Meaning, 3),
            ("fontname", ExpandablePrimitive::FontName, 4),
            ("jobname", ExpandablePrimitive::JobName, 5),
        ] {
            let symbol = universe.intern(name).symbol();
            universe.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
            let mut state = universe.command_context();
            let command = CurrentCommand::resolve(
                TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN),
                DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            );

            assert_eq!(
                canonical_current_command_identity(&command),
                ("convert".into(), Some(operand))
            );
        }
    }

    #[test]
    fn tex82_diagnostics_use_shared_xray_selectors() {
        let mut universe = tex_state::Universe::new();

        for (name, primitive, operand) in [
            ("show", UnexpandablePrimitive::Show, 0),
            ("showbox", UnexpandablePrimitive::ShowBox, 1),
            ("showthe", UnexpandablePrimitive::ShowThe, 2),
            ("showlists", UnexpandablePrimitive::ShowLists, 3),
        ] {
            let symbol = universe.intern(name).symbol();
            universe.set_meaning(symbol, Meaning::UnexpandablePrimitive(primitive));
            let mut state = universe.command_context();
            let command = CurrentCommand::resolve(
                TracedTokenWord::pack(Token::Cs(symbol), OriginId::UNKNOWN),
                DeliveryStamp::new(0, 0, 0),
                None,
                false,
                &mut state,
            );

            assert_eq!(
                canonical_current_command_identity(&command),
                ("xray".into(), Some(operand))
            );
        }
    }

    #[test]
    fn row_returns_use_tex82_car_ret_identities() {
        for (primitive, operand) in [
            (UnexpandablePrimitive::Cr, 257),
            (UnexpandablePrimitive::CrCr, 258),
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("car_ret".into(), Some(operand))
            );
        }
    }

    #[test]
    fn omit_uses_tex82_omit_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)),
            ("omit".into(), Some(0))
        );
    }

    #[test]
    fn noalign_uses_tex82_no_align_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::NoAlign
            )),
            ("no_align".into(), Some(0))
        );
    }

    #[test]
    fn toks_uses_tex82_register_base() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks)),
            ("toks_register".into(), Some(0))
        );
    }

    #[test]
    fn named_glue_parameters_use_tex82_assign_glue_selectors() {
        assert_eq!(
            canonical_command_identity(Meaning::GlueParam(11)),
            ("assign_glue".into(), Some(24_538))
        );
    }

    #[test]
    fn explicit_groups_use_tex82_command_identities() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::BeginGroup
            )),
            ("begin_group".into(), Some(0))
        );
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::EndGroup
            )),
            ("end_group".into(), Some(0))
        );
    }

    #[test]
    fn setbox_uses_tex82_command_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::SetBox
            )),
            ("set_box".into(), Some(0))
        );
    }

    #[test]
    fn glue_primitives_use_tex82_skip_command_selectors() {
        for (primitive, operand) in [
            (UnexpandablePrimitive::HFil, 0),
            (UnexpandablePrimitive::HFill, 1),
            (UnexpandablePrimitive::HSs, 2),
            (UnexpandablePrimitive::HFilNeg, 3),
            (UnexpandablePrimitive::HSkip, 4),
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("hskip".into(), Some(operand))
            );
        }
        for (primitive, operand) in [
            (UnexpandablePrimitive::VFil, 0),
            (UnexpandablePrimitive::VFill, 1),
            (UnexpandablePrimitive::VSs, 2),
            (UnexpandablePrimitive::VFilNeg, 3),
            (UnexpandablePrimitive::VSkip, 4),
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("vskip".into(), Some(operand))
            );
        }
    }

    #[test]
    fn shipout_uses_tex82_leader_ship_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::Shipout
            )),
            ("leader_ship".into(), Some(99))
        );
    }

    #[test]
    fn definition_and_the_identities_follow_tex82_command_codes() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Edef)),
            ("def".into(), Some(2))
        );
        assert_eq!(
            canonical_command_identity(Meaning::ExpandablePrimitive(
                tex_state::meaning::ExpandablePrimitive::The
            )),
            ("the".into(), Some(0))
        );
    }

    #[test]
    fn box_uses_tex82_make_box_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)),
            ("make_box".into(), Some(0))
        );
    }

    #[test]
    fn output_primitives_use_tex82_extension_identities() {
        for (primitive, operand) in [
            (UnexpandablePrimitive::OpenOut, 0),
            (UnexpandablePrimitive::Write, 1),
            (UnexpandablePrimitive::CloseOut, 2),
            (UnexpandablePrimitive::Special, 3),
            (UnexpandablePrimitive::Immediate, 4),
            (UnexpandablePrimitive::SetLanguage, 5),
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("extension".into(), Some(operand))
            );
        }
    }

    #[test]
    fn input_uses_tex82_filename_command_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::ExpandablePrimitive(ExpandablePrimitive::Input)),
            ("input".into(), Some(0))
        );
    }

    #[test]
    fn tex82_conditionals_use_shared_if_test_identity() {
        let expected = [
            (ExpandablePrimitive::If, 0),
            (ExpandablePrimitive::IfCat, 1),
            (ExpandablePrimitive::IfNum, 2),
            (ExpandablePrimitive::IfDim, 3),
            (ExpandablePrimitive::IfOdd, 4),
            (ExpandablePrimitive::IfVMode, 5),
            (ExpandablePrimitive::IfHMode, 6),
            (ExpandablePrimitive::IfMMode, 7),
            (ExpandablePrimitive::IfInner, 8),
            (ExpandablePrimitive::IfVoid, 9),
            (ExpandablePrimitive::IfHBox, 10),
            (ExpandablePrimitive::IfVBox, 11),
            (ExpandablePrimitive::IfX, 12),
            (ExpandablePrimitive::IfEof, 13),
            (ExpandablePrimitive::IfTrue, 14),
            (ExpandablePrimitive::IfFalse, 15),
            (ExpandablePrimitive::IfCase, 16),
        ];

        for (primitive, operand) in expected {
            assert_eq!(
                canonical_command_identity(Meaning::ExpandablePrimitive(primitive)),
                ("if_test".into(), Some(operand))
            );
        }
    }

    #[test]
    fn tex82_conditional_delimiters_use_shared_fi_or_else_identity() {
        let expected = [
            (ExpandablePrimitive::Fi, 2),
            (ExpandablePrimitive::Else, 3),
            (ExpandablePrimitive::Or, 4),
        ];

        for (primitive, operand) in expected {
            assert_eq!(
                canonical_command_identity(Meaning::ExpandablePrimitive(primitive)),
                ("fi_or_else".into(), Some(operand))
            );
        }
    }
}

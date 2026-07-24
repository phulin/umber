//! Test and instrumentation-only command semantic observation.
//!
//! These records deliberately belong to `tex-command`, rather than to the
//! fixture/oracle crate.  They are built from values already available at the
//! transition seam and are delivered only after the transition has committed.
//! In particular, an observer is non-fallible and never participates in
//! command state, snapshots, delivery, expansion, or scanner control flow.

#![cfg(any(test, feature = "instrumentation"))]

use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{DeliveryStamp, SourceRange};

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
}

impl PartialEq for CommandProvenance {
    fn eq(&self, other: &Self) -> bool {
        self.input_level == other.input_level
            && self.position == other.position
            && self.delivery_sequence == other.delivery_sequence
            && self.has_origin == other.has_origin
            && self.source_range == other.source_range
    }
}

impl CommandProvenance {
    pub(crate) fn from_stamp(
        stamp: DeliveryStamp,
        origin: OriginId,
        source_range: Option<SourceRange>,
    ) -> Self {
        Self {
            input_level: stamp.input_level(),
            position: stamp.position(),
            delivery_sequence: stamp.sequence(),
            has_origin: origin != OriginId::UNKNOWN,
            origin,
            source_range,
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
        Meaning::CharToken { .. } => ("character".into(), None),
        // TeX.web's `relax` command has the fixed `cur_chr` value 256.  It
        // is a distinguished meaning rather than a primitive-registry entry,
        // but remains observable at both raw and expanded delivery.
        Meaning::Relax => ("relax".into(), Some(256)),
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
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::The) => {
            ("the".into(), Some(0))
        }
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::NoExpand) => {
            ("no_expand".into(), Some(0))
        }
        // TeX82 stores every `\if...` primitive under the shared `if_test`
        // command code; `cur_chr` selects the particular test. The Rust
        // primitive enum's discriminants are an implementation detail, so
        // observation maps the canonical TeX82 identity explicitly.
        Meaning::ExpandablePrimitive(primitive) => match primitive {
            ExpandablePrimitive::If => ("if_test".into(), Some(0)),
            ExpandablePrimitive::IfCat => ("if_test".into(), Some(1)),
            ExpandablePrimitive::IfNum => ("if_test".into(), Some(2)),
            ExpandablePrimitive::IfDim => ("if_test".into(), Some(3)),
            ExpandablePrimitive::IfOdd => ("if_test".into(), Some(4)),
            ExpandablePrimitive::IfVMode => ("if_test".into(), Some(5)),
            ExpandablePrimitive::IfHMode => ("if_test".into(), Some(6)),
            ExpandablePrimitive::IfMMode => ("if_test".into(), Some(7)),
            ExpandablePrimitive::IfInner => ("if_test".into(), Some(8)),
            ExpandablePrimitive::IfVoid => ("if_test".into(), Some(9)),
            ExpandablePrimitive::IfHBox => ("if_test".into(), Some(10)),
            ExpandablePrimitive::IfVBox => ("if_test".into(), Some(11)),
            ExpandablePrimitive::IfX => ("if_test".into(), Some(12)),
            ExpandablePrimitive::IfEof => ("if_test".into(), Some(13)),
            ExpandablePrimitive::IfTrue => ("if_test".into(), Some(14)),
            ExpandablePrimitive::IfFalse => ("if_test".into(), Some(15)),
            ExpandablePrimitive::IfCase => ("if_test".into(), Some(16)),
            // TeX82 likewise stores conditional delimiters under one command
            // code. Their `cur_chr` operands are `fi_code`, `else_code`, and
            // `or_code`, rather than the Rust primitive enum discriminants.
            ExpandablePrimitive::Fi => ("fi_or_else".into(), Some(2)),
            ExpandablePrimitive::Else => ("fi_or_else".into(), Some(3)),
            ExpandablePrimitive::Or => ("fi_or_else".into(), Some(4)),
            // `\input` is a TeX82 command family with its fixed filename
            // selector, rather than a generic expandable command.
            ExpandablePrimitive::Input => ("input".into(), Some(0)),
            _ => ("expandable".into(), None),
        },
        Meaning::IntParam(index) => ("assign_int".into(), Some(27_167 + i64::from(index))),
        // TeX82's named glue parameters occupy the contiguous `assign_glue`
        // command range. Their selector is the glue-parameter base plus the
        // stored parameter index (for example, `\\tabskip` is 24538).
        Meaning::GlueParam(index) => ("assign_glue".into(), Some(24_527 + i64::from(index))),
        Meaning::TokParam(index) => ("assign_toks".into(), Some(25_058 + i64::from(index))),
        Meaning::UnexpandablePrimitive(primitive) => match primitive {
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef => (
                "def".into(),
                Some(match primitive {
                    UnexpandablePrimitive::Def => 0,
                    UnexpandablePrimitive::Gdef => 1,
                    UnexpandablePrimitive::Edef => 2,
                    UnexpandablePrimitive::Xdef => 3,
                    _ => unreachable!("definition primitive is matched above"),
                }),
            ),
            UnexpandablePrimitive::Long => ("prefix".into(), Some(1)),
            UnexpandablePrimitive::Outer => ("prefix".into(), Some(2)),
            UnexpandablePrimitive::Global => ("prefix".into(), Some(4)),
            UnexpandablePrimitive::Let => ("let".into(), Some(0)),
            UnexpandablePrimitive::FutureLet => ("let".into(), Some(1)),
            UnexpandablePrimitive::Count => ("register".into(), Some(0)),
            UnexpandablePrimitive::Dimen => ("register".into(), Some(1)),
            UnexpandablePrimitive::Skip => ("register".into(), Some(2)),
            // `\toks` is its own command family in TeX82 and starts at
            // `cur_chr = 0`; the Rust selector is not a trace operand.
            UnexpandablePrimitive::Toks => ("toks_register".into(), Some(0)),
            UnexpandablePrimitive::CatCode => ("def_code".into(), Some(25_631)),
            UnexpandablePrimitive::LcCode => ("def_code".into(), Some(25_887)),
            // TeX.web's primitive `\par` is `par_end` with the distinguished
            // `cur_chr` value 256; this is not the Rust primitive enum's
            // storage operand.
            UnexpandablePrimitive::Par => ("par_end".into(), Some(256)),
            // TeX82 gives both row-return primitives the shared `car_ret`
            // command identity. Their selectors distinguish `\cr` from
            // `\crcr` and are consumed by alignment handling after raw
            // delivery; the Rust primitive enum must not leak into traces.
            // See TeX.web's `cr_code` and `cr_cr_code` definitions.
            UnexpandablePrimitive::Cr => ("car_ret".into(), Some(257)),
            UnexpandablePrimitive::CrCr => ("car_ret".into(), Some(258)),
            UnexpandablePrimitive::HAlign => ("halign".into(), Some(0)),
            // TeX82 registers the horizontal glue shorthands with the same
            // `hskip` command code and a zero `cur_chr`; their distinct
            // Rust variants select the executor's prebuilt glue value only.
            UnexpandablePrimitive::HSkip
            | UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg => ("hskip".into(), Some(0)),
            // The vertical family is analogous, sharing TeX82's `vskip`
            // command identity rather than exposing shorthand variants.
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg => ("vskip".into(), Some(0)),
            UnexpandablePrimitive::SetBox => ("set_box".into(), Some(0)),
            UnexpandablePrimitive::HBox => ("make_box".into(), Some(4)),
            UnexpandablePrimitive::VBox => ("make_box".into(), Some(5)),
            UnexpandablePrimitive::VTop => ("make_box".into(), Some(6)),
            // Rule primitives have fixed TeX82 command codes and a zero
            // selector; the executor's Rust primitive identity is not a
            // canonical command-trace operand.
            UnexpandablePrimitive::VRule => ("vrule".into(), Some(0)),
            UnexpandablePrimitive::HRule => ("hrule".into(), Some(0)),
            // Explicit group primitives share TeX82's `begin_group` and
            // `end_group` command codes with a zero selector. Their Rust
            // enum discriminants must not leak into the trace.
            UnexpandablePrimitive::BeginGroup => ("begin_group".into(), Some(0)),
            UnexpandablePrimitive::EndGroup => ("end_group".into(), Some(0)),
            UnexpandablePrimitive::Message => ("message".into(), Some(0)),
            UnexpandablePrimitive::End => ("stop".into(), Some(0)),
            _ => ("unexpandable".into(), None),
        },
        Meaning::Undefined => ("undefined_cs".into(), Some(-268_435_455)),
        _ => ("internal".into(), None),
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
}

/// One input transition with its deterministic aggregate provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputRecord {
    pub transition: InputTransition,
    pub reason: InputReason,
    pub level: u64,
    pub position: u64,
}

/// One backup or canonical recovery insertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub backup: bool,
    pub tokens: Vec<ObservedToken>,
}

/// A committed entry to or restoration from a live scanner episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerStatusRecord {
    pub entering: bool,
    pub status: String,
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
}

/// A stable semantic diagnostic selected by a committed command transition.
///
/// Formatting remains executor/host policy; this record preserves only the
/// TeX82 diagnostic identity needed by detached conformance observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub severity: &'static str,
    pub diagnostic: &'static str,
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

    #[test]
    fn par_uses_tex82_par_end_identity() {
        assert_eq!(
            canonical_command_identity(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)),
            ("par_end".into(), Some(256))
        );
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
    fn glue_shorthands_use_tex82_skip_command_identities() {
        for primitive in [
            UnexpandablePrimitive::HSkip,
            UnexpandablePrimitive::HFil,
            UnexpandablePrimitive::HFill,
            UnexpandablePrimitive::HSs,
            UnexpandablePrimitive::HFilNeg,
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("hskip".into(), Some(0))
            );
        }
        for primitive in [
            UnexpandablePrimitive::VSkip,
            UnexpandablePrimitive::VFil,
            UnexpandablePrimitive::VFill,
            UnexpandablePrimitive::VSs,
            UnexpandablePrimitive::VFilNeg,
        ] {
            assert_eq!(
                canonical_command_identity(Meaning::UnexpandablePrimitive(primitive)),
                ("vskip".into(), Some(0))
            );
        }
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

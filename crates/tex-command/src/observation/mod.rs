//! Command semantic observation.
//!
//! These records deliberately belong to `tex-command`, rather than to the
//! fixture/oracle crate.  They are built from values already available at the
//! transition seam and are delivered only after the transition has committed.
//! In particular, an observer is non-fallible and never participates in
//! command state, snapshots, delivery, expansion, or scanner control flow.

use std::sync::Arc;

use tex_state::meaning::{Meaning, MeaningFlags, ResolvedMeaning};

use crate::command::{CommandIdentity, CurrentCommand};
use crate::profile::CommandProfile;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::{DeliveryStamp, SourceLocation, SourceNameClass, SourceProvenance, SourceRange};

pub mod canonical_names;
mod primitive_identity;
mod variable_identity;
use canonical_names::character_command_name;
use primitive_identity::{expandable_primitive_identity, unexpandable_primitive_identity};
pub(crate) use variable_identity::LOCAL_BASE;
pub use variable_identity::parameter_mutation_key_for_dialect;
pub use variable_identity::{ParameterClass, parameter_mutation_key};

/// An owned, allocation-independent spelling used by command observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservedToken {
    Character {
        character: char,
        catcode: Catcode,
    },
    ControlSequence(String),
    MacroMatch,
    MacroEndMatch,
    Parameter(u8),
    FrozenEndTemplate,
    FrozenEndV,
    /// A frozen primitive sentinel, carrying tex.web's `text` for the frozen
    /// control sequence rather than an engine-local slot index. The spelling
    /// is resolved where the token is observed, because an observation payload
    /// must never carry an allocation identity a transport would have to
    /// render itself.
    FrozenPrimitive(String),
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
        delivery_sequence: u64,
        origin: OriginId,
        source_provenance: Option<SourceProvenance>,
    ) -> Self {
        Self {
            input_level: stamp.input_level(),
            position: stamp.position(),
            delivery_sequence,
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
    /// Portable semantic operand for commands whose WEB operand is an
    /// allocator address rather than a stable selector.
    pub semantic_operand: Option<String>,
    pub provenance: CommandProvenance,
}

/// Canonical TeX82 command identity for an already delivered meaning.
///
/// This is instrumentation-only metadata.  It is derived from the installed
/// primitive registry, not from a fixture or host replay policy.
///
/// The match below is EXHAUSTIVE over `Meaning`: a variant added to
/// `tex_state::meaning::Meaning` without a deliberate arm here is a build
/// failure (`error[E0004]`), not a silent generic identity. It used to end
/// in `_ => ("internal", None)`, which is strictly worse than failing --
/// every register-defining and parameter meaning that reached it was
/// reported to the differential tracer as a plausible-looking command with
/// no selector, so the oracle comparison ran against fabricated data. This
/// is `docs/tex_command_core.md` §33.2's dispatch-completeness invariant
/// applied to classification, the same remedy `primitive_identity` already
/// applies beneath the two primitive arms.
pub(crate) fn canonical_command_identity(meaning: Meaning) -> (String, Option<i64>) {
    canonical_command_identity_for_profile(CommandProfile::TEX82, meaning)
}

pub(crate) fn canonical_command_identity_for_profile(
    profile: CommandProfile,
    meaning: Meaning,
) -> (String, Option<i64>) {
    let dialect = profile.dialect();
    match meaning {
        // TeX82 §23 can replace an offending outer control sequence's
        // `cur_cmd`/`cur_chr` with a space while the original control-sequence
        // token remains backed up for rereading.  Project character commands
        // from that effective pair, never from the input spelling.
        //
        // The command name comes from `canonical_names::character_command_name`,
        // the single §207 table for character commands. It used to end in a
        // `_ => "character"` catch-all, which silently reported every
        // `\catcode`-7 and `\catcode`-8 character as a plausible-looking
        // command name no engine installs, masking real divergences behind it
        // (`umber2-johp.141`).
        Meaning::CharToken { ch, cat } => (
            character_command_name(cat)
                .unwrap_or_else(|| {
                    // §341's `get_next` consumes escape, ignore, comment, and
                    // invalid characters and replaces an active character by
                    // its meaning, so none of them can reach a delivered
                    // command. A stored meaning that claims otherwise is a
                    // `tex-state` defect and must look like one rather than
                    // borrowing a real command's name.
                    debug_assert!(false, "catcode {cat:?} has no §207 character command");
                    "uncommandable_character"
                })
                .into(),
            Some(i64::from(u32::from(ch))),
        ),
        // TeX.web's `relax` command has the fixed `cur_chr` value 256.  It
        // is a distinguished meaning rather than a primitive-registry entry,
        // but remains observable at both raw and expanded delivery.
        Meaning::Relax => ("relax".into(), Some(256)),
        Meaning::EndV => ("endv".into(), Some(249_988)),
        // Every `ExpandablePrimitive` variant (including `The`/`NoExpand`/
        // `EndTemplate`, which used to have their own arms here) has a real
        // tex.web/e-TeX/pdfTeX command identity, computed exhaustively by
        // `expandable_primitive_identity` (`docs/tex_command_core.md`
        // §33.2's dispatch-completeness invariant, applied to this
        // classifier): a variant added to the enum without a named arm there
        // is a build failure, not a silent generic fallback.
        Meaning::ExpandablePrimitive(primitive) => {
            expandable_primitive_identity(dialect, primitive)
        }
        // TeX82's named parameters and classical registers are variables
        // whose command selector is a real eqtb address. `variable_identity`
        // owns both the region bases and the translation from Umber's dense
        // bank slot to tex.web's parameter code, which is NOT the identity
        // map: `\\fam` is Umber slot 59 and tex.web §236 code 44.
        Meaning::IntParam(slot) => (
            "assign_int".into(),
            variable_identity::int_parameter_code(dialect, slot)
                .map(|code| variable_identity::int_base(dialect) + code),
        ),
        Meaning::DimenParam(slot) => (
            "assign_dimen".into(),
            variable_identity::dimen_parameter_address(dialect, slot),
        ),
        Meaning::GlueParam(slot) => (
            "assign_glue".into(),
            variable_identity::glue_parameter_code(slot)
                .map(|code| variable_identity::GLUE_BASE + code),
        ),
        // TeX82 §224 stores `\\thinmuskip`, `\\medmuskip`, and `\\thickmuskip`
        // in the same glue-parameter region as the ordinary glue parameters
        // (codes 15..17); only their command differs.
        Meaning::MuGlueParam(slot) => (
            "assign_mu_glue".into(),
            variable_identity::glue_parameter_code(slot)
                .map(|code| variable_identity::GLUE_BASE + code),
        ),
        Meaning::TokParam(slot) => (
            "assign_toks".into(),
            variable_identity::token_parameter_address(dialect, slot),
        ),
        // TeX82 §1224's `shorthand_def` gives a `\\countdef`/`\\dimendef`/
        // `\\skipdef`/`\\muskipdef`/`\\toksdef` control sequence the command
        // of the register class it names and the register's own eqtb address
        // as its selector -- `define(p,assign_int,count_base+cur_val)` for
        // `\\countdef`. Such a control sequence is therefore indistinguishable
        // from a named parameter of the same class at every delivery
        // boundary, which is exactly what lets §1226's `prefixed_command`
        // assign through it.
        Meaning::CountRegister(index) if profile.capabilities().supports_etex() && index > 255 => {
            ("register".into(), None)
        }
        Meaning::DimenRegister(index) if profile.capabilities().supports_etex() && index > 255 => {
            ("register".into(), None)
        }
        Meaning::SkipRegister(index) if profile.capabilities().supports_etex() && index > 255 => {
            ("register".into(), None)
        }
        Meaning::MuskipRegister(index) if profile.capabilities().supports_etex() && index > 255 => {
            ("register".into(), None)
        }
        Meaning::ToksRegister(index) if profile.capabilities().supports_etex() && index > 255 => {
            ("toks_register".into(), None)
        }
        Meaning::CountRegister(index) => (
            "assign_int".into(),
            Some(variable_identity::count_base(dialect) + i64::from(index)),
        ),
        Meaning::DimenRegister(index) => (
            "assign_dimen".into(),
            Some(variable_identity::scaled_base(dialect) + i64::from(index)),
        ),
        Meaning::SkipRegister(index) => (
            "assign_glue".into(),
            Some(variable_identity::SKIP_BASE + i64::from(index)),
        ),
        Meaning::MuskipRegister(index) => (
            "assign_mu_glue".into(),
            Some(variable_identity::MU_SKIP_BASE + i64::from(index)),
        ),
        Meaning::ToksRegister(index) => (
            "assign_toks".into(),
            Some(variable_identity::toks_base(dialect) + i64::from(index)),
        ),
        // TeX82 §982/§986 install the page-so-far quantities under
        // `set_page_dimen` and `set_page_int`, selected by their small
        // `page_so_far`/`dead_cycles` ordinal rather than an eqtb address.
        Meaning::PageDimension(dimension) => {
            ("set_page_dimen".into(), Some(i64::from(dimension.index())))
        }
        Meaning::PageInteger(integer) => ("set_page_int".into(), Some(i64::from(integer.index()))),
        // TeX82 §416 installs the read-only internal quantities under the
        // shared `last_item` command; `primitive_identity` classifies the
        // ones Umber models as primitives, and this arm the ones it models as
        // state (`\\badness`, `\\inputlineno`, and the e-TeX/pdfTeX
        // extensions).
        Meaning::InternalInteger(integer) => (
            "last_item".into(),
            variable_identity::internal_integer_code(dialect, integer),
        ),
        // TeX82 §1257's `new_font` defines a font identifier as
        // `define(u,set_font,null_font)` and then stores the internal font
        // number, so a font control sequence delivers `set_font` with that
        // number (§577's `\\nullfont` is the same command with number zero).
        Meaning::Font(font) => ("set_font".into(), Some(i64::from(font.raw()))),
        // Every `UnexpandablePrimitive` variant has a real tex.web/e-TeX/pdfTeX
        // command identity, computed exhaustively by `unexpandable_primitive_identity`
        // (`docs/tex_command_core.md` §33.2's dispatch-completeness invariant,
        // applied to this classifier): a variant added to the enum without a
        // named arm there is a build failure, not a silent generic fallback.
        Meaning::UnexpandablePrimitive(primitive) => {
            unexpandable_primitive_identity(dialect, primitive)
        }
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
        // A stored meaning word whose opcode, flags, or operand does not
        // decode into any modeled meaning. This is not a TeX command at all,
        // so it deliberately gets a name no engine installs rather than
        // being folded into a real command family: a spelling that reaches
        // the trace under this name is a `tex-state` decoding defect, and
        // must look like one.
        Meaning::Unknown(_) => ("undecodable_meaning".into(), None),
    }
}

/// e-TeX 2.6 [49.1221--1224] stores a sparse register shorthand's array-node
/// pointer in `cur_chr`. Sections [49.5508--5523] define the portable identity
/// encoded by that node as its register type and `print_sa_num` index.
pub(crate) fn canonical_sparse_register_operand<G>(
    profile: CommandProfile,
    meaning: tex_state::ResolvedMeaning<G>,
) -> Option<String> {
    if !profile.capabilities().supports_etex() {
        return None;
    }
    let tex_state::ResolvedMeaning::Static(meaning) = meaning else {
        return None;
    };
    canonical_names::sparse_register_operand_name(meaning)
}

/// Canonical TeX82 observer identity for one delivered current command.
///
/// Most identities derive from the effective meaning. `\\expandafter`,
/// `\\csname`, `\\endcsname`, and `\\noexpand` are the deliberate exceptions:
/// the first three have TeX82's dedicated command codes, while the latter keeps the relaxed
/// meaning but changes its current-character identity to `no_expand_flag`
/// (257). Both distinctions are carried by `CurrentCommand`, so observation
/// merely projects command state.
#[cfg(test)]
pub(crate) fn canonical_current_command_identity<G>(
    command: &CurrentCommand<G>,
) -> (String, Option<i64>) {
    canonical_current_command_identity_for_profile(CommandProfile::TEX82, command)
}

pub(crate) fn canonical_current_command_identity_for_profile<G>(
    profile: CommandProfile,
    command: &CurrentCommand<G>,
) -> (String, Option<i64>) {
    match command.identity() {
        CommandIdentity::Ordinary => {
            let (name, operand) = match command.meaning() {
                ResolvedMeaning::Static(meaning) => {
                    canonical_command_identity_for_profile(profile, meaning)
                }
                ResolvedMeaning::Macro { flags, .. } => (macro_command_name(flags).into(), None),
            };
            (name, operand)
        }
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
        CommandIdentity::Convert(selector) => {
            let operand = match (profile.dialect(), selector) {
                (crate::CommandDialect::Etex26, crate::command::ConvertSelector::JobName) => 6,
                (crate::CommandDialect::Pdftex14029, crate::command::ConvertSelector::JobName) => {
                    33
                }
                _ => selector.operand(),
            };
            ("convert".into(), Some(operand))
        }
        // TeX82 §18 installs the classic diagnostic primitives under the
        // shared `xray` command. The selector remains with the delivery even
        // though the executor later dispatches each typed diagnostic action.
        CommandIdentity::XRay(selector) => ("xray".into(), Some(selector.operand())),
        CommandIdentity::NoExpandFrozenRelax => {
            debug_assert_eq!(command.meaning(), ResolvedMeaning::Static(Meaning::Relax));
            ("relax".into(), Some(257))
        }
    }
}

fn macro_command_name(flags: MeaningFlags) -> &'static str {
    if flags.contains(MeaningFlags::LONG) && flags.contains(MeaningFlags::OUTER) {
        "long_outer_call"
    } else if flags.contains(MeaningFlags::LONG) {
        "long_call"
    } else if flags.contains(MeaningFlags::OUTER) {
        "outer_call"
    } else {
        "call"
    }
}

/// Logical input changes observable at the canonical raw-input seam.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
///
/// tex.web splits the classification in two. §303's `state` separates a file
/// or terminal level from a token-list level, and §307's `token_type` names
/// _which_ of sixteen kinds of token list a token-list level is. Collapsing
/// the nine `token_type` codes at and above `output_text` into one
/// `TokenList` variant left the transport with nothing to name them from, so
/// every stored level was reported as `\output`'s (`umber2-johp.191`). Each
/// variant below is therefore exactly one `token_type`, and its code is
/// stated; there is no variant that stands for more than one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputReason {
    /// §303 `state<>token_list`: a file or the terminal.
    Source,
    /// §307 `parameter=0`.
    Parameter,
    /// §307 `u_template=1`.
    AlignmentUTemplate,
    /// §307 `v_template=2`.
    AlignmentVTemplate,
    /// §307 `backed_up=3`.
    Backup,
    /// §307 `inserted=4`.
    Recovery,
    /// §307 `macro=5`.
    Macro,
    /// §307 `output_text=6`.
    OutputRoutine,
    /// §307 `every_par_text=7`.
    EveryPar,
    /// §307 `every_math_text=8`.
    EveryMath,
    /// §307 `every_display_text=9`.
    EveryDisplay,
    /// §307 `every_hbox_text=10`.
    EveryHBox,
    /// §307 `every_vbox_text=11`.
    EveryVBox,
    /// §307 `every_job_text=12`.
    EveryJob,
    /// §307 `every_cr_text=13`.
    EveryCr,
    /// e-TeX §22.307 `every_eof_text`.
    EveryEof,
    /// §307 `mark_text=14`.
    Mark,
    /// §307 `write_text=15`.
    Write,
    /// A replay level Umber's command state owns for material tex.web reads
    /// live, which therefore has no §307 `token_type` to report.
    ///
    /// These are named under an `umber` marker for the same reason
    /// `parameter_mutation_key` names a bank slot with no tex.web code that
    /// way: reporting one of them as `output` would be indistinguishable
    /// from -- and could silently agree with -- a real `\output` level.
    UmberReplay(UmberReplayKind),
}

/// One replay level Umber owns that tex.web has no `token_type` for.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum UmberReplayKind {
    /// A `\discretionary` part (tex.web §1117 reads each part live).
    Discretionary,
}

/// One input transition with its deterministic aggregate provenance.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InputRecord {
    pub transition: InputTransition,
    pub reason: InputReason,
    /// tex.web §303's `name` classification of a source level.
    ///
    /// This is `Some` exactly when `reason` is [`InputReason::Source`].
    /// §303 partitions `name` into the terminal, a `\read` stream, and a text
    /// file, and §329's `end_file_reading` acts on that partition; none of it
    /// is expressible as an `InputReason`, whose remaining arms are a strict
    /// one-to-one model of §307's `token_type` codes.
    pub source_name: Option<SourceNameClass>,
    /// Immutable engine source identity for source-level transitions.
    ///
    /// This is absent for token-list levels. Carrying it at the owning input
    /// transition lets detached projection name nested retirements without a
    /// parallel source stack.
    pub source: Option<tex_state::SourceId>,
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
///
/// Both ends carry tex.web §305's `scanner_status` name, never a `Debug`
/// rendering of Umber's own variant and its episode context: a transport must
/// receive the canonical vocabulary, not reconstruct it by prefix-matching a
/// Rust type's spelling (`umber2-johp.141`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannerStatusRecord {
    pub from: &'static str,
    pub to: &'static str,
}

/// A completed scalar macro-match milestone.
///
/// Every variant carries the required macro name. Definition allocation or
/// content identity is deliberately absent: separately allocated equal
/// definitions remain distinct engine owners but produce the same portable
/// TeX observation when invoked through the same control sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MacroRecord {
    Activation {
        control_sequence: String,
        argument_count: u8,
        token_count: u64,
    },
    Argument {
        control_sequence: String,
        parameter: u8,
        token_count: u64,
        tokens: Vec<ObservedToken>,
    },
}

/// A committed condition-stack transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConditionRecord {
    pub transition: &'static str,
    /// Stable stack identity for diagnostic context only; it is not part of
    /// the portable oracle event.
    pub identity: u64,
    /// Canonical TeX/e-TeX conditional name, e.g. `iftrue`, `ifcase`, or
    /// `unless_iftrue`.
    pub condition: String,
    /// Canonical TeX `if_limit` name at the transition seam.
    pub limit: &'static str,
    /// A branch selected at this seam, when applicable.
    pub branch: Option<String>,
}

/// An observation value captured in its semantic domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationValue {
    None,
    Integer(i64),
    Character(u32),
    Scaled(i64),
    Glue {
        width: i64,
        stretch: i64,
        stretch_order: &'static str,
        shrink: i64,
        shrink_order: &'static str,
    },
    Name(String),
    Bytes(Vec<u8>),
    Tokens(Vec<ObservedToken>),
}

/// A completed typed scanner result. Values come directly from the scanner's
/// owned semantic result, never from aggregate allocation state or a string
/// encoding that a detached observer must parse.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannerRecord {
    pub kind: &'static str,
    pub value: ObservationValue,
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
    /// Portable one-based alignment nesting owned by command state.
    pub nesting: Option<u32>,
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

/// Canonical state domain changed by a committed assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationTarget {
    Meaning,
    Catcode,
    CodeTable,
    Parameter,
    Register,
}

impl std::fmt::Display for MutationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Meaning => "meaning",
            Self::Catcode => "catcode",
            Self::CodeTable => "code_table",
            Self::Parameter => "parameter",
            Self::Register => "register",
        })
    }
}

/// A typed command-relevant assignment seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationRecord {
    pub target: MutationTarget,
    pub key: ObservationValue,
    pub value: ObservationValue,
    pub global: bool,
}

/// Canonical class of a committed externally visible effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationEffectKind {
    Input,
    Message,
    Write,
    Open,
    Close,
    Shipout,
    Terminate,
    ShowTokens,
    ShowIfs,
    ShowGroups,
}

impl std::fmt::Display for ObservationEffectKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Input => "input",
            Self::Message => "message",
            Self::Write => "write",
            Self::Open => "open",
            Self::Close => "close",
            Self::Shipout => "shipout",
            Self::Terminate => "terminate",
            Self::ShowTokens => "showtokens",
            Self::ShowIfs => "showifs",
            Self::ShowGroups => "showgroups",
        })
    }
}

/// A committed externally-visible command effect or final ordering marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectRecord {
    pub kind: ObservationEffectKind,
    pub channel: String,
    pub value: ObservationValue,
    /// Exact source selected by TeX82 §537's successful `start_input`.
    ///
    /// Identity and immutable bytes are captured together at the successful
    /// open boundary. Detached observers must not reacquire the backing later
    /// from the packed name or mutable aggregate world state. Non-input
    /// effects leave this absent.
    pub source: Option<OpenedSourceSnapshot>,
}

/// Immutable source identity and backing captured by a successful file open.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenedSourceSnapshot {
    pub id: tex_state::SourceId,
    pub bytes: Arc<[u8]>,
}

/// Detached backing context for a command-owned generated source.
///
/// This is not a semantic transition. It precedes the corresponding
/// [`InputRecord`] so buffered and live observers can resolve later command
/// provenance without reacquiring mutable command state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedSourceRecord {
    pub name: String,
    pub source: OpenedSourceSnapshot,
}

/// Finalized dimensions from one canonical packing or shipout commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryRecord {
    Hpack {
        width_sp: i64,
        height_sp: i64,
        depth_sp: i64,
        line: u32,
        source: Option<tex_state::SourceId>,
    },
    Vpack {
        width_sp: i64,
        height_sp: i64,
        depth_sp: i64,
        line: u32,
        source: Option<tex_state::SourceId>,
    },
    Shipout {
        page_width_sp: i64,
        page_height_sp: i64,
        /// TeX82 §617's `count0..count9` BOP snapshot.
        counts: [i32; 10],
        line: u32,
        source: Option<tex_state::SourceId>,
    },
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

/// Typed semantic class of a schema-v4 diagnostic report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticClass {
    RecoverableError,
    Warning,
    Fatal,
}

/// TeX82 §76 history carried by the terminal lifecycle observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticHistory {
    Spotless,
    WarningIssued,
    ErrorMessageIssued,
    FatalErrorStop,
}

/// Terminal semantic result paired with final history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticOutcome {
    Completed,
    Aborted,
}

/// Source-located report or final history/outcome for schema v4 observers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticLifecycleRecord {
    Report {
        class: DiagnosticClass,
        severity: &'static str,
        diagnostic: &'static str,
        arguments: Vec<DiagnosticArgument>,
        location: SourceLocation,
    },
    Outcome {
        history: DiagnosticHistory,
        outcome: DiagnosticOutcome,
    },
}

impl DiagnosticLifecycleRecord {
    /// Constructs the one terminal lifecycle record from TeX82's durable
    /// error-channel history. Rendered help and context are deliberately not
    /// consulted here.
    #[must_use]
    pub const fn terminal(history: tex_state::print::ErrorHistory, aborted: bool) -> Self {
        Self::Outcome {
            history: match history {
                tex_state::print::ErrorHistory::Spotless => DiagnosticHistory::Spotless,
                tex_state::print::ErrorHistory::WarningIssued => DiagnosticHistory::WarningIssued,
                tex_state::print::ErrorHistory::ErrorMessageIssued => {
                    DiagnosticHistory::ErrorMessageIssued
                }
                tex_state::print::ErrorHistory::FatalErrorStop => DiagnosticHistory::FatalErrorStop,
            },
            outcome: if aborted {
                DiagnosticOutcome::Aborted
            } else {
                DiagnosticOutcome::Completed
            },
        }
    }
}

/// One committed command-core observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandObservation {
    Command(CommandDeliveryRecord),
    Input(InputRecord),
    GeneratedSource(GeneratedSourceRecord),
    Recovery(RecoveryRecord),
    ScannerStatus(ScannerStatusRecord),
    Macro(MacroRecord),
    Condition(ConditionRecord),
    Scanner(ScannerRecord),
    TokenList(TokenListRecord),
    Alignment(AlignmentRecord),
    Mutation(MutationRecord),
    Diagnostic(DiagnosticRecord),
    DiagnosticLifecycle(DiagnosticLifecycleRecord),
    Effect(EffectRecord),
    Geometry(GeometryRecord),
}

/// Test/instrumentation sink for committed command-owned semantic records.
///
/// This interface is intentionally non-fallible. An instrumentation transport
/// must buffer or handle its own failures outside the command operation.
pub trait CommandObserver {
    fn observes_geometry(&self) -> bool {
        false
    }

    fn committed(&mut self, observation: CommandObservation);
}

/// TeX82's spelling for the dummy `undefined_control_sequence` location.
///
/// §222 places it immediately after the hash array; web2c's `tex.ch` sizes
/// `hash` through that slot and clears it with `hash[hash_base]`, whose
/// `text` §257 sets to 0. §48 makes the first 256 strings the printable forms
/// of the characters, and character 0 is unprintable, so string 0 is the
/// three characters below. (§262's `print_cs` guards the slot and prints
/// `\IMPOSSIBLE.`; §263's `sprint_cs`, which the trace mirrors, does not.)
const UNDEFINED_CONTROL_SEQUENCE_TEXT: &str = "^^@";

pub(crate) fn observed_token(
    token: TracedTokenWord,
    resolve: impl FnOnce(tex_state::interner::Symbol) -> String,
    frozen_spelling: impl FnOnce(Token) -> Option<String>,
) -> ObservedToken {
    let semantic = token.semantic_token();
    match semantic {
        // §353's `get_next` gives an active character the control sequence
        // `active_base + c`, and §365's `cur_tok` therefore stores it as
        // `cs_token_flag + cur_cs` -- a control-sequence token whose §289
        // spelling is the single character -- never as a character token with
        // command code 13. Umber keeps the character representation
        // internally, so the observation renders it TeX's way rather than
        // exposing Umber's storage (`umber2-johp.141`).
        Token::Char {
            ch,
            cat: Catcode::Active,
        } => ObservedToken::ControlSequence(ch.to_string()),
        Token::Char { ch, cat } => ObservedToken::Character {
            character: ch,
            catcode: cat,
        },
        Token::Cs(symbol) => ObservedToken::ControlSequence(resolve(symbol)),
        Token::Param(slot) => ObservedToken::Parameter(slot),
        // §222's dummy `undefined_control_sequence` sits one slot past the
        // hash array, whose `text` web2c initializes to `hash[hash_base]`'s
        // zero. §263's `sprint_cs` therefore spells it with string number 0,
        // and §48 builds string 0 as the printable form of character 0.
        Token::Frozen(_) if semantic.is_undefined_control_sequence() => {
            ObservedToken::ControlSequence(UNDEFINED_CONTROL_SEQUENCE_TEXT.to_owned())
        }
        Token::Frozen(_) if semantic.is_frozen_end_template() => ObservedToken::FrozenEndTemplate,
        Token::Frozen(_) if semantic.is_frozen_endv() => ObservedToken::FrozenEndV,
        // A frozen primitive is one of tex.web's frozen control sequences, so
        // it is observed by the spelling tex.web assigns its `text`, never by
        // an engine-local slot index a transport would have to render.
        Token::Frozen(_) => frozen_spelling(semantic)
            .map_or(ObservedToken::FrozenOther, ObservedToken::FrozenPrimitive),
    }
}

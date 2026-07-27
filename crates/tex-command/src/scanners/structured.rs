//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename termination only.  Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{FontSizeSpec, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{SourceId, TracedTokenList};

use crate::input::{
    BackupTreatment, ReplayTrace, RetirementBehavior, SharedTokenBuffer, StoredReplayReason,
    TokenBehavior, TokenPayload,
};
use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerStatus, ScannerWarning, TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::scanners::RestrictedIntegerClass;
use crate::{
    AlignmentCellTemplates, AlignmentPreamble, CommandError, CommandProcessor, InternalValue,
    processor::{meaning_text, render_the_value, string_text},
};

/// Stable pending-diagnostic identities for TeX82 §760 template recovery.
const MISSING_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0001;
const EXTRA_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0002;

/// Provenance for a completed structured scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructuredProvenance {
    /// Origin of the first non-ignored token accepted by the scan.
    pub primary: OriginId,
}

/// A balanced token list frozen through the aggregate token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBalancedText {
    pub tokens: TracedTokenList,
    pub provenance: StructuredProvenance,
}

/// The two immutable lists collected for a macro definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMacroDefinition {
    /// The raw control-sequence (or active-character) target accepted by
    /// TeX82's `prefixed_command`.  Target delivery is command-owned so the
    /// executor never has to reopen raw input between the primitive and its
    /// parameter/replacement scan.
    pub target: Symbol,
    pub parameter_text: TracedTokenList,
    pub replacement_text: TracedTokenList,
    pub provenance: StructuredProvenance,
    /// TeX82 §1215 substituted the inaccessible recovery target.
    pub missing_target: bool,
    /// TeX82's parameter-text scanner repaired an out-of-order marker.
    pub malformed_parameter: bool,
}

/// A completed TeX82 `\let` or `\futurelet` assignment.
///
/// The command processor owns every raw operand delivery, including the
/// optional equals sign and `\futurelet`'s lookahead replay. Replay receives
/// only the target and its already-resolved source meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedLetAssignment {
    pub target: Symbol,
    pub source: Option<Symbol>,
    pub meaning: Meaning,
}

/// A completed TeX82 §1224 `\\chardef` or `\\mathchardef` operand.
///
/// Command processing owns the raw target, optional equals sign, and the
/// class-restricted integer scan (§434 or §436) including its recovery. Main
/// control receives no token or input capability: it only applies the
/// assignment's effective scope and reports the recovery diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedCharacterDefinition {
    pub target: Symbol,
    /// The restricted class §1224 selects for this primitive.
    pub class: RestrictedIntegerClass,
    /// `cur_val` after §434/§436's recovery.
    pub value: i32,
    /// The unrecovered `scan_int` result, which `int_error` reports.
    pub scanned: i32,
    /// Whether recovery replaced an out-of-range value with zero.
    pub recovered: bool,
}

/// A completed TeX82 §1221 register-definition assignment.
///
/// The processor owns the raw target, its provisional `\relax` meaning,
/// optional equals sign, and bounded classical register index. Main control
/// receives only the chosen target and register selector to apply with the
/// already determined assignment scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedRegisterDefinition {
    pub target: Symbol,
    pub index: u16,
}

/// TeX82 §§1254--1261's completed `\\font` definition request.
///
/// The target, optional equals, expanded filename, and size clause are all
/// consumed while the command processor is borrowed.  Resource acquisition
/// deliberately happens later through the transient host capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontLoadRequest {
    pub target: Symbol,
    pub name: String,
    pub size: FontSizeSpec,
}

/// Immutable, command-owned identity of one pdfTeX `\\pdfximage` lookup.
///
/// This deliberately contains the selected filename and scalar scan results,
/// but neither an open file nor parsed image state.  The host supplies those
/// only after the enclosing canonical operation has suspended.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfImageRequest {
    pub name: String,
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
    pub page: i32,
    pub page_box: PdfImagePageBox,
    /// Whether source selected `page_box` rather than leaving it to the live
    /// pdfTeX page-box parameters applied by canonical main control.
    pub page_box_explicit: bool,
    pub attr: Option<TracedTokenList>,
}

/// pdfTeX's `scan_pdf_box_spec` selectors.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfImagePageBox {
    Media,
    Crop,
    Bleed,
    Trim,
    Art,
}

/// Immutable command-owned request for one pdfTeX graphics whatsit.
///
/// The balanced text has already been collected (and, where pdfTeX requires
/// it, expanded) by [`CommandProcessor`].  Replay receives neither a token
/// cursor nor a mutable input frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfGraphicsRequest {
    Literal {
        mode: tex_state::node::PdfLiteralMode,
        deferred: bool,
        text: ScannedBalancedText,
    },
    SetMatrix {
        text: ScannedBalancedText,
    },
    Save,
    Restore,
    ColorStack {
        id: i32,
        action: Option<PdfColorStackActionRequest>,
    },
    SavePosition,
}

/// The completed action word and, for setters, its expanded payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfColorStackActionRequest {
    Set(ScannedBalancedText),
    Push(ScannedBalancedText),
    Pop,
    Current,
}

/// Completed `\\pdfobj` request.  The processor owns keyword recognition and
/// every retained general-text scan; object allocation remains an application
/// concern so it occurs after the processor borrow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfObjectRequest {
    Reserve,
    Define {
        use_object: Option<i32>,
        stream: bool,
        stream_attr: Option<ScannedBalancedText>,
        file: bool,
        data: ScannedBalancedText,
    },
}

/// Completed `\\pdfxform`/`\\pdfrefxform` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFormRequest {
    Create {
        attr: Option<ScannedBalancedText>,
        resources: Option<ScannedBalancedText>,
        box_register: i32,
    },
    Reference {
        object: i32,
    },
}

/// Completed `\\pdfrefobj` operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfReferenceObjectRequest {
    pub object: i32,
}

/// Completed document-level PDF token-list assignment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDocumentFragmentRequest {
    pub kind: tex_state::PdfDocumentFragmentKind,
    pub text: ScannedBalancedText,
    pub open_action: Option<tex_state::PdfActionSpec>,
}

/// Fully scanned pdfTeX navigation whatsit.  All general text is frozen in
/// the command token store; application never reopens input to finish an
/// action or rule specification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfNavigationRequest {
    Annotation(PdfAnnotationRequest),
    StartLink(PdfStartLinkRequest),
    EndLink,
    Destination(PdfDestinationRequest),
    Thread(PdfThreadRequest),
    EndThread,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfAnnotationRequest {
    Reserve,
    Define {
        use_object: Option<i32>,
        dimensions: tex_state::PdfAnnotationDimensions,
        entries: ScannedBalancedText,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfStartLinkRequest {
    pub dimensions: tex_state::PdfAnnotationDimensions,
    pub attributes: Option<ScannedBalancedText>,
    pub action: tex_state::PdfActionSpec,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationRequest {
    pub structure: Option<u32>,
    pub identifier: tex_state::PdfActionIdentifier,
    pub kind: tex_state::node::PdfDestinationKind,
}

/// Fully scanned `\\pdfthread` or `\\pdfstartthread` marker.  The
/// dimensions deliberately retain running values: pdfTeX resolves them while
/// traversing the containing box at shipout, not while it scans the command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfThreadRequest {
    pub dimensions: tex_state::PdfAnnotationDimensions,
    pub attributes: Option<ScannedBalancedText>,
    pub identifier: tex_state::PdfActionIdentifier,
    pub running: bool,
}

/// The command-owned operand prefix of TeX82's `\setbox` assignment.
///
/// The following box command deliberately remains a normal main-control
/// delivery.  This preserves TeX82's `scan_int`/optional-equals recovery
/// before executor-owned box construction takes over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedSetBoxAssignment {
    pub index: i32,
}

/// The completed command-owned prefix of a TeX82 box construction.
///
/// `scan_spec` (§645) "scans a box specification and left brace": the optional
/// `to`/`spread` clause and then the mandatory opening brace, which it
/// consumes. Keeping both operations here means replay only receives a typed
/// construction request and never needs to reopen input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxConstruction {
    pub kind: ScannedBoxKind,
    pub packing: ScannedPackingSpec,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedBoxKind {
    HBox,
    VBox,
    VTop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedPackingSpec {
    Natural,
    Exactly(Scaled),
    Spread(Scaled),
}

/// The completed command-owned prefix of TeX82 §1099's `begin_insert_or_adjust`
/// for `\insert` and `\vadjust`.
///
/// `scan_eight_bit_int`'s range clamp and §1099's reserved-255 recovery both
/// need to write a `Universe`-routed diagnostic, so this keeps only the raw
/// scanned class number (any `i32` the integer scanner produced); the
/// mandatory opening brace is consumed here, exactly as §1099's
/// `new_save_level(insert_group); scan_left_brace` does. Replay performs the
/// bounded 0..=255 recovery and the `\insert255` rejection immediately before
/// opening the insertion group -- but only for `\insert`: `\vadjust` sets
/// `class:=255` unconditionally (`if cur_cmd=vadjust then cur_val:=255`)
/// without ever calling `scan_eight_bit_int`, so `is_vadjust` tells replay to
/// skip both diagnostics for that already-valid sentinel class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedInsertConstruction {
    pub class: i32,
    pub is_vadjust: bool,
}

/// The completed command-owned operand of a TeX82 §1073 box-shift prefix
/// (`\raise`, `\lower`, `\moveleft`, `\moveright`): `scan_box`'s own
/// `make_box` dispatch (§1084), scanned after the shift's dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedBoxShiftPayload {
    /// `scan_box`'s "A <box> was supposed to be here" recovery: the rejected
    /// command has already been backed up for ordinary replay.
    Missing,
    BoxRegister {
        index: i32,
        copy: bool,
    },
    LastBox,
    VSplit(ScannedVSplit),
    Construction(ScannedBoxConstruction),
}

/// A completed TeX82 §1073 box-shift prefix: the already-signed shift amount
/// (tex.web's `box_context`) paired with the following box operand it
/// applies to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxShift {
    pub delta: Scaled,
    pub payload: ScannedBoxShiftPayload,
}

/// The completed register operand of TeX82's `\\box` command.
///
/// `make_box(box_code)` calls `scan_int` before main control can apply the
/// resulting box-list operation. Keeping that scan here preserves the raw
/// digit delivery and any integer-scanner backup entirely in command control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxRegister {
    pub index: i32,
}

/// The complete command-owned operand of TeX82's `\\vsplit`.
///
/// The keyword's absence is preserved so replay can issue its diagnostic, but
/// both the register and dimension have already been consumed canonically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedVSplit {
    pub index: i32,
    pub height: Scaled,
    pub missing_to: bool,
}

/// A completed display diagnostic. Its text and source origin are frozen while
/// command input is borrowed, leaving replay no operand-reading work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedDisplayDiagnostic {
    pub text: String,
    pub provenance: StructuredProvenance,
}

/// The completed payload prefix of TeX82's `\\leaders` family.
///
/// A constructed box deliberately remains a construction request: its body is
/// replayed through the ordinary box lifecycle before command control scans
/// the following glue operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedLeaderPayload {
    Missing,
    BoxRegister { index: i32, copy: bool },
    Construction(ScannedBoxConstruction),
    Rule(ScannedRuleSpec),
}

/// A completed named glue-parameter assignment.
///
/// Command processing owns the optional equals sign and scalar glue scan;
/// replay receives only the parameter selector and its finished value.  The
/// `mu` flag preserves the TeX distinction between ordinary and math glue
/// parameters without exposing another input path to the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlueParameterAssignment {
    pub index: u16,
    pub value: GlueSpec,
    pub mu: bool,
}

/// A completed TeX82 `\hrule` or `\vrule` specification.
///
/// The command processor owns the expanded `width`, `height`, and `depth`
/// keyword scans and their scalar operands. Replay receives only these final
/// dimensions, so applying a rule cannot open another source-consumption path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedRuleSpec {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}

/// The character selected as the base of a completed TeX82 `\accent` scan.
///
/// A missing base is deliberately represented explicitly: TeX82 backs the
/// first non-character command up, then inserts the accent by itself.  The
/// command processor owns that backup, so the executor never needs the
/// rejected command or an input cursor to implement this case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccentBase {
    pub character: u8,
    pub provenance: StructuredProvenance,
}

/// Completed command-owned operands for TeX82's text `\accent`.
///
/// The accent code is scanned as an integer, and the following expanded
/// character (including `\char`'s integer operand) is consumed here.  If the
/// next expanded command is not a character, it has already been replayed by
/// the command processor and `base` is `None`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccent {
    pub accent: i32,
    pub accent_provenance: StructuredProvenance,
    pub base: Option<ScannedAccentBase>,
}

/// Completed command-owned group material for TeX82's `\discretionary`.
///
/// Each list is an immutable, traced token list.  The group delimiters and
/// all nested token collection stay in command control; a caller may execute
/// each completed list in an isolated restricted-horizontal episode without
/// reopening source input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDiscretionary {
    pub pre_break: ScannedBalancedText,
    pub post_break: ScannedBalancedText,
    pub replacement: ScannedBalancedText,
}

/// A completed TeX82 math-character operand (`\\mathchar` or `\\mathaccent`).
///
/// The command processor validates the canonical 15-bit range before this
/// crosses the main-control boundary.  Replay therefore has neither an
/// integer scanner nor an invalid-code recovery path for the operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathCharacter {
    pub code: u16,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 delimiter code.  `0` is the canonical missing-delimiter
/// replacement; the diagnostic and rejected-command replay remain command
/// owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathDelimiter {
    pub code: u32,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// The font-size bank addressed by a math family assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFamilySize {
    Text,
    Script,
    ScriptScript,
}

/// The completed family index prefix of `\\textfont`, `\\scriptfont`, or
/// `\\scriptscriptfont`.  Resolving the following font meaning is deliberately
/// a separate typed operation, so source delivery cannot leak into replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFamily {
    pub size: MathFamilySize,
    pub family: u8,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// Placement selected by TeX82's math script controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathScriptKind {
    Subscript,
    Superscript,
}

/// A script marker whose following math field is collected by the canonical
/// math-field episode.  Keeping the marker typed prevents replay from ever
/// reinterpreting `^` or `_` as source tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathScript {
    pub kind: MathScriptKind,
    pub provenance: StructuredProvenance,
}

/// Recovery performed while completing a math field or braced math-list
/// episode. The rejected command has already been retained for canonical
/// replay; consumers receive no source cursor or raw command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathEpisodeRecovery {
    None,
    MissingOpeningBrace,
    MissingField,
}

/// How the stomach must realize one completed math field.
///
/// TeX82 §1151's `scan_math` has exactly two outcomes: an unbraced field is
/// one already-fetched command, while a braced field is §1153's
/// ``back_input; scan_left_brace; ... push_math(math_group)`` -- the
/// mandatory brace is consumed and the subformula body is then read *live*
/// by ordinary main control, closed by §1186's `math_group` arm of
/// `handle_right_brace`. A braced field is therefore not command-owned
/// material at all, and must never be absorbed into a token list: doing so
/// backs the brace up a second time, opens an extra replay input level, and
/// swallows the closing brace that TeX delivers as a command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFieldBody {
    /// Frozen command-owned material replayed as its own episode: the single
    /// unbraced command spelling, or the empty list left behind when
    /// `scan_left_brace` recovered a missing mandatory brace.
    Replay,
    /// TeX82 §1153: `math_group`'s opening brace has been consumed and the
    /// body is live input the stomach reads through main control.
    OpenGroup,
    /// No field is available at all.
    Missing,
}

/// Immutable command-owned material for one math field.
///
/// The frozen payload is deliberately private. A consumer can only schedule
/// it through [`CommandState`](crate::CommandState)'s typed replay entry point,
/// keeping command delivery, expansion, and provenance in `tex-command`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathFieldEpisode {
    pub(crate) tokens: TracedTokenList,
    pub body: MathFieldBody,
    pub provenance: StructuredProvenance,
}

/// Immutable command-owned braced mlist episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathGroupEpisode {
    pub(crate) tokens: TracedTokenList,
    pub recovery: MathEpisodeRecovery,
    pub provenance: StructuredProvenance,
}

/// The four independently frozen branches of TeX82's `\mathchoice`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathChoiceEpisodes {
    pub display: MathGroupEpisode,
    pub text: MathGroupEpisode,
    pub script: MathGroupEpisode,
    pub scriptscript: MathGroupEpisode,
}

/// A completed script attachment. The executor selects the incomplete noad;
/// command processing has already completed the field it attaches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathScriptAttachment {
    pub kind: MathScriptKind,
    pub field: MathFieldEpisode,
}

/// The structural delimiter boundary selected by `\left`, `\right`, or
/// e-TeX's `\middle`. The corresponding delimiter scan is complete before
/// this value crosses the command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathDelimiterBoundary {
    pub kind: MathDelimiterBoundaryKind,
    pub delimiter: ScannedMathDelimiter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathDelimiterBoundaryKind {
    Left,
    Right,
    Middle,
}

/// The generalized-fraction form selected before its numerator is frozen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFractionKind {
    Over,
    Atop,
    Above,
}

/// Completed command-owned operands of a generalized fraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFraction {
    pub kind: MathFractionKind,
    pub left_delimiter: Option<ScannedMathDelimiter>,
    pub right_delimiter: Option<ScannedMathDelimiter>,
    pub thickness: Option<Scaled>,
}

/// A completed `\\mskip` or `\\mkern` operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedMathMuMaterial {
    Glue(GlueSpec),
    Kern(Scaled),
}

/// Which side receives a display equation number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquationNumberSide {
    Right,
    Left,
}

/// Immutable entry request for `\\eqno` and `\\leqno`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedEquationNumber {
    pub side: EquationNumberSide,
}

/// The noad constructor selected by a math-text primitive. Its field is
/// completed by the dedicated canonical math-field episode, not by executor
/// source reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathTextFieldKind {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
    Underline,
    Overline,
    VCenter,
}

/// Immutable request kinds delivered from command processing to canonical main
/// control for TeX82 §§691–734.  Variants that introduce an mlist episode
/// deliberately contain no source cursor: the later stomach migration can
/// consume only the already-classified request and ask the same processor for
/// the next completed field/group episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalMathRequest {
    Character(ScannedMathCharacter),
    Family(ScannedMathFamily),
    TextField(MathTextFieldKind),
    Script(ScannedMathScript),
    Limits(MathLimitKind),
    Fraction(ScannedMathFraction),
    Style(MathStyleKind),
    Choice,
    Delimiter(ScannedMathDelimiter),
    Radical(ScannedMathDelimiter),
    Accent(ScannedMathCharacter),
    MuMaterial(ScannedMathMuMaterial),
    EquationNumber(ScannedEquationNumber),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathLimitKind {
    Limits,
    NoLimits,
    DisplayLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathStyleKind {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// The canonical boundary that stopped an unbraced filename scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNameTermination {
    Group,
    Space,
    NonCharacter,
    EndOfInput,
}

/// A filename scanned from expanded command-owned input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedFileName {
    pub name: String,
    pub termination: FileNameTermination,
    pub provenance: StructuredProvenance,
}

/// Completed input-stream operation.  The command core owns every operand;
/// replay only acquires an already-registered immutable resource and mutates
/// World stream state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputStreamRequest {
    Open {
        stream: i32,
        file_name: ScannedFileName,
    },
    Close {
        stream: i32,
    },
    Read {
        stream: i32,
        target: Symbol,
        raw_catcodes: bool,
    },
}

/// A completed TeX82 §53 `\immediate` extension request.
///
/// Command control owns the recursive expanded lookahead and all operand
/// scanning.  In particular, a non-I/O lookahead has already been backed up
/// when `Continue` is returned, so replay never needs raw input access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmediateExtension {
    Continue,
    OpenOut {
        stream: i32,
        file_name: ScannedFileName,
    },
    Write {
        stream: i32,
        tokens: TracedTokenList,
    },
    CloseOut {
        stream: i32,
    },
    PdfObject(PdfObjectRequest),
    PdfForm(PdfFormRequest),
}

/// One successfully opened capability-registered input source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredInput {
    pub file_name: ScannedFileName,
    pub source: SourceId,
}

/// The typed result of TeX82's `init_col` entry lookahead.
///
/// `\omit` is consumed as that lookahead rather than backed up for the
/// selected u-template. The executor receives this semantic distinction,
/// never the command spelling that established it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentCellOpening {
    /// The selected column uses its ordinary u-template.
    Template,
    /// The selected column starts with TeX82's template-free `\omit` path.
    Omit,
}

impl CommandProcessor<'_> {
    /// Scans TeX82 §1224's complete `\\chardef` or `\\mathchardef` operand.
    ///
    /// The target remains a raw control-sequence delivery as required by
    /// `get_r_token`; the optional equals sign and numeric value use the
    /// canonical command-owned scalar scanners. §1224 spells the value scan
    /// as `char_def_code: scan_char_num` and `math_char_def_code:
    /// scan_fifteen_bit_int`, so the class-specific bound and its
    /// recover-to-zero belong to this scan and not to the assignment that
    /// consumes it.
    pub fn scan_character_definition(
        &mut self,
        class: RestrictedIntegerClass,
        provisional_global: bool,
    ) -> Result<ScannedCharacterDefinition, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::input_invariant())?;
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Mutation(crate::MutationRecord {
            target: "meaning",
            value: "relax".into(),
            key: Some(self.state.resolve(target).to_owned()),
            tokens: None,
            global: provisional_global,
        }));
        let _ = self.scan_optional_equals()?;
        let scanned = self.scan_restricted_integer(class)?;
        Ok(ScannedCharacterDefinition {
            target,
            class,
            value: scanned.value,
            scanned: scanned.scanned,
            recovered: scanned.recovered,
        })
    }

    /// Scans TeX82 §1221's complete register-definition operand.
    ///
    /// As in §1224, TeX temporarily gives the target `\relax` before the
    /// index scan. This makes a repeated target terminate its own integer
    /// scan rather than expand its previous meaning or report undefined.
    pub fn scan_register_definition(
        &mut self,
        provisional_global: bool,
    ) -> Result<ScannedRegisterDefinition, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::input_invariant())?;
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Mutation(crate::MutationRecord {
            target: "meaning",
            value: "relax".into(),
            key: Some(self.state.resolve(target).to_owned()),
            tokens: None,
            global: provisional_global,
        }));
        let _ = self.scan_optional_equals()?;
        Ok(ScannedRegisterDefinition {
            target,
            index: self.scan_eight_bit_register_index()?,
        })
    }

    /// Scans the unexpandable pdfTeX graphics whatsit family.
    ///
    /// This follows pdftex.web's `pdfliteral` through `pdfrestore` scanners:
    /// `shipout` is recognized before the literal mode, immediate literals
    /// and setters expand their balanced text now, and a shipout literal
    /// retains its unexpanded token list for traversal-time expansion.
    pub fn scan_pdf_graphics_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        use PdfColorStackActionRequest as Action;
        use PdfGraphicsRequest as Request;

        let request = match primitive {
            UnexpandablePrimitive::PdfLiteral => {
                let deferred = self.scan_keyword("shipout")?.value;
                let mode = if self.scan_keyword("direct")?.value {
                    tex_state::node::PdfLiteralMode::Direct
                } else if self.scan_keyword("page")?.value {
                    tex_state::node::PdfLiteralMode::Page
                } else {
                    tex_state::node::PdfLiteralMode::Origin
                };
                Request::Literal {
                    mode,
                    deferred,
                    text: self.scan_balanced_text(!deferred)?,
                }
            }
            UnexpandablePrimitive::PdfSetMatrix => Request::SetMatrix {
                text: self.scan_balanced_text(true)?,
            },
            UnexpandablePrimitive::PdfSave => Request::Save,
            UnexpandablePrimitive::PdfRestore => Request::Restore,
            UnexpandablePrimitive::PdfColorStack => {
                let id = self.scan_integer()?.value;
                let action = if self.scan_keyword("set")?.value {
                    Some(Action::Set(self.scan_balanced_text(true)?))
                } else if self.scan_keyword("push")?.value {
                    Some(Action::Push(self.scan_balanced_text(true)?))
                } else if self.scan_keyword("pop")?.value {
                    Some(Action::Pop)
                } else if self.scan_keyword("current")?.value {
                    Some(Action::Current)
                } else {
                    None
                };
                Request::ColorStack { id, action }
            }
            UnexpandablePrimitive::PdfSavePos => Request::SavePosition,
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    /// Scans the pdfTeX annotation/link/destination/thread family (pdftex.web
    /// 34847--35208).  `scan_alt_rule` deliberately resets all dimensions on
    /// each invocation and accepts repeated fields, with the last one winning.
    pub fn scan_pdf_navigation_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfNavigationRequest, CommandError> {
        use PdfNavigationRequest as Request;
        use tex_state::node::PdfDestinationKind;

        match primitive {
            UnexpandablePrimitive::PdfAnnot => {
                if self.scan_keyword("reserveobjnum")?.value {
                    return Ok(Request::Annotation(PdfAnnotationRequest::Reserve));
                }
                let use_object = self
                    .scan_keyword("useobjnum")?
                    .value
                    .then(|| self.scan_integer().map(|value| value.value))
                    .transpose()?;
                Ok(Request::Annotation(PdfAnnotationRequest::Define {
                    use_object,
                    dimensions: self.scan_pdf_alt_rule()?,
                    entries: self.scan_balanced_text(true)?,
                }))
            }
            UnexpandablePrimitive::PdfStartLink => Ok(Request::StartLink(PdfStartLinkRequest {
                dimensions: self.scan_pdf_alt_rule()?,
                attributes: self
                    .scan_keyword("attr")?
                    .value
                    .then(|| self.scan_balanced_text(true))
                    .transpose()?,
                action: self.scan_pdf_action()?,
            })),
            UnexpandablePrimitive::PdfEndLink => Ok(Request::EndLink),
            UnexpandablePrimitive::PdfDest => {
                let structure = if self.scan_keyword("struct")?.value {
                    Some(self.scan_pdf_positive("struct identifier", false)?)
                } else {
                    None
                };
                let identifier = self.scan_pdf_identifier("destination identifier", true)?;
                // Prefix-sharing names must be tested longest-first.
                let kind = if self.scan_keyword("xyz")?.value {
                    let zoom = if self.scan_keyword("zoom")?.value {
                        let value = self.scan_integer()?.value;
                        if value > 1_073_741_823 {
                            return Err(CommandError::PdfNavigation(
                                "pdfTeX error (ext1): number too big",
                            ));
                        }
                        Some(value)
                    } else {
                        None
                    };
                    PdfDestinationKind::Xyz { zoom }
                } else if self.scan_keyword("fitbh")?.value {
                    PdfDestinationKind::FitBoundingBoxHorizontal
                } else if self.scan_keyword("fitbv")?.value {
                    PdfDestinationKind::FitBoundingBoxVertical
                } else if self.scan_keyword("fitb")?.value {
                    PdfDestinationKind::FitBoundingBox
                } else if self.scan_keyword("fith")?.value {
                    PdfDestinationKind::FitHorizontal
                } else if self.scan_keyword("fitv")?.value {
                    PdfDestinationKind::FitVertical
                } else if self.scan_keyword("fitr")?.value {
                    PdfDestinationKind::FitRectangle(self.scan_pdf_alt_rule()?)
                } else if self.scan_keyword("fit")?.value {
                    PdfDestinationKind::Fit
                } else {
                    return Err(CommandError::PdfNavigation(
                        "pdfTeX error (ext1): destination type missing",
                    ));
                };
                Ok(Request::Destination(PdfDestinationRequest {
                    structure,
                    identifier,
                    kind,
                }))
            }
            primitive @ (UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread) => Ok(Request::Thread(PdfThreadRequest {
                dimensions: self.scan_pdf_alt_rule()?,
                attributes: self
                    .scan_keyword("attr")?
                    .value
                    .then(|| self.scan_balanced_text(true))
                    .transpose()?,
                identifier: self.scan_pdf_identifier("thread identifier", true)?,
                running: primitive == UnexpandablePrimitive::PdfStartThread,
            })),
            UnexpandablePrimitive::PdfEndThread => Ok(Request::EndThread),
            _ => Err(CommandError::input_invariant()),
        }
    }

    fn scan_pdf_alt_rule(&mut self) -> Result<tex_state::PdfAnnotationDimensions, CommandError> {
        let mut dimensions = tex_state::PdfAnnotationDimensions::RUNNING;
        loop {
            if self.scan_keyword("width")?.value {
                dimensions.width = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("height")?.value {
                dimensions.height = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("depth")?.value {
                dimensions.depth = Some(self.scan_dimension()?.value);
            } else {
                return Ok(dimensions);
            }
        }
    }

    fn scan_pdf_positive(
        &mut self,
        kind: &'static str,
        bounded_by_halfword: bool,
    ) -> Result<u32, CommandError> {
        let value = self.scan_integer()?.value;
        if value <= 0 {
            return Err(CommandError::PdfNavigation(match kind {
                "struct identifier" => "pdfTeX error (ext1): struct identifier must be positive",
                "page number" => "pdfTeX error (ext1): page number must be positive",
                _ => "pdfTeX error (ext1): num identifier must be positive",
            }));
        }
        if bounded_by_halfword && value > 1_073_741_823 {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): number too big",
            ));
        }
        Ok(value as u32)
    }

    fn scan_pdf_identifier(
        &mut self,
        kind: &'static str,
        bounded_by_halfword: bool,
    ) -> Result<tex_state::PdfActionIdentifier, CommandError> {
        if self.scan_keyword("name")?.value {
            Ok(tex_state::PdfActionIdentifier::Name(
                self.scan_balanced_text(true)?.tokens.token_list(),
            ))
        } else if self.scan_keyword("num")?.value {
            Ok(tex_state::PdfActionIdentifier::Number(
                self.scan_pdf_positive(kind, bounded_by_halfword)?,
            ))
        } else {
            Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): identifier type missing",
            ))
        }
    }

    fn scan_pdf_action(&mut self) -> Result<tex_state::PdfActionSpec, CommandError> {
        use tex_state::{PdfActionDestination, PdfActionSpec, PdfActionTarget, PdfActionWindow};
        if self.scan_keyword("user")?.value {
            return Ok(PdfActionSpec::User(
                self.scan_balanced_text(true)?.tokens.token_list(),
            ));
        }
        let goto = if self.scan_keyword("goto")?.value {
            true
        } else if self.scan_keyword("thread")?.value {
            false
        } else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): action type missing",
            ));
        };
        let file = self
            .scan_keyword("file")?
            .value
            .then(|| {
                self.scan_balanced_text(true)
                    .map(|text| text.tokens.token_list())
            })
            .transpose()?;
        let structure = if self.scan_keyword("struct")?.value {
            if !goto {
                return Err(CommandError::PdfNavigation(
                    "pdfTeX error (ext1): only GoTo action can be used with `struct'",
                ));
            }
            if file.is_some() {
                Some(tex_state::PdfActionIdentifier::Raw(
                    self.scan_balanced_text(true)?.tokens.token_list(),
                ))
            } else {
                Some(self.scan_pdf_identifier("struct identifier", false)?)
            }
        } else {
            None
        };
        let target = if self.scan_keyword("page")?.value {
            if !goto {
                return Err(CommandError::PdfNavigation(
                    "pdfTeX error (ext1): only GoTo action can be used with `page'",
                ));
            }
            let number = self.scan_pdf_positive("page number", false)?;
            PdfActionTarget::Page {
                number,
                view: self.scan_balanced_text(true)?.tokens.token_list(),
            }
        } else if self.scan_keyword("name")?.value {
            PdfActionTarget::Destination(tex_state::PdfActionIdentifier::Name(
                self.scan_balanced_text(true)?.tokens.token_list(),
            ))
        } else if self.scan_keyword("num")?.value {
            if goto && file.is_some() {
                return Err(CommandError::PdfNavigation(
                    "pdfTeX error (ext1): `goto' option cannot be used with both `file' and `num'",
                ));
            }
            PdfActionTarget::Destination(tex_state::PdfActionIdentifier::Number(
                self.scan_pdf_positive("num identifier", false)?,
            ))
        } else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): identifier type missing",
            ));
        };
        let window = if self.scan_keyword("newwindow")?.value {
            PdfActionWindow::New
        } else if self.scan_keyword("nonewwindow")?.value {
            PdfActionWindow::Same
        } else {
            PdfActionWindow::Unspecified
        };
        if window != PdfActionWindow::Unspecified && (!goto || file.is_none()) {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): `newwindow'/`nonewwindow' must be used with `goto' and `file' option",
            ));
        }
        let action = PdfActionDestination {
            file,
            structure,
            target,
            window,
        };
        Ok(if goto {
            PdfActionSpec::GoTo(action)
        } else {
            PdfActionSpec::Thread(action)
        })
    }

    /// Scans pdfTeX's raw-object, form, and document-fragment extensions.
    ///
    /// This is the command boundary corresponding to pdftex.web's extension
    /// cases: `scan_keyword` and expanded `scan_pdf_ext_toks` are complete
    /// before the executor mutates its PDF ledger or mode list.
    pub fn scan_pdf_object_request(&mut self) -> Result<PdfObjectRequest, CommandError> {
        if self.scan_keyword("reserveobjnum")?.value {
            return Ok(PdfObjectRequest::Reserve);
        }
        let use_object = self
            .scan_keyword("useobjnum")?
            .value
            .then(|| self.scan_integer().map(|value| value.value))
            .transpose()?;
        let stream = self.scan_keyword("stream")?.value;
        let stream_attr = if stream && self.scan_keyword("attr")?.value {
            Some(self.scan_balanced_text(true)?)
        } else {
            None
        };
        let file = self.scan_keyword("file")?.value;
        let data = self.scan_balanced_text(true)?;
        Ok(PdfObjectRequest::Define {
            use_object,
            stream,
            stream_attr,
            file,
            data,
        })
    }

    pub fn scan_pdf_form_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfFormRequest, CommandError> {
        if primitive == UnexpandablePrimitive::PdfRefXForm {
            return Ok(PdfFormRequest::Reference {
                object: self.scan_integer()?.value,
            });
        }
        let attr = self
            .scan_keyword("attr")?
            .value
            .then(|| self.scan_balanced_text(true))
            .transpose()?;
        let resources = self
            .scan_keyword("resources")?
            .value
            .then(|| self.scan_balanced_text(true))
            .transpose()?;
        Ok(PdfFormRequest::Create {
            attr,
            resources,
            box_register: self.scan_integer()?.value,
        })
    }

    pub fn scan_pdf_reference_object_request(
        &mut self,
    ) -> Result<PdfReferenceObjectRequest, CommandError> {
        Ok(PdfReferenceObjectRequest {
            object: self.scan_integer()?.value,
        })
    }

    pub fn scan_pdf_document_fragment_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfDocumentFragmentRequest, CommandError> {
        use tex_state::PdfDocumentFragmentKind as Kind;
        let kind = match primitive {
            UnexpandablePrimitive::PdfInfo => Kind::Info,
            UnexpandablePrimitive::PdfCatalog => Kind::Catalog,
            UnexpandablePrimitive::PdfNames => Kind::Names,
            UnexpandablePrimitive::PdfTrailer => Kind::Trailer,
            UnexpandablePrimitive::PdfTrailerId => Kind::TrailerId,
            _ => return Err(CommandError::input_invariant()),
        };
        let text = self.scan_balanced_text(true)?;
        let open_action = if primitive == UnexpandablePrimitive::PdfCatalog
            && self.scan_keyword("openaction")?.value
        {
            Some(self.scan_pdf_action()?)
        } else {
            None
        };
        Ok(PdfDocumentFragmentRequest {
            kind,
            text,
            open_action,
        })
    }

    /// Completes one TeX82 §1151 `scan_math` field.
    ///
    /// An unbraced field is one expanded command spelling, frozen here.
    /// Active characters are resolved by `get_x_token` before freezing, so
    /// their replay cannot reopen source input or bypass command provenance.
    /// A braced field is §1153: the mandatory brace is consumed by
    /// `scan_left_brace` and nothing is absorbed, because `push_math`'s
    /// `math_group` reads its body from live input.
    pub fn scan_math_field_episode(&mut self) -> Result<MathFieldEpisode, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(MathFieldEpisode {
                    tokens: self.state.finish_traced_token_list(&[]),
                    body: MathFieldBody::Missing,
                    provenance: StructuredProvenance {
                        primary: OriginId::UNKNOWN,
                    },
                });
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    // TeX82 §1153 verbatim: `back_input; scan_left_brace;
                    // ... push_math(math_group)`. The brace is re-read, not
                    // re-consumed from `command`, because `scan_left_brace`
                    // is what TeX runs here and its skipped spaces and
                    // recovery are observable.
                    self.back_input(command)?;
                    return Ok(match self.scan_left_brace(true) {
                        Ok(opening) => MathFieldEpisode {
                            tokens: self.state.finish_traced_token_list(&[]),
                            body: MathFieldBody::OpenGroup,
                            provenance: StructuredProvenance {
                                primary: opening.origin(),
                            },
                        },
                        Err(CommandError::InputInvariant(_)) => MathFieldEpisode {
                            tokens: self.state.finish_traced_token_list(&[]),
                            body: MathFieldBody::Replay,
                            provenance: StructuredProvenance {
                                primary: OriginId::UNKNOWN,
                            },
                        },
                        Err(error) => return Err(error),
                    });
                }
                _ => {
                    let provenance = StructuredProvenance {
                        primary: command.origin(),
                    };
                    return Ok(MathFieldEpisode {
                        tokens: self.state.finish_traced_token_list(&[command.spelling()]),
                        body: MathFieldBody::Replay,
                        provenance,
                    });
                }
            }
        }
    }

    /// Completes one required braced math list. A missing opening brace is
    /// recovered here after `scan_left_brace` has backed the rejected command
    /// up, matching TeX's recovery ownership without exposing it to replay.
    ///
    /// This absorbing form serves only `\mathchoice` (TeX82 §1172), which
    /// needs all four branches before any is built. §1151's `scan_math`
    /// braced field does *not* use it: see [`MathFieldBody::OpenGroup`].
    pub fn scan_math_group_episode(&mut self) -> Result<MathGroupEpisode, CommandError> {
        match self.scan_left_brace(true) {
            Ok(opening) => {
                let primary = opening.origin();
                self.back_input(opening)?;
                let scanned = self.scan_toks(ScanToksMode::General { expanded: false })?;
                Ok(MathGroupEpisode {
                    tokens: scanned.replacement_text,
                    recovery: MathEpisodeRecovery::None,
                    provenance: StructuredProvenance { primary },
                })
            }
            Err(CommandError::InputInvariant(_)) => Ok(MathGroupEpisode {
                tokens: self.state.finish_traced_token_list(&[]),
                recovery: MathEpisodeRecovery::MissingOpeningBrace,
                provenance: StructuredProvenance {
                    primary: OriginId::UNKNOWN,
                },
            }),
            Err(error) => Err(error),
        }
    }

    /// Completes all four required `\mathchoice` groups before replay starts
    /// constructing any branch.
    pub fn scan_math_choice_episodes(&mut self) -> Result<MathChoiceEpisodes, CommandError> {
        Ok(MathChoiceEpisodes {
            display: self.scan_math_group_episode()?,
            text: self.scan_math_group_episode()?,
            script: self.scan_math_group_episode()?,
            scriptscript: self.scan_math_group_episode()?,
        })
    }

    /// Completes a script marker and its field in one command-owned episode.
    pub fn scan_math_script_attachment(
        &mut self,
        kind: MathScriptKind,
    ) -> Result<MathScriptAttachment, CommandError> {
        Ok(MathScriptAttachment {
            kind,
            field: self.scan_math_field_episode()?,
        })
    }

    /// Completes the delimiter immediately following a structural math
    /// boundary (`\left`, `\right`, or `\middle`).
    pub fn scan_math_delimiter_boundary(
        &mut self,
        kind: MathDelimiterBoundaryKind,
    ) -> Result<MathDelimiterBoundary, CommandError> {
        Ok(MathDelimiterBoundary {
            kind,
            delimiter: self.scan_math_delimiter()?,
        })
    }

    /// Scans TeX82 §436's `scan_fifteen_bit_int` math-character number.
    pub fn scan_math_character(&mut self) -> Result<ScannedMathCharacter, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::FifteenBit)?;
        Ok(ScannedMathCharacter {
            code: scanned.value as u16,
            recovered: scanned.recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Scans TeX82 §437's `scan_twenty_seven_bit_int` delimiter number.
    pub fn scan_math_delimiter(&mut self) -> Result<ScannedMathDelimiter, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::TwentySevenBit)?;
        Ok(ScannedMathDelimiter {
            code: scanned.value as u32,
            recovered: scanned.recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Scans TeX82 §435's `scan_four_bit_int` family index, the prefix common
    /// to the three math-font assignment primitives (§1234's `def_family`).
    /// The later font-meaning scan is intentionally not part of this request.
    pub fn scan_math_family(
        &mut self,
        size: MathFamilySize,
    ) -> Result<ScannedMathFamily, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::FourBit)?;
        Ok(ScannedMathFamily {
            size,
            family: scanned.value as u8,
            recovered: scanned.recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Collects the command-owned scalar prefix of TeX82's generalized
    /// fraction forms. Numerator/denominator mlist construction stays in the
    /// executor and is deliberately absent from this scanner boundary.
    pub fn scan_math_fraction(
        &mut self,
        kind: MathFractionKind,
        with_delimiters: bool,
    ) -> Result<ScannedMathFraction, CommandError> {
        let (left_delimiter, right_delimiter) = if with_delimiters {
            (
                Some(self.scan_math_delimiter()?),
                Some(self.scan_math_delimiter()?),
            )
        } else {
            (None, None)
        };
        let thickness = match kind {
            MathFractionKind::Above => Some(self.scan_dimension()?.value),
            MathFractionKind::Atop => Some(Scaled::from_raw(0)),
            MathFractionKind::Over => None,
        };
        Ok(ScannedMathFraction {
            kind,
            left_delimiter,
            right_delimiter,
            thickness,
        })
    }

    /// Scans the `mu`-unit material operands used only in math mode.
    pub fn scan_math_mu_material(
        &mut self,
        glue: bool,
    ) -> Result<ScannedMathMuMaterial, CommandError> {
        if glue {
            Ok(ScannedMathMuMaterial::Glue(self.scan_glue(true)?.value))
        } else {
            Ok(ScannedMathMuMaterial::Kern(self.scan_mu_dimension()?.value))
        }
    }

    /// Completes the scalar portion of one math-mode command.  Any following
    /// field or braced list is intentionally represented by a later opaque
    /// replay episode, so the stomach never receives a source cursor.
    ///
    /// The table is keyed on the delivered [`Meaning`], not on
    /// [`UnexpandablePrimitive`], because TeX82's math vocabulary is not
    /// exclusively primitive-shaped: §1154's `mmode+math_given` case carries
    /// its math code in the delivered command itself (a `\\mathchardef`
    /// target, §1224), exactly as `mmode+math_char_num` carries it in the
    /// integer `\\mathchar` scans. Keying on the primitive alone silently
    /// excluded `math_given` from the whole mmode table.
    pub fn scan_canonical_math_request(
        &mut self,
        command: &crate::CurrentCommand,
    ) -> Result<Option<CanonicalMathRequest>, CommandError> {
        use CanonicalMathRequest as Request;
        use MathTextFieldKind as Field;
        // TeX82 §1154's `mmode+math_given: set_math_char(cur_chr)`. Unlike
        // `mmode+math_char_num`, which reaches the same `set_math_char`
        // (§1155) through §436's `scan_fifteen_bit_int`, the code is already
        // complete in the delivered command, so nothing is scanned and the
        // math char's provenance is the delivering token's own origin.
        if let Meaning::MathCharGiven(code) = command.meaning() {
            return Ok(Some(Request::Character(ScannedMathCharacter {
                code,
                recovered: false,
                provenance: StructuredProvenance {
                    primary: command.origin(),
                },
            })));
        }
        let Meaning::UnexpandablePrimitive(primitive) = command.meaning() else {
            return Ok(None);
        };
        let request = match primitive {
            UnexpandablePrimitive::MathChar => Request::Character(self.scan_math_character()?),
            UnexpandablePrimitive::Delimiter => Request::Delimiter(self.scan_math_delimiter()?),
            UnexpandablePrimitive::MathOrd => Request::TextField(Field::Ord),
            UnexpandablePrimitive::MathOp => Request::TextField(Field::Op),
            UnexpandablePrimitive::MathBin => Request::TextField(Field::Bin),
            UnexpandablePrimitive::MathRel => Request::TextField(Field::Rel),
            UnexpandablePrimitive::MathOpen => Request::TextField(Field::Open),
            UnexpandablePrimitive::MathClose => Request::TextField(Field::Close),
            UnexpandablePrimitive::MathPunct => Request::TextField(Field::Punct),
            UnexpandablePrimitive::MathInner => Request::TextField(Field::Inner),
            UnexpandablePrimitive::Underline => Request::TextField(Field::Underline),
            UnexpandablePrimitive::Overline => Request::TextField(Field::Overline),
            UnexpandablePrimitive::VCenter => Request::TextField(Field::VCenter),
            UnexpandablePrimitive::Limits => Request::Limits(MathLimitKind::Limits),
            UnexpandablePrimitive::NoLimits => Request::Limits(MathLimitKind::NoLimits),
            UnexpandablePrimitive::DisplayLimits => Request::Limits(MathLimitKind::DisplayLimits),
            UnexpandablePrimitive::Over => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, false)?)
            }
            UnexpandablePrimitive::Atop => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, false)?)
            }
            UnexpandablePrimitive::Above => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, false)?)
            }
            UnexpandablePrimitive::OverWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, true)?)
            }
            UnexpandablePrimitive::AtopWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, true)?)
            }
            UnexpandablePrimitive::AboveWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, true)?)
            }
            UnexpandablePrimitive::Radical => Request::Radical(self.scan_math_delimiter()?),
            UnexpandablePrimitive::Accent | UnexpandablePrimitive::MathAccent => {
                Request::Accent(self.scan_math_character()?)
            }
            UnexpandablePrimitive::MSkip => Request::MuMaterial(self.scan_math_mu_material(true)?),
            UnexpandablePrimitive::MKern => Request::MuMaterial(self.scan_math_mu_material(false)?),
            UnexpandablePrimitive::MathChoice => Request::Choice,
            UnexpandablePrimitive::DisplayStyle => Request::Style(MathStyleKind::Display),
            UnexpandablePrimitive::TextStyle => Request::Style(MathStyleKind::Text),
            UnexpandablePrimitive::ScriptStyle => Request::Style(MathStyleKind::Script),
            UnexpandablePrimitive::ScriptScriptStyle => Request::Style(MathStyleKind::ScriptScript),
            UnexpandablePrimitive::EqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Right,
            }),
            UnexpandablePrimitive::LeftEqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Left,
            }),
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    /// Scans TeX82's `\\openin`, `\\closein`, `\\read`, and e-TeX's
    /// `\\readline` operands without exposing a raw delivery to replay.
    pub fn scan_input_stream_request(
        &mut self,
        primitive: tex_state::meaning::UnexpandablePrimitive,
    ) -> Result<InputStreamRequest, CommandError> {
        use tex_state::meaning::UnexpandablePrimitive;

        let stream = self.scan_integer()?.value;
        match primitive {
            UnexpandablePrimitive::OpenIn => {
                let _ = self.scan_optional_equals()?;
                Ok(InputStreamRequest::Open {
                    stream,
                    file_name: self.scan_file_name()?,
                })
            }
            UnexpandablePrimitive::CloseIn => Ok(InputStreamRequest::Close { stream }),
            UnexpandablePrimitive::Read | UnexpandablePrimitive::ReadLine => {
                if !self.scan_keyword("to")?.value {
                    return Err(CommandError::input_invariant());
                }
                let target = self
                    .next_non_space_raw()?
                    .and_then(|command| command.control_sequence())
                    .ok_or(CommandError::input_invariant())?;
                Ok(InputStreamRequest::Read {
                    stream,
                    target,
                    raw_catcodes: primitive == UnexpandablePrimitive::ReadLine,
                })
            }
            _ => Err(CommandError::input_invariant()),
        }
    }

    /// Scans a complete ordinary font definition without retaining a raw
    /// command, cursor, or host capability.
    ///
    /// TeX82 §1257's `new_font` runs `define(u,set_font,null_font)` on the
    /// `get_r_token` target *before* `scan_optional_equals` and
    /// `scan_file_name`, exactly as §1224 gives a `\\chardef` target a
    /// provisional `\\relax`. The identifier therefore already denotes the
    /// null font while the file name and `at`/`scaled` size are scanned, and
    /// §1257's `common_ending: equiv(u):=f` later overwrites that equivalent
    /// in place rather than through a second `eq_define`.
    pub fn scan_font_definition(
        &mut self,
        provisional_global: bool,
    ) -> Result<FontLoadRequest, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::input_invariant())?;
        self.state.set_provisional_meaning(
            target,
            Meaning::Font(tex_state::font::NULL_FONT),
            provisional_global,
        );
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Mutation(crate::MutationRecord {
            target: "meaning",
            value: "set_font".into(),
            key: Some(self.state.resolve(target).to_owned()),
            tokens: None,
            global: provisional_global,
        }));
        let _ = self.scan_optional_equals()?;
        let file_name = self.scan_file_name()?;
        let size = if self.scan_keyword("at")?.value {
            let requested = self.scan_dimension()?.value;
            FontSizeSpec::At(
                if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
                    requested
                } else {
                    Scaled::from_raw(10 * Scaled::UNITY)
                },
            )
        } else if self.scan_keyword("scaled")?.value {
            let requested = self.scan_integer()?.value;
            FontSizeSpec::Scale(if (1..=32_768).contains(&requested) {
                requested
            } else {
                1000
            })
        } else {
            FontSizeSpec::Design
        };
        Ok(FontLoadRequest {
            target,
            name: file_name.name,
            size,
        })
    }

    /// Scans pdfTeX's `scan_image` request prefix.
    ///
    /// The ordering follows pdfTeX 1.40.27's `scan_image`: a repeated rule
    /// specification, optional `attr` general text, optional `page`, then one
    /// page-box selector and the filename.  Resource acquisition is expressly
    /// outside this scanner.
    pub fn scan_pdf_image_request(&mut self) -> Result<PdfImageRequest, CommandError> {
        let mut width = None;
        let mut height = None;
        let mut depth = None;
        loop {
            if self.scan_keyword("width")?.value {
                width = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("height")?.value {
                height = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("depth")?.value {
                depth = Some(self.scan_dimension()?.value);
            } else {
                break;
            }
        }
        let attr = if self.scan_keyword("attr")?.value {
            Some(self.scan_balanced_text(true)?.tokens)
        } else {
            None
        };
        let page = if self.scan_keyword("page")?.value {
            self.scan_integer()?.value
        } else {
            1
        };
        let page_box = if self.scan_keyword("mediabox")?.value {
            Some(PdfImagePageBox::Media)
        } else if self.scan_keyword("cropbox")?.value {
            Some(PdfImagePageBox::Crop)
        } else if self.scan_keyword("bleedbox")?.value {
            Some(PdfImagePageBox::Bleed)
        } else if self.scan_keyword("trimbox")?.value {
            Some(PdfImagePageBox::Trim)
        } else if self.scan_keyword("artbox")?.value {
            Some(PdfImagePageBox::Art)
        } else {
            None
        };
        let name = self.scan_file_name()?.name;
        Ok(PdfImageRequest {
            name,
            width,
            height,
            depth,
            page,
            // pdfTeX's default `pdf_pagebox` is configured outside the
            // scanner; Crop is the engine's effective no-parameter default.
            page_box_explicit: page_box.is_some(),
            page_box: page_box.unwrap_or(PdfImagePageBox::Crop),
            attr,
        })
    }
    /// Scans TeX82 §1124's text-accent operands through command-owned input.
    ///
    /// Assignment execution between the accent code and base character is an
    /// executor lifecycle concern; this bounded scanner intentionally owns
    /// only expanded delivery, `\char`'s scalar operand, and the canonical
    /// replay of a non-character lookahead.
    pub fn scan_accent(&mut self) -> Result<ScannedAccent, CommandError> {
        let accent = self.scan_integer()?;
        let base = loop {
            let Some(command) = self.get_x_token()? else {
                break None;
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }
                | Meaning::CharGiven(ch)
                | Meaning::CharToken {
                    ch,
                    cat: Catcode::Active,
                } => {
                    let character =
                        u8::try_from(ch as u32).map_err(|_| CommandError::input_invariant())?;
                    break Some(ScannedAccentBase {
                        character,
                        provenance: StructuredProvenance {
                            primary: command.origin(),
                        },
                    });
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                    let character = u8::try_from(self.scan_integer()?.value)
                        .map_err(|_| CommandError::input_invariant())?;
                    break Some(ScannedAccentBase {
                        character,
                        provenance: StructuredProvenance {
                            primary: command.origin(),
                        },
                    });
                }
                _ => {
                    self.back_input(command)?;
                    break None;
                }
            }
        };
        Ok(ScannedAccent {
            accent: accent.value,
            accent_provenance: StructuredProvenance {
                primary: accent.provenance.primary,
            },
            base,
        })
    }

    /// Collects all three TeX82 `\discretionary` groups as immutable traced
    /// material. Their eventual restricted-horizontal execution is separate
    /// from raw source collection and cannot access an `InputStack`.
    pub fn scan_discretionary(&mut self) -> Result<ScannedDiscretionary, CommandError> {
        Ok(ScannedDiscretionary {
            pre_break: self.scan_balanced_text(false)?,
            post_break: self.scan_balanced_text(false)?,
            replacement: self.scan_balanced_text(false)?,
        })
    }

    /// Scans TeX82 §1350's `new_write_whatsit` stream number for a
    /// `write_node_size` extension.
    ///
    /// `new_write_whatsit` normalizes the scanned number *before* it reaches
    /// `write_stream(tail)`:
    ///
    /// ```text
    /// else begin scan_int;
    ///   if cur_val<0 then cur_val:=17
    ///   else if cur_val>15 then cur_val:=16;
    ///   end;
    /// write_stream(tail):=cur_val;
    /// ```
    ///
    /// §1342 explains the two extra slots: `write_open[16]` stands for every
    /// stream number above 15 and `write_open[17]` for every negative one, so
    /// the recorded stream is always in `0..=17`. This is deliberately *not*
    /// §433's `scan_four_bit_int`, which `new_write_whatsit` uses only for
    /// the `open_node_size` case (`\openout`) and which reports "Bad number"
    /// and recovers as stream zero instead.
    pub fn scan_write_stream(&mut self) -> Result<i32, CommandError> {
        let value = self.scan_integer()?.value;
        Ok(if value < 0 {
            17
        } else if value > 15 {
            16
        } else {
            value
        })
    }

    /// Scans TeX82 §53's one-token `\immediate` extension execution.
    ///
    /// `do_extension` calls `get_x_token`, executes only `openout`, `write`,
    /// and `closeout`, and backs every other expanded command up for ordinary
    /// main control.  The integer, optional-equals, filename, and write-text
    /// scans remain in this command-owned episode.
    pub fn scan_immediate_extension(&mut self) -> Result<ImmediateExtension, CommandError> {
        let command = loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut) => {
                let stream = self.scan_integer()?.value;
                let _ = self.scan_optional_equals()?;
                let file_name = self.scan_file_name()?;
                Ok(ImmediateExtension::OpenOut { stream, file_name })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
                let stream = self.scan_write_stream()?;
                // TeX82 §53 first saves write text without expansion, then
                // `write_out` replays it under an outer `\\endwrite` stopper
                // and scans the resulting expanded text. Keep both episodes
                // command-owned; replay receives only the frozen result.
                let tokens = self.scan_immediate_write_text()?;
                let tokens = self.expand_write_text(tokens)?;
                Ok(ImmediateExtension::Write { stream, tokens })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
                let stream = self.scan_integer()?.value;
                Ok(ImmediateExtension::CloseOut { stream })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject) => Ok(
                ImmediateExtension::PdfObject(self.scan_pdf_object_request()?),
            ),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXForm) => {
                Ok(ImmediateExtension::PdfForm(
                    self.scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)?,
                ))
            }
            _ => {
                self.back_input(command)?;
                Ok(ImmediateExtension::Continue)
            }
        }
    }

    fn expand_write_text(
        &mut self,
        tokens: TracedTokenList,
    ) -> Result<TracedTokenList, CommandError> {
        let endwrite = self
            .state
            .primitive_token("endwrite")
            .ok_or(CommandError::input_invariant())?;
        let right_brace = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let left_brace = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };

        // The bottom stopper delivers the synthetic closing brace followed
        // by frozen outer `\\endwrite`; the write list and opening brace sit
        // above it exactly as TeX82's three `ins_list` calls do.
        self.push_write_recovery(vec![right_brace, endwrite], right_brace);
        let write_level = self.command.push_token_level(
            TokenPayload::Stored {
                tokens: tokens.token_list(),
                origins: tokens.origin_list(),
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::Write),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        {
            // §53 names this artificial replay as a write input lifetime.
            // Keep that observer classification at the scanner/control seam;
            // the raw delivery loop must not inspect the level's trace.
            self.observe_immediate_write_retirement(write_level);
            self.observe(crate::CommandObservation::Input(crate::InputRecord {
                transition: crate::InputTransition::Push,
                reason: crate::InputReason::Write,
                level: write_level.0,
                position: 0,
            }));
        }
        self.push_write_recovery(vec![left_brace], left_brace);

        let expanded = self.scan_balanced_text(true)?.tokens;
        let stopper = self.get_token()?.ok_or(CommandError::input_invariant())?;
        if stopper.spelling().semantic_token() != endwrite {
            return Err(CommandError::input_invariant());
        }
        self.retire_last_delivery_level()?;
        Ok(expanded)
    }

    /// Freezes the ordinary `\\write` text after TeX82's `scan_int`
    /// terminator has been validated and backed up. Unlike general-text
    /// callers, §53's `new_write_whatsit` enters the absorbing collection at
    /// that already-backed-up brace.
    fn scan_immediate_write_text(&mut self) -> Result<TracedTokenList, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralAfterOpening {
            expanded: false,
            primary: OriginId::UNKNOWN,
        })?;
        Ok(scanned.replacement_text)
    }

    fn push_write_recovery(&mut self, tokens: Vec<Token>, observed: Token) {
        let level = self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(
                tokens
                    .into_iter()
                    .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                    .collect::<Vec<_>>(),
            )),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(crate::CommandObservation::Input(crate::InputRecord {
                transition: crate::InputTransition::Recovery,
                reason: crate::InputReason::Recovery,
                level: level.0,
                position: 0,
            }));
            self.observe(crate::CommandObservation::Recovery(crate::RecoveryRecord {
                kind: crate::RecoveryKind::InsertedToken,
                tokens: vec![
                    self.observed_token(TracedTokenWord::pack(observed, OriginId::UNKNOWN)),
                ],
            }));
        }
    }

    /// Scans the register number and optional equals sign of `\setbox`.
    ///
    /// TeX.web's `prefixed_command` dispatches `set_box` to `scan_int` then
    /// `scan_optional_equals`; the latter must retain its ordinary backup
    /// transition when the equals sign is present.
    pub fn scan_setbox_assignment(&mut self) -> Result<ScannedSetBoxAssignment, CommandError> {
        let index = self.scan_integer()?.value;
        let _ = self.scan_optional_equals()?;
        Ok(ScannedSetBoxAssignment { index })
    }

    /// Scans the register operand of TeX82 §1079's `make_box(box_code)`.
    pub fn scan_box_register(&mut self) -> Result<ScannedBoxRegister, CommandError> {
        Ok(ScannedBoxRegister {
            index: self.scan_integer()?.value,
        })
    }

    /// Scans TeX82 §1082's `\\vsplit <number> to <dimen>` prefix.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let index = self.scan_integer()?.value;
        let missing_to = !self.scan_keyword("to")?.value;
        let height = self.scan_dimension()?.value;
        Ok(ScannedVSplit {
            index,
            height,
            missing_to,
        })
    }

    /// TeX82 §46's raw `\\show` operand scan.
    pub fn scan_show(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let command = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let token = command.spelling().semantic_token();
        let text = match token {
            Token::Cs(_)
            | Token::Char {
                cat: Catcode::Active,
                ..
            } => format!(
                "\n> {}={}.\n",
                string_text(&self.state, token),
                meaning_text(&self.state, &command)
            ),
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                format!("\n> {}.\n", meaning_text(&self.state, &command))
            }
        };
        Ok(ScannedDisplayDiagnostic {
            text,
            provenance: StructuredProvenance {
                primary: command.origin(),
            },
        })
    }

    /// TeX82 §46's `\\showthe` internal-value scan.
    pub fn scan_showthe(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let value = self
            .scan_internal_value()?
            .ok_or(CommandError::input_invariant())?;
        let text = match value.value {
            value @ (InternalValue::Integer(_)
            | InternalValue::Dimension(_)
            | InternalValue::Glue(_)
            | InternalValue::MuGlue(_)) => {
                render_the_value(value).expect("non-token values render")
            }
            InternalValue::Font(symbol) => string_text(&self.state, Token::Cs(symbol)),
            InternalValue::Tokens { tokens, .. } => self
                .state
                .tokens(tokens)
                .iter()
                .copied()
                .map(|token| string_text(&self.state, token))
                .collect(),
        };
        Ok(ScannedDisplayDiagnostic {
            text: format!("\n> {text}.\n"),
            provenance: StructuredProvenance {
                primary: value.provenance.primary,
            },
        })
    }

    /// TeX82 §46's expanded box-register scan for `\\showbox`.
    pub fn scan_showbox(&mut self) -> Result<(i32, StructuredProvenance), CommandError> {
        let index = self.scan_integer()?;
        Ok((
            index.value,
            StructuredProvenance {
                primary: index.provenance.primary,
            },
        ))
    }

    /// Scans the payload prefix of TeX82 §1090's leader commands.
    pub fn scan_leader_payload(&mut self) -> Result<ScannedLeaderPayload, CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(ScannedLeaderPayload::Missing);
        };
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box) => {
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.scan_integer()?.value,
                    copy: false,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.scan_integer()?.value,
                    copy: true,
                })
            }
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HBox
                | UnexpandablePrimitive::VBox
                | UnexpandablePrimitive::VTop),
            ) => Ok(ScannedLeaderPayload::Construction(
                self.scan_box_construction(primitive)?,
            )),
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
            ) => Ok(ScannedLeaderPayload::Rule(self.scan_rule_spec(primitive)?)),
            _ => {
                self.back_input(command)?;
                Ok(ScannedLeaderPayload::Missing)
            }
        }
    }

    /// Scans TeX82's named glue-parameter assignment operand.
    ///
    /// This follows `scan_optional_equals` and `scan_glue`, retaining their
    /// canonical backup and alignment-delivery transitions before replay
    /// applies the aggregate mutation.
    pub fn scan_glue_parameter_assignment(
        &mut self,
        index: u16,
        mu: bool,
    ) -> Result<ScannedGlueParameterAssignment, CommandError> {
        let _ = self.scan_optional_equals()?;
        let value = self.scan_glue(mu)?.value;
        Ok(ScannedGlueParameterAssignment { index, value, mu })
    }

    /// Scans the complete expanded specification of a TeX82 rule.
    ///
    /// This is TeX.web's `scan_rule_spec`: keyword recognition and dimension
    /// scanning stay in command control, including failed-keyword replay.
    pub fn scan_rule_spec(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedRuleSpec, CommandError> {
        let default_rule = Scaled::from_raw(26_214);
        let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
            (Some(default_rule), None, None)
        } else {
            (None, Some(default_rule), Some(Scaled::from_raw(0)))
        };
        loop {
            if self.scan_keyword("width")?.value {
                width = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("height")?.value {
                height = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("depth")?.value {
                depth = Some(self.scan_dimension()?.value);
            } else {
                break;
            }
        }
        Ok(ScannedRuleSpec {
            width,
            height,
            depth,
        })
    }

    /// Reads a box body's mandatory opening brace: TeX82 §403's
    /// `scan_left_brace`, which every box-opening site reaches through
    /// §645's `scan_spec` (`new_save_level(c); scan_left_brace`) or §1099's
    /// `begin_insert_or_adjust` (`new_save_level(insert_group);
    /// scan_left_brace`).
    ///
    /// §403 *consumes* that brace; the save level it belongs to was already
    /// opened by the caller, so nothing is delivered to main control on its
    /// behalf. The brace is therefore never backed up here: replay opens the
    /// group when it receives the construction, exactly as `new_save_level`
    /// runs before `scan_left_brace`.
    ///
    /// When the mandatory brace is absent, §403 recovers by backing up the
    /// offending command and behaving as though a `{` had been read.
    /// `scan_left_brace` has already performed that backup, so this returns
    /// on the same footing: the brace is accounted for either way.
    fn scan_box_group_opening(&mut self) -> Result<(), CommandError> {
        match self.scan_left_brace(true) {
            Ok(_) => Ok(()),
            Err(CommandError::InputInvariant(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Scans the `to`/`spread` clause of TeX82 §645's `scan_spec`.
    ///
    /// §645 is the single routine every specification-taking group opener
    /// runs: `if scan_keyword("to") then spec_code:=exactly else if
    /// scan_keyword("spread") then spec_code:=additional else begin
    /// spec_code:=additional; cur_val:=0; goto found end; scan_normal_dimen`.
    /// An absent clause is `spread 0pt`, which packs at natural size.
    ///
    /// Both call sites in this crate -- §1083's box construction and §774's
    /// `init_align` -- must share it: `\halign to <dimen>{`, `\halign spread
    /// <dimen>{`, and `\hbox to <dimen>{` are the same scan, and a site that
    /// skipped straight to §403's mandatory left brace would reject the `t`
    /// of `to` as a missing brace.
    fn scan_spec_packing(&mut self) -> Result<ScannedPackingSpec, CommandError> {
        if self.scan_keyword("to")?.value {
            Ok(ScannedPackingSpec::Exactly(self.scan_dimension()?.value))
        } else if self.scan_keyword("spread")?.value {
            Ok(ScannedPackingSpec::Spread(self.scan_dimension()?.value))
        } else {
            Ok(ScannedPackingSpec::Natural)
        }
    }

    /// Scans TeX82 §1083's complete box-construction prefix: §645's
    /// `scan_spec`, whose optional `to`/`spread` clause and mandatory left
    /// brace are both consumed before replay enters the box group.
    pub fn scan_box_construction(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxConstruction, CommandError> {
        let kind = match primitive {
            UnexpandablePrimitive::HBox => ScannedBoxKind::HBox,
            UnexpandablePrimitive::VBox => ScannedBoxKind::VBox,
            UnexpandablePrimitive::VTop => ScannedBoxKind::VTop,
            _ => return Err(CommandError::input_invariant()),
        };
        let packing = self.scan_spec_packing()?;
        self.scan_box_group_opening()?;
        Ok(ScannedBoxConstruction { kind, packing })
    }

    /// Scans TeX82 §1099's `begin_insert_or_adjust` prefix, the one routine
    /// both `\insert` and `\vadjust` enter: `if cur_cmd=vadjust then
    /// cur_val:=255 else scan_eight_bit_int`, then
    /// `new_save_level(insert_group); scan_left_brace`.
    ///
    /// The raw integer is carried through unvalidated because
    /// `scan_eight_bit_int`'s range clamp and the reserved-255 rejection both
    /// need a `Universe` diagnostic sink; `\vadjust` skips the scan entirely,
    /// so its fixed 255 is never subject to either.
    pub fn scan_insert_construction(
        &mut self,
        is_vadjust: bool,
    ) -> Result<ScannedInsertConstruction, CommandError> {
        let class = if is_vadjust {
            255
        } else {
            self.scan_integer()?.value
        };
        self.scan_box_group_opening()?;
        Ok(ScannedInsertConstruction { class, is_vadjust })
    }

    /// Scans TeX82 §1073's box-shift prefix (`\raise`, `\lower`, `\moveleft`,
    /// `\moveright`) once the caller has already validated `abs(mode)+cur_cmd`
    /// legality (tex.web's "Forbidden cases": `vmode+vmove`, `hmode+hmove`,
    /// and `mmode+hmove` never reach `scan_normal_dimen` at all).
    ///
    /// The main-control case reads: `t:=cur_chr; scan_normal_dimen; if t=0
    /// then scan_box(cur_val) else scan_box(-cur_val)`. `\lower`/`\moveright`
    /// have `chr_code=0` and keep the scanned dimension; `\raise`/`\moveleft`
    /// have `chr_code=1` and negate it. This is `box_context`, later stored
    /// verbatim as `shift_amount(cur_box)`.
    pub fn scan_box_shift(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxShift, CommandError> {
        let amount = self.scan_dimension()?.value;
        let delta = match primitive {
            UnexpandablePrimitive::Lower | UnexpandablePrimitive::MoveRight => amount,
            UnexpandablePrimitive::Raise | UnexpandablePrimitive::MoveLeft => -amount,
            _ => return Err(CommandError::input_invariant()),
        };
        let payload = self.scan_box_shift_payload()?;
        Ok(ScannedBoxShift { delta, payload })
    }

    /// Scans TeX82 §1084's `scan_box` operand for a box-shift prefix: `scan_box`
    /// begins with "the next non-blank non-relax" token (§1084's own
    /// `get_x_token` loop), then requires `cur_cmd=make_box`. Since `box_context`
    /// here is always a signed dimension (bounded by `max_dimen`), it can never
    /// reach `leader_flag`, so `scan_box`'s rule-spec branch never applies to a
    /// box-shift operand -- only `\hbox`/`\vbox`/`\vtop`, `\box`, `\copy`,
    /// `\lastbox`, and `\vsplit` are accepted, matching `scan_box_value`'s
    /// `make_box` family exactly. Anything else is `scan_box`'s "A <box> was
    /// supposed to be here" recovery: the rejected command is backed up
    /// (`back_error`) for ordinary replay, and replay alone reports the
    /// diagnostic since it needs a `Universe` sink.
    fn scan_box_shift_payload(&mut self) -> Result<ScannedBoxShiftPayload, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(ScannedBoxShiftPayload::Missing);
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: false,
                    });
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: true,
                    });
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => {
                    return Ok(ScannedBoxShiftPayload::LastBox);
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => {
                    return Ok(ScannedBoxShiftPayload::VSplit(self.scan_vsplit()?));
                }
                Meaning::UnexpandablePrimitive(
                    primitive @ (UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop),
                ) => {
                    return Ok(ScannedBoxShiftPayload::Construction(
                        self.scan_box_construction(primitive)?,
                    ));
                }
                _ => {
                    self.back_input(command)?;
                    return Ok(ScannedBoxShiftPayload::Missing);
                }
            }
        }
    }

    /// Runs TeX82 §774 `init_align`'s `scan_spec(align_group,false)`: §645's
    /// optional `to`/`spread` clause followed by §403's mandatory left brace.
    ///
    /// `\halign`/`\valign` take the same specification as `\hbox`, and §805
    /// packages the preamble prototype box with `hpack(preamble, saved(1),
    /// saved(0))` -- the very values §645 scanned here -- so the clause is
    /// returned rather than discarded.
    ///
    /// The brace is *consumed*, not backed up, exactly as §645 leaves it: the
    /// following `@<Scan the preamble...@>` starts from the token after it.
    /// The two input backups an oracle trace shows here are §407
    /// `scan_keyword`'s own, one per failed keyword, and they are produced by
    /// running the real keyword scans rather than by replaying the brace.
    pub fn scan_alignment_preamble_opening(&mut self) -> Result<ScannedPackingSpec, CommandError> {
        let packing = self.scan_spec_packing()?;
        let _ = self.scan_left_brace(true)?;
        Ok(packing)
    }

    /// Delivers the first alignment cell's lookahead, then backs it up before
    /// the selected u-template is installed.
    ///
    /// This is TeX82's `init_col` lookahead ordering: every non-`\omit`
    /// command, including an ordinary unbraced cell such as `\vrule`, changes
    /// and then restores `align_state` through command-owned backup. `\omit`
    /// instead remains consumed and selects the typed template-free path.
    /// TeX82 §765 does not require the backed-up lookahead to be a left brace.
    pub fn scan_alignment_cell_opening(&mut self) -> Result<AlignmentCellOpening, CommandError> {
        loop {
            let opening = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            match opening.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } => continue,
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit) => {
                    self.command
                        .prepare_alignment_cell_lookahead()
                        .map_err(|_| CommandError::input_invariant())?;
                    return Ok(AlignmentCellOpening::Omit);
                }
                _ => {
                    self.back_input(opening)?;
                    return Ok(AlignmentCellOpening::Template);
                }
            }
        }
    }

    /// Performs TeX82 `fin_col`'s next-entry lookahead. Spaces are delivered
    /// normally; the first non-space token is restored before the selected
    /// u-template is installed.
    pub fn scan_alignment_next_cell_opening(
        &mut self,
    ) -> Result<AlignmentCellOpening, CommandError> {
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                continue;
            }
            if matches!(
                command.meaning(),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)
            ) {
                return Ok(AlignmentCellOpening::Omit);
            }
            self.back_input(command)?;
            return Ok(AlignmentCellOpening::Template);
        }
    }

    /// Consumes the compulsory opener following `align_peek`'s `\\noalign`.
    ///
    /// TeX82 §37 sets `align_state := 1000000`, recognizes the expanded
    /// `no_align` command, then calls `scan_left_brace` before the executor
    /// creates `no_align_group`.  Unlike an `init_col` lookahead, this brace
    /// is not backed up: its raw delivery is the canonical `1000000 ->
    /// 1000001` transition.
    pub fn scan_alignment_noalign_opening(&mut self) -> Result<(), CommandError> {
        let _ = self.scan_left_brace(true)?;
        Ok(())
    }

    /// Installs TeX82 §37's `align_peek` sentinel before its expanded
    /// lookahead.  The command processor owns this state because it is raw
    /// token-delivery state, not executor group state.
    pub fn begin_alignment_peek(&mut self, after_noalign: bool) -> Result<(), CommandError> {
        let changed = self.command.alignment.align_state != 1_000_000;
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        // TeX82 §37 assigns the `align_peek` sentinel before its first
        // expanded lookahead.  Keep that transition command-owned and emit
        // it before an exhausted backup is retired by `get_x_token`.
        #[cfg(any(test, feature = "instrumentation"))]
        if changed || after_noalign {
            self.observe(crate::CommandObservation::Alignment(
                crate::AlignmentRecord {
                    transition: "state_change",
                    alignment: self
                        .command
                        .alignment
                        .active_alignment
                        .map(|alignment| alignment.raw()),
                    align_state: self.command.alignment.align_state,
                    delimiter: None,
                    previous_align_state: None,
                },
            ));
        }
        Ok(())
    }

    /// Enters TeX82's live alignment-preamble scanner episode.
    ///
    /// `init_align` establishes `scanner_status := aligning` after its
    /// required brace has been replayed and backed up, but before the first
    /// `get_preamble_token` retires that backup.  The status therefore belongs
    /// to the command-owned input transition, rather than to executor replay
    /// or the preamble parser.
    pub fn begin_alignment_preamble_scan(&mut self) -> Result<(), CommandError> {
        // TeX82 §776's `@<Scan the preamble...@>` opens with the comment
        // "at this point, |cur_cmd=left_brace|": `scan_spec` has already
        // consumed the opener, so this must not fetch another token. A raw
        // fetch here would discard an immediate `#` in `\\halign{#\\cr}`.
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::input_invariant())?;
        self.command
            .alignment
            .set_preamble_phase(alignment)
            .map_err(|_| CommandError::input_invariant())?;
        let _prior =
            self.command
                .begin_scanner_status(ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(alignment.raw()),
                    builder: TokenBuilderId(0),
                    warning: ScannerWarning(0),
                }));
        self.observe_scanner_status_transition(
            _prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_start",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        let mut columns = Vec::new();
        let mut repeat_start = None;
        loop {
            // These are deliberately separate loops, matching TeX82 §760's
            // `done1`/`done2` labels. A missing `#` leaves the delivered
            // delimiter backed up, then the v-template loop reads it again.
            // A single combined u/v phase loses that replay boundary.
            let mut u_template = Vec::new();
            loop {
                let command = self.get_next()?.ok_or(CommandError::input_invariant())?;
                let token = command.spelling().semantic_token();
                if matches!(
                    token,
                    Token::Char {
                        cat: Catcode::Parameter,
                        ..
                    }
                ) {
                    break;
                }
                let tab = matches!(
                    token,
                    Token::Char {
                        cat: Catcode::AlignmentTab,
                        ..
                    }
                );
                let terminator = tab
                    || matches!(
                        command.meaning(),
                        Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                        )
                    );
                if terminator && self.command.alignment.align_state == -1_000_000 {
                    // The `&&` case is the one exception: the second tab
                    // starts the periodic suffix and u-template scanning
                    // continues. Every other delimiter is TeX's
                    // `Missing # inserted` / `back_error` path.
                    if tab && u_template.is_empty() && repeat_start.is_none() {
                        repeat_start = Some(columns.len());
                        continue;
                    }
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(crate::CommandObservation::Alignment(
                        crate::AlignmentRecord {
                            transition: "missing_parameter",
                            alignment: Some(alignment.raw()),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },
                    ));
                    self.back_error(command, MISSING_PARAMETER_DIAGNOSTIC)?;
                    break;
                }
                if !matches!(
                    command.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) || !u_template.is_empty()
                {
                    // TeX82 §760 eliminates only leading u-template spaces.
                    u_template.push(command.spelling());
                }
            }

            let mut v_template = Vec::new();
            let ends_preamble = loop {
                let command = self.get_next()?.ok_or(CommandError::input_invariant())?;
                let token = command.spelling().semantic_token();
                let ends_column = matches!(
                    token,
                    Token::Char {
                        cat: Catcode::AlignmentTab,
                        ..
                    }
                );
                let ends_preamble = matches!(
                    command.meaning(),
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                    )
                );
                if (ends_column || ends_preamble)
                    && self.command.alignment.align_state == -1_000_000
                {
                    break ends_preamble;
                }
                // §760 reports and discards extra parameter markers in a
                // v-template; it then resumes this same loop.
                if matches!(
                    token,
                    Token::Char {
                        cat: Catcode::Parameter,
                        ..
                    }
                ) {
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(crate::CommandObservation::Alignment(
                        crate::AlignmentRecord {
                            transition: "extra_parameter",
                            alignment: Some(alignment.raw()),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },
                    ));
                    self.command
                        .expansion
                        .pending_diagnostics
                        .push(EXTRA_PARAMETER_DIAGNOSTIC);
                    continue;
                }
                v_template.push(command.spelling());
            };
            columns.push(AlignmentCellTemplates {
                // `init_col` installs a u-template even when its token list
                // is empty. `None` is reserved for the typed `\\omit` path.
                u_template: Some(self.state.finish_traced_token_list(&u_template)),
                v_template: self.state.finish_traced_token_list(&v_template),
            });
            if ends_preamble {
                break;
            }
        }
        self.command
            .alignment
            .complete_preamble(
                alignment,
                AlignmentPreamble {
                    columns,
                    repeat_start,
                },
            )
            .map_err(|_| CommandError::input_invariant())?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        // TeX's `fin_align` boundary becomes observable before `scanner_status`
        // returns to normal. Retain the live aligning episode while publishing
        // its completion, then restore normal status; otherwise an exit record
        // loses its `aligning` identity and reverses the canonical ordering.
        let prior = self.command.begin_scanner_status(ScannerStatus::Normal);
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        Ok(())
    }

    /// Scans TeX's balanced general text through the canonical `scan_toks`
    /// collector. `expanded` controls its TeX82 expanded-collection mode.
    pub fn scan_balanced_text(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedBalancedText, CommandError> {
        // TeX82 §473's `scan_toks` sets `scanner_status` *before* §403's
        // `scan_left_brace` removes the compulsory opening brace, for both
        // collection modes. Callers that reach here -- §1288's `shift_case`,
        // §1352's `\write`, `\special`, and the pdfTeX graphics family --
        // enter `scan_toks` directly, so the brace must not be scanned and
        // backed up here first: that produced a raw delivery and a
        // backup/recovery pair ahead of the absorbing transition, and cost
        // the redelivered brace its source location. The one call site where
        // TeX really does look the brace up first, §1227's token-list
        // assignment, states that explicitly through `GeneralAfterOpening`.
        let scanned = self.scan_toks(ScanToksMode::General { expanded })?;
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance: provenance(&scanned),
        })
    }

    /// Performs TeX82 §1288's complete `shift_case`.
    ///
    /// `\uppercase`/`\lowercase` are `any_mode` main-control cases that never
    /// reach the stomach: §1288 collects a general text with `scan_toks`,
    /// rewrites each token through the current `\uccode`/`\lccode` table, and
    /// hands the result straight back to the input stack with
    /// `back_list(link(def_ref))`. §323's `back_list` is
    /// `begin_token_list(p, backed_up)`, so the resulting level is a
    /// backed-up token list -- one observed input push, and a retirement that
    /// reports backup rather than a stored token-list replay. Keeping the
    /// whole section here makes the observed command-processor path the only
    /// path: no executor-side step re-pushes this list behind the observer.
    pub fn shift_case(&mut self, uppercase: bool) -> Result<(), CommandError> {
        let scanned = self.scan_balanced_text(false)?.tokens;
        // §1288 changes only tokens below `cs_token_flag+single_base`, i.e.
        // character tokens and active characters (both `Token::Char` here),
        // and leaves the `cmd` alone; a zero `\uccode`/`\lccode` entry means
        // "no change".  Multiletter control sequences and frozen tokens are
        // above that bound and are never rewritten.
        let tokens = self.state.tokens(scanned.token_list()).to_vec();
        let origins = self.state.origin_list(scanned.origin_list()).to_vec();
        let shifted = tokens
            .into_iter()
            .enumerate()
            .map(|(index, token)| {
                let origin = origins.get(index).copied().unwrap_or(OriginId::UNKNOWN);
                let token = match token {
                    Token::Char { ch, cat } => {
                        let code = if uppercase {
                            self.state.uccode(ch)
                        } else {
                            self.state.lccode(ch)
                        };
                        char::from_u32(code)
                            .filter(|_| code != 0)
                            .map_or(token, |ch| Token::Char { ch, cat })
                    }
                    Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => token,
                };
                TracedTokenWord::pack(token, origin)
            })
            .collect::<Vec<_>>();
        let level = self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(shifted)),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        #[cfg(not(any(test, feature = "instrumentation")))]
        let _ = level;
        #[cfg(any(test, feature = "instrumentation"))]
        // `back_list` is a plain `begin_token_list`, not §325's `back_input`:
        // it pushes a backed-up level without the accompanying recovery
        // record that a backed-up raw delivery reports.
        self.observe(crate::CommandObservation::Input(crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::Backup,
            level: level.0,
            position: 0,
        }));
        Ok(())
    }

    /// Scans TeX82 §53's `\special` general text.
    ///
    /// Like `new_whatsit`, this expands the balanced general text while the
    /// command processor owns the input episode.  Main control receives only
    /// the immutable result and appends the deferred node; it never reads a
    /// token or opens a compatibility input stack during shipout.
    pub fn scan_special(&mut self) -> Result<ScannedBalancedText, CommandError> {
        self.scan_balanced_text(true)
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedMacroDefinition, CommandError> {
        let command = self
            .next_non_space_raw()?
            .ok_or(CommandError::input_invariant())?;
        let (target, missing_target) = if let Some(target) = command.control_sequence() {
            (target, false)
        } else {
            self.back_input(command)?;
            (self.state.intern_control_sequence("inaccessible"), true)
        };
        let scanned = self.scan_toks(ScanToksMode::MacroDefinition { expanded })?;
        Ok(ScannedMacroDefinition {
            target,
            parameter_text: scanned.parameter_text,
            replacement_text: scanned.replacement_text,
            provenance: provenance(&scanned),
            missing_target,
            malformed_parameter: scanned.malformed_parameter,
        })
    }

    /// Scans TeX82's raw `\let` operand sequence.
    ///
    /// `future` selects `future_let`: the first two raw tokens following the
    /// target are restored in their original order after the second token's
    /// meaning has been captured.
    pub fn scan_let_assignment(
        &mut self,
        future: bool,
    ) -> Result<ScannedLetAssignment, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::input_invariant())?;
        let (source, meaning) = if future {
            let first = self.get_token()?.ok_or(CommandError::input_invariant())?;
            let second = self.get_token()?.ok_or(CommandError::input_invariant())?;
            let source = second.control_sequence();
            let meaning = second.meaning();
            self.replay_raw_commands([first, second]);
            (source, meaning)
        } else {
            let mut source = self.get_token()?.ok_or(CommandError::input_invariant())?;
            if matches!(source.meaning(), Meaning::CharToken { ch: '=', .. }) {
                source = self.get_token()?.ok_or(CommandError::input_invariant())?;
                if matches!(
                    source.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) {
                    source = self.get_token()?.ok_or(CommandError::input_invariant())?;
                }
            }
            (source.control_sequence(), source.meaning())
        };
        Ok(ScannedLetAssignment {
            target,
            source,
            meaning,
        })
    }

    /// TeX's `scan_file_name`, returning a typed boundary instead of an input
    /// cursor or a backed-up raw command.
    pub fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        let first = loop {
            let command = self.get_x_token()?.ok_or(CommandError::input_invariant())?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        let provenance = StructuredProvenance {
            primary: first.origin(),
        };
        let grouped = matches!(
            first.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        );
        // `scan_file_name` replays its first non-space token before consuming
        // the filename. TeX82 exposes this `back_input` hand-off, and it
        // keeps the group-opening case on the same ordinary delivery path.
        self.back_input(first)?;
        let mut name = String::new();
        let mut quoted = false;
        let mut next = None;
        let termination = loop {
            let command = match next.take() {
                Some(command) => command,
                None => match self.get_x_token()? {
                    Some(command) => command,
                    None => break FileNameTermination::EndOfInput,
                },
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } if grouped => {}
                Meaning::CharToken { ch: '"', .. } => quoted = !quoted,
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } if grouped && !quoted => {
                    break FileNameTermination::Group;
                }
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } if !grouped && !quoted => {
                    break FileNameTermination::Space;
                }
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ if !grouped => {
                    self.back_input(command)?;
                    break FileNameTermination::NonCharacter;
                }
                _ => return Err(CommandError::input_invariant()),
            }
        };
        if name.is_empty() {
            return Err(CommandError::input_invariant());
        }
        Ok(ScannedFileName {
            name,
            termination,
            provenance,
        })
    }

    /// Scans and opens one input through the borrow-scoped registered-input
    /// capability. No filesystem or host lookup escapes this boundary.
    pub fn open_registered_input(&mut self) -> Result<RegisteredInput, CommandError> {
        let file_name = self.scan_file_name()?;
        let source = self
            .host
            .input(&file_name.name)
            .ok_or_else(|| CommandError::MissingInput(file_name.name.clone()))?;
        let source = self
            .command
            .register_source(source)
            .map_err(|_| CommandError::input_invariant())?;
        self.command
            .open_registered_source(source)
            .map_err(|_| CommandError::input_invariant())?;
        Ok(RegisteredInput { file_name, source })
    }

    fn next_non_space_raw(&mut self) -> Result<Option<crate::CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_token()? else {
                return Ok(None);
            };
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(Some(command));
            }
        }
    }

    fn replay_raw_commands(&mut self, commands: [crate::CurrentCommand; 2]) {
        for command in &commands {
            self.undo_alignment_delivery(command);
        }
        self.command.push_token_level(
            crate::input::TokenPayload::BackedUp(crate::input::SharedBackedUpBuffer::new(
                commands.map(|command| crate::input::BackedUpToken {
                    spelling: command.spelling(),
                    source_provenance: command.source_provenance(),
                }),
            )),
            crate::input::TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::BackedUp,
        );
    }
}

fn provenance(scanned: &ScannedToks) -> StructuredProvenance {
    StructuredProvenance {
        primary: scanned.primary,
    }
}

#[cfg(test)]
mod tests;

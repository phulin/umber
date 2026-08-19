//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename scanning only. Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use std::sync::Arc;

use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{FontSizeSpec, Scaled};
use tex_state::token::{Catcode, OriginId, RootedTracedTokenBuffer, Token, TracedTokenWord};
use tex_state::{
    SourceId, TracedTokenList,
    env::banks::{GlueParam, IntParam},
};

use crate::input::{
    BackupTreatment, InputLevelId, ReplayTrace, RetirementBehavior, StoredReplayReason,
    TokenBehavior, TokenPayload,
};
use crate::processor::alignment::{PREAMBLE_ALIGN_STATE, is_character_command};
use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerStatus, ScannerStatusVisibility, ScannerWarning,
    TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::scanners::RestrictedIntegerClass;
use crate::{
    AlignmentCellTemplates, AlignmentPreamble, CommandError, CommandProcessor,
    CommandReplayDelivery, CurrentCommand, InternalValue,
    processor::{print_cs_text, render_the_value, selector_meaning_text, string_text},
};

/// Stable pending-diagnostic identities for TeX82 §760 template recovery.
const MISSING_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0001;
const EXTRA_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0002;
const MISSING_DELIMITER_DIAGNOSTIC: u64 = 0x6d61_7468_0000_0001;

const MISSING_DELIMITER_HELP: &[&str] = &[
    "I was expecting to see something like `(' or `\\{' or",
    "`\\}' here. If you typed, e.g., `{' instead of `\\{', you",
    "should probably delete the `{' by typing `1' now, so that",
    "braces don't get unbalanced. Otherwise just proceed.",
    "Acceptable delimiters are characters whose \\delcode is",
    "nonnegative, or you can use `\\delimiter <delimiter code>'.",
];

/// Provenance for a completed structured scan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StructuredProvenance {
    /// Origin of the first non-ignored token accepted by the scan.
    pub primary: OriginId,
}

/// A balanced token list frozen through the aggregate token store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedBalancedText {
    pub tokens: TracedTokenList,
    pub provenance: StructuredProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpandedWriteText {
    pub tokens: TracedTokenList,
    pub unbalanced: bool,
    /// TeX82 §1372's live §310 context, captured before recovery consumes
    /// the artificial write input episode.
    pub error_context: Option<String>,
}

/// The two immutable lists collected for a macro definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedMacroDefinition {
    /// The raw control-sequence (or active-character) target accepted by
    /// TeX82's `prefixed_command`.  Target delivery is command-owned so the
    /// executor never has to reopen raw input between the primitive and its
    /// parameter/replacement scan.
    pub target: Symbol,
    pub parameter_text: RootedTracedTokenBuffer,
    pub replacement_text: RootedTracedTokenBuffer,
    pub provenance: StructuredProvenance,
    pub definition_origin: tex_state::provenance::OriginRef,
}

/// A completed TeX82 `\let` or `\futurelet` assignment.
///
/// The command processor owns every raw operand delivery, including the
/// optional equals sign and `\futurelet`'s lookahead replay. Replay receives
/// only the target and its already-resolved source meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedLetAssignment {
    pub target: Symbol,
    pub source: Option<Symbol>,
    pub meaning: Meaning,
    #[doc(hidden)]
    pub macro_root: Option<tex_state::macro_store::MacroDefinitionRef>,
}

/// A completed TeX82 §1224 `\\chardef` or `\\mathchardef` operand.
///
/// Command processing owns the raw target, optional equals sign, and the
/// class-restricted integer scan (§434 or §436) including its recovery. Main
/// control receives no token or input capability: it only applies the
/// assignment's effective scope and reports the recovery diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedCharacterDefinition {
    pub target: Symbol,
    /// The meaning replaced by §1224's scanner-time provisional `\relax`.
    pub provisional_old: Meaning,
    #[doc(hidden)]
    pub provisional_macro_root: Option<tex_state::macro_store::MacroDefinitionRef>,
    /// The restricted class §1224 selects for this primitive.
    pub class: RestrictedIntegerClass,
    /// `cur_val` after §434/§436's recovery.
    pub value: i32,
    /// The unrecovered `scan_int` result, which `int_error` reports.
    pub scanned: i32,
    /// Whether recovery replaced an out-of-range value with zero.
    pub recovered: bool,
}

/// A completed TeX82 §1224 register-definition assignment.
///
/// The processor owns the raw target, its provisional `\relax` meaning,
/// optional equals sign, and bounded classical register index. Main control
/// receives only the chosen target and register selector to apply with the
/// already determined assignment scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedRegisterDefinition {
    pub target: Symbol,
    /// The meaning replaced by §1224's scanner-time provisional `\relax`.
    pub provisional_old: Meaning,
    #[doc(hidden)]
    pub provisional_macro_root: Option<tex_state::macro_store::MacroDefinitionRef>,
    pub index: u16,
}

fn meaning_macro_root(
    state: &tex_state::CommandContext<'_>,
    meaning: Meaning,
) -> Option<tex_state::macro_store::MacroDefinitionRef> {
    match meaning {
        Meaning::Macro { definition, .. } => Some(state.macro_definition_ref(definition)),
        _ => None,
    }
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
    /// The recovery tex.web §1258/§1259 performed on an illegal size, if any.
    ///
    /// Both sections replace the stated size *and* report it; the replacement
    /// is the scanner's, the report the stomach's, because the command core
    /// owns no text sink.
    pub size_recovery: Option<FontSizeRecovery>,
    /// TeX.web §561's error context after the size clause has been
    /// scanned and its delimiter backed up. Host resource failure is known
    /// only after the command processor borrow ends, so the canonical apply
    /// seam must carry this detached snapshot to report at the original
    /// semantic point.
    pub error_context: String,
}

/// pdfTeX's two generated-font constructors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratedFontKind {
    Copy,
    Letterspace,
}

/// A completed pdfTeX generated-font definition.
///
/// The command processor owns the raw definition target, provisional
/// `nullfont` binding, optional equals sign, source-font selector, and (for
/// `\letterspacefont`) the bounded amount and optional `nolig` keyword.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGeneratedFontDefinition {
    pub kind: GeneratedFontKind,
    pub target: Symbol,
    pub source: FontId,
    pub amount: i16,
    pub no_ligatures: bool,
}

/// tex.web §1258's and §1259's illegal-size recoveries.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum FontSizeRecovery {
    /// §1259: ``Improper `at' size (<s>pt), replaced by 10pt``, for a stated
    /// `at` size outside `0 < s < 2048pt`.
    ImproperAtSize { size: Scaled, context: String },
    /// §1258: `Illegal magnification has been changed to 1000`, reported
    /// through §91's `int_error`, for a `scaled` factor outside `1..=32768`.
    IllegalMagnification { value: i32, context: String },
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
    pub page: PdfImagePageSelection,
    /// pdftex.web §1550's signed, unchecked raster color-space object number.
    ///
    /// Zero selects the image's natural device color space. PDF-page
    /// inclusion deliberately ignores this operand, as upstream does.
    pub color_space_object: i32,
    pub page_box: PdfImagePageBox,
    /// Whether source selected `page_box` rather than leaving it to the live
    /// pdfTeX page-box parameters applied by canonical main control.
    pub page_box_explicit: bool,
    pub attr: Option<TracedTokenList>,
}

impl PdfImageRequest {
    /// Whether two requests select the same immutable host image resource.
    ///
    /// pdftex.web §1550's `read_image` receives the file/page/page-box facts;
    /// rule dimensions and `attr` are command/output state. Dimensions remain
    /// in this deliberately conservative key, but `attr` cannot: its
    /// `TracedTokenList` carries allocator-owned handles that are regenerated
    /// when an aggregate resource suspension rolls back and retries. The
    /// retried request still carries its fresh attribute list to application.
    pub(crate) fn same_resource_as(&self, other: &Self) -> bool {
        self.name == other.name
            && self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.page == other.page
            && self.color_space_object == other.color_space_object
            && self.page_box == other.page_box
            && self.page_box_explicit == other.page_box_explicit
    }
}

/// pdftex.web §1550's mutually exclusive page selectors.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PdfImagePageSelection {
    Number(i32),
    Named(Vec<u8>),
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
    SnapReferencePoint,
    SnapY {
        glue: GlueSpec,
    },
    SnapYComp {
        ratio: u16,
    },
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
        box_register: u16,
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
    Outline(PdfOutlineRequest),
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

/// Fully scanned `\\pdfoutline` document-state mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutlineRequest {
    pub attributes: Option<ScannedBalancedText>,
    pub action: tex_state::PdfActionSpec,
    pub count: i32,
    pub title: ScannedBalancedText,
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

/// The complete command-owned operand of TeX82's `\setbox` assignment.
///
/// TeX82 §1241 calls §1084's `scan_box` from inside `prefixed_command`, so
/// the required `make_box` command never returns to §1030's `big_switch` and
/// must not receive a second main-control command trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedSetBoxAssignment {
    pub index: u16,
    pub path: ScannedSetBoxPath,
}

/// The two distinct TeX82 §1241 paths after `\setbox` has scanned its
/// register and optional equals sign.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScannedSetBoxPath {
    /// `set_box_allowed` was false, so §1241 calls `error` immediately.
    /// No box command has been fetched or backed up.
    Forbidden { error_context: String },
    /// `set_box_allowed` was true and §1084's ordinary `scan_box` ran.
    /// A missing payload has therefore already backed up its rejected command.
    Payload(ScannedBoxShiftPayload),
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
    /// TeX82 §1167's `mmode+vcenter`: `scan_spec(vcenter_group,false);
    /// normal_paragraph; push_nest; mode:=-vmode`. `\vcenter` opens the same
    /// §645 `scan_spec` prefix and the same internal vertical list as
    /// `\vbox`; only §1168's closing action differs (a `vcenter_noad`
    /// nucleus instead of §1075's `box_end`), which is why it shares this
    /// scan and not the math-text-field scan a noad-building primitive would
    /// otherwise take.
    VCenter,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedInsertConstruction {
    pub class: i32,
    pub is_vadjust: bool,
    pub pre: bool,
    /// TeX82 §1099 calls §82's `error` before `scan_left_brace`, so preserve
    /// the live input display at the point the reserved class is detected.
    pub reserved_class_context: Option<String>,
}

/// The completed command-owned operand of TeX82 §1084's `scan_box`.
///
/// Box shifts and `\setbox` share this exact `make_box` vocabulary and
/// recovery; the historical type name is retained as part of the public API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScannedBoxShiftPayload {
    /// `scan_box`'s "A <box> was supposed to be here" recovery: the rejected
    /// command has already been backed up for ordinary replay.
    Missing,
    BoxRegister {
        index: u16,
        copy: bool,
    },
    /// §1081 may diagnose against the live input while taking the last box.
    LastBox {
        error_context: String,
    },
    VSplit(ScannedVSplit),
    Construction(ScannedBoxConstruction),
}

/// A completed TeX82 §1073 box-shift prefix: the already-signed shift amount
/// (tex.web's `box_context`) paired with the following box operand it
/// applies to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedBoxShift {
    pub delta: Scaled,
    pub payload: ScannedBoxShiftPayload,
}

/// The completed register operand of TeX82's `\\box` command.
///
/// `make_box(box_code)` calls §433's `scan_eight_bit_int` before main control
/// can apply the resulting box-list operation. Keeping that scan here
/// preserves the raw digit delivery, bounded recovery, and integer-scanner
/// backup entirely in command control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxRegister {
    pub index: u16,
}

/// The complete command-owned operand of TeX82's `\\vsplit`.
///
/// The keyword's absence is preserved so replay can issue its diagnostic, but
/// both the register and dimension have already been consumed canonically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedVSplit {
    pub index: u16,
    pub height: Scaled,
    /// TeX82 §1082 reports a missing `to` before scanning the dimension.
    pub missing_to_context: Option<String>,
    /// Context after `scan_dimen`, used by the source-free box-kind check.
    pub split_context: String,
}

/// A completed display diagnostic. Its display-line content and source origin
/// are frozen while command input is borrowed, leaving replay no
/// operand-reading or envelope-decoding work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedDisplayDiagnostic {
    /// The content passed to TeX82 §62's `print_nl`, excluding §1293's
    /// terminating period and error completion.
    pub content: String,
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
    BoxRegister { index: u16, copy: bool },
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

/// One step of TeX82 §1123's post-`scan_char_num` lookahead for `\accent`.
///
/// §1123's `make_accent` does not classify the base character directly after
/// the accent code: it runs §1270's `do_assignments` in between, and §1270's
/// loop body is `prefixed_command` -- executor state, not scanner state. The
/// lookahead is therefore delivered one command at a time.
#[derive(Debug, Eq, PartialEq)]
pub enum ScannedAccentBase {
    /// §1124's `letter`, `other_char`, `char_given`, or `char_num` base.
    Character {
        character: u8,
        provenance: StructuredProvenance,
    },
    /// §1270's `prefixed_command`: the delivered assignment the executor must
    /// run before the lookahead continues.
    Assignment(CurrentCommand),
    /// §1124's `else back_input`, already performed, or end of input. Either
    /// way §1123 appends the accent by itself.
    Missing,
}

/// Completed command-owned operands for TeX82 §1123's text `\accent`.
///
/// Only `scan_char_num`'s accent code is command-owned. The base character
/// arrives through [`CommandProcessor::scan_accent_base`], one §1270
/// `do_assignments` iteration at a time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccent {
    pub accent: i32,
    pub accent_provenance: StructuredProvenance,
}

/// The source position at which TeX82 §1117/§1120 opened one live
/// `\discretionary` part.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDiscretionaryOpening {
    pub provenance: StructuredProvenance,
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
    /// §1161 rejected a non-delimiter token and backed it up. The executor
    /// owns the resulting error report after the scanner borrow ends.
    pub missing_delimiter: bool,
    pub provenance: StructuredProvenance,
}

/// The font-size bank addressed by a math family assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFamilySize {
    Text,
    Script,
    ScriptScript,
}

impl MathFamilySize {
    /// Recognizes TeX82 §1234's `def_family` command code.
    ///
    /// `def_family`'s `chr_code` selects the size bank, and every routine that
    /// reaches one of the three primitives -- §415's font-identifier fetch,
    /// §577's `scan_font_ident`, and §1257's assignment -- needs the same
    /// mapping. `None` is "this command is not `def_family`".
    #[must_use]
    pub const fn of_primitive(primitive: UnexpandablePrimitive) -> Option<Self> {
        match primitive {
            UnexpandablePrimitive::TextFont => Some(Self::Text),
            UnexpandablePrimitive::ScriptFont => Some(Self::Script),
            UnexpandablePrimitive::ScriptScriptFont => Some(Self::ScriptScript),
            _ => None,
        }
    }
}

impl From<MathFamilySize> for tex_state::math::MathFontSize {
    fn from(size: MathFamilySize) -> Self {
        match size {
            MathFamilySize::Text => Self::Text,
            MathFamilySize::Script => Self::Script,
            MathFamilySize::ScriptScript => Self::ScriptScript,
        }
    }
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

/// How the stomach must realize one completed math field.
///
/// TeX82 §1151's `scan_math` has exactly two outcomes. An unbraced field is
/// resolved *in place*, by the same procedure that fetched the command: its
/// six scalar cases (`letter`, `other_char`, `char_given`, `char_num`,
/// `math_char_num`, `math_given`, `delim_num`) each end by assigning a single
/// math code `c`, and nothing is ever re-read. A braced field is §1153's
/// ``back_input; scan_left_brace; ... push_math(math_group)`` -- the
/// mandatory brace is consumed and the subformula body is then read *live*
/// by ordinary main control, closed by §1186's `math_group` arm of
/// `handle_right_brace`. A braced field is therefore not command-owned
/// material at all, and must never be absorbed into a token list: doing so
/// backs the brace up a second time, opens an extra replay input level, and
/// swallows the closing brace that TeX delivers as a command.
///
/// A scalar field must not be absorbed and replayed either. §1151 never
/// pushes an input level for it, so a frozen-spelling replay delivers the
/// same command twice, opens and retires a level tex.web has no `token_type`
/// for, and reconstructs the field through a nested mlist -- which also
/// loses `c`'s class bits, because §1151 stores `math_type:=math_char` and
/// drops the class a noad would have carried (`umber2-johp.265`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFieldBody {
    /// TeX82 §1151's scalar outcome: the math code `c` its six cases
    /// produce, which the stomach stores as
    /// ``math_type:=math_char; character:=qi(c mod 256); fam:=...``.
    Character(u16),
    /// TeX82 §1153: `math_group`'s opening brace has been consumed and the
    /// body is live input the stomach reads through main control.
    OpenGroup,
    /// No field is available at all.
    Missing,
}

/// One completed math field, ready for the stomach to store.
///
/// Nothing here is deferred input: §1151 has already read, expanded, and
/// classified everything the field consumed, so the stomach receives a value
/// rather than a replay handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathFieldEpisode {
    pub body: MathFieldBody,
    pub provenance: StructuredProvenance,
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
}

/// Immutable request kinds delivered from command processing to canonical main
/// control for TeX82 §§691–734.  Variants that introduce an mlist episode
/// deliberately contain no source cursor: the later stomach migration can
/// consume only the already-classified request and ask the same processor for
/// the next completed field/group episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathRequest {
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
    Accent {
        character: Option<ScannedMathCharacter>,
    },
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

/// TeX82 §§511–520's three-part current filename.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FileNameComponents {
    pub area: String,
    pub name: String,
    pub extension: String,
}

impl FileNameComponents {
    #[must_use]
    pub fn packed(&self) -> String {
        format!("{}{}{}", self.area, self.name, self.extension)
    }

    pub fn apply_default_extension(&mut self, extension: &str) {
        if self.extension.is_empty() {
            self.extension.push_str(extension);
        }
    }

    fn push_character(&mut self, ch: char) {
        match ch {
            '/' | '\\' | ':' => {
                self.area.push_str(&self.name);
                self.area.push_str(&self.extension);
                self.area.push(ch);
                self.name.clear();
                self.extension.clear();
            }
            // TeX82 §§516--519: the first dot after the final area
            // delimiter starts `cur_ext`; later dots stay in that same
            // component.
            '.' => self.extension.push(ch),
            _ if self.extension.is_empty() => self.name.push(ch),
            _ => self.extension.push(ch),
        }
    }
}

/// A filename scanned from expanded command-owned input.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScannedFileName {
    pub components: FileNameComponents,
    pub provenance: StructuredProvenance,
}

impl ScannedFileName {
    #[must_use]
    pub fn packed(&self) -> String {
        self.components.packed()
    }
}

const FILE_NAME_POOL_CAPACITY: usize = 32_000;

/// Completed input-stream operation.  The command core owns every operand;
/// replay only acquires an already-registered immutable resource and mutates
/// World stream state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputStreamRequest {
    Open {
        stream: i32,
        /// The unrecovered §435 `scan_int` result for `int_error`.
        scanned: i32,
        /// Whether §435 replaced `scanned` with stream zero.
        recovered: bool,
        file_name: ScannedFileName,
    },
    Close {
        stream: i32,
        scanned: i32,
        recovered: bool,
    },
    /// TeX82 §482's `read_toks` has already run: the collected list is
    /// carried here, not a stream the executor must go read itself.
    ///
    /// §1225 calls `read_toks(n,r)` inside `prefixed_command`, so the
    /// collection is part of scanning `\\read` and belongs to the command
    /// core. Replay only installs the parameterless macro §482 built.
    Read {
        /// §1225's plain `scan_int`, unrestricted: §482 maps anything outside
        /// `0..=15` onto stream 16 (the terminal) without diagnosing it.
        stream: i32,
        target: Symbol,
        /// Effective TeX82 §1214 scope selected by `prefixed_command`
        /// before §1225 enters `read_toks`.
        global: bool,
        tokens: tex_state::TracedTokenList,
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
    /// The recursive expanded-command lookahead found a PDF-only extension,
    /// whose own pdftex.web case runs `check_pdfoutput` before every operand
    /// scan. The stomach turns this typed command identity into the canonical
    /// DVI-mode error without giving the scanner the diagnostic channel.
    PdfExtensionInDviMode(UnexpandablePrimitive),
    OpenOut {
        /// TeX82 §435's effective stream after `scan_four_bit_int`.
        stream: u8,
        file_name: ScannedFileName,
    },
    Write {
        stream: WriteStreamSelector,
        tokens: TracedTokenList,
    },
    CloseOut {
        stream: WriteStreamSelector,
    },
    PdfObject(PdfObjectRequest),
    PdfForm(PdfFormRequest),
    PdfImage(PdfImageRequest),
}

/// TeX82 §§1342/1350's normalized selector stored in a write whatsit.
///
/// Slots 16 and 17 are deliberately represented rather than clamped: they
/// stand for every stream above 15 and every negative stream, respectively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteStreamSelector {
    Stream(u8),
    AboveRange,
    Negative,
}

impl WriteStreamSelector {
    #[must_use]
    pub const fn normalized_number(self) -> i32 {
        match self {
            Self::Stream(slot) => slot as i32,
            Self::AboveRange => 16,
            Self::Negative => 17,
        }
    }

    pub fn stream_slot(self) -> Option<tex_state::world::StreamSlot> {
        match self {
            Self::Stream(slot) => Some(tex_state::world::StreamSlot::new(slot)),
            Self::AboveRange | Self::Negative => None,
        }
    }
}

/// One successfully opened capability-registered input source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredInput {
    pub file_name: ScannedFileName,
    pub source: SourceId,
    pub bytes: Arc<[u8]>,
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
    /// Expands a frozen whatsit payload at output traversal time.
    ///
    /// The caller decides how the resulting token spellings are rendered;
    /// this operation owns only canonical replay/expansion state.
    pub fn expand_output_replay(
        &mut self,
        tokens: TracedTokenList,
    ) -> Result<TracedTokenList, CommandError> {
        let episode = self.command.push_output_replay_episode(tokens);
        let mut expanded = self.traced_token_scratch();
        loop {
            match self.get_x_or_protected_with_replay_completion()? {
                Some(CommandReplayDelivery::Command(command)) => {
                    expanded.push(command.rooted_spelling());
                }
                Some(CommandReplayDelivery::Completed(completed)) if completed == episode => break,
                Some(CommandReplayDelivery::Completed(_)) => continue,
                None => return Err(CommandError::input_invariant()),
            }
        }
        Ok(self.state.finish_rooted_traced_token_list(&expanded))
    }

    /// TeX82 §1215's `get_r_token`, including its restart after inserting
    /// the inaccessible target. The rejected delivery is backed up, so the
    /// caller's following operand scan still owns it.
    fn scan_definition_target(&mut self) -> Result<tex_state::interner::Symbol, CommandError> {
        loop {
            let command = match self.next_non_space_raw()? {
                Some(command) => command,
                None => self
                    .next_non_space_raw()?
                    .ok_or(CommandError::input_invariant())?,
            };
            if let Some(target) = command.control_sequence() {
                return Ok(target);
            }

            // §1215 backs up an ordinary non-control token (`cur_cs=0`),
            // while an already-frozen control token is consumed before the
            // inaccessible sentinel is inserted.
            if !matches!(
                command.spelling().semantic_token(),
                tex_state::token::Token::Frozen(_)
            ) {
                self.back_input(command)?;
            }
            let inaccessible =
                Token::Cs(self.state.intern_internal_control_sequence("inaccessible"));
            // §1215's `ins_error` is §327: the synthesized token is a live
            // `inserted` level during §82's report, and `goto restart` then
            // consumes that same level as the definition target.
            self.push_inserted_error_token(inaccessible);
            let context = self.command.output_open_context(&self.state);
            let mut report = self.state.print_err("Missing control sequence inserted");
            report
                .help(&[
                    "Please don't say `\\def cs{...}', say `\\def\\cs{...}'.",
                    "I've inserted an inaccessible control sequence so that your",
                    "definition will be completed without mixing me up too badly.",
                    "You can recover graciously from this error, if you're",
                    "careful; see exercise 27.2 in The TeXbook.",
                ])
                .context(context);
            report.error().jump_out()?;
        }
    }

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
        let target = self.scan_definition_target()?;
        let provisional_old = self.state.meaning(target);
        let provisional_macro_root = meaning_macro_root(&self.state, provisional_old);
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("relax".into()),
                global: provisional_global,
            }),
        );
        let _ = self.scan_optional_equals()?;
        let scanned = self.scan_restricted_integer(class)?;
        Ok(ScannedCharacterDefinition {
            target,
            provisional_old,
            provisional_macro_root,
            class,
            value: scanned.value,
            scanned: scanned.scanned,
            recovered: scanned.recovered,
        })
    }

    /// Scans TeX82 §1224's complete register-definition operand.
    ///
    /// As in §1224, TeX temporarily gives the target `\relax` before the
    /// index scan. This makes a repeated target terminate its own integer
    /// scan rather than expand its previous meaning or report undefined.
    pub fn scan_register_definition(
        &mut self,
        provisional_global: bool,
    ) -> Result<ScannedRegisterDefinition, CommandError> {
        let target = self.scan_definition_target()?;
        let provisional_old = self.state.meaning(target);
        let provisional_macro_root = meaning_macro_root(&self.state, provisional_old);
        self.state
            .set_provisional_meaning(target, Meaning::Relax, provisional_global);
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("relax".into()),
                global: provisional_global,
            }),
        );
        let _ = self.scan_optional_equals()?;
        // TeX82 §1224 uses `scan_eight_bit_int`, while e-TeX 2.6
        // etex.ch [49.1224] replaces that scan with `scan_register_num` so
        // sparse register shorthands may address 0..=32767. pdfTeX inherits
        // the same e-TeX register extension.
        let index = if self.command.profile().capabilities().supports_etex() {
            self.scan_extended_register_index()?
        } else {
            self.scan_eight_bit_register_index()?
        };
        Ok(ScannedRegisterDefinition {
            target,
            provisional_old,
            provisional_macro_root,
            index,
        })
    }

    /// Scans the unexpandable pdfTeX graphics whatsit family.
    ///
    /// This follows pdftex.web's `pdfliteral` through `pdfsnapycomp` scanners:
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
            UnexpandablePrimitive::PdfSnapRefPoint => Request::SnapReferencePoint,
            UnexpandablePrimitive::PdfSnapY => Request::SnapY {
                glue: self.scan_glue(false)?.value,
            },
            UnexpandablePrimitive::PdfSnapYComp => Request::SnapYComp {
                ratio: self.scan_integer()?.value.clamp(0, 1000) as u16,
            },
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
            UnexpandablePrimitive::PdfOutline => Ok(Request::Outline(PdfOutlineRequest {
                attributes: self
                    .scan_keyword("attr")?
                    .value
                    .then(|| self.scan_balanced_text(true))
                    .transpose()?,
                action: self.scan_pdf_action()?,
                count: if self.scan_keyword("count")?.value {
                    self.scan_integer()?.value
                } else {
                    0
                },
                title: self.scan_balanced_text(true)?,
            })),
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
                self.scan_balanced_text(true)?.tokens.token_ref().clone(),
            ))
        } else if self.scan_keyword("num")?.value {
            Ok(tex_state::PdfActionIdentifier::Number(
                self.scan_pdf_positive(kind, bounded_by_halfword)?,
            ))
        } else {
            Err(CommandError::PdfNavigation(match kind {
                "thread identifier" => "pdfTeX error (ext4): thread identifier type missing",
                _ => "pdfTeX error (ext1): identifier type missing",
            }))
        }
    }

    fn scan_pdf_action(&mut self) -> Result<tex_state::PdfActionSpec, CommandError> {
        use tex_state::{PdfActionDestination, PdfActionSpec, PdfActionTarget, PdfActionWindow};
        if self.scan_keyword("user")?.value {
            return Ok(PdfActionSpec::User(
                self.scan_balanced_text(true)?.tokens.token_ref().clone(),
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
                    .map(|text| text.tokens.token_ref().clone())
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
                    self.scan_balanced_text(true)?.tokens.token_ref().clone(),
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
                view: self.scan_balanced_text(true)?.tokens.token_ref().clone(),
            }
        } else if self.scan_keyword("name")?.value {
            PdfActionTarget::Destination(tex_state::PdfActionIdentifier::Name(
                self.scan_balanced_text(true)?.tokens.token_ref().clone(),
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
            box_register: self.scan_extended_register_index()?,
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
    /// §1151 is a classification, not an absorption:
    ///
    /// ```text
    /// begin restart:<Get the next non-blank non-relax non-call token>;
    /// reswitch: case cur_cmd of
    /// letter,other_char,char_given: begin c:=ho(math_code(cur_chr));
    ///     if c=@'100000 then begin <Treat cur_chr as an active character>;
    ///       goto restart; end; end;
    /// char_num: begin scan_char_num; cur_chr:=cur_val; cur_cmd:=char_given;
    ///   goto reswitch; end;
    /// math_char_num: begin scan_fifteen_bit_int; c:=cur_val; end;
    /// math_given: c:=cur_chr;
    /// delim_num: begin scan_twenty_seven_bit_int; c:=cur_val div @'10000; end;
    /// othercases <Scan a subformula enclosed in braces and return>
    /// endcases;
    /// ```
    ///
    /// Every scalar case ends holding a math code and nothing else: no input
    /// level is pushed, no token is backed up, and the command that carried
    /// the case is never delivered a second time. The only `back_input` in
    /// the whole procedure belongs to §1152's active-character restart and to
    /// §1153's braced field.
    ///
    /// `othercases` is the *entire* rest of the vocabulary, not just a left
    /// brace: §1153's `back_input; scan_left_brace` runs §403, which either
    /// consumes a real `{` or reports ``Missing { inserted``, backs the
    /// rejected command up, and behaves as though a brace had been read. The
    /// `math_group` opens either way, so a rejected command becomes the first
    /// token of the subformula body rather than being silently dropped.
    pub fn scan_math_field_episode(&mut self) -> Result<MathFieldEpisode, CommandError> {
        loop {
            // §1151's `restart` label: §404's shared "next non-blank
            // non-relax non-call token", the same fetch §403 opens with.
            let Some(command) = self.next_non_blank_non_relax_x_token()? else {
                return Ok(MathFieldEpisode {
                    body: MathFieldBody::Missing,
                    provenance: StructuredProvenance {
                        primary: OriginId::UNKNOWN,
                    },
                });
            };
            let provenance = StructuredProvenance {
                primary: command.origin(),
            };
            // §1151's `reswitch`: `char_num` scans its selector and re-enters
            // the table as `char_given`, so both reach one `math_code` read.
            let character = match command.meaning() {
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                } => Some(ch),
                Meaning::CharGiven(ch) => Some(ch),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                    Some(self.scan_character_number()?)
                }
                _ => None,
            };
            if let Some(ch) = character {
                let code = self.state.mathcode(ch);
                // §1151 tests `c=@'100000` exactly, and §1152 then resolves
                // the character's active meaning, expands it once with
                // `x_token`, backs the result up, and restarts the field.
                if code == 0o100000 {
                    self.treat_as_active_character(ch, provenance.primary)?;
                    continue;
                }
                return Ok(MathFieldEpisode {
                    body: MathFieldBody::Character(code as u16),
                    provenance,
                });
            }
            let (code, provenance) = match command.meaning() {
                // §1224's `\mathchardef` target carries its own code.
                Meaning::MathCharGiven(code) => (code, provenance),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathChar) => {
                    let scanned = self.scan_math_character()?;
                    (scanned.code, scanned.provenance)
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter) => {
                    let scanned = self.scan_delimiter_number()?;
                    // §1151: `c:=cur_val div @'10000`.
                    ((scanned.code / 0o10000) as u16, scanned.provenance)
                }
                // §1153's `othercases`, verbatim: `back_input;
                // scan_left_brace; ... push_math(math_group); return`. The
                // brace is re-read rather than consumed from `command`
                // because §403's skipped spaces and its missing-brace
                // recovery are both observable.
                _ => {
                    self.back_input(command)?;
                    return Ok(match self.scan_left_brace(true)? {
                        crate::scan_toks::ScannedLeftBrace::Consumed(opening) => MathFieldEpisode {
                            body: MathFieldBody::OpenGroup,
                            provenance: StructuredProvenance {
                                primary: opening.origin(),
                            },
                        },
                        // §403's recovery reaches §1153 with `cur_cmd =
                        // left_brace`, so `push_math(math_group)` runs
                        // unconditionally; the rejected command is already
                        // backed up and opens the body.
                        crate::scan_toks::ScannedLeftBrace::Inserted => MathFieldEpisode {
                            body: MathFieldBody::OpenGroup,
                            provenance: StructuredProvenance {
                                primary: OriginId::UNKNOWN,
                            },
                        },
                    });
                }
            };
            return Ok(MathFieldEpisode {
                body: MathFieldBody::Character(code),
                provenance,
            });
        }
    }

    /// Consumes the mandatory opening brace of one `\mathchoice` branch.
    ///
    /// TeX82 §1172's `append_choices` and §1174's `build_choices` both end in
    /// `push_math(math_choice_group); scan_left_brace`, so all four branches
    /// go through this one scan. Nothing is absorbed: like §1153's braced
    /// math field, a branch body is ordinary input that main control reads
    /// live, closed by §1174's `math_choice_group` arm of
    /// `handle_right_brace`.
    ///
    /// §403's recovery is to behave as though a `{` had been read, so the
    /// group opens either way and the rejected command -- already backed up
    /// by `scan_left_brace` -- becomes the first thing the branch body
    /// reads. The returned flag reports only whether that recovery ran.
    pub fn scan_math_choice_group(&mut self) -> Result<bool, CommandError> {
        Ok(matches!(
            self.scan_left_brace(true)?,
            crate::scan_toks::ScannedLeftBrace::Inserted
        ))
    }

    /// Completes the delimiter immediately following a structural math
    /// boundary (`\left`, `\right`, or `\middle`).
    pub fn scan_math_delimiter_boundary(
        &mut self,
        kind: MathDelimiterBoundaryKind,
    ) -> Result<MathDelimiterBoundary, CommandError> {
        Ok(MathDelimiterBoundary {
            kind,
            delimiter: self.scan_delimiter(false)?,
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

    /// Scans TeX82 §437's `scan_twenty_seven_bit_int` delimiter number: an
    /// ordinary `scan_int` whose result is replaced by zero when it leaves
    /// the 27-bit range.
    ///
    /// This is the whole delimiter operand only where tex.web calls
    /// `scan_twenty_seven_bit_int` directly -- §1154's `mmode+delim_num` and
    /// §1151's `scan_math` `delim_num` case -- and the `r=true` half of
    /// §1160. Every other delimiter position goes through
    /// [`Self::scan_delimiter`].
    pub fn scan_delimiter_number(&mut self) -> Result<ScannedMathDelimiter, CommandError> {
        let scanned = self.scan_restricted_integer(RestrictedIntegerClass::TwentySevenBit)?;
        Ok(ScannedMathDelimiter {
            code: scanned.value as u32,
            recovered: scanned.recovered,
            missing_delimiter: false,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// TeX82 §1160's `scan_delimiter(p, r)`.
    ///
    /// `radical` is tex.web's `r`, "tells if this delimiter follows
    /// `\radical` or not". Only §1163's `math_radical` passes `true`, and
    /// only then is the operand a bare `scan_twenty_seven_bit_int`. Every
    /// other delimiter position -- §1191's `\left`/`\right`, §1192's
    /// `\right` recovery, §1182's `\abovewithdelims` family and §1183's
    /// ambiguous-fraction recovery -- passes `false`, where §1160 instead
    /// fetches §404's next non-blank non-relax non-call token and classifies
    /// it:
    ///
    /// ```text
    /// letter,other_char: cur_val:=del_code(cur_chr);
    /// delim_num: scan_twenty_seven_bit_int;
    /// othercases cur_val:=-1
    /// ```
    ///
    /// so `\left(` reads `(`'s `\delcode` and `\left\delimiter"426830A`
    /// consumes the already-delivered `\delimiter` in place. Scanning the
    /// `r=false` positions as if they were `r=true` made the fetched command
    /// the first token of a `scan_int` instead: `\delimiter` is not a numeric
    /// constant, so §444's `vacuous` case backed it up, published a zero
    /// delimiter, and then re-delivered `\delimiter` to main control as an
    /// independent §1154 math character.
    ///
    /// §1161 owns the negative result: `back_error` returns the rejected
    /// token to the input and the delimiter becomes null. The `delim_num`
    /// branch cannot reach it, because §437 has already clamped an
    /// out-of-range code to zero.
    pub fn scan_delimiter(&mut self, radical: bool) -> Result<ScannedMathDelimiter, CommandError> {
        if radical {
            return self.scan_delimiter_number();
        }
        let Some(command) = self.next_non_blank_non_relax_x_token()? else {
            return Ok(ScannedMathDelimiter {
                code: 0,
                recovered: true,
                missing_delimiter: true,
                provenance: StructuredProvenance {
                    primary: OriginId::UNKNOWN,
                },
            });
        };
        let primary = command.origin();
        let code = match command.meaning() {
            Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            } => self.state.delcode(ch),
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter) => {
                return self.scan_delimiter_number();
            }
            _ => -1,
        };
        if code < 0 {
            // TeX82 §1161: "Missing delimiter (. inserted)" reports through
            // `back_error`, which returns the offending token to the input
            // before the error and leaves the null delimiter behind.
            self.back_input(command)?;
            self.missing_delimiter_error()?;
            return Ok(ScannedMathDelimiter {
                code: 0,
                recovered: true,
                missing_delimiter: true,
                provenance: StructuredProvenance { primary },
            });
        }
        Ok(ScannedMathDelimiter {
            code: code as u32,
            recovered: false,
            missing_delimiter: false,
            provenance: StructuredProvenance { primary },
        })
    }

    /// TeX82 §1161's invalid-delimiter `back_error` report.
    ///
    /// This belongs to the scanner episode, immediately after the rejected
    /// token is backed up. In particular, §1182 must report both delimiter
    /// recoveries before §448 starts scanning `\abovewithdelims`'s rule
    /// thickness. Deferring the reports to stomach application reverses that
    /// order when the thickness itself takes §446's missing-number recovery.
    fn missing_delimiter_error(&mut self) -> Result<(), CommandError> {
        let context = self.command.output_open_context(&self.state);
        if !self.command.semantic_diagnostics.is_empty() || self.command.expanding_deferred_write()
        {
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Recoverable {
                    identity: MISSING_DELIMITER_DIAGNOSTIC,
                    runaway: None,
                    message: "Missing delimiter (. inserted)".into(),
                    help: MISSING_DELIMITER_HELP,
                    context,
                });
            return Ok(());
        }
        let mut report = self.state.print_err("Missing delimiter (. inserted)");
        report.help(MISSING_DELIMITER_HELP).context(context);
        report.error().jump_out()?;
        Ok(())
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
                Some(self.scan_delimiter(false)?),
                Some(self.scan_delimiter(false)?),
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
    pub fn scan_math_request(
        &mut self,
        command: &crate::CurrentCommand,
    ) -> Result<Option<MathRequest>, CommandError> {
        use MathRequest as Request;
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
            UnexpandablePrimitive::Delimiter => Request::Delimiter(self.scan_delimiter_number()?),
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
            UnexpandablePrimitive::Radical => Request::Radical(self.scan_delimiter(true)?),
            // TeX82 §1110 diagnoses a text `\accent` before `math_ac`
            // reaches §436's `scan_fifteen_bit_int`. Keep that operand
            // pending so §82's `show_context` still sees the input level
            // that delivered the command. A real `\mathaccent` has no
            // intervening error and can complete its scalar scan here.
            UnexpandablePrimitive::Accent => Request::Accent { character: None },
            UnexpandablePrimitive::MathAccent => Request::Accent {
                character: Some(self.scan_math_character()?),
            },
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
        read_global: bool,
    ) -> Result<InputStreamRequest, CommandError> {
        use tex_state::meaning::UnexpandablePrimitive;

        match primitive {
            // §§1272--1275's `in_stream` command scans §435's
            // `scan_four_bit_int`. Recovery is complete before the request is
            // committed; the raw value crosses the apply seam only so §435's
            // `int_error` can report it first.
            UnexpandablePrimitive::OpenIn => {
                let scanned = self.scan_restricted_integer(RestrictedIntegerClass::FourBit)?;
                let _ = self.scan_optional_equals()?;
                let file_name = self.scan_file_name()?;
                Ok(InputStreamRequest::Open {
                    stream: scanned.value,
                    scanned: scanned.scanned,
                    recovered: scanned.recovered,
                    file_name,
                })
            }
            UnexpandablePrimitive::CloseIn => {
                let scanned = self.scan_restricted_integer(RestrictedIntegerClass::FourBit)?;
                Ok(InputStreamRequest::Close {
                    stream: scanned.value,
                    scanned: scanned.scanned,
                    recovered: scanned.recovered,
                })
            }
            UnexpandablePrimitive::Read | UnexpandablePrimitive::ReadLine => {
                // §1225's `read_to_cs` scans a plain `scan_int`, *not*
                // §435's four-bit selector: §482 answers an out-of-range
                // stream with `if (n<0)or(n>15) then m:=16`, reading from the
                // terminal, and no error is reported at all.
                let stream = self.scan_integer()?.value;
                // tex.web §1225 reports a missing `to` and inserts it, then
                // runs `get_r_token` regardless: the keyword is recovered,
                // not required. §1225 reports it *here*, between the failed
                // keyword and `get_r_token`, so §82's context still shows the
                // target as `<to be read again>` and no `read_toks` prompt has
                // been printed yet.
                if !self.scan_keyword("to")?.value {
                    let context = self.command.output_open_context(&self.state);
                    let mut report = self.state.print_err("Missing `to' inserted");
                    report.help(&[
                        "You should have said `\\read<number> to \\cs'.",
                        "I'm going to look for the \\cs now.",
                    ]);
                    report.context(context);
                    report.error().jump_out()?;
                }
                // §1215's `get_r_token` backs a rejected ordinary target up
                // immediately. Its §325 stack-conservation step first retires
                // the exhausted keyword-mismatch backup, leaving the rejected
                // target live below §483's temporary read-line source.
                let target = self.scan_definition_target()?;
                // TeX82 §1225: `\\read` scans `n`, `to`, and `r`, then runs
                // §482's `read_toks(n,r)` on the spot. The collector needs
                // live input levels, category codes, `align_state`, and
                // `scanner_status`, all of which are the command core's.
                let tokens =
                    self.read_toks(stream, target, primitive == UnexpandablePrimitive::ReadLine)?;
                Ok(InputStreamRequest::Read {
                    stream,
                    target,
                    global: read_global,
                    tokens,
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
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("set_font".into()),
                global: provisional_global,
            }),
        );
        let _ = self.scan_optional_equals()?;
        let file_name = self.scan_file_name()?;
        let mut size_recovery = None;
        let size = if self.scan_keyword("at")?.value {
            let requested = self.scan_dimension()?.value;
            // §1259's `if (s<=0)or(s>=@'1000000000)`.
            FontSizeSpec::At(
                if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
                    requested
                } else {
                    size_recovery = Some(FontSizeRecovery::ImproperAtSize {
                        size: requested,
                        context: self.command.output_open_context(&self.state),
                    });
                    Scaled::from_raw(10 * Scaled::UNITY)
                },
            )
        } else if self.scan_keyword("scaled")?.value {
            let requested = self.scan_integer()?.value;
            // §1258's `if (cur_val<=0)or(cur_val>32768)`.
            FontSizeSpec::Scale(if (1..=32_768).contains(&requested) {
                requested
            } else {
                size_recovery = Some(FontSizeRecovery::IllegalMagnification {
                    value: requested,
                    context: self.command.output_open_context(&self.state),
                });
                1000
            })
        } else {
            FontSizeSpec::Design
        };
        Ok(FontLoadRequest {
            target,
            name: file_name.packed(),
            size,
            size_recovery,
            error_context: self.command.output_open_context(&self.state),
        })
    }

    /// Scans pdfTeX's `\pdfcopyfont` and `\letterspacefont` definitions.
    ///
    /// Like TeX82 §1257's `new_font`, pdfTeX installs `nullfont` before it
    /// scans any operand following the target. This is significant for a
    /// self-referential definition and for every later failure path.
    pub fn scan_generated_font_definition(
        &mut self,
        kind: GeneratedFontKind,
        provisional_global: bool,
    ) -> Result<ScannedGeneratedFontDefinition, CommandError> {
        let target = self.scan_definition_target()?;
        self.state.set_provisional_meaning(
            target,
            Meaning::Font(tex_state::font::NULL_FONT),
            provisional_global,
        );
        observe!(
            self,
            crate::CommandObservation::Mutation(crate::MutationRecord {
                target: crate::MutationTarget::Meaning,
                key: crate::ObservationValue::Name(self.state.resolve(target).to_owned()),
                value: crate::ObservationValue::Name("set_font".into()),
                global: provisional_global,
            }),
        );
        let _ = self.scan_optional_equals()?;
        let source = self.scan_font_selector()?;
        let (amount, no_ligatures) = match kind {
            GeneratedFontKind::Copy => (0, false),
            GeneratedFontKind::Letterspace => {
                let amount = self.scan_integer()?.value.clamp(-1000, 1000) as i16;
                let no_ligatures = self.scan_keyword("nolig")?.value;
                (amount, no_ligatures)
            }
        };
        Ok(ScannedGeneratedFontDefinition {
            kind,
            target,
            source,
            amount,
            no_ligatures,
        })
    }

    /// Scans pdfTeX's `scan_image` request prefix.
    ///
    /// The ordering follows pdfTeX 1.40.29's `scan_image`: a repeated rule
    /// specification, optional `attr` general text, mutually exclusive
    /// `named` expanded general text or `page` integer, optional `colorspace`
    /// integer, then one page-box selector and the filename operand. The
    /// filename uses TeX82 §§511–520's expanded
    /// filename scanner, so braces are optional, quotes protect spaces, and
    /// the first unquoted space or noncharacter remains the request boundary.
    /// Resource acquisition is expressly outside this scanner.
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
        let page = if self.scan_keyword("named")?.value {
            let tokens = self.scan_balanced_text(true)?.tokens;
            PdfImagePageSelection::Named(
                crate::processor::token_slice_string_text(
                    &mut self.state,
                    tokens.token_ref().tokens(),
                )
                .into_bytes(),
            )
        } else if self.scan_keyword("page")?.value {
            PdfImagePageSelection::Number(self.scan_integer()?.value)
        } else {
            PdfImagePageSelection::Number(1)
        };
        let color_space_object = if self.scan_keyword("colorspace")?.value {
            self.scan_integer()?.value
        } else {
            0
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
        let name = self.scan_file_name()?.packed();
        Ok(PdfImageRequest {
            name,
            width,
            height,
            depth,
            page,
            color_space_object,
            // pdfTeX's default `pdf_pagebox` is configured outside the
            // scanner; Crop is the engine's effective no-parameter default.
            page_box_explicit: page_box.is_some(),
            page_box: page_box.unwrap_or(PdfImagePageBox::Crop),
            attr,
        })
    }
    /// Scans TeX82 §1123's `make_accent` accent code.
    ///
    /// §1123 is `scan_char_num; f:=cur_font; p:=new_character(f,cur_val)` and
    /// only then `do_assignments`, so the accent code is the whole of what the
    /// command layer owns before the executor takes over.
    pub fn scan_accent(&mut self) -> Result<ScannedAccent, CommandError> {
        let accent = self.scan_integer()?;
        Ok(ScannedAccent {
            accent: accent.value,
            accent_provenance: StructuredProvenance {
                primary: accent.provenance.primary,
            },
        })
    }

    /// Delivers one step of TeX82 §1123's post-`scan_char_num` lookahead.
    ///
    /// §404's `<Get the next non-blank non-relax non-call token>` is shared by
    /// §1270's `do_assignments` and §1124's base-character classification --
    /// §1270 leaves the token it stops on in `cur_cmd`, and §1124 classifies
    /// exactly that token. This therefore performs the fetch and §1124's
    /// classification, and hands any other command back to the executor.
    ///
    /// A `prefixed_command` must not be replayed. §1270 executes it in place,
    /// with no `back_input` at all, so backing it up would push a backup
    /// level, emit a recovery record and deliver the command a second time,
    /// none of which tex.web does (`umber2-johp.196`, `umber2-johp.264`). It
    /// is handed to the executor still delivered; only §1124's own `else`
    /// branch replays, and it does so here, inside the delivery episode that
    /// owns the command.
    pub fn scan_accent_base(&mut self) -> Result<ScannedAccentBase, CommandError> {
        let Some(command) = self.next_non_blank_non_relax_x_token()? else {
            return Ok(ScannedAccentBase::Missing);
        };
        let provenance = StructuredProvenance {
            primary: command.origin(),
        };
        match command.meaning() {
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
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                let character = u8::try_from(self.scan_integer()?.value)
                    .map_err(|_| CommandError::input_invariant())?;
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            meaning if crate::primitives::is_prefixed_command(meaning) => {
                Ok(ScannedAccentBase::Assignment(command))
            }
            _ => {
                self.back_input(command)?;
                Ok(ScannedAccentBase::Missing)
            }
        }
    }

    /// Delivers one command from TeX82 §1270's `do_assignments` fetch.
    ///
    /// The fetch itself is §404's "next non-blank non-relax non-call token".
    /// Callers must dispatch the returned command in place: `do_assignments`
    /// neither backs up assignments nor refetches the first non-assignment it
    /// stops on. This boundary is also used by §1206 after `fin_align`, where
    /// blanks before the display-closing command must not reach main control.
    pub fn next_do_assignments_command(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        self.next_non_blank_non_relax_x_token()
    }

    /// Consumes only §1117/§1120's opening brace. The body remains on the
    /// live input stack and returns to main control in restricted horizontal
    /// mode; in particular, no macro or conditional from the body is expanded
    /// before the executor has installed `disc_group`.
    pub fn scan_discretionary_opening(
        &mut self,
    ) -> Result<ScannedDiscretionaryOpening, CommandError> {
        let opening = self.scan_left_brace(true)?;
        Ok(ScannedDiscretionaryOpening {
            provenance: StructuredProvenance {
                primary: opening.origin(),
            },
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
    pub fn scan_write_stream(&mut self) -> Result<WriteStreamSelector, CommandError> {
        let value = self.scan_integer()?.value;
        Ok(if value < 0 {
            WriteStreamSelector::Negative
        } else if value > 15 {
            WriteStreamSelector::AboveRange
        } else {
            WriteStreamSelector::Stream(value as u8)
        })
    }

    /// Scans TeX82 §53's one-token `\immediate` extension execution.
    ///
    /// `do_extension` calls `get_x_token`, executes only `openout`, `write`,
    /// and `closeout`, and backs every other expanded command up for ordinary
    /// main control.  The integer, optional-equals, filename, and write-text
    /// scans remain in this command-owned episode.
    pub fn scan_immediate_extension(
        &mut self,
        pdf_output_enabled: bool,
    ) -> Result<ImmediateExtension, CommandError> {
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
                let stream = self
                    .scan_restricted_integer(RestrictedIntegerClass::FourBit)?
                    .value as u8;
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
                let expanded = self.expand_write_text(tokens)?;
                Ok(ImmediateExtension::Write {
                    stream,
                    tokens: expanded.tokens,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
                let stream = self.scan_write_stream()?;
                Ok(ImmediateExtension::CloseOut { stream })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject) => {
                if !pdf_output_enabled {
                    Ok(ImmediateExtension::PdfExtensionInDviMode(
                        UnexpandablePrimitive::PdfObject,
                    ))
                } else {
                    Ok(ImmediateExtension::PdfObject(
                        self.scan_pdf_object_request()?,
                    ))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXForm) => {
                if !pdf_output_enabled {
                    Ok(ImmediateExtension::PdfExtensionInDviMode(
                        UnexpandablePrimitive::PdfXForm,
                    ))
                } else {
                    Ok(ImmediateExtension::PdfForm(
                        self.scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)?,
                    ))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfXImage) => {
                if !pdf_output_enabled {
                    Ok(ImmediateExtension::PdfExtensionInDviMode(
                        UnexpandablePrimitive::PdfXImage,
                    ))
                } else {
                    Ok(ImmediateExtension::PdfImage(self.scan_pdf_image_request()?))
                }
            }
            _ => {
                self.back_input(command)?;
                Ok(ImmediateExtension::Continue)
            }
        }
    }

    /// Expands TeX82 §§1369--1372 write text inside the artificial brace and
    /// frozen-`\endwrite` input episode installed by `write_out`.
    ///
    /// The returned recovery flag is §1371's `cur_tok<>end_write_token`
    /// test. In that case TeX reports "Unbalanced write command" and consumes
    /// through the inaccessible sentinel, never through the surrounding
    /// source input.
    pub fn expand_write_text(
        &mut self,
        tokens: TracedTokenList,
    ) -> Result<ExpandedWriteText, CommandError> {
        self.write_expansion_depth = self
            .write_expansion_depth
            .checked_add(1)
            .ok_or_else(CommandError::input_invariant)?;
        let result = self.expand_write_text_inner(tokens);
        self.write_expansion_depth -= 1;
        result
    }

    fn expand_write_text_inner(
        &mut self,
        tokens: TracedTokenList,
    ) -> Result<ExpandedWriteText, CommandError> {
        let write_words = tokens.token_ref().tokens().len();
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
        let stopper_level = self.push_write_recovery([right_brace, endwrite], right_brace);
        let write_level = self.command.push_token_level(
            TokenPayload::stored(tokens.token_ref().clone(), tokens.origin_ref().clone()),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::Write),
        );
        // TeX82 §§323 and 1370 trace the named write_text list at
        // begin_token_list, before the opening-brace insertion and expanded
        // scan_toks can report an error. The whole write expansion runs
        // inside the shipout artifact transaction, so carry this print in the
        // command diagnostic queue instead of letting staging consume it.
        if self
            .state
            .int_param(tex_state::env::banks::IntParam::TRACING_MACROS)
            > 1
        {
            let mut text = String::new();
            crate::processor::expand::append_print_esc_text(&self.state, "write", &mut text);
            text.push_str("->");
            for token in tokens.token_ref().tokens().iter().copied() {
                crate::processor::expand::append_token_list_token_text(
                    &self.state,
                    token,
                    &mut text,
                );
            }
            self.command
                .semantic_diagnostics
                .push(crate::CommandSemanticDiagnostic::Trace {
                    text,
                    force_newline: false,
                });
        }
        self.observe_write_list_push(write_level);
        self.push_write_recovery([left_brace], left_brace);

        self.outer_recovered_while_absorbing = false;
        let expanded = self.scan_balanced_text(true)?.tokens;
        let transient_words = self.command.transient_dynamic_words();
        let expanded_words = expanded.token_ref().tokens().len();
        // TeX82 §1370 keeps the original write list, its expanded scan result,
        // the command-owned transient input nodes, and the three artificial
        // brace/`endwrite` nodes live on the same `write_out` call stack. The
        // expanded list also owns §200's reference-count head.
        self.state.observe_transient_token_words(
            write_words
                .saturating_add(expanded_words)
                .saturating_add(transient_words)
                .saturating_add(4),
        );
        let mut stopper = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let unbalanced =
            self.outer_recovered_while_absorbing || stopper.spelling().semantic_token() != endwrite;
        self.outer_recovered_while_absorbing = false;
        // §1372 calls `error` before its recovery loop consumes through the
        // frozen stopper. Preserve that instant: the write and inserted-list
        // levels are gone by the time shipout can render the queued report.
        let error_context = unbalanced.then(|| self.command.output_open_context(&self.state));
        while stopper.spelling().semantic_token() != endwrite {
            stopper = self.get_token()?.ok_or(CommandError::input_invariant())?;
        }
        self.retire_last_delivery_level()?;
        if unbalanced {
            self.retire_exhausted_through(stopper_level)?;
        }
        Ok(ExpandedWriteText {
            tokens: expanded,
            unbalanced,
            error_context,
        })
    }

    /// Freezes the ordinary `\\write` text after TeX82's `scan_int`
    /// terminator has been validated and backed up. Unlike general-text
    /// callers, §53's `new_write_whatsit` enters the absorbing collection at
    /// that already-backed-up brace.
    fn scan_immediate_write_text(&mut self) -> Result<TracedTokenList, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralAfterOpening {
            expanded: false,
            primary: OriginId::UNKNOWN,
            owner: None,
        })?;
        Ok(scanned.replacement_text)
    }

    fn push_write_recovery(
        &mut self,
        tokens: impl IntoIterator<Item = Token>,
        observed: Token,
    ) -> InputLevelId {
        let level = self.command.push_token_level(
            TokenPayload::transient(
                tokens
                    .into_iter()
                    .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN)),
            ),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Inserted,
        );
        self.observe_inserted_token_recovery(level, observed);
        level
    }

    /// Scans TeX82 §1241's complete `\setbox` assignment operand.
    ///
    /// TeX.web's `prefixed_command` dispatches `set_box` to §433's
    /// `scan_eight_bit_int` then `scan_optional_equals`, followed immediately
    /// by the `set_box_allowed` test. Its false branch reports directly,
    /// without fetching a box command; its true branch enters §1084's
    /// `scan_box`. None of the scanned operand returns to main control.
    /// e-TeX 2.6 [49.1241] widens only the target to `scan_register_num` while
    /// retaining the same complete operand ownership and backup transitions.
    pub fn scan_setbox_assignment(
        &mut self,
        set_box_allowed: bool,
    ) -> Result<ScannedSetBoxAssignment, CommandError> {
        let index = self.scan_profile_register_index()?;
        let _ = self.scan_optional_equals()?;
        let path = if set_box_allowed {
            ScannedSetBoxPath::Payload(self.scan_box_payload()?)
        } else {
            ScannedSetBoxPath::Forbidden {
                error_context: self.command.output_open_context(&self.state),
            }
        };
        Ok(ScannedSetBoxAssignment { index, path })
    }

    /// Scans the register operand of TeX82 §1079's `make_box(box_code)` and
    /// e-TeX 2.6 [47.1079]'s sparse-array replacement.
    pub fn scan_box_register(&mut self) -> Result<ScannedBoxRegister, CommandError> {
        Ok(ScannedBoxRegister {
            index: self.scan_profile_register_index()?,
        })
    }

    /// Scans TeX82 §1082's `\\vsplit <number> to <dimen>` prefix.
    ///
    /// e-TeX 2.6 [47.1082] widens the source box selector from
    /// `scan_eight_bit_int` to `scan_register_num`.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let index = self.scan_profile_register_index()?;
        let missing_to_context = (!self.scan_keyword("to")?.value)
            .then(|| self.command.output_open_context(&self.state));
        let height = self.scan_dimension()?.value;
        let split_context = self.command.output_open_context(&self.state);
        Ok(ScannedVSplit {
            index,
            height,
            missing_to_context,
            split_context,
        })
    }

    /// TeX82 §296's `print_meaning` as `\\show` reaches it.
    ///
    /// A macro's meaning is `print_cmd_chr`, then `print_char(":")`, then
    /// `print_ln`, then `token_show` of the body -- so `\\show\\cs` puts the
    /// replacement text on its own line. `\\meaning` and `\\showthe` share
    /// `print_meaning` but run it under §471's `new_string` selector, where
    /// §57's `print_ln` does nothing, which is why only this caller breaks
    /// the line.
    fn shown_meaning_text(
        state: &mut tex_state::CommandContext<'_>,
        command: &crate::CurrentCommand,
    ) -> String {
        let text = selector_meaning_text(state, command);
        let breaks_after_colon = matches!(command.meaning(), Meaning::Macro { .. })
            || matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndTemplate)
            )
            || matches!(
                command.meaning(),
                Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::TopMark
                        | tex_state::meaning::ExpandablePrimitive::FirstMark
                        | tex_state::meaning::ExpandablePrimitive::BotMark
                        | tex_state::meaning::ExpandablePrimitive::SplitFirstMark
                        | tex_state::meaning::ExpandablePrimitive::SplitBotMark
                )
            );
        if breaks_after_colon {
            text.replacen(':', ":\n", 1)
        } else {
            text
        }
    }

    /// TeX82 §46's raw `\\show` operand scan.
    pub fn scan_show(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let command = self.get_token()?.ok_or(CommandError::input_invariant())?;
        let token = command.spelling().semantic_token();
        let content = match token {
            Token::Cs(_)
            | Token::Char {
                cat: Catcode::Active,
                ..
            } => {
                let raw = string_text(&self.state, token);
                let mut shown = String::new();
                self.state.append_selector_string_text(&raw, &mut shown);
                format!(
                    "> {shown}={}",
                    Self::shown_meaning_text(&mut self.state, &command)
                )
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                format!("> {}", Self::shown_meaning_text(&mut self.state, &command))
            }
        };
        Ok(ScannedDisplayDiagnostic {
            content,
            provenance: StructuredProvenance {
                primary: command.origin(),
            },
        })
    }

    /// TeX82 §46's `\\showthe` internal-value scan.
    pub fn scan_showthe(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let value = self.scan_internal_value_or_zero()?;
        let text = match value.value {
            value @ (InternalValue::Integer(_)
            | InternalValue::Dimension(_)
            | InternalValue::Glue(_)
            | InternalValue::MuGlue(_)) => {
                render_the_value(&value).expect("non-token values render")
            }
            // TeX82 §§262/1297: `the_toks` turns an `ident_val` into a
            // control-sequence token, then `token_show` uses `print_cs`.
            // Its control-word delimiter therefore precedes §1293's period.
            InternalValue::Font(symbol) => print_cs_text(&mut self.state, symbol),
            InternalValue::Tokens { tokens, .. } => {
                let mut text = String::new();
                for &token in tokens.tokens() {
                    self.state.append_token_selector_text(token, &mut text);
                }
                text
            }
        };
        Ok(ScannedDisplayDiagnostic {
            content: format!("> {text}"),
            provenance: StructuredProvenance {
                primary: value.provenance.primary,
            },
        })
    }

    /// Scans e-TeX 2.6 `etex.ch` [17.3623--3660]'s unexpanded general text
    /// operand for `\\showtokens`.
    ///
    /// The compulsory braces are removed and the balanced interior is
    /// retained verbatim; expansion is never entered.
    pub fn scan_showtokens(&mut self) -> Result<ScannedBalancedText, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralText {
            purpose: "detokenize",
        })?;
        let provenance = provenance(&scanned);
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance,
        })
    }

    /// e-TeX 2.6 `etex.ch` [49.1296]'s extended box-register scan for
    /// `\\showbox`.
    ///
    /// The change from TeX82's `scan_eight_bit_int` to `scan_register_num`
    /// retains the restricted scanner's invalid-to-zero recovery before the
    /// box lookup.
    pub fn scan_showbox(&mut self) -> Result<(u16, StructuredProvenance), CommandError> {
        let class = if self.command.profile().capabilities().supports_etex() {
            RestrictedIntegerClass::Register
        } else {
            RestrictedIntegerClass::EightBit
        };
        let index = self.scan_restricted_integer(class)?;
        Ok((
            u16::try_from(index.value).expect("recovered register number is in range"),
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
                    index: self.scan_eight_bit_register_index()?,
                    copy: false,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.scan_eight_bit_register_index()?,
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
        let _ = self.scan_left_brace(true)?;
        Ok(())
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
    ///
    /// §1167's `mmode+vcenter` runs the identical prefix
    /// (`scan_spec(vcenter_group,false)`), so `\vcenter` is scanned here
    /// rather than as a math text field: its body is an internal vertical
    /// list, not an mlist, and only §1168's closing action distinguishes it
    /// from `\vbox`.
    pub fn scan_box_construction(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxConstruction, CommandError> {
        let kind = match primitive {
            UnexpandablePrimitive::HBox => ScannedBoxKind::HBox,
            UnexpandablePrimitive::VBox => ScannedBoxKind::VBox,
            UnexpandablePrimitive::VTop => ScannedBoxKind::VTop,
            UnexpandablePrimitive::VCenter => ScannedBoxKind::VCenter,
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
    /// `scan_eight_bit_int` owns its range clamp and queues §433's diagnostic
    /// for the executor's canonical error channel. The effective value is
    /// carried to replay; `\vadjust` skips the scan entirely, so its fixed
    /// 255 is never subject to that diagnostic.
    pub fn scan_insert_construction(
        &mut self,
        is_vadjust: bool,
    ) -> Result<ScannedInsertConstruction, CommandError> {
        let pre = is_vadjust
            && self.command.profile().capabilities().supports_pdftex()
            && self.scan_keyword("pre")?.value;
        let (class, reserved_class_context) = if is_vadjust {
            (255, None)
        } else {
            let class = self
                .scan_restricted_integer(RestrictedIntegerClass::EightBit)?
                .value;
            let context = (class == 255).then(|| self.command.output_open_context(&self.state));
            (class, context)
        };
        self.scan_box_group_opening()?;
        Ok(ScannedInsertConstruction {
            class,
            is_vadjust,
            pre,
            reserved_class_context,
        })
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
        let payload = self.scan_box_payload()?;
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
    fn scan_box_payload(&mut self) -> Result<ScannedBoxShiftPayload, CommandError> {
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
                    return Ok(ScannedBoxShiftPayload::LastBox {
                        error_context: self.error_context(),
                    });
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

    /// Performs TeX82 §791 `fin_col`'s next-entry lookahead. TeX82 uses
    /// `get_x_token`; e-TeX 2.6 change section [37.791] and pdfTeX use
    /// `get_x_or_protected`. The profile-aware fetch and pending observation
    /// ownership are shared with §785's post-row `align_peek`.
    pub fn scan_alignment_next_cell_opening(
        &mut self,
    ) -> Result<AlignmentCellOpening, CommandError> {
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        let lookahead = self
            .next_alignment_lookahead()?
            .ok_or(CommandError::input_invariant())?;
        {
            if matches!(
                lookahead.command().meaning(),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)
            ) {
                let _ = self.commit_alignment_lookahead_delivery(lookahead);
                return Ok(AlignmentCellOpening::Omit);
            }
            self.back_alignment_lookahead(lookahead)?;
        }
        Ok(AlignmentCellOpening::Template)
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
    pub fn begin_alignment_peek(&mut self, restarting: bool) -> Result<(), CommandError> {
        let changed = self.command.alignment.align_state != 1_000_000;
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::input_invariant())?;
        // TeX82 §785's `restart` label assigns the sentinel on every pass.
        // The initial pass is already represented when its caller changed
        // the value; an ignored `\crcr` returns to the label and must publish
        // the otherwise-idempotent assignment too.
        self.observe_alignment_peek_sentinel(changed || restarting);
        Ok(())
    }

    /// Enters TeX82's live alignment-preamble scanner episode.
    ///
    /// `init_align` establishes `scanner_status := aligning` after its
    /// required brace has been replayed and backed up, but before the first
    /// `get_preamble_token` retires that backup.  The status therefore belongs
    /// to the command-owned input transition, rather than to executor replay
    /// or the preamble parser.
    pub fn begin_alignment_preamble_scan(
        &mut self,
        owner: Option<tex_state::interner::Symbol>,
    ) -> Result<(), CommandError> {
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
        let builder = TokenBuilderId(self.command.transient.next_builder_identity);
        self.command.transient.next_builder_identity =
            self.command.transient.next_builder_identity.wrapping_add(1);
        self.command
            .transient
            .builders
            .push(crate::state::LiveTokenBuilder {
                identity: builder.0,
                tokens: RootedTracedTokenBuffer::default(),
            });
        let scanner_episode = self.begin_scanner_episode(
            ScannerStatus::Aligning(AlignmentScanContext {
                alignment: AlignmentId(alignment.raw()),
                builder,
                owner,
                warning: ScannerWarning(0),
            }),
            ScannerStatusVisibility::Observed,
        );
        observe!(
            self,
            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                transition: "preamble_start",
                alignment: Some(alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },),
        );
        let mut columns = Vec::new();
        let tabskip = self.state.glue_param(GlueParam::TAB_SKIP.raw());
        let mut current_tabskip = self.state.glue(tabskip);
        let mut tabskips = vec![current_tabskip];
        let mut repeat_start = None;
        loop {
            // These are deliberately separate loops, matching TeX82 §760's
            // `done1`/`done2` labels. A missing `#` leaves the delivered
            // delimiter backed up, then the v-template loop reads it again.
            // A single combined u/v phase loses that replay boundary.
            let mut u_template = RootedTracedTokenBuffer::default();
            loop {
                let command = self
                    .get_preamble_token()?
                    .ok_or(CommandError::input_invariant())?;
                if matches!(
                    command.meaning(),
                    Meaning::GlueParam(index) if index == GlueParam::TAB_SKIP.raw()
                ) {
                    // TeX82 §759 executes only a direct `\tabskip`, then
                    // restarts instead of copying it into the template.
                    let _ = self.scan_optional_equals()?;
                    current_tabskip = self.scan_glue(false)?.value;
                    let global = self.state.int_param(IntParam::GLOBAL_DEFS) > 0;
                    self.state.define_preamble_tabskip(current_tabskip, global);
                    // TeX82 §759 has already appended the glue node for the
                    // boundary before this u-template. This assignment is
                    // therefore the value for the next boundary, which the
                    // completed-column path appends below.
                    continue;
                }
                if is_character_command(&command, Catcode::Parameter) {
                    break;
                }
                let tab = is_character_command(&command, Catcode::AlignmentTab);
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
                    observe!(
                        self,
                        crate::CommandObservation::Alignment(crate::AlignmentRecord {
                            transition: "missing_parameter",
                            alignment: Some(alignment.raw()),
                            nesting: self.command.alignment_observation_nesting(),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },),
                    );
                    self.back_error_reporting(
                        command,
                        MISSING_PARAMETER_DIAGNOSTIC,
                        "Missing # inserted in alignment preamble".to_owned(),
                        &[
                            "There should be exactly one # between &'s, when an",
                            "\\halign or \\valign is being set up. In this case you had",
                            "none, so I've put one in; maybe that will work.",
                        ],
                    )?;
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
                    u_template.push(command.rooted_spelling());
                    self.command
                        .transient
                        .builders
                        .iter_mut()
                        .find(|live| live.identity == builder.0)
                        .ok_or(CommandError::input_invariant())?
                        .tokens
                        .push(command.rooted_spelling());
                }
            }

            let mut v_template = RootedTracedTokenBuffer::default();
            let ends_preamble = loop {
                let command = self
                    .get_preamble_token()?
                    .ok_or(CommandError::input_invariant())?;
                if matches!(
                    command.meaning(),
                    Meaning::GlueParam(index) if index == GlueParam::TAB_SKIP.raw()
                ) {
                    let _ = self.scan_optional_equals()?;
                    current_tabskip = self.scan_glue(false)?.value;
                    let global = self.state.int_param(IntParam::GLOBAL_DEFS) > 0;
                    self.state.define_preamble_tabskip(current_tabskip, global);
                    // The current boundary was frozen before this template;
                    // the completed-column path appends this new value for
                    // the following boundary.
                    continue;
                }
                let ends_column = is_character_command(&command, Catcode::AlignmentTab);
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
                if is_character_command(&command, Catcode::Parameter) {
                    observe!(
                        self,
                        crate::CommandObservation::Alignment(crate::AlignmentRecord {
                            transition: "extra_parameter",
                            alignment: Some(alignment.raw()),
                            nesting: self.command.alignment_observation_nesting(),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },),
                    );
                    self.report_recoverable(
                        EXTRA_PARAMETER_DIAGNOSTIC,
                        "Only one # is allowed per tab".to_owned(),
                        &[
                            "There should be exactly one # between &'s, when an",
                            "\\halign or \\valign is being set up. In this case you had",
                            "more than one, so I'm ignoring all but the first.",
                        ],
                    );
                    continue;
                }
                v_template.push(command.rooted_spelling());
                self.command
                    .transient
                    .builders
                    .iter_mut()
                    .find(|live| live.identity == builder.0)
                    .ok_or(CommandError::input_invariant())?
                    .tokens
                    .push(command.rooted_spelling());
            };
            columns.push(AlignmentCellTemplates {
                // `init_col` installs a u-template even when its token list
                // is empty. `None` is reserved for the typed `\\omit` path.
                u_template: Some(self.state.finish_rooted_traced_token_list(&u_template)),
                v_template: self.state.finish_rooted_traced_token_list(&v_template),
            });
            tabskips.push(current_tabskip);
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
                    tabskips,
                    default_tabskip: current_tabskip,
                    repeat_start,
                },
            )
            .map_err(|_| CommandError::input_invariant())?;
        observe!(
            self,
            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(alignment.raw()),
                nesting: self.command.alignment_observation_nesting(),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },),
        );
        // TeX's `fin_align` boundary becomes observable before `scanner_status`
        // returns to normal. Retain the live aligning episode while publishing
        // its completion, then restore normal status; otherwise an exit record
        // loses its `aligning` identity and reverses the canonical ordering.
        self.finish_scanner_episode(scanner_episode);
        self.command
            .transient
            .builders
            .retain(|live| live.identity != builder.0);
        Ok(())
    }

    /// TeX82 §759's `get_preamble_token`.
    ///
    /// A `\span` is not template material: it fetches the following token,
    /// expands that token exactly once when expandable, and repeats if the
    /// resulting raw token is another `\span`. Ordinary template tokens stay
    /// raw so their meanings are resolved when each cell is executed.
    fn get_preamble_token(&mut self) -> Result<Option<CurrentCommand>, CommandError> {
        let mut command = self.get_token()?;
        while command.as_ref().is_some_and(|command| {
            matches!(
                command.meaning(),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span)
            )
        }) {
            let Some(next) = self.get_token()? else {
                return Ok(None);
            };
            if crate::processor::expand::is_expandable_command(&next) {
                self.expand(&next)?;
                command = self.get_token()?;
            } else {
                command = Some(next);
            }
        }
        if command.as_ref().is_some_and(|command| {
            matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::EndGroup,
                    ..
                }
            ) && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE
        }) && let Some(cr) = self.command.alignment.pending_outer_recovery_cr.take()
        {
            // §336's first inserted `\cr` was seen while the runaway brace
            // was still open. Once the follow-up `}` restores the preamble
            // sentinel, replay the owned delimiter/brace tail before the
            // backed-up forbidden command can open a second runaway episode.
            self.conserve_input_stack()?;
            self.command.push_token_level(
                TokenPayload::transient([
                    cr,
                    TracedTokenWord::pack(
                        Token::Char {
                            ch: '}',
                            cat: Catcode::EndGroup,
                        },
                        OriginId::UNKNOWN,
                    ),
                ]),
                TokenBehavior::Recovery,
                RetirementBehavior::Pop,
                ReplayTrace::Inserted,
            );
        }
        Ok(command)
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
        let provenance = provenance(&scanned);
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance,
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
        let token_count = scanned.token_ref().tokens().len();
        let mut shifted = Vec::with_capacity(token_count);
        for index in 0..token_count {
            // Copy one immutable word at a time so the source interned lists
            // remain in place while the case-code lookup records its mutable
            // dependency read. Only the rewritten backup list needs storage.
            let token = scanned.token_ref().tokens()[index];
            let origin = scanned
                .origin_ref()
                .origins()
                .get(index)
                .copied()
                .unwrap_or(OriginId::UNKNOWN);
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
            shifted.push(TracedTokenWord::pack(token, origin));
        }
        let level = self.command.push_token_level(
            TokenPayload::transient(shifted),
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        // `back_list` is a plain `begin_token_list`, not §325's `back_input`:
        // it pushes a backed-up level without the accompanying recovery
        // record that a backed-up raw delivery reports.
        self.observe(crate::CommandObservation::Input(crate::InputRecord {
            transition: crate::InputTransition::Push,
            reason: crate::InputReason::Backup,
            source_name: None,
            source: None,
            level: level.0,
            position: 0,
        }));
        Ok(())
    }

    /// Scans `\special`, including pdfTeX's optional `shipout` keyword.
    ///
    /// The ordinary form expands its general text immediately, as TeX82 does.
    /// The `shipout` form retains the unexpanded balanced tokens so traversal
    /// can expand them against the state current when their box is shipped.
    pub fn scan_special(&mut self) -> Result<(bool, ScannedBalancedText), CommandError> {
        // TeX82 §473 enters `scan_toks` immediately. The preceding optional
        // keyword probe belongs only to pdfTeX 1.40.29 §1534; in particular,
        // an e-TeX job must enter `absorbing` before delivering the opening
        // brace instead of speculatively backing it up and replaying it.
        let deferred =
            self.profile().capabilities().supports_pdftex() && self.scan_keyword("shipout")?.value;
        self.scan_balanced_text(!deferred)
            .map(|text| (deferred, text))
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedMacroDefinition, CommandError> {
        let target = if let Some(target) = self
            .command
            .pending_scan_toks
            .last()
            .and_then(|pending| pending.macro_definition_target(expanded))
        {
            target
        } else {
            let command = self
                .next_non_space_raw()?
                .ok_or(CommandError::input_invariant())?;
            if let Some(target) = command.control_sequence() {
                target
            } else {
                self.back_input(command)?;
                self.scan_definition_target()?
            }
        };
        let scanned =
            self.scan_toks_buffers(ScanToksMode::MacroDefinitionFor { expanded, target })?;
        let provenance = StructuredProvenance {
            primary: scanned.primary,
        };
        Ok(ScannedMacroDefinition {
            target,
            parameter_text: scanned.parameter_text,
            replacement_text: scanned.replacement_text,
            provenance,
            definition_origin: scanned.primary_root,
        })
    }

    /// Scans TeX82 §1221's raw `\let` operand sequence.
    ///
    /// `future` selects `future_let`, whose §1221 body is `get_token;
    /// q:=cur_tok; get_token; back_input; cur_tok:=q; back_input`. Both halves
    /// are ordinary §325 `back_input` calls, so the two tokens are restored on
    /// two separate backup levels -- the second token's level pushed first and
    /// the saved first token's on top of it, which rereads them in their
    /// original order. The meaning defined afterwards is the second token's,
    /// because §325 "doesn't affect `cur_cmd`, `cur_chr`".
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
            self.back_input(second)?;
            self.back_input_saved(first)?;
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
            macro_root: meaning_macro_root(&self.state, meaning),
        })
    }

    /// TeX's `scan_file_name`, returning a typed boundary instead of an input
    /// cursor or a backed-up raw command.
    pub fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        self.command.begin_file_name()?;
        let result = self.scan_file_name_inner();
        self.command.end_file_name();
        result
    }

    fn scan_file_name_inner(&mut self) -> Result<ScannedFileName, CommandError> {
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
        let mut components = FileNameComponents::default();
        let mut character_count = 0usize;
        let mut quoted = false;
        let mut next = None;
        loop {
            let command = match next.take() {
                Some(command) => command,
                None => match self.get_x_token()? {
                    Some(command) => command,
                    None => break,
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
                    break;
                }
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } if !grouped && !quoted => {
                    break;
                }
                Meaning::CharToken { ch, .. } => {
                    character_count += 1;
                    if character_count > FILE_NAME_POOL_CAPACITY {
                        return Err(CommandError::Fatal(crate::FatalError::overflow(
                            "pool size",
                            FILE_NAME_POOL_CAPACITY as i32,
                        )));
                    }
                    components.push_character(ch);
                }
                _ if !grouped => {
                    self.back_input(command)?;
                    break;
                }
                _ => return Err(CommandError::input_invariant()),
            }
        }
        // Web2C tex.ch [29.517] applies `search_string`/`slow_make_string`
        // independently to TeX82 §§516--520's nonempty components.
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                self.state.slow_make_string_pool_string(component);
            }
        }
        Ok(ScannedFileName {
            components,
            provenance,
        })
    }

    /// Scans and opens one input through the borrow-scoped registered-input
    /// capability. No filesystem or host lookup escapes this boundary.
    pub fn open_registered_input(&mut self) -> Result<RegisteredInput, CommandError> {
        let mut file_name = match self.command.take_pending_input_open() {
            Some(file_name) => file_name,
            None => self.scan_file_name()?,
        };
        loop {
            let retry_file_name = file_name.clone();
            let original_name = file_name.packed();
            file_name.components.apply_default_extension(".tex");
            let has_area = !file_name.components.area.is_empty();
            let packed_name = file_name.packed();
            let attempts = crate::host::input_lookup_candidates(&packed_name, has_area);
            self.state.unsupported_host_capability();

            let mut unresolved = false;
            for attempted_name in attempts {
                let Some(registration) = self.host.input(&attempted_name) else {
                    unresolved |= !self.host.input_is_unavailable(&attempted_name);
                    continue;
                };
                let bytes = registration.shared_bytes();
                // §537's `a_make_name_string`: tex.web records the name it
                // actually opened on the level, and later prints exactly that
                // as the transcript's `(name` -- so it is the resolved name,
                // not the name the user typed. Only the host knows what
                // resolving did: web2c's kpathsea answers a bare `child.tex`
                // found beside the job with `./child.tex`, and prints the `./`.
                // A host that reports a resolved name keeps it; one that does
                // not falls back to the name that matched.
                let resolved_name = registration
                    .name()
                    .unwrap_or(attempted_name.as_str())
                    .to_owned();
                let registration = match registration.name() {
                    Some(_) => registration,
                    None => registration.with_name(attempted_name.as_str()),
                };
                let source = self
                    .command
                    .register_source(registration)
                    .map_err(|_| CommandError::input_invariant())?;
                self.command
                    .open_registered_source(source)
                    .map_err(|_| CommandError::input_invariant())?;
                // e-TeX 2.6 [23.328]'s `grp_stack[in_open]:=cur_boundary;
                // if_stack[in_open]:=cond_ptr`, recorded for `\tracingnesting`'s
                // `file_warning` at this level's eventual `end_file_reading`.
                if let Some(level) = self.command.top_input_level_identity() {
                    self.command.record_source_open_depths(
                        level,
                        self.state.group_lineages().into_boxed_slice(),
                        self.command
                            .conditions
                            .frames
                            .iter()
                            .map(|frame| frame.identity.0)
                            .collect::<Vec<_>>()
                            .into_boxed_slice(),
                    );
                }
                let endlinechar = self.state.int_param(IntParam::END_LINE_CHAR);
                self.command
                    .prepare_started_input(endlinechar)
                    .ok_or_else(CommandError::input_invariant)?;
                self.host.initialize_job_name(&attempted_name);
                // TeX82 §537 retains `a_make_name_string` for the opened
                // request; Web2C additionally retains its full resolved name.
                self.state.make_string_pool_string(&attempted_name);
                if resolved_name != attempted_name {
                    self.state.make_string_pool_string(&resolved_name);
                }
                if attempted_name != packed_name {
                    file_name.components.area = "TeXinputs:".to_owned();
                }
                return Ok(RegisteredInput {
                    file_name,
                    source,
                    bytes,
                });
            }
            if unresolved {
                self.command.retain_pending_input_open(retry_file_name);
                return Err(CommandError::MissingInput {
                    name: packed_name,
                    original_name,
                });
            }
            file_name = self.prompt_for_input_file_name(&file_name)?;
        }
    }

    /// TeX82 §530's `prompt_file_name("input file name", ".tex")` after
    /// the retained host has authoritatively answered that both §537 input
    /// candidates are absent.
    fn prompt_for_input_file_name(
        &mut self,
        missing: &ScannedFileName,
    ) -> Result<ScannedFileName, CommandError> {
        let context = self.command.output_open_context(&self.state);
        self.state
            .printer()
            .print_nl("! I can't find file `")
            .print(&missing.packed())
            .print("'.")
            .print_rendered(&context)
            .print_nl("Please type another input file name");

        if !self.state.interaction_permits_terminal_input() {
            let help = "*** (job aborted, file error in nonstop mode)";
            let mut report = self.state.print_err("Emergency stop");
            report.help(&[help]).context(context);
            report.succumb();
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(
                "job aborted, file error in nonstop mode",
            )));
        }

        let Some(line) = self
            .state
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: ": " })
        else {
            let help = "End of file on the terminal!";
            let mut report = self.state.print_err("Emergency stop");
            report.help(&[help]).context(context);
            report.succumb();
            return Err(CommandError::Fatal(crate::FatalError::emergency_stop(help)));
        };
        self.file_name_from_terminal_line(&line)
    }

    fn file_name_from_terminal_line(
        &mut self,
        line: &str,
    ) -> Result<ScannedFileName, CommandError> {
        let mut components = FileNameComponents::default();
        let mut quoted = false;
        let mut character_count = 0usize;
        for ch in line.chars().skip_while(|ch| *ch == ' ') {
            if ch == '"' {
                quoted = !quoted;
                continue;
            }
            if ch == ' ' && !quoted {
                break;
            }
            character_count += 1;
            if character_count > FILE_NAME_POOL_CAPACITY {
                return Err(CommandError::Fatal(crate::FatalError::overflow(
                    "pool size",
                    FILE_NAME_POOL_CAPACITY as i32,
                )));
            }
            components.push_character(ch);
        }
        for component in [&components.area, &components.name, &components.extension] {
            if !component.is_empty() {
                self.state.slow_make_string_pool_string(component);
            }
        }
        Ok(ScannedFileName {
            components,
            provenance: StructuredProvenance {
                primary: OriginId::UNKNOWN,
            },
        })
    }

    /// TeX82 §1215's `repeat get_token until cur_tok<>space_token`.
    ///
    /// This tests the raw spelling, not `cur_cmd`: a control sequence whose
    /// current meaning is a space remains a legal definition target.
    fn next_non_space_raw(&mut self) -> Result<Option<crate::CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_token()? else {
                return Ok(None);
            };
            if !matches!(
                command.spelling().semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(Some(command));
            }
        }
    }
}

fn provenance(scanned: &ScannedToks) -> StructuredProvenance {
    StructuredProvenance {
        primary: scanned.primary,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "filename/tests.rs"]
mod filename_tests;

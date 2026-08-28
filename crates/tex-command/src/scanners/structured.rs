//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename scanning only. Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use std::sync::Arc;

use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, ResolvedMeaning, UnexpandablePrimitive};
use tex_state::scaled::{FontSizeSpec, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{
    SourceId,
    env::banks::{GlueParam, IntParam},
};

use crate::attempt::{AttemptDefinitionId, AttemptTokenBufferId, AttemptTokenListId};

use crate::input::{
    BackupTreatment, InputLevelId, PackedTokenSpanHandle, ReplayTrace, RetirementBehavior,
    StoredReplayReason, TokenBehavior,
};
use crate::processor::alignment::{PREAMBLE_ALIGN_STATE, is_character_command};
use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerEpisode, ScannerStatus, ScannerStatusVisibility,
    ScannerWarning, TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::scanners::RestrictedIntegerClass;
use crate::{
    AlignmentCellTemplates, AlignmentIdentity, AlignmentPreamble, CommandError, CommandProcessor,
    CurrentCommand, InternalValue,
    processor::{
        DeliveryStatus, print_cs_text, render_the_value, selector_meaning_text, string_text,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlignmentPreamblePhase {
    UTemplate,
    VTemplate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlignmentPreambleChildDestination {
    SpanExpansion,
    Scalar,
}

#[derive(Debug, Eq, PartialEq)]
struct PendingPreambleSpanExpansion<G> {
    command: CurrentCommand<G>,
    child:
        Option<crate::execution_scratch::ChildContinuation<G, AlignmentPreambleChildDestination>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlignmentPreambleScalarPhase {
    TabskipEquals,
    TabskipGlue,
}

#[derive(Debug, Eq, PartialEq)]
struct PendingPreambleScalar<G> {
    phase: AlignmentPreambleScalarPhase,
    child:
        Option<crate::execution_scratch::ChildContinuation<G, AlignmentPreambleChildDestination>>,
}

/// Exact in-process owner of an alignment preamble suspended while expanding
/// the token following `\span`.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingAlignmentPreamble<G> {
    alignment: AlignmentIdentity,
    builder: TokenBuilderId,
    scanner_episode: ScannerEpisode,
    columns: Vec<AlignmentCellTemplates>,
    tabskips: Vec<GlueSpec>,
    current_tabskip: GlueSpec,
    repeat_start: Option<usize>,
    u_template: AttemptTokenBufferId,
    v_template: AttemptTokenBufferId,
    phase: AlignmentPreamblePhase,
    span_expansion: Option<PendingPreambleSpanExpansion<G>>,
    scalar_scan: Option<PendingPreambleScalar<G>>,
}

impl<G> PendingAlignmentPreamble<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.scalar_scan
            .as_mut()
            .and_then(|pending| pending.child.take())
            .or_else(|| {
                self.span_expansion
                    .as_mut()
                    .and_then(|pending| pending.child.take())
            })
            .map(|child| child.restore().0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StructuredScannerChildDestination {
    Scalar,
    TokenListRightHandSide,
    PdfObjectStreamAttribute,
    PdfObjectData,
    PdfFormAttribute,
    PdfFormResources,
    PdfGlyphName,
    PdfGlyphUnicode,
    PdfImageAttribute,
    PdfImagePageName,
    PdfGraphicsLiteral,
    PdfColorStackText,
    SpecialText,
    PdfNavigationAnnotationEntries,
    PdfNavigationAttributes,
    PdfNavigationTitle,
    PdfNavigationIdentifier,
    PdfDocumentFragmentText,
    PdfActionUser,
    PdfActionFile,
    PdfActionStructure,
    PdfActionPageView,
    PdfActionTargetName,
    ImmediateChild,
    WriteExpansionText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PendingPdfActionOwner {
    StartLink {
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: Option<ScannedBalancedText>,
    },
    Outline {
        attributes: Option<ScannedBalancedText>,
    },
    DocumentFragment {
        kind: tex_state::PdfDocumentFragmentKind,
        text: ScannedBalancedText,
    },
}

#[derive(Debug, Eq, PartialEq)]
enum PendingPdfColorStackAction {
    Set,
    Push,
}

#[derive(Debug, Eq, PartialEq)]
enum PendingPdfActionPhase {
    User,
    File {
        goto: bool,
    },
    StructureRaw {
        goto: bool,
        file: AttemptTokenListId,
    },
    StructureName {
        goto: bool,
        file: Option<AttemptTokenListId>,
    },
    PageView {
        goto: bool,
        file: Option<AttemptTokenListId>,
        structure: Option<PdfActionIdentifier>,
        number: u32,
    },
    TargetName {
        goto: bool,
        file: Option<AttemptTokenListId>,
        structure: Option<PdfActionIdentifier>,
    },
}

#[derive(Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub(super) enum PendingStructuredScannerPhase<G> {
    PdfObjectStreamAttribute {
        use_object: Option<i32>,
    },
    PdfObjectData {
        use_object: Option<i32>,
        stream: bool,
        stream_attr: Option<ScannedBalancedText>,
        file: bool,
    },
    PdfFormAttribute,
    PdfFormResources {
        attr: Option<ScannedBalancedText>,
    },
    PdfGlyphName {
        primitive: UnexpandablePrimitive,
        font: Option<FontId>,
    },
    PdfGlyphUnicode {
        font: Option<FontId>,
        first: AttemptTokenListId,
    },
    PdfImageAttribute {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    PdfImagePageName {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        attr: Option<AttemptTokenListId>,
    },
    PdfGraphicsLiteral {
        mode: tex_state::node::PdfLiteralMode,
        deferred: bool,
    },
    PdfColorStackText {
        id: i32,
        action: PendingPdfColorStackAction,
    },
    SpecialText {
        deferred: bool,
    },
    PdfAnnotationEntries {
        use_object: Option<i32>,
        dimensions: tex_state::PdfAnnotationDimensions,
    },
    PdfStartLinkAttributes {
        dimensions: tex_state::PdfAnnotationDimensions,
    },
    PdfOutlineAttributes,
    PdfOutlineTitle {
        attributes: Option<ScannedBalancedText>,
        action: PdfActionSpec,
        count: i32,
    },
    PdfThreadAttributes {
        primitive: UnexpandablePrimitive,
        dimensions: tex_state::PdfAnnotationDimensions,
    },
    PdfThreadIdentifier {
        primitive: UnexpandablePrimitive,
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: Option<ScannedBalancedText>,
    },
    PdfDestinationIdentifier {
        structure: Option<u32>,
    },
    PdfDocumentFragmentText {
        kind: tex_state::PdfDocumentFragmentKind,
    },
    PdfAction {
        owner: PendingPdfActionOwner,
        phase: PendingPdfActionPhase,
    },
    Scalar(PendingStructuredScalarPhase<G>),
    TokenListRightHandSide(PendingTokenListOwner),
    Immediate(PendingImmediatePhase),
    WriteExpansion {
        tokens: AttemptTokenListId,
        stopper_level: InputLevelId,
        write_words: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingTokenListOwner {
    Register {
        owner: Symbol,
        index: u16,
    },
    Value {
        owner: Symbol,
    },
    Parameter {
        parameter: tex_state::env::banks::TokParam,
        owner: Symbol,
    },
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub(super) enum PendingStructuredScalarPhase<G> {
    Unary(StructuredUnaryScalar),
    CharacterDefinitionEquals {
        target: Symbol,
        provisional_old: ResolvedMeaning<G>,
        class: RestrictedIntegerClass,
    },
    CharacterDefinitionValue {
        target: Symbol,
        provisional_old: ResolvedMeaning<G>,
        class: RestrictedIntegerClass,
    },
    RegisterDefinitionEquals {
        target: Symbol,
        provisional_old: ResolvedMeaning<G>,
    },
    RegisterDefinitionIndex {
        target: Symbol,
        provisional_old: ResolvedMeaning<G>,
    },
    GlueParameterEquals {
        index: u16,
        mu: bool,
    },
    GlueParameterValue {
        index: u16,
        mu: bool,
    },
    VSplitIndex,
    VSplitTo {
        index: u16,
    },
    VSplitHeight {
        index: u16,
        missing_to_context: Option<String>,
    },
    Rule {
        primitive: UnexpandablePrimitive,
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        phase: RuleScalarPhase,
    },
    SpecialKeyword,
    Packing {
        owner: PackingOwner,
        phase: PackingScalarPhase,
    },
    InsertPre,
    InsertClass {
        pre: bool,
    },
    BoxShiftDimension {
        primitive: UnexpandablePrimitive,
    },
    PdfGraphics {
        primitive: UnexpandablePrimitive,
        phase: PdfGraphicsScalarPhase,
    },
    InputStream {
        primitive: UnexpandablePrimitive,
        read_global: bool,
        phase: InputStreamScalarPhase,
    },
    FontDefinition {
        target: Symbol,
        phase: FontDefinitionScalarPhase,
    },
    GeneratedFont {
        kind: GeneratedFontKind,
        target: Symbol,
        phase: GeneratedFontScalarPhase,
    },
    MathFractionThickness {
        kind: MathFractionKind,
        left_delimiter: Option<ScannedMathDelimiter>,
        right_delimiter: Option<ScannedMathDelimiter>,
    },
    AccentBaseCharacter {
        provenance: StructuredProvenance,
    },
    SetBoxIndex,
    SetBoxEquals {
        index: u16,
    },
    TokenListEquals(PendingTokenListOwner),
    TokenRegisterIndex {
        owner: Symbol,
    },
    TokenListRhsRegister(PendingTokenListOwner),
    PdfImage(PdfImageScalarProgress),
    PdfObject(PdfObjectScalarProgress),
    PdfForm(PdfFormScalarProgress),
    PdfDocumentOpenAction {
        kind: tex_state::PdfDocumentFragmentKind,
        text: ScannedBalancedText,
    },
    PdfOutlineCount {
        attributes: Option<ScannedBalancedText>,
        action: PdfActionSpec,
        phase: PdfOutlineScalarPhase,
    },
    PdfThreadIdentifier {
        primitive: UnexpandablePrimitive,
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: Option<ScannedBalancedText>,
        phase: PdfThreadScalarPhase,
    },
    ImmediateOpenOut(ImmediateOpenOutScalarPhase),
    ImmediateWriteStream {
        close: bool,
    },
    PdfAction {
        owner: PendingPdfActionOwner,
        progress: PdfActionScalarProgress,
    },
    PdfNavigation(PdfNavigationScalarProgress),
    PdfFontAction {
        primitive: UnexpandablePrimitive,
    },
    MathFieldRestricted {
        provenance: StructuredProvenance,
        kind: MathFieldRestrictedKind,
    },
    LeaderRegister {
        copy: bool,
    },
    Hyphenation(crate::scanners::hyphenation::PendingHyphenationData),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfNavigationScalarProgress {
    primitive: UnexpandablePrimitive,
    use_object: Option<i32>,
    dimensions: tex_state::PdfAnnotationDimensions,
    attributes: Option<ScannedBalancedText>,
    structure: Option<u32>,
    identifier: Option<PdfActionIdentifier>,
    phase: PdfNavigationScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfNavigationScalarPhase {
    AnnotationReserve,
    AnnotationUse,
    AnnotationUseObject,
    WidthKeyword,
    WidthDimension,
    HeightKeyword,
    HeightDimension,
    DepthKeyword,
    DepthDimension,
    AttributeKeyword,
    DestinationStructure,
    DestinationStructureValue,
    DestinationName,
    DestinationNumber,
    DestinationNumberValue,
    DestinationXyz,
    DestinationZoom,
    DestinationZoomValue,
    DestinationFitBh,
    DestinationFitBv,
    DestinationFitB,
    DestinationFitH,
    DestinationFitV,
    DestinationFitR,
    DestinationFit,
    FitRWidthKeyword,
    FitRWidthDimension,
    FitRHeightKeyword,
    FitRHeightDimension,
    FitRDepthKeyword,
    FitRDepthDimension,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfActionScalarProgress {
    goto: Option<bool>,
    file: Option<AttemptTokenListId>,
    structure: Option<PdfActionIdentifier>,
    target: Option<PdfActionTarget>,
    phase: PdfActionScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfActionScalarPhase {
    UserKeyword,
    GotoKeyword,
    ThreadKeyword,
    FileKeyword,
    StructureKeyword,
    StructureNameKeyword,
    StructureNumberKeyword,
    StructureNumber,
    PageKeyword,
    PageNumber,
    NameKeyword,
    NumberKeyword,
    Number,
    NewWindowKeyword,
    NoNewWindowKeyword,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImmediateOpenOutScalarPhase {
    Stream,
    Equals { stream: u8 },
    FileName { stream: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingImmediatePhase {
    WriteText {
        stream: WriteStreamSelector,
    },
    WriteExpansion {
        stream: WriteStreamSelector,
        tokens: AttemptTokenListId,
    },
    Pdf {
        primitive: UnexpandablePrimitive,
        pdf_output_enabled: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfOutlineScalarPhase {
    CountKeyword,
    CountValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfThreadScalarPhase {
    NameKeyword,
    NumKeyword,
    NumValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfObjectScalarProgress {
    use_object: Option<i32>,
    stream: bool,
    stream_attr: Option<ScannedBalancedText>,
    phase: PdfObjectScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfObjectScalarPhase {
    ReserveKeyword,
    UseKeyword,
    UseObject,
    StreamKeyword,
    AttributeKeyword,
    FileKeyword,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PdfFormScalarProgress {
    attr: Option<ScannedBalancedText>,
    resources: Option<ScannedBalancedText>,
    phase: PdfFormScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfFormScalarPhase {
    AttributeKeyword,
    ResourcesKeyword,
    BoxRegister,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PdfImageScalarProgress {
    width: Option<Scaled>,
    height: Option<Scaled>,
    depth: Option<Scaled>,
    attr: Option<AttemptTokenListId>,
    page: PendingPdfImagePage,
    color_space_object: i32,
    page_box: Option<PdfImagePageBox>,
    phase: PdfImageScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingPdfImagePage {
    Unset,
    Number(i32),
    Named(AttemptTokenListId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfImageScalarPhase {
    WidthKeyword,
    WidthDimension,
    HeightKeyword,
    HeightDimension,
    DepthKeyword,
    DepthDimension,
    AttributeKeyword,
    NamedKeyword,
    PageKeyword,
    PageNumber,
    ColorSpaceKeyword,
    ColorSpaceObject,
    MediaBox,
    CropBox,
    BleedBox,
    TrimBox,
    ArtBox,
    FileName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum InputStreamScalarPhase {
    Selector,
    OpenEquals { scanned: crate::RestrictedInteger },
    OpenFileName { scanned: crate::RestrictedInteger },
    ReadTo { stream: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FontDefinitionScalarPhase {
    Equals,
    FileName,
    AtKeyword { file_name: ScannedFileName },
    AtDimension { file_name: ScannedFileName },
    ScaledKeyword { file_name: ScannedFileName },
    ScaledInteger { file_name: ScannedFileName },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedFontScalarPhase {
    Equals,
    Source,
    Amount { source: FontId },
    NoLigatures { source: FontId, amount: i16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfGraphicsScalarPhase {
    LiteralShipout,
    LiteralDirect { deferred: bool },
    LiteralPage { deferred: bool },
    ColorId,
    ColorSet { id: i32 },
    ColorPush { id: i32 },
    ColorPop { id: i32 },
    ColorCurrent { id: i32 },
    SnapY,
    SnapYComp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuleScalarPhase {
    WidthKeyword,
    WidthDimension,
    HeightKeyword,
    HeightDimension,
    DepthKeyword,
    DepthDimension,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackingOwner {
    Box(UnexpandablePrimitive),
    Alignment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackingScalarPhase {
    ToKeyword,
    SpreadKeyword,
    Dimension { exactly: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StructuredUnaryScalar {
    MathCharacter,
    DelimiterNumber,
    MathFamily(MathFamilySize),
    MathMu(bool),
    Accent,
    WriteStream,
    BoxRegister,
    ShowBox,
    PdfFormReference,
    PdfReferenceObject,
    ShowThe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MathFieldRestrictedKind {
    Character,
    MathCharacter,
    Delimiter,
}

/// Exact structured-scanner caller and operand destination retained across a
/// nested immutable-resource suspension.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PendingStructuredScanner<G> {
    pub(super) phase: PendingStructuredScannerPhase<G>,
    pub(super) child:
        Option<crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>>,
}

impl<G> PendingStructuredScanner<G> {
    pub(crate) fn take_child(&mut self) -> Option<crate::execution_scratch::ScannerFrameKey<G>> {
        self.child.take().map(|child| child.restore().0)
    }
}

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

fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

/// Provenance for a completed structured scan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StructuredProvenance {
    /// Origin of the first non-ignored token accepted by the scan.
    pub primary: OriginId,
}

/// A balanced token list frozen through the aggregate token store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedBalancedText {
    pub tokens: AttemptTokenListId,
    pub provenance: StructuredProvenance,
}

/// Attempt-local PDF action identifier. Token text remains in the sole live
/// command attempt until the executor promotes the completed request.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionIdentifier {
    Name(AttemptTokenListId),
    Number(u32),
    Raw(AttemptTokenListId),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionTarget {
    Page {
        number: u32,
        view: AttemptTokenListId,
    },
    Destination(PdfActionIdentifier),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfActionDestination {
    pub file: Option<AttemptTokenListId>,
    pub structure: Option<PdfActionIdentifier>,
    pub target: PdfActionTarget,
    pub window: tex_state::PdfActionWindow,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfActionSpec {
    User(AttemptTokenListId),
    GoTo(PdfActionDestination),
    Thread(PdfActionDestination),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpandedWriteText {
    pub tokens: AttemptTokenListId,
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
    pub definition: AttemptDefinitionId,
    pub parameter_text: AttemptTokenListId,
    pub replacement_text: AttemptTokenListId,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 `\let` or `\futurelet` assignment.
///
/// The command processor owns every raw operand delivery, including the
/// optional equals sign and `\futurelet`'s lookahead replay. Replay receives
/// only the target and its already-resolved source meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedLetAssignment<G> {
    pub target: Symbol,
    pub source: Option<Symbol>,
    pub meaning: ResolvedMeaning<G>,
}

/// A completed TeX82 §1224 `\\chardef` or `\\mathchardef` operand.
///
/// Command processing owns the raw target, optional equals sign, and the
/// class-restricted integer scan (§434 or §436) including its recovery. Main
/// control receives no token or input capability: it only applies the
/// assignment's effective scope and reports the recovery diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedCharacterDefinition<G> {
    pub target: Symbol,
    /// The meaning replaced by §1224's scanner-time provisional `\relax`.
    pub provisional_old: ResolvedMeaning<G>,
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
pub struct ScannedRegisterDefinition<G> {
    pub target: Symbol,
    /// The meaning replaced by §1224's scanner-time provisional `\relax`.
    pub provisional_old: ResolvedMeaning<G>,
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
    pub attr: Option<AttemptTokenListId>,
}

impl PdfImageRequest {
    /// Whether two requests select the same immutable host image resource.
    ///
    /// pdftex.web §1550's `read_image` receives the file/page/page-box facts;
    /// rule dimensions and `attr` are command/output state. Dimensions remain
    /// in this deliberately conservative key, but `attr` cannot: its
    /// Attribute text is command-attempt state, not part of host resource
    /// identity. A retried request carries the coordinate in its owned
    /// continuation arena.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedPdfFontAction {
    pub font: Option<FontId>,
    pub first: Option<AttemptTokenListId>,
    pub second: Option<AttemptTokenListId>,
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
    pub open_action: Option<PdfActionSpec>,
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
    pub action: PdfActionSpec,
}

/// Fully scanned `\\pdfoutline` document-state mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfOutlineRequest {
    pub attributes: Option<ScannedBalancedText>,
    pub action: PdfActionSpec,
    pub count: i32,
    pub title: ScannedBalancedText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationRequest {
    pub structure: Option<u32>,
    pub identifier: PdfActionIdentifier,
    pub kind: tex_state::node::PdfDestinationKind,
}

/// Fully scanned `\\pdfthread` or `\\pdfstartthread` marker.  The
/// dimensions deliberately retain running values: pdfTeX resolves them while
/// traversing the containing box at shipout, not while it scans the command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfThreadRequest {
    pub dimensions: tex_state::PdfAnnotationDimensions,
    pub attributes: Option<ScannedBalancedText>,
    pub identifier: PdfActionIdentifier,
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
// Assignment delivery is an allocation-free handoff consumed immediately;
// boxing it would put a heap allocation in ordinary accent lookahead.
#[allow(clippy::large_enum_variant)]
pub enum ScannedAccentBase<G> {
    /// §1124's `letter`, `other_char`, `char_given`, or `char_num` base.
    Character {
        character: u8,
        provenance: StructuredProvenance,
    },
    /// §1270's `prefixed_command`: the delivered assignment the executor must
    /// run before the lookahead continues.
    Assignment(CurrentCommand<G>),
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
    /// Applies TeX82 §§516--519's platform-independent component scan to an
    /// already selected startup name.
    #[must_use]
    pub fn from_tex_name(value: &str) -> Self {
        let mut components = Self::default();
        for ch in value.chars() {
            components.push_character(ch);
        }
        components
    }

    #[must_use]
    pub fn packed(&self) -> String {
        format!("{}{}{}", self.area, self.name, self.extension)
    }

    pub fn apply_default_extension(&mut self, extension: &str) {
        if self.extension.is_empty() {
            self.extension.push_str(extension);
        }
    }

    pub(crate) fn push_character(&mut self, ch: char) {
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

pub(crate) const FILE_NAME_POOL_CAPACITY: usize = 32_000;

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
        tokens: AttemptTokenListId,
        /// Parameterless macro definition allocated in the same command
        /// attempt as `tokens`; both roots are promoted atomically before
        /// semantic apply.
        definition: AttemptDefinitionId,
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
        tokens: AttemptTokenListId,
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

impl<G> CommandProcessor<'_, '_, G> {
    pub(super) fn take_pending_structured_scanner(
        &mut self,
    ) -> Result<Option<PendingStructuredScanner<G>>, CommandError> {
        if !self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_structured_scanner)
        {
            return Ok(None);
        }
        let key = self
            .scanner_resume
            .take()
            .expect("matched structured-scanner frame");
        self.command
            .scratch
            .take_structured_scanner_frame(key)
            .map(Some)
            .map_err(crate::scan_toks::scratch_command_error)
    }

    pub(super) fn restore_structured_scanner_child(
        &mut self,
        child: &mut Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
        expected: StructuredScannerChildDestination,
    ) -> Result<(), CommandError> {
        if let Some(child) = child.take() {
            let (key, destination) = child.restore();
            if destination != expected {
                self.abort_continuation(key)?;
                return Err(CommandError::input_invariant());
            }
            self.install_scanner_resume(Some(key));
        }
        Ok(())
    }

    pub(super) fn retain_structured_scanner(
        &mut self,
        phase: PendingStructuredScannerPhase<G>,
        destination: StructuredScannerChildDestination,
    ) -> Result<(), CommandError> {
        let key = match self
            .command
            .scratch
            .store_structured_scanner_frame(PendingStructuredScanner { phase, child: None })
        {
            Ok(key) => key,
            Err(error) => {
                if let Some(child) = self.scanner_resume.take() {
                    self.abort_continuation(child)?;
                }
                return Err(crate::scan_toks::scratch_command_error(error));
            }
        };
        let pending = match self.command.scratch.structured_scanner_frame_mut(&key) {
            Ok(pending) => pending,
            Err(error) => {
                let abort_result = if let Some(child) = self.scanner_resume.take() {
                    self.abort_continuation(child)
                } else {
                    Ok(())
                };
                let discard_result = self
                    .command
                    .scratch
                    .discard_structured_scanner_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error);
                abort_result?;
                discard_result?;
                return Err(crate::scan_toks::scratch_command_error(error));
            }
        };
        pending.child = crate::execution_scratch::ChildContinuation::capture(
            &mut self.scanner_resume,
            destination,
        );
        if self.scanner_resume.replace(key).is_some() {
            return Err(CommandError::input_invariant());
        }
        Ok(())
    }

    pub(super) fn retain_structured_scalar<T>(
        &mut self,
        result: crate::RetainedScalarScan<G, T>,
        phase: PendingStructuredScalarPhase<G>,
    ) -> Result<T, CommandError> {
        match result {
            crate::RetainedScalarScan::Complete(value) => Ok(value),
            crate::RetainedScalarScan::Failed(error) => Err(error),
            crate::RetainedScalarScan::Suspended { error, child } => {
                let key = match self.command.scratch.store_structured_scanner_frame(
                    PendingStructuredScanner {
                        phase: PendingStructuredScannerPhase::Scalar(phase),
                        child: None,
                    },
                ) {
                    Ok(key) => key,
                    Err(store_error) => {
                        self.abort_continuation(child)?;
                        return Err(crate::scan_toks::scratch_command_error(store_error));
                    }
                };
                match self.command.scratch.structured_scanner_frame_mut(&key) {
                    Ok(pending) => {
                        pending.child =
                            Some(crate::execution_scratch::ChildContinuation::from_key(
                                child,
                                StructuredScannerChildDestination::Scalar,
                            ));
                    }
                    Err(store_error) => {
                        let abort_result = self.abort_continuation(child);
                        let discard_result = self
                            .command
                            .scratch
                            .discard_structured_scanner_frame(key)
                            .map_err(crate::scan_toks::scratch_command_error);
                        abort_result?;
                        discard_result?;
                        return Err(crate::scan_toks::scratch_command_error(store_error));
                    }
                }
                if self.scanner_resume.replace(key).is_some() {
                    return Err(CommandError::input_invariant());
                }
                Err(error)
            }
        }
    }

    pub(super) fn retain_structured_scalar_progress<T>(
        &mut self,
        result: crate::RetainedScalarScan<G, T>,
        phase: PendingStructuredScalarPhase<G>,
    ) -> Result<(T, PendingStructuredScalarPhase<G>), CommandError> {
        match result {
            crate::RetainedScalarScan::Complete(value) => Ok((value, phase)),
            crate::RetainedScalarScan::Failed(error) => Err(error),
            crate::RetainedScalarScan::Suspended { error, child } => {
                let key = match self.command.scratch.store_structured_scanner_frame(
                    PendingStructuredScanner {
                        phase: PendingStructuredScannerPhase::Scalar(phase),
                        child: None,
                    },
                ) {
                    Ok(key) => key,
                    Err(store_error) => {
                        self.abort_continuation(child)?;
                        return Err(crate::scan_toks::scratch_command_error(store_error));
                    }
                };
                match self.command.scratch.structured_scanner_frame_mut(&key) {
                    Ok(pending) => {
                        pending.child =
                            Some(crate::execution_scratch::ChildContinuation::from_key(
                                child,
                                StructuredScannerChildDestination::Scalar,
                            ));
                    }
                    Err(store_error) => {
                        let abort_result = self.abort_continuation(child);
                        let discard_result = self
                            .command
                            .scratch
                            .discard_structured_scanner_frame(key)
                            .map_err(crate::scan_toks::scratch_command_error);
                        abort_result?;
                        discard_result?;
                        return Err(crate::scan_toks::scratch_command_error(store_error));
                    }
                }
                if self.scanner_resume.replace(key).is_some() {
                    return Err(CommandError::input_invariant());
                }
                Err(error)
            }
        }
    }

    fn restore_structured_unary(
        &mut self,
        expected: StructuredUnaryScalar,
    ) -> Result<(), CommandError> {
        let Some(pending) = self.take_pending_structured_scanner()? else {
            return Ok(());
        };
        let PendingStructuredScanner { phase, mut child } = pending;
        match phase {
            PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::Unary(site))
                if site == expected =>
            {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )
            }
            _ => {
                if let Some(child) = child.take() {
                    self.abort_continuation(child.restore().0)?;
                }
                Err(CommandError::input_invariant())
            }
        }
    }

    fn finish_structured_unary<T>(
        &mut self,
        result: crate::RetainedScalarScan<G, T>,
        site: StructuredUnaryScalar,
    ) -> Result<T, CommandError> {
        self.retain_structured_scalar(result, PendingStructuredScalarPhase::Unary(site))
    }

    /// Expands a frozen whatsit payload at output traversal time.
    ///
    /// The caller decides how the resulting token spellings are rendered;
    /// this operation owns only canonical replay/expansion state.
    pub fn expand_output_replay(
        &mut self,
        tokens: tex_state::TokenListId<G>,
    ) -> Result<crate::attempt::AttemptTokenListId, CommandError> {
        let episode = self.command.push_output_replay_episode(self.state, tokens);
        let expanded = self
            .command
            .attempt
            .arena_mut()
            .allocate_token_buffer()
            .map_err(|_| CommandError::input_invariant())?;
        let mut destination = None;
        loop {
            match self.get_x_or_protected_with_replay_completion_into(&mut destination)? {
                DeliveryStatus::Command => {
                    let command = destination.take().ok_or(CommandError::input_invariant())?;
                    self.command
                        .attempt
                        .arena_mut()
                        .push_buffer_token(expanded, command.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                }
                DeliveryStatus::ReplayCompleted(completed) if completed == episode => break,
                DeliveryStatus::ReplayCompleted(_) => continue,
                DeliveryStatus::End => return Err(CommandError::input_invariant()),
                _ => return Err(CommandError::input_invariant()),
            }
        }
        self.command
            .attempt
            .arena_mut()
            .finish_token_buffer(expanded)
            .map_err(|_| CommandError::input_invariant())
    }

    /// TeX82 §1215's `get_r_token`, including its restart after inserting
    /// the inaccessible target. The rejected delivery is backed up, so the
    /// caller's following operand scan still owns it.
    fn delivered_definition_target(
        &mut self,
        command: &crate::CurrentCommand<G>,
    ) -> Option<tex_state::interner::Symbol> {
        command
            .control_sequence()
            .or_else(|| match command.spelling().semantic_token() {
                Token::Char {
                    ch,
                    cat: Catcode::Active,
                } => Some(self.state.intern_active_character(ch)),
                _ => None,
            })
    }

    fn scan_definition_target(&mut self) -> Result<tex_state::interner::Symbol, CommandError> {
        let mut destination = None;
        loop {
            let command = match self.next_non_space_raw_into(&mut destination)? {
                DeliveryStatus::Command => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                DeliveryStatus::End => {
                    if self.next_non_space_raw_into(&mut destination)? != DeliveryStatus::Command {
                        return Err(CommandError::input_invariant());
                    }
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                _ => return Err(CommandError::input_invariant()),
            };
            if let Some(target) = self.delivered_definition_target(&command) {
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
            let context = self.command.output_open_context(self.state);
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
            let outcome = report.error();
            self.finish_error_outcome(outcome)?;
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
    ) -> Result<ScannedCharacterDefinition<G>, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (target, provisional_old, class, value_phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::CharacterDefinitionEquals {
                            target,
                            provisional_old,
                            class,
                        },
                    ),
                child,
            }) => (target, provisional_old, class, false, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::CharacterDefinitionValue {
                            target,
                            provisional_old,
                            class,
                        },
                    ),
                child,
            }) => (target, provisional_old, class, true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => {
                let target = self.scan_definition_target()?;
                let provisional_old = self.state.meaning(target);
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
                (target, provisional_old, class, false, None)
            }
        };
        if !value_phase {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let equals = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                equals,
                PendingStructuredScalarPhase::CharacterDefinitionEquals {
                    target,
                    provisional_old: provisional_old.clone(),
                    class,
                },
            )?;
        }
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let value = self.scan_restricted_integer_retained(class);
        let scanned = self.retain_structured_scalar(
            value,
            PendingStructuredScalarPhase::CharacterDefinitionValue {
                target,
                provisional_old: provisional_old.clone(),
                class,
            },
        )?;
        Ok(ScannedCharacterDefinition {
            target,
            provisional_old,
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
    ) -> Result<ScannedRegisterDefinition<G>, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (target, provisional_old, index_phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::RegisterDefinitionEquals {
                            target,
                            provisional_old,
                        },
                    ),
                child,
            }) => (target, provisional_old, false, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::RegisterDefinitionIndex {
                            target,
                            provisional_old,
                        },
                    ),
                child,
            }) => (target, provisional_old, true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => {
                let target = self.scan_definition_target()?;
                let provisional_old = self.state.meaning(target);
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
                (target, provisional_old, false, None)
            }
        };
        if !index_phase {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let equals = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                equals,
                PendingStructuredScalarPhase::RegisterDefinitionEquals {
                    target,
                    provisional_old: provisional_old.clone(),
                },
            )?;
        }
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        // TeX82 §1224 uses `scan_eight_bit_int`, while e-TeX 2.6
        // etex.ch [49.1224] replaces that scan with `scan_register_num` so
        // sparse register shorthands may address 0..=32767. pdfTeX inherits
        // the same e-TeX register extension.
        let index = if self.command.profile().capabilities().supports_etex() {
            let result = self.scan_extended_register_index_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::RegisterDefinitionIndex {
                    target,
                    provisional_old: provisional_old.clone(),
                },
            )?
        } else {
            let result = self.scan_eight_bit_register_index_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::RegisterDefinitionIndex {
                    target,
                    provisional_old: provisional_old.clone(),
                },
            )?
        };
        Ok(ScannedRegisterDefinition {
            target,
            provisional_old,
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

        if let Some(pending) = self.take_pending_structured_scanner()? {
            let PendingStructuredScanner { phase, mut child } = pending;
            return match phase {
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::PdfGraphics {
                        primitive: retained_primitive,
                        phase,
                    },
                ) if retained_primitive == primitive => {
                    self.scan_pdf_graphics_scalar(primitive, phase, child)
                }
                PendingStructuredScannerPhase::PdfGraphicsLiteral { mode, deferred } => {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::PdfGraphicsLiteral,
                    )?;
                    let text = match self.scan_balanced_text(!deferred) {
                        Ok(text) => text,
                        Err(error) => {
                            if error.is_resource_suspension() {
                                self.retain_structured_scanner(
                                    PendingStructuredScannerPhase::PdfGraphicsLiteral {
                                        mode,
                                        deferred,
                                    },
                                    StructuredScannerChildDestination::PdfGraphicsLiteral,
                                )?;
                            }
                            return Err(error);
                        }
                    };
                    Ok(Some(Request::Literal {
                        mode,
                        deferred,
                        text,
                    }))
                }
                PendingStructuredScannerPhase::PdfColorStackText { id, action } => {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::PdfColorStackText,
                    )?;
                    let text = match self.scan_balanced_text(true) {
                        Ok(text) => text,
                        Err(error) => {
                            if error.is_resource_suspension() {
                                self.retain_structured_scanner(
                                    PendingStructuredScannerPhase::PdfColorStackText { id, action },
                                    StructuredScannerChildDestination::PdfColorStackText,
                                )?;
                            }
                            return Err(error);
                        }
                    };
                    Ok(Some(Request::ColorStack {
                        id,
                        action: Some(match action {
                            PendingPdfColorStackAction::Set => Action::Set(text),
                            PendingPdfColorStackAction::Push => Action::Push(text),
                        }),
                    }))
                }
                _ => {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    Err(CommandError::input_invariant())
                }
            };
        }

        let request = match primitive {
            UnexpandablePrimitive::PdfLiteral => {
                return self.scan_pdf_graphics_scalar(
                    primitive,
                    PdfGraphicsScalarPhase::LiteralShipout,
                    None,
                );
            }
            UnexpandablePrimitive::PdfSetMatrix => Request::SetMatrix {
                text: self.scan_balanced_text(true)?,
            },
            UnexpandablePrimitive::PdfSave => Request::Save,
            UnexpandablePrimitive::PdfRestore => Request::Restore,
            UnexpandablePrimitive::PdfColorStack => {
                return self.scan_pdf_graphics_scalar(
                    primitive,
                    PdfGraphicsScalarPhase::ColorId,
                    None,
                );
            }
            UnexpandablePrimitive::PdfSavePos => Request::SavePosition,
            UnexpandablePrimitive::PdfSnapRefPoint => Request::SnapReferencePoint,
            UnexpandablePrimitive::PdfSnapY => {
                return self.scan_pdf_graphics_scalar(
                    primitive,
                    PdfGraphicsScalarPhase::SnapY,
                    None,
                );
            }
            UnexpandablePrimitive::PdfSnapYComp => {
                return self.scan_pdf_graphics_scalar(
                    primitive,
                    PdfGraphicsScalarPhase::SnapYComp,
                    None,
                );
            }
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    fn scan_pdf_graphics_scalar(
        &mut self,
        primitive: UnexpandablePrimitive,
        mut phase: PdfGraphicsScalarPhase,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        use PdfColorStackActionRequest as Action;
        use PdfGraphicsRequest as Request;
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = |phase| PendingStructuredScalarPhase::PdfGraphics { primitive, phase };
            match phase {
                PdfGraphicsScalarPhase::LiteralShipout => {
                    let result = self.scan_keyword_retained("shipout");
                    let deferred = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value;
                    phase = PdfGraphicsScalarPhase::LiteralDirect { deferred };
                }
                PdfGraphicsScalarPhase::LiteralDirect { deferred } => {
                    let result = self.scan_keyword_retained("direct");
                    if self
                        .retain_structured_scalar(result, retained(phase))?
                        .value
                    {
                        return self.finish_pdf_graphics_literal(
                            tex_state::node::PdfLiteralMode::Direct,
                            deferred,
                        );
                    }
                    phase = PdfGraphicsScalarPhase::LiteralPage { deferred };
                }
                PdfGraphicsScalarPhase::LiteralPage { deferred } => {
                    let result = self.scan_keyword_retained("page");
                    let page = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value;
                    let mode = if page {
                        tex_state::node::PdfLiteralMode::Page
                    } else {
                        tex_state::node::PdfLiteralMode::Origin
                    };
                    return self.finish_pdf_graphics_literal(mode, deferred);
                }
                PdfGraphicsScalarPhase::ColorId => {
                    let result = self.scan_integer_retained();
                    let id = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value;
                    phase = PdfGraphicsScalarPhase::ColorSet { id };
                }
                PdfGraphicsScalarPhase::ColorSet { id } => {
                    let result = self.scan_keyword_retained("set");
                    if self
                        .retain_structured_scalar(result, retained(phase))?
                        .value
                    {
                        return self
                            .finish_pdf_color_stack_text(id, PendingPdfColorStackAction::Set);
                    }
                    phase = PdfGraphicsScalarPhase::ColorPush { id };
                }
                PdfGraphicsScalarPhase::ColorPush { id } => {
                    let result = self.scan_keyword_retained("push");
                    if self
                        .retain_structured_scalar(result, retained(phase))?
                        .value
                    {
                        return self
                            .finish_pdf_color_stack_text(id, PendingPdfColorStackAction::Push);
                    }
                    phase = PdfGraphicsScalarPhase::ColorPop { id };
                }
                PdfGraphicsScalarPhase::ColorPop { id } => {
                    let result = self.scan_keyword_retained("pop");
                    if self
                        .retain_structured_scalar(result, retained(phase))?
                        .value
                    {
                        return Ok(Some(Request::ColorStack {
                            id,
                            action: Some(Action::Pop),
                        }));
                    }
                    phase = PdfGraphicsScalarPhase::ColorCurrent { id };
                }
                PdfGraphicsScalarPhase::ColorCurrent { id } => {
                    let result = self.scan_keyword_retained("current");
                    let current = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value;
                    return Ok(Some(Request::ColorStack {
                        id,
                        action: current.then_some(Action::Current),
                    }));
                }
                PdfGraphicsScalarPhase::SnapY => {
                    let result = self.scan_glue_retained(false);
                    let glue = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value;
                    return Ok(Some(Request::SnapY { glue }));
                }
                PdfGraphicsScalarPhase::SnapYComp => {
                    let result = self.scan_integer_retained();
                    let ratio = self
                        .retain_structured_scalar(result, retained(phase))?
                        .value
                        .clamp(0, 1000) as u16;
                    return Ok(Some(Request::SnapYComp { ratio }));
                }
            }
        }
    }

    fn finish_pdf_graphics_literal(
        &mut self,
        mode: tex_state::node::PdfLiteralMode,
        deferred: bool,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        match self.scan_balanced_text(!deferred) {
            Ok(text) => Ok(Some(PdfGraphicsRequest::Literal {
                mode,
                deferred,
                text,
            })),
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::PdfGraphicsLiteral { mode, deferred },
                        StructuredScannerChildDestination::PdfGraphicsLiteral,
                    )?;
                }
                Err(error)
            }
        }
    }

    fn finish_pdf_color_stack_text(
        &mut self,
        id: i32,
        action: PendingPdfColorStackAction,
    ) -> Result<Option<PdfGraphicsRequest>, CommandError> {
        match self.scan_balanced_text(true) {
            Ok(text) => Ok(Some(PdfGraphicsRequest::ColorStack {
                id,
                action: Some(match action {
                    PendingPdfColorStackAction::Set => PdfColorStackActionRequest::Set(text),
                    PendingPdfColorStackAction::Push => PdfColorStackActionRequest::Push(text),
                }),
            })),
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::PdfColorStackText { id, action },
                        StructuredScannerChildDestination::PdfColorStackText,
                    )?;
                }
                Err(error)
            }
        }
    }

    fn scan_pdf_navigation_text(
        &mut self,
        child: &mut Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
        phase: PendingStructuredScannerPhase<G>,
        destination: StructuredScannerChildDestination,
    ) -> Result<(ScannedBalancedText, PendingStructuredScannerPhase<G>), CommandError> {
        self.restore_structured_scanner_child(child, destination)?;
        match self.scan_balanced_text(true) {
            Ok(text) => Ok((text, phase)),
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(phase, destination)?;
                }
                Err(error)
            }
        }
    }

    fn finish_pdf_outline(
        &mut self,
        attributes: Option<ScannedBalancedText>,
        action: PdfActionSpec,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
        retained_count: Option<i32>,
        scalar_phase: Option<PdfOutlineScalarPhase>,
    ) -> Result<PdfNavigationRequest, CommandError> {
        let count = if let Some(count) = retained_count {
            count
        } else {
            let phase = scalar_phase.unwrap_or(PdfOutlineScalarPhase::CountKeyword);
            let count_keyword = if phase == PdfOutlineScalarPhase::CountValue {
                true
            } else {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("count");
                self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::PdfOutlineCount {
                        attributes: attributes.clone(),
                        action,
                        phase: PdfOutlineScalarPhase::CountKeyword,
                    },
                )?
                .value
            };
            if count_keyword {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_integer_retained();
                self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::PdfOutlineCount {
                        attributes: attributes.clone(),
                        action,
                        phase: PdfOutlineScalarPhase::CountValue,
                    },
                )?
                .value
            } else {
                0
            }
        };
        let (title, phase) = self.scan_pdf_navigation_text(
            &mut child,
            PendingStructuredScannerPhase::PdfOutlineTitle {
                attributes,
                action,
                count,
            },
            StructuredScannerChildDestination::PdfNavigationTitle,
        )?;
        let PendingStructuredScannerPhase::PdfOutlineTitle {
            attributes,
            action,
            count,
        } = phase
        else {
            return Err(CommandError::input_invariant());
        };
        Ok(PdfNavigationRequest::Outline(PdfOutlineRequest {
            attributes,
            action,
            count,
            title,
        }))
    }

    fn scan_pdf_thread_identifier_owned(
        &mut self,
        primitive: UnexpandablePrimitive,
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: Option<ScannedBalancedText>,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
        resume_name: bool,
        scalar_phase: Option<PdfThreadScalarPhase>,
    ) -> Result<PdfNavigationRequest, CommandError> {
        let mut phase = scalar_phase.unwrap_or(PdfThreadScalarPhase::NameKeyword);
        let name = if resume_name {
            true
        } else if phase == PdfThreadScalarPhase::NameKeyword {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_keyword_retained("name");
            let name = self
                .retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::PdfThreadIdentifier {
                        primitive,
                        dimensions,
                        attributes: attributes.clone(),
                        phase,
                    },
                )?
                .value;
            if !name {
                phase = PdfThreadScalarPhase::NumKeyword;
            }
            name
        } else {
            false
        };
        if name {
            let (text, phase) = self.scan_pdf_navigation_text(
                &mut child,
                PendingStructuredScannerPhase::PdfThreadIdentifier {
                    primitive,
                    dimensions,
                    attributes,
                },
                StructuredScannerChildDestination::PdfNavigationIdentifier,
            )?;
            let PendingStructuredScannerPhase::PdfThreadIdentifier {
                primitive,
                dimensions,
                attributes,
            } = phase
            else {
                return Err(CommandError::input_invariant());
            };
            return Ok(PdfNavigationRequest::Thread(PdfThreadRequest {
                dimensions,
                attributes,
                identifier: PdfActionIdentifier::Name(text.tokens),
                running: primitive == UnexpandablePrimitive::PdfStartThread,
            }));
        }
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let num = if phase == PdfThreadScalarPhase::NumValue {
            true
        } else {
            let result = self.scan_keyword_retained("num");
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::PdfThreadIdentifier {
                    primitive,
                    dimensions,
                    attributes: attributes.clone(),
                    phase: PdfThreadScalarPhase::NumKeyword,
                },
            )?
            .value
        };
        let identifier = if num {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_integer_retained();
            let value = self
                .retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::PdfThreadIdentifier {
                        primitive,
                        dimensions,
                        attributes: attributes.clone(),
                        phase: PdfThreadScalarPhase::NumValue,
                    },
                )?
                .value;
            PdfActionIdentifier::Number(Self::finish_pdf_positive(
                value,
                "thread identifier",
                true,
            )?)
        } else {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext4): thread identifier type missing",
            ));
        };
        Ok(PdfNavigationRequest::Thread(PdfThreadRequest {
            dimensions,
            attributes,
            identifier,
            running: primitive == UnexpandablePrimitive::PdfStartThread,
        }))
    }

    /// Scans the pdfTeX annotation/link/destination/thread family (pdftex.web
    /// 34847--35208).  `scan_alt_rule` deliberately resets all dimensions on
    /// each invocation and accepts repeated fields, with the last one winning.
    pub fn scan_pdf_navigation_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfNavigationRequest, CommandError> {
        use PdfNavigationRequest as Request;

        let pending = self.take_pending_structured_scanner()?;
        if let Some(pending) = pending {
            let PendingStructuredScanner { phase, mut child } = pending;
            return match phase {
                PendingStructuredScannerPhase::PdfAnnotationEntries {
                    use_object,
                    dimensions,
                } => {
                    let (entries, phase) = self.scan_pdf_navigation_text(
                        &mut child,
                        PendingStructuredScannerPhase::PdfAnnotationEntries {
                            use_object,
                            dimensions,
                        },
                        StructuredScannerChildDestination::PdfNavigationAnnotationEntries,
                    )?;
                    let PendingStructuredScannerPhase::PdfAnnotationEntries {
                        use_object,
                        dimensions,
                    } = phase
                    else {
                        return Err(CommandError::input_invariant());
                    };
                    Ok(Request::Annotation(PdfAnnotationRequest::Define {
                        use_object,
                        dimensions,
                        entries,
                    }))
                }
                PendingStructuredScannerPhase::PdfStartLinkAttributes { dimensions } => {
                    let (attributes, phase) = self.scan_pdf_navigation_text(
                        &mut child,
                        PendingStructuredScannerPhase::PdfStartLinkAttributes { dimensions },
                        StructuredScannerChildDestination::PdfNavigationAttributes,
                    )?;
                    let PendingStructuredScannerPhase::PdfStartLinkAttributes { dimensions } =
                        phase
                    else {
                        return Err(CommandError::input_invariant());
                    };
                    let (owner, action) = self.scan_pdf_action_for_owner(
                        PendingPdfActionOwner::StartLink {
                            dimensions,
                            attributes: Some(attributes),
                        },
                        None,
                    )?;
                    let PendingPdfActionOwner::StartLink {
                        dimensions,
                        attributes,
                    } = owner
                    else {
                        return Err(CommandError::input_invariant());
                    };
                    Ok(Request::StartLink(PdfStartLinkRequest {
                        dimensions,
                        attributes,
                        action,
                    }))
                }
                PendingStructuredScannerPhase::PdfOutlineAttributes => {
                    let (attributes, phase) = self.scan_pdf_navigation_text(
                        &mut child,
                        PendingStructuredScannerPhase::PdfOutlineAttributes,
                        StructuredScannerChildDestination::PdfNavigationAttributes,
                    )?;
                    if !matches!(phase, PendingStructuredScannerPhase::PdfOutlineAttributes) {
                        return Err(CommandError::input_invariant());
                    }
                    let (owner, action) = self.scan_pdf_action_for_owner(
                        PendingPdfActionOwner::Outline {
                            attributes: Some(attributes),
                        },
                        None,
                    )?;
                    let PendingPdfActionOwner::Outline { attributes } = owner else {
                        return Err(CommandError::input_invariant());
                    };
                    self.finish_pdf_outline(attributes, action, None, None, None)
                }
                PendingStructuredScannerPhase::PdfOutlineTitle {
                    attributes,
                    action,
                    count,
                } => self.finish_pdf_outline(attributes, action, child, Some(count), None),
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::PdfOutlineCount {
                        attributes,
                        action,
                        phase,
                    },
                ) => self.finish_pdf_outline(attributes, action, child, None, Some(phase)),
                PendingStructuredScannerPhase::PdfThreadAttributes {
                    primitive,
                    dimensions,
                } => {
                    let (attributes, phase) = self.scan_pdf_navigation_text(
                        &mut child,
                        PendingStructuredScannerPhase::PdfThreadAttributes {
                            primitive,
                            dimensions,
                        },
                        StructuredScannerChildDestination::PdfNavigationAttributes,
                    )?;
                    let PendingStructuredScannerPhase::PdfThreadAttributes {
                        primitive,
                        dimensions,
                    } = phase
                    else {
                        return Err(CommandError::input_invariant());
                    };
                    self.scan_pdf_thread_identifier_owned(
                        primitive,
                        dimensions,
                        Some(attributes),
                        None,
                        false,
                        None,
                    )
                }
                PendingStructuredScannerPhase::PdfThreadIdentifier {
                    primitive,
                    dimensions,
                    attributes,
                } => self.scan_pdf_thread_identifier_owned(
                    primitive, dimensions, attributes, child, true, None,
                ),
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::PdfThreadIdentifier {
                        primitive,
                        dimensions,
                        attributes,
                        phase,
                    },
                ) => self.scan_pdf_thread_identifier_owned(
                    primitive,
                    dimensions,
                    attributes,
                    child,
                    false,
                    Some(phase),
                ),
                PendingStructuredScannerPhase::PdfDestinationIdentifier { structure } => {
                    let (identifier, phase) = self.scan_pdf_navigation_text(
                        &mut child,
                        PendingStructuredScannerPhase::PdfDestinationIdentifier { structure },
                        StructuredScannerChildDestination::PdfNavigationIdentifier,
                    )?;
                    let PendingStructuredScannerPhase::PdfDestinationIdentifier { structure } =
                        phase
                    else {
                        return Err(CommandError::input_invariant());
                    };
                    self.scan_pdf_navigation_scalar(
                        PdfNavigationScalarProgress {
                            primitive: UnexpandablePrimitive::PdfDest,
                            use_object: None,
                            dimensions: tex_state::PdfAnnotationDimensions::RUNNING,
                            attributes: None,
                            structure,
                            identifier: Some(PdfActionIdentifier::Name(identifier.tokens)),
                            phase: PdfNavigationScalarPhase::DestinationXyz,
                        },
                        None,
                    )
                }
                PendingStructuredScannerPhase::PdfAction { owner, phase } => {
                    let (owner, action) =
                        self.scan_pdf_action_for_owner(owner, Some((phase, child)))?;
                    match owner {
                        PendingPdfActionOwner::StartLink {
                            dimensions,
                            attributes,
                        } => Ok(Request::StartLink(PdfStartLinkRequest {
                            dimensions,
                            attributes,
                            action,
                        })),
                        PendingPdfActionOwner::Outline { attributes } => {
                            self.finish_pdf_outline(attributes, action, None, None, None)
                        }
                        PendingPdfActionOwner::DocumentFragment { .. } => {
                            Err(CommandError::input_invariant())
                        }
                    }
                }
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::PdfAction { owner, progress },
                ) => {
                    let (owner, action) = self.scan_pdf_action_scalar(owner, progress, child)?;
                    match owner {
                        PendingPdfActionOwner::StartLink {
                            dimensions,
                            attributes,
                        } => Ok(Request::StartLink(PdfStartLinkRequest {
                            dimensions,
                            attributes,
                            action,
                        })),
                        PendingPdfActionOwner::Outline { attributes } => {
                            self.finish_pdf_outline(attributes, action, None, None, None)
                        }
                        PendingPdfActionOwner::DocumentFragment { .. } => {
                            Err(CommandError::input_invariant())
                        }
                    }
                }
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::PdfNavigation(progress),
                ) => self.scan_pdf_navigation_scalar(progress, child),
                _ => {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    Err(CommandError::input_invariant())
                }
            };
        }

        let phase = match primitive {
            UnexpandablePrimitive::PdfAnnot => PdfNavigationScalarPhase::AnnotationReserve,
            UnexpandablePrimitive::PdfStartLink
            | UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread => PdfNavigationScalarPhase::WidthKeyword,
            UnexpandablePrimitive::PdfOutline => PdfNavigationScalarPhase::AttributeKeyword,
            UnexpandablePrimitive::PdfDest => PdfNavigationScalarPhase::DestinationStructure,
            UnexpandablePrimitive::PdfEndLink => return Ok(Request::EndLink),
            UnexpandablePrimitive::PdfEndThread => return Ok(Request::EndThread),
            _ => return Err(CommandError::input_invariant()),
        };
        self.scan_pdf_navigation_scalar(
            PdfNavigationScalarProgress {
                primitive,
                use_object: None,
                dimensions: tex_state::PdfAnnotationDimensions::RUNNING,
                attributes: None,
                structure: None,
                identifier: None,
                phase,
            },
            None,
        )
    }

    fn scan_pdf_navigation_scalar(
        &mut self,
        mut progress: PdfNavigationScalarProgress,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
    ) -> Result<PdfNavigationRequest, CommandError> {
        use tex_state::node::PdfDestinationKind as Kind;
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = PendingStructuredScalarPhase::PdfNavigation(progress.clone());
            match progress.phase {
                PdfNavigationScalarPhase::AnnotationReserve => {
                    let result = self.scan_keyword_retained("reserveobjnum");
                    if self.retain_structured_scalar(result, retained)?.value {
                        return Ok(PdfNavigationRequest::Annotation(
                            PdfAnnotationRequest::Reserve,
                        ));
                    }
                    progress.phase = PdfNavigationScalarPhase::AnnotationUse;
                }
                PdfNavigationScalarPhase::AnnotationUse => {
                    let result = self.scan_keyword_retained("useobjnum");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfNavigationScalarPhase::AnnotationUseObject
                    } else {
                        PdfNavigationScalarPhase::WidthKeyword
                    };
                }
                PdfNavigationScalarPhase::AnnotationUseObject => {
                    let result = self.scan_integer_retained();
                    progress.use_object =
                        Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase = PdfNavigationScalarPhase::WidthKeyword;
                }
                PdfNavigationScalarPhase::WidthKeyword
                | PdfNavigationScalarPhase::FitRWidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRWidthKeyword;
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        if fitr {
                            PdfNavigationScalarPhase::FitRWidthDimension
                        } else {
                            PdfNavigationScalarPhase::WidthDimension
                        }
                    } else if fitr {
                        PdfNavigationScalarPhase::FitRHeightKeyword
                    } else {
                        PdfNavigationScalarPhase::HeightKeyword
                    };
                }
                PdfNavigationScalarPhase::WidthDimension
                | PdfNavigationScalarPhase::FitRWidthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.width =
                        Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRWidthDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::HeightKeyword
                | PdfNavigationScalarPhase::FitRHeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRHeightKeyword;
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        if fitr {
                            PdfNavigationScalarPhase::FitRHeightDimension
                        } else {
                            PdfNavigationScalarPhase::HeightDimension
                        }
                    } else if fitr {
                        PdfNavigationScalarPhase::FitRDepthKeyword
                    } else {
                        PdfNavigationScalarPhase::DepthKeyword
                    };
                }
                PdfNavigationScalarPhase::HeightDimension
                | PdfNavigationScalarPhase::FitRHeightDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.height =
                        Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRHeightDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::DepthKeyword
                | PdfNavigationScalarPhase::FitRDepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    let fitr = progress.phase == PdfNavigationScalarPhase::FitRDepthKeyword;
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.phase = if fitr {
                            PdfNavigationScalarPhase::FitRDepthDimension
                        } else {
                            PdfNavigationScalarPhase::DepthDimension
                        };
                    } else if fitr {
                        let dimensions = progress.dimensions;
                        return self
                            .finish_pdf_destination(progress, Kind::FitRectangle(dimensions));
                    } else {
                        match progress.primitive {
                            UnexpandablePrimitive::PdfAnnot => {
                                let (entries, _) = self.scan_pdf_navigation_text(
                                    &mut None,
                                    PendingStructuredScannerPhase::PdfAnnotationEntries {
                                        use_object: progress.use_object,
                                        dimensions: progress.dimensions,
                                    },
                                    StructuredScannerChildDestination::PdfNavigationAnnotationEntries,
                                )?;
                                return Ok(PdfNavigationRequest::Annotation(
                                    PdfAnnotationRequest::Define {
                                        use_object: progress.use_object,
                                        dimensions: progress.dimensions,
                                        entries,
                                    },
                                ));
                            }
                            UnexpandablePrimitive::PdfStartLink
                            | UnexpandablePrimitive::PdfThread
                            | UnexpandablePrimitive::PdfStartThread => {
                                progress.phase = PdfNavigationScalarPhase::AttributeKeyword;
                            }
                            _ => return Err(CommandError::input_invariant()),
                        }
                    }
                }
                PdfNavigationScalarPhase::DepthDimension
                | PdfNavigationScalarPhase::FitRDepthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.dimensions.depth =
                        Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase =
                        if progress.phase == PdfNavigationScalarPhase::FitRDepthDimension {
                            PdfNavigationScalarPhase::FitRWidthKeyword
                        } else {
                            PdfNavigationScalarPhase::WidthKeyword
                        };
                }
                PdfNavigationScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    let has_attr = self.retain_structured_scalar(result, retained)?.value;
                    match progress.primitive {
                        UnexpandablePrimitive::PdfStartLink => {
                            if has_attr {
                                progress.attributes = Some(
                                    self.scan_pdf_navigation_text(
                                        &mut None,
                                        PendingStructuredScannerPhase::PdfStartLinkAttributes {
                                            dimensions: progress.dimensions,
                                        },
                                        StructuredScannerChildDestination::PdfNavigationAttributes,
                                    )?
                                    .0,
                                );
                            }
                            let (owner, action) = self.scan_pdf_action_for_owner(
                                PendingPdfActionOwner::StartLink {
                                    dimensions: progress.dimensions,
                                    attributes: progress.attributes,
                                },
                                None,
                            )?;
                            let PendingPdfActionOwner::StartLink {
                                dimensions,
                                attributes,
                            } = owner
                            else {
                                return Err(CommandError::input_invariant());
                            };
                            return Ok(PdfNavigationRequest::StartLink(PdfStartLinkRequest {
                                dimensions,
                                attributes,
                                action,
                            }));
                        }
                        UnexpandablePrimitive::PdfOutline => {
                            if has_attr {
                                progress.attributes = Some(
                                    self.scan_pdf_navigation_text(
                                        &mut None,
                                        PendingStructuredScannerPhase::PdfOutlineAttributes,
                                        StructuredScannerChildDestination::PdfNavigationAttributes,
                                    )?
                                    .0,
                                );
                            }
                            let (owner, action) = self.scan_pdf_action_for_owner(
                                PendingPdfActionOwner::Outline {
                                    attributes: progress.attributes,
                                },
                                None,
                            )?;
                            let PendingPdfActionOwner::Outline { attributes } = owner else {
                                return Err(CommandError::input_invariant());
                            };
                            return self.finish_pdf_outline(attributes, action, None, None, None);
                        }
                        primitive @ (UnexpandablePrimitive::PdfThread
                        | UnexpandablePrimitive::PdfStartThread) => {
                            if has_attr {
                                progress.attributes = Some(
                                    self.scan_pdf_navigation_text(
                                        &mut None,
                                        PendingStructuredScannerPhase::PdfThreadAttributes {
                                            primitive,
                                            dimensions: progress.dimensions,
                                        },
                                        StructuredScannerChildDestination::PdfNavigationAttributes,
                                    )?
                                    .0,
                                );
                            }
                            return self.scan_pdf_thread_identifier_owned(
                                primitive,
                                progress.dimensions,
                                progress.attributes,
                                None,
                                false,
                                None,
                            );
                        }
                        _ => return Err(CommandError::input_invariant()),
                    }
                }
                PdfNavigationScalarPhase::DestinationStructure => {
                    let result = self.scan_keyword_retained("struct");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfNavigationScalarPhase::DestinationStructureValue
                    } else {
                        PdfNavigationScalarPhase::DestinationName
                    };
                }
                PdfNavigationScalarPhase::DestinationStructureValue => {
                    let result = self.scan_integer_retained();
                    progress.structure = Some(Self::finish_pdf_positive(
                        self.retain_structured_scalar(result, retained)?.value,
                        "struct identifier",
                        false,
                    )?);
                    progress.phase = PdfNavigationScalarPhase::DestinationName;
                }
                PdfNavigationScalarPhase::DestinationName => {
                    let result = self.scan_keyword_retained("name");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let (identifier, _) = self.scan_pdf_navigation_text(
                            &mut None,
                            PendingStructuredScannerPhase::PdfDestinationIdentifier {
                                structure: progress.structure,
                            },
                            StructuredScannerChildDestination::PdfNavigationIdentifier,
                        )?;
                        progress.identifier = Some(PdfActionIdentifier::Name(identifier.tokens));
                        progress.phase = PdfNavigationScalarPhase::DestinationXyz;
                    } else {
                        progress.phase = PdfNavigationScalarPhase::DestinationNumber;
                    }
                }
                PdfNavigationScalarPhase::DestinationNumber => {
                    let result = self.scan_keyword_retained("num");
                    if !self.retain_structured_scalar(result, retained)?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    progress.phase = PdfNavigationScalarPhase::DestinationNumberValue;
                }
                PdfNavigationScalarPhase::DestinationNumberValue => {
                    let result = self.scan_integer_retained();
                    progress.identifier =
                        Some(PdfActionIdentifier::Number(Self::finish_pdf_positive(
                            self.retain_structured_scalar(result, retained)?.value,
                            "destination identifier",
                            true,
                        )?));
                    progress.phase = PdfNavigationScalarPhase::DestinationXyz;
                }
                PdfNavigationScalarPhase::DestinationXyz => {
                    let result = self.scan_keyword_retained("xyz");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.phase = PdfNavigationScalarPhase::DestinationZoom;
                    } else {
                        progress.phase = PdfNavigationScalarPhase::DestinationFitBh;
                    }
                }
                PdfNavigationScalarPhase::DestinationZoom => {
                    let result = self.scan_keyword_retained("zoom");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.phase = PdfNavigationScalarPhase::DestinationZoomValue;
                    } else {
                        return self.finish_pdf_destination(progress, Kind::Xyz { zoom: None });
                    }
                }
                PdfNavigationScalarPhase::DestinationZoomValue => {
                    let result = self.scan_integer_retained();
                    let zoom = self.retain_structured_scalar(result, retained)?.value;
                    if zoom > 1_073_741_823 {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): number too big",
                        ));
                    }
                    return self.finish_pdf_destination(progress, Kind::Xyz { zoom: Some(zoom) });
                }
                PdfNavigationScalarPhase::DestinationFitBh
                | PdfNavigationScalarPhase::DestinationFitBv
                | PdfNavigationScalarPhase::DestinationFitB
                | PdfNavigationScalarPhase::DestinationFitH
                | PdfNavigationScalarPhase::DestinationFitV
                | PdfNavigationScalarPhase::DestinationFitR
                | PdfNavigationScalarPhase::DestinationFit => {
                    let (keyword, kind, next) = match progress.phase {
                        PdfNavigationScalarPhase::DestinationFitBh => (
                            "fitbh",
                            Some(Kind::FitBoundingBoxHorizontal),
                            PdfNavigationScalarPhase::DestinationFitBv,
                        ),
                        PdfNavigationScalarPhase::DestinationFitBv => (
                            "fitbv",
                            Some(Kind::FitBoundingBoxVertical),
                            PdfNavigationScalarPhase::DestinationFitB,
                        ),
                        PdfNavigationScalarPhase::DestinationFitB => (
                            "fitb",
                            Some(Kind::FitBoundingBox),
                            PdfNavigationScalarPhase::DestinationFitH,
                        ),
                        PdfNavigationScalarPhase::DestinationFitH => (
                            "fith",
                            Some(Kind::FitHorizontal),
                            PdfNavigationScalarPhase::DestinationFitV,
                        ),
                        PdfNavigationScalarPhase::DestinationFitV => (
                            "fitv",
                            Some(Kind::FitVertical),
                            PdfNavigationScalarPhase::DestinationFitR,
                        ),
                        PdfNavigationScalarPhase::DestinationFitR => {
                            ("fitr", None, PdfNavigationScalarPhase::DestinationFit)
                        }
                        PdfNavigationScalarPhase::DestinationFit => (
                            "fit",
                            Some(Kind::Fit),
                            PdfNavigationScalarPhase::DestinationFit,
                        ),
                        _ => unreachable!(),
                    };
                    let result = self.scan_keyword_retained(keyword);
                    if self.retain_structured_scalar(result, retained)?.value {
                        if progress.phase == PdfNavigationScalarPhase::DestinationFitR {
                            progress.dimensions = tex_state::PdfAnnotationDimensions::RUNNING;
                            progress.phase = PdfNavigationScalarPhase::FitRWidthKeyword;
                        } else {
                            return self.finish_pdf_destination(
                                progress,
                                kind.expect("non-fitr destination has a kind"),
                            );
                        }
                    } else if progress.phase == PdfNavigationScalarPhase::DestinationFit {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): destination type missing",
                        ));
                    } else {
                        progress.phase = next;
                    }
                }
            }
        }
    }

    fn finish_pdf_destination(
        &self,
        progress: PdfNavigationScalarProgress,
        kind: tex_state::node::PdfDestinationKind,
    ) -> Result<PdfNavigationRequest, CommandError> {
        Ok(PdfNavigationRequest::Destination(PdfDestinationRequest {
            structure: progress.structure,
            identifier: progress.identifier.ok_or(CommandError::input_invariant())?,
            kind,
        }))
    }

    fn finish_pdf_positive(
        value: i32,
        kind: &'static str,
        bounded_by_halfword: bool,
    ) -> Result<u32, CommandError> {
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

    fn scan_pdf_action_owned_text(
        &mut self,
        owner: &mut Option<PendingPdfActionOwner>,
        child: &mut Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
        phase: PendingPdfActionPhase,
        destination: StructuredScannerChildDestination,
    ) -> Result<ScannedBalancedText, CommandError> {
        self.restore_structured_scanner_child(child, destination)?;
        match self.scan_balanced_text(true) {
            Ok(text) => Ok(text),
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::PdfAction {
                            owner: owner
                                .take()
                                .expect("suspended PDF action retains its outer owner"),
                            phase,
                        },
                        destination,
                    )?;
                }
                Err(error)
            }
        }
    }

    fn scan_pdf_action_for_owner(
        &mut self,
        owner: PendingPdfActionOwner,
        pending: Option<(
            PendingPdfActionPhase,
            Option<
                crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
            >,
        )>,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        let mut owner = Some(owner);
        let (phase, mut child) =
            pending.map_or((None, None), |(phase, child)| (Some(phase), child));
        let progress = match phase {
            Some(PendingPdfActionPhase::User) => {
                let text = self.scan_pdf_action_owned_text(
                    &mut owner,
                    &mut child,
                    PendingPdfActionPhase::User,
                    StructuredScannerChildDestination::PdfActionUser,
                )?;
                return Ok((
                    owner.expect("successful PDF action retains its owner"),
                    PdfActionSpec::User(text.tokens),
                ));
            }
            Some(PendingPdfActionPhase::File { goto }) => {
                let file = self
                    .scan_pdf_action_owned_text(
                        &mut owner,
                        &mut child,
                        PendingPdfActionPhase::File { goto },
                        StructuredScannerChildDestination::PdfActionFile,
                    )?
                    .tokens;
                PdfActionScalarProgress {
                    goto: Some(goto),
                    file: Some(file),
                    structure: None,
                    target: None,
                    phase: PdfActionScalarPhase::StructureKeyword,
                }
            }
            Some(PendingPdfActionPhase::StructureRaw { goto, file }) => {
                let structure = self
                    .scan_pdf_action_owned_text(
                        &mut owner,
                        &mut child,
                        PendingPdfActionPhase::StructureRaw { goto, file },
                        StructuredScannerChildDestination::PdfActionStructure,
                    )?
                    .tokens;
                PdfActionScalarProgress {
                    goto: Some(goto),
                    file: Some(file),
                    structure: Some(PdfActionIdentifier::Raw(structure)),
                    target: None,
                    phase: PdfActionScalarPhase::PageKeyword,
                }
            }
            Some(PendingPdfActionPhase::StructureName { goto, file }) => {
                let structure = self
                    .scan_pdf_action_owned_text(
                        &mut owner,
                        &mut child,
                        PendingPdfActionPhase::StructureName { goto, file },
                        StructuredScannerChildDestination::PdfActionStructure,
                    )?
                    .tokens;
                PdfActionScalarProgress {
                    goto: Some(goto),
                    file,
                    structure: Some(PdfActionIdentifier::Name(structure)),
                    target: None,
                    phase: PdfActionScalarPhase::PageKeyword,
                }
            }
            Some(PendingPdfActionPhase::PageView {
                goto,
                file,
                structure,
                number,
            }) => {
                let view = self
                    .scan_pdf_action_owned_text(
                        &mut owner,
                        &mut child,
                        PendingPdfActionPhase::PageView {
                            goto,
                            file,
                            structure,
                            number,
                        },
                        StructuredScannerChildDestination::PdfActionPageView,
                    )?
                    .tokens;
                PdfActionScalarProgress {
                    goto: Some(goto),
                    file,
                    structure,
                    target: Some(PdfActionTarget::Page { number, view }),
                    phase: PdfActionScalarPhase::NewWindowKeyword,
                }
            }
            Some(PendingPdfActionPhase::TargetName {
                goto,
                file,
                structure,
            }) => {
                let name = self
                    .scan_pdf_action_owned_text(
                        &mut owner,
                        &mut child,
                        PendingPdfActionPhase::TargetName {
                            goto,
                            file,
                            structure,
                        },
                        StructuredScannerChildDestination::PdfActionTargetName,
                    )?
                    .tokens;
                PdfActionScalarProgress {
                    goto: Some(goto),
                    file,
                    structure,
                    target: Some(PdfActionTarget::Destination(PdfActionIdentifier::Name(
                        name,
                    ))),
                    phase: PdfActionScalarPhase::NewWindowKeyword,
                }
            }
            None => PdfActionScalarProgress {
                goto: None,
                file: None,
                structure: None,
                target: None,
                phase: PdfActionScalarPhase::UserKeyword,
            },
        };
        self.scan_pdf_action_scalar(
            owner.expect("balanced PDF action retains its owner"),
            progress,
            child,
        )
    }

    fn scan_pdf_action_scalar(
        &mut self,
        owner: PendingPdfActionOwner,
        mut progress: PdfActionScalarProgress,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        use tex_state::PdfActionWindow;
        let mut owner = Some(owner);
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = PendingStructuredScalarPhase::PdfAction {
                owner: owner
                    .as_ref()
                    .expect("active PDF action retains its owner")
                    .clone(),
                progress: progress.clone(),
            };
            match progress.phase {
                PdfActionScalarPhase::UserKeyword => {
                    let result = self.scan_keyword_retained("user");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let text = self.scan_pdf_action_owned_text(
                            &mut owner,
                            &mut child,
                            PendingPdfActionPhase::User,
                            StructuredScannerChildDestination::PdfActionUser,
                        )?;
                        return Ok((
                            owner.expect("successful PDF action retains its owner"),
                            PdfActionSpec::User(text.tokens),
                        ));
                    }
                    progress.phase = PdfActionScalarPhase::GotoKeyword;
                }
                PdfActionScalarPhase::GotoKeyword => {
                    let result = self.scan_keyword_retained("goto");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.goto = Some(true);
                        progress.phase = PdfActionScalarPhase::FileKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::ThreadKeyword;
                    }
                }
                PdfActionScalarPhase::ThreadKeyword => {
                    let result = self.scan_keyword_retained("thread");
                    if !self.retain_structured_scalar(result, retained)?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): action type missing",
                        ));
                    }
                    progress.goto = Some(false);
                    progress.phase = PdfActionScalarPhase::FileKeyword;
                }
                PdfActionScalarPhase::FileKeyword => {
                    let result = self.scan_keyword_retained("file");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                        progress.file = Some(
                            self.scan_pdf_action_owned_text(
                                &mut owner,
                                &mut child,
                                PendingPdfActionPhase::File { goto },
                                StructuredScannerChildDestination::PdfActionFile,
                            )?
                            .tokens,
                        );
                    }
                    progress.phase = PdfActionScalarPhase::StructureKeyword;
                }
                PdfActionScalarPhase::StructureKeyword => {
                    let result = self.scan_keyword_retained("struct");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                        if !goto {
                            return Err(CommandError::PdfNavigation(
                                "pdfTeX error (ext1): only GoTo action can be used with `struct'",
                            ));
                        }
                        if let Some(file) = progress.file {
                            progress.structure = Some(PdfActionIdentifier::Raw(
                                self.scan_pdf_action_owned_text(
                                    &mut owner,
                                    &mut child,
                                    PendingPdfActionPhase::StructureRaw { goto, file },
                                    StructuredScannerChildDestination::PdfActionStructure,
                                )?
                                .tokens,
                            ));
                            progress.phase = PdfActionScalarPhase::PageKeyword;
                        } else {
                            progress.phase = PdfActionScalarPhase::StructureNameKeyword;
                        }
                    } else {
                        progress.phase = PdfActionScalarPhase::PageKeyword;
                    }
                }
                PdfActionScalarPhase::StructureNameKeyword => {
                    let result = self.scan_keyword_retained("name");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                        progress.structure = Some(PdfActionIdentifier::Name(
                            self.scan_pdf_action_owned_text(
                                &mut owner,
                                &mut child,
                                PendingPdfActionPhase::StructureName {
                                    goto,
                                    file: progress.file,
                                },
                                StructuredScannerChildDestination::PdfActionStructure,
                            )?
                            .tokens,
                        ));
                        progress.phase = PdfActionScalarPhase::PageKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::StructureNumberKeyword;
                    }
                }
                PdfActionScalarPhase::StructureNumberKeyword => {
                    let result = self.scan_keyword_retained("num");
                    if !self.retain_structured_scalar(result, retained)?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    progress.phase = PdfActionScalarPhase::StructureNumber;
                }
                PdfActionScalarPhase::StructureNumber => {
                    let result = self.scan_integer_retained();
                    let value = self.retain_structured_scalar(result, retained)?.value;
                    progress.structure = Some(PdfActionIdentifier::Number(
                        Self::finish_pdf_positive(value, "struct identifier", false)?,
                    ));
                    progress.phase = PdfActionScalarPhase::PageKeyword;
                }
                PdfActionScalarPhase::PageKeyword => {
                    let result = self.scan_keyword_retained("page");
                    if self.retain_structured_scalar(result, retained)?.value {
                        if !progress.goto.ok_or(CommandError::input_invariant())? {
                            return Err(CommandError::PdfNavigation(
                                "pdfTeX error (ext1): only GoTo action can be used with `page'",
                            ));
                        }
                        progress.phase = PdfActionScalarPhase::PageNumber;
                    } else {
                        progress.phase = PdfActionScalarPhase::NameKeyword;
                    }
                }
                PdfActionScalarPhase::PageNumber => {
                    let result = self.scan_integer_retained();
                    let number = Self::finish_pdf_positive(
                        self.retain_structured_scalar(result, retained)?.value,
                        "page number",
                        false,
                    )?;
                    let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                    let view = self
                        .scan_pdf_action_owned_text(
                            &mut owner,
                            &mut child,
                            PendingPdfActionPhase::PageView {
                                goto,
                                file: progress.file,
                                structure: progress.structure,
                                number,
                            },
                            StructuredScannerChildDestination::PdfActionPageView,
                        )?
                        .tokens;
                    progress.target = Some(PdfActionTarget::Page { number, view });
                    progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                }
                PdfActionScalarPhase::NameKeyword => {
                    let result = self.scan_keyword_retained("name");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                        let name = self
                            .scan_pdf_action_owned_text(
                                &mut owner,
                                &mut child,
                                PendingPdfActionPhase::TargetName {
                                    goto,
                                    file: progress.file,
                                    structure: progress.structure,
                                },
                                StructuredScannerChildDestination::PdfActionTargetName,
                            )?
                            .tokens;
                        progress.target = Some(PdfActionTarget::Destination(
                            PdfActionIdentifier::Name(name),
                        ));
                        progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                    } else {
                        progress.phase = PdfActionScalarPhase::NumberKeyword;
                    }
                }
                PdfActionScalarPhase::NumberKeyword => {
                    let result = self.scan_keyword_retained("num");
                    if !self.retain_structured_scalar(result, retained)?.value {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): identifier type missing",
                        ));
                    }
                    let goto = progress.goto.ok_or(CommandError::input_invariant())?;
                    if goto && progress.file.is_some() {
                        return Err(CommandError::PdfNavigation(
                            "pdfTeX error (ext1): `goto' option cannot be used with both `file' and `num'",
                        ));
                    }
                    progress.phase = PdfActionScalarPhase::Number;
                }
                PdfActionScalarPhase::Number => {
                    let result = self.scan_integer_retained();
                    let value = Self::finish_pdf_positive(
                        self.retain_structured_scalar(result, retained)?.value,
                        "num identifier",
                        false,
                    )?;
                    progress.target = Some(PdfActionTarget::Destination(
                        PdfActionIdentifier::Number(value),
                    ));
                    progress.phase = PdfActionScalarPhase::NewWindowKeyword;
                }
                PdfActionScalarPhase::NewWindowKeyword => {
                    let result = self.scan_keyword_retained("newwindow");
                    if self.retain_structured_scalar(result, retained)?.value {
                        return self.finish_pdf_action(owner, progress, PdfActionWindow::New);
                    }
                    progress.phase = PdfActionScalarPhase::NoNewWindowKeyword;
                }
                PdfActionScalarPhase::NoNewWindowKeyword => {
                    let result = self.scan_keyword_retained("nonewwindow");
                    let same = self.retain_structured_scalar(result, retained)?.value;
                    return self.finish_pdf_action(
                        owner,
                        progress,
                        if same {
                            PdfActionWindow::Same
                        } else {
                            PdfActionWindow::Unspecified
                        },
                    );
                }
            }
        }
    }

    fn finish_pdf_action(
        &self,
        owner: Option<PendingPdfActionOwner>,
        progress: PdfActionScalarProgress,
        window: tex_state::PdfActionWindow,
    ) -> Result<(PendingPdfActionOwner, PdfActionSpec), CommandError> {
        let goto = progress.goto.ok_or(CommandError::input_invariant())?;
        if window != tex_state::PdfActionWindow::Unspecified && (!goto || progress.file.is_none()) {
            return Err(CommandError::PdfNavigation(
                "pdfTeX error (ext1): `newwindow'/`nonewwindow' must be used with `goto' and `file' option",
            ));
        }
        let action = PdfActionDestination {
            file: progress.file,
            structure: progress.structure,
            target: progress.target.ok_or(CommandError::input_invariant())?,
            window,
        };
        Ok((
            owner.expect("successful PDF action retains its owner"),
            if goto {
                PdfActionSpec::GoTo(action)
            } else {
                PdfActionSpec::Thread(action)
            },
        ))
    }

    /// Scans pdfTeX's raw-object, form, and document-fragment extensions.
    ///
    /// This is the command boundary corresponding to pdftex.web's extension
    /// cases: `scan_keyword` and expanded `scan_pdf_ext_toks` are complete
    /// before the executor mutates its PDF ledger or mode list.
    pub fn scan_pdf_object_request(&mut self) -> Result<PdfObjectRequest, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (mut progress, mut child) = match pending {
            Some(pending) => {
                let PendingStructuredScanner { phase, mut child } = pending;
                match phase {
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::PdfObject(progress),
                    ) => (progress, child),
                    PendingStructuredScannerPhase::PdfObjectStreamAttribute { use_object } => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfObjectStreamAttribute,
                        )?;
                        let stream_attr = match self.scan_balanced_text(true) {
                            Ok(value) => Some(value),
                            Err(error) => {
                                if error.is_resource_suspension() {
                                    self.retain_structured_scanner(
                                        PendingStructuredScannerPhase::PdfObjectStreamAttribute {
                                            use_object,
                                        },
                                        StructuredScannerChildDestination::PdfObjectStreamAttribute,
                                    )?;
                                }
                                return Err(error);
                            }
                        };
                        (
                            PdfObjectScalarProgress {
                                use_object,
                                stream: true,
                                stream_attr,
                                phase: PdfObjectScalarPhase::FileKeyword,
                            },
                            None,
                        )
                    }
                    PendingStructuredScannerPhase::PdfObjectData {
                        use_object,
                        stream,
                        stream_attr,
                        file,
                    } => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfObjectData,
                        )?;
                        let data = self.scan_balanced_text(true)?;
                        return Ok(PdfObjectRequest::Define {
                            use_object,
                            stream,
                            stream_attr,
                            file,
                            data,
                        });
                    }
                    _ => {
                        if let Some(child) = child.take() {
                            self.abort_continuation(child.restore().0)?;
                        }
                        return Err(CommandError::input_invariant());
                    }
                }
            }
            None => (
                PdfObjectScalarProgress {
                    use_object: None,
                    stream: false,
                    stream_attr: None,
                    phase: PdfObjectScalarPhase::ReserveKeyword,
                },
                None,
            ),
        };
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = PendingStructuredScalarPhase::PdfObject(progress.clone());
            match progress.phase {
                PdfObjectScalarPhase::ReserveKeyword => {
                    let result = self.scan_keyword_retained("reserveobjnum");
                    if self.retain_structured_scalar(result, retained)?.value {
                        return Ok(PdfObjectRequest::Reserve);
                    }
                    progress.phase = PdfObjectScalarPhase::UseKeyword;
                }
                PdfObjectScalarPhase::UseKeyword => {
                    let result = self.scan_keyword_retained("useobjnum");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfObjectScalarPhase::UseObject
                    } else {
                        PdfObjectScalarPhase::StreamKeyword
                    };
                }
                PdfObjectScalarPhase::UseObject => {
                    let result = self.scan_integer_retained();
                    progress.use_object =
                        Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase = PdfObjectScalarPhase::StreamKeyword;
                }
                PdfObjectScalarPhase::StreamKeyword => {
                    let result = self.scan_keyword_retained("stream");
                    progress.stream = self.retain_structured_scalar(result, retained)?.value;
                    progress.phase = if progress.stream {
                        PdfObjectScalarPhase::AttributeKeyword
                    } else {
                        PdfObjectScalarPhase::FileKeyword
                    };
                }
                PdfObjectScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.stream_attr = match self.scan_balanced_text(true) {
                            Ok(value) => Some(value),
                            Err(error) => {
                                if error.is_resource_suspension() {
                                    self.retain_structured_scanner(
                                        PendingStructuredScannerPhase::PdfObjectStreamAttribute {
                                            use_object: progress.use_object,
                                        },
                                        StructuredScannerChildDestination::PdfObjectStreamAttribute,
                                    )?;
                                }
                                return Err(error);
                            }
                        };
                    }
                    progress.phase = PdfObjectScalarPhase::FileKeyword;
                }
                PdfObjectScalarPhase::FileKeyword => {
                    let result = self.scan_keyword_retained("file");
                    let file = self.retain_structured_scalar(result, retained)?.value;
                    let data = match self.scan_balanced_text(true) {
                        Ok(data) => data,
                        Err(error) => {
                            if error.is_resource_suspension() {
                                self.retain_structured_scanner(
                                    PendingStructuredScannerPhase::PdfObjectData {
                                        use_object: progress.use_object,
                                        stream: progress.stream,
                                        stream_attr: progress.stream_attr,
                                        file,
                                    },
                                    StructuredScannerChildDestination::PdfObjectData,
                                )?;
                            }
                            return Err(error);
                        }
                    };
                    return Ok(PdfObjectRequest::Define {
                        use_object: progress.use_object,
                        stream: progress.stream,
                        stream_attr: progress.stream_attr,
                        file,
                        data,
                    });
                }
            }
        }
    }

    pub fn scan_pdf_form_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfFormRequest, CommandError> {
        if primitive == UnexpandablePrimitive::PdfRefXForm {
            self.restore_structured_unary(StructuredUnaryScalar::PdfFormReference)?;
            let result = self.scan_integer_retained();
            return Ok(PdfFormRequest::Reference {
                object: self
                    .finish_structured_unary(result, StructuredUnaryScalar::PdfFormReference)?
                    .value,
            });
        }
        let pending = self.take_pending_structured_scanner()?;
        let (mut progress, mut child) = match pending {
            Some(pending) => {
                let PendingStructuredScanner { phase, mut child } = pending;
                match phase {
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::PdfForm(progress),
                    ) => (progress, child),
                    PendingStructuredScannerPhase::PdfFormAttribute => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfFormAttribute,
                        )?;
                        let attr = match self.scan_balanced_text(true) {
                            Ok(attr) => Some(attr),
                            Err(error) => {
                                if error.is_resource_suspension() {
                                    self.retain_structured_scanner(
                                        PendingStructuredScannerPhase::PdfFormAttribute,
                                        StructuredScannerChildDestination::PdfFormAttribute,
                                    )?;
                                }
                                return Err(error);
                            }
                        };
                        (
                            PdfFormScalarProgress {
                                attr,
                                resources: None,
                                phase: PdfFormScalarPhase::ResourcesKeyword,
                            },
                            None,
                        )
                    }
                    PendingStructuredScannerPhase::PdfFormResources { attr } => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfFormResources,
                        )?;
                        let resources = self.scan_balanced_text(true)?;
                        let result = self.scan_extended_register_index_retained();
                        let box_register = self.retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::PdfForm(PdfFormScalarProgress {
                                attr: attr.clone(),
                                resources: Some(resources.clone()),
                                phase: PdfFormScalarPhase::BoxRegister,
                            }),
                        )?;
                        return Ok(PdfFormRequest::Create {
                            attr,
                            resources: Some(resources),
                            box_register,
                        });
                    }
                    _ => {
                        if let Some(child) = child.take() {
                            self.abort_continuation(child.restore().0)?;
                        }
                        return Err(CommandError::input_invariant());
                    }
                }
            }
            None => (
                PdfFormScalarProgress {
                    attr: None,
                    resources: None,
                    phase: PdfFormScalarPhase::AttributeKeyword,
                },
                None,
            ),
        };
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = PendingStructuredScalarPhase::PdfForm(progress.clone());
            match progress.phase {
                PdfFormScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.attr = match self.scan_balanced_text(true) {
                            Ok(attr) => Some(attr),
                            Err(error) => {
                                if error.is_resource_suspension() {
                                    self.retain_structured_scanner(
                                        PendingStructuredScannerPhase::PdfFormAttribute,
                                        StructuredScannerChildDestination::PdfFormAttribute,
                                    )?;
                                }
                                return Err(error);
                            }
                        };
                    }
                    progress.phase = PdfFormScalarPhase::ResourcesKeyword;
                }
                PdfFormScalarPhase::ResourcesKeyword => {
                    let result = self.scan_keyword_retained("resources");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let resources = match self.scan_balanced_text(true) {
                            Ok(resources) => resources,
                            Err(error) => {
                                if error.is_resource_suspension() {
                                    self.retain_structured_scanner(
                                        PendingStructuredScannerPhase::PdfFormResources {
                                            attr: progress.attr,
                                        },
                                        StructuredScannerChildDestination::PdfFormResources,
                                    )?;
                                }
                                return Err(error);
                            }
                        };
                        let result = self.scan_extended_register_index_retained();
                        let box_register = self.retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::PdfForm(PdfFormScalarProgress {
                                attr: progress.attr.clone(),
                                resources: Some(resources.clone()),
                                phase: PdfFormScalarPhase::BoxRegister,
                            }),
                        )?;
                        return Ok(PdfFormRequest::Create {
                            attr: progress.attr,
                            resources: Some(resources),
                            box_register,
                        });
                    }
                    progress.phase = PdfFormScalarPhase::BoxRegister;
                }
                PdfFormScalarPhase::BoxRegister => {
                    let result = self.scan_extended_register_index_retained();
                    let box_register = self.retain_structured_scalar(result, retained)?;
                    return Ok(PdfFormRequest::Create {
                        attr: progress.attr,
                        resources: progress.resources,
                        box_register,
                    });
                }
            }
        }
    }

    pub fn scan_pdf_reference_object_request(
        &mut self,
    ) -> Result<PdfReferenceObjectRequest, CommandError> {
        self.restore_structured_unary(StructuredUnaryScalar::PdfReferenceObject)?;
        let result = self.scan_integer_retained();
        Ok(PdfReferenceObjectRequest {
            object: self
                .finish_structured_unary(result, StructuredUnaryScalar::PdfReferenceObject)?
                .value,
        })
    }

    pub fn scan_pdf_font_action(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedPdfFontAction, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (font, retained_first) = match pending {
            Some(pending) => {
                let PendingStructuredScanner { phase, mut child } = pending;
                match phase {
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::PdfFontAction { primitive: owner },
                    ) if owner == primitive => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::Scalar,
                        )?;
                        let scan = self.scan_font_selector_retained();
                        let font = self.retain_structured_scalar(
                            scan,
                            PendingStructuredScalarPhase::PdfFontAction { primitive },
                        )?;
                        (Some(font), None)
                    }
                    PendingStructuredScannerPhase::PdfGlyphName {
                        primitive: owner,
                        font,
                    } => {
                        if owner != primitive {
                            if let Some(child) = child.take() {
                                self.abort_continuation(child.restore().0)?;
                            }
                            return Err(CommandError::input_invariant());
                        }
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfGlyphName,
                        )?;
                        (font, None)
                    }
                    PendingStructuredScannerPhase::PdfGlyphUnicode { font, first }
                        if primitive == UnexpandablePrimitive::PdfGlyphToUnicode =>
                    {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::PdfGlyphUnicode,
                        )?;
                        (font, Some(first))
                    }
                    _ => {
                        if let Some(child) = child.take() {
                            self.abort_continuation(child.restore().0)?;
                        }
                        return Err(CommandError::input_invariant());
                    }
                }
            }
            None => {
                let needs_font = matches!(
                    primitive,
                    UnexpandablePrimitive::PdfFontAttr
                        | UnexpandablePrimitive::PdfIncludeChars
                        | UnexpandablePrimitive::PdfNoBuiltinToUnicode
                );
                let font = if needs_font {
                    let scan = self.scan_font_selector_retained();
                    Some(self.retain_structured_scalar(
                        scan,
                        PendingStructuredScalarPhase::PdfFontAction { primitive },
                    )?)
                } else {
                    None
                };
                (font, None)
            }
        };
        if primitive == UnexpandablePrimitive::PdfNoBuiltinToUnicode {
            return Ok(ScannedPdfFontAction {
                font,
                first: None,
                second: None,
            });
        }
        let first = if let Some(first) = retained_first {
            first
        } else {
            match self.scan_balanced_text(true) {
                Ok(first) => first.tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        self.retain_structured_scanner(
                            PendingStructuredScannerPhase::PdfGlyphName { primitive, font },
                            StructuredScannerChildDestination::PdfGlyphName,
                        )?;
                    }
                    return Err(error);
                }
            }
        };
        let second = if primitive == UnexpandablePrimitive::PdfGlyphToUnicode {
            match self.scan_balanced_text(true) {
                Ok(second) => Some(second.tokens),
                Err(error) => {
                    if error.is_resource_suspension() {
                        self.retain_structured_scanner(
                            PendingStructuredScannerPhase::PdfGlyphUnicode { font, first },
                            StructuredScannerChildDestination::PdfGlyphUnicode,
                        )?;
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(ScannedPdfFontAction {
            font,
            first: Some(first),
            second,
        })
    }

    pub fn scan_pdf_document_fragment_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<PdfDocumentFragmentRequest, CommandError> {
        use tex_state::PdfDocumentFragmentKind as Kind;
        let pending = self.take_pending_structured_scanner()?;
        let (kind, text) = match pending {
            Some(pending) => {
                let PendingStructuredScanner { phase, mut child } = pending;
                match phase {
                    PendingStructuredScannerPhase::PdfDocumentFragmentText { kind } => {
                        let (text, phase) = self.scan_pdf_navigation_text(
                            &mut child,
                            PendingStructuredScannerPhase::PdfDocumentFragmentText { kind },
                            StructuredScannerChildDestination::PdfDocumentFragmentText,
                        )?;
                        let PendingStructuredScannerPhase::PdfDocumentFragmentText { kind } = phase
                        else {
                            return Err(CommandError::input_invariant());
                        };
                        (kind, text)
                    }
                    PendingStructuredScannerPhase::PdfAction { owner, phase } => {
                        let (owner, action) =
                            self.scan_pdf_action_for_owner(owner, Some((phase, child)))?;
                        let PendingPdfActionOwner::DocumentFragment { kind, text } = owner else {
                            return Err(CommandError::input_invariant());
                        };
                        return Ok(PdfDocumentFragmentRequest {
                            kind,
                            text,
                            open_action: Some(action),
                        });
                    }
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::PdfAction { owner, progress },
                    ) => {
                        let (owner, action) =
                            self.scan_pdf_action_scalar(owner, progress, child)?;
                        let PendingPdfActionOwner::DocumentFragment { kind, text } = owner else {
                            return Err(CommandError::input_invariant());
                        };
                        return Ok(PdfDocumentFragmentRequest {
                            kind,
                            text,
                            open_action: Some(action),
                        });
                    }
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::PdfDocumentOpenAction { kind, text },
                    ) => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::Scalar,
                        )?;
                        let result = self.scan_keyword_retained("openaction");
                        let open_action = self
                            .retain_structured_scalar(
                                result,
                                PendingStructuredScalarPhase::PdfDocumentOpenAction {
                                    kind,
                                    text: text.clone(),
                                },
                            )?
                            .value;
                        if open_action {
                            let (owner, action) = self.scan_pdf_action_for_owner(
                                PendingPdfActionOwner::DocumentFragment { kind, text },
                                None,
                            )?;
                            let PendingPdfActionOwner::DocumentFragment { kind, text } = owner
                            else {
                                return Err(CommandError::input_invariant());
                            };
                            return Ok(PdfDocumentFragmentRequest {
                                kind,
                                text,
                                open_action: Some(action),
                            });
                        }
                        return Ok(PdfDocumentFragmentRequest {
                            kind,
                            text,
                            open_action: None,
                        });
                    }
                    _ => {
                        if let Some(child) = child.take() {
                            self.abort_continuation(child.restore().0)?;
                        }
                        return Err(CommandError::input_invariant());
                    }
                }
            }
            None => {
                let kind = match primitive {
                    UnexpandablePrimitive::PdfInfo => Kind::Info,
                    UnexpandablePrimitive::PdfCatalog => Kind::Catalog,
                    UnexpandablePrimitive::PdfNames => Kind::Names,
                    UnexpandablePrimitive::PdfTrailer => Kind::Trailer,
                    UnexpandablePrimitive::PdfTrailerId => Kind::TrailerId,
                    _ => return Err(CommandError::input_invariant()),
                };
                let (text, phase) = self.scan_pdf_navigation_text(
                    &mut None,
                    PendingStructuredScannerPhase::PdfDocumentFragmentText { kind },
                    StructuredScannerChildDestination::PdfDocumentFragmentText,
                )?;
                let PendingStructuredScannerPhase::PdfDocumentFragmentText { kind } = phase else {
                    return Err(CommandError::input_invariant());
                };
                (kind, text)
            }
        };
        let open_action = if kind == Kind::Catalog && {
            let result = self.scan_keyword_retained("openaction");
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::PdfDocumentOpenAction {
                    kind,
                    text: text.clone(),
                },
            )?
            .value
        } {
            let (owner, action) = self.scan_pdf_action_for_owner(
                PendingPdfActionOwner::DocumentFragment { kind, text },
                None,
            )?;
            let PendingPdfActionOwner::DocumentFragment { kind, text } = owner else {
                return Err(CommandError::input_invariant());
            };
            return Ok(PdfDocumentFragmentRequest {
                kind,
                text,
                open_action: Some(action),
            });
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
    fn scan_math_field_restricted(
        &mut self,
        provenance: StructuredProvenance,
        kind: MathFieldRestrictedKind,
    ) -> Result<Option<MathFieldEpisode>, CommandError> {
        let class = match kind {
            MathFieldRestrictedKind::Character => RestrictedIntegerClass::CharacterCode,
            MathFieldRestrictedKind::MathCharacter => RestrictedIntegerClass::FifteenBit,
            MathFieldRestrictedKind::Delimiter => RestrictedIntegerClass::TwentySevenBit,
        };
        let result = self.scan_restricted_integer_retained(class);
        let scanned = self.retain_structured_scalar(
            result,
            PendingStructuredScalarPhase::MathFieldRestricted { provenance, kind },
        )?;
        let (code, provenance) = match kind {
            MathFieldRestrictedKind::Character => {
                let ch = char::from_u32(scanned.value as u32)
                    .expect("recovered character number is in range");
                let code = self.state.mathcode(ch);
                if code == 0o100000 {
                    self.treat_as_active_character(ch, provenance.primary)?;
                    return Ok(None);
                }
                (code as u16, provenance)
            }
            MathFieldRestrictedKind::MathCharacter => (
                scanned.value as u16,
                StructuredProvenance {
                    primary: scanned.provenance.primary,
                },
            ),
            MathFieldRestrictedKind::Delimiter => (
                (scanned.value as u32 / 0o10000) as u16,
                StructuredProvenance {
                    primary: scanned.provenance.primary,
                },
            ),
        };
        Ok(Some(MathFieldEpisode {
            body: MathFieldBody::Character(code),
            provenance,
        }))
    }

    pub fn scan_math_field_episode(&mut self) -> Result<MathFieldEpisode, CommandError> {
        if let Some(pending) = self.take_pending_structured_scanner()? {
            let PendingStructuredScanner { phase, mut child } = pending;
            let PendingStructuredScannerPhase::Scalar(
                PendingStructuredScalarPhase::MathFieldRestricted { provenance, kind },
            ) = phase
            else {
                if let Some(child) = child.take() {
                    self.abort_continuation(child.restore().0)?;
                }
                return Err(CommandError::input_invariant());
            };
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            if let Some(field) = self.scan_math_field_restricted(provenance, kind)? {
                return Ok(field);
            }
        }
        let mut destination = None;
        loop {
            // §1151's `restart` label: §404's shared "next non-blank
            // non-relax non-call token", the same fetch §403 opens with.
            match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
                DeliveryStatus::End => {
                    return Ok(MathFieldEpisode {
                        body: MathFieldBody::Missing,
                        provenance: StructuredProvenance {
                            primary: OriginId::UNKNOWN,
                        },
                    });
                }
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            };
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            let provenance = StructuredProvenance {
                primary: command.origin(),
            };
            // §1151's `reswitch`: `char_num` scans its selector and re-enters
            // the table as `char_given`, so both reach one `math_code` read.
            let character = match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }) => Some(ch),
                Some(Meaning::CharGiven(ch)) => Some(ch),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)) => {
                    if let Some(field) = self.scan_math_field_restricted(
                        provenance,
                        MathFieldRestrictedKind::Character,
                    )? {
                        return Ok(field);
                    }
                    continue;
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
            let (code, provenance) = match static_meaning(command.meaning()) {
                // §1224's `\mathchardef` target carries its own code.
                Some(Meaning::MathCharGiven(code)) => (code, provenance),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::MathChar)) => {
                    return self
                        .scan_math_field_restricted(
                            provenance,
                            MathFieldRestrictedKind::MathCharacter,
                        )?
                        .ok_or_else(|| CommandError::input_invariant());
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter)) => {
                    return self
                        .scan_math_field_restricted(provenance, MathFieldRestrictedKind::Delimiter)?
                        .ok_or_else(|| CommandError::input_invariant());
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
        self.restore_structured_unary(StructuredUnaryScalar::MathCharacter)?;
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FifteenBit);
        let scanned = self.finish_structured_unary(result, StructuredUnaryScalar::MathCharacter)?;
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
        self.restore_structured_unary(StructuredUnaryScalar::DelimiterNumber)?;
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::TwentySevenBit);
        let scanned =
            self.finish_structured_unary(result, StructuredUnaryScalar::DelimiterNumber)?;
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
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => {
                return Ok(ScannedMathDelimiter {
                    code: 0,
                    recovered: true,
                    missing_delimiter: true,
                    provenance: StructuredProvenance {
                        primary: OriginId::UNKNOWN,
                    },
                });
            }
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let primary = command.origin();
        let code = match static_meaning(command.meaning()) {
            Some(Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            }) => self.state.delcode(ch),
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Delimiter)) => {
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
        let context = self.command.output_open_context(self.state);
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
                    integer_error: None,
                });
            return Ok(());
        }
        let mut report = self.state.print_err("Missing delimiter (. inserted)");
        report.help(MISSING_DELIMITER_HELP).context(context);
        let outcome = report.error();
        self.finish_error_outcome(outcome)?;
        Ok(())
    }

    fn push_alignment_live_token(
        &mut self,
        builder: TokenBuilderId,
        spelling: TracedTokenWord,
    ) -> Result<(), CommandError> {
        let live_tokens = self
            .command
            .transient
            .builders
            .iter()
            .find(|live| live.identity == builder.0)
            .ok_or(CommandError::input_invariant())?
            .tokens;
        self.command
            .attempt
            .arena_mut()
            .push_buffer_token(live_tokens, spelling)
            .map_err(|_| CommandError::input_invariant())
    }

    pub(crate) fn abort_alignment_preamble(
        &mut self,
        mut pending: PendingAlignmentPreamble<G>,
    ) -> Result<(), CommandError> {
        if let Some(child) = pending.take_child() {
            self.abort_continuation(child)?;
        }
        self.finish_scanner_episode(pending.scanner_episode);
        self.command
            .transient
            .builders
            .retain(|live| live.identity != pending.builder.0);
        Ok(())
    }

    fn retain_alignment_scalar(
        &mut self,
        pending: PendingAlignmentPreamble<G>,
        child: crate::ScannerFrameKey<G>,
        error: CommandError,
    ) -> Result<(), CommandError> {
        self.install_scanner_resume(Some(child));
        let key = match self.command.scratch.store_alignment_preamble_frame(pending) {
            Ok(key) => key,
            Err(store_error) => {
                if let Some(child) = self.scanner_resume.take() {
                    self.abort_continuation(child)?;
                }
                return Err(crate::scan_toks::scratch_command_error(store_error));
            }
        };
        let frame = match self.command.scratch.alignment_preamble_frame_mut(&key) {
            Ok(frame) => frame,
            Err(store_error) => {
                let abort_result = if let Some(child) = self.scanner_resume.take() {
                    self.abort_continuation(child)
                } else {
                    Ok(())
                };
                let discard_result = self
                    .command
                    .scratch
                    .discard_alignment_preamble_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error);
                abort_result?;
                discard_result?;
                return Err(crate::scan_toks::scratch_command_error(store_error));
            }
        };
        let Some(scalar) = frame.scalar_scan.as_mut() else {
            let abort_result = if let Some(child) = self.scanner_resume.take() {
                self.abort_continuation(child)
            } else {
                Ok(())
            };
            let discard_result = self
                .command
                .scratch
                .discard_alignment_preamble_frame(key)
                .map_err(crate::scan_toks::scratch_command_error);
            abort_result?;
            discard_result?;
            return Err(CommandError::input_invariant());
        };
        scalar.child = crate::execution_scratch::ChildContinuation::capture(
            &mut self.scanner_resume,
            AlignmentPreambleChildDestination::Scalar,
        );
        if self.scanner_resume.replace(key).is_some() {
            return Err(CommandError::input_invariant());
        }
        Err(error)
    }

    /// Scans TeX82 §435's `scan_four_bit_int` family index, the prefix common
    /// to the three math-font assignment primitives (§1234's `def_family`).
    /// The later font-meaning scan is intentionally not part of this request.
    pub fn scan_math_family(
        &mut self,
        size: MathFamilySize,
    ) -> Result<ScannedMathFamily, CommandError> {
        self.restore_structured_unary(StructuredUnaryScalar::MathFamily(size))?;
        let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
        let scanned =
            self.finish_structured_unary(result, StructuredUnaryScalar::MathFamily(size))?;
        Ok(ScannedMathFamily {
            size,
            family: scanned.value as u8,
            recovered: scanned.recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    pub fn scan_math_family_retained(
        &mut self,
        size: MathFamilySize,
    ) -> crate::RetainedScalarScan<G, ScannedMathFamily> {
        let result = self.scan_math_family(size);
        self.detach_retained_scalar(result)
    }

    /// Collects the command-owned scalar prefix of TeX82's generalized
    /// fraction forms. Numerator/denominator mlist construction stays in the
    /// executor and is deliberately absent from this scanner boundary.
    pub fn scan_math_fraction(
        &mut self,
        kind: MathFractionKind,
        with_delimiters: bool,
    ) -> Result<ScannedMathFraction, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (left_delimiter, right_delimiter, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::MathFractionThickness {
                            kind: retained_kind,
                            left_delimiter,
                            right_delimiter,
                        },
                    ),
                child,
            }) if retained_kind == kind => (left_delimiter, right_delimiter, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None if with_delimiters => (
                Some(self.scan_delimiter(false)?),
                Some(self.scan_delimiter(false)?),
                None,
            ),
            None => (None, None, None),
        };
        let thickness = match kind {
            MathFractionKind::Above => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_dimension_retained();
                Some(
                    self.retain_structured_scalar(
                        result,
                        PendingStructuredScalarPhase::MathFractionThickness {
                            kind,
                            left_delimiter,
                            right_delimiter,
                        },
                    )?
                    .value,
                )
            }
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
        self.restore_structured_unary(StructuredUnaryScalar::MathMu(glue))?;
        if glue {
            let result = self.scan_glue_retained(true);
            Ok(ScannedMathMuMaterial::Glue(
                self.finish_structured_unary(result, StructuredUnaryScalar::MathMu(glue))?
                    .value,
            ))
        } else {
            let result = self.scan_mu_dimension_retained();
            Ok(ScannedMathMuMaterial::Kern(
                self.finish_structured_unary(result, StructuredUnaryScalar::MathMu(glue))?
                    .value,
            ))
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
        command: &crate::CurrentCommand<G>,
    ) -> Result<Option<MathRequest>, CommandError> {
        use MathRequest as Request;
        use MathTextFieldKind as Field;
        // TeX82 §1154's `mmode+math_given: set_math_char(cur_chr)`. Unlike
        // `mmode+math_char_num`, which reaches the same `set_math_char`
        // (§1155) through §436's `scan_fifteen_bit_int`, the code is already
        // complete in the delivered command, so nothing is scanned and the
        // math char's provenance is the delivering token's own origin.
        if let Some(Meaning::MathCharGiven(code)) = static_meaning(command.meaning()) {
            return Ok(Some(Request::Character(ScannedMathCharacter {
                code,
                recovered: false,
                provenance: StructuredProvenance {
                    primary: command.origin(),
                },
            })));
        }
        let Some(Meaning::UnexpandablePrimitive(primitive)) = static_meaning(command.meaning())
        else {
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
        let pending = self.take_pending_structured_scanner()?;
        let (mut phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::InputStream {
                        primitive: retained_primitive,
                        read_global: retained_global,
                        phase,
                    }),
                child,
            }) if retained_primitive == primitive && retained_global == read_global => {
                (phase, child)
            }
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (InputStreamScalarPhase::Selector, None),
        };
        let retained = |phase| PendingStructuredScalarPhase::InputStream {
            primitive,
            read_global,
            phase,
        };
        match primitive {
            // §§1272--1275's `in_stream` command scans §435's
            // `scan_four_bit_int`. Recovery is complete before the request is
            // committed; the raw value crosses the apply seam only so §435's
            // `int_error` can report it first.
            UnexpandablePrimitive::OpenIn => {
                let scanned = match phase {
                    InputStreamScalarPhase::Selector => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::Scalar,
                        )?;
                        let result =
                            self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
                        let scanned =
                            self.retain_structured_scalar(result, retained(phase.clone()))?;
                        phase = InputStreamScalarPhase::OpenEquals { scanned };
                        scanned
                    }
                    InputStreamScalarPhase::OpenEquals { scanned }
                    | InputStreamScalarPhase::OpenFileName { scanned } => scanned,
                    _ => return Err(CommandError::input_invariant()),
                };
                if matches!(phase, InputStreamScalarPhase::OpenEquals { .. }) {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::Scalar,
                    )?;
                    let result = self.scan_optional_equals_retained();
                    self.retain_structured_scalar(result, retained(phase.clone()))?;
                    phase = InputStreamScalarPhase::OpenFileName { scanned };
                }
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_file_name_retained();
                let file_name = self.retain_structured_scalar(result, retained(phase))?;
                Ok(InputStreamRequest::Open {
                    stream: scanned.value,
                    scanned: scanned.scanned,
                    recovered: scanned.recovered,
                    file_name,
                })
            }
            UnexpandablePrimitive::CloseIn => {
                if !matches!(phase, InputStreamScalarPhase::Selector) {
                    return Err(CommandError::input_invariant());
                }
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
                let scanned = self.retain_structured_scalar(result, retained(phase))?;
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
                let stream = match phase {
                    InputStreamScalarPhase::Selector => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::Scalar,
                        )?;
                        let result = self.scan_integer_retained();
                        self.retain_structured_scalar(result, retained(phase))?
                            .value
                    }
                    InputStreamScalarPhase::ReadTo { stream } => stream,
                    _ => return Err(CommandError::input_invariant()),
                };
                // tex.web §1225 reports a missing `to` and inserts it, then
                // runs `get_r_token` regardless: the keyword is recovered,
                // not required. §1225 reports it *here*, between the failed
                // keyword and `get_r_token`, so §82's context still shows the
                // target as `<to be read again>` and no `read_toks` prompt has
                // been printed yet.
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("to");
                let found_to = self
                    .retain_structured_scalar(
                        result,
                        retained(InputStreamScalarPhase::ReadTo { stream }),
                    )?
                    .value;
                if !found_to {
                    let context = self.command.output_open_context(self.state);
                    let mut report = self.state.print_err("Missing `to' inserted");
                    report.help(&[
                        "You should have said `\\read<number> to \\cs'.",
                        "I'm going to look for the \\cs now.",
                    ]);
                    report.context(context);
                    let outcome = report.error();
                    self.finish_error_outcome(outcome)?;
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
                let parameter_text = self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_list([])
                    .map_err(crate::scan_toks::attempt_command_error)?;
                let definition = self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_definition(parameter_text, tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?;
                Ok(InputStreamRequest::Read {
                    stream,
                    target,
                    global: read_global,
                    tokens,
                    definition,
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
        let pending = self.take_pending_structured_scanner()?;
        let (target, mut phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::FontDefinition { target, phase },
                    ),
                child,
            }) => (target, phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => {
                let mut destination = None;
                if self.next_non_space_raw_into(&mut destination)? != DeliveryStatus::Command {
                    return Err(CommandError::input_invariant());
                }
                let command = destination.take().ok_or(CommandError::input_invariant())?;
                let target = self
                    .delivered_definition_target(&command)
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
                (target, FontDefinitionScalarPhase::Equals, None)
            }
        };
        let retained = |phase| PendingStructuredScalarPhase::FontDefinition { target, phase };
        if matches!(phase, FontDefinitionScalarPhase::Equals) {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(result, retained(phase.clone()))?;
            phase = FontDefinitionScalarPhase::FileName;
        }
        let file_name = match phase {
            FontDefinitionScalarPhase::FileName => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_file_name_retained();
                let file_name = self.retain_structured_scalar(result, retained(phase))?;
                phase = FontDefinitionScalarPhase::AtKeyword {
                    file_name: file_name.clone(),
                };
                file_name
            }
            FontDefinitionScalarPhase::AtKeyword { ref file_name }
            | FontDefinitionScalarPhase::AtDimension { ref file_name }
            | FontDefinitionScalarPhase::ScaledKeyword { ref file_name }
            | FontDefinitionScalarPhase::ScaledInteger { ref file_name } => file_name.clone(),
            FontDefinitionScalarPhase::Equals => unreachable!("equals advanced to filename"),
        };
        let mut size_recovery = None;
        let size = if matches!(phase, FontDefinitionScalarPhase::AtDimension { .. })
            || matches!(phase, FontDefinitionScalarPhase::AtKeyword { .. }) && {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("at");
                self.retain_structured_scalar(result, retained(phase.clone()))?
                    .value
            } {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_dimension_retained();
            let requested = self
                .retain_structured_scalar(
                    result,
                    retained(FontDefinitionScalarPhase::AtDimension {
                        file_name: file_name.clone(),
                    }),
                )?
                .value;
            // §1259's `if (s<=0)or(s>=@'1000000000)`.
            FontSizeSpec::At(
                if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
                    requested
                } else {
                    size_recovery = Some(FontSizeRecovery::ImproperAtSize {
                        size: requested,
                        context: self.command.output_open_context(self.state),
                    });
                    Scaled::from_raw(10 * Scaled::UNITY)
                },
            )
        } else {
            let scaled = if matches!(phase, FontDefinitionScalarPhase::ScaledInteger { .. }) {
                true
            } else {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("scaled");
                self.retain_structured_scalar(
                    result,
                    retained(FontDefinitionScalarPhase::ScaledKeyword {
                        file_name: file_name.clone(),
                    }),
                )?
                .value
            };
            if scaled {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_integer_retained();
                let requested = self
                    .retain_structured_scalar(
                        result,
                        retained(FontDefinitionScalarPhase::ScaledInteger {
                            file_name: file_name.clone(),
                        }),
                    )?
                    .value;
                // §1258's `if (cur_val<=0)or(cur_val>32768)`.
                FontSizeSpec::Scale(if (1..=32_768).contains(&requested) {
                    requested
                } else {
                    size_recovery = Some(FontSizeRecovery::IllegalMagnification {
                        value: requested,
                        context: self.command.output_open_context(self.state),
                    });
                    1000
                })
            } else {
                FontSizeSpec::Design
            }
        };
        Ok(FontLoadRequest {
            target,
            name: file_name.packed(),
            size,
            size_recovery,
            error_context: self.command.output_open_context(self.state),
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
        let pending = self.take_pending_structured_scanner()?;
        let (target, phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::GeneratedFont {
                        kind: retained_kind,
                        target,
                        phase,
                    }),
                child,
            }) if retained_kind == kind => (target, phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => {
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
                (target, GeneratedFontScalarPhase::Equals, None)
            }
        };
        let retained = |phase| PendingStructuredScalarPhase::GeneratedFont {
            kind,
            target,
            phase,
        };
        if matches!(phase, GeneratedFontScalarPhase::Equals) {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(result, retained(phase))?;
        }
        let source = match phase {
            GeneratedFontScalarPhase::Amount { source }
            | GeneratedFontScalarPhase::NoLigatures { source, .. } => source,
            GeneratedFontScalarPhase::Equals | GeneratedFontScalarPhase::Source => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_font_selector_retained();
                self.retain_structured_scalar(result, retained(GeneratedFontScalarPhase::Source))?
            }
        };
        let (amount, no_ligatures) = match kind {
            GeneratedFontKind::Copy => (0, false),
            GeneratedFontKind::Letterspace => {
                let amount = match phase {
                    GeneratedFontScalarPhase::NoLigatures { amount, .. } => amount,
                    _ => {
                        self.restore_structured_scanner_child(
                            &mut child,
                            StructuredScannerChildDestination::Scalar,
                        )?;
                        let result = self.scan_integer_retained();
                        self.retain_structured_scalar(
                            result,
                            retained(GeneratedFontScalarPhase::Amount { source }),
                        )?
                        .value
                        .clamp(-1000, 1000) as i16
                    }
                };
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("nolig");
                let no_ligatures = self
                    .retain_structured_scalar(
                        result,
                        retained(GeneratedFontScalarPhase::NoLigatures { source, amount }),
                    )?
                    .value;
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
        let pending = self.take_pending_structured_scanner()?;
        let (mut progress, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::PdfImage(
                        progress,
                    )),
                child,
            }) => (progress, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::PdfImageAttribute {
                        width,
                        height,
                        depth,
                    },
                mut child,
            }) => {
                let (attr, _) = self.scan_pdf_navigation_text(
                    &mut child,
                    PendingStructuredScannerPhase::PdfImageAttribute {
                        width,
                        height,
                        depth,
                    },
                    StructuredScannerChildDestination::PdfImageAttribute,
                )?;
                (
                    PdfImageScalarProgress {
                        width,
                        height,
                        depth,
                        attr: Some(attr.tokens),
                        page: PendingPdfImagePage::Unset,
                        color_space_object: 0,
                        page_box: None,
                        phase: PdfImageScalarPhase::NamedKeyword,
                    },
                    None,
                )
            }
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::PdfImagePageName {
                        width,
                        height,
                        depth,
                        attr,
                    },
                mut child,
            }) => {
                let (text, _) = self.scan_pdf_navigation_text(
                    &mut child,
                    PendingStructuredScannerPhase::PdfImagePageName {
                        width,
                        height,
                        depth,
                        attr,
                    },
                    StructuredScannerChildDestination::PdfImagePageName,
                )?;
                (
                    PdfImageScalarProgress {
                        width,
                        height,
                        depth,
                        attr,
                        page: PendingPdfImagePage::Named(text.tokens),
                        color_space_object: 0,
                        page_box: None,
                        phase: PdfImageScalarPhase::ColorSpaceKeyword,
                    },
                    None,
                )
            }
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (
                PdfImageScalarProgress {
                    width: None,
                    height: None,
                    depth: None,
                    attr: None,
                    page: PendingPdfImagePage::Unset,
                    color_space_object: 0,
                    page_box: None,
                    phase: PdfImageScalarPhase::WidthKeyword,
                },
                None,
            ),
        };
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained = PendingStructuredScalarPhase::PdfImage(progress);
            match progress.phase {
                PdfImageScalarPhase::WidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfImageScalarPhase::WidthDimension
                    } else {
                        PdfImageScalarPhase::HeightKeyword
                    };
                }
                PdfImageScalarPhase::WidthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.width = Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::HeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfImageScalarPhase::HeightDimension
                    } else {
                        PdfImageScalarPhase::DepthKeyword
                    };
                }
                PdfImageScalarPhase::HeightDimension => {
                    let result = self.scan_dimension_retained();
                    progress.height = Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::DepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfImageScalarPhase::DepthDimension
                    } else {
                        PdfImageScalarPhase::AttributeKeyword
                    };
                }
                PdfImageScalarPhase::DepthDimension => {
                    let result = self.scan_dimension_retained();
                    progress.depth = Some(self.retain_structured_scalar(result, retained)?.value);
                    progress.phase = PdfImageScalarPhase::WidthKeyword;
                }
                PdfImageScalarPhase::AttributeKeyword => {
                    let result = self.scan_keyword_retained("attr");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let (attr, _) = self.scan_pdf_navigation_text(
                            &mut None,
                            PendingStructuredScannerPhase::PdfImageAttribute {
                                width: progress.width,
                                height: progress.height,
                                depth: progress.depth,
                            },
                            StructuredScannerChildDestination::PdfImageAttribute,
                        )?;
                        progress.attr = Some(attr.tokens);
                    }
                    progress.phase = PdfImageScalarPhase::NamedKeyword;
                }
                PdfImageScalarPhase::NamedKeyword => {
                    let result = self.scan_keyword_retained("named");
                    if self.retain_structured_scalar(result, retained)?.value {
                        let (text, _) = self.scan_pdf_navigation_text(
                            &mut None,
                            PendingStructuredScannerPhase::PdfImagePageName {
                                width: progress.width,
                                height: progress.height,
                                depth: progress.depth,
                                attr: progress.attr,
                            },
                            StructuredScannerChildDestination::PdfImagePageName,
                        )?;
                        progress.page = PendingPdfImagePage::Named(text.tokens);
                        progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                    } else {
                        progress.phase = PdfImageScalarPhase::PageKeyword;
                    }
                }
                PdfImageScalarPhase::PageKeyword => {
                    let result = self.scan_keyword_retained("page");
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.phase = PdfImageScalarPhase::PageNumber;
                    } else {
                        progress.page = PendingPdfImagePage::Number(1);
                        progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                    }
                }
                PdfImageScalarPhase::PageNumber => {
                    let result = self.scan_integer_retained();
                    progress.page = PendingPdfImagePage::Number(
                        self.retain_structured_scalar(result, retained)?.value,
                    );
                    progress.phase = PdfImageScalarPhase::ColorSpaceKeyword;
                }
                PdfImageScalarPhase::ColorSpaceKeyword => {
                    let result = self.scan_keyword_retained("colorspace");
                    progress.phase = if self.retain_structured_scalar(result, retained)?.value {
                        PdfImageScalarPhase::ColorSpaceObject
                    } else {
                        PdfImageScalarPhase::MediaBox
                    };
                }
                PdfImageScalarPhase::ColorSpaceObject => {
                    let result = self.scan_integer_retained();
                    progress.color_space_object =
                        self.retain_structured_scalar(result, retained)?.value;
                    progress.phase = PdfImageScalarPhase::MediaBox;
                }
                PdfImageScalarPhase::MediaBox
                | PdfImageScalarPhase::CropBox
                | PdfImageScalarPhase::BleedBox
                | PdfImageScalarPhase::TrimBox
                | PdfImageScalarPhase::ArtBox => {
                    let (keyword, selected, next) = match progress.phase {
                        PdfImageScalarPhase::MediaBox => (
                            "mediabox",
                            PdfImagePageBox::Media,
                            PdfImageScalarPhase::CropBox,
                        ),
                        PdfImageScalarPhase::CropBox => (
                            "cropbox",
                            PdfImagePageBox::Crop,
                            PdfImageScalarPhase::BleedBox,
                        ),
                        PdfImageScalarPhase::BleedBox => (
                            "bleedbox",
                            PdfImagePageBox::Bleed,
                            PdfImageScalarPhase::TrimBox,
                        ),
                        PdfImageScalarPhase::TrimBox => (
                            "trimbox",
                            PdfImagePageBox::Trim,
                            PdfImageScalarPhase::ArtBox,
                        ),
                        PdfImageScalarPhase::ArtBox => (
                            "artbox",
                            PdfImagePageBox::Art,
                            PdfImageScalarPhase::FileName,
                        ),
                        _ => unreachable!(),
                    };
                    let result = self.scan_keyword_retained(keyword);
                    if self.retain_structured_scalar(result, retained)?.value {
                        progress.page_box = Some(selected);
                        progress.phase = PdfImageScalarPhase::FileName;
                    } else {
                        progress.phase = next;
                    }
                }
                PdfImageScalarPhase::FileName => {
                    let result = self.scan_file_name_retained();
                    let name = self.retain_structured_scalar(result, retained)?.packed();
                    let page = match progress.page {
                        PendingPdfImagePage::Unset => PdfImagePageSelection::Number(1),
                        PendingPdfImagePage::Number(page) => PdfImagePageSelection::Number(page),
                        PendingPdfImagePage::Named(tokens) => {
                            let semantic = self
                                .command
                                .attempt
                                .arena()
                                .token_words(tokens)
                                .map_err(|_| CommandError::input_invariant())?
                                .iter()
                                .map(|word| word.semantic_token())
                                .collect::<Vec<_>>();
                            PdfImagePageSelection::Named(
                                crate::processor::expand::token_slice_string_text(
                                    self.state, &semantic,
                                )
                                .into_bytes(),
                            )
                        }
                    };
                    let page_box = progress.page_box;
                    return Ok(PdfImageRequest {
                        name,
                        width: progress.width,
                        height: progress.height,
                        depth: progress.depth,
                        page,
                        color_space_object: progress.color_space_object,
                        page_box_explicit: page_box.is_some(),
                        page_box: page_box.unwrap_or(PdfImagePageBox::Crop),
                        attr: progress.attr,
                    });
                }
            }
        }
    }
    /// Scans TeX82 §1123's `make_accent` accent code.
    ///
    /// §1123 is `scan_char_num; f:=cur_font; p:=new_character(f,cur_val)` and
    /// only then `do_assignments`, so the accent code is the whole of what the
    /// command layer owns before the executor takes over.
    pub fn scan_accent(&mut self) -> Result<ScannedAccent, CommandError> {
        self.restore_structured_unary(StructuredUnaryScalar::Accent)?;
        let result = self.scan_integer_retained();
        let accent = self.finish_structured_unary(result, StructuredUnaryScalar::Accent)?;
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
    pub fn scan_accent_base(&mut self) -> Result<ScannedAccentBase<G>, CommandError> {
        if let Some(pending) = self.take_pending_structured_scanner()? {
            let PendingStructuredScanner { phase, mut child } = pending;
            return match phase {
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::AccentBaseCharacter { provenance },
                ) => {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::Scalar,
                    )?;
                    let result = self.scan_integer_retained();
                    let character = u8::try_from(
                        self.retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::AccentBaseCharacter { provenance },
                        )?
                        .value,
                    )
                    .map_err(|_| CommandError::input_invariant())?;
                    Ok(ScannedAccentBase::Character {
                        character,
                        provenance,
                    })
                }
                _ => {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    Err(CommandError::input_invariant())
                }
            };
        }
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => return Ok(ScannedAccentBase::Missing),
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let provenance = StructuredProvenance {
            primary: command.origin(),
        };
        match static_meaning(command.meaning()) {
            Some(Meaning::CharToken {
                ch,
                cat: Catcode::Letter | Catcode::Other,
            })
            | Some(Meaning::CharGiven(ch))
            | Some(Meaning::CharToken {
                ch,
                cat: Catcode::Active,
            }) => {
                let character =
                    u8::try_from(ch as u32).map_err(|_| CommandError::input_invariant())?;
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char)) => {
                let result = self.scan_integer_retained();
                let character = u8::try_from(
                    self.retain_structured_scalar(
                        result,
                        PendingStructuredScalarPhase::AccentBaseCharacter { provenance },
                    )?
                    .value,
                )
                .map_err(|_| CommandError::input_invariant())?;
                Ok(ScannedAccentBase::Character {
                    character,
                    provenance,
                })
            }
            Some(meaning) if crate::primitives::is_prefixed_command(meaning) => {
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
    pub fn next_do_assignments_command(
        &mut self,
    ) -> Result<Option<CurrentCommand<G>>, CommandError> {
        let mut destination = None;
        match self.next_non_blank_non_relax_x_token_into(&mut destination)? {
            DeliveryStatus::End => Ok(None),
            DeliveryStatus::Command => Ok(destination),
            _ => Err(CommandError::input_invariant()),
        }
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
        self.restore_structured_unary(StructuredUnaryScalar::WriteStream)?;
        let result = self.scan_integer_retained();
        let value = self
            .finish_structured_unary(result, StructuredUnaryScalar::WriteStream)?
            .value;
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
        if self
            .scanner_resume
            .as_ref()
            .is_some_and(crate::ScannerFrameKey::is_structured_scanner)
        {
            let pending = self.take_pending_structured_scanner()?;
            let Some(PendingStructuredScanner { phase, child }) = pending else {
                return Err(CommandError::input_invariant());
            };
            match phase {
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::ImmediateOpenOut(phase),
                ) => return self.finish_immediate_open_out(phase, child),
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::ImmediateWriteStream { close },
                ) => {
                    let mut child = child;
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::Scalar,
                    )?;
                    let stream = self.scan_immediate_write_stream_selector(close)?;
                    return if close {
                        Ok(ImmediateExtension::CloseOut { stream })
                    } else {
                        self.finish_immediate_write(stream, None)
                    };
                }
                PendingStructuredScannerPhase::Immediate(phase) => {
                    let mut child = child;
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::ImmediateChild,
                    )?;
                    return match phase {
                        PendingImmediatePhase::WriteText { stream } => {
                            self.finish_immediate_write(stream, None)
                        }
                        PendingImmediatePhase::WriteExpansion { stream, tokens } => {
                            self.finish_immediate_write(stream, Some(tokens))
                        }
                        PendingImmediatePhase::Pdf {
                            primitive,
                            pdf_output_enabled: retained,
                        } if retained == pdf_output_enabled => {
                            self.finish_immediate_pdf(primitive, pdf_output_enabled)
                        }
                        _ => Err(CommandError::input_invariant()),
                    };
                }
                phase => {
                    let mut pending = PendingStructuredScanner { phase, child };
                    if let Some(child) = pending.take_child() {
                        self.abort_continuation(child)?;
                    }
                }
            }
            return Err(CommandError::input_invariant());
        }
        let mut destination = None;
        let command = loop {
            if self.get_x_token_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            if !matches!(
                static_meaning(command.meaning()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                break command;
            }
        };
        match static_meaning(command.meaning()) {
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut)) => {
                self.finish_immediate_open_out(ImmediateOpenOutScalarPhase::Stream, None)
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write)) => {
                let stream = self.scan_immediate_write_stream_selector(false)?;
                self.finish_immediate_write(stream, None)
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut)) => {
                let stream = self.scan_immediate_write_stream_selector(true)?;
                Ok(ImmediateExtension::CloseOut { stream })
            }
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::PdfObject
                | UnexpandablePrimitive::PdfXForm
                | UnexpandablePrimitive::PdfXImage),
            )) => self.finish_immediate_pdf(primitive, pdf_output_enabled),
            _ => {
                self.back_input(command)?;
                Ok(ImmediateExtension::Continue)
            }
        }
    }

    fn scan_immediate_write_stream_selector(
        &mut self,
        close: bool,
    ) -> Result<WriteStreamSelector, CommandError> {
        let result = self.scan_integer_retained();
        let value = self
            .retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::ImmediateWriteStream { close },
            )?
            .value;
        Ok(if value < 0 {
            WriteStreamSelector::Negative
        } else if value > 15 {
            WriteStreamSelector::AboveRange
        } else {
            WriteStreamSelector::Stream(value as u8)
        })
    }

    fn finish_immediate_write(
        &mut self,
        stream: WriteStreamSelector,
        retained_tokens: Option<AttemptTokenListId>,
    ) -> Result<ImmediateExtension, CommandError> {
        // TeX82 §53 first saves write text without expansion, then
        // `write_out` replays it under an outer `\\endwrite` stopper.
        let tokens = if let Some(tokens) = retained_tokens {
            tokens
        } else {
            match self.scan_immediate_write_text() {
                Ok(tokens) => tokens,
                Err(error) => {
                    if error.is_resource_suspension() {
                        self.retain_structured_scanner(
                            PendingStructuredScannerPhase::Immediate(
                                PendingImmediatePhase::WriteText { stream },
                            ),
                            StructuredScannerChildDestination::ImmediateChild,
                        )?;
                    }
                    return Err(error);
                }
            }
        };
        let expanded = match self.expand_write_text(tokens) {
            Ok(expanded) => expanded,
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::Immediate(
                            PendingImmediatePhase::WriteExpansion { stream, tokens },
                        ),
                        StructuredScannerChildDestination::ImmediateChild,
                    )?;
                }
                return Err(error);
            }
        };
        Ok(ImmediateExtension::Write {
            stream,
            tokens: expanded.tokens,
        })
    }

    fn finish_immediate_pdf(
        &mut self,
        primitive: UnexpandablePrimitive,
        pdf_output_enabled: bool,
    ) -> Result<ImmediateExtension, CommandError> {
        if !pdf_output_enabled {
            return Ok(ImmediateExtension::PdfExtensionInDviMode(primitive));
        }
        let result = match primitive {
            UnexpandablePrimitive::PdfObject => self
                .scan_pdf_object_request()
                .map(ImmediateExtension::PdfObject),
            UnexpandablePrimitive::PdfXForm => self
                .scan_pdf_form_request(UnexpandablePrimitive::PdfXForm)
                .map(ImmediateExtension::PdfForm),
            UnexpandablePrimitive::PdfXImage => self
                .scan_pdf_image_request()
                .map(ImmediateExtension::PdfImage),
            _ => return Err(CommandError::input_invariant()),
        };
        match result {
            Err(error) if error.is_resource_suspension() => {
                self.retain_structured_scanner(
                    PendingStructuredScannerPhase::Immediate(PendingImmediatePhase::Pdf {
                        primitive,
                        pdf_output_enabled,
                    }),
                    StructuredScannerChildDestination::ImmediateChild,
                )?;
                Err(error)
            }
            result => result,
        }
    }

    fn finish_immediate_open_out(
        &mut self,
        phase: ImmediateOpenOutScalarPhase,
        mut child: Option<
            crate::execution_scratch::ChildContinuation<G, StructuredScannerChildDestination>,
        >,
    ) -> Result<ImmediateExtension, CommandError> {
        let stream = match phase {
            ImmediateOpenOutScalarPhase::Stream => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::FourBit);
                self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::ImmediateOpenOut(
                        ImmediateOpenOutScalarPhase::Stream,
                    ),
                )?
                .value as u8
            }
            ImmediateOpenOutScalarPhase::Equals { stream }
            | ImmediateOpenOutScalarPhase::FileName { stream } => stream,
        };
        if !matches!(phase, ImmediateOpenOutScalarPhase::FileName { .. }) {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::ImmediateOpenOut(
                    ImmediateOpenOutScalarPhase::Equals { stream },
                ),
            )?;
        }
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let result = self.scan_file_name_retained();
        let file_name = self.retain_structured_scalar(
            result,
            PendingStructuredScalarPhase::ImmediateOpenOut(ImmediateOpenOutScalarPhase::FileName {
                stream,
            }),
        )?;
        Ok(ImmediateExtension::OpenOut { stream, file_name })
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
        tokens: AttemptTokenListId,
    ) -> Result<ExpandedWriteText, CommandError> {
        self.write_expansion_depth = self
            .write_expansion_depth
            .checked_add(1)
            .ok_or_else(|| CommandError::input_invariant())?;
        let result = self.expand_write_text_inner(tokens);
        self.write_expansion_depth -= 1;
        result
    }

    /// Expands one generation-durable write payload without exposing the
    /// operation-local coordinate used by the write scanner.
    ///
    /// The durable words are copied into the current attempt before the
    /// artificial brace and frozen-`\endwrite` episode is installed. The
    /// attempt id remains entirely command-owned.
    pub fn expand_durable_write_text(
        &mut self,
        tokens: tex_state::TokenListId<G>,
    ) -> Result<ExpandedWriteText, CommandError> {
        let tokens = self.copy_durable_token_list_into_attempt(Some(tokens))?;
        self.expand_write_text(tokens)
    }

    fn expand_write_text_inner(
        &mut self,
        tokens: AttemptTokenListId,
    ) -> Result<ExpandedWriteText, CommandError> {
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
        let pending = self.take_pending_structured_scanner()?;
        let (stopper_level, write_words, mut child) =
            if let Some(PendingStructuredScanner { phase, child }) = pending {
                match phase {
                    PendingStructuredScannerPhase::WriteExpansion {
                        tokens: retained,
                        stopper_level,
                        write_words,
                    } if retained == tokens => (stopper_level, write_words, child),
                    phase => {
                        let mut pending = PendingStructuredScanner { phase, child };
                        if let Some(child) = pending.take_child() {
                            self.abort_continuation(child)?;
                        }
                        return Err(CommandError::input_invariant());
                    }
                }
            } else {
                let write_words = self
                    .command
                    .attempt
                    .arena()
                    .token_words(tokens)
                    .map_err(|_| CommandError::input_invariant())?
                    .len();
                // The bottom stopper delivers the synthetic closing brace followed
                // by frozen outer `\\endwrite`; the write list and opening brace sit
                // above it exactly as TeX82's three `ins_list` calls do.
                let stopper_level = self.push_write_recovery([right_brace, endwrite], right_brace);
                let write_level = self
                    .command
                    .push_attempt_list_level(
                        tokens,
                        u32::try_from(write_words).map_err(|_| CommandError::input_invariant())?,
                        TokenBehavior::Ordinary,
                        RetirementBehavior::Pop,
                        ReplayTrace::Stored(StoredReplayReason::Write),
                    )
                    .map_err(|_| CommandError::input_invariant())?;
                // TeX82 §§323 and 1370 trace the named write_text list at
                // begin_token_list, before the opening-brace insertion and expanded
                // scan_toks can report an error.
                if self
                    .state
                    .int_param(tex_state::env::banks::IntParam::TRACING_MACROS)
                    > 1
                {
                    let mut text = String::new();
                    crate::processor::expand::append_print_esc_text(self.state, "write", &mut text);
                    text.push_str("->");
                    let words = self
                        .command
                        .attempt
                        .arena()
                        .token_words(tokens)
                        .map_err(|_| CommandError::input_invariant())?;
                    for word in words {
                        crate::processor::expand::append_token_list_token_text(
                            self.state,
                            word.semantic_token(),
                            &mut text,
                        );
                    }
                    self.command.semantic_diagnostics.push(
                        crate::CommandSemanticDiagnostic::Trace {
                            text,
                            force_newline: false,
                        },
                    );
                }
                self.observe_write_list_push(write_level);
                self.push_write_recovery([left_brace], left_brace);
                (stopper_level, write_words, None)
            };

        self.outer_recovered_while_absorbing = false;
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::WriteExpansionText,
        )?;
        let expanded = match self.scan_balanced_text(true) {
            Ok(expanded) => expanded.tokens,
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::WriteExpansion {
                            tokens,
                            stopper_level,
                            write_words,
                        },
                        StructuredScannerChildDestination::WriteExpansionText,
                    )?;
                }
                return Err(error);
            }
        };
        let transient_words = self.command.transient_dynamic_words();
        let expanded_words = self
            .command
            .attempt
            .arena()
            .token_words(expanded)
            .map_err(|_| CommandError::input_invariant())?
            .len();
        // TeX82 §1370 keeps the source list, expanded result, live command
        // buffers, and three artificial tokens until the stopper is read.
        self.state.observe_transient_token_words(
            write_words
                .saturating_add(expanded_words)
                .saturating_add(transient_words)
                .saturating_add(4),
        );
        let mut destination = None;
        if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let mut stopper = destination.take().ok_or(CommandError::input_invariant())?;
        let unbalanced =
            self.outer_recovered_while_absorbing || stopper.spelling().semantic_token() != endwrite;
        self.outer_recovered_while_absorbing = false;
        // §1372 calls `error` before its recovery loop consumes through the
        // frozen stopper. Preserve that instant: the write and inserted-list
        // levels are gone by the time shipout can render the queued report.
        let error_context = unbalanced.then(|| self.command.output_open_context(self.state));
        while stopper.spelling().semantic_token() != endwrite {
            if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            stopper = destination.take().ok_or(CommandError::input_invariant())?;
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
    fn scan_immediate_write_text(&mut self) -> Result<AttemptTokenListId, CommandError> {
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
            PackedTokenSpanHandle::transient(
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
        let pending = self.take_pending_structured_scanner()?;
        let (phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase: PendingStructuredScannerPhase::Scalar(phase),
                child,
            }) => (phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (PendingStructuredScalarPhase::SetBoxIndex, None),
        };
        let index = match phase {
            PendingStructuredScalarPhase::SetBoxIndex => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_profile_register_index_retained();
                self.retain_structured_scalar(result, PendingStructuredScalarPhase::SetBoxIndex)?
            }
            PendingStructuredScalarPhase::SetBoxEquals { index } => index,
            _ => return Err(CommandError::input_invariant()),
        };
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let result = self.scan_optional_equals_retained();
        self.retain_structured_scalar(
            result,
            PendingStructuredScalarPhase::SetBoxEquals { index },
        )?;
        let path = if set_box_allowed {
            ScannedSetBoxPath::Payload(self.scan_box_payload()?)
        } else {
            ScannedSetBoxPath::Forbidden {
                error_context: self.command.output_open_context(self.state),
            }
        };
        Ok(ScannedSetBoxAssignment { index, path })
    }

    /// Scans the register operand of TeX82 §1079's `make_box(box_code)` and
    /// e-TeX 2.6 [47.1079]'s sparse-array replacement.
    pub fn scan_box_register(&mut self) -> Result<ScannedBoxRegister, CommandError> {
        self.restore_structured_unary(StructuredUnaryScalar::BoxRegister)?;
        let result = self.scan_profile_register_index_retained();
        Ok(ScannedBoxRegister {
            index: self.finish_structured_unary(result, StructuredUnaryScalar::BoxRegister)?,
        })
    }

    /// Scans TeX82 §1082's `\\vsplit <number> to <dimen>` prefix.
    ///
    /// e-TeX 2.6 [47.1082] widens the source box selector from
    /// `scan_eight_bit_int` to `scan_register_num`.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (mut phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase: PendingStructuredScannerPhase::Scalar(phase),
                child,
            }) => (phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (PendingStructuredScalarPhase::VSplitIndex, None),
        };
        let index = match phase {
            PendingStructuredScalarPhase::VSplitIndex => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_profile_register_index_retained();
                let index = self
                    .retain_structured_scalar(result, PendingStructuredScalarPhase::VSplitIndex)?;
                phase = PendingStructuredScalarPhase::VSplitTo { index };
                index
            }
            PendingStructuredScalarPhase::VSplitTo { index }
            | PendingStructuredScalarPhase::VSplitHeight { index, .. } => index,
            _ => {
                if let Some(child) = child.take() {
                    self.abort_continuation(child.restore().0)?;
                }
                return Err(CommandError::input_invariant());
            }
        };
        let missing_to_context = match phase {
            PendingStructuredScalarPhase::VSplitHeight {
                missing_to_context, ..
            } => missing_to_context,
            PendingStructuredScalarPhase::VSplitTo { .. } => {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("to");
                let found = self.retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::VSplitTo { index },
                )?;
                (!found.value).then(|| self.command.output_open_context(self.state))
            }
            _ => unreachable!("index phase advanced to to/height"),
        };
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let result = self.scan_dimension_retained();
        let height = self
            .retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::VSplitHeight {
                    index,
                    missing_to_context: missing_to_context.clone(),
                },
            )?
            .value;
        let split_context = self.command.output_open_context(self.state);
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
        state: &mut tex_state::CommandContext<'_, G>,
        command: &crate::CurrentCommand<G>,
    ) -> String {
        let text = selector_meaning_text(state, command);
        let breaks_after_colon = matches!(command.meaning(), ResolvedMeaning::Macro { .. })
            || matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::EndTemplate
                ))
            )
            || matches!(
                static_meaning(command.meaning()),
                Some(Meaning::ExpandablePrimitive(
                    tex_state::meaning::ExpandablePrimitive::TopMark
                        | tex_state::meaning::ExpandablePrimitive::FirstMark
                        | tex_state::meaning::ExpandablePrimitive::BotMark
                        | tex_state::meaning::ExpandablePrimitive::SplitFirstMark
                        | tex_state::meaning::ExpandablePrimitive::SplitBotMark
                ))
            );
        if breaks_after_colon {
            text.replacen(':', ":\n", 1)
        } else {
            text
        }
    }

    /// TeX82 §46's raw `\\show` operand scan.
    pub fn scan_show(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let mut destination = None;
        if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let token = command.spelling().semantic_token();
        let content = match token {
            Token::Cs(_)
            | Token::Char {
                cat: Catcode::Active,
                ..
            } => {
                let raw = string_text(self.state, token);
                let mut shown = String::new();
                self.state.append_selector_string_text(&raw, &mut shown);
                format!(
                    "> {shown}={}",
                    Self::shown_meaning_text(self.state, &command)
                )
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                format!("> {}", Self::shown_meaning_text(self.state, &command))
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
        self.restore_structured_unary(StructuredUnaryScalar::ShowThe)?;
        let result = self.scan_internal_value_or_zero_retained();
        let value = self.finish_structured_unary(result, StructuredUnaryScalar::ShowThe)?;
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
            InternalValue::Font(symbol) => print_cs_text(self.state, symbol),
            InternalValue::Tokens { tokens, .. } => {
                let mut text = String::new();
                let words = self
                    .command
                    .attempt_token_words(tokens)
                    .map_err(crate::scan_toks::attempt_command_error)?
                    .to_vec();
                for token in words {
                    self.state
                        .append_token_selector_text(token.semantic_token(), &mut text);
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
        self.restore_structured_unary(StructuredUnaryScalar::ShowBox)?;
        let result = self.scan_restricted_integer_retained(class);
        let index = self.finish_structured_unary(result, StructuredUnaryScalar::ShowBox)?;
        Ok((
            u16::try_from(index.value).expect("recovered register number is in range"),
            StructuredProvenance {
                primary: index.provenance.primary,
            },
        ))
    }

    /// Scans the payload prefix of TeX82 §1090's leader commands.
    pub fn scan_leader_payload(&mut self) -> Result<ScannedLeaderPayload, CommandError> {
        if let Some(pending) = self.take_pending_structured_scanner()? {
            let PendingStructuredScanner { phase, mut child } = pending;
            let PendingStructuredScannerPhase::Scalar(
                PendingStructuredScalarPhase::LeaderRegister { copy },
            ) = phase
            else {
                if let Some(child) = child.take() {
                    self.abort_continuation(child.restore().0)?;
                }
                return Err(CommandError::input_invariant());
            };
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_eight_bit_register_index_retained();
            let index = self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::LeaderRegister { copy },
            )?;
            return Ok(ScannedLeaderPayload::BoxRegister { index, copy });
        }
        let mut destination = None;
        match self.get_x_token_into(&mut destination)? {
            DeliveryStatus::End => return Ok(ScannedLeaderPayload::Missing),
            DeliveryStatus::Command => {}
            _ => return Err(CommandError::input_invariant()),
        };
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        match static_meaning(command.meaning()) {
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)) => {
                let result = self.scan_eight_bit_register_index_retained();
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.retain_structured_scalar(
                        result,
                        PendingStructuredScalarPhase::LeaderRegister { copy: false },
                    )?,
                    copy: false,
                })
            }
            Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy)) => {
                let result = self.scan_eight_bit_register_index_retained();
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.retain_structured_scalar(
                        result,
                        PendingStructuredScalarPhase::LeaderRegister { copy: true },
                    )?,
                    copy: true,
                })
            }
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HBox
                | UnexpandablePrimitive::VBox
                | UnexpandablePrimitive::VTop),
            )) => Ok(ScannedLeaderPayload::Construction(
                self.scan_box_construction(primitive)?,
            )),
            Some(Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
            )) => Ok(ScannedLeaderPayload::Rule(self.scan_rule_spec(primitive)?)),
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
        let pending = self.take_pending_structured_scanner()?;
        let (value_phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::GlueParameterEquals {
                            index: retained_index,
                            mu: retained_mu,
                        },
                    ),
                child,
            }) if retained_index == index && retained_mu == mu => (false, child),
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::GlueParameterValue {
                            index: retained_index,
                            mu: retained_mu,
                        },
                    ),
                child,
            }) if retained_index == index && retained_mu == mu => (true, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (false, None),
        };
        if !value_phase {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_optional_equals_retained();
            self.retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::GlueParameterEquals { index, mu },
            )?;
        }
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let result = self.scan_glue_retained(mu);
        let value = self
            .retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::GlueParameterValue { index, mu },
            )?
            .value;
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
        let pending = self.take_pending_structured_scanner()?;
        let (mut width, mut height, mut depth, mut phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::Rule {
                        primitive: retained_primitive,
                        width,
                        height,
                        depth,
                        phase,
                    }),
                child,
            }) if retained_primitive == primitive => (width, height, depth, phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None if primitive == UnexpandablePrimitive::VRule => (
                Some(default_rule),
                None,
                None,
                RuleScalarPhase::WidthKeyword,
                None,
            ),
            None => (
                None,
                Some(default_rule),
                Some(Scaled::from_raw(0)),
                RuleScalarPhase::WidthKeyword,
                None,
            ),
        };
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let retained_phase = |phase| PendingStructuredScalarPhase::Rule {
                primitive,
                width,
                height,
                depth,
                phase,
            };
            match phase {
                RuleScalarPhase::WidthKeyword => {
                    let result = self.scan_keyword_retained("width");
                    if self
                        .retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::WidthKeyword),
                        )?
                        .value
                    {
                        phase = RuleScalarPhase::WidthDimension;
                    } else {
                        phase = RuleScalarPhase::HeightKeyword;
                    }
                }
                RuleScalarPhase::WidthDimension => {
                    let result = self.scan_dimension_retained();
                    width = Some(
                        self.retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::WidthDimension),
                        )?
                        .value,
                    );
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::HeightKeyword => {
                    let result = self.scan_keyword_retained("height");
                    if self
                        .retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::HeightKeyword),
                        )?
                        .value
                    {
                        phase = RuleScalarPhase::HeightDimension;
                    } else {
                        phase = RuleScalarPhase::DepthKeyword;
                    }
                }
                RuleScalarPhase::HeightDimension => {
                    let result = self.scan_dimension_retained();
                    height = Some(
                        self.retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::HeightDimension),
                        )?
                        .value,
                    );
                    phase = RuleScalarPhase::WidthKeyword;
                }
                RuleScalarPhase::DepthKeyword => {
                    let result = self.scan_keyword_retained("depth");
                    if self
                        .retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::DepthKeyword),
                        )?
                        .value
                    {
                        phase = RuleScalarPhase::DepthDimension;
                    } else {
                        break;
                    }
                }
                RuleScalarPhase::DepthDimension => {
                    let result = self.scan_dimension_retained();
                    depth = Some(
                        self.retain_structured_scalar(
                            result,
                            retained_phase(RuleScalarPhase::DepthDimension),
                        )?
                        .value,
                    );
                    phase = RuleScalarPhase::WidthKeyword;
                }
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
    fn scan_spec_packing(
        &mut self,
        owner: PackingOwner,
    ) -> Result<ScannedPackingSpec, CommandError> {
        let pending = self.take_pending_structured_scanner()?;
        let (mut phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(PendingStructuredScalarPhase::Packing {
                        owner: retained_owner,
                        phase,
                    }),
                child,
            }) if retained_owner == owner => (phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (PackingScalarPhase::ToKeyword, None),
        };
        loop {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            match phase {
                PackingScalarPhase::ToKeyword => {
                    let result = self.scan_keyword_retained("to");
                    if self
                        .retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::Packing { owner, phase },
                        )?
                        .value
                    {
                        phase = PackingScalarPhase::Dimension { exactly: true };
                    } else {
                        phase = PackingScalarPhase::SpreadKeyword;
                    }
                }
                PackingScalarPhase::SpreadKeyword => {
                    let result = self.scan_keyword_retained("spread");
                    if self
                        .retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::Packing { owner, phase },
                        )?
                        .value
                    {
                        phase = PackingScalarPhase::Dimension { exactly: false };
                    } else {
                        return Ok(ScannedPackingSpec::Natural);
                    }
                }
                PackingScalarPhase::Dimension { exactly } => {
                    let result = self.scan_dimension_retained();
                    let value = self
                        .retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::Packing { owner, phase },
                        )?
                        .value;
                    return Ok(if exactly {
                        ScannedPackingSpec::Exactly(value)
                    } else {
                        ScannedPackingSpec::Spread(value)
                    });
                }
            }
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
        let packing = self.scan_spec_packing(PackingOwner::Box(primitive))?;
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
        let pending = self.take_pending_structured_scanner()?;
        let (phase, mut child) = match pending {
            Some(PendingStructuredScanner {
                phase: PendingStructuredScannerPhase::Scalar(phase),
                child,
            }) => (phase, child),
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => (PendingStructuredScalarPhase::InsertPre, None),
        };
        let pre = match phase {
            PendingStructuredScalarPhase::InsertPre
                if is_vadjust && self.command.profile().capabilities().supports_pdftex() =>
            {
                self.restore_structured_scanner_child(
                    &mut child,
                    StructuredScannerChildDestination::Scalar,
                )?;
                let result = self.scan_keyword_retained("pre");
                self.retain_structured_scalar(result, PendingStructuredScalarPhase::InsertPre)?
                    .value
            }
            PendingStructuredScalarPhase::InsertPre => false,
            PendingStructuredScalarPhase::InsertClass { pre } => pre,
            _ => {
                if let Some(child) = child.take() {
                    self.abort_continuation(child.restore().0)?;
                }
                return Err(CommandError::input_invariant());
            }
        };
        let (class, reserved_class_context) = if is_vadjust {
            (255, None)
        } else {
            self.restore_structured_scanner_child(
                &mut child,
                StructuredScannerChildDestination::Scalar,
            )?;
            let result = self.scan_restricted_integer_retained(RestrictedIntegerClass::EightBit);
            let class = self
                .retain_structured_scalar(
                    result,
                    PendingStructuredScalarPhase::InsertClass { pre },
                )?
                .value;
            let context = (class == 255).then(|| self.command.output_open_context(self.state));
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
        let pending = self.take_pending_structured_scanner()?;
        let mut child = match pending {
            Some(PendingStructuredScanner {
                phase:
                    PendingStructuredScannerPhase::Scalar(
                        PendingStructuredScalarPhase::BoxShiftDimension {
                            primitive: retained_primitive,
                        },
                    ),
                child,
            }) if retained_primitive == primitive => child,
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => None,
        };
        self.restore_structured_scanner_child(
            &mut child,
            StructuredScannerChildDestination::Scalar,
        )?;
        let result = self.scan_dimension_retained();
        let amount = self
            .retain_structured_scalar(
                result,
                PendingStructuredScalarPhase::BoxShiftDimension { primitive },
            )?
            .value;
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
        let mut destination = None;
        loop {
            match self.get_x_token_into(&mut destination)? {
                DeliveryStatus::End => return Ok(ScannedBoxShiftPayload::Missing),
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            };
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
                | Some(Meaning::Relax) => continue,
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box)) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: false,
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy)) => {
                    return Ok(ScannedBoxShiftPayload::BoxRegister {
                        index: self.scan_box_register()?.index,
                        copy: true,
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox)) => {
                    return Ok(ScannedBoxShiftPayload::LastBox {
                        error_context: self.error_context(),
                    });
                }
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit)) => {
                    return Ok(ScannedBoxShiftPayload::VSplit(self.scan_vsplit()?));
                }
                Some(Meaning::UnexpandablePrimitive(
                    primitive @ (UnexpandablePrimitive::HBox
                    | UnexpandablePrimitive::VBox
                    | UnexpandablePrimitive::VTop),
                )) => {
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
        let packing = self.scan_spec_packing(PackingOwner::Alignment)?;
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
        let mut destination = None;
        loop {
            if self.get_x_token_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let opening = destination.take().ok_or(CommandError::input_invariant())?;
            match static_meaning(opening.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }) => continue,
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)) => {
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
                static_meaning(lookahead.command().meaning()),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit))
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
        let pending = match self.scanner_resume.take() {
            Some(key) if key.is_alignment_preamble() => Some(
                self.command
                    .scratch
                    .take_alignment_preamble_frame(key)
                    .map_err(crate::scan_toks::scratch_command_error)?,
            ),
            Some(key) => {
                self.scanner_resume = Some(key);
                return Err(CommandError::input_invariant());
            }
            None => None,
        };
        let mut pending = if let Some(pending) = pending {
            pending
        } else {
            // TeX82 §776's preamble scan begins with the opener already
            // consumed. It owns both template sinks before its first token
            // demand, so a nested expansion can suspend without moving either
            // result out of the attempt arena.
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
            let live_tokens = self
                .command
                .attempt
                .arena_mut()
                .allocate_token_buffer()
                .map_err(|_| CommandError::input_invariant())?;
            self.command
                .transient
                .builders
                .push(crate::state::LiveTokenBuilder {
                    identity: builder.0,
                    tokens: live_tokens,
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
            let current_tabskip = self
                .state
                .glue_param(GlueParam::TAB_SKIP)
                .map_or_else(|| GlueSpec::ZERO, |id| self.state.glue(id));
            PendingAlignmentPreamble {
                alignment,
                builder,
                scanner_episode,
                columns: Vec::new(),
                tabskips: vec![current_tabskip],
                current_tabskip,
                repeat_start: None,
                u_template: self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(|_| CommandError::input_invariant())?,
                v_template: self
                    .command
                    .attempt
                    .arena_mut()
                    .allocate_token_buffer()
                    .map_err(|_| CommandError::input_invariant())?,
                phase: AlignmentPreamblePhase::UTemplate,
                span_expansion: None,
                scalar_scan: None,
            }
        };
        loop {
            if let Some(mut scalar) = pending.scalar_scan.take() {
                if let Some(child) = scalar.child.take() {
                    let (key, destination) = child.restore();
                    if destination != AlignmentPreambleChildDestination::Scalar {
                        self.abort_continuation(key)?;
                        self.abort_alignment_preamble(pending)?;
                        return Err(CommandError::input_invariant());
                    }
                    self.install_scanner_resume(Some(key));
                }
                match scalar.phase {
                    AlignmentPreambleScalarPhase::TabskipEquals => {
                        match self.scan_optional_equals_retained() {
                            crate::RetainedScalarScan::Complete(_) => {
                                pending.scalar_scan = Some(PendingPreambleScalar {
                                    phase: AlignmentPreambleScalarPhase::TabskipGlue,
                                    child: None,
                                });
                                continue;
                            }
                            crate::RetainedScalarScan::Failed(error) => {
                                self.abort_alignment_preamble(pending)?;
                                return Err(error);
                            }
                            crate::RetainedScalarScan::Suspended { error, child } => {
                                pending.scalar_scan = Some(PendingPreambleScalar {
                                    phase: AlignmentPreambleScalarPhase::TabskipEquals,
                                    child: None,
                                });
                                return self.retain_alignment_scalar(pending, child, error);
                            }
                        }
                    }
                    AlignmentPreambleScalarPhase::TabskipGlue => {
                        match self.scan_glue_retained(false) {
                            crate::RetainedScalarScan::Complete(value) => {
                                pending.current_tabskip = value.value;
                                let global = self.state.int_param(IntParam::GLOBAL_DEFS) > 0;
                                self.state
                                    .define_preamble_tabskip(pending.current_tabskip, global);
                                continue;
                            }
                            crate::RetainedScalarScan::Failed(error) => {
                                self.abort_alignment_preamble(pending)?;
                                return Err(error);
                            }
                            crate::RetainedScalarScan::Suspended { error, child } => {
                                pending.scalar_scan = Some(PendingPreambleScalar {
                                    phase: AlignmentPreambleScalarPhase::TabskipGlue,
                                    child: None,
                                });
                                return self.retain_alignment_scalar(pending, child, error);
                            }
                        }
                    }
                }
            }
            let mut destination = None;
            let command =
                match self.get_preamble_token(&mut pending.span_expansion, &mut destination) {
                    Ok(DeliveryStatus::Command) => {
                        destination.take().ok_or(CommandError::input_invariant())?
                    }
                    Ok(DeliveryStatus::End) => {
                        self.abort_alignment_preamble(pending)?;
                        return Err(CommandError::input_invariant());
                    }
                    Ok(_) => {
                        self.abort_alignment_preamble(pending)?;
                        return Err(CommandError::input_invariant());
                    }
                    Err(error) if error.is_resource_suspension() => {
                        let key = self
                            .command
                            .scratch
                            .store_alignment_preamble_frame(pending)
                            .map_err(crate::scan_toks::scratch_command_error)?;
                        if self.scanner_resume.replace(key).is_some() {
                            return Err(CommandError::input_invariant());
                        }
                        return Err(error);
                    }
                    Err(error) => {
                        self.abort_alignment_preamble(pending)?;
                        return Err(error);
                    }
                };
            if matches!(
                static_meaning(command.meaning()),
                Some(Meaning::GlueParam(index)) if index == GlueParam::TAB_SKIP.raw()
            ) {
                pending.scalar_scan = Some(PendingPreambleScalar {
                    phase: AlignmentPreambleScalarPhase::TabskipEquals,
                    child: None,
                });
                continue;
            }

            match pending.phase {
                AlignmentPreamblePhase::UTemplate => {
                    if is_character_command(&command, Catcode::Parameter) {
                        pending.phase = AlignmentPreamblePhase::VTemplate;
                        continue;
                    }
                    let tab = is_character_command(&command, Catcode::AlignmentTab);
                    let terminator = tab
                        || matches!(
                            static_meaning(command.meaning()),
                            Some(Meaning::UnexpandablePrimitive(
                                UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                            ))
                        );
                    if terminator && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE {
                        if tab
                            && self
                                .command
                                .attempt
                                .arena()
                                .token_buffer(pending.u_template)
                                .map_err(|_| CommandError::input_invariant())?
                                .is_empty()
                            && pending.repeat_start.is_none()
                        {
                            pending.repeat_start = Some(pending.columns.len());
                            continue;
                        }
                        observe!(
                            self,
                            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                                transition: "missing_parameter",
                                alignment: Some(pending.alignment.raw()),
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
                        pending.phase = AlignmentPreamblePhase::VTemplate;
                        continue;
                    }
                    if !matches!(
                        static_meaning(command.meaning()),
                        Some(Meaning::CharToken {
                            cat: Catcode::Space,
                            ..
                        })
                    ) || !self
                        .command
                        .attempt
                        .arena()
                        .token_buffer(pending.u_template)
                        .map_err(|_| CommandError::input_invariant())?
                        .is_empty()
                    {
                        self.command
                            .attempt
                            .arena_mut()
                            .push_buffer_token(pending.u_template, command.spelling())
                            .map_err(|_| CommandError::input_invariant())?;
                        self.push_alignment_live_token(pending.builder, command.spelling())?;
                    }
                }
                AlignmentPreamblePhase::VTemplate => {
                    let ends_column = is_character_command(&command, Catcode::AlignmentTab);
                    let ends_preamble = matches!(
                        static_meaning(command.meaning()),
                        Some(Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                        ))
                    );
                    if (ends_column || ends_preamble)
                        && self.command.alignment.align_state == PREAMBLE_ALIGN_STATE
                    {
                        let u_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .finish_token_buffer(pending.u_template)
                            .map_err(|_| CommandError::input_invariant())?;
                        let v_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .finish_token_buffer(pending.v_template)
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.columns.push(AlignmentCellTemplates {
                            u_template: Some(u_template),
                            v_template,
                        });
                        pending.tabskips.push(pending.current_tabskip);
                        if ends_preamble {
                            break;
                        }
                        pending.u_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .allocate_token_buffer()
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.v_template = self
                            .command
                            .attempt
                            .arena_mut()
                            .allocate_token_buffer()
                            .map_err(|_| CommandError::input_invariant())?;
                        pending.phase = AlignmentPreamblePhase::UTemplate;
                        continue;
                    }
                    if is_character_command(&command, Catcode::Parameter) {
                        observe!(
                            self,
                            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                                transition: "extra_parameter",
                                alignment: Some(pending.alignment.raw()),
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
                    self.command
                        .attempt
                        .arena_mut()
                        .push_buffer_token(pending.v_template, command.spelling())
                        .map_err(|_| CommandError::input_invariant())?;
                    self.push_alignment_live_token(pending.builder, command.spelling())?;
                }
            }
        }
        self.command
            .alignment
            .complete_preamble(
                pending.alignment,
                AlignmentPreamble {
                    columns: pending.columns,
                    tabskips: pending.tabskips,
                    default_tabskip: pending.current_tabskip,
                    repeat_start: pending.repeat_start,
                },
            )
            .map_err(|_| CommandError::input_invariant())?;
        observe!(
            self,
            crate::CommandObservation::Alignment(crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(pending.alignment.raw()),
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
        self.finish_scanner_episode(pending.scanner_episode);
        self.command
            .transient
            .builders
            .retain(|live| live.identity != pending.builder.0);
        Ok(())
    }

    /// TeX82 §759's `get_preamble_token`.
    ///
    /// A `\span` is not template material: it fetches the following token,
    /// expands that token exactly once when expandable, and repeats if the
    /// resulting raw token is another `\span`. Ordinary template tokens stay
    /// raw so their meanings are resolved when each cell is executed.
    fn get_preamble_token(
        &mut self,
        pending: &mut Option<PendingPreambleSpanExpansion<G>>,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        let delivery = if let Some(mut resumed) = pending.take() {
            self.resume_current_command(&resumed.command);
            if let Some(child) = resumed.child.take() {
                let (key, destination) = child.restore();
                if destination != AlignmentPreambleChildDestination::SpanExpansion {
                    return Err(CommandError::input_invariant());
                }
                self.scanner_resume = Some(key);
            }
            if let Err(error) = self.expand(&resumed.command) {
                if error.is_resource_suspension() {
                    *pending = Some(PendingPreambleSpanExpansion {
                        command: resumed.command,
                        child: crate::execution_scratch::ChildContinuation::capture(
                            &mut self.scanner_resume,
                            AlignmentPreambleChildDestination::SpanExpansion,
                        ),
                    });
                }
                return Err(error);
            }
            if self.scanner_resume.is_some() {
                return Err(CommandError::input_invariant());
            }
            self.get_token_into(destination)?
        } else {
            self.get_token_into(destination)?
        };
        if delivery == DeliveryStatus::End {
            return Ok(DeliveryStatus::End);
        }
        if delivery != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        while destination.as_ref().is_some_and(|command| {
            matches!(
                static_meaning(command.meaning()),
                Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Span))
            )
        }) {
            destination.take();
            match self.get_token_into(destination)? {
                DeliveryStatus::End => return Ok(DeliveryStatus::End),
                DeliveryStatus::Command => {}
                _ => return Err(CommandError::input_invariant()),
            }
            let next = destination.take().ok_or(CommandError::input_invariant())?;
            if crate::processor::expand::is_expandable_command(&next) {
                if let Err(error) = self.expand(&next) {
                    if error.is_resource_suspension() {
                        *pending = Some(PendingPreambleSpanExpansion {
                            command: next,
                            child: crate::execution_scratch::ChildContinuation::capture(
                                &mut self.scanner_resume,
                                AlignmentPreambleChildDestination::SpanExpansion,
                            ),
                        });
                    }
                    return Err(error);
                }
                if self.scanner_resume.is_some() {
                    return Err(CommandError::input_invariant());
                }
                match self.get_token_into(destination)? {
                    DeliveryStatus::End => return Ok(DeliveryStatus::End),
                    DeliveryStatus::Command => {}
                    _ => return Err(CommandError::input_invariant()),
                }
            } else {
                *destination = Some(next);
            }
        }
        if destination.as_ref().is_some_and(|command| {
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
                PackedTokenSpanHandle::transient([
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
        Ok(DeliveryStatus::Command)
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

    pub fn scan_balanced_text_retained(
        &mut self,
        expanded: bool,
    ) -> crate::RetainedScalarScan<G, ScannedBalancedText> {
        let result = self.scan_balanced_text(expanded);
        self.detach_retained_scalar(result)
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
        let source = self
            .command
            .attempt
            .arena()
            .token_words(scanned)
            .map_err(|_| CommandError::input_invariant())?
            .to_vec();
        let mut shifted = Vec::with_capacity(source.len());
        for word in source {
            // Copy one immutable word at a time so the source interned lists
            // remain in place while the case-code lookup records its mutable
            // dependency read. Only the rewritten backup list needs storage.
            let token = word.semantic_token();
            let origin = word.origin();
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
        // The backed-up level is a parent-owned input chunk, not a coordinate
        // into the scanner child. Commit may therefore reclaim the scanner
        // scope without leaving an attempt id in accepted command roots.
        let level = self.command.push_token_level(
            PackedTokenSpanHandle::transient(shifted),
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
        if let Some(pending) = self.take_pending_structured_scanner()? {
            let PendingStructuredScanner { phase, mut child } = pending;
            return match phase {
                PendingStructuredScannerPhase::Scalar(
                    PendingStructuredScalarPhase::SpecialKeyword,
                ) => {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::Scalar,
                    )?;
                    let result = self.scan_keyword_retained("shipout");
                    let deferred = self
                        .retain_structured_scalar(
                            result,
                            PendingStructuredScalarPhase::SpecialKeyword,
                        )?
                        .value;
                    match self.scan_balanced_text(!deferred) {
                        Ok(text) => Ok((deferred, text)),
                        Err(error) => {
                            if error.is_resource_suspension() {
                                self.retain_structured_scanner(
                                    PendingStructuredScannerPhase::SpecialText { deferred },
                                    StructuredScannerChildDestination::SpecialText,
                                )?;
                            }
                            Err(error)
                        }
                    }
                }
                PendingStructuredScannerPhase::SpecialText { deferred } => {
                    self.restore_structured_scanner_child(
                        &mut child,
                        StructuredScannerChildDestination::SpecialText,
                    )?;
                    match self.scan_balanced_text(!deferred) {
                        Ok(text) => Ok((deferred, text)),
                        Err(error) => {
                            if error.is_resource_suspension() {
                                self.retain_structured_scanner(
                                    PendingStructuredScannerPhase::SpecialText { deferred },
                                    StructuredScannerChildDestination::SpecialText,
                                )?;
                            }
                            Err(error)
                        }
                    }
                }
                _ => {
                    if let Some(child) = child.take() {
                        self.abort_continuation(child.restore().0)?;
                    }
                    Err(CommandError::input_invariant())
                }
            };
        }
        // TeX82 §473 enters `scan_toks` immediately. The preceding optional
        // keyword probe belongs only to pdfTeX 1.40.29 §1534; in particular,
        // an e-TeX job must enter `absorbing` before delivering the opening
        // brace instead of speculatively backing it up and replaying it.
        let deferred = if self.profile().capabilities().supports_pdftex() {
            let result = self.scan_keyword_retained("shipout");
            self.retain_structured_scalar(result, PendingStructuredScalarPhase::SpecialKeyword)?
                .value
        } else {
            false
        };
        match self.scan_balanced_text(!deferred) {
            Ok(text) => Ok((deferred, text)),
            Err(error) => {
                if error.is_resource_suspension() {
                    self.retain_structured_scanner(
                        PendingStructuredScannerPhase::SpecialText { deferred },
                        StructuredScannerChildDestination::SpecialText,
                    )?;
                }
                Err(error)
            }
        }
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedMacroDefinition, CommandError> {
        let target = if let Some(target) = self
            .pending_scanner_frame()
            .map_err(|_| CommandError::input_invariant())?
            .and_then(|pending| pending.macro_definition_target(expanded))
        {
            target
        } else {
            let mut destination = None;
            if self.next_non_space_raw_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let command = destination.take().ok_or(CommandError::input_invariant())?;
            if let Some(target) = self.delivered_definition_target(&command) {
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
        let definition = self
            .command
            .attempt
            .arena_mut()
            .allocate_definition(scanned.parameter_text, scanned.replacement_text)
            .map_err(|_| CommandError::input_invariant())?;
        Ok(ScannedMacroDefinition {
            target,
            definition,
            parameter_text: scanned.parameter_text,
            replacement_text: scanned.replacement_text,
            provenance,
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
    ) -> Result<ScannedLetAssignment<G>, CommandError> {
        let mut destination = None;
        if self.next_non_space_raw_into(&mut destination)? != DeliveryStatus::Command {
            return Err(CommandError::input_invariant());
        }
        let command = destination.take().ok_or(CommandError::input_invariant())?;
        let target = self
            .delivered_definition_target(&command)
            .ok_or(CommandError::input_invariant())?;
        let (source, meaning) = if future {
            let mut first_destination = None;
            if self.get_token_into(&mut first_destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let mut second_destination = None;
            if self.get_token_into(&mut second_destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let second = second_destination
                .take()
                .ok_or(CommandError::input_invariant())?;
            let source = second.control_sequence();
            let meaning = second.meaning();
            self.back_input(second)?;
            let first = first_destination
                .take()
                .ok_or(CommandError::input_invariant())?;
            self.back_input_saved(first)?;
            (source, meaning)
        } else {
            if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            let mut source = destination.take().ok_or(CommandError::input_invariant())?;
            if matches!(
                static_meaning(source.meaning()),
                Some(Meaning::CharToken { ch: '=', .. })
            ) {
                if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                    return Err(CommandError::input_invariant());
                }
                source = destination.take().ok_or(CommandError::input_invariant())?;
                if matches!(
                    static_meaning(source.meaning()),
                    Some(Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    })
                ) {
                    if self.get_token_into(&mut destination)? != DeliveryStatus::Command {
                        return Err(CommandError::input_invariant());
                    }
                    source = destination.take().ok_or(CommandError::input_invariant())?;
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
    fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        self.command.begin_file_name()?;
        let result = self.scan_file_name_inner();
        self.command.end_file_name();
        result
    }

    pub fn scan_file_name_retained(&mut self) -> crate::RetainedScalarScan<G, ScannedFileName> {
        let result = self.scan_file_name();
        self.detach_retained_scalar(result)
    }

    fn scan_file_name_inner(&mut self) -> Result<ScannedFileName, CommandError> {
        let pending = self.take_pending_scalar_frame()?;
        let mut suspended = None;
        let result = match pending {
            Some(crate::scanners::PendingScalarFrame::FileNameLeading { mut child }) => {
                self.restore_scalar_child(
                    &mut child,
                    crate::scanners::ScalarChildDestination::FileNameLeadingToken,
                )?;
                self.scan_file_name_leading(&mut suspended)
            }
            Some(crate::scanners::PendingScalarFrame::FileNameCharacters {
                components,
                character_count,
                quoted,
                grouped,
                provenance,
                mut child,
            }) => {
                self.restore_scalar_child(
                    &mut child,
                    crate::scanners::ScalarChildDestination::FileNameCharacter,
                )?;
                self.scan_file_name_characters(
                    components,
                    character_count,
                    quoted,
                    grouped,
                    provenance,
                    &mut suspended,
                )
            }
            Some(mut pending) => {
                if let Some(child) = pending.take_child() {
                    self.abort_continuation(child)?;
                }
                return Err(CommandError::input_invariant());
            }
            None => self.scan_file_name_leading(&mut suspended),
        };
        self.finish_scalar_call(result, suspended)
    }

    fn scan_file_name_leading(
        &mut self,
        suspended: &mut Option<crate::scanners::PendingScalarFrame<G>>,
    ) -> Result<ScannedFileName, CommandError> {
        let mut destination = None;
        let first = loop {
            let command = match self.get_x_token_into(&mut destination) {
                Ok(DeliveryStatus::Command) => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                Ok(DeliveryStatus::End) | Ok(_) => {
                    return Err(CommandError::input_invariant());
                }
                Err(error) => {
                    *suspended =
                        Some(crate::scanners::PendingScalarFrame::FileNameLeading { child: None });
                    return Err(error);
                }
            };
            if !matches!(
                static_meaning(command.meaning()),
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                })
            ) {
                break command;
            }
        };
        let provenance = StructuredProvenance {
            primary: first.origin(),
        };
        let grouped = matches!(
            static_meaning(first.meaning()),
            Some(Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            })
        );
        // `scan_file_name` replays its first non-space token before consuming
        // the filename. TeX82 exposes this `back_input` hand-off, and it
        // keeps the group-opening case on the same ordinary delivery path.
        self.back_input(first)?;
        self.scan_file_name_characters(
            FileNameComponents::default(),
            0,
            false,
            grouped,
            provenance.primary,
            suspended,
        )
    }

    fn scan_file_name_characters(
        &mut self,
        mut components: FileNameComponents,
        mut character_count: usize,
        mut quoted: bool,
        grouped: bool,
        provenance: OriginId,
        suspended: &mut Option<crate::scanners::PendingScalarFrame<G>>,
    ) -> Result<ScannedFileName, CommandError> {
        let mut destination = None;
        loop {
            let command = match self.get_x_token_into(&mut destination) {
                Ok(DeliveryStatus::Command) => {
                    destination.take().ok_or(CommandError::input_invariant())?
                }
                Ok(DeliveryStatus::End) => break,
                Ok(_) => return Err(CommandError::input_invariant()),
                Err(error) => {
                    *suspended = Some(crate::scanners::PendingScalarFrame::FileNameCharacters {
                        components,
                        character_count,
                        quoted,
                        grouped,
                        provenance,
                        child: None,
                    });
                    return Err(error);
                }
            };
            match static_meaning(command.meaning()) {
                Some(Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                }) if grouped => {}
                Some(Meaning::CharToken { ch: '"', .. }) => quoted = !quoted,
                Some(Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                }) if grouped && !quoted => {
                    break;
                }
                Some(Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }) if !grouped && !quoted => {
                    break;
                }
                Some(Meaning::CharToken { ch, .. }) => {
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
            provenance: StructuredProvenance {
                primary: provenance,
            },
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
                // e-TeX 2.6 [23.328]'s `grp_stack[in_open]:=cur_boundary;
                // if_stack[in_open]:=cond_ptr`, recorded for `\tracingnesting`'s
                // `file_warning` at this level's eventual `end_file_reading`.
                let open_depths = self.capture_source_open_depths();
                let (_, framing_name) = self
                    .command
                    .open_registered_file_with_depths(source, open_depths)
                    .map_err(|_| CommandError::input_invariant())?;
                if let Some(name) = framing_name {
                    self.state.print_file_open(&name);
                }
                self.prepare_started_input()?;
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
        let context = self.command.output_open_context(self.state);
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
    fn next_non_space_raw_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let delivery = self.get_token_into(destination)?;
            if delivery == DeliveryStatus::End {
                return Ok(DeliveryStatus::End);
            }
            if delivery != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            if !matches!(
                destination
                    .as_ref()
                    .ok_or(CommandError::input_invariant())?
                    .spelling()
                    .semantic_token(),
                Token::Char {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
        }
    }

    /// TeX82 §404's expanded nonblank/non-relax fetch, delivered directly
    /// into the structured scanner operation that will classify or hand off
    /// the command.
    fn next_non_blank_non_relax_x_token_into(
        &mut self,
        destination: &mut Option<CurrentCommand<G>>,
    ) -> Result<DeliveryStatus, CommandError> {
        loop {
            let delivery = self.get_x_token_into(destination)?;
            if delivery == DeliveryStatus::End {
                return Ok(DeliveryStatus::End);
            }
            if delivery != DeliveryStatus::Command {
                return Err(CommandError::input_invariant());
            }
            if !matches!(
                static_meaning(
                    destination
                        .as_ref()
                        .ok_or(CommandError::input_invariant())?
                        .meaning()
                ),
                Some(
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    } | Meaning::Relax
                )
            ) {
                return Ok(DeliveryStatus::Command);
            }
            destination.take();
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

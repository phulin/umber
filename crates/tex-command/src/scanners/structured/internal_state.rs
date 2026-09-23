use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlignmentPreamblePhase {
    UTemplate,
    VTemplate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlignmentPreambleScalarPhase {
    TabskipEquals,
    TabskipGlue,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct AlignmentPreambleScalar<G> {
    pub(super) phase: AlignmentPreambleScalarPhase,
    pub(super) _generation: PhantomData<fn(&G) -> &G>,
}

/// Exact in-process owner of an alignment preamble suspended while expanding
/// the token following `\span`.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AlignmentPreambleState<G> {
    pub(super) alignment: AlignmentIdentity,
    pub(super) builder: TokenBuilderId,
    pub(super) scanner_episode: ScannerEpisode,
    pub(super) columns: Vec<AlignmentCellTemplates>,
    pub(super) tabskips: Vec<GlueSpec>,
    pub(super) current_tabskip: GlueSpec,
    pub(super) repeat_start: Option<usize>,
    pub(super) u_template: AttemptTokenBufferId,
    pub(super) v_template: AttemptTokenBufferId,
    pub(super) phase: AlignmentPreamblePhase,
    pub(super) scalar_scan: Option<AlignmentPreambleScalar<G>>,
    pub(super) _generation: PhantomData<fn(&G) -> &G>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PendingPdfActionOwner {
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
pub(super) enum PendingPdfColorStackAction {
    Set,
    Push,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PdfNavigationScalarProgress {
    pub(super) primitive: UnexpandablePrimitive,
    pub(super) use_object: Option<i32>,
    pub(super) dimensions: tex_state::PdfAnnotationDimensions,
    pub(super) attributes: Option<ScannedBalancedText>,
    pub(super) structure: Option<u32>,
    pub(super) identifier: Option<PdfActionIdentifier>,
    pub(super) phase: PdfNavigationScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfNavigationScalarPhase {
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
pub(super) struct PdfActionScalarProgress {
    pub(super) goto: Option<bool>,
    pub(super) file: Option<AttemptTokenListId>,
    pub(super) structure: Option<PdfActionIdentifier>,
    pub(super) target: Option<PdfActionTarget>,
    pub(super) phase: PdfActionScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfActionScalarPhase {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PdfObjectScalarProgress {
    pub(super) use_object: Option<i32>,
    pub(super) stream: bool,
    pub(super) stream_attr: Option<ScannedBalancedText>,
    pub(super) phase: PdfObjectScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfObjectScalarPhase {
    ReserveKeyword,
    UseKeyword,
    UseObject,
    StreamKeyword,
    AttributeKeyword,
    FileKeyword,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PdfFormScalarProgress {
    pub(super) attr: Option<ScannedBalancedText>,
    pub(super) resources: Option<ScannedBalancedText>,
    pub(super) phase: PdfFormScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfFormScalarPhase {
    AttributeKeyword,
    ResourcesKeyword,
    BoxRegister,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PdfImageScalarProgress {
    pub(super) width: Option<Scaled>,
    pub(super) height: Option<Scaled>,
    pub(super) depth: Option<Scaled>,
    pub(super) attr: Option<AttemptTokenListId>,
    pub(super) page: PendingPdfImagePage,
    pub(super) color_space_object: i32,
    pub(super) page_box: Option<PdfImagePageBox>,
    pub(super) phase: PdfImageScalarPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingPdfImagePage {
    Unset,
    Number(i32),
    Named(AttemptTokenListId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfImageScalarPhase {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PdfGraphicsScalarPhase {
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
pub(super) enum RuleScalarPhase {
    WidthKeyword,
    WidthDimension,
    HeightKeyword,
    HeightDimension,
    DepthKeyword,
    DepthDimension,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PackingScalarPhase {
    ToKeyword,
    SpreadKeyword,
    Dimension { exactly: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MathFieldRestrictedKind {
    Character,
    MathCharacter,
    Delimiter,
}

/// Stable pending-diagnostic identities for TeX82 §760 template recovery.
pub(super) const MISSING_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0001;
pub(super) const EXTRA_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0002;
pub(super) const MISSING_DELIMITER_DIAGNOSTIC: u64 = 0x6d61_7468_0000_0001;

pub(super) const MISSING_DELIMITER_HELP: &[&str] = &[
    "I was expecting to see something like `(' or `\\{' or",
    "`\\}' here. If you typed, e.g., `{' instead of `\\{', you",
    "should probably delete the `{' by typing `1' now, so that",
    "braces don't get unbalanced. Otherwise just proceed.",
    "Acceptable delimiters are characters whose \\delcode is",
    "nonnegative, or you can use `\\delimiter <delimiter code>'.",
];

pub(super) fn static_meaning<G>(meaning: ResolvedMeaning<G>) -> Option<Meaning> {
    match meaning {
        ResolvedMeaning::Static(meaning) => Some(meaning),
        ResolvedMeaning::Macro { .. } => None,
    }
}

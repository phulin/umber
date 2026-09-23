use super::*;

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

/// One checked attempt-local macro-definition builder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedMacroDefinition<G> {
    /// The raw control-sequence (or active-character) target accepted by
    /// TeX82's `prefixed_command`.  Target delivery is command-owned so the
    /// executor never has to reopen raw input between the primitive and its
    /// single-builder parameter/replacement scan.
    pub target: Symbol,
    pub definition: tex_state::DefinitionRef<G>,
    pub provenance: StructuredProvenance,
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

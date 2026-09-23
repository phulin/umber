use super::*;

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
    /// pdftex.web §1553's live fallback DPI, frozen when the host lookup begins.
    pub resolution: u32,
    pub attr: Option<AttemptTokenListId>,
}

impl PdfImageRequest {
    /// Whether two requests select the same immutable host image resource.
    ///
    /// pdftex.web §1550's `read_image` receives the file/page/page-box facts;
    /// rule dimensions and `attr` are command/output state. Dimensions remain
    /// in this deliberately conservative key, but `attr` cannot: its
    /// Attribute text is command-attempt state, not part of host resource
    /// identity. A replayed request obtains its fresh coordinate from the
    /// ordinary scan.
    pub(crate) fn same_resource_as(&self, other: &Self) -> bool {
        self.name == other.name
            && self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.page == other.page
            && self.color_space_object == other.color_space_object
            && self.page_box == other.page_box
            && self.page_box_explicit == other.page_box_explicit
            && self.resolution == other.resolution
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

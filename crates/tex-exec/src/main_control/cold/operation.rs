//! Borrow-barrier values for uncommon main-control operations.
//!
//! These values are runtime-only and never own an interpreter or semantic
//! state. Ranked commands must not acquire a variant here.

use super::super::*;

/// General text whose storage coordinate is selected by the operation phase.
/// Scanning uses attempt-local ids; prepared operations use generation-durable
/// ids without retaining an attempt coordinate anywhere in their type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedBalancedText<T> {
    pub(in crate::main_control) tokens: T,
    pub(in crate::main_control) provenance: tex_command::StructuredProvenance,
}

impl From<tex_command::ScannedBalancedText>
    for RootedBalancedText<tex_command::AttemptTokenListId>
{
    fn from(text: tex_command::ScannedBalancedText) -> Self {
        Self {
            tokens: text.tokens,
            provenance: text.provenance,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::main_control) struct RootedPdfImageRequest<T> {
    pub(in crate::main_control) name: String,
    pub(in crate::main_control) width: Option<Scaled>,
    pub(in crate::main_control) height: Option<Scaled>,
    pub(in crate::main_control) depth: Option<Scaled>,
    pub(in crate::main_control) page: tex_command::PdfImagePageSelection,
    pub(in crate::main_control) color_space_object: i32,
    pub(in crate::main_control) page_box: tex_command::PdfImagePageBox,
    pub(in crate::main_control) page_box_explicit: bool,
    pub(in crate::main_control) attr: Option<T>,
}

impl From<tex_command::PdfImageRequest> for RootedPdfImageRequest<tex_command::AttemptTokenListId> {
    fn from(request: tex_command::PdfImageRequest) -> Self {
        Self {
            name: request.name,
            width: request.width,
            height: request.height,
            depth: request.depth,
            page: request.page,
            color_space_object: request.color_space_object,
            page_box: request.page_box,
            page_box_explicit: request.page_box_explicit,
            attr: request.attr,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::main_control) enum RootedPdfActionIdentifier<T> {
    Name(T),
    Number(u32),
    Raw(T),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::main_control) enum RootedPdfActionTarget<T> {
    Page { number: u32, view: T },
    Destination(RootedPdfActionIdentifier<T>),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::main_control) struct RootedPdfActionDestination<T> {
    pub(in crate::main_control) file: Option<T>,
    pub(in crate::main_control) structure: Option<RootedPdfActionIdentifier<T>>,
    pub(in crate::main_control) target: RootedPdfActionTarget<T>,
    pub(in crate::main_control) window: tex_state::PdfActionWindow,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::main_control) enum RootedPdfActionSpec<T> {
    User(T),
    GoTo(RootedPdfActionDestination<T>),
    Thread(RootedPdfActionDestination<T>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfColorStackAction<T> {
    Set(RootedBalancedText<T>),
    Push(RootedBalancedText<T>),
    Pop,
    Current,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfGraphicsRequest<T> {
    Literal {
        mode: tex_state::node::PdfLiteralMode,
        deferred: bool,
        text: RootedBalancedText<T>,
    },
    SetMatrix {
        text: RootedBalancedText<T>,
    },
    Save,
    Restore,
    ColorStack {
        id: i32,
        action: Option<RootedPdfColorStackAction<T>>,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfObjectRequest<T> {
    Reserve,
    Define {
        use_object: Option<i32>,
        stream: bool,
        stream_attr: Option<RootedBalancedText<T>>,
        file: bool,
        data: RootedBalancedText<T>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfFormRequest<T> {
    Create {
        attr: Option<RootedBalancedText<T>>,
        resources: Option<RootedBalancedText<T>>,
        box_register: u16,
    },
    Reference {
        object: i32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedPdfDocumentFragmentRequest<T> {
    pub(in crate::main_control) kind: tex_state::PdfDocumentFragmentKind,
    pub(in crate::main_control) text: RootedBalancedText<T>,
    pub(in crate::main_control) open_action: Option<RootedPdfActionSpec<T>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfAnnotationRequest<T> {
    Reserve,
    Define {
        use_object: Option<i32>,
        dimensions: tex_state::PdfAnnotationDimensions,
        entries: RootedBalancedText<T>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedPdfStartLinkRequest<T> {
    pub(in crate::main_control) dimensions: tex_state::PdfAnnotationDimensions,
    pub(in crate::main_control) attributes: Option<RootedBalancedText<T>>,
    pub(in crate::main_control) action: RootedPdfActionSpec<T>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedPdfOutlineRequest<T> {
    pub(in crate::main_control) attributes: Option<RootedBalancedText<T>>,
    pub(in crate::main_control) action: RootedPdfActionSpec<T>,
    pub(in crate::main_control) count: i32,
    pub(in crate::main_control) title: RootedBalancedText<T>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedPdfDestinationRequest<T> {
    pub(in crate::main_control) structure: Option<u32>,
    pub(in crate::main_control) identifier: RootedPdfActionIdentifier<T>,
    pub(in crate::main_control) kind: tex_state::node::PdfDestinationKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) struct RootedPdfThreadRequest<T> {
    pub(in crate::main_control) dimensions: tex_state::PdfAnnotationDimensions,
    pub(in crate::main_control) attributes: Option<RootedBalancedText<T>>,
    pub(in crate::main_control) identifier: RootedPdfActionIdentifier<T>,
    pub(in crate::main_control) running: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedPdfNavigationRequest<T> {
    Annotation(RootedPdfAnnotationRequest<T>),
    StartLink(RootedPdfStartLinkRequest<T>),
    EndLink,
    Outline(RootedPdfOutlineRequest<T>),
    Destination(RootedPdfDestinationRequest<T>),
    Thread(RootedPdfThreadRequest<T>),
    EndThread,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedInputStreamRequest<T, D = (), S = Symbol> {
    Open {
        stream: i32,
        scanned: i32,
        recovered: bool,
        file_name: tex_command::ScannedFileName,
    },
    Close {
        stream: i32,
        scanned: i32,
        recovered: bool,
    },
    Read {
        stream: i32,
        target: S,
        global: bool,
        tokens: T,
        definition: D,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum RootedImmediateExtension<T> {
    Continue,
    PdfExtensionInDviMode(UnexpandablePrimitive),
    OpenOut {
        stream: u8,
        file_name: tex_command::ScannedFileName,
    },
    Write {
        stream: tex_command::WriteStreamSelector,
        tokens: T,
    },
    CloseOut {
        stream: tex_command::WriteStreamSelector,
    },
    PdfObject(RootedPdfObjectRequest<T>),
    PdfForm(RootedPdfFormRequest<T>),
    PdfImage(RootedPdfImageRequest<T>),
}

fn rooted_pdf_identifier(
    identifier: tex_command::PdfActionIdentifier,
) -> RootedPdfActionIdentifier<tex_command::AttemptTokenListId> {
    match identifier {
        tex_command::PdfActionIdentifier::Name(tokens) => RootedPdfActionIdentifier::Name(tokens),
        tex_command::PdfActionIdentifier::Number(number) => {
            RootedPdfActionIdentifier::Number(number)
        }
        tex_command::PdfActionIdentifier::Raw(tokens) => RootedPdfActionIdentifier::Raw(tokens),
    }
}

fn rooted_pdf_action(
    action: tex_command::PdfActionSpec,
) -> RootedPdfActionSpec<tex_command::AttemptTokenListId> {
    fn destination(
        destination: tex_command::PdfActionDestination,
    ) -> RootedPdfActionDestination<tex_command::AttemptTokenListId> {
        let target = match destination.target {
            tex_command::PdfActionTarget::Page { number, view } => {
                RootedPdfActionTarget::Page { number, view }
            }
            tex_command::PdfActionTarget::Destination(identifier) => {
                RootedPdfActionTarget::Destination(rooted_pdf_identifier(identifier))
            }
        };
        RootedPdfActionDestination {
            file: destination.file,
            structure: destination.structure.map(rooted_pdf_identifier),
            target,
            window: destination.window,
        }
    }

    match action {
        tex_command::PdfActionSpec::User(tokens) => RootedPdfActionSpec::User(tokens),
        tex_command::PdfActionSpec::GoTo(value) => RootedPdfActionSpec::GoTo(destination(value)),
        tex_command::PdfActionSpec::Thread(value) => {
            RootedPdfActionSpec::Thread(destination(value))
        }
    }
}

impl From<tex_command::PdfGraphicsRequest>
    for RootedPdfGraphicsRequest<tex_command::AttemptTokenListId>
{
    fn from(request: tex_command::PdfGraphicsRequest) -> Self {
        match request {
            tex_command::PdfGraphicsRequest::Literal {
                mode,
                deferred,
                text,
            } => Self::Literal {
                mode,
                deferred,
                text: text.into(),
            },
            tex_command::PdfGraphicsRequest::SetMatrix { text } => {
                Self::SetMatrix { text: text.into() }
            }
            tex_command::PdfGraphicsRequest::Save => Self::Save,
            tex_command::PdfGraphicsRequest::Restore => Self::Restore,
            tex_command::PdfGraphicsRequest::ColorStack { id, action } => Self::ColorStack {
                id,
                action: action.map(|action| match action {
                    tex_command::PdfColorStackActionRequest::Set(text) => {
                        RootedPdfColorStackAction::Set(text.into())
                    }
                    tex_command::PdfColorStackActionRequest::Push(text) => {
                        RootedPdfColorStackAction::Push(text.into())
                    }
                    tex_command::PdfColorStackActionRequest::Pop => RootedPdfColorStackAction::Pop,
                    tex_command::PdfColorStackActionRequest::Current => {
                        RootedPdfColorStackAction::Current
                    }
                }),
            },
            tex_command::PdfGraphicsRequest::SavePosition => Self::SavePosition,
            tex_command::PdfGraphicsRequest::SnapReferencePoint => Self::SnapReferencePoint,
            tex_command::PdfGraphicsRequest::SnapY { glue } => Self::SnapY { glue },
            tex_command::PdfGraphicsRequest::SnapYComp { ratio } => Self::SnapYComp { ratio },
        }
    }
}

impl From<tex_command::PdfObjectRequest>
    for RootedPdfObjectRequest<tex_command::AttemptTokenListId>
{
    fn from(request: tex_command::PdfObjectRequest) -> Self {
        match request {
            tex_command::PdfObjectRequest::Reserve => Self::Reserve,
            tex_command::PdfObjectRequest::Define {
                use_object,
                stream,
                stream_attr,
                file,
                data,
            } => Self::Define {
                use_object,
                stream,
                stream_attr: stream_attr.map(Into::into),
                file,
                data: data.into(),
            },
        }
    }
}

impl From<tex_command::PdfFormRequest> for RootedPdfFormRequest<tex_command::AttemptTokenListId> {
    fn from(request: tex_command::PdfFormRequest) -> Self {
        match request {
            tex_command::PdfFormRequest::Create {
                attr,
                resources,
                box_register,
            } => Self::Create {
                attr: attr.map(Into::into),
                resources: resources.map(Into::into),
                box_register,
            },
            tex_command::PdfFormRequest::Reference { object } => Self::Reference { object },
        }
    }
}

impl From<tex_command::PdfDocumentFragmentRequest>
    for RootedPdfDocumentFragmentRequest<tex_command::AttemptTokenListId>
{
    fn from(request: tex_command::PdfDocumentFragmentRequest) -> Self {
        Self {
            kind: request.kind,
            text: request.text.into(),
            open_action: request.open_action.map(rooted_pdf_action),
        }
    }
}

impl From<tex_command::PdfNavigationRequest>
    for RootedPdfNavigationRequest<tex_command::AttemptTokenListId>
{
    fn from(request: tex_command::PdfNavigationRequest) -> Self {
        match request {
            tex_command::PdfNavigationRequest::Annotation(request) => {
                Self::Annotation(match request {
                    tex_command::PdfAnnotationRequest::Reserve => {
                        RootedPdfAnnotationRequest::Reserve
                    }
                    tex_command::PdfAnnotationRequest::Define {
                        use_object,
                        dimensions,
                        entries,
                    } => RootedPdfAnnotationRequest::Define {
                        use_object,
                        dimensions,
                        entries: entries.into(),
                    },
                })
            }
            tex_command::PdfNavigationRequest::StartLink(request) => {
                Self::StartLink(RootedPdfStartLinkRequest {
                    dimensions: request.dimensions,
                    attributes: request.attributes.map(Into::into),
                    action: rooted_pdf_action(request.action),
                })
            }
            tex_command::PdfNavigationRequest::EndLink => Self::EndLink,
            tex_command::PdfNavigationRequest::Outline(request) => {
                Self::Outline(RootedPdfOutlineRequest {
                    attributes: request.attributes.map(Into::into),
                    action: rooted_pdf_action(request.action),
                    count: request.count,
                    title: request.title.into(),
                })
            }
            tex_command::PdfNavigationRequest::Destination(request) => {
                Self::Destination(RootedPdfDestinationRequest {
                    structure: request.structure,
                    identifier: rooted_pdf_identifier(request.identifier),
                    kind: request.kind,
                })
            }
            tex_command::PdfNavigationRequest::Thread(request) => {
                Self::Thread(RootedPdfThreadRequest {
                    dimensions: request.dimensions,
                    attributes: request.attributes.map(Into::into),
                    identifier: rooted_pdf_identifier(request.identifier),
                    running: request.running,
                })
            }
            tex_command::PdfNavigationRequest::EndThread => Self::EndThread,
        }
    }
}

impl From<tex_command::InputStreamRequest>
    for RootedInputStreamRequest<tex_command::AttemptTokenListId, tex_command::AttemptDefinitionId>
{
    fn from(request: tex_command::InputStreamRequest) -> Self {
        match request {
            tex_command::InputStreamRequest::Open {
                stream,
                scanned,
                recovered,
                file_name,
            } => Self::Open {
                stream,
                scanned,
                recovered,
                file_name,
            },
            tex_command::InputStreamRequest::Close {
                stream,
                scanned,
                recovered,
            } => Self::Close {
                stream,
                scanned,
                recovered,
            },
            tex_command::InputStreamRequest::Read {
                stream,
                target,
                global,
                tokens,
                definition,
            } => Self::Read {
                stream,
                target,
                global,
                tokens,
                definition,
            },
        }
    }
}

impl From<tex_command::ImmediateExtension>
    for RootedImmediateExtension<tex_command::AttemptTokenListId>
{
    fn from(request: tex_command::ImmediateExtension) -> Self {
        match request {
            tex_command::ImmediateExtension::Continue => Self::Continue,
            tex_command::ImmediateExtension::PdfExtensionInDviMode(primitive) => {
                Self::PdfExtensionInDviMode(primitive)
            }
            tex_command::ImmediateExtension::OpenOut { stream, file_name } => {
                Self::OpenOut { stream, file_name }
            }
            tex_command::ImmediateExtension::Write { stream, tokens } => {
                Self::Write { stream, tokens }
            }
            tex_command::ImmediateExtension::CloseOut { stream } => Self::CloseOut { stream },
            tex_command::ImmediateExtension::PdfObject(request) => Self::PdfObject(request.into()),
            tex_command::ImmediateExtension::PdfForm(request) => Self::PdfForm(request.into()),
            tex_command::ImmediateExtension::PdfImage(request) => Self::PdfImage(request.into()),
        }
    }
}

#[derive(Clone)]
pub(in crate::main_control) enum ColdOperation<
    G,
    T = tex_command::AttemptTokenListId,
    D = tex_command::AttemptDefinitionId,
> {
    Continue,
    Relax,
    /// e-TeX 2.6 `etex.ch` [17.3822--3880]'s `hmode+valign` extension:
    /// the four nonzero `valign` modifiers are text-direction nodes when
    /// `\TeXXeTstate>0`; otherwise `eTeX_enabled` diagnoses and ignores them.
    TextDirection {
        direction: tex_state::node::Direction,
        enabled: bool,
    },
    AlignPeekRestart {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §1128's `abs(align_state)>2` recovery: report the delivered
    /// delimiter and drop it without a backup or inserted brace.
    MisplacedAlignmentDelimiter {
        token: Token,
        context: String,
    },
    /// TeX82 §1129's command-specific misplaced-alignment report.
    MisplacedAlignmentCommand {
        omit: bool,
    },
    /// TeX82 §342 has just run §789's
    /// ``@<Insert the ⟨v_j⟩ template and |goto restart|@>``.
    ///
    /// This is not a `main_control` step at all: §789 runs *inside* `get_next`
    /// and jumps back to its own `restart` label, so the fetch that triggered
    /// it is still in progress. Umber routes the push out to the executor
    /// because the alignment's identity lives here, but must not treat the
    /// round trip as a return to §1030's `big_switch` -- main control is still
    /// parked wherever it was, and §1030's two fetch labels disagree about the
    /// `end_template` that a ⟨v_j⟩ template is about to deliver: §380's
    /// `get_x_token` rewrites it to `endv` in place, while §1038's `x_token`
    /// reaches §366 `expand` and §375 backs up a separate `frozen_endv` token
    /// to be reread.
    AlignmentTemplateEntered,
    /// TeX82 §1132's `align_group` recovery has backed up the closing brace
    /// and inserted frozen `\cr`; §82's `ins_error` diagnostic must run
    /// before alignment delivery fetches that inserted row terminator.
    MissingAlignmentCr,
    MissingMathShift,
    ReplayCompleted(tex_command::CommandReplayEpisode),
    Math(MathRequest),
    DisplayAlignmentRecovery,
    MathDelimiter(MathDelimiterBoundary),
    MathFamily {
        family: tex_command::ScannedMathFamily,
        font: FontId,
        global: bool,
    },
    EndOfInput,
    /// TeX82 §1045's `vmode+stop: if its_all_over then return` -- the only
    /// exit from `main_control`. §1335's `final_cleanup` has already unwound
    /// the abandoned input stack, so the job-termination effect is this
    /// step's, not a later input-exhaustion step's.
    ///
    /// §1335's `c` -- 0 for `\end`, 1 for `\dump` -- selects the whole tail
    /// of `final_cleanup`, so it is carried rather than discarded.
    End {
        dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    },
    /// TeX82 §1051's `privileged` failure for `\\end`/`\\dump` in internal
    /// vertical mode (`mode<0`): `report_illegal_case`, and the job keeps
    /// running. Shares the recovery shape of `IllegalBoxShift` and friends.
    IllegalStop {
        token: Token,
    },
    /// TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
    /// parameter command is diagnosed and discarded in every mode; it does
    /// not terminate main control.
    IllegalMacroParameter {
        token: Token,
    },
    /// TeX82 §1135's `cs_error`: a stray `\endcsname` is diagnosed and
    /// ignored exactly once, without changing input or mode state.
    ExtraEndCsName,
    /// TeX82 §1054's `its_all_over` false branch: the backed-up stop stays
    /// live while `\\hbox to \\hsize{}`, `\\vfill`, and
    /// `\\penalty-'10000000000` are appended to the contribution list and
    /// §994's `build_page` runs. Whether that fires `\\output` is §1005's
    /// decision.
    EjectResidualPage,
    Count {
        index: u16,
        value: i32,
        global: bool,
    },
    Dimen {
        index: u16,
        value: Scaled,
        global: bool,
    },
    /// TeX82 §1055's `assign_box_dimen: alter_box_dimen(cur_chr)` (`\wd`,
    /// `\ht`, `\dp`): `scan_eight_bit_int`, `scan_optional_equals`, then
    /// `scan_normal_dimen` set the given dimension on the box register's
    /// visible node if it is not void; a void or absent register is a
    /// documented no-op (§1055 skips the mutation entirely rather than
    /// erroring), matched by `Universe::set_box_dimension`'s own early
    /// return.
    BoxDimensionAssignment {
        index: u16,
        dimension: tex_state::BoxDimension,
        value: Scaled,
        global: bool,
    },
    Skip {
        index: u16,
        value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        source_skip_index: Option<u16>,
        redundant: bool,
        reassigning: bool,
        global: bool,
    },
    Muskip {
        index: u16,
        value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        redundant: bool,
        reassigning: bool,
        global: bool,
    },
    HorizontalSkip {
        value: GlueSpec,
    },
    VerticalSkip {
        value: GlueSpec,
    },
    Kern {
        amount: Scaled,
    },
    /// TeX82 §1102's `any_mode(break_penalty): append_penalty` -- `\penalty`
    /// is legal in every mode (vertical, horizontal, and math alike) with no
    /// mode switch of its own, unlike `\hskip`/`\kern`'s vertical-mode
    /// paragraph-start recovery above.
    Penalty {
        amount: i32,
    },
    CharacterCode {
        value: i32,
        origin: tex_state::token::OriginId,
        suppress_left_boundary: bool,
    },
    /// TeX82 §1105's `any_mode(remove_item): delete_last` -- `\unpenalty`,
    /// `\unkern`, and `\unskip` are legal in every mode with no scan of their
    /// own (the removed node, if any, is selected purely by matching the
    /// primitive against the current list's tail).
    DeleteLast {
        primitive: UnexpandablePrimitive,
        context: String,
    },
    /// TeX82 §1264's `new_interaction`: `\batchmode`/`\nonstopmode`/
    /// `\scrollmode`/`\errorstopmode` carry no operand of their own -- the
    /// target `InteractionMode` is selected from the delivered primitive at
    /// apply time, mirroring `DeleteLast` above.
    SetInteractionMode(UnexpandablePrimitive),
    /// e-TeX 2.6 etex.ch §3736's assignable `\interactionmode` primitive.
    SetInteractionModeValue {
        value: i32,
        context: String,
    },
    /// TeX82 §1112's `hmode+ital_corr: append_italic_correction` (the
    /// procedure itself is §1113) or its math-mode twin (§1112's
    /// `mmode+ital_corr: tail_append(new_kern(0))`); which applies is
    /// resolved from the live mode at apply time since neither takes an
    /// operand. Vertical mode is instead `IllegalItalicCorrection` below
    /// (`\/` is one of §1111's "Forbidden cases", not a paragraph-starting
    /// command).
    ItalicCorrection,
    /// TeX82 §1111's "Forbidden cases" `you_cant`/`report_illegal_case`
    /// recovery for `\/` in vertical or internal-vertical mode
    /// (`vmode+ital_corr`).
    IllegalItalicCorrection {
        token: Token,
    },
    /// TeX82 §1038's lookahead consumes `no_boundary` after a character run
    /// by setting `bchar:=non_char`, suppressing only that run's right
    /// boundary processing. The §1030 big-switch occurrence has different
    /// semantics and is folded into its following command during scanning.
    /// §1045's math-mode occurrence is a no-op.
    NoBoundary {
        suppress_right: bool,
    },
    /// TeX82 §1171's `mmode+non_script: tail_append(new_glue(zero_glue));
    /// subtype(tail):=cond_math_glue`. Legal only in math/display-math mode;
    /// §1046's `non_math(non_script)` routes every other mode through
    /// `MissingMathShift` instead (`scan_command` selects between the two
    /// before this step is produced).
    NonScript,
    /// TeX82 §1030's `hmode+ex_space,mmode+ex_space: goto append_normal_space`
    /// and §1090's `vmode+ex_space: back_input; new_graf(true)` -- `\ ` (the
    /// explicit control-space primitive) starts a paragraph in vertical mode
    /// exactly like a letter, then always appends the plain interword glue
    /// regardless of the current space factor.
    ControlSpace,
    /// TeX82 §1243's `set_aux` assignment (`alter_aux`) for the vertical-mode
    /// modifier: `\prevdepth=<dimen>` sets the enclosing vertical list's
    /// `prev_depth`, the field `append_to_vlist` (§679) reads to decide
    /// baselineskip/lineskip insertion before the next box. Legal only in
    /// vertical or internal-vertical mode; §1243's `report_illegal_case`
    /// (`abs(mode)<>vmode`) otherwise leaves the value alone.
    PrevDepth {
        value: Scaled,
    },
    /// TeX82 §1243 checks `cur_chr<>abs(mode)` before calling either
    /// `scan_optional_equals` or `scan_normal_dimen`. Thus an illegal-mode
    /// `\prevdepth` reports `report_illegal_case` while preserving the very
    /// next token as an ordinary main-control command.
    IllegalPrevDepth {
        token: Token,
    },
    /// TeX82 §1243's `set_aux` assignment for the horizontal-mode modifier:
    /// `\spacefactor=<number>` sets the current horizontal list's space
    /// factor. Legal only in horizontal or restricted-horizontal mode;
    /// §1243's `report_illegal_case` (`abs(mode)<>hmode`) otherwise leaves
    /// the value alone. A scanned value outside `1..32767` is a "Bad space
    /// factor" diagnostic that likewise leaves the space factor unchanged.
    SpaceFactor {
        value: i32,
    },
    /// TeX82 §1243 checks `cur_chr<>abs(mode)` before calling either
    /// `scan_optional_equals` or `scan_int`. Thus an illegal-mode
    /// `\spacefactor` reports `report_illegal_case` while preserving the
    /// very next token as an ordinary main-control command.
    IllegalSpaceFactor {
        token: Token,
    },
    /// TeX82 §1244's `set_prev_graf` (`alter_prev_graf`): `\prevgraf=<number>`
    /// is `any_mode` and walks the mode nest up to its nearest enclosing
    /// vertical level (`while abs(nest[p].mode_field)<>vmode do decr(p)`),
    /// setting that level's `prev_graf` (paragraph count so far) directly --
    /// unlike `\spacefactor`/`\prevdepth`, it never reports an illegal case. A
    /// negative scanned value is a "Bad \prevgraf" diagnostic that leaves the
    /// count unchanged.
    PrevGraf {
        value: i32,
    },
    /// TeX82 §1242's `set_page_dimen: alter_page_so_far`, whose body is
    /// §1245: `c:=cur_chr; scan_optional_equals; scan_normal_dimen;
    /// page_so_far[c]:=cur_val`. This is `\pagegoal`, `\pagetotal`,
    /// `\pagestretch`, `\pagefilstretch`, `\pagefillstretch`,
    /// `\pagefilllstretch`, `\pageshrink`, and `\pagedepth`.
    ///
    /// There is deliberately no `global` field: §1242 states outright that
    /// "these definitions are always global", and `page_so_far` is a plain
    /// engine array rather than an `eqtb` entry, so neither the `\global`
    /// prefix nor `\globaldefs` can reach it and no save-stack entry is
    /// pushed. `PrevDepth`/`SpaceFactor`/`PrevGraf` above and
    /// `BoxDimensionAssignment` below are scoped identically for the same
    /// reason.
    PageDimension {
        dimension: PageDimension,
        value: Scaled,
    },
    /// TeX82 §1242's `set_page_int: alter_integer`, whose body is §1246:
    /// `c:=cur_chr; scan_optional_equals; scan_int; if c=0 then
    /// dead_cycles:=cur_val else insert_penalties:=cur_val`. This is
    /// `\deadcycles` and `\insertpenalties`.
    ///
    /// Unscoped for the same §1242 reason as `PageDimension` above.
    PageInteger {
        integer: PageInteger,
        value: i32,
    },
    FixedHorizontalGlue {
        primitive: UnexpandablePrimitive,
    },
    FixedVerticalGlue {
        primitive: UnexpandablePrimitive,
    },
    ParagraphIndent {
        indent: bool,
    },
    ParagraphShape {
        lines: Vec<ParagraphShapeLine>,
        global: bool,
    },
    PenaltyArray {
        kind: PenaltyArrayKind,
        values: Vec<i32>,
        global: bool,
    },
    Toks {
        index: u16,
        tokens: T,
        global: bool,
    },
    IntParam {
        index: u16,
        value: i32,
        global: bool,
    },
    DimenParam {
        index: u16,
        value: Scaled,
        global: bool,
    },
    TokParam {
        index: u16,
        tokens: Option<T>,
        global: bool,
    },
    GlueParam {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    CodeTable {
        primitive: UnexpandablePrimitive,
        character: char,
        value: i32,
        global: bool,
    },
    /// pdftex.web §§468--470's globally owned per-font byte-code assignment.
    PdfFontCode {
        table: tex_state::PdfFontCode,
        font: FontId,
        character: u8,
        value: i32,
    },
    /// pdftex.web §471 suppresses ligature construction for one live font.
    PdfNoLigatures {
        font: FontId,
    },
    FontSelect {
        font: FontId,
        selector: Option<Symbol>,
        global: bool,
    },
    FontDefinition {
        request: FontLoadRequest,
        resource: Box<Option<FontResource>>,
        global: bool,
    },
    GeneratedFontDefinition {
        definition: ScannedGeneratedFontDefinition,
        global: bool,
    },
    InputStream {
        request: RootedInputStreamRequest<T, D>,
        resource: Option<SourceRegistration>,
    },
    PdfXImage {
        request: RootedPdfImageRequest<T>,
        resource: PdfImageResource,
    },
    PdfRefXImage {
        object: i32,
    },
    /// pdftex.web §1585's `\pdfsetrandomseed`: the command scanner has
    /// consumed one ordinary integer and normalized its sign; application
    /// replaces the ungrouped job RNG state atomically.
    PdfSetRandomSeed {
        seed: i32,
    },
    /// pdftex.web §1586's `\pdfresettimer`: there is no operand; application
    /// atomically rebases the ungrouped job timer to the deterministic
    /// monotonic sample already held by `World`.
    PdfResetTimer,
    /// pdftex.web §§1594–1596's operand-free interword-space controls.
    /// Application appends an ordered whatsit after flushing any pending
    /// horizontal character run; shipout traversal owns the toggle state.
    PdfInterwordSpace(tex_state::node::PdfAccessibilityControl),
    /// pdftex.web §§1597–1598's operand-free running-link shipout controls.
    /// Application appends an ordered whatsit; PDF traversal owns the
    /// initially-enabled policy for continuation annotations.
    PdfRunningLink(bool),
    /// pdftex.web §1599's expanded balanced text selecting the global,
    /// job-owned fallback font name used by accessible-space PDF output.
    PdfSpaceFont(T),
    PdfGraphics(RootedPdfGraphicsRequest<T>),
    PdfObject(RootedPdfObjectRequest<T>),
    PdfReferenceObject(PdfReferenceObjectRequest),
    PdfForm(RootedPdfFormRequest<T>),
    PdfDocumentFragment(RootedPdfDocumentFragmentRequest<T>),
    PdfNavigation(RootedPdfNavigationRequest<T>),
    FontDimen {
        font: FontId,
        /// tex.web §578's `n`, unrecovered. A number at or below zero
        /// resolves to §578's scratch `fmem_ptr` and is reported by §579, so
        /// the scan must carry it rather than reject it.
        number: i32,
        value: Scaled,
        /// §82's `show_context` at the point §578 decided the number was
        /// unusable -- after the font identifier, before `=<dimen>`. `None`
        /// when §578 accepted it, which is also what says no §579 report is
        /// due.
        recovery_context: Option<String>,
    },
    FontInteger {
        font: FontId,
        skew: bool,
        value: i32,
    },
    /// pdftex.web §§1680--1682's font expansion configuration.
    PdfFontExpand {
        font: FontId,
        spec: tex_typeset::expansion::FontExpansionSpec,
    },
    /// pdftex.web §§1601--1607's font and map extension actions.  Operand
    /// expansion belongs to the command processor; host-neutral state
    /// mutation remains at the apply seam.
    PdfFontAction {
        primitive: UnexpandablePrimitive,
        font: Option<FontId>,
        first: Option<T>,
        second: Option<T>,
    },
    DeferredOpenOut {
        stream: u8,
        file_name: String,
    },
    DeferredCloseOut {
        stream: tex_command::WriteStreamSelector,
    },
    DeferredWrite {
        stream: tex_command::WriteStreamSelector,
        tokens: T,
    },
    DeferredSpecial {
        deferred: bool,
        tokens: T,
    },
    /// TeX82 §1377's `@<Implement \setlanguage@>`, reached from §1348's
    /// `do_extension` on `set_language_code` (§1344's `extension` modifier
    /// 5). Unlike §1376's `fix_language`, which appends a §1341
    /// `language_node` only when the language actually changes, §1377
    /// appends one unconditionally -- `\setlanguage` is an explicit request,
    /// so a same-language `\setlanguage` still produces a whatsit.
    ///
    /// `language` is `cur_val` exactly as `scan_int` left it; §1377's own
    /// `<=0`/`>255` normalization to `clang` is performed at the apply seam
    /// together with `norm_min` (§1091), because both write mode-nest and
    /// list state the scan phase does not own.
    SetLanguage {
        language: i32,
    },
    /// TeX82 §1377's `if abs(mode)<>hmode then report_illegal_case`.
    ///
    /// The mode test precedes both `new_whatsit` and `scan_int`, so an
    /// out-of-mode `\setlanguage` scans no operand at all -- matching
    /// `IllegalBoxShift`/`IllegalInsertOrAdjust`'s same-shaped recovery, and
    /// unlike `\prevdepth`'s §1243 check, which runs after its value is
    /// scanned.
    IllegalSetLanguage {
        token: Token,
    },
    Arithmetic {
        primitive: UnexpandablePrimitive,
        target: ArithmeticTarget,
        operand: ArithmeticOperand,
        global: bool,
    },
    /// TeX82 §1236's recoverable invalid-target return from
    /// `do_register_command`. The target command has been consumed, but no
    /// operand is scanned and no value is changed.
    InvalidArithmeticTarget {
        primitive: UnexpandablePrimitive,
        target: tex_command::PrintCommand<G>,
    },
    CharacterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        /// Meaning replaced by §1224's already-applied provisional `\relax`.
        provisional_old: tex_state::meaning::ResolvedMeaning<G>,
        /// `cur_val` after §434/§436's recover-to-zero.
        value: i32,
        global: bool,
    },
    /// TeX82 §1252's `hyph_data` command: `\patterns` (`chr_code=1`) installs
    /// pattern data through §960's `new_patterns`; `\hyphenation`
    /// (`chr_code=0`) installs exception words through §934's
    /// `new_hyph_exceptions`. The scan carries §935's raw exception words or
    /// §962's normalized pattern specs; the flag selects which table they
    /// populate.
    HyphenationData {
        words: Vec<Vec<char>>,
        pattern_specs: Vec<tex_state::hyphenation::PatternSpec>,
        patterns: bool,
        /// §82's context for whichever of the two `\patterns` rejections the
        /// apply seam raises. Both report before the braced group is read --
        /// §960's `trie_not_ready=false` branch before §473 discards it, and
        /// §1252's production branch before its own `repeat get_token` flush --
        /// so the context has to be captured at the pre-scan cursor, with
        /// `\patterns` behind it and the group still ahead.
        rejection_context: String,
        /// Whether §960's trie was already built, which is the half of the
        /// rejection test the command core can see. The other half -- whether
        /// tex.web's `init`/`tini` split would have produced this binary at
        /// all -- is the session's, and is applied with it.
        trie_built: bool,
    },
    RegisterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        /// Meaning replaced by §1224's already-applied provisional `\relax`.
        provisional_old: tex_state::meaning::ResolvedMeaning<G>,
        index: u16,
        global: bool,
    },
    AfterGroup(tex_state::token::TracedTokenWord),
    AfterAssignment(tex_state::token::TracedTokenWord),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        horizontal: bool,
    },
    HRuleHereExceptLeaders,
    Message {
        tokens: T,
        error: bool,
    },
    DisplayDiagnostic(ScannedDisplayDiagnostic),
    ShowBox {
        index: u16,
    },
    /// TeX82 §1293's `show_lists_code` branch of `show_whatever`, which takes
    /// no operand: `begin_diagnostic; show_activities`. The mode is carried
    /// so the committed effect can name the nest it reported.
    ShowLists,
    /// e-TeX 2.6 `etex.ch` [17.3623--3671]'s `\\showtokens`: command
    /// processing has removed the compulsory braces and frozen the
    /// unexpanded balanced interior. Replay owns only diagnostic rendering.
    ShowTokens {
        tokens: T,
    },
    /// e-TeX 2.6 `etex.ch` [17.3703--3732]'s read-only conditional-stack
    /// diagnostic, detached in innermost-to-outermost traversal order.
    ShowIfs {
        conditions: Vec<tex_command::ActiveCondition>,
    },
    /// e-TeX 2.6 [49.1292]'s operand-free, any-mode `\showgroups`.
    ShowGroups {
        diagnostic: Option<crate::diagnostics::ShowGroupsDiagnostic>,
    },
    VSplit(ScannedVSplit),
    ImmediateExtension(RootedImmediateExtension<T>),
    BoxRegister {
        index: u16,
        copy: bool,
        ships_out: bool,
    },
    Unbox {
        primitive: UnexpandablePrimitive,
        index: u16,
        error_context: String,
    },
    /// e-TeX 2.6 `etex.ch` [45.999]'s operand-free extensions of TeX82's
    /// `un_vbox` command. The selected saved list is detached and spliced
    /// into the current list atomically; unlike `\unvbox`, no register
    /// number is scanned.
    SavedVerticalDiscards(UnexpandablePrimitive),
    LastBox {
        error_context: String,
    },
    Leaders {
        kind: GlueKind,
        payload: LeaderPayload,
        glue: GlueSpec,
    },
    LeaderRegister {
        kind: GlueKind,
        index: u16,
        copy: bool,
        glue: GlueSpec,
    },
    MissingLeaderPayload,
    LeadersNotFollowedByGlue,
    BeginShipout,
    BeginAlignment {
        vertical: bool,
        owner: Option<tex_state::interner::Symbol>,
    },
    AlignmentPreambleOpening {
        alignment: AlignmentIdentity,
        packing: ScannedPackingSpec,
    },
    AlignmentPreambleStart {
        alignment: AlignmentIdentity,
    },
    AlignmentCellOpening {
        alignment: AlignmentIdentity,
        opening: AlignmentCellOpening,
    },
    /// TeX82's `do_endv` completed a command-owned v-template.  Applying
    /// this result retires that exact frame before the backed-up delimiter
    /// resumes through `get_next`.
    AlignmentCellFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 delivered the alignment-closing right brace, so `fin_align`
    /// must complete before the outer suspended delivery context resumes.
    AlignmentFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 has consumed `\\noalign` and its compulsory opening brace.
    /// Command control owns both deliveries; the executor now owns the
    /// no-align group's structural entry.
    BeginNoAlign {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §1127's `align_error` has backed up the delimiter and inserted
    /// this balancing brace. Applying the step emits the matching `ins_error`
    /// diagnostic before replay fetches the inserted token.
    AlignmentRecovery {
        brace: Catcode,
    },
    BeginSimpleGroup,
    EndSimpleGroup,
    /// TeX82 §1068's `handle_right_brace` for a brace the current group
    /// cannot account for. `forgotten` is §1069's `case cur_group of` -- the
    /// opener the brace was probably standing in for -- and `None` is §1068's
    /// own `bottom_level` arm, "Too many }'s".
    ExtraRightBrace {
        forgotten: Option<ForgottenGroupOpener>,
    },
    /// TeX82 §1186: the closing brace of a `math_group` opened by §1153's
    /// `push_math`, or §1174's `build_choices` closing a `math_choice_group`
    /// opened by §1172/§1174. Applying it only `unsave`s; the nested loop in
    /// `execute_live_math_group` notices the level is gone and finishes the
    /// mlist.
    EndMathGroup(GroupKind),
    /// TeX82 §1064's general `off_save`: the innermost group could not
    /// accommodate the scanned command, so `scan_off_save` already chose and
    /// inserted the matching closer ahead of the backed-up command
    /// (`CommandProcessor::recover_off_save`). The execute phase only prints
    /// §1064's report naming what was inserted; the closer travels as the
    /// typed choice §1065 already made rather than as rendered text, because
    /// two of the four spell a control sequence and so must be printed
    /// through §63's `print_esc` under the live `\\escapechar`.
    OffSave(OffSaveCloser),
    /// TeX82 §§1064/1066's bottom-level `off_save`: no enclosing group
    /// existed to close, so the offending command was dropped outright
    /// (`CommandProcessor::report_off_save_bottom_drop` already ran); the
    /// execute phase prints "Extra `<command>`" naming its own spelling.
    OffSaveBottomDrop {
        token: Token,
    },
    OutputRoutineOpeningBrace,
    EndOutputRoutine,
    AlignmentPeekCell {
        alignment: AlignmentIdentity,
        omit: bool,
    },
    NoAlignEndGroup {
        alignment: AlignmentIdentity,
    },
    SetBox {
        target: SetBoxTarget,
        path: ScannedSetBoxPath,
    },
    BeginBox(ScannedBoxConstruction),
    BeginLeaderBox {
        construction: ScannedBoxConstruction,
        kind: GlueKind,
    },
    /// TeX82 §1073's box-shift prefixes (`\raise`, `\lower`, `\moveleft`,
    /// `\moveright`): the already-signed shift amount (tex.web's
    /// `box_context`) plus `scan_box`'s own `make_box` operand (§1084).
    /// `ScannedBoxShiftPayload::Construction` shares the `BeginBox`/
    /// `BoxEndGroup` body-closing machinery; the other
    /// variants resolve to a node immediately, exactly like `\box<n>`,
    /// `\lastbox`, and `\vsplit` do outside a shift.
    BoxShift(ScannedBoxShift),
    /// TeX82's "Forbidden cases" `you_cant`/`report_illegal_case` recovery
    /// for a box-shift prefix used in the wrong mode (`vmode+vmove`,
    /// `hmode+hmove`, `mmode+hmove`): the dimension is never scanned, unlike
    /// `\prevdepth`'s mode check, which runs only after its value is
    /// scanned.
    IllegalBoxShift {
        token: Token,
    },
    /// TeX82 §1099's `begin_insert_or_adjust` for `\insert` and `\vadjust`.
    /// The class-number bound check (0..=255) and the reserved-255 recovery
    /// both need `Universe` diagnostics, so replay receives only the raw
    /// scanned class (fixed at 255 for `\vadjust`); the body's mandatory
    /// opening brace was already consumed by `scan_left_brace`. It shares the
    /// same brace-matching machinery as `BeginBox`/`BoxEndGroup`.
    BeginInsert(ScannedInsertConstruction),
    /// TeX82's "Forbidden cases" `vmode+vadjust`: `\vadjust` never reaches
    /// its mandatory `scan_left_brace` in vertical mode, matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`'s same-shaped recovery.
    IllegalInsertOrAdjust {
        token: Token,
    },
    /// TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)` (added to the
    /// shared Forbidden-cases list first built at §1048): `\eqno`/`\leqno`
    /// outside math mode take `report_illegal_case` ("You can't use
    /// `\eqno' in ... mode") rather than §1047's `insert_dollar_sign`, even
    /// though tex.web registers them under the same `eq_no` command code as
    /// the math-request vocabulary `scan_math_request` otherwise
    /// dispatches. Reaching this arm proves `mode` is not
    /// `Math`/`DisplayMath` (that gate would have consumed the primitive
    /// first via `Request::EquationNumber`), matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`/`IllegalInsertOrAdjust`'s
    /// same-shaped recovery. `mmode+eq_no` itself (gated by
    /// `privileged`/`cur_group`) is unaffected.
    IllegalEqNo {
        token: Token,
    },
    /// TeX82 §§1051 and 1130's `mmode+halign: if privileged ...`:
    /// inline math has negative `mmode`, so `privileged` diagnoses the
    /// command and returns before `init_align` scans any preamble input.
    IllegalHAlign {
        token: Token,
    },
    /// TeX82 §1048's `@<Forbidden cases@>=...,any_mode(last_item),...` (the
    /// same module `IllegalBoxShift`'s `vmode+vmove`/`hmode+hmove`/
    /// `mmode+hmove` triple comes from, and that `IllegalEqNo`'s §1144
    /// addition later extends): `\lastpenalty`, `\lastkern`, and `\lastskip`
    /// have no assignment form and no standalone typesetting meaning in any
    /// mode -- they are legal only as an internal-value operand inside a
    /// scan (`CommandProcessor::internal_value_from_command`'s `LastPenalty`/
    /// `LastKern`/`LastSkip` arms). Reaching main control with one of these
    /// as the delivered command therefore always means `report_illegal_case`,
    /// matching `IllegalBoxShift`/`IllegalInsertOrAdjust`/`IllegalEqNo`'s
    /// same-shaped recovery.
    IllegalLastItem {
        token: Token,
        context: String,
    },
    BoxEndGroup {
        ships_out: bool,
    },
    /// TeX82 §1101 and e-TeX 2.6 `etex.ch` [26.424]'s `make_mark`: a fully
    /// expanded balanced general text, appended as the selected mark class.
    Mark {
        class: u16,
        tokens: T,
    },
    Paragraph,
    MathShift {
        pairing: MathShiftPairing,
    },
    ParagraphStart,
    Character {
        ch: char,
        cat: Catcode,
        origin: tex_state::token::OriginId,
        suppress_left_boundary: bool,
    },
    Accent(ScannedAccent),
    DiscretionaryOpening(ScannedDiscretionaryOpening),
    DiscretionaryPartEnd,
    DiscretionaryHyphen {
        origin: tex_state::token::OriginId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum MathShiftPairing {
    Unpaired,
    Paired,
    ProbeDisplayEnd,
}

/// A cold operation after its complete attempt-root set has crossed the one
/// outer promotion boundary. The type contains no attempt-local coordinate.
pub(in crate::main_control) type PreparedColdCommand<G> =
    ColdOperation<G, tex_state::TokenListId<G>, tex_state::DefinitionId<G>>;

#[derive(Debug)]
pub(in crate::main_control) enum ColdPreparationError {
    Promotion(tex_command::AttemptError),
    ReceiptUnderflow,
    ReceiptRemainder { remaining: usize },
    Definition(tex_state::UniverseError),
}

impl From<tex_command::AttemptError> for ColdPreparationError {
    fn from(error: tex_command::AttemptError) -> Self {
        Self::Promotion(error)
    }
}

struct PromotionCursor<T> {
    token_lists: std::vec::IntoIter<T>,
}

impl<T> PromotionCursor<T> {
    fn new(token_lists: Vec<T>) -> Self {
        Self {
            token_lists: token_lists.into_iter(),
        }
    }

    fn token(&mut self) -> Result<T, ColdPreparationError> {
        self.token_lists
            .next()
            .ok_or(ColdPreparationError::ReceiptUnderflow)
    }

    fn finish(self) -> Result<(), ColdPreparationError> {
        let remaining = self.token_lists.len();
        if remaining == 0 {
            Ok(())
        } else {
            Err(ColdPreparationError::ReceiptRemainder { remaining })
        }
    }
}

fn prepare_balanced<G>(
    text: RootedBalancedText<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedBalancedText<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(RootedBalancedText {
        tokens: cursor.token()?,
        provenance: text.provenance,
    })
}

fn prepare_pdf_identifier<G>(
    identifier: RootedPdfActionIdentifier<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfActionIdentifier<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match identifier {
        RootedPdfActionIdentifier::Name(_) => RootedPdfActionIdentifier::Name(cursor.token()?),
        RootedPdfActionIdentifier::Number(number) => RootedPdfActionIdentifier::Number(number),
        RootedPdfActionIdentifier::Raw(_) => RootedPdfActionIdentifier::Raw(cursor.token()?),
    })
}

fn prepare_pdf_target<G>(
    target: RootedPdfActionTarget<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfActionTarget<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match target {
        RootedPdfActionTarget::Page { number, .. } => RootedPdfActionTarget::Page {
            number,
            view: cursor.token()?,
        },
        RootedPdfActionTarget::Destination(identifier) => {
            RootedPdfActionTarget::Destination(prepare_pdf_identifier(identifier, cursor)?)
        }
    })
}

fn prepare_pdf_destination<G>(
    destination: RootedPdfActionDestination<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfActionDestination<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(RootedPdfActionDestination {
        file: destination.file.map(|_| cursor.token()).transpose()?,
        structure: destination
            .structure
            .map(|identifier| prepare_pdf_identifier(identifier, cursor))
            .transpose()?,
        target: prepare_pdf_target(destination.target, cursor)?,
        window: destination.window,
    })
}

fn prepare_pdf_action<G>(
    action: RootedPdfActionSpec<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfActionSpec<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match action {
        RootedPdfActionSpec::User(_) => RootedPdfActionSpec::User(cursor.token()?),
        RootedPdfActionSpec::GoTo(destination) => {
            RootedPdfActionSpec::GoTo(prepare_pdf_destination(destination, cursor)?)
        }
        RootedPdfActionSpec::Thread(destination) => {
            RootedPdfActionSpec::Thread(prepare_pdf_destination(destination, cursor)?)
        }
    })
}

fn prepare_pdf_graphics<G>(
    request: RootedPdfGraphicsRequest<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfGraphicsRequest<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match request {
        RootedPdfGraphicsRequest::Literal {
            mode,
            deferred,
            text,
        } => RootedPdfGraphicsRequest::Literal {
            mode,
            deferred,
            text: prepare_balanced(text, cursor)?,
        },
        RootedPdfGraphicsRequest::SetMatrix { text } => RootedPdfGraphicsRequest::SetMatrix {
            text: prepare_balanced(text, cursor)?,
        },
        RootedPdfGraphicsRequest::Save => RootedPdfGraphicsRequest::Save,
        RootedPdfGraphicsRequest::Restore => RootedPdfGraphicsRequest::Restore,
        RootedPdfGraphicsRequest::ColorStack { id, action } => {
            RootedPdfGraphicsRequest::ColorStack {
                id,
                action: action
                    .map(|action| {
                        Ok::<_, ColdPreparationError>(match action {
                            RootedPdfColorStackAction::Set(text) => {
                                RootedPdfColorStackAction::Set(prepare_balanced(text, cursor)?)
                            }
                            RootedPdfColorStackAction::Push(text) => {
                                RootedPdfColorStackAction::Push(prepare_balanced(text, cursor)?)
                            }
                            RootedPdfColorStackAction::Pop => RootedPdfColorStackAction::Pop,
                            RootedPdfColorStackAction::Current => {
                                RootedPdfColorStackAction::Current
                            }
                        })
                    })
                    .transpose()?,
            }
        }
        RootedPdfGraphicsRequest::SavePosition => RootedPdfGraphicsRequest::SavePosition,
        RootedPdfGraphicsRequest::SnapReferencePoint => {
            RootedPdfGraphicsRequest::SnapReferencePoint
        }
        RootedPdfGraphicsRequest::SnapY { glue } => RootedPdfGraphicsRequest::SnapY { glue },
        RootedPdfGraphicsRequest::SnapYComp { ratio } => {
            RootedPdfGraphicsRequest::SnapYComp { ratio }
        }
    })
}

fn prepare_pdf_object<G>(
    request: RootedPdfObjectRequest<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfObjectRequest<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match request {
        RootedPdfObjectRequest::Reserve => RootedPdfObjectRequest::Reserve,
        RootedPdfObjectRequest::Define {
            use_object,
            stream,
            stream_attr,
            file,
            data,
        } => RootedPdfObjectRequest::Define {
            use_object,
            stream,
            stream_attr: stream_attr
                .map(|text| prepare_balanced(text, cursor))
                .transpose()?,
            file,
            data: prepare_balanced(data, cursor)?,
        },
    })
}

fn prepare_pdf_form<G>(
    request: RootedPdfFormRequest<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfFormRequest<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match request {
        RootedPdfFormRequest::Create {
            attr,
            resources,
            box_register,
        } => RootedPdfFormRequest::Create {
            attr: attr
                .map(|text| prepare_balanced(text, cursor))
                .transpose()?,
            resources: resources
                .map(|text| prepare_balanced(text, cursor))
                .transpose()?,
            box_register,
        },
        RootedPdfFormRequest::Reference { object } => RootedPdfFormRequest::Reference { object },
    })
}

fn prepare_pdf_navigation<G>(
    request: RootedPdfNavigationRequest<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfNavigationRequest<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match request {
        RootedPdfNavigationRequest::Annotation(request) => {
            RootedPdfNavigationRequest::Annotation(match request {
                RootedPdfAnnotationRequest::Reserve => RootedPdfAnnotationRequest::Reserve,
                RootedPdfAnnotationRequest::Define {
                    use_object,
                    dimensions,
                    entries,
                } => RootedPdfAnnotationRequest::Define {
                    use_object,
                    dimensions,
                    entries: prepare_balanced(entries, cursor)?,
                },
            })
        }
        RootedPdfNavigationRequest::StartLink(request) => {
            RootedPdfNavigationRequest::StartLink(RootedPdfStartLinkRequest {
                dimensions: request.dimensions,
                attributes: request
                    .attributes
                    .map(|text| prepare_balanced(text, cursor))
                    .transpose()?,
                action: prepare_pdf_action(request.action, cursor)?,
            })
        }
        RootedPdfNavigationRequest::EndLink => RootedPdfNavigationRequest::EndLink,
        RootedPdfNavigationRequest::Outline(request) => {
            RootedPdfNavigationRequest::Outline(RootedPdfOutlineRequest {
                attributes: request
                    .attributes
                    .map(|text| prepare_balanced(text, cursor))
                    .transpose()?,
                action: prepare_pdf_action(request.action, cursor)?,
                count: request.count,
                title: prepare_balanced(request.title, cursor)?,
            })
        }
        RootedPdfNavigationRequest::Destination(request) => {
            RootedPdfNavigationRequest::Destination(RootedPdfDestinationRequest {
                structure: request.structure,
                identifier: prepare_pdf_identifier(request.identifier, cursor)?,
                kind: request.kind,
            })
        }
        RootedPdfNavigationRequest::Thread(request) => {
            RootedPdfNavigationRequest::Thread(RootedPdfThreadRequest {
                dimensions: request.dimensions,
                attributes: request
                    .attributes
                    .map(|text| prepare_balanced(text, cursor))
                    .transpose()?,
                identifier: prepare_pdf_identifier(request.identifier, cursor)?,
                running: request.running,
            })
        }
        RootedPdfNavigationRequest::EndThread => RootedPdfNavigationRequest::EndThread,
    })
}

fn prepare_pdf_image<G>(
    request: RootedPdfImageRequest<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedPdfImageRequest<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(RootedPdfImageRequest {
        name: request.name,
        width: request.width,
        height: request.height,
        depth: request.depth,
        page: request.page,
        color_space_object: request.color_space_object,
        page_box: request.page_box,
        page_box_explicit: request.page_box_explicit,
        attr: request.attr.map(|_| cursor.token()).transpose()?,
    })
}

fn prepare_immediate_extension<G>(
    request: RootedImmediateExtension<tex_command::AttemptTokenListId>,
    cursor: &mut PromotionCursor<tex_state::TokenListId<G>>,
) -> Result<RootedImmediateExtension<tex_state::TokenListId<G>>, ColdPreparationError> {
    Ok(match request {
        RootedImmediateExtension::Continue => RootedImmediateExtension::Continue,
        RootedImmediateExtension::PdfExtensionInDviMode(primitive) => {
            RootedImmediateExtension::PdfExtensionInDviMode(primitive)
        }
        RootedImmediateExtension::OpenOut { stream, file_name } => {
            RootedImmediateExtension::OpenOut { stream, file_name }
        }
        RootedImmediateExtension::Write { stream, .. } => RootedImmediateExtension::Write {
            stream,
            tokens: cursor.token()?,
        },
        RootedImmediateExtension::CloseOut { stream } => {
            RootedImmediateExtension::CloseOut { stream }
        }
        RootedImmediateExtension::PdfObject(request) => {
            RootedImmediateExtension::PdfObject(prepare_pdf_object(request, cursor)?)
        }
        RootedImmediateExtension::PdfForm(request) => {
            RootedImmediateExtension::PdfForm(prepare_pdf_form(request, cursor)?)
        }
        RootedImmediateExtension::PdfImage(request) => {
            RootedImmediateExtension::PdfImage(prepare_pdf_image(request, cursor)?)
        }
    })
}

/// Promotes all attempt-local roots in one validated batch, then rebuilds the
/// operation by consuming the ordered receipt exactly once.
pub(in crate::main_control) fn prepare_cold_operation<G>(
    operation: ColdOperation<G>,
    command: &tex_command::CommandState<G>,
    stores: &mut Universe<G>,
) -> Result<PreparedColdCommand<G>, ColdPreparationError> {
    let mut roots = Vec::new();
    operation.attempt_token_roots(&mut roots);
    let mut definitions = Vec::new();
    operation.attempt_definition_roots(&mut definitions);
    let receipt = command.promote_attempt_roots(
        stores,
        tex_command::AttemptPromotionRoots::new(&roots, &[], &definitions, &[]),
    )?;
    let mut cursor = PromotionCursor::new(receipt.token_lists);
    let mut definition_cursor = PromotionCursor::new(receipt.definitions);
    let prepared = match operation {
        ColdOperation::Continue => ColdOperation::Continue,
        ColdOperation::Relax => ColdOperation::Relax,
        ColdOperation::TextDirection { direction, enabled } => {
            ColdOperation::TextDirection { direction, enabled }
        }
        ColdOperation::AlignPeekRestart { alignment } => {
            ColdOperation::AlignPeekRestart { alignment }
        }
        ColdOperation::MisplacedAlignmentDelimiter { token, context } => {
            ColdOperation::MisplacedAlignmentDelimiter { token, context }
        }
        ColdOperation::MisplacedAlignmentCommand { omit } => {
            ColdOperation::MisplacedAlignmentCommand { omit }
        }
        ColdOperation::AlignmentTemplateEntered => ColdOperation::AlignmentTemplateEntered,
        ColdOperation::MissingAlignmentCr => ColdOperation::MissingAlignmentCr,
        ColdOperation::MissingMathShift => ColdOperation::MissingMathShift,
        ColdOperation::ReplayCompleted(episode) => ColdOperation::ReplayCompleted(episode),
        ColdOperation::Math(request) => ColdOperation::Math(request),
        ColdOperation::DisplayAlignmentRecovery => ColdOperation::DisplayAlignmentRecovery,
        ColdOperation::MathDelimiter(boundary) => ColdOperation::MathDelimiter(boundary),
        ColdOperation::MathFamily {
            family,
            font,
            global,
        } => ColdOperation::MathFamily {
            family,
            font,
            global,
        },
        ColdOperation::EndOfInput => ColdOperation::EndOfInput,
        ColdOperation::End {
            dump,
            incomplete_conditions,
        } => ColdOperation::End {
            dump,
            incomplete_conditions,
        },
        ColdOperation::IllegalStop { token } => ColdOperation::IllegalStop { token },
        ColdOperation::IllegalMacroParameter { token } => {
            ColdOperation::IllegalMacroParameter { token }
        }
        ColdOperation::ExtraEndCsName => ColdOperation::ExtraEndCsName,
        ColdOperation::EjectResidualPage => ColdOperation::EjectResidualPage,
        ColdOperation::Count {
            index,
            value,
            global,
        } => ColdOperation::Count {
            index,
            value,
            global,
        },
        ColdOperation::Dimen {
            index,
            value,
            global,
        } => ColdOperation::Dimen {
            index,
            value,
            global,
        },
        ColdOperation::BoxDimensionAssignment {
            index,
            dimension,
            value,
            global,
        } => ColdOperation::BoxDimensionAssignment {
            index,
            dimension,
            value,
            global,
        },
        ColdOperation::Skip {
            index,
            value,
            source_identity,
            source_skip_index,
            redundant,
            reassigning,
            global,
        } => ColdOperation::Skip {
            index,
            value,
            source_identity,
            source_skip_index,
            redundant,
            reassigning,
            global,
        },
        ColdOperation::Muskip {
            index,
            value,
            source_identity,
            redundant,
            reassigning,
            global,
        } => ColdOperation::Muskip {
            index,
            value,
            source_identity,
            redundant,
            reassigning,
            global,
        },
        ColdOperation::HorizontalSkip { value } => ColdOperation::HorizontalSkip { value },
        ColdOperation::VerticalSkip { value } => ColdOperation::VerticalSkip { value },
        ColdOperation::Kern { amount } => ColdOperation::Kern { amount },
        ColdOperation::Penalty { amount } => ColdOperation::Penalty { amount },
        ColdOperation::CharacterCode {
            value,
            origin,
            suppress_left_boundary,
        } => ColdOperation::CharacterCode {
            value,
            origin,
            suppress_left_boundary,
        },
        ColdOperation::DeleteLast { primitive, context } => {
            ColdOperation::DeleteLast { primitive, context }
        }
        ColdOperation::SetInteractionMode(primitive) => {
            ColdOperation::SetInteractionMode(primitive)
        }
        ColdOperation::SetInteractionModeValue { value, context } => {
            ColdOperation::SetInteractionModeValue { value, context }
        }
        ColdOperation::ItalicCorrection => ColdOperation::ItalicCorrection,
        ColdOperation::IllegalItalicCorrection { token } => {
            ColdOperation::IllegalItalicCorrection { token }
        }
        ColdOperation::NoBoundary { suppress_right } => {
            ColdOperation::NoBoundary { suppress_right }
        }
        ColdOperation::NonScript => ColdOperation::NonScript,
        ColdOperation::ControlSpace => ColdOperation::ControlSpace,
        ColdOperation::PrevDepth { value } => ColdOperation::PrevDepth { value },
        ColdOperation::IllegalPrevDepth { token } => ColdOperation::IllegalPrevDepth { token },
        ColdOperation::SpaceFactor { value } => ColdOperation::SpaceFactor { value },
        ColdOperation::IllegalSpaceFactor { token } => ColdOperation::IllegalSpaceFactor { token },
        ColdOperation::PrevGraf { value } => ColdOperation::PrevGraf { value },
        ColdOperation::PageDimension { dimension, value } => {
            ColdOperation::PageDimension { dimension, value }
        }
        ColdOperation::PageInteger { integer, value } => {
            ColdOperation::PageInteger { integer, value }
        }
        ColdOperation::FixedHorizontalGlue { primitive } => {
            ColdOperation::FixedHorizontalGlue { primitive }
        }
        ColdOperation::FixedVerticalGlue { primitive } => {
            ColdOperation::FixedVerticalGlue { primitive }
        }
        ColdOperation::ParagraphIndent { indent } => ColdOperation::ParagraphIndent { indent },
        ColdOperation::ParagraphShape { lines, global } => {
            ColdOperation::ParagraphShape { lines, global }
        }
        ColdOperation::PenaltyArray {
            kind,
            values,
            global,
        } => ColdOperation::PenaltyArray {
            kind,
            values,
            global,
        },
        ColdOperation::Toks {
            index,
            tokens: _,
            global,
        } => ColdOperation::Toks {
            index,
            tokens: cursor.token()?,
            global,
        },
        ColdOperation::IntParam {
            index,
            value,
            global,
        } => ColdOperation::IntParam {
            index,
            value,
            global,
        },
        ColdOperation::DimenParam {
            index,
            value,
            global,
        } => ColdOperation::DimenParam {
            index,
            value,
            global,
        },
        ColdOperation::TokParam {
            index,
            tokens,
            global,
        } => ColdOperation::TokParam {
            index,
            tokens: tokens.map(|_| cursor.token()).transpose()?,
            global,
        },
        ColdOperation::GlueParam {
            index,
            value,
            global,
        } => ColdOperation::GlueParam {
            index,
            value,
            global,
        },
        ColdOperation::CodeTable {
            primitive,
            character,
            value,
            global,
        } => ColdOperation::CodeTable {
            primitive,
            character,
            value,
            global,
        },
        ColdOperation::PdfFontCode {
            table,
            font,
            character,
            value,
        } => ColdOperation::PdfFontCode {
            table,
            font,
            character,
            value,
        },
        ColdOperation::PdfNoLigatures { font } => ColdOperation::PdfNoLigatures { font },
        ColdOperation::FontSelect {
            font,
            selector,
            global,
        } => ColdOperation::FontSelect {
            font,
            selector,
            global,
        },
        ColdOperation::FontDefinition {
            request,
            resource,
            global,
        } => ColdOperation::FontDefinition {
            request,
            resource,
            global,
        },
        ColdOperation::GeneratedFontDefinition { definition, global } => {
            ColdOperation::GeneratedFontDefinition { definition, global }
        }
        ColdOperation::InputStream { request, resource } => {
            let request = match request {
                RootedInputStreamRequest::Open {
                    stream,
                    scanned,
                    recovered,
                    file_name,
                } => RootedInputStreamRequest::Open {
                    stream,
                    scanned,
                    recovered,
                    file_name,
                },
                RootedInputStreamRequest::Close {
                    stream,
                    scanned,
                    recovered,
                } => RootedInputStreamRequest::Close {
                    stream,
                    scanned,
                    recovered,
                },
                RootedInputStreamRequest::Read {
                    stream,
                    target,
                    global,
                    tokens: _,
                    definition: _,
                } => {
                    let tokens = cursor.token()?;
                    let definition = definition_cursor.token()?;
                    RootedInputStreamRequest::Read {
                        stream,
                        target,
                        global,
                        tokens,
                        definition,
                    }
                }
            };
            ColdOperation::InputStream { request, resource }
        }
        ColdOperation::PdfXImage { request, resource } => ColdOperation::PdfXImage {
            request: prepare_pdf_image(request, &mut cursor)?,
            resource,
        },
        ColdOperation::PdfRefXImage { object } => ColdOperation::PdfRefXImage { object },
        ColdOperation::PdfSetRandomSeed { seed } => ColdOperation::PdfSetRandomSeed { seed },
        ColdOperation::PdfResetTimer => ColdOperation::PdfResetTimer,
        ColdOperation::PdfInterwordSpace(control) => ColdOperation::PdfInterwordSpace(control),
        ColdOperation::PdfRunningLink(enabled) => ColdOperation::PdfRunningLink(enabled),
        ColdOperation::PdfSpaceFont(_) => ColdOperation::PdfSpaceFont(cursor.token()?),
        ColdOperation::PdfGraphics(request) => {
            ColdOperation::PdfGraphics(prepare_pdf_graphics(request, &mut cursor)?)
        }
        ColdOperation::PdfObject(request) => {
            ColdOperation::PdfObject(prepare_pdf_object(request, &mut cursor)?)
        }
        ColdOperation::PdfReferenceObject(request) => ColdOperation::PdfReferenceObject(request),
        ColdOperation::PdfForm(request) => {
            ColdOperation::PdfForm(prepare_pdf_form(request, &mut cursor)?)
        }
        ColdOperation::PdfDocumentFragment(request) => {
            ColdOperation::PdfDocumentFragment(RootedPdfDocumentFragmentRequest {
                kind: request.kind,
                text: prepare_balanced(request.text, &mut cursor)?,
                open_action: request
                    .open_action
                    .map(|action| prepare_pdf_action(action, &mut cursor))
                    .transpose()?,
            })
        }
        ColdOperation::PdfNavigation(request) => {
            ColdOperation::PdfNavigation(prepare_pdf_navigation(request, &mut cursor)?)
        }
        ColdOperation::FontDimen {
            font,
            number,
            value,
            recovery_context,
        } => ColdOperation::FontDimen {
            font,
            number,
            value,
            recovery_context,
        },
        ColdOperation::FontInteger { font, skew, value } => {
            ColdOperation::FontInteger { font, skew, value }
        }
        ColdOperation::PdfFontExpand { font, spec } => ColdOperation::PdfFontExpand { font, spec },
        ColdOperation::PdfFontAction {
            primitive,
            font,
            first,
            second,
        } => ColdOperation::PdfFontAction {
            primitive,
            font,
            first: first.map(|_| cursor.token()).transpose()?,
            second: second.map(|_| cursor.token()).transpose()?,
        },
        ColdOperation::DeferredOpenOut { stream, file_name } => {
            ColdOperation::DeferredOpenOut { stream, file_name }
        }
        ColdOperation::DeferredCloseOut { stream } => ColdOperation::DeferredCloseOut { stream },
        ColdOperation::DeferredWrite { stream, tokens: _ } => ColdOperation::DeferredWrite {
            stream,
            tokens: cursor.token()?,
        },
        ColdOperation::DeferredSpecial {
            deferred,
            tokens: _,
        } => ColdOperation::DeferredSpecial {
            deferred,
            tokens: cursor.token()?,
        },
        ColdOperation::SetLanguage { language } => ColdOperation::SetLanguage { language },
        ColdOperation::IllegalSetLanguage { token } => ColdOperation::IllegalSetLanguage { token },
        ColdOperation::Arithmetic {
            primitive,
            target,
            operand,
            global,
        } => ColdOperation::Arithmetic {
            primitive,
            target,
            operand,
            global,
        },
        ColdOperation::InvalidArithmeticTarget { primitive, target } => {
            ColdOperation::InvalidArithmeticTarget { primitive, target }
        }
        ColdOperation::CharacterDefinition {
            primitive,
            target,
            provisional_old,
            value,
            global,
        } => ColdOperation::CharacterDefinition {
            primitive,
            target,
            provisional_old,
            value,
            global,
        },
        ColdOperation::HyphenationData {
            words,
            pattern_specs,
            patterns,
            rejection_context,
            trie_built,
        } => ColdOperation::HyphenationData {
            words,
            pattern_specs,
            patterns,
            rejection_context,
            trie_built,
        },
        ColdOperation::RegisterDefinition {
            primitive,
            target,
            provisional_old,
            index,
            global,
        } => ColdOperation::RegisterDefinition {
            primitive,
            target,
            provisional_old,
            index,
            global,
        },
        ColdOperation::AfterGroup(token) => ColdOperation::AfterGroup(token),
        ColdOperation::AfterAssignment(token) => ColdOperation::AfterAssignment(token),
        ColdOperation::Rule {
            width,
            height,
            depth,
            horizontal,
        } => ColdOperation::Rule {
            width,
            height,
            depth,
            horizontal,
        },
        ColdOperation::HRuleHereExceptLeaders => ColdOperation::HRuleHereExceptLeaders,
        ColdOperation::Message { tokens: _, error } => ColdOperation::Message {
            tokens: cursor.token()?,
            error,
        },
        ColdOperation::DisplayDiagnostic(diagnostic) => {
            ColdOperation::DisplayDiagnostic(diagnostic)
        }
        ColdOperation::ShowBox { index } => ColdOperation::ShowBox { index },
        ColdOperation::ShowLists => ColdOperation::ShowLists,
        ColdOperation::ShowTokens { tokens: _ } => ColdOperation::ShowTokens {
            tokens: cursor.token()?,
        },
        ColdOperation::ShowIfs { conditions } => ColdOperation::ShowIfs { conditions },
        ColdOperation::ShowGroups { diagnostic } => ColdOperation::ShowGroups { diagnostic },
        ColdOperation::VSplit(split) => ColdOperation::VSplit(split),
        ColdOperation::ImmediateExtension(request) => {
            ColdOperation::ImmediateExtension(prepare_immediate_extension(request, &mut cursor)?)
        }
        ColdOperation::BoxRegister {
            index,
            copy,
            ships_out,
        } => ColdOperation::BoxRegister {
            index,
            copy,
            ships_out,
        },
        ColdOperation::Unbox {
            primitive,
            index,
            error_context,
        } => ColdOperation::Unbox {
            primitive,
            index,
            error_context,
        },
        ColdOperation::SavedVerticalDiscards(primitive) => {
            ColdOperation::SavedVerticalDiscards(primitive)
        }
        ColdOperation::LastBox { error_context } => ColdOperation::LastBox { error_context },
        ColdOperation::Leaders {
            kind,
            payload,
            glue,
        } => ColdOperation::Leaders {
            kind,
            payload,
            glue,
        },
        ColdOperation::LeaderRegister {
            kind,
            index,
            copy,
            glue,
        } => ColdOperation::LeaderRegister {
            kind,
            index,
            copy,
            glue,
        },
        ColdOperation::MissingLeaderPayload => ColdOperation::MissingLeaderPayload,
        ColdOperation::LeadersNotFollowedByGlue => ColdOperation::LeadersNotFollowedByGlue,
        ColdOperation::BeginShipout => ColdOperation::BeginShipout,
        ColdOperation::BeginAlignment { vertical, owner } => {
            ColdOperation::BeginAlignment { vertical, owner }
        }
        ColdOperation::AlignmentPreambleOpening { alignment, packing } => {
            ColdOperation::AlignmentPreambleOpening { alignment, packing }
        }
        ColdOperation::AlignmentPreambleStart { alignment } => {
            ColdOperation::AlignmentPreambleStart { alignment }
        }
        ColdOperation::AlignmentCellOpening { alignment, opening } => {
            ColdOperation::AlignmentCellOpening { alignment, opening }
        }
        ColdOperation::AlignmentCellFinish { alignment } => {
            ColdOperation::AlignmentCellFinish { alignment }
        }
        ColdOperation::AlignmentFinish { alignment } => {
            ColdOperation::AlignmentFinish { alignment }
        }
        ColdOperation::BeginNoAlign { alignment } => ColdOperation::BeginNoAlign { alignment },
        ColdOperation::AlignmentRecovery { brace } => ColdOperation::AlignmentRecovery { brace },
        ColdOperation::BeginSimpleGroup => ColdOperation::BeginSimpleGroup,
        ColdOperation::EndSimpleGroup => ColdOperation::EndSimpleGroup,
        ColdOperation::ExtraRightBrace { forgotten } => {
            ColdOperation::ExtraRightBrace { forgotten }
        }
        ColdOperation::EndMathGroup(kind) => ColdOperation::EndMathGroup(kind),
        ColdOperation::OffSave(closer) => ColdOperation::OffSave(closer),
        ColdOperation::OffSaveBottomDrop { token } => ColdOperation::OffSaveBottomDrop { token },
        ColdOperation::OutputRoutineOpeningBrace => ColdOperation::OutputRoutineOpeningBrace,
        ColdOperation::EndOutputRoutine => ColdOperation::EndOutputRoutine,
        ColdOperation::AlignmentPeekCell { alignment, omit } => {
            ColdOperation::AlignmentPeekCell { alignment, omit }
        }
        ColdOperation::NoAlignEndGroup { alignment } => {
            ColdOperation::NoAlignEndGroup { alignment }
        }
        ColdOperation::SetBox { target, path } => ColdOperation::SetBox { target, path },
        ColdOperation::BeginBox(construction) => ColdOperation::BeginBox(construction),
        ColdOperation::BeginLeaderBox { construction, kind } => {
            ColdOperation::BeginLeaderBox { construction, kind }
        }
        ColdOperation::BoxShift(shift) => ColdOperation::BoxShift(shift),
        ColdOperation::IllegalBoxShift { token } => ColdOperation::IllegalBoxShift { token },
        ColdOperation::BeginInsert(construction) => ColdOperation::BeginInsert(construction),
        ColdOperation::IllegalInsertOrAdjust { token } => {
            ColdOperation::IllegalInsertOrAdjust { token }
        }
        ColdOperation::IllegalEqNo { token } => ColdOperation::IllegalEqNo { token },
        ColdOperation::IllegalHAlign { token } => ColdOperation::IllegalHAlign { token },
        ColdOperation::IllegalLastItem { token, context } => {
            ColdOperation::IllegalLastItem { token, context }
        }
        ColdOperation::BoxEndGroup { ships_out } => ColdOperation::BoxEndGroup { ships_out },
        ColdOperation::Mark { class, tokens: _ } => ColdOperation::Mark {
            class,
            tokens: cursor.token()?,
        },
        ColdOperation::Paragraph => ColdOperation::Paragraph,
        ColdOperation::MathShift { pairing } => ColdOperation::MathShift { pairing },
        ColdOperation::ParagraphStart => ColdOperation::ParagraphStart,
        ColdOperation::Character {
            ch,
            cat,
            origin,
            suppress_left_boundary,
        } => ColdOperation::Character {
            ch,
            cat,
            origin,
            suppress_left_boundary,
        },
        ColdOperation::Accent(accent) => ColdOperation::Accent(accent),
        ColdOperation::DiscretionaryOpening(opening) => {
            ColdOperation::DiscretionaryOpening(opening)
        }
        ColdOperation::DiscretionaryPartEnd => ColdOperation::DiscretionaryPartEnd,
        ColdOperation::DiscretionaryHyphen { origin } => {
            ColdOperation::DiscretionaryHyphen { origin }
        }
    };
    cursor.finish()?;
    definition_cursor.finish()?;
    Ok(prepared)
}

impl<G> ColdOperation<G> {
    /// Collects every attempt-local token root in deterministic field order.
    /// The outer preparation barrier passes this exact sequence to
    /// `CommandState::promote_attempt_roots`; reconstruction consumes the
    /// receipt in the same order.
    pub(in crate::main_control) fn attempt_token_roots(
        &self,
        roots: &mut Vec<tex_command::AttemptTokenListId>,
    ) {
        match self {
            Self::Toks { tokens, .. }
            | Self::PdfSpaceFont(tokens)
            | Self::DeferredWrite { tokens, .. }
            | Self::DeferredSpecial { tokens, .. }
            | Self::Message { tokens, .. }
            | Self::ShowTokens { tokens }
            | Self::Mark { tokens, .. } => roots.push(*tokens),
            Self::TokParam {
                tokens: Some(tokens),
                ..
            } => roots.push(*tokens),
            Self::PdfFontAction { first, second, .. } => {
                roots.extend(first);
                roots.extend(second);
            }
            Self::InputStream { request, .. } => input_stream_attempt_roots(request, roots),
            Self::PdfXImage { request, .. } => {
                roots.extend(request.attr);
            }
            Self::PdfGraphics(request) => pdf_graphics_attempt_roots(request, roots),
            Self::PdfObject(request) => pdf_object_attempt_roots(request, roots),
            Self::PdfForm(request) => pdf_form_attempt_roots(request, roots),
            Self::PdfDocumentFragment(request) => {
                roots.push(request.text.tokens);
                if let Some(action) = &request.open_action {
                    pdf_action_attempt_roots(action, roots);
                }
            }
            Self::PdfNavigation(request) => pdf_navigation_attempt_roots(request, roots),
            Self::ImmediateExtension(request) => immediate_extension_attempt_roots(request, roots),
            _ => {}
        }
    }

    pub(in crate::main_control) const fn fires_afterassignment(&self) -> bool {
        matches!(
            self,
            Self::Count { .. }
                | Self::Dimen { .. }
                | Self::BoxDimensionAssignment { .. }
                | Self::Skip { .. }
                | Self::Muskip { .. }
                | Self::Toks { .. }
                | Self::IntParam { .. }
                | Self::DimenParam { .. }
                | Self::TokParam { .. }
                | Self::GlueParam { .. }
                | Self::CodeTable { .. }
                | Self::PdfFontCode { .. }
                | Self::PdfNoLigatures { .. }
                | Self::FontDimen { .. }
                | Self::FontInteger { .. }
                | Self::FontDefinition { .. }
                | Self::GeneratedFontDefinition { .. }
                | Self::InputStream { .. }
                | Self::Arithmetic { .. }
                | Self::InvalidArithmeticTarget { .. }
                | Self::CharacterDefinition { .. }
                | Self::RegisterDefinition { .. }
                | Self::ParagraphShape { .. }
                | Self::PenaltyArray { .. }
                | Self::FontSelect { .. }
                | Self::MathFamily { .. }
                | Self::SetBox { .. }
                | Self::PrevDepth { .. }
                | Self::SpaceFactor { .. }
                | Self::PrevGraf { .. }
                | Self::PageDimension { .. }
                | Self::PageInteger { .. }
                | Self::HyphenationData { .. }
                | Self::SetInteractionMode(..)
        )
    }

    pub(in crate::main_control) fn main_loop_character(&self) -> Option<char> {
        match *self {
            Self::Character {
                ch,
                cat: Catcode::Letter | Catcode::Other,
                ..
            } => Some(ch),
            Self::CharacterCode { value, .. } => u32::try_from(value).ok().and_then(char::from_u32),
            _ => None,
        }
    }

    /// Collects attempt-local macro definitions which must be promoted in
    /// the same validated batch as their token-list children.
    pub(in crate::main_control) fn attempt_definition_roots(
        &self,
        roots: &mut Vec<tex_command::AttemptDefinitionId>,
    ) {
        if let Self::InputStream { request, .. } = self {
            input_stream_attempt_definition_roots(request, roots);
        }
    }
}

fn balanced_attempt_root<T: Copy>(text: &RootedBalancedText<T>, roots: &mut Vec<T>) {
    roots.push(text.tokens);
}

fn pdf_identifier_attempt_roots<T: Copy>(
    identifier: &RootedPdfActionIdentifier<T>,
    roots: &mut Vec<T>,
) {
    match identifier {
        RootedPdfActionIdentifier::Name(tokens) | RootedPdfActionIdentifier::Raw(tokens) => {
            roots.push(*tokens);
        }
        RootedPdfActionIdentifier::Number(_) => {}
    }
}

fn pdf_action_attempt_roots<T: Copy>(action: &RootedPdfActionSpec<T>, roots: &mut Vec<T>) {
    match action {
        RootedPdfActionSpec::User(tokens) => roots.push(*tokens),
        RootedPdfActionSpec::GoTo(destination) | RootedPdfActionSpec::Thread(destination) => {
            roots.extend(destination.file);
            if let Some(identifier) = &destination.structure {
                pdf_identifier_attempt_roots(identifier, roots);
            }
            match &destination.target {
                RootedPdfActionTarget::Page { view, .. } => roots.push(*view),
                RootedPdfActionTarget::Destination(identifier) => {
                    pdf_identifier_attempt_roots(identifier, roots);
                }
            }
        }
    }
}

fn pdf_graphics_attempt_roots<T: Copy>(request: &RootedPdfGraphicsRequest<T>, roots: &mut Vec<T>) {
    match request {
        RootedPdfGraphicsRequest::Literal { text, .. }
        | RootedPdfGraphicsRequest::SetMatrix { text } => {
            balanced_attempt_root(text, roots);
        }
        RootedPdfGraphicsRequest::ColorStack {
            action:
                Some(RootedPdfColorStackAction::Set(text) | RootedPdfColorStackAction::Push(text)),
            ..
        } => balanced_attempt_root(text, roots),
        _ => {}
    }
}

fn pdf_object_attempt_roots<T: Copy>(request: &RootedPdfObjectRequest<T>, roots: &mut Vec<T>) {
    if let RootedPdfObjectRequest::Define {
        stream_attr, data, ..
    } = request
    {
        if let Some(text) = stream_attr {
            balanced_attempt_root(text, roots);
        }
        balanced_attempt_root(data, roots);
    }
}

fn pdf_form_attempt_roots<T: Copy>(request: &RootedPdfFormRequest<T>, roots: &mut Vec<T>) {
    if let RootedPdfFormRequest::Create {
        attr, resources, ..
    } = request
    {
        if let Some(text) = attr {
            balanced_attempt_root(text, roots);
        }
        if let Some(text) = resources {
            balanced_attempt_root(text, roots);
        }
    }
}

fn pdf_navigation_attempt_roots<T: Copy>(
    request: &RootedPdfNavigationRequest<T>,
    roots: &mut Vec<T>,
) {
    match request {
        RootedPdfNavigationRequest::Annotation(RootedPdfAnnotationRequest::Define {
            entries,
            ..
        }) => {
            balanced_attempt_root(entries, roots);
        }
        RootedPdfNavigationRequest::StartLink(request) => {
            if let Some(text) = &request.attributes {
                balanced_attempt_root(text, roots);
            }
            pdf_action_attempt_roots(&request.action, roots);
        }
        RootedPdfNavigationRequest::Outline(request) => {
            if let Some(text) = &request.attributes {
                balanced_attempt_root(text, roots);
            }
            pdf_action_attempt_roots(&request.action, roots);
            balanced_attempt_root(&request.title, roots);
        }
        RootedPdfNavigationRequest::Destination(request) => {
            pdf_identifier_attempt_roots(&request.identifier, roots);
        }
        RootedPdfNavigationRequest::Thread(request) => {
            if let Some(text) = &request.attributes {
                balanced_attempt_root(text, roots);
            }
            pdf_identifier_attempt_roots(&request.identifier, roots);
        }
        RootedPdfNavigationRequest::Annotation(RootedPdfAnnotationRequest::Reserve)
        | RootedPdfNavigationRequest::EndLink
        | RootedPdfNavigationRequest::EndThread => {}
    }
}

fn immediate_extension_attempt_roots<T: Copy>(
    request: &RootedImmediateExtension<T>,
    roots: &mut Vec<T>,
) {
    match request {
        RootedImmediateExtension::Write { tokens, .. } => roots.push(*tokens),
        RootedImmediateExtension::PdfObject(request) => pdf_object_attempt_roots(request, roots),
        RootedImmediateExtension::PdfForm(request) => pdf_form_attempt_roots(request, roots),
        RootedImmediateExtension::PdfImage(request) => roots.extend(request.attr),
        RootedImmediateExtension::Continue
        | RootedImmediateExtension::PdfExtensionInDviMode(_)
        | RootedImmediateExtension::OpenOut { .. }
        | RootedImmediateExtension::CloseOut { .. } => {}
    }
}

fn input_stream_attempt_roots<T: Copy, D, S>(
    request: &RootedInputStreamRequest<T, D, S>,
    roots: &mut Vec<T>,
) {
    if let RootedInputStreamRequest::Read { tokens, .. } = request {
        roots.push(*tokens);
    }
}

fn input_stream_attempt_definition_roots<T, D: Copy, S>(
    request: &RootedInputStreamRequest<T, D, S>,
    roots: &mut Vec<D>,
) {
    if let RootedInputStreamRequest::Read { definition, .. } = request {
        roots.push(*definition);
    }
}

/// A completed assignable quantity selector.  It is intentionally a semantic
/// selector, never a delivered command or a raw input handle.
#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticTarget {
    IntegerRegister(u16),
    DimensionRegister(u16),
    GlueRegister { index: u16, mu: bool },
    IntegerParameter(u16),
    DimensionParameter(u16),
    GlueParameter { index: u16, mu: bool },
}

#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticOperand {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    fn text(tokens: u8) -> RootedBalancedText<u8> {
        RootedBalancedText {
            tokens,
            provenance: tex_command::StructuredProvenance {
                primary: tex_state::token::OriginId::UNKNOWN,
            },
        }
    }

    #[test]
    fn nested_navigation_roots_follow_reconstruction_order() {
        let request = RootedPdfNavigationRequest::Outline(RootedPdfOutlineRequest {
            attributes: Some(text(1)),
            action: RootedPdfActionSpec::GoTo(RootedPdfActionDestination {
                file: Some(2),
                structure: Some(RootedPdfActionIdentifier::Name(3)),
                target: RootedPdfActionTarget::Page { number: 7, view: 4 },
                window: tex_state::PdfActionWindow::New,
            }),
            count: -2,
            title: text(5),
        });
        let mut roots = Vec::new();
        pdf_navigation_attempt_roots(&request, &mut roots);
        assert_eq!(roots, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn immediate_pdf_and_input_roots_are_structural() {
        let immediate = RootedImmediateExtension::PdfObject(RootedPdfObjectRequest::Define {
            use_object: None,
            stream: true,
            stream_attr: Some(text(6)),
            file: false,
            data: text(7),
        });
        let input = RootedInputStreamRequest::Read {
            stream: 3,
            target: (),
            global: false,
            tokens: 8,
            definition: 9,
        };
        let mut roots = Vec::new();
        immediate_extension_attempt_roots(&immediate, &mut roots);
        input_stream_attempt_roots(&input, &mut roots);
        assert_eq!(roots, [6, 7, 8]);
        let mut definitions = Vec::new();
        input_stream_attempt_definition_roots(&input, &mut definitions);
        assert_eq!(definitions, [9]);
    }

    #[test]
    fn promotion_cursor_rejects_underflow_and_remainder() {
        let mut short = PromotionCursor::new(Vec::<u8>::new());
        assert!(matches!(
            short.token(),
            Err(ColdPreparationError::ReceiptUnderflow)
        ));

        let extra = PromotionCursor::new(vec![1_u8, 2]);
        assert!(matches!(
            extra.finish(),
            Err(ColdPreparationError::ReceiptRemainder { remaining: 2 })
        ));
    }
}

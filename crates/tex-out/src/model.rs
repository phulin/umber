use crate::ContentHash;
pub use tex_arith::GlueSetRatio;
use tex_arith::Scaled;

/// Detached page data that has not yet crossed the validation boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnvalidatedPageArtifact {
    pub job: JobInfo,
    pub fonts: Vec<FontResource>,
    pub counts: [i32; 10],
    pub root: PageNode,
    pub effects: Vec<PageEffect>,
}

/// A detached page artifact whose references and traversal budgets were validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageArtifact(UnvalidatedPageArtifact);

impl PageArtifact {
    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::SerializeError> {
        self.to_bytes_with_limits(crate::ArtifactCodecLimits::default())
    }

    pub fn to_bytes_with_limits(
        &self,
        limits: crate::ArtifactCodecLimits,
    ) -> Result<Vec<u8>, crate::SerializeError> {
        crate::binary::to_bytes(self, limits)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::ParseError> {
        Self::from_bytes_with_limits(bytes, crate::ArtifactCodecLimits::default())
    }

    pub fn from_bytes_with_limits(
        bytes: &[u8],
        limits: crate::ArtifactCodecLimits,
    ) -> Result<Self, crate::ParseError> {
        crate::binary::from_bytes(bytes, limits)?
            .validate()
            .map_err(Into::into)
    }

    pub fn content_hash(&self) -> Result<ContentHash, crate::SerializeError> {
        Ok(ContentHash::for_domain(
            crate::ContentDomain::Artifact,
            &self.to_bytes()?,
        ))
    }

    #[cfg(test)]
    pub(crate) fn testing_mut(&mut self) -> &mut UnvalidatedPageArtifact {
        &mut self.0
    }
}

impl std::ops::Deref for PageArtifact {
    type Target = UnvalidatedPageArtifact;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
impl std::ops::DerefMut for PageArtifact {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl UnvalidatedPageArtifact {
    pub fn validate(self) -> Result<PageArtifact, ArtifactValidationError> {
        self.validate_with_limits(ArtifactValidationLimits::default())
    }

    pub fn validate_with_limits(
        self,
        limits: ArtifactValidationLimits,
    ) -> Result<PageArtifact, ArtifactValidationError> {
        validate_artifact(&self, limits)?;
        Ok(PageArtifact(self))
    }
}

/// Builder that cannot publish its fields without validation.
pub struct PageArtifactBuilder(UnvalidatedPageArtifact);

impl PageArtifactBuilder {
    #[must_use]
    pub fn new(artifact: UnvalidatedPageArtifact) -> Self {
        Self(artifact)
    }

    pub fn build(self) -> Result<PageArtifact, ArtifactValidationError> {
        self.0.validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactValidationLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_fonts: usize,
    pub max_effects: usize,
}

impl Default for ArtifactValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 1_000_000,
            max_depth: 4096,
            max_fonts: 65_536,
            max_effects: 1_000_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactValidationError {
    RootNotBox,
    TooManyFonts { count: usize, limit: usize },
    TooManyEffects { count: usize, limit: usize },
    TooManyNodes { count: usize, limit: usize },
    NestingTooDeep { depth: usize, limit: usize },
    DuplicateFont { font_id: u32 },
    EmptyFontName { font_id: u32 },
    InvalidFontSize { font_id: u32 },
    MissingFont { font_id: u32 },
    MissingFontSource { font_id: u32, source_font_id: u32 },
    FontSourceIdentityMismatch { font_id: u32, source_font_id: u32 },
    MissingEffect { effect_index: u32 },
    CharacterOutOfRange { ch: u32 },
    InvalidLigatureSourceLength { count: usize },
    InvalidTokenScalar { ch: u32 },
    InvalidStream { stream: u8 },
}

impl std::fmt::Display for ArtifactValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid page artifact: {self:?}")
    }
}

impl std::error::Error for ArtifactValidationError {}

/// Job-level data captured at shipout for downstream output drivers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobInfo {
    pub mag: i32,
    pub banner: String,
    pub h_offset: Scaled,
    pub v_offset: Scaled,
}

impl Default for JobInfo {
    fn default() -> Self {
        Self {
            mag: 1000,
            banner: DEFAULT_BANNER.to_owned(),
            h_offset: Scaled::from_raw(0),
            v_offset: Scaled::from_raw(0),
        }
    }
}

pub const DEFAULT_BANNER: &str = "  Umber DVI 1970.01.01:0000";

/// A font resource referenced by the page tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontResource {
    pub font_id: u32,
    pub name: String,
    pub tfm_content_hash: ContentHash,
    pub tfm_checksum: u32,
    pub design_size: Scaled,
    pub at_size: Scaled,
    pub opentype: Option<OpenTypeFontResource>,
    pub semantic_identity: tex_fonts::FontSourceIdentity,
    pub construction: FontResourceConstruction,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FontResourceConstruction {
    Loaded,
    Copied {
        source_font_id: u32,
        source_identity: tex_fonts::FontSourceIdentity,
    },
    Letterspaced {
        source_font_id: u32,
        source_identity: tex_fonts::FontSourceIdentity,
        amount: i16,
        no_ligatures: bool,
    },
    Expanded {
        source_font_id: u32,
        source_identity: tex_fonts::FontSourceIdentity,
        ratio: i16,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenTypeFontResource {
    pub program_identity: tex_fonts::FontProgramIdentity,
    pub object_identity: tex_fonts::FontObjectIdentity,
    pub instance_identity: tex_fonts::FontInstanceIdentity,
    pub container: tex_fonts::FontContainer,
}

/// A driver-facing shipped node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageNode {
    Char {
        font_id: u32,
        ch: u32,
        width: Scaled,
    },
    Lig {
        font_id: u32,
        ch: u32,
        /// Complete source-code provenance consumed to form this glyph.
        source: Vec<u32>,
        width: Scaled,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: GlueSpec,
        kind: GlueKind,
        leader: Option<LeaderPayload>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode),
    VList(BoxNode),
    Disc {
        kind: DiscKind,
        pre: Vec<PageNode>,
        post: Vec<PageNode>,
        replace: Vec<PageNode>,
    },
    Mark {
        class: u16,
        tokens: Vec<PageToken>,
    },
    Insert {
        class: u16,
        content: Vec<PageNode>,
    },
    WhatsitAnchor {
        effect_index: u32,
    },
    MathOn(Scaled),
    MathOff(Scaled),
    Adjust(Vec<PageNode>),
}

/// A shipped hlist/vlist payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoxNode {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    pub glue_set: GlueSetRatio,
    pub glue_sign: GlueSign,
    pub glue_order: GlueOrder,
    pub children: Vec<PageNode>,
}

/// A concrete glue specification, lowered out of the live glue store.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlueSpec {
    pub width: Scaled,
    pub stretch: Scaled,
    pub stretch_order: GlueOrder,
    pub shrink: Scaled,
    pub shrink_order: GlueOrder,
}

/// Repeated material attached to a leader glue node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaderPayload {
    HList(BoxNode),
    VList(BoxNode),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueOrder {
    Normal,
    Fil,
    Fill,
    Filll,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueSign {
    Normal,
    Stretching,
    Shrinking,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KernKind {
    Explicit,
    Font,
    Auto,
    Accent,
    LeftMargin,
    RightMargin,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlueKind {
    Normal,
    BaselineSkip,
    LineSkip,
    LeftSkip,
    RightSkip,
    ParFillSkip,
    Leaders,
    Cleaders,
    Xleaders,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DiscKind {
    Discretionary,
    ExplicitHyphen,
    AutomaticHyphen,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum PageToken {
    Char { ch: u32, cat: TokenCatcode },
    ControlSequence(String),
    ActiveControlSequence(u32),
    Param(u8),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenCatcode {
    Escape,
    BeginGroup,
    EndGroup,
    MathShift,
    AlignmentTab,
    EndLine,
    Parameter,
    Superscript,
    Subscript,
    Ignored,
    Space,
    Letter,
    Other,
    Active,
    Comment,
    Invalid,
}

/// One committed side-effect payload associated with the page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageEffect {
    OpenOut {
        stream: u8,
        path: String,
    },
    CloseOut {
        stream: u8,
    },
    Write {
        sink: EffectSink,
        text: String,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    PdfAccessibility(PdfAccessibilityEffect),
    PdfAnnotation(PdfAnnotationEffect),
    PdfLiteral {
        mode: PdfLiteralMode,
        payload: Vec<u8>,
    },
    PdfSetMatrix {
        payload: Vec<u8>,
    },
    PdfSave,
    PdfRestore,
    PdfColorStack {
        mode: PdfLiteralMode,
        payload: Vec<u8>,
        page_start: bool,
    },
    PdfSavePosition,
    PdfSnapState {
        x: Scaled,
        y: Scaled,
    },
    PdfSnapRefPoint,
    PdfSnapY {
        spec: GlueSpec,
    },
    PdfSnapYComp {
        ratio: u16,
    },
    PdfRefXForm {
        object: u32,
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    PdfRefXImage {
        object: u32,
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
}

/// Ordered PDF-only accessibility control retained at its shipped position.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfAccessibilityEffect {
    InterwordSpaceOn,
    InterwordSpaceOff,
    FakeSpace,
}

/// Ordered typed annotation/link marker retained at its shipped position.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PdfAnnotationEffect {
    Annotation { object: u32 },
    LinkStart { object: u32 },
    LinkEnd { object: u32 },
    RunningLink(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfLiteralMode {
    Origin,
    Page,
    Direct,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EffectSink {
    Terminal,
    Log,
    TerminalAndLog,
    Stream(u8),
}

fn validate_artifact(
    artifact: &UnvalidatedPageArtifact,
    limits: ArtifactValidationLimits,
) -> Result<(), ArtifactValidationError> {
    if !matches!(artifact.root, PageNode::HList(_) | PageNode::VList(_)) {
        return Err(ArtifactValidationError::RootNotBox);
    }
    if artifact.fonts.len() > limits.max_fonts {
        return Err(ArtifactValidationError::TooManyFonts {
            count: artifact.fonts.len(),
            limit: limits.max_fonts,
        });
    }
    if artifact.effects.len() > limits.max_effects {
        return Err(ArtifactValidationError::TooManyEffects {
            count: artifact.effects.len(),
            limit: limits.max_effects,
        });
    }

    let mut font_ids = std::collections::BTreeSet::new();
    let mut font_identities = std::collections::BTreeMap::new();
    for font in &artifact.fonts {
        if !font_ids.insert(font.font_id) {
            return Err(ArtifactValidationError::DuplicateFont {
                font_id: font.font_id,
            });
        }
        if font.name.is_empty() {
            return Err(ArtifactValidationError::EmptyFontName {
                font_id: font.font_id,
            });
        }
        if font.design_size.raw() <= 0 || font.at_size.raw() <= 0 {
            return Err(ArtifactValidationError::InvalidFontSize {
                font_id: font.font_id,
            });
        }
        font_identities.insert(font.font_id, font.semantic_identity);
    }
    for font in &artifact.fonts {
        let source = match font.construction {
            FontResourceConstruction::Loaded => None,
            FontResourceConstruction::Copied {
                source_font_id,
                source_identity,
            }
            | FontResourceConstruction::Letterspaced {
                source_font_id,
                source_identity,
                ..
            }
            | FontResourceConstruction::Expanded {
                source_font_id,
                source_identity,
                ..
            } => Some((source_font_id, source_identity)),
        };
        if let Some((source_font_id, source_identity)) = source {
            let Some(actual) = font_identities.get(&source_font_id) else {
                return Err(ArtifactValidationError::MissingFontSource {
                    font_id: font.font_id,
                    source_font_id,
                });
            };
            if *actual != source_identity {
                return Err(ArtifactValidationError::FontSourceIdentityMismatch {
                    font_id: font.font_id,
                    source_font_id,
                });
            }
        }
    }
    for effect in &artifact.effects {
        let stream = match effect {
            PageEffect::OpenOut { stream, .. } | PageEffect::CloseOut { stream } => Some(*stream),
            PageEffect::Write {
                sink: EffectSink::Stream(stream),
                ..
            } => Some(*stream),
            PageEffect::Write { .. }
            | PageEffect::Special { .. }
            | PageEffect::PdfAccessibility(_) => None,
            PageEffect::PdfAnnotation(_)
            | PageEffect::PdfLiteral { .. }
            | PageEffect::PdfSetMatrix { .. }
            | PageEffect::PdfSave
            | PageEffect::PdfRestore => None,
            PageEffect::PdfColorStack { .. } => None,
            PageEffect::PdfSavePosition
            | PageEffect::PdfSnapState { .. }
            | PageEffect::PdfSnapRefPoint
            | PageEffect::PdfSnapY { .. }
            | PageEffect::PdfSnapYComp { .. } => None,
            PageEffect::PdfRefXForm { .. } | PageEffect::PdfRefXImage { .. } => None,
        };
        if stream.is_some_and(|stream| stream >= 16) {
            return Err(ArtifactValidationError::InvalidStream {
                stream: stream.expect("checked Some"),
            });
        }
    }

    let mut stack = vec![(&artifact.root, 1usize)];
    let mut count = 0usize;
    while let Some((node, depth)) = stack.pop() {
        count = count.saturating_add(1);
        if count > limits.max_nodes {
            return Err(ArtifactValidationError::TooManyNodes {
                count,
                limit: limits.max_nodes,
            });
        }
        if depth > limits.max_depth {
            return Err(ArtifactValidationError::NestingTooDeep {
                depth,
                limit: limits.max_depth,
            });
        }
        match node {
            PageNode::Char { font_id, ch, .. } => {
                validate_font_and_char(&font_ids, *font_id, *ch)?;
            }
            PageNode::Lig {
                font_id,
                ch,
                source,
                ..
            } => {
                validate_font_and_char(&font_ids, *font_id, *ch)?;
                if source.is_empty() || source.len() > 63 {
                    return Err(ArtifactValidationError::InvalidLigatureSourceLength {
                        count: source.len(),
                    });
                }
                for source in source {
                    validate_character(*source)?;
                }
            }
            PageNode::HList(box_node) | PageNode::VList(box_node) => {
                push_nodes(&mut stack, &box_node.children, depth + 1);
            }
            PageNode::Disc {
                pre, post, replace, ..
            } => {
                push_nodes(&mut stack, pre, depth + 1);
                push_nodes(&mut stack, post, depth + 1);
                push_nodes(&mut stack, replace, depth + 1);
            }
            PageNode::Insert { content, .. } | PageNode::Adjust(content) => {
                push_nodes(&mut stack, content, depth + 1);
            }
            PageNode::Glue {
                leader: Some(LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node)),
                ..
            } => push_nodes(&mut stack, &box_node.children, depth + 1),
            PageNode::WhatsitAnchor { effect_index } => {
                let index = usize::try_from(*effect_index).unwrap_or(usize::MAX);
                if index >= artifact.effects.len() {
                    return Err(ArtifactValidationError::MissingEffect {
                        effect_index: *effect_index,
                    });
                }
            }
            PageNode::Mark { tokens, .. } => {
                for token in tokens {
                    match token {
                        PageToken::Char { ch, .. } | PageToken::ActiveControlSequence(ch)
                            if char::from_u32(*ch).is_none() =>
                        {
                            return Err(ArtifactValidationError::InvalidTokenScalar { ch: *ch });
                        }
                        PageToken::Param(slot) if !(1..=9).contains(slot) => {
                            return Err(ArtifactValidationError::InvalidTokenScalar {
                                ch: u32::from(*slot),
                            });
                        }
                        _ => {}
                    }
                }
            }
            PageNode::Kern { .. }
            | PageNode::Penalty(_)
            | PageNode::Rule { .. }
            | PageNode::Glue { .. }
            | PageNode::MathOn(_)
            | PageNode::MathOff(_) => {}
        }
    }
    Ok(())
}

fn validate_font_and_char(
    fonts: &std::collections::BTreeSet<u32>,
    font_id: u32,
    ch: u32,
) -> Result<(), ArtifactValidationError> {
    if !fonts.contains(&font_id) {
        return Err(ArtifactValidationError::MissingFont { font_id });
    }
    validate_character(ch)
}

fn validate_character(ch: u32) -> Result<(), ArtifactValidationError> {
    if ch > u8::MAX.into() {
        return Err(ArtifactValidationError::CharacterOutOfRange { ch });
    }
    Ok(())
}

fn push_nodes<'a>(stack: &mut Vec<(&'a PageNode, usize)>, nodes: &'a [PageNode], depth: usize) {
    stack.extend(nodes.iter().rev().map(|node| (node, depth)));
}

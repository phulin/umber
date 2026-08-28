//! Immutable TeX node model.

use crate::glue::{GlueSpec, Order};
use crate::ids::FontId;
use crate::math::{MathChoice, MathFraction, MathListNode, MathNoad, MathStyle};
use crate::node_arena::PageListId;
use crate::scaled::{GlueSetRatio, Scaled};
use crate::token::{OriginId, TokenWord};
use crate::world::{PrintSink, StreamSlot};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Node-owned token payload used before and inside node arenas.
///
/// Stored token-list transitions clone the existing generation-branded
/// non-atomic owner, so the words remain in their final immutable allocation.
/// Standalone construction is reserved for detached/cold values and tests.
#[derive(Debug, Default)]
pub struct NodeTokenList {
    words: Option<Rc<[TokenWord]>>,
    accounting: Option<crate::memory_accounting::MemoryAccounting>,
}

impl Clone for NodeTokenList {
    fn clone(&self) -> Self {
        Self {
            words: self.words.as_ref().map(Rc::clone),
            accounting: self.accounting.clone(),
        }
    }
}

impl Drop for NodeTokenList {
    fn drop(&mut self) {
        if let Some(words) = &self.words
            && Rc::strong_count(words) == 1
            && let Some(accounting) = &self.accounting
        {
            accounting
                .release_shared_dynamic(words.len().checked_add(1).expect("token word count"));
        }
    }
}

impl PartialEq for NodeTokenList {
    fn eq(&self, other: &Self) -> bool {
        self.words() == other.words()
    }
}

impl Eq for NodeTokenList {}

impl Hash for NodeTokenList {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.words().hash(state);
    }
}

impl serde::Serialize for NodeTokenList {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(
            &self
                .words()
                .iter()
                .map(|word| word.raw())
                .collect::<Vec<_>>(),
            serializer,
        )
    }
}

impl<'de> serde::Deserialize<'de> for NodeTokenList {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let words = <Vec<u32> as serde::Deserialize>::deserialize(deserializer)?
            .into_iter()
            .map(TokenWord::from_raw)
            .collect::<Vec<_>>();
        Ok(Self::new(words))
    }
}

fn serialize_font_id<S: serde::Serializer>(
    font: &FontId,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_u32(font.raw())
}

fn deserialize_font_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<FontId, D::Error> {
    Ok(FontId::new(<u32 as serde::Deserialize>::deserialize(
        deserializer,
    )?))
}

impl NodeTokenList {
    #[must_use]
    pub fn new(words: impl Into<Box<[TokenWord]>>) -> Self {
        let words = words.into();
        Self {
            words: (!words.is_empty()).then(|| Rc::from(words)),
            accounting: None,
        }
    }

    pub(crate) fn shared(
        words: Rc<[TokenWord]>,
        accounting: crate::memory_accounting::MemoryAccounting,
    ) -> Self {
        Self {
            words: Some(words),
            accounting: Some(accounting),
        }
    }

    #[must_use]
    pub fn words(&self) -> &[TokenWord] {
        self.words.as_deref().unwrap_or(&[])
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words().is_empty()
    }
}

/// Stable logical node kinds shared by owned and compact node views.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum NodeKind {
    Char,
    Lig,
    Kern,
    MarginKern,
    Glue,
    Penalty,
    Rule,
    HList,
    VList,
    Unset,
    Disc,
    Mark,
    Ins,
    Whatsit,
    MathOn,
    MathOff,
    Direction,
    MathNoad,
    FractionNoad,
    MathStyle,
    MathChoice,
    MathList,
    Nonscript,
    Adjust,
}

impl NodeKind {
    pub const ALL: [Self; 24] = [
        Self::Char,
        Self::Lig,
        Self::Kern,
        Self::MarginKern,
        Self::Glue,
        Self::Penalty,
        Self::Rule,
        Self::HList,
        Self::VList,
        Self::Unset,
        Self::Disc,
        Self::Mark,
        Self::Ins,
        Self::Whatsit,
        Self::MathOn,
        Self::MathOff,
        Self::Direction,
        Self::MathNoad,
        Self::FractionNoad,
        Self::MathStyle,
        Self::MathChoice,
        Self::MathList,
        Self::Nonscript,
        Self::Adjust,
    ];

    #[must_use]
    pub const fn etex_type(self) -> i32 {
        match self {
            Self::Char => 0,
            Self::HList => 1,
            Self::VList => 2,
            Self::Rule => 3,
            Self::Ins => 4,
            Self::Mark => 5,
            Self::Adjust => 6,
            Self::Lig => 7,
            Self::Disc => 8,
            Self::Whatsit => 9,
            Self::MathOn | Self::MathOff | Self::Direction => 10,
            Self::Glue | Self::Nonscript => 11,
            Self::Kern | Self::MarginKern => 12,
            Self::Penalty => 13,
            Self::Unset => 14,
            Self::MathNoad
            | Self::FractionNoad
            | Self::MathStyle
            | Self::MathChoice
            | Self::MathList => 15,
        }
    }
}

/// A frozen TeX node.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "List: serde::Serialize, Glue: serde::Serialize, Tokens: serde::Serialize",
    deserialize = "List: serde::Deserialize<'de>, Glue: serde::Deserialize<'de>, Tokens: serde::Deserialize<'de>"
))]
pub enum Node<List = PageListId, Glue = GlueSpec, Tokens = NodeTokenList> {
    Char {
        #[serde(
            serialize_with = "serialize_font_id",
            deserialize_with = "deserialize_font_id"
        )]
        font: FontId,
        ch: char,
        /// Diagnostic-only source provenance; excluded from semantic identity.
        #[serde(skip, default)]
        origin: OriginId,
    },
    Lig {
        #[serde(
            serialize_with = "serialize_font_id",
            deserialize_with = "deserialize_font_id"
        )]
        font: FontId,
        ch: char,
        orig: Vec<char>,
        left_hit: bool,
        right_hit: bool,
        /// One origin per original character consumed by the ligature.
        #[serde(skip, default)]
        origins: Vec<OriginId>,
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    /// pdfTeX character protrusion retaining the exact contributing glyph.
    MarginKern {
        amount: Scaled,
        side: MarginKernSide,
        #[serde(
            serialize_with = "serialize_font_id",
            deserialize_with = "deserialize_font_id"
        )]
        font: FontId,
        ch: u8,
    },
    Glue {
        spec: Glue,
        kind: GlueKind,
        leader: Option<LeaderPayload<List>>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode<List>),
    VList(BoxNode<List>),
    Unset(UnsetNode<List>),
    Disc {
        kind: DiscKind,
        pre: List,
        post: List,
        replace: List,
        /// TeX's physical `replace_count`, retained only for diagnostics.
        #[serde(skip, default)]
        physical_replace_count: u8,
    },
    Mark {
        class: u16,
        tokens: Tokens,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: Glue,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: List,
    },
    Whatsit(Whatsit<Glue, Tokens>),
    MathOn(Scaled),
    MathOff(Scaled),
    Direction(Direction),
    MathNoad(MathNoad<List>),
    FractionNoad(MathFraction<List>),
    MathStyle(MathStyle),
    MathChoice(MathChoice<List>),
    MathList(MathListNode<List>),
    Nonscript,
    Adjust(AdjustNode<List>),
}

/// Borrowed semantic projection of a node.
///
/// Diagnostic source, physical-list, and allocator sidecars are deliberately
/// absent. Keeping this projection beside [`Node`] makes equality and hashing
/// share one exhaustive field list.
#[derive(Eq, Hash, PartialEq)]
enum SemanticNodeRef<'a, List, Glue, Tokens> {
    Char(&'a FontId, &'a char),
    Lig(&'a FontId, &'a char, &'a [char], &'a bool, &'a bool),
    Kern(&'a Scaled, &'a KernKind),
    MarginKern(&'a Scaled, &'a MarginKernSide, &'a FontId, &'a u8),
    Glue(&'a Glue, &'a GlueKind, &'a Option<LeaderPayload<List>>),
    Penalty(&'a i32),
    Rule(&'a Option<Scaled>, &'a Option<Scaled>, &'a Option<Scaled>),
    HList(&'a BoxNode<List>),
    VList(&'a BoxNode<List>),
    Unset(&'a UnsetNode<List>),
    Disc(&'a DiscKind, &'a List, &'a List, &'a List),
    Mark(&'a u16, &'a Tokens),
    Ins(&'a u16, &'a Scaled, &'a Glue, &'a Scaled, &'a i32, &'a List),
    Whatsit(&'a Whatsit<Glue, Tokens>),
    MathOn(&'a Scaled),
    MathOff(&'a Scaled),
    Direction(&'a Direction),
    MathNoad(&'a MathNoad<List>),
    FractionNoad(&'a MathFraction<List>),
    MathStyle(&'a MathStyle),
    MathChoice(&'a MathChoice<List>),
    MathList(&'a MathListNode<List>),
    Nonscript,
    Adjust(&'a AdjustNode<List>),
}

impl<List, Glue, Tokens> Node<List, Glue, Tokens> {
    /// TeX82 §§125/133--157's main-memory words owned by this node.
    ///
    /// The pair is `(variable-size, one-word)`. It describes the canonical
    /// allocation event only; Rust enum size and arena representation are
    /// deliberately irrelevant. Character cells and a ligature's source
    /// characters come from the one-word arena, while every other node uses
    /// its WEB-declared variable-size record.
    pub(crate) fn tex_memory_words(&self, etex_node_sizes: bool) -> (usize, usize) {
        let synctex_extra = usize::from(etex_node_sizes) * 2;
        let variable = match self {
            Self::Char { .. } => return (0, 1),
            Self::Lig { orig, .. } => return (2, orig.len()),
            Self::HList(_) | Self::VList(_) | Self::Unset(_) => 7 + synctex_extra,
            Self::Rule { .. } => 4 + synctex_extra,
            Self::Ins { .. } => 5,
            Self::MathNoad(noad) => match noad.kind {
                crate::math::NoadKind::Radical { .. } | crate::math::NoadKind::Accent { .. } => 5,
                _ => 4,
            },
            Self::FractionNoad(_) => 6,
            Self::MathStyle(_) => 3,
            Self::MathChoice(_) => 3,
            Self::MarginKern { .. } => 3,
            Self::Kern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Nonscript => 2 + synctex_extra,
            Self::Direction(_) if etex_node_sizes => 2 + synctex_extra,
            Self::Disc { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::Direction(_)
            | Self::MathList(_)
            | Self::Adjust(_) => 2,
        };
        (variable, 0)
    }

    fn semantic_ref(&self) -> SemanticNodeRef<'_, List, Glue, Tokens> {
        match self {
            Self::Char { font, ch, .. } => SemanticNodeRef::Char(font, ch),
            Self::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                ..
            } => SemanticNodeRef::Lig(font, ch, orig, left_hit, right_hit),
            Self::Kern { amount, kind } => SemanticNodeRef::Kern(amount, kind),
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => SemanticNodeRef::MarginKern(amount, side, font, ch),
            Self::Glue { spec, kind, leader } => SemanticNodeRef::Glue(spec, kind, leader),
            Self::Penalty(value) => SemanticNodeRef::Penalty(value),
            Self::Rule {
                width,
                height,
                depth,
            } => SemanticNodeRef::Rule(width, height, depth),
            Self::HList(value) => SemanticNodeRef::HList(value),
            Self::VList(value) => SemanticNodeRef::VList(value),
            Self::Unset(value) => SemanticNodeRef::Unset(value),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                ..
            } => SemanticNodeRef::Disc(kind, pre, post, replace),
            Self::Mark { class, tokens } => SemanticNodeRef::Mark(class, tokens),
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => SemanticNodeRef::Ins(
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            ),
            Self::Whatsit(value) => SemanticNodeRef::Whatsit(value),
            Self::MathOn(value) => SemanticNodeRef::MathOn(value),
            Self::MathOff(value) => SemanticNodeRef::MathOff(value),
            Self::Direction(value) => SemanticNodeRef::Direction(value),
            Self::MathNoad(value) => SemanticNodeRef::MathNoad(value),
            Self::FractionNoad(value) => SemanticNodeRef::FractionNoad(value),
            Self::MathStyle(value) => SemanticNodeRef::MathStyle(value),
            Self::MathChoice(value) => SemanticNodeRef::MathChoice(value),
            Self::MathList(value) => SemanticNodeRef::MathList(value),
            Self::Nonscript => SemanticNodeRef::Nonscript,
            Self::Adjust(value) => SemanticNodeRef::Adjust(value),
        }
    }

    /// Erases sidecars excluded from semantic equality and checkpoint hashes.
    pub(crate) fn erase_diagnostic_sidecars(&mut self) {
        fn erase_math_char<List>(field: &mut crate::math::MathField<List>) {
            if let crate::math::MathField::MathChar(value)
            | crate::math::MathField::MathTextChar(value) = field
            {
                value.origin = OriginId::UNKNOWN;
            }
        }

        match self {
            Self::Char { origin, .. } => *origin = OriginId::UNKNOWN,
            Self::Lig { origins, .. } => origins.clear(),
            Self::HList(box_node) | Self::VList(box_node) => {
                box_node.erase_diagnostic_sidecars();
            }
            Self::Glue {
                leader: Some(LeaderPayload::HList(box_node) | LeaderPayload::VList(box_node)),
                ..
            } => box_node.erase_diagnostic_sidecars(),
            Self::Disc {
                physical_replace_count,
                ..
            } => *physical_replace_count = 0,
            Self::MathNoad(noad) => {
                if let crate::math::NoadKind::Accent { accent } = &mut noad.kind {
                    accent.origin = OriginId::UNKNOWN;
                }
                erase_math_char(&mut noad.nucleus);
                erase_math_char(&mut noad.subscript);
                erase_math_char(&mut noad.superscript);
            }
            _ => {}
        }
    }
}

impl<List: PartialEq, Glue: PartialEq, Tokens: PartialEq> PartialEq for Node<List, Glue, Tokens> {
    fn eq(&self, other: &Self) -> bool {
        self.semantic_ref() == other.semantic_ref()
    }
}

impl<List: Eq, Glue: Eq, Tokens: Eq> Eq for Node<List, Glue, Tokens> {}

impl<List: Hash, Glue: Hash, Tokens: Hash> Hash for Node<List, Glue, Tokens> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.semantic_ref().hash(state);
    }
}

impl<List, Glue, Tokens> Node<List, Glue, Tokens> {
    /// Rewrites every direct immutable-font coordinate in this node.
    pub(crate) fn map_fonts(mut self, mut map: impl FnMut(FontId) -> FontId) -> Self {
        match &mut self {
            Self::Char { font, .. } | Self::Lig { font, .. } | Self::MarginKern { font, .. } => {
                *font = map(*font)
            }
            _ => {}
        }
        self
    }

    /// Visits exact immutable-font coordinates retained directly by this node.
    pub fn visit_fonts(&self, mut visit: impl FnMut(FontId)) {
        match self {
            Self::Char { font, .. } | Self::Lig { font, .. } | Self::MarginKern { font, .. } => {
                visit(*font)
            }
            _ => {}
        }
    }

    pub fn visit_payloads(
        &self,
        mut visit_glue: impl FnMut(&Glue),
        mut visit_tokens: impl FnMut(&Tokens),
    ) {
        match self {
            Self::Glue { spec, .. } => visit_glue(spec),
            Self::Mark { tokens, .. } => visit_tokens(tokens),
            Self::Ins { split_top_skip, .. } => visit_glue(split_top_skip),
            Self::Whatsit(whatsit) => whatsit.visit_payloads(visit_glue, visit_tokens),
            _ => {}
        }
    }

    /// Visits token words embedded directly in rare PDF action identifiers.
    /// Generic token-list payload coordinates are reported by `visit_payloads`;
    /// this companion visitor covers the node-owned name/raw spellings.
    pub(crate) fn visit_embedded_token_words(&self, mut visit: impl FnMut(TokenWord)) {
        let mut identifier = |identifier: &NodePdfActionIdentifier| match identifier {
            NodePdfActionIdentifier::Name(tokens) | NodePdfActionIdentifier::Raw(tokens) => {
                for &word in tokens.words() {
                    visit(word);
                }
            }
            NodePdfActionIdentifier::Number(_) => {}
        };
        match self {
            Self::Whatsit(Whatsit::PdfDestination(destination)) => {
                identifier(&destination.identifier);
            }
            Self::Whatsit(Whatsit::PdfThread(thread)) => identifier(&thread.identifier),
            _ => {}
        }
    }

    pub fn visit_semantic_node_lists(&self, mut visit: impl FnMut(&List)) {
        fn field<List>(field: &crate::math::MathField<List>, visit: &mut impl FnMut(&List)) {
            if let crate::math::MathField::SubBox(list) | crate::math::MathField::SubMlist(list) =
                field
            {
                visit(list);
            }
        }

        match self {
            Self::HList(node) | Self::VList(node) => visit(&node.children),
            Self::Unset(node) => visit(&node.children),
            Self::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => visit(&node.children),
            Self::Disc {
                pre, post, replace, ..
            } => {
                visit(pre);
                visit(post);
                visit(replace);
            }
            Self::Ins { content, .. } => visit(content),
            Self::MathNoad(noad) => {
                field(&noad.nucleus, &mut visit);
                field(&noad.subscript, &mut visit);
                field(&noad.superscript, &mut visit);
            }
            Self::FractionNoad(fraction) => {
                visit(&fraction.numerator);
                visit(&fraction.denominator);
            }
            Self::MathChoice(choice) => {
                visit(&choice.display);
                visit(&choice.text);
                visit(&choice.script);
                visit(&choice.script_script);
            }
            Self::MathList(list) => visit(&list.content),
            Self::Adjust(adjustment) => visit(&adjustment.content),
            Self::Char { .. }
            | Self::Lig { .. }
            | Self::Kern { .. }
            | Self::MarginKern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Direction(_)
            | Self::MathStyle(_)
            | Self::Nonscript => {}
        }
    }

    #[allow(dead_code)] // Used only by the retained legacy NodeArena memory census.
    pub(crate) fn visit_diagnostic_node_lists(&self, mut visit: impl FnMut(&List, u32)) {
        match self {
            Self::HList(node) | Self::VList(node) => {
                if let Some(children) = &node.diagnostic_children {
                    visit(children, node.allocator_high_cell_overlap);
                }
            }
            Self::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => {
                if let Some(children) = &node.diagnostic_children {
                    visit(children, node.allocator_high_cell_overlap);
                }
            }
            _ => {}
        }
    }

    /// Visits every direct structurally owned child list.
    pub fn visit_node_lists(&self, mut visit: impl FnMut(&List)) {
        fn field<List>(field: &crate::math::MathField<List>, visit: &mut impl FnMut(&List)) {
            if let crate::math::MathField::SubBox(list) | crate::math::MathField::SubMlist(list) =
                field
            {
                visit(list);
            }
        }

        match self {
            Self::HList(node) | Self::VList(node) => {
                visit(&node.children);
                if let Some(children) = &node.diagnostic_children {
                    visit(children);
                }
            }
            Self::Unset(node) => visit(&node.children),
            Self::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => {
                visit(&node.children);
                if let Some(children) = &node.diagnostic_children {
                    visit(children);
                }
            }
            Self::Disc {
                pre, post, replace, ..
            } => {
                visit(pre);
                visit(post);
                visit(replace);
            }
            Self::Ins { content, .. } => visit(content),
            Self::MathNoad(noad) => {
                field(&noad.nucleus, &mut visit);
                field(&noad.subscript, &mut visit);
                field(&noad.superscript, &mut visit);
            }
            Self::FractionNoad(fraction) => {
                visit(&fraction.numerator);
                visit(&fraction.denominator);
            }
            Self::MathChoice(choice) => {
                visit(&choice.display);
                visit(&choice.text);
                visit(&choice.script);
                visit(&choice.script_script);
            }
            Self::MathList(list) => visit(&list.content),
            Self::Adjust(adjustment) => visit(&adjustment.content),
            Self::Char { .. }
            | Self::Lig { .. }
            | Self::Kern { .. }
            | Self::MarginKern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Direction(_)
            | Self::MathStyle(_)
            | Self::Nonscript => {}
        }
    }

    /// Visits every direct child-list handle carried by this owned node.
    ///
    /// The visitor does not recurse into frozen lists.  It is the typed root
    /// projection used when an operation publishes its still-owned nodes.
    pub fn visit_node_lists_mut(&mut self, mut visit: impl FnMut(&mut List)) {
        fn field<List>(
            field: &mut crate::math::MathField<List>,
            visit: &mut impl FnMut(&mut List),
        ) {
            if let crate::math::MathField::SubBox(list) | crate::math::MathField::SubMlist(list) =
                field
            {
                visit(list);
            }
        }

        match self {
            Self::HList(node) | Self::VList(node) => {
                visit(&mut node.children);
                if let Some(children) = &mut node.diagnostic_children {
                    visit(children);
                }
            }
            Self::Unset(node) => visit(&mut node.children),
            Self::Glue {
                leader: Some(LeaderPayload::HList(node) | LeaderPayload::VList(node)),
                ..
            } => {
                visit(&mut node.children);
                if let Some(children) = &mut node.diagnostic_children {
                    visit(children);
                }
            }
            Self::Disc {
                pre, post, replace, ..
            } => {
                visit(pre);
                visit(post);
                visit(replace);
            }
            Self::Ins { content, .. } => visit(content),
            Self::MathNoad(noad) => {
                field(&mut noad.nucleus, &mut visit);
                field(&mut noad.subscript, &mut visit);
                field(&mut noad.superscript, &mut visit);
            }
            Self::FractionNoad(fraction) => {
                visit(&mut fraction.numerator);
                visit(&mut fraction.denominator);
            }
            Self::MathChoice(choice) => {
                visit(&mut choice.display);
                visit(&mut choice.text);
                visit(&mut choice.script);
                visit(&mut choice.script_script);
            }
            Self::MathList(list) => visit(&mut list.content),
            Self::Adjust(adjustment) => visit(&mut adjustment.content),
            Self::Char { .. }
            | Self::Lig { .. }
            | Self::Kern { .. }
            | Self::MarginKern { .. }
            | Self::Glue { .. }
            | Self::Penalty(_)
            | Self::Rule { .. }
            | Self::Mark { .. }
            | Self::Whatsit(_)
            | Self::MathOn(_)
            | Self::MathOff(_)
            | Self::Direction(_)
            | Self::MathStyle(_)
            | Self::Nonscript => {}
        }
    }
}

impl<List, Glue, Tokens> Node<List, Glue, Tokens> {
    /// Rewrites only typed child coordinates while preserving node payloads.
    ///
    /// This is a shallow ownership move: it does not visit or copy any child
    /// graph.
    pub fn map_lists<Other>(self, mut map: impl FnMut(List) -> Other) -> Node<Other, Glue, Tokens> {
        match self {
            Self::Char { font, ch, origin } => Node::Char { font, ch, origin },
            Self::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            },
            Self::Kern { amount, kind } => Node::Kern { amount, kind },
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Node::MarginKern {
                amount,
                side,
                font,
                ch,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec,
                kind,
                leader: leader.map(|value| value.map_lists(&mut map)),
            },
            Self::Penalty(value) => Node::Penalty(value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width,
                height,
                depth,
            },
            Self::HList(value) => Node::HList(value.map_lists(&mut map)),
            Self::VList(value) => Node::VList(value.map_lists(&mut map)),
            Self::Unset(value) => Node::Unset(value.map_list(&mut map)),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind,
                pre: map(pre),
                post: map(post),
                replace: map(replace),
                physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark { class, tokens },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content: map(content),
            },
            Self::Whatsit(value) => Node::Whatsit(value),
            Self::MathOn(value) => Node::MathOn(value),
            Self::MathOff(value) => Node::MathOff(value),
            Self::Direction(value) => Node::Direction(value),
            Self::MathNoad(value) => Node::MathNoad(value.map_lists(&mut map)),
            Self::FractionNoad(value) => Node::FractionNoad(value.map_lists(&mut map)),
            Self::MathStyle(value) => Node::MathStyle(value),
            Self::MathChoice(value) => Node::MathChoice(value.map_lists(&mut map)),
            Self::MathList(value) => Node::MathList(value.map_list(&mut map)),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(value) => Node::Adjust(value.map_list(map)),
        }
    }

    /// Rewrites typed glue and token payload coordinates without visiting any
    /// child node graph.
    pub fn map_payloads<OtherGlue, OtherTokens>(
        self,
        mut map_glue: impl FnMut(Glue) -> OtherGlue,
        mut map_tokens: impl FnMut(Tokens) -> OtherTokens,
    ) -> Node<List, OtherGlue, OtherTokens> {
        match self {
            Self::Char { font, ch, origin } => Node::Char { font, ch, origin },
            Self::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            } => Node::Lig {
                font,
                ch,
                orig,
                left_hit,
                right_hit,
                origins,
            },
            Self::Kern { amount, kind } => Node::Kern { amount, kind },
            Self::MarginKern {
                amount,
                side,
                font,
                ch,
            } => Node::MarginKern {
                amount,
                side,
                font,
                ch,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: map_glue(spec),
                kind,
                leader,
            },
            Self::Penalty(value) => Node::Penalty(value),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width,
                height,
                depth,
            },
            Self::HList(value) => Node::HList(value),
            Self::VList(value) => Node::VList(value),
            Self::Unset(value) => Node::Unset(value),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class,
                tokens: map_tokens(tokens),
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class,
                size,
                split_top_skip: map_glue(split_top_skip),
                split_max_depth,
                floating_penalty,
                content,
            },
            Self::Whatsit(value) => Node::Whatsit(value.map_payloads(map_glue, map_tokens)),
            Self::MathOn(value) => Node::MathOn(value),
            Self::MathOff(value) => Node::MathOff(value),
            Self::Direction(value) => Node::Direction(value),
            Self::MathNoad(value) => Node::MathNoad(value),
            Self::FractionNoad(value) => Node::FractionNoad(value),
            Self::MathStyle(value) => Node::MathStyle(value),
            Self::MathChoice(value) => Node::MathChoice(value),
            Self::MathList(value) => Node::MathList(value),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(value) => Node::Adjust(value),
        }
    }
}

/// A pdfTeX adjustment node payload.
///
/// Ordinary TeX adjustments migrate after their containing horizontal box;
/// pdfTeX's `pre` form migrates before it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct AdjustNode<List = PageListId> {
    pub content: List,
    pub pre: bool,
}

impl<List> AdjustNode<List> {
    #[must_use]
    pub fn ordinary(content: List) -> Self {
        Self {
            content,
            pre: false,
        }
    }

    pub(crate) fn map_list<Other>(self, map: impl FnOnce(List) -> Other) -> AdjustNode<Other> {
        AdjustNode {
            content: map(self.content),
            pre: self.pre,
        }
    }
}

/// A TeX box node payload shared by hlist and vlist nodes.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "List: serde::Serialize",
    deserialize = "List: serde::Deserialize<'de>"
))]
pub struct BoxNode<List = PageListId> {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    /// Merged e-TeX WEB §53a's `box_lr` subtype.
    pub box_lr: BoxLr,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: List,
    #[serde(skip, default)]
    pub diagnostic_children: Option<List>,
    /// Direct high-memory cells shared with `diagnostic_children` by exact
    /// allocator lineage. This allocator projection is nonsemantic and is not
    /// part of the portable format schema.
    #[serde(skip, default)]
    pub allocator_high_cell_overlap: u32,
}

impl<List> BoxNode<List> {
    /// Creates a box payload.
    #[must_use]
    pub fn new(fields: BoxNodeFields<List>) -> Self {
        Self {
            width: fields.width,
            height: fields.height,
            depth: fields.depth,
            shift: fields.shift,
            box_lr: fields.box_lr,
            glue_set: fields.glue_set,
            glue_sign: fields.glue_sign,
            glue_order: fields.glue_order,
            children: fields.children,
            diagnostic_children: None,
            allocator_high_cell_overlap: 0,
        }
    }

    pub(crate) fn map_lists<Other>(self, mut map: impl FnMut(List) -> Other) -> BoxNode<Other> {
        BoxNode {
            width: self.width,
            height: self.height,
            depth: self.depth,
            shift: self.shift,
            box_lr: self.box_lr,
            glue_set: self.glue_set,
            glue_sign: self.glue_sign,
            glue_order: self.glue_order,
            children: map(self.children),
            diagnostic_children: self.diagnostic_children.map(map),
            allocator_high_cell_overlap: self.allocator_high_cell_overlap,
        }
    }
}

impl<List: PartialEq> PartialEq for BoxNode<List> {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.depth == other.depth
            && self.shift == other.shift
            && self.box_lr == other.box_lr
            && self.glue_set == other.glue_set
            && self.glue_sign == other.glue_sign
            && self.glue_order == other.glue_order
            && self.children == other.children
    }
}

impl<List: Eq> Eq for BoxNode<List> {}

impl<List: Hash> Hash for BoxNode<List> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.width.hash(state);
        self.height.hash(state);
        self.depth.hash(state);
        self.shift.hash(state);
        self.box_lr.hash(state);
        self.glue_set.hash(state);
        self.glue_sign.hash(state);
        self.glue_order.hash(state);
        self.children.hash(state);
    }
}

impl<List> BoxNode<List> {
    fn erase_diagnostic_sidecars(&mut self) {
        self.diagnostic_children = None;
        self.allocator_high_cell_overlap = 0;
    }
}

/// Construction fields for a TeX box node payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxNodeFields<List = PageListId> {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    pub shift: Scaled,
    pub box_lr: BoxLr,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
    pub children: List,
}

/// Direction/reversal identity carried by an e-TeX horizontal box.
///
/// The numeric values are the canonical hlist subtypes from merged e-TeX WEB
/// §53a. Vertical boxes retain [`BoxLr::Normal`].
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize,
)]
#[repr(u8)]
pub enum BoxLr {
    #[default]
    Normal = 0,
    Reversed = 1,
    DList = 2,
}

/// Repeated material attached to a leader glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(bound(
    serialize = "List: serde::Serialize",
    deserialize = "List: serde::Deserialize<'de>"
))]
pub enum LeaderPayload<List = PageListId> {
    HList(BoxNode<List>),
    VList(BoxNode<List>),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
}

impl<List> LeaderPayload<List> {
    /// Rewrites only child coordinates of a leader payload.
    pub fn map_lists<Other>(self, map: impl FnMut(List) -> Other) -> LeaderPayload<Other> {
        match self {
            Self::HList(value) => LeaderPayload::HList(value.map_lists(map)),
            Self::VList(value) => LeaderPayload::VList(value.map_lists(map)),
            Self::Rule {
                width,
                height,
                depth,
            } => LeaderPayload::Rule {
                width,
                height,
                depth,
            },
        }
    }
}

/// A TeX unset box used while alignments are being measured and resolved.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct UnsetNode<List = PageListId> {
    pub kind: UnsetKind,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX82 §796's zero-based quarterword encoding (columns minus one).
    pub span_count: u16,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
    pub children: List,
}

impl<List> UnsetNode<List> {
    /// Creates an unset box payload.
    #[must_use]
    pub fn new(fields: UnsetNodeFields<List>) -> Self {
        Self {
            kind: fields.kind,
            width: fields.width,
            height: fields.height,
            depth: fields.depth,
            span_count: fields.span_count,
            stretch: fields.stretch,
            stretch_order: fields.stretch_order,
            shrink: fields.shrink,
            shrink_order: fields.shrink_order,
            children: fields.children,
        }
    }

    pub(crate) fn map_list<Other>(self, map: impl FnOnce(List) -> Other) -> UnsetNode<Other> {
        UnsetNode {
            kind: self.kind,
            width: self.width,
            height: self.height,
            depth: self.depth,
            span_count: self.span_count,
            stretch: self.stretch,
            stretch_order: self.stretch_order,
            shrink: self.shrink,
            shrink_order: self.shrink_order,
            children: map(self.children),
        }
    }
}

/// Construction fields for an unset alignment box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnsetNodeFields<List = PageListId> {
    pub kind: UnsetKind,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX82 §796's zero-based quarterword encoding (columns minus one).
    pub span_count: u16,
    pub stretch: Scaled,
    pub stretch_order: Order,
    pub shrink: Scaled,
    pub shrink_order: Order,
    pub children: List,
}

/// Whether an unset node was packaged with horizontal or vertical metrics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum UnsetKind {
    HBox,
    VBox,
}

/// The source of a kern node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum KernKind {
    Explicit,
    Font,
    /// Automatic kern from pdfTeX's `knbc`/`knac` character-code tables.
    Auto,
    Accent,
    Mu,
    /// pdfTeX character protrusion at the left edge of a finalized line.
    LeftMargin,
    /// pdfTeX character protrusion at the right edge of a finalized line.
    RightMargin,
}

/// The edge at which pdfTeX materialized a character-protrusion kern.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum MarginKernSide {
    Left,
    Right,
}

/// The source of a glue node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum GlueKind {
    Normal,
    SpaceSkip,
    XSpaceSkip,
    TabSkip,
    BaselineSkip,
    LineSkip,
    TopSkip,
    SplitTopSkip,
    LeftSkip,
    RightSkip,
    ParSkip,
    ParFillSkip,
    AboveDisplaySkip,
    BelowDisplaySkip,
    AboveDisplayShortSkip,
    BelowDisplayShortSkip,
    Leaders,
    Cleaders,
    Xleaders,
    MuSkip,
    ThinMuSkip,
    MedMuSkip,
    ThickMuSkip,
    NonScript,
}

/// The source of a discretionary node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum DiscKind {
    Discretionary,
    ExplicitHyphen,
    AutomaticHyphen,
}

/// An e-TeX M/L/R math boundary (merged e-TeX WEB §12).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
#[repr(u8)]
pub enum MathBoundary {
    BeginM = 4,
    EndM = 5,
    BeginL = 0,
    EndL = 1,
    BeginR = 2,
    EndR = 3,
}

/// Compatibility name for the L/R users of [`MathBoundary`].
pub type Direction = MathBoundary;

/// Merged e-TeX WEB §53a's split LR anomaly counter.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct LrAnomalies {
    pub missing: u32,
    pub extra: u32,
}

/// Canonical M/L/R nesting state used by list-boundary algorithms.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathBoundaryStack {
    open: Vec<MathBoundary>,
    anomalies: LrAnomalies,
}

impl MathBoundaryStack {
    pub fn observe(&mut self, boundary: MathBoundary) {
        match boundary {
            MathBoundary::BeginM | MathBoundary::BeginL | MathBoundary::BeginR => {
                self.open.push(boundary)
            }
            MathBoundary::EndM | MathBoundary::EndL | MathBoundary::EndR => {
                if self
                    .open
                    .last()
                    .copied()
                    .is_some_and(|open| open.matches(boundary))
                {
                    self.open.pop();
                } else {
                    self.anomalies.extra = self.anomalies.extra.saturating_add(1);
                }
            }
        }
    }

    #[must_use]
    pub fn finish(mut self) -> LrAnomalies {
        self.anomalies.missing = self
            .anomalies
            .missing
            .saturating_add(u32::try_from(self.open.len()).unwrap_or(u32::MAX));
        self.anomalies
    }
}

impl MathBoundary {
    const fn matches(self, end: Self) -> bool {
        matches!(
            (self, end),
            (Self::BeginM, Self::EndM) | (Self::BeginL, Self::EndL) | (Self::BeginR, Self::EndR)
        )
    }
}

/// The sign of box glue adjustment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Sign {
    Normal,
    Stretching,
    Shrinking,
}

/// Extension nodes whose effects are interpreted by later subsystems.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Whatsit<Glue = GlueSpec, Tokens = NodeTokenList> {
    OpenOut {
        slot: StreamSlot,
        path: String,
    },
    CloseOut {
        /// `None` is §1342's permanently closed normalized slot 16 or 17.
        slot: Option<StreamSlot>,
    },
    DeferredWrite {
        sink: PrintSink,
        tokens: Tokens,
    },
    Special {
        class: String,
        payload: Vec<u8>,
    },
    DeferredSpecial {
        class: String,
        tokens: Tokens,
    },
    PdfReferenceObject {
        object: u32,
    },
    PdfAccessibility(PdfAccessibilityControl),
    PdfAnnotation {
        object: u32,
    },
    PdfLinkStart {
        object: u32,
    },
    PdfLinkEnd {
        object: u32,
    },
    PdfRunningLink(bool),
    PdfLiteral {
        mode: PdfLiteralMode,
        payload: Vec<u8>,
    },
    DeferredPdfLiteral {
        mode: PdfLiteralMode,
        tokens: Tokens,
    },
    PdfSetMatrix {
        payload: Vec<u8>,
    },
    PdfSave,
    PdfRestore,
    PdfColorStack {
        id: u32,
        action: crate::PdfColorStackAction,
    },
    PdfSavePos,
    PdfSnapRefPoint,
    PdfSnapY {
        glue: Glue,
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
    PdfDestination(Box<PdfDestinationNode>),
    PdfThread(Box<PdfThreadNode<Tokens>>),
    PdfEndThread,
    Language {
        language: u8,
        left_hyphen_min: u8,
        right_hyphen_min: u8,
    },
}

impl<Glue, Tokens> Whatsit<Glue, Tokens> {
    fn visit_payloads(
        &self,
        mut visit_glue: impl FnMut(&Glue),
        mut visit_tokens: impl FnMut(&Tokens),
    ) {
        match self {
            Self::DeferredWrite { tokens, .. }
            | Self::DeferredSpecial { tokens, .. }
            | Self::DeferredPdfLiteral { tokens, .. } => visit_tokens(tokens),
            Self::PdfSnapY { glue } => visit_glue(glue),
            Self::PdfThread(thread) => visit_tokens(&thread.attributes),
            _ => {}
        }
    }

    fn map_payloads<OtherGlue, OtherTokens>(
        self,
        mut map_glue: impl FnMut(Glue) -> OtherGlue,
        mut map_tokens: impl FnMut(Tokens) -> OtherTokens,
    ) -> Whatsit<OtherGlue, OtherTokens> {
        match self {
            Self::OpenOut { slot, path } => Whatsit::OpenOut { slot, path },
            Self::CloseOut { slot } => Whatsit::CloseOut { slot },
            Self::DeferredWrite { sink, tokens } => Whatsit::DeferredWrite {
                sink,
                tokens: map_tokens(tokens),
            },
            Self::Special { class, payload } => Whatsit::Special { class, payload },
            Self::DeferredSpecial { class, tokens } => Whatsit::DeferredSpecial {
                class,
                tokens: map_tokens(tokens),
            },
            Self::PdfReferenceObject { object } => Whatsit::PdfReferenceObject { object },
            Self::PdfAccessibility(value) => Whatsit::PdfAccessibility(value),
            Self::PdfAnnotation { object } => Whatsit::PdfAnnotation { object },
            Self::PdfLinkStart { object } => Whatsit::PdfLinkStart { object },
            Self::PdfLinkEnd { object } => Whatsit::PdfLinkEnd { object },
            Self::PdfRunningLink(value) => Whatsit::PdfRunningLink(value),
            Self::PdfLiteral { mode, payload } => Whatsit::PdfLiteral { mode, payload },
            Self::DeferredPdfLiteral { mode, tokens } => Whatsit::DeferredPdfLiteral {
                mode,
                tokens: map_tokens(tokens),
            },
            Self::PdfSetMatrix { payload } => Whatsit::PdfSetMatrix { payload },
            Self::PdfSave => Whatsit::PdfSave,
            Self::PdfRestore => Whatsit::PdfRestore,
            Self::PdfColorStack { id, action } => Whatsit::PdfColorStack { id, action },
            Self::PdfSavePos => Whatsit::PdfSavePos,
            Self::PdfSnapRefPoint => Whatsit::PdfSnapRefPoint,
            Self::PdfSnapY { glue } => Whatsit::PdfSnapY {
                glue: map_glue(glue),
            },
            Self::PdfSnapYComp { ratio } => Whatsit::PdfSnapYComp { ratio },
            Self::PdfRefXForm {
                object,
                width,
                height,
                depth,
            } => Whatsit::PdfRefXForm {
                object,
                width,
                height,
                depth,
            },
            Self::PdfRefXImage {
                object,
                width,
                height,
                depth,
            } => Whatsit::PdfRefXImage {
                object,
                width,
                height,
                depth,
            },
            Self::PdfDestination(value) => Whatsit::PdfDestination(value),
            Self::PdfThread(value) => Whatsit::PdfThread(Box::new(PdfThreadNode {
                identifier: value.identifier,
                dimensions: value.dimensions,
                attributes: map_tokens(value.attributes),
                running: value.running,
            })),
            Self::PdfEndThread => Whatsit::PdfEndThread,
            Self::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            } => Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            },
        }
    }
}

/// Rare article-thread marker kept out of the hot inline node representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PdfThreadNode<Tokens = NodeTokenList> {
    pub identifier: NodePdfActionIdentifier,
    pub dimensions: crate::PdfAnnotationDimensions,
    pub attributes: Tokens,
    pub running: bool,
}

/// Rare destination marker kept out of the hot inline node representation.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PdfDestinationNode {
    pub identifier: NodePdfActionIdentifier,
    pub structure: Option<u32>,
    pub kind: PdfDestinationKind,
}

/// A navigation identifier copied into the semantic lifetime of a node.
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum NodePdfActionIdentifier {
    Name(NodeTokenList),
    Number(u32),
    Raw(NodeTokenList),
}

/// A page destination view, retained until final traversal resolves geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PdfDestinationKind {
    Xyz { zoom: Option<i32> },
    FitBoundingBoxHorizontal,
    FitBoundingBoxVertical,
    FitBoundingBox,
    FitHorizontal,
    FitVertical,
    FitRectangle(crate::PdfAnnotationDimensions),
    Fit,
}
/// Ordered PDF text-accessibility controls interpreted during page traversal.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PdfAccessibilityControl {
    InterwordSpaceOn,
    InterwordSpaceOff,
    FakeSpace,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum PdfLiteralMode {
    Origin,
    Page,
    Direct,
}

impl<List, Glue, Tokens> Node<List, Glue, Tokens> {
    /// Returns this node's source-independent logical kind.
    #[must_use]
    pub fn kind(&self) -> NodeKind {
        match self {
            Self::Char { .. } => NodeKind::Char,
            Self::Lig { .. } => NodeKind::Lig,
            Self::Kern { .. } => NodeKind::Kern,
            Self::MarginKern { .. } => NodeKind::MarginKern,
            Self::Glue { .. } => NodeKind::Glue,
            Self::Penalty(_) => NodeKind::Penalty,
            Self::Rule { .. } => NodeKind::Rule,
            Self::HList(_) => NodeKind::HList,
            Self::VList(_) => NodeKind::VList,
            Self::Unset(_) => NodeKind::Unset,
            Self::Disc { .. } => NodeKind::Disc,
            Self::Mark { .. } => NodeKind::Mark,
            Self::Ins { .. } => NodeKind::Ins,
            Self::Whatsit(_) => NodeKind::Whatsit,
            Self::MathOn(_) => NodeKind::MathOn,
            Self::MathOff(_) => NodeKind::MathOff,
            Self::Direction(_) => NodeKind::Direction,
            Self::MathNoad(_) => NodeKind::MathNoad,
            Self::FractionNoad(_) => NodeKind::FractionNoad,
            Self::MathStyle(_) => NodeKind::MathStyle,
            Self::MathChoice(_) => NodeKind::MathChoice,
            Self::MathList(_) => NodeKind::MathList,
            Self::Nonscript => NodeKind::Nonscript,
            Self::Adjust(_) => NodeKind::Adjust,
        }
    }

    /// e-TeX `\lastnodetype` code for this node.
    #[must_use]
    pub fn etex_type(&self) -> i32 {
        self.kind().etex_type()
    }
}

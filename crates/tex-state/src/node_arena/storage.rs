use super::builder::CompactBuilderNode;
use super::tables::{BoxTable, InsertionTable, NoadTable, UnsetTable};
use super::view::NodeList;
use super::{checked_len, preflight_capacity};
use crate::ids::NodeListId;
use crate::math::MathStyle;
use crate::node::{DiscKind, GlueKind, KernKind, MarginKernSide, Node};
use crate::provenance::OriginRef;
use crate::scaled::Scaled;
use crate::token::OriginId;
use crate::token_store::TokenListRef;

const TAG_SHIFT: u32 = 59;
const PAYLOAD_MASK: u64 = (1_u64 << TAG_SHIFT) - 1;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NodeWord(u64);

const _: [(); 8] = [(); core::mem::size_of::<NodeWord>()];

impl NodeWord {
    const fn new(tag: u8, payload: u64) -> Self {
        assert!(tag < 32, "node-word tag exceeds five bits");
        assert!(payload <= PAYLOAD_MASK, "node-word payload exceeds 59 bits");
        Self(((tag as u64) << TAG_SHIFT) | payload)
    }

    pub(super) const fn tag(self) -> u8 {
        (self.0 >> TAG_SHIFT) as u8
    }

    pub(super) const fn payload(self) -> u64 {
        self.0 & PAYLOAD_MASK
    }

    pub(super) const fn sidecar(tag: u8, index: u32) -> Self {
        Self::new(tag, index as u64)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SidecarNeeds {
    any: bool,
    pub(super) ligatures: u32,
    pub(super) boxes: u32,
    pub(super) unsets: u32,
    pub(super) rules: u32,
    pub(super) leaders: u32,
    pub(super) discs: u32,
    pub(super) marks: u32,
    pub(super) insertions: u32,
    pub(super) whatsits: u32,
    pub(super) noads: u32,
    pub(super) fractions: u32,
    pub(super) choices: u32,
    pub(super) math_lists: u32,
    pub(super) adjusts: u32,
}

impl SidecarNeeds {
    pub(crate) fn preflight_and_count<List>(&mut self, node: &Node<List>) {
        let target = match node {
            Node::Lig {
                ch, orig, origins, ..
            } => {
                assert!(
                    (*ch as u32) <= u8::MAX as u32,
                    "ligature glyph exceeds TFM byte domain"
                );
                // TeX82 §§1034-1036 can form a ligature from a deleted left
                // boundary while retaining the first real character. Such a
                // ligature has an empty source list by construction.
                assert!(orig.len() <= 63, "ligature source exceeds TeX word limit");
                assert_eq!(
                    orig.len(),
                    origins.len(),
                    "ligature source/provenance length mismatch"
                );
                assert!(
                    orig.iter().all(|ch| (*ch as u32) <= u8::MAX as u32),
                    "ligature original exceeds TFM byte domain"
                );
                Some(&mut self.ligatures)
            }
            Node::HList(_) | Node::VList(_) => Some(&mut self.boxes),
            Node::Unset(_) => Some(&mut self.unsets),
            Node::Rule { .. } => Some(&mut self.rules),
            Node::Glue {
                leader: Some(_), ..
            } => Some(&mut self.leaders),
            Node::Disc { .. } => Some(&mut self.discs),
            Node::Mark { .. } => Some(&mut self.marks),
            Node::Ins { .. } => Some(&mut self.insertions),
            Node::Whatsit(_) => Some(&mut self.whatsits),
            Node::MathNoad(_) => Some(&mut self.noads),
            Node::FractionNoad(_) => Some(&mut self.fractions),
            Node::MathChoice(_) => Some(&mut self.choices),
            Node::MathList(_) => Some(&mut self.math_lists),
            Node::Adjust(_) => Some(&mut self.adjusts),
            Node::Char { .. }
            | Node::Kern { .. }
            | Node::MarginKern { .. }
            | Node::Glue { leader: None, .. }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
            | Node::Direction(_)
            | Node::MathStyle(_)
            | Node::Nonscript => None,
        };
        if let Some(target) = target {
            self.any = true;
            *target = target.checked_add(1).expect("sidecar count overflow");
        }
    }

    #[cfg(feature = "profiling")]
    pub(super) fn as_array(self) -> [u32; 14] {
        [
            self.ligatures,
            self.boxes,
            self.unsets,
            self.rules,
            self.leaders,
            self.discs,
            self.marks,
            self.insertions,
            self.whatsits,
            self.noads,
            self.fractions,
            self.choices,
            self.math_lists,
            self.adjusts,
        ]
    }
}

#[derive(Clone, Debug)]
pub(super) struct LigatureSidecar {
    pub(super) font: crate::ids::FontId,
    pub(super) ch: char,
    pub(super) orig: Vec<char>,
    pub(super) origins: Vec<OriginId>,
    pub(super) origin_roots: Vec<OriginRef>,
    pub(super) left_hit: bool,
    pub(super) right_hit: bool,
}

/// Canonical compact storage inside one structurally owned payload.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeStorage {
    pub(super) words: Vec<NodeWord>,
    /// Strong glue owner aligned one-for-one with ordinary glue node words.
    pub(super) glue_roots: Vec<Option<crate::glue::GlueSpecRef>>,
    /// Diagnostic-only provenance aligned one-for-one with `words`.
    pub(super) origins: Vec<OriginId>,
    /// Strong character provenance aligned one-for-one with `words`.
    /// Ligature roots live in their ragged sidecar row.
    pub(super) origin_roots: Vec<Option<OriginRef>>,
    pub(super) ligatures: Vec<LigatureSidecar>,
    pub(super) boxes: BoxTable,
    pub(super) unsets: UnsetTable,
    pub(super) rules: Vec<(Option<Scaled>, Option<Scaled>, Option<Scaled>)>,
    pub(super) leaders: Vec<(
        crate::glue::GlueSpecRef,
        GlueKind,
        crate::node::LeaderPayload<NodeListId>,
    )>,
    pub(super) discs: Vec<(DiscKind, NodeListId, NodeListId, NodeListId, u8)>,
    pub(super) marks: Vec<(u16, TokenListRef)>,
    pub(super) insertions: InsertionTable,
    pub(super) whatsits: Vec<crate::node::Whatsit>,
    pub(super) noads: NoadTable,
    pub(super) fractions: Vec<crate::math::MathFraction<NodeListId>>,
    pub(super) choices: Vec<crate::math::MathChoice<NodeListId>>,
    pub(super) math_lists: Vec<crate::math::MathListNode<NodeListId>>,
    pub(super) adjusts: Vec<crate::node::AdjustNode<NodeListId>>,
    /// Exact totals for heap allocations owned below ligature and whatsit
    /// sidecar rows. Profiling reads these after every append, so keep the
    /// totals incrementally instead of rescanning all accumulated rows.
    #[cfg(feature = "profiling")]
    pub(super) nested_payload_logical: u64,
    #[cfg(feature = "profiling")]
    pub(super) nested_payload_retained: u64,
}

impl NodeStorage {
    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }
    pub(crate) fn append_owned_preflighted(
        &mut self,
        nodes: &mut Vec<Node>,
        needs: SidecarNeeds,
    ) -> (u32, u32) {
        #[cfg(feature = "profiling")]
        let capacity_before = self.capacity_signature();
        #[cfg(feature = "profiling")]
        let retained_before = self.retained_payload_bytes();
        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(nodes.len(), "node list exceeds u32 entries");
        start
            .checked_add(len)
            .expect("node arena span overflows u32");
        if needs.any {
            self.preflight_sidecars(needs);
        }
        self.words.reserve(nodes.len());
        self.origins.reserve(nodes.len());
        self.origin_roots.reserve(nodes.len());
        self.glue_roots.reserve(nodes.len());
        if needs.any {
            self.reserve_sidecars(needs);
        }
        for node in nodes.drain(..) {
            let glue_root = match &node {
                Node::Glue {
                    spec, leader: None, ..
                } => Some(*spec),
                _ => None,
            };
            let origin = match &node {
                Node::Char { origin, .. } => origin.id(),
                Node::Lig { origins, .. } => {
                    origins.first().map_or(OriginId::UNKNOWN, OriginRef::id)
                }
                _ => OriginId::UNKNOWN,
            };
            let origin_root = match &node {
                Node::Char { origin, .. } => Some(origin.clone()),
                _ => None,
            };
            let word = self.encode_owned(node);
            self.words.push(word);
            self.glue_roots.push(glue_root);
            self.origins.push(origin);
            self.origin_roots.push(origin_root);
        }
        #[cfg(feature = "profiling")]
        {
            let capacity_after = self.capacity_signature();
            let growth_by_column = core::array::from_fn(|index| {
                u8::from(capacity_before[index] != capacity_after[index])
            });
            let retained_after = self.retained_payload_bytes();
            crate::measurement::record_node_append(
                len as usize,
                needs.as_array(),
                growth_by_column,
                retained_after.saturating_sub(retained_before),
                false,
            );
            self.record_peak();
        }
        (start, len)
    }

    pub(crate) fn append_compact_builder(&mut self, rows: Vec<CompactBuilderNode>) -> (u32, u32) {
        #[cfg(feature = "profiling")]
        let capacity_before = self.capacity_signature();
        #[cfg(feature = "profiling")]
        let retained_before = self.retained_payload_bytes();
        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(rows.len(), "node list exceeds u32 entries");
        start
            .checked_add(len)
            .expect("node arena span overflows u32");
        self.words.reserve(rows.len());
        self.origins.reserve(rows.len());
        self.origin_roots.reserve(rows.len());
        self.glue_roots.reserve(rows.len());
        for row in rows {
            let (word, origin_root) = if let Some((font, ch)) = row.as_character() {
                (
                    NodeWord::new(0, (ch as u64) | ((font.raw() as u64) << 21)),
                    Some(OriginRef::unknown()),
                )
            } else {
                let (amount, kind) = row.as_kern().expect("compact builder row has a node tag");
                (
                    NodeWord::new(
                        2,
                        amount.raw() as u32 as u64 | ((kern_code(kind) as u64) << 32),
                    ),
                    None,
                )
            };
            self.words.push(word);
            self.glue_roots.push(None);
            self.origins.push(OriginId::UNKNOWN);
            self.origin_roots.push(origin_root);
        }
        #[cfg(feature = "profiling")]
        {
            let capacity_after = self.capacity_signature();
            let growth_by_column = core::array::from_fn(|index| {
                u8::from(capacity_before[index] != capacity_after[index])
            });
            let retained_after = self.retained_payload_bytes();
            crate::measurement::record_node_append(
                len as usize,
                SidecarNeeds::default().as_array(),
                growth_by_column,
                retained_after.saturating_sub(retained_before),
                false,
            );
            self.record_peak();
        }
        (start, len)
    }

    pub(crate) fn append_compact_nodes(&mut self, nodes: &[Node<NodeListId>]) -> (u32, u32) {
        let mut needs = SidecarNeeds::default();
        for node in nodes {
            needs.preflight_and_count(node);
        }
        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(nodes.len(), "node list exceeds u32 entries");
        start
            .checked_add(len)
            .expect("node arena span overflows u32");
        self.preflight_sidecars(needs);
        self.words.reserve(nodes.len());
        self.origins.reserve(nodes.len());
        self.origin_roots.reserve(nodes.len());
        self.glue_roots.reserve(nodes.len());
        self.reserve_sidecars(needs);
        for node in nodes {
            let word = self.encode(node, |id| *id);
            self.words.push(word);
            self.glue_roots.push(match node {
                Node::Glue {
                    spec, leader: None, ..
                } => Some(*spec),
                _ => None,
            });
            self.origins.push(match node {
                Node::Char { origin, .. } => origin.id(),
                Node::Lig { origins, .. } => {
                    origins.first().map_or(OriginId::UNKNOWN, OriginRef::id)
                }
                _ => OriginId::UNKNOWN,
            });
            self.origin_roots.push(match node {
                Node::Char { origin, .. } => Some(origin.clone()),
                _ => None,
            });
        }
        (start, len)
    }

    pub(super) fn preflight_sidecars(&self, needs: SidecarNeeds) {
        macro_rules! preflight_if_needed {
            ($field:ident, $message:literal) => {
                if needs.$field != 0 {
                    preflight_capacity(
                        checked_len(self.$field.len(), $message),
                        needs.$field,
                        $message,
                    );
                }
            };
        }
        preflight_if_needed!(ligatures, "ligature sidecar exceeds u32 entries");
        preflight_if_needed!(boxes, "box sidecar exceeds u32 entries");
        preflight_if_needed!(unsets, "unset sidecar exceeds u32 entries");
        preflight_if_needed!(rules, "rule sidecar exceeds u32 entries");
        preflight_if_needed!(leaders, "leader sidecar exceeds u32 entries");
        preflight_if_needed!(discs, "disc sidecar exceeds u32 entries");
        preflight_if_needed!(marks, "mark sidecar exceeds u32 entries");
        preflight_if_needed!(insertions, "insertion sidecar exceeds u32 entries");
        preflight_if_needed!(whatsits, "whatsit sidecar exceeds u32 entries");
        preflight_if_needed!(noads, "noad sidecar exceeds u32 entries");
        preflight_if_needed!(fractions, "fraction sidecar exceeds u32 entries");
        preflight_if_needed!(choices, "choice sidecar exceeds u32 entries");
        preflight_if_needed!(math_lists, "math-list sidecar exceeds u32 entries");
        preflight_if_needed!(adjusts, "adjust sidecar exceeds u32 entries");
    }

    pub(super) fn reserve_sidecars(&mut self, needs: SidecarNeeds) {
        macro_rules! reserve_if_needed {
            ($field:ident) => {
                if needs.$field != 0 {
                    self.$field.reserve(needs.$field as usize);
                }
            };
        }
        reserve_if_needed!(ligatures);
        reserve_if_needed!(boxes);
        reserve_if_needed!(unsets);
        reserve_if_needed!(rules);
        reserve_if_needed!(leaders);
        reserve_if_needed!(discs);
        reserve_if_needed!(marks);
        reserve_if_needed!(insertions);
        reserve_if_needed!(whatsits);
        reserve_if_needed!(noads);
        reserve_if_needed!(fractions);
        reserve_if_needed!(choices);
        reserve_if_needed!(math_lists);
        reserve_if_needed!(adjusts);
    }

    fn encode<List: Clone>(
        &mut self,
        node: &Node<List>,
        child_id: impl Copy + Fn(&List) -> NodeListId,
    ) -> NodeWord {
        match node {
            Node::Char { font, ch, .. } => {
                NodeWord::new(0, (*ch as u64) | ((font.raw() as u64) << 21))
            }
            Node::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => {
                // Character nodes store only the dense font slot in their packed
                // word. Canonicalize ligature sidecars the same way so a live
                // epoch-bearing handle and a packed character handle cannot look
                // like two distinct resources with the same public font id.
                let font = crate::ids::FontId::new(font.raw());
                let word = push_sidecar(
                    1,
                    &mut self.ligatures,
                    LigatureSidecar {
                        font,
                        ch: *ch,
                        orig: orig.clone(),
                        origins: origins.iter().map(OriginRef::id).collect(),
                        origin_roots: origins.clone(),
                        left_hit: *left_hit,
                        right_hit: *right_hit,
                    },
                );
                #[cfg(feature = "profiling")]
                self.record_last_ligature_payload();
                word
            }
            Node::Kern { amount, kind } => NodeWord::new(
                2,
                amount.raw() as u32 as u64 | ((kern_code(*kind) as u64) << 32),
            ),
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => encode_margin_kern(*amount, *side, *font, *ch),
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => NodeWord::new(
                3,
                spec.id().raw() as u64 | ((glue_code(*kind) as u64) << 32),
            ),
            Node::Penalty(value) => NodeWord::new(4, *value as u32 as u64),
            Node::MathOn(value) => NodeWord::new(5, value.raw() as u32 as u64),
            Node::MathOff(value) => NodeWord::new(6, value.raw() as u32 as u64),
            Node::Direction(direction) => NodeWord::new(23, *direction as u64),
            Node::MathStyle(style) => NodeWord::new(7, style_code(*style) as u64),
            Node::Nonscript => NodeWord::new(8, 0),
            Node::HList(value) => NodeWord::sidecar(
                9,
                self.boxes
                    .push(value.clone().map_lists(|list| child_id(&list))),
            ),
            Node::VList(value) => NodeWord::sidecar(
                10,
                self.boxes
                    .push(value.clone().map_lists(|list| child_id(&list))),
            ),
            Node::Unset(value) => NodeWord::sidecar(
                11,
                self.unsets
                    .push(value.clone().map_list(|list| child_id(&list))),
            ),
            Node::Rule {
                width,
                height,
                depth,
            } => push_sidecar(12, &mut self.rules, (*width, *height, *depth)),
            Node::Glue {
                spec,
                kind,
                leader: Some(value),
            } => push_sidecar(
                13,
                &mut self.leaders,
                (
                    *spec,
                    *kind,
                    value.clone().map_lists(|list| child_id(&list)),
                ),
            ),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => push_sidecar(
                14,
                &mut self.discs,
                (
                    *kind,
                    child_id(pre),
                    child_id(post),
                    child_id(replace),
                    *physical_replace_count,
                ),
            ),
            Node::Mark { class, tokens } => push_sidecar(15, &mut self.marks, (*class, *tokens)),
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => NodeWord::sidecar(
                16,
                self.insertions.push((
                    *class,
                    *size,
                    *split_top_skip,
                    *split_max_depth,
                    *floating_penalty,
                    child_id(content),
                )),
            ),
            Node::Whatsit(value) => {
                let word = push_sidecar(17, &mut self.whatsits, value.clone());
                #[cfg(feature = "profiling")]
                self.record_last_whatsit_payload();
                word
            }
            Node::MathNoad(value) => NodeWord::sidecar(
                18,
                self.noads
                    .push(value.clone().map_lists(|list| child_id(&list))),
            ),
            Node::FractionNoad(value) => push_sidecar(
                19,
                &mut self.fractions,
                value.clone().map_lists(|list| child_id(&list)),
            ),
            Node::MathChoice(value) => push_sidecar(
                20,
                &mut self.choices,
                value.clone().map_lists(|list| child_id(&list)),
            ),
            Node::MathList(value) => push_sidecar(
                21,
                &mut self.math_lists,
                value.clone().map_list(|list| child_id(&list)),
            ),
            Node::Adjust(value) => push_sidecar(
                22,
                &mut self.adjusts,
                value.clone().map_list(|list| child_id(&list)),
            ),
        }
    }

    // Keep the complete match here instead of forwarding non-owning variants
    // to `encode`: this is the hot owned-freeze loop, and a second tag dispatch
    // measurably gives back part of the move-encoding win.
    fn encode_owned(&mut self, node: Node) -> NodeWord {
        match node {
            Node::Char { font, ch, .. } => {
                NodeWord::new(0, (ch as u64) | ((font.raw() as u64) << 21))
            }
            Node::Lig {
                font,
                ch,
                orig,
                origins,
                left_hit,
                right_hit,
            } => {
                let font = crate::ids::FontId::new(font.raw());
                let word = push_sidecar(
                    1,
                    &mut self.ligatures,
                    LigatureSidecar {
                        font,
                        ch,
                        orig,
                        origins: origins.iter().map(OriginRef::id).collect(),
                        origin_roots: origins,
                        left_hit,
                        right_hit,
                    },
                );
                #[cfg(feature = "profiling")]
                self.record_last_ligature_payload();
                word
            }
            Node::Kern { amount, kind } => NodeWord::new(
                2,
                amount.raw() as u32 as u64 | ((kern_code(kind) as u64) << 32),
            ),
            Node::MarginKern {
                amount,
                side,
                font,
                ch,
            } => encode_margin_kern(amount, side, font, ch),
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => NodeWord::new(3, spec.id().raw() as u64 | ((glue_code(kind) as u64) << 32)),
            Node::Penalty(value) => NodeWord::new(4, value as u32 as u64),
            Node::MathOn(value) => NodeWord::new(5, value.raw() as u32 as u64),
            Node::MathOff(value) => NodeWord::new(6, value.raw() as u32 as u64),
            Node::Direction(direction) => NodeWord::new(23, direction as u64),
            Node::MathStyle(style) => NodeWord::new(7, style_code(style) as u64),
            Node::Nonscript => NodeWord::new(8, 0),
            Node::HList(value) => {
                NodeWord::sidecar(9, self.boxes.push(value.map_lists(|list| list.id())))
            }
            Node::VList(value) => {
                NodeWord::sidecar(10, self.boxes.push(value.map_lists(|list| list.id())))
            }
            Node::Unset(value) => {
                NodeWord::sidecar(11, self.unsets.push(value.map_list(|list| list.id())))
            }
            Node::Rule {
                width,
                height,
                depth,
            } => push_sidecar(12, &mut self.rules, (width, height, depth)),
            Node::Glue {
                spec,
                kind,
                leader: Some(value),
            } => push_sidecar(
                13,
                &mut self.leaders,
                (spec, kind, value.map_lists(|list| list.id())),
            ),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
                physical_replace_count,
            } => push_sidecar(
                14,
                &mut self.discs,
                (
                    kind,
                    pre.id(),
                    post.id(),
                    replace.id(),
                    physical_replace_count,
                ),
            ),
            Node::Mark { class, tokens } => push_sidecar(15, &mut self.marks, (class, tokens)),
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => NodeWord::sidecar(
                16,
                self.insertions.push((
                    class,
                    size,
                    split_top_skip,
                    split_max_depth,
                    floating_penalty,
                    content.id(),
                )),
            ),
            Node::Whatsit(value) => {
                let word = push_sidecar(17, &mut self.whatsits, value);
                #[cfg(feature = "profiling")]
                self.record_last_whatsit_payload();
                word
            }
            Node::MathNoad(value) => {
                NodeWord::sidecar(18, self.noads.push(value.map_lists(|list| list.id())))
            }
            Node::FractionNoad(value) => {
                push_sidecar(19, &mut self.fractions, value.map_lists(|list| list.id()))
            }
            Node::MathChoice(value) => {
                push_sidecar(20, &mut self.choices, value.map_lists(|list| list.id()))
            }
            Node::MathList(value) => {
                push_sidecar(21, &mut self.math_lists, value.map_list(|list| list.id()))
            }
            Node::Adjust(value) => {
                push_sidecar(22, &mut self.adjusts, value.map_list(|list| list.id()))
            }
        }
    }

    pub(crate) fn view(&self, start: u32, len: u32) -> NodeList<'_> {
        let end = start as usize + len as usize;
        assert!(end <= self.words.len(), "node-list id is not live");
        NodeList {
            storage: self,
            start: start as usize,
            end,
        }
    }
}

fn push_sidecar<T>(tag: u8, table: &mut Vec<T>, value: T) -> NodeWord {
    let i = checked_len(table.len(), "node sidecar exceeds u32 entries");
    table.push(value);
    NodeWord::sidecar(tag, i)
}

fn encode_margin_kern(
    amount: Scaled,
    side: MarginKernSide,
    font: crate::ids::FontId,
    ch: u8,
) -> NodeWord {
    debug_assert!(font.raw() < (1 << 15), "font id exceeds TeX font domain");
    let side = u64::from(matches!(side, MarginKernSide::Right));
    NodeWord::new(
        24,
        amount.raw() as u32 as u64
            | (side << 32)
            | ((font.raw() as u64) << 33)
            | ((ch as u64) << 48),
    )
}
fn kern_code(v: KernKind) -> u8 {
    match v {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
        KernKind::LeftMargin => 4,
        KernKind::RightMargin => 5,
        KernKind::Auto => 6,
    }
}
fn style_code(v: MathStyle) -> u8 {
    match v {
        MathStyle::Display => 0,
        MathStyle::Text => 1,
        MathStyle::Script => 2,
        MathStyle::ScriptScript => 3,
    }
}
fn glue_code(v: GlueKind) -> u8 {
    match v {
        GlueKind::Normal => 0,
        GlueKind::TabSkip => 1,
        GlueKind::BaselineSkip => 2,
        GlueKind::LineSkip => 3,
        GlueKind::TopSkip => 4,
        GlueKind::SplitTopSkip => 5,
        GlueKind::LeftSkip => 6,
        GlueKind::RightSkip => 7,
        GlueKind::ParFillSkip => 8,
        GlueKind::AboveDisplaySkip => 9,
        GlueKind::BelowDisplaySkip => 10,
        GlueKind::AboveDisplayShortSkip => 11,
        GlueKind::BelowDisplayShortSkip => 12,
        GlueKind::Leaders => 13,
        GlueKind::Cleaders => 14,
        GlueKind::Xleaders => 15,
        GlueKind::MuSkip => 16,
        GlueKind::ThinMuSkip => 17,
        GlueKind::MedMuSkip => 18,
        GlueKind::ThickMuSkip => 19,
        GlueKind::NonScript => 20,
        GlueKind::ParSkip => 21,
        GlueKind::SpaceSkip => 22,
        GlueKind::XSpaceSkip => 23,
    }
}

pub(super) fn decode_kern(value: u8) -> KernKind {
    match value {
        0 => KernKind::Explicit,
        1 => KernKind::Font,
        2 => KernKind::Accent,
        3 => KernKind::Mu,
        4 => KernKind::LeftMargin,
        5 => KernKind::RightMargin,
        6 => KernKind::Auto,
        _ => unreachable!(),
    }
}
pub(super) fn decode_style(value: u8) -> MathStyle {
    match value {
        0 => MathStyle::Display,
        1 => MathStyle::Text,
        2 => MathStyle::Script,
        3 => MathStyle::ScriptScript,
        _ => unreachable!(),
    }
}
pub(super) fn decode_glue(value: u8) -> GlueKind {
    match value {
        0 => GlueKind::Normal,
        1 => GlueKind::TabSkip,
        2 => GlueKind::BaselineSkip,
        3 => GlueKind::LineSkip,
        4 => GlueKind::TopSkip,
        5 => GlueKind::SplitTopSkip,
        6 => GlueKind::LeftSkip,
        7 => GlueKind::RightSkip,
        8 => GlueKind::ParFillSkip,
        9 => GlueKind::AboveDisplaySkip,
        10 => GlueKind::BelowDisplaySkip,
        11 => GlueKind::AboveDisplayShortSkip,
        12 => GlueKind::BelowDisplayShortSkip,
        13 => GlueKind::Leaders,
        14 => GlueKind::Cleaders,
        15 => GlueKind::Xleaders,
        16 => GlueKind::MuSkip,
        17 => GlueKind::ThinMuSkip,
        18 => GlueKind::MedMuSkip,
        19 => GlueKind::ThickMuSkip,
        20 => GlueKind::NonScript,
        21 => GlueKind::ParSkip,
        22 => GlueKind::SpaceSkip,
        23 => GlueKind::XSpaceSkip,
        _ => unreachable!(),
    }
}

use super::tables::{BoxTable, InsertionTable, NoadTable, UnsetTable};
use super::view::NodeList;
use super::{checked_len, preflight_capacity};
use crate::identity::IdentityMark;
use crate::ids::{GlueId, NodeListId};
use crate::math::MathStyle;
use crate::node::{DiscKind, GlueKind, KernKind, Node};
use crate::scaled::Scaled;

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

/// One opaque aggregate rollback watermark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeArenaMark {
    pub(super) storage: StorageMark,
    pub(super) identities: IdentityMark,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct StorageMark {
    pub(super) words: u32,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SidecarNeeds {
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
    fn count(&mut self, node: &Node) {
        let target = match node {
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
            | Node::Lig { .. }
            | Node::Kern { .. }
            | Node::Glue { leader: None, .. }
            | Node::Penalty(_)
            | Node::MathOn(_)
            | Node::MathOff(_)
            | Node::Direction(_)
            | Node::MathStyle(_)
            | Node::Nonscript => None,
        };
        if let Some(target) = target {
            *target = target.checked_add(1).expect("sidecar count overflow");
        }
    }

    pub(super) fn as_array(self) -> [u32; 13] {
        [
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

/// Canonical compact storage shared by epoch and survivor arenas.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeStorage {
    pub(super) words: Vec<NodeWord>,
    pub(super) boxes: BoxTable,
    pub(super) unsets: UnsetTable,
    pub(super) rules: Vec<(Option<Scaled>, Option<Scaled>, Option<Scaled>)>,
    pub(super) leaders: Vec<(GlueId, GlueKind, crate::node::LeaderPayload)>,
    pub(super) discs: Vec<(DiscKind, NodeListId, NodeListId, NodeListId)>,
    pub(super) marks: Vec<(u16, crate::ids::TokenListId)>,
    pub(super) insertions: InsertionTable,
    pub(super) whatsits: Vec<crate::node::Whatsit>,
    pub(super) noads: NoadTable,
    pub(super) fractions: Vec<crate::math::MathFraction>,
    pub(super) choices: Vec<crate::math::MathChoice>,
    pub(super) math_lists: Vec<crate::math::MathListNode>,
    pub(super) adjusts: Vec<NodeListId>,
}

impl NodeStorage {
    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }
    pub(crate) fn node_capacity(&self) -> usize {
        self.words.capacity()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
    pub(crate) fn clear(&mut self) {
        self.truncate(StorageMark::default());
    }

    pub(super) fn mark(&self) -> StorageMark {
        StorageMark {
            words: checked_len(self.words.len(), "node arena exceeds u32 entries"),
            boxes: checked_len(self.boxes.len(), "box sidecar exceeds u32 entries"),
            unsets: checked_len(self.unsets.len(), "unset sidecar exceeds u32 entries"),
            rules: checked_len(self.rules.len(), "rule sidecar exceeds u32 entries"),
            leaders: checked_len(self.leaders.len(), "leader sidecar exceeds u32 entries"),
            discs: checked_len(self.discs.len(), "disc sidecar exceeds u32 entries"),
            marks: checked_len(self.marks.len(), "mark sidecar exceeds u32 entries"),
            insertions: checked_len(
                self.insertions.len(),
                "insertion sidecar exceeds u32 entries",
            ),
            whatsits: checked_len(self.whatsits.len(), "whatsit sidecar exceeds u32 entries"),
            noads: checked_len(self.noads.len(), "noad sidecar exceeds u32 entries"),
            fractions: checked_len(self.fractions.len(), "fraction sidecar exceeds u32 entries"),
            choices: checked_len(self.choices.len(), "choice sidecar exceeds u32 entries"),
            math_lists: checked_len(
                self.math_lists.len(),
                "math-list sidecar exceeds u32 entries",
            ),
            adjusts: checked_len(self.adjusts.len(), "adjust sidecar exceeds u32 entries"),
        }
    }

    pub(super) fn truncate(&mut self, mark: StorageMark) {
        // Validate the entire tuple before mutating any stream.
        assert!(mark.words as usize <= self.words.len());
        assert!(mark.boxes as usize <= self.boxes.len());
        assert!(mark.unsets as usize <= self.unsets.len());
        assert!(mark.rules as usize <= self.rules.len());
        assert!(mark.leaders as usize <= self.leaders.len());
        assert!(mark.discs as usize <= self.discs.len());
        assert!(mark.marks as usize <= self.marks.len());
        assert!(mark.insertions as usize <= self.insertions.len());
        assert!(mark.whatsits as usize <= self.whatsits.len());
        assert!(mark.noads as usize <= self.noads.len());
        assert!(mark.fractions as usize <= self.fractions.len());
        assert!(mark.choices as usize <= self.choices.len());
        assert!(mark.math_lists as usize <= self.math_lists.len());
        assert!(mark.adjusts as usize <= self.adjusts.len());
        self.words.truncate(mark.words as usize);
        self.boxes.truncate(mark.boxes as usize);
        self.unsets.truncate(mark.unsets as usize);
        self.rules.truncate(mark.rules as usize);
        self.leaders.truncate(mark.leaders as usize);
        self.discs.truncate(mark.discs as usize);
        self.marks.truncate(mark.marks as usize);
        self.insertions.truncate(mark.insertions as usize);
        self.whatsits.truncate(mark.whatsits as usize);
        self.noads.truncate(mark.noads as usize);
        self.fractions.truncate(mark.fractions as usize);
        self.choices.truncate(mark.choices as usize);
        self.math_lists.truncate(mark.math_lists as usize);
        self.adjusts.truncate(mark.adjusts as usize);
    }

    pub(crate) fn append(&mut self, nodes: &[Node]) -> (u32, u32) {
        #[cfg(feature = "profiling-stats")]
        let capacity_before = self.capacity_signature();
        #[cfg(feature = "profiling-stats")]
        let retained_before = self.retained_payload_bytes();
        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(nodes.len(), "node list exceeds u32 entries");
        start
            .checked_add(len)
            .expect("node arena span overflows u32");
        // Validate every encoding and selected table before reserving or
        // publishing either rows or words. Publication below is infallible
        // apart from process-aborting allocation failure.
        let mut needs = SidecarNeeds::default();
        for node in nodes {
            preflight_encoding(node);
            needs.count(node);
        }
        for (have, add) in self.sidecar_lengths().into_iter().zip(needs.as_array()) {
            preflight_capacity(have, add, "node sidecar exceeds u32 entries");
        }
        self.words.reserve(nodes.len());
        self.reserve_sidecars(needs);
        for node in nodes {
            let word = self.encode(node);
            self.words.push(word);
        }
        #[cfg(feature = "profiling-stats")]
        {
            let capacity_after = self.capacity_signature();
            let growth_events = capacity_before
                .iter()
                .zip(capacity_after)
                .filter(|(before, after)| **before != *after)
                .count();
            let retained_after = self.retained_payload_bytes();
            crate::measurement::record_node_append(
                nodes.len(),
                needs.as_array(),
                growth_events,
                retained_after.saturating_sub(retained_before),
            );
            self.record_peak();
        }
        (start, len)
    }

    pub(super) fn sidecar_lengths(&self) -> [u32; 13] {
        let m = self.mark();
        [
            m.boxes,
            m.unsets,
            m.rules,
            m.leaders,
            m.discs,
            m.marks,
            m.insertions,
            m.whatsits,
            m.noads,
            m.fractions,
            m.choices,
            m.math_lists,
            m.adjusts,
        ]
    }

    pub(super) fn reserve_sidecars(&mut self, needs: SidecarNeeds) {
        self.boxes.reserve(needs.boxes as usize);
        self.unsets.reserve(needs.unsets as usize);
        self.rules.reserve(needs.rules as usize);
        self.leaders.reserve(needs.leaders as usize);
        self.discs.reserve(needs.discs as usize);
        self.marks.reserve(needs.marks as usize);
        self.insertions.reserve(needs.insertions as usize);
        self.whatsits.reserve(needs.whatsits as usize);
        self.noads.reserve(needs.noads as usize);
        self.fractions.reserve(needs.fractions as usize);
        self.choices.reserve(needs.choices as usize);
        self.math_lists.reserve(needs.math_lists as usize);
        self.adjusts.reserve(needs.adjusts as usize);
    }

    fn encode(&mut self, node: &Node) -> NodeWord {
        match node {
            Node::Char { font, ch } => NodeWord::new(0, (*ch as u64) | ((font.raw() as u64) << 21)),
            Node::Lig { font, ch, orig } => {
                // The complete input slice was domain-checked before any
                // storage mutation, so these narrowing conversions are exact.
                let ch = *ch as u8;
                let left = orig.0 as u8;
                let right = orig.1 as u8;
                NodeWord::new(
                    1,
                    ch as u64
                        | ((left as u64) << 8)
                        | ((right as u64) << 16)
                        | ((font.raw() as u64) << 24),
                )
            }
            Node::Kern { amount, kind } => NodeWord::new(
                2,
                amount.raw() as u32 as u64 | ((kern_code(*kind) as u64) << 32),
            ),
            Node::Glue {
                spec,
                kind,
                leader: None,
            } => NodeWord::new(3, spec.raw() as u64 | ((glue_code(*kind) as u64) << 32)),
            Node::Penalty(value) => NodeWord::new(4, *value as u32 as u64),
            Node::MathOn(value) => NodeWord::new(5, value.raw() as u32 as u64),
            Node::MathOff(value) => NodeWord::new(6, value.raw() as u32 as u64),
            Node::Direction(direction) => NodeWord::new(23, *direction as u64),
            Node::MathStyle(style) => NodeWord::new(7, style_code(*style) as u64),
            Node::Nonscript => NodeWord::new(8, 0),
            Node::HList(value) => NodeWord::sidecar(9, self.boxes.push(*value)),
            Node::VList(value) => NodeWord::sidecar(10, self.boxes.push(*value)),
            Node::Unset(value) => NodeWord::sidecar(11, self.unsets.push(*value)),
            Node::Rule {
                width,
                height,
                depth,
            } => push_sidecar(12, &mut self.rules, (*width, *height, *depth)),
            Node::Glue {
                spec,
                kind,
                leader: Some(value),
            } => push_sidecar(13, &mut self.leaders, (*spec, *kind, *value)),
            Node::Disc {
                kind,
                pre,
                post,
                replace,
            } => push_sidecar(14, &mut self.discs, (*kind, *pre, *post, *replace)),
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
                    *content,
                )),
            ),
            Node::Whatsit(value) => push_sidecar(17, &mut self.whatsits, value.clone()),
            Node::MathNoad(value) => NodeWord::sidecar(18, self.noads.push(value.clone())),
            Node::FractionNoad(value) => push_sidecar(19, &mut self.fractions, value.clone()),
            Node::MathChoice(value) => push_sidecar(20, &mut self.choices, value.clone()),
            Node::MathList(value) => push_sidecar(21, &mut self.math_lists, *value),
            Node::Adjust(value) => push_sidecar(22, &mut self.adjusts, *value),
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

    #[cfg(test)]
    pub(crate) fn all_nodes(&self) -> NodeList<'_> {
        self.view(
            0,
            checked_len(self.words.len(), "node arena exceeds u32 entries"),
        )
    }

    #[cfg(test)]
    pub(super) fn testing_sidecar_lengths(&self) -> [u32; 13] {
        let m = self.mark();
        [
            m.boxes,
            m.unsets,
            m.rules,
            m.leaders,
            m.discs,
            m.marks,
            m.insertions,
            m.whatsits,
            m.noads,
            m.fractions,
            m.choices,
            m.math_lists,
            m.adjusts,
        ]
    }

    #[cfg(test)]
    pub(super) fn testing_tags(&self) -> Vec<u8> {
        self.words.iter().map(|word| word.tag()).collect()
    }
}

fn push_sidecar<T>(tag: u8, table: &mut Vec<T>, value: T) -> NodeWord {
    let i = checked_len(table.len(), "node sidecar exceeds u32 entries");
    table.push(value);
    NodeWord::sidecar(tag, i)
}
fn preflight_encoding(node: &Node) {
    if let Node::Lig { ch, orig, .. } = node {
        assert!(
            (*ch as u32) <= u8::MAX as u32,
            "ligature glyph exceeds TFM byte domain"
        );
        assert!(
            (orig.0 as u32) <= u8::MAX as u32,
            "ligature original exceeds TFM byte domain"
        );
        assert!(
            (orig.1 as u32) <= u8::MAX as u32,
            "ligature original exceeds TFM byte domain"
        );
    }
}
fn kern_code(v: KernKind) -> u8 {
    match v {
        KernKind::Explicit => 0,
        KernKind::Font => 1,
        KernKind::Accent => 2,
        KernKind::Mu => 3,
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
    }
}

pub(super) fn decode_kern(value: u8) -> KernKind {
    match value {
        0 => KernKind::Explicit,
        1 => KernKind::Font,
        2 => KernKind::Accent,
        3 => KernKind::Mu,
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
        _ => unreachable!(),
    }
}

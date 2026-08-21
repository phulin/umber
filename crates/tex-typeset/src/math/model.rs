use tex_fonts::CharMetrics;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::node::{GlueKind, KernKind, LeaderPayload, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

use crate::metrics::{ListMetrics, MetricEvent, MetricOverflow};

#[cfg(test)]
mod tests;

/// One detached native-node transaction produced by Appendix G.
///
/// Entries are stored in postorder so every box refers only to an earlier
/// child span.  The executor can therefore commit the transaction in one
/// iterative pass without rebuilding a second, recursive node vocabulary.
#[derive(Clone, Debug, PartialEq)]
pub struct MathLayout {
    nodes: Vec<MathNode>,
    root: FrozenHList,
    pack_observations: Vec<MathPackObservation>,
    conversion_events: Vec<MathConversionEvent>,
    recovered: bool,
}

impl MathLayout {
    #[must_use]
    pub const fn root(&self) -> FrozenHList {
        self.root
    }

    #[must_use]
    pub fn nodes(&self, list: FrozenHList) -> &[MathNode] {
        let start = list.start as usize;
        let end = start + list.len as usize;
        assert!(end <= self.nodes.len(), "math layout span is not live");
        &self.nodes[start..end]
    }

    #[must_use]
    pub fn conversion_events(&self) -> &[MathConversionEvent] {
        &self.conversion_events
    }

    #[must_use]
    pub fn pack_observations(&self) -> &[MathPackObservation] {
        &self.pack_observations
    }

    /// Whether Appendix G requested deletion of the whole formula.
    #[must_use]
    pub const fn recovered(&self) -> bool {
        self.recovered
    }

    #[cfg(test)]
    pub(crate) fn transaction_entry_count(&self) -> usize {
        self.nodes.len()
    }

    #[cfg(test)]
    pub(crate) fn logical_nodes(&self, list: FrozenHList) -> Vec<&MathNode> {
        let mut out = Vec::new();
        self.collect_logical_nodes(list, &mut out);
        out
    }

    #[cfg(test)]
    fn collect_logical_nodes<'a>(&'a self, list: FrozenHList, out: &mut Vec<&'a MathNode>) {
        let mut stack = vec![(list, 0usize)];
        while let Some((list, index)) = stack.pop() {
            let nodes = self.nodes(list);
            if let Some(node) = nodes.get(index) {
                stack.push((list, index + 1));
                match node {
                    MathNode::Sequence(child) => stack.push((*child, 0)),
                    node => out.push(node),
                }
            }
        }
    }
}

/// Execution-visible evidence produced by pure Appendix G character fetching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathConversionEvent {
    MissingCharacter {
        font: FontId,
        character: char,
    },
    UndefinedFamily {
        size: tex_state::math::MathFontSize,
        family: u8,
        character: char,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathPackObservation {
    pub axis: BoxAxis,
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
}

/// An immutable, measured horizontal-list span in a [`MathLayout`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenHList {
    start: u32,
    len: u32,
    node_count: u32,
    width: Scaled,
    height: Scaled,
    depth: Scaled,
}

impl Default for FrozenHList {
    fn default() -> Self {
        Self {
            start: 0,
            len: 0,
            node_count: 0,
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
        }
    }
}

impl FrozenHList {
    #[must_use]
    pub const fn width(self) -> Scaled {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> Scaled {
        self.height
    }

    #[must_use]
    pub const fn depth(self) -> Scaled {
        self.depth
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.node_count == 0
    }

    #[must_use]
    pub const fn node_count(self) -> usize {
        self.node_count as usize
    }
}

/// An entry in the detached math transaction.
///
/// Ordinary leaves use the canonical [`Node`] vocabulary.  Only selected
/// OpenType glyphs (which retain a glyph id and selected metrics until output
/// grows a glyph-aware native node) and direct glue (which remains a copied
/// value) are math-specific drafts.
#[derive(Clone, Debug, PartialEq)]
pub enum MathNode {
    Char {
        font: FontId,
        ch: char,
        /// Exact glyph selected by OpenType MATH, including `ssty`.
        glyph_id: Option<u16>,
        metrics: CharMetrics,
        origin: tex_state::token::OriginId,
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
    HList(MathBox),
    VList(MathBox),
    /// A canonical native node carried unchanged to the executor commit.
    Native(Box<Node>),
    /// Transparent concatenation of an already-built earlier span.
    #[doc(hidden)]
    Sequence(FrozenHList),
}

/// Glue subtype retained without requiring a `GlueId`.
pub type MathGlueKind = GlueKind;

/// Owned box node whose children are stored in the surrounding layout arena.
#[derive(Clone, Debug, PartialEq)]
pub struct MathBox {
    pub width: Scaled,
    pub height: Scaled,
    pub depth: Scaled,
    /// TeX.web `shift_amount`: positive moves down in an hlist and right in a vlist.
    pub shift: Scaled,
    pub list: FrozenHList,
    pub axis: BoxAxis,
    pub display: bool,
    pub glue_set: GlueSetRatio,
    pub glue_sign: Sign,
    pub glue_order: Order,
}

/// Box orientation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BoxAxis {
    Horizontal,
    Vertical,
}

pub(crate) struct NativeNodeTransaction {
    nodes: Vec<MathNode>,
    pack_observations: Vec<MathPackObservation>,
}

impl NativeNodeTransaction {
    pub(crate) fn new() -> Self {
        Self {
            nodes: Vec::new(),
            pack_observations: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn finish(self, root: FrozenHList) -> MathLayout {
        self.finish_with_conversion(root, Vec::new(), false)
    }

    pub(crate) fn finish_with_conversion(
        self,
        root: FrozenHList,
        conversion_events: Vec<MathConversionEvent>,
        recovered: bool,
    ) -> MathLayout {
        debug_assert!(
            root.end() <= self.nodes.len(),
            "math layout root must belong to this arena"
        );
        MathLayout {
            nodes: self.nodes,
            root,
            pack_observations: self.pack_observations,
            conversion_events,
            recovered,
        }
    }

    pub(crate) fn empty(&self) -> FrozenHList {
        FrozenHList::default()
    }

    pub(crate) fn hlist(&mut self, nodes: impl IntoIterator<Item = MathNode>) -> FrozenHList {
        let start = self.nodes.len();
        self.nodes.extend(nodes);
        let end = self.nodes.len();
        self.validate_new_span(start, end);
        let mut meas = Measurement::ZERO;
        self.measure_hnodes(start, end, &mut meas);
        self.span(start, end, meas)
    }

    /// Records one completed TeX packaging call at its common return seam.
    pub(crate) fn observe_completed_pack(&mut self, boxed: &MathBox) {
        self.pack_observations.push(MathPackObservation {
            axis: boxed.axis,
            width: boxed.width,
            height: boxed.height,
            depth: boxed.depth,
        });
    }

    pub(crate) fn take_pack_observations_since(
        &mut self,
        start: usize,
    ) -> Vec<MathPackObservation> {
        self.pack_observations.split_off(start)
    }

    pub(crate) fn replay_pack_observations(&mut self, observations: &[MathPackObservation]) {
        self.pack_observations.extend_from_slice(observations);
    }

    pub(crate) fn pack_observation_count(&self) -> usize {
        self.pack_observations.len()
    }

    /// Stores the already-boxed child payload of a source box.
    ///
    /// The owning source box carries authoritative width, height, and depth,
    /// so Appendix G must not repack or remeasure this payload.
    pub(crate) fn box_payload(&mut self, nodes: impl IntoIterator<Item = MathNode>) -> FrozenHList {
        let start = self.nodes.len();
        self.nodes.extend(nodes);
        let end = self.nodes.len();
        self.validate_new_span(start, end);
        let node_count = u32::try_from(end - start).expect("math list exceeds u32 nodes");
        self.span(
            start,
            end,
            Measurement {
                node_count,
                ..Measurement::ZERO
            },
        )
    }

    fn validate_new_span(&self, start: usize, end: usize) {
        for node in &self.nodes[start..end] {
            let child = match node {
                MathNode::Sequence(child) => Some(*child),
                MathNode::HList(boxed) | MathNode::VList(boxed) => Some(boxed.list),
                _ => None,
            };
            debug_assert!(
                child.is_none_or(|list| list.end() <= start),
                "math arena references must point to an earlier span"
            );
        }
    }

    pub(crate) fn hpack(&mut self, list: FrozenHList) -> MathBox {
        let boxed = MathBox {
            width: list.width,
            height: list.height,
            depth: list.depth,
            shift: Scaled::from_raw(0),
            list,
            axis: BoxAxis::Horizontal,
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
        };
        // TeX82 §651's `hpack` has one canonical return seam. Appendix G
        // reaches it for every structural clean/sub-mlist box, not only for
        // source boxes that arrived already packaged. Retain the finalized
        // dimensions here so execution can publish every such transition.
        self.observe_completed_pack(&boxed);
        boxed
    }

    /// TeX82 §720's already-clean `new_null_box` for an empty math field.
    pub(crate) fn null_hbox(&mut self) -> MathBox {
        let list = self.empty();
        MathBox {
            width: list.width,
            height: list.height,
            depth: list.depth,
            shift: Scaled::from_raw(0),
            list,
            axis: BoxAxis::Horizontal,
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
        }
    }

    pub(crate) fn vpack(&mut self, list: FrozenHList) -> MathBox {
        let mut meas = Measurement::ZERO;
        self.measure_vnodes(list, &mut meas);
        let boxed = MathBox {
            width: meas.width,
            height: meas.height,
            depth: meas.depth,
            shift: Scaled::from_raw(0),
            list,
            axis: BoxAxis::Vertical,
            display: false,
            glue_set: GlueSetRatio::from_raw(0),
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
        };
        // TeX82 §668's vertical package is complete and observable here,
        // independently of how Appendix G later consumes its box.
        self.observe_completed_pack(&boxed);
        boxed
    }

    pub(crate) fn nodes(&self, list: FrozenHList) -> &[MathNode] {
        let start = list.start as usize;
        let end = start + list.len as usize;
        &self.nodes[start..end]
    }

    pub(crate) fn first_node(&self, list: FrozenHList) -> Option<&MathNode> {
        let mut stack = vec![(list, 0usize)];
        while let Some((list, index)) = stack.pop() {
            let nodes = self.nodes(list);
            if let Some(node) = nodes.get(index) {
                stack.push((list, index + 1));
                match node {
                    MathNode::Sequence(child) => stack.push((*child, 0)),
                    node => return Some(node),
                }
            }
        }
        None
    }

    pub(crate) fn single_node(&self, list: FrozenHList) -> Option<&MathNode> {
        if list.node_count == 1 {
            self.first_node(list)
        } else {
            None
        }
    }

    pub(crate) fn trivial_character_before_kern(&self, list: FrozenHList) -> Option<MathNode> {
        if list.node_count != 2 {
            return None;
        }
        let mut logical = Vec::with_capacity(2);
        self.collect_nodes_bounded(list, &mut logical, 3);
        match logical.as_slice() {
            [character @ MathNode::Char { .. }, MathNode::Kern { .. }] => {
                Some((*character).clone())
            }
            _ => None,
        }
    }

    fn collect_nodes_bounded<'a>(
        &'a self,
        list: FrozenHList,
        out: &mut Vec<&'a MathNode>,
        limit: usize,
    ) {
        for node in self.nodes(list) {
            if out.len() >= limit {
                return;
            }
            match node {
                MathNode::Sequence(child) => self.collect_nodes_bounded(*child, out, limit),
                node => out.push(node),
            }
        }
    }

    fn span(&self, start: usize, end: usize, meas: Measurement) -> FrozenHList {
        FrozenHList {
            start: u32::try_from(start).expect("math layout exceeds u32 nodes"),
            len: u32::try_from(end - start).expect("math list exceeds u32 nodes"),
            node_count: meas.node_count,
            width: meas.width,
            height: meas.height,
            depth: meas.depth,
        }
    }

    fn measure_hnodes(&self, start: usize, end: usize, meas: &mut Measurement) {
        for node in &self.nodes[start..end] {
            match node {
                MathNode::Sequence(list) => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(list.node_count)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(
                        MetricEvent::Box {
                            width: list.width,
                            height: list.height,
                            depth: list.depth,
                            shift: Scaled::from_raw(0),
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::Char { metrics, .. } => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(
                        MetricEvent::Glyph {
                            width: metrics.width,
                            height: metrics.height,
                            depth: metrics.depth,
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::Kern { amount, .. } => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(MetricEvent::Kern(*amount), MetricOverflow::APPENDIX_G);
                }
                MathNode::Glue { spec, .. } => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(MetricEvent::Glue(*spec), MetricOverflow::APPENDIX_G);
                }
                MathNode::Penalty(_) => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                }
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(
                        MetricEvent::Rule {
                            width: width.unwrap_or(Scaled::from_raw(0)),
                            height: height.unwrap_or(Scaled::from_raw(0)),
                            depth: depth.unwrap_or(Scaled::from_raw(0)),
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    meas.observe_horizontal(
                        MetricEvent::Box {
                            width: boxed.width,
                            height: boxed.height,
                            depth: boxed.depth,
                            shift: boxed.shift,
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::Native(node) => {
                    meas.node_count = meas
                        .node_count
                        .checked_add(1)
                        .expect("math node count exceeds u32");
                    measure_opaque_hnode(node, meas);
                }
            }
        }
    }

    fn measure_vnodes(&self, list: FrozenHList, meas: &mut Measurement) {
        let mut stack = vec![(list, 0usize)];
        while let Some((list, index)) = stack.pop() {
            let nodes = self.nodes(list);
            let Some(node) = nodes.get(index) else {
                continue;
            };
            stack.push((list, index + 1));
            match node {
                MathNode::Sequence(child) => stack.push((*child, 0)),
                MathNode::HList(boxed) | MathNode::VList(boxed) => {
                    meas.observe_vertical(
                        MetricEvent::Box {
                            width: boxed.width,
                            height: boxed.height,
                            depth: boxed.depth,
                            shift: boxed.shift,
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::Kern { amount, .. } => {
                    meas.observe_vertical(MetricEvent::Kern(*amount), MetricOverflow::APPENDIX_G);
                }
                MathNode::Glue { spec, .. } => {
                    meas.observe_vertical(MetricEvent::Glue(*spec), MetricOverflow::APPENDIX_G);
                }
                MathNode::Rule {
                    width,
                    height,
                    depth,
                } => {
                    meas.observe_vertical(
                        MetricEvent::Rule {
                            width: width.unwrap_or(Scaled::from_raw(0)),
                            height: height.unwrap_or(Scaled::from_raw(0)),
                            depth: depth.unwrap_or(Scaled::from_raw(0)),
                        },
                        MetricOverflow::APPENDIX_G,
                    );
                }
                MathNode::Native(node) => measure_native_vnode(node, meas),
                MathNode::Penalty(_) | MathNode::Char { .. } => {}
            }
        }
    }
}

impl FrozenHList {
    fn end(self) -> usize {
        self.start as usize + self.len as usize
    }
}

pub(crate) fn boxed_node(boxed: MathBox) -> MathNode {
    match boxed.axis {
        BoxAxis::Horizontal => MathNode::HList(boxed),
        BoxAxis::Vertical => MathNode::VList(boxed),
    }
}

pub(crate) fn node_is_char(node: &MathNode) -> bool {
    matches!(node, MathNode::Char { .. })
}

fn measure_opaque_hnode(node: &Node, meas: &mut Measurement) {
    let event = match node {
        Node::Kern { amount, .. } => MetricEvent::Kern(*amount),
        Node::Rule {
            width,
            height,
            depth,
        } => MetricEvent::Rule {
            width: width.unwrap_or(Scaled::from_raw(0)),
            height: height.unwrap_or(Scaled::from_raw(0)),
            depth: depth.unwrap_or(Scaled::from_raw(0)),
        },
        Node::HList(boxed) | Node::VList(boxed) => MetricEvent::Box {
            width: boxed.width,
            height: boxed.height,
            depth: boxed.depth,
            shift: boxed.shift,
        },
        _ => MetricEvent::Ignored,
    };
    meas.observe_horizontal(event, MetricOverflow::APPENDIX_G);
}

fn measure_native_vnode(node: &Node, meas: &mut Measurement) {
    let event = match node {
        Node::HList(boxed) | Node::VList(boxed) => MetricEvent::Box {
            width: boxed.width,
            height: boxed.height,
            depth: boxed.depth,
            shift: boxed.shift,
        },
        Node::Kern { amount, .. } => MetricEvent::Kern(*amount),
        Node::Rule {
            width,
            height,
            depth,
        } => MetricEvent::Rule {
            width: width.unwrap_or(Scaled::from_raw(0)),
            height: height.unwrap_or(Scaled::from_raw(0)),
            depth: depth.unwrap_or(Scaled::from_raw(0)),
        },
        _ => MetricEvent::Ignored,
    };
    meas.observe_vertical(event, MetricOverflow::APPENDIX_G);
}

#[derive(Clone, Copy, Debug)]
struct Measurement {
    node_count: u32,
    metrics: ListMetrics,
}

impl Measurement {
    const ZERO: Self = Self {
        node_count: 0,
        metrics: ListMetrics::ZERO,
    };
}

impl std::ops::Deref for Measurement {
    type Target = ListMetrics;

    fn deref(&self) -> &Self::Target {
        &self.metrics
    }
}

impl std::ops::DerefMut for Measurement {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.metrics
    }
}

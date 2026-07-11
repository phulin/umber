//! Compact epoch storage for immutable node lists.
//!
//! The word stream and every sidecar are one aggregate allocation domain.
//! Consumers traverse opaque logical views over the canonical words and
//! sidecars; no decoded compatibility mirror is retained.

use crate::ids::{ArenaRef, GlueId, NodeListId};
use crate::math::MathStyle;
use crate::node::{
    BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, Node, UnsetNode, UnsetNodeFields,
};
use crate::scaled::Scaled;
use crate::survivor::SurvivorArena;

#[cfg(feature = "node-stats")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "node-stats")]
use std::sync::{Mutex, OnceLock};

const TAG_SHIFT: u32 = 59;
const PAYLOAD_MASK: u64 = (1_u64 << TAG_SHIFT) - 1;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeWord(u64);

const _: [(); 8] = [(); core::mem::size_of::<NodeWord>()];

impl NodeWord {
    const fn new(tag: u8, payload: u64) -> Self {
        assert!(tag < 32, "node-word tag exceeds five bits");
        assert!(payload <= PAYLOAD_MASK, "node-word payload exceeds 59 bits");
        Self(((tag as u64) << TAG_SHIFT) | payload)
    }

    const fn tag(self) -> u8 {
        (self.0 >> TAG_SHIFT) as u8
    }

    const fn payload(self) -> u64 {
        self.0 & PAYLOAD_MASK
    }

    const fn sidecar(tag: u8, index: u32) -> Self {
        Self::new(tag, index as u64)
    }
}

/// One opaque aggregate rollback watermark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeArenaMark(StorageMark);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct StorageMark {
    words: u32,
    boxes: u32,
    unsets: u32,
    rules: u32,
    leaders: u32,
    discs: u32,
    marks: u32,
    insertions: u32,
    whatsits: u32,
    noads: u32,
    fractions: u32,
    choices: u32,
    math_lists: u32,
    adjusts: u32,
}

/// Canonical compact storage shared by epoch and survivor arenas.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeStorage {
    words: Vec<NodeWord>,
    boxes: BoxTable,
    unsets: UnsetTable,
    rules: Vec<(Option<Scaled>, Option<Scaled>, Option<Scaled>)>,
    leaders: Vec<(GlueId, GlueKind, crate::node::LeaderPayload)>,
    discs: Vec<(DiscKind, NodeListId, NodeListId, NodeListId)>,
    marks: Vec<(u16, crate::ids::TokenListId)>,
    insertions: InsertionTable,
    whatsits: Vec<crate::node::Whatsit>,
    noads: NoadTable,
    fractions: Vec<crate::math::MathFraction>,
    choices: Vec<crate::math::MathChoice>,
    math_lists: Vec<crate::math::MathListNode>,
    adjusts: Vec<NodeListId>,
}

/// One allocator-backed compact-node column in a diagnostic memory report.
///
/// This is process-local measurement data. It is computed on demand and is
/// never stored in `Universe`, snapshots, hashes, or replay state.
#[cfg(feature = "node-stats")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeMemoryColumn {
    pub name: String,
    pub len: usize,
    pub capacity: usize,
    pub element_bytes: usize,
    pub logical_bytes: usize,
    pub retained_payload_bytes: usize,
}

#[cfg(feature = "node-stats")]
impl NodeMemoryColumn {
    fn from_vec<T>(name: String, values: &Vec<T>) -> Self {
        let element_bytes = core::mem::size_of::<T>();
        Self {
            name,
            len: values.len(),
            capacity: values.capacity(),
            element_bytes,
            logical_bytes: values.len() * element_bytes,
            retained_payload_bytes: values.capacity() * element_bytes,
        }
    }

    fn byte_payload(name: String, len: usize, capacity: usize) -> Self {
        Self {
            name,
            len,
            capacity,
            element_bytes: 1,
            logical_bytes: len,
            retained_payload_bytes: capacity,
        }
    }
}

#[cfg(feature = "node-stats")]
static PEAK_STORAGE_LOGICAL: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "node-stats")]
static PEAK_STORAGE_RETAINED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "node-stats")]
static PEAK_STORAGE_COLUMNS: OnceLock<Mutex<Vec<NodeMemoryColumn>>> = OnceLock::new();

/// Largest individual canonical storage observed during this process.
/// Survivor scratch is reported separately; aggregate end-state storage is
/// available through `Universe::node_memory_columns`.
#[cfg(feature = "node-stats")]
#[must_use]
pub fn peak_node_storage_measurement() -> (u64, u64, Vec<NodeMemoryColumn>) {
    let columns = PEAK_STORAGE_COLUMNS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("node measurement mutex poisoned")
        .clone();
    (
        PEAK_STORAGE_LOGICAL.load(Ordering::Relaxed),
        PEAK_STORAGE_RETAINED.load(Ordering::Relaxed),
        columns,
    )
}

#[derive(Clone, Debug, Default)]
struct BoxTable {
    width: Vec<Scaled>,
    height: Vec<Scaled>,
    depth: Vec<Scaled>,
    shift: Vec<Scaled>,
    display: Vec<bool>,
    glue_set: Vec<crate::scaled::GlueSetRatio>,
    glue_sign: Vec<crate::node::Sign>,
    glue_order: Vec<crate::glue::Order>,
    children: Vec<NodeListId>,
}

impl BoxTable {
    fn len(&self) -> usize {
        self.width.len()
    }
    fn push(&mut self, value: crate::node::BoxNode) -> u32 {
        let index = checked_len(self.len(), "box sidecar exceeds u32 entries");
        self.width.push(value.width);
        self.height.push(value.height);
        self.depth.push(value.depth);
        self.shift.push(value.shift);
        self.display.push(value.display);
        self.glue_set.push(value.glue_set);
        self.glue_sign.push(value.glue_sign);
        self.glue_order.push(value.glue_order);
        self.children.push(value.children);
        index
    }
    fn replace(&mut self, index: usize, value: crate::node::BoxNode) {
        self.width[index] = value.width;
        self.height[index] = value.height;
        self.depth[index] = value.depth;
        self.shift[index] = value.shift;
        self.display[index] = value.display;
        self.glue_set[index] = value.glue_set;
        self.glue_sign[index] = value.glue_sign;
        self.glue_order[index] = value.glue_order;
        self.children[index] = value.children;
    }
    fn truncate(&mut self, len: usize) {
        self.width.truncate(len);
        self.height.truncate(len);
        self.depth.truncate(len);
        self.shift.truncate(len);
        self.display.truncate(len);
        self.glue_set.truncate(len);
        self.glue_sign.truncate(len);
        self.glue_order.truncate(len);
        self.children.truncate(len);
    }
}

#[derive(Clone, Debug, Default)]
struct UnsetTable {
    kind: Vec<crate::node::UnsetKind>,
    width: Vec<Scaled>,
    height: Vec<Scaled>,
    depth: Vec<Scaled>,
    span_count: Vec<u16>,
    stretch: Vec<Scaled>,
    stretch_order: Vec<crate::glue::Order>,
    shrink: Vec<Scaled>,
    shrink_order: Vec<crate::glue::Order>,
    children: Vec<NodeListId>,
}
impl UnsetTable {
    fn len(&self) -> usize {
        self.kind.len()
    }
    fn push(&mut self, v: crate::node::UnsetNode) -> u32 {
        let i = checked_len(self.len(), "unset sidecar exceeds u32 entries");
        self.kind.push(v.kind);
        self.width.push(v.width);
        self.height.push(v.height);
        self.depth.push(v.depth);
        self.span_count.push(v.span_count);
        self.stretch.push(v.stretch);
        self.stretch_order.push(v.stretch_order);
        self.shrink.push(v.shrink);
        self.shrink_order.push(v.shrink_order);
        self.children.push(v.children);
        i
    }
    fn replace(&mut self, i: usize, v: crate::node::UnsetNode) {
        self.kind[i] = v.kind;
        self.width[i] = v.width;
        self.height[i] = v.height;
        self.depth[i] = v.depth;
        self.span_count[i] = v.span_count;
        self.stretch[i] = v.stretch;
        self.stretch_order[i] = v.stretch_order;
        self.shrink[i] = v.shrink;
        self.shrink_order[i] = v.shrink_order;
        self.children[i] = v.children
    }
    fn truncate(&mut self, n: usize) {
        self.kind.truncate(n);
        self.width.truncate(n);
        self.height.truncate(n);
        self.depth.truncate(n);
        self.span_count.truncate(n);
        self.stretch.truncate(n);
        self.stretch_order.truncate(n);
        self.shrink.truncate(n);
        self.shrink_order.truncate(n);
        self.children.truncate(n)
    }
}

#[derive(Clone, Debug, Default)]
struct InsertionTable {
    class: Vec<u16>,
    size: Vec<Scaled>,
    split_top_skip: Vec<GlueId>,
    split_max_depth: Vec<Scaled>,
    floating_penalty: Vec<i32>,
    content: Vec<NodeListId>,
}
impl InsertionTable {
    fn len(&self) -> usize {
        self.class.len()
    }
    fn push(&mut self, v: (u16, Scaled, GlueId, Scaled, i32, NodeListId)) -> u32 {
        let i = checked_len(self.len(), "insertion sidecar exceeds u32 entries");
        self.class.push(v.0);
        self.size.push(v.1);
        self.split_top_skip.push(v.2);
        self.split_max_depth.push(v.3);
        self.floating_penalty.push(v.4);
        self.content.push(v.5);
        i
    }
    fn replace(&mut self, i: usize, v: (u16, Scaled, GlueId, Scaled, i32, NodeListId)) {
        self.class[i] = v.0;
        self.size[i] = v.1;
        self.split_top_skip[i] = v.2;
        self.split_max_depth[i] = v.3;
        self.floating_penalty[i] = v.4;
        self.content[i] = v.5
    }
    fn truncate(&mut self, n: usize) {
        self.class.truncate(n);
        self.size.truncate(n);
        self.split_top_skip.truncate(n);
        self.split_max_depth.truncate(n);
        self.floating_penalty.truncate(n);
        self.content.truncate(n)
    }
}

#[derive(Clone, Debug, Default)]
struct NoadTable {
    kind: Vec<crate::math::NoadKind>,
    nucleus: Vec<crate::math::MathField>,
    subscript: Vec<crate::math::MathField>,
    superscript: Vec<crate::math::MathField>,
}
impl NoadTable {
    fn len(&self) -> usize {
        self.kind.len()
    }
    fn push(&mut self, v: crate::math::MathNoad) -> u32 {
        let i = checked_len(self.len(), "noad sidecar exceeds u32 entries");
        self.kind.push(v.kind);
        self.nucleus.push(v.nucleus);
        self.subscript.push(v.subscript);
        self.superscript.push(v.superscript);
        i
    }
    fn replace(&mut self, i: usize, v: crate::math::MathNoad) {
        self.kind[i] = v.kind;
        self.nucleus[i] = v.nucleus;
        self.subscript[i] = v.subscript;
        self.superscript[i] = v.superscript
    }
    fn truncate(&mut self, n: usize) {
        self.kind.truncate(n);
        self.nucleus.truncate(n);
        self.subscript.truncate(n);
        self.superscript.truncate(n)
    }
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

    #[cfg(feature = "node-stats")]
    fn capacity_signature(&self) -> [usize; 39] {
        [
            self.words.capacity(),
            self.boxes.width.capacity(),
            self.boxes.height.capacity(),
            self.boxes.depth.capacity(),
            self.boxes.shift.capacity(),
            self.boxes.display.capacity(),
            self.boxes.glue_set.capacity(),
            self.boxes.glue_sign.capacity(),
            self.boxes.glue_order.capacity(),
            self.boxes.children.capacity(),
            self.unsets.kind.capacity(),
            self.unsets.width.capacity(),
            self.unsets.height.capacity(),
            self.unsets.depth.capacity(),
            self.unsets.span_count.capacity(),
            self.unsets.stretch.capacity(),
            self.unsets.stretch_order.capacity(),
            self.unsets.shrink.capacity(),
            self.unsets.shrink_order.capacity(),
            self.unsets.children.capacity(),
            self.rules.capacity(),
            self.leaders.capacity(),
            self.discs.capacity(),
            self.marks.capacity(),
            self.insertions.class.capacity(),
            self.insertions.size.capacity(),
            self.insertions.split_top_skip.capacity(),
            self.insertions.split_max_depth.capacity(),
            self.insertions.floating_penalty.capacity(),
            self.insertions.content.capacity(),
            self.whatsits.capacity(),
            self.noads.kind.capacity(),
            self.noads.nucleus.capacity(),
            self.noads.subscript.capacity(),
            self.noads.superscript.capacity(),
            self.fractions.capacity(),
            self.choices.capacity(),
            self.math_lists.capacity(),
            self.adjusts.capacity(),
        ]
    }

    #[cfg(feature = "node-stats")]
    fn retained_payload_bytes(&self) -> usize {
        fn bytes<T>(values: &Vec<T>) -> usize {
            values.capacity() * core::mem::size_of::<T>()
        }
        let mut retained = 0;
        macro_rules! add {
            ($value:expr) => {
                retained += bytes(&$value);
            };
        }
        add!(self.words);
        add!(self.boxes.width);
        add!(self.boxes.height);
        add!(self.boxes.depth);
        add!(self.boxes.shift);
        add!(self.boxes.display);
        add!(self.boxes.glue_set);
        add!(self.boxes.glue_sign);
        add!(self.boxes.glue_order);
        add!(self.boxes.children);
        add!(self.unsets.kind);
        add!(self.unsets.width);
        add!(self.unsets.height);
        add!(self.unsets.depth);
        add!(self.unsets.span_count);
        add!(self.unsets.stretch);
        add!(self.unsets.stretch_order);
        add!(self.unsets.shrink);
        add!(self.unsets.shrink_order);
        add!(self.unsets.children);
        add!(self.rules);
        add!(self.leaders);
        add!(self.discs);
        add!(self.marks);
        add!(self.insertions.class);
        add!(self.insertions.size);
        add!(self.insertions.split_top_skip);
        add!(self.insertions.split_max_depth);
        add!(self.insertions.floating_penalty);
        add!(self.insertions.content);
        add!(self.whatsits);
        add!(self.noads.kind);
        add!(self.noads.nucleus);
        add!(self.noads.subscript);
        add!(self.noads.superscript);
        add!(self.fractions);
        add!(self.choices);
        add!(self.math_lists);
        add!(self.adjusts);
        retained
    }

    #[cfg(feature = "node-stats")]
    pub(crate) fn memory_columns(&self, prefix: &str) -> Vec<NodeMemoryColumn> {
        let mut out = Vec::new();
        macro_rules! column {
            ($name:literal, $value:expr) => {
                out.push(NodeMemoryColumn::from_vec(
                    format!("{prefix}.{}", $name),
                    $value,
                ));
            };
        }
        column!("words", &self.words);
        column!("boxes.width", &self.boxes.width);
        column!("boxes.height", &self.boxes.height);
        column!("boxes.depth", &self.boxes.depth);
        column!("boxes.shift", &self.boxes.shift);
        column!("boxes.display", &self.boxes.display);
        column!("boxes.glue_set", &self.boxes.glue_set);
        column!("boxes.glue_sign", &self.boxes.glue_sign);
        column!("boxes.glue_order", &self.boxes.glue_order);
        column!("boxes.children", &self.boxes.children);
        column!("unsets.kind", &self.unsets.kind);
        column!("unsets.width", &self.unsets.width);
        column!("unsets.height", &self.unsets.height);
        column!("unsets.depth", &self.unsets.depth);
        column!("unsets.span_count", &self.unsets.span_count);
        column!("unsets.stretch", &self.unsets.stretch);
        column!("unsets.stretch_order", &self.unsets.stretch_order);
        column!("unsets.shrink", &self.unsets.shrink);
        column!("unsets.shrink_order", &self.unsets.shrink_order);
        column!("unsets.children", &self.unsets.children);
        column!("rules", &self.rules);
        column!("leaders", &self.leaders);
        column!("discs", &self.discs);
        column!("marks", &self.marks);
        column!("insertions.class", &self.insertions.class);
        column!("insertions.size", &self.insertions.size);
        column!("insertions.split_top_skip", &self.insertions.split_top_skip);
        column!(
            "insertions.split_max_depth",
            &self.insertions.split_max_depth
        );
        column!(
            "insertions.floating_penalty",
            &self.insertions.floating_penalty
        );
        column!("insertions.content", &self.insertions.content);
        column!("whatsits", &self.whatsits);
        column!("noads.kind", &self.noads.kind);
        column!("noads.nucleus", &self.noads.nucleus);
        column!("noads.subscript", &self.noads.subscript);
        column!("noads.superscript", &self.noads.superscript);
        column!("fractions", &self.fractions);
        column!("choices", &self.choices);
        column!("math_lists", &self.math_lists);
        column!("adjusts", &self.adjusts);

        let mut string_len = 0;
        let mut string_capacity = 0;
        let mut payload_len = 0;
        let mut payload_capacity = 0;
        for whatsit in &self.whatsits {
            match whatsit {
                crate::node::Whatsit::OpenOut { path, .. } => {
                    string_len += path.len();
                    string_capacity += path.capacity();
                }
                crate::node::Whatsit::Special { class, payload } => {
                    string_len += class.len();
                    string_capacity += class.capacity();
                    payload_len += payload.len();
                    payload_capacity += payload.capacity();
                }
                crate::node::Whatsit::CloseOut { .. }
                | crate::node::Whatsit::DeferredWrite { .. }
                | crate::node::Whatsit::Language { .. } => {}
            }
        }
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.whatsits.owned_strings"),
            string_len,
            string_capacity,
        ));
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.whatsits.owned_payloads"),
            payload_len,
            payload_capacity,
        ));
        out
    }

    #[cfg(feature = "node-stats")]
    fn record_peak(&self) {
        fn bytes<T>(value: &Vec<T>) -> (u64, u64) {
            (
                (value.len() * core::mem::size_of::<T>()) as u64,
                (value.capacity() * core::mem::size_of::<T>()) as u64,
            )
        }
        let mut logical = 0_u64;
        let mut retained = 0_u64;
        macro_rules! add {
            ($value:expr) => {{
                let measured = bytes(&$value);
                logical += measured.0;
                retained += measured.1;
            }};
        }
        add!(self.words);
        add!(self.boxes.width);
        add!(self.boxes.height);
        add!(self.boxes.depth);
        add!(self.boxes.shift);
        add!(self.boxes.display);
        add!(self.boxes.glue_set);
        add!(self.boxes.glue_sign);
        add!(self.boxes.glue_order);
        add!(self.boxes.children);
        add!(self.unsets.kind);
        add!(self.unsets.width);
        add!(self.unsets.height);
        add!(self.unsets.depth);
        add!(self.unsets.span_count);
        add!(self.unsets.stretch);
        add!(self.unsets.stretch_order);
        add!(self.unsets.shrink);
        add!(self.unsets.shrink_order);
        add!(self.unsets.children);
        add!(self.rules);
        add!(self.leaders);
        add!(self.discs);
        add!(self.marks);
        add!(self.insertions.class);
        add!(self.insertions.size);
        add!(self.insertions.split_top_skip);
        add!(self.insertions.split_max_depth);
        add!(self.insertions.floating_penalty);
        add!(self.insertions.content);
        add!(self.whatsits);
        add!(self.noads.kind);
        add!(self.noads.nucleus);
        add!(self.noads.subscript);
        add!(self.noads.superscript);
        add!(self.fractions);
        add!(self.choices);
        add!(self.math_lists);
        add!(self.adjusts);
        for whatsit in &self.whatsits {
            match whatsit {
                crate::node::Whatsit::OpenOut { path, .. } => {
                    logical += path.len() as u64;
                    retained += path.capacity() as u64;
                }
                crate::node::Whatsit::Special { class, payload } => {
                    logical += (class.len() + payload.len()) as u64;
                    retained += (class.capacity() + payload.capacity()) as u64;
                }
                crate::node::Whatsit::CloseOut { .. }
                | crate::node::Whatsit::DeferredWrite { .. }
                | crate::node::Whatsit::Language { .. } => {}
            }
        }
        let previous = PEAK_STORAGE_LOGICAL.fetch_max(logical, Ordering::Relaxed);
        PEAK_STORAGE_RETAINED.fetch_max(retained, Ordering::Relaxed);
        if logical > previous {
            let columns = self.memory_columns("peak");
            let mut recorded = PEAK_STORAGE_COLUMNS
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("node measurement mutex poisoned");
            if logical == PEAK_STORAGE_LOGICAL.load(Ordering::Relaxed) {
                *recorded = columns;
            }
        }
    }

    fn mark(&self) -> StorageMark {
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

    fn truncate(&mut self, mark: StorageMark) {
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
        #[cfg(feature = "node-stats")]
        let capacity_before = self.capacity_signature();
        #[cfg(feature = "node-stats")]
        let retained_before = self.retained_payload_bytes();
        let start = checked_len(self.words.len(), "node arena exceeds u32 entries");
        let len = checked_len(nodes.len(), "node list exceeds u32 entries");
        start
            .checked_add(len)
            .expect("node arena span overflows u32");
        // Validate every encoding and selected table before reserving or
        // publishing either rows or words. Publication below is infallible
        // apart from process-aborting allocation failure.
        let mut needs = [0_u32; 14];
        for node in nodes {
            preflight_encoding(node);
            let class = sidecar_class(node);
            needs[class] = needs[class].checked_add(1).expect("sidecar count overflow");
        }
        let current = self.sidecar_lengths();
        for (have, add) in current.into_iter().zip(needs) {
            preflight_capacity(have, add, "node sidecar exceeds u32 entries");
        }
        self.words.reserve(nodes.len());
        for node in nodes {
            let word = self.encode(node);
            self.words.push(word);
        }
        #[cfg(feature = "node-stats")]
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
                [
                    needs[0], needs[1], needs[2], needs[3], needs[4], needs[5], needs[6], needs[7],
                    needs[8], needs[9], needs[10], needs[11], needs[12],
                ],
                growth_events,
                retained_after.saturating_sub(retained_before),
            );
            self.record_peak();
        }
        (start, len)
    }

    fn sidecar_lengths(&self) -> [u32; 14] {
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
            0,
        ]
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

    pub(crate) fn replace_node(&mut self, index: usize, node: Node) {
        // Survivor remapping changes handles but not table shape. Replace the
        // corresponding sidecar row and word through the aggregate storage.
        let old = self.words[index];
        let side = old.payload() as usize;
        match old.tag() {
            9 | 10 => {
                if let Node::HList(v) | Node::VList(v) = node {
                    self.boxes.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            11 => {
                if let Node::Unset(v) = node {
                    self.unsets.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            13 => {
                if let Node::Glue {
                    spec,
                    kind,
                    leader: Some(v),
                } = node
                {
                    self.leaders[side] = (spec, kind, v)
                } else {
                    unreachable!()
                }
            }
            14 => {
                if let Node::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                } = node
                {
                    self.discs[side] = (kind, pre, post, replace)
                } else {
                    unreachable!()
                }
            }
            16 => {
                if let Node::Ins {
                    class,
                    size,
                    split_top_skip,
                    split_max_depth,
                    floating_penalty,
                    content,
                } = node
                {
                    self.insertions.replace(
                        side,
                        (
                            class,
                            size,
                            split_top_skip,
                            split_max_depth,
                            floating_penalty,
                            content,
                        ),
                    )
                } else {
                    unreachable!()
                }
            }
            18 => {
                if let Node::MathNoad(v) = node {
                    self.noads.replace(side, v)
                } else {
                    unreachable!()
                }
            }
            19 => {
                if let Node::FractionNoad(v) = node {
                    self.fractions[side] = v
                } else {
                    unreachable!()
                }
            }
            20 => {
                if let Node::MathChoice(v) = node {
                    self.choices[side] = v
                } else {
                    unreachable!()
                }
            }
            21 => {
                if let Node::MathList(v) = node {
                    self.math_lists[side] = v
                } else {
                    unreachable!()
                }
            }
            22 => {
                if let Node::Adjust(v) = node {
                    self.adjusts[side] = v
                } else {
                    unreachable!()
                }
            }
            _ => {}
        }
    }

    pub(crate) fn all_nodes(&self) -> NodeList<'_> {
        self.view(
            0,
            checked_len(self.words.len(), "node arena exceeds u32 entries"),
        )
    }

    #[cfg(test)]
    fn testing_sidecar_lengths(&self) -> [u32; 13] {
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
    fn testing_tags(&self) -> Vec<u8> {
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
fn sidecar_class(node: &Node) -> usize {
    match node {
        Node::HList(_) | Node::VList(_) => 0,
        Node::Unset(_) => 1,
        Node::Rule { .. } => 2,
        Node::Glue {
            leader: Some(_), ..
        } => 3,
        Node::Disc { .. } => 4,
        Node::Mark { .. } => 5,
        Node::Ins { .. } => 6,
        Node::Whatsit(_) => 7,
        Node::MathNoad(_) => 8,
        Node::FractionNoad(_) => 9,
        Node::MathChoice(_) => 10,
        Node::MathList(_) => 11,
        Node::Adjust(_) => 12,
        _ => 13,
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

fn decode_kern(value: u8) -> KernKind {
    match value {
        0 => KernKind::Explicit,
        1 => KernKind::Font,
        2 => KernKind::Accent,
        3 => KernKind::Mu,
        _ => unreachable!(),
    }
}
fn decode_style(value: u8) -> MathStyle {
    match value {
        0 => MathStyle::Display,
        1 => MathStyle::Text,
        2 => MathStyle::Script,
        3 => MathStyle::ScriptScript,
        _ => unreachable!(),
    }
}
fn decode_glue(value: u8) -> GlueKind {
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

/// A zero-allocation logical view of one compact arena node.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeRef<'a> {
    Char {
        font: crate::ids::FontId,
        ch: char,
    },
    Lig {
        font: crate::ids::FontId,
        ch: char,
        orig: (char, char),
    },
    Kern {
        amount: Scaled,
        kind: KernKind,
    },
    Glue {
        spec: GlueId,
        kind: GlueKind,
        leader: Option<&'a crate::node::LeaderPayload>,
    },
    Penalty(i32),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
    },
    HList(BoxNode),
    VList(BoxNode),
    Unset(UnsetNode),
    Disc {
        kind: DiscKind,
        pre: NodeListId,
        post: NodeListId,
        replace: NodeListId,
    },
    Mark {
        class: u16,
        tokens: crate::ids::TokenListId,
    },
    Ins {
        class: u16,
        size: Scaled,
        split_top_skip: GlueId,
        split_max_depth: Scaled,
        floating_penalty: i32,
        content: NodeListId,
    },
    Whatsit(&'a crate::node::Whatsit),
    MathOn(Scaled),
    MathOff(Scaled),
    MathNoad(crate::math::MathNoad),
    FractionNoad(&'a crate::math::MathFraction),
    MathStyle(MathStyle),
    MathChoice(&'a crate::math::MathChoice),
    MathList(crate::math::MathListNode),
    Nonscript,
    Adjust(NodeListId),
}

impl NodeRef<'_> {
    /// Materializes an owned node for builder/list-surgery output, never for storage.
    #[must_use]
    pub fn to_owned(&self) -> Node {
        match self {
            Self::Char { font, ch } => Node::Char {
                font: *font,
                ch: *ch,
            },
            Self::Lig { font, ch, orig } => Node::Lig {
                font: *font,
                ch: *ch,
                orig: *orig,
            },
            Self::Kern { amount, kind } => Node::Kern {
                amount: *amount,
                kind: *kind,
            },
            Self::Glue { spec, kind, leader } => Node::Glue {
                spec: *spec,
                kind: *kind,
                leader: leader.cloned(),
            },
            Self::Penalty(v) => Node::Penalty(*v),
            Self::Rule {
                width,
                height,
                depth,
            } => Node::Rule {
                width: *width,
                height: *height,
                depth: *depth,
            },
            Self::HList(v) => Node::HList(*v),
            Self::VList(v) => Node::VList(*v),
            Self::Unset(v) => Node::Unset(*v),
            Self::Disc {
                kind,
                pre,
                post,
                replace,
            } => Node::Disc {
                kind: *kind,
                pre: *pre,
                post: *post,
                replace: *replace,
            },
            Self::Mark { class, tokens } => Node::Mark {
                class: *class,
                tokens: *tokens,
            },
            Self::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Node::Ins {
                class: *class,
                size: *size,
                split_top_skip: *split_top_skip,
                split_max_depth: *split_max_depth,
                floating_penalty: *floating_penalty,
                content: *content,
            },
            Self::Whatsit(v) => Node::Whatsit((*v).clone()),
            Self::MathOn(v) => Node::MathOn(*v),
            Self::MathOff(v) => Node::MathOff(*v),
            Self::MathNoad(v) => Node::MathNoad(v.clone()),
            Self::FractionNoad(v) => Node::FractionNoad((*v).clone()),
            Self::MathStyle(v) => Node::MathStyle(*v),
            Self::MathChoice(v) => Node::MathChoice((*v).clone()),
            Self::MathList(v) => Node::MathList(*v),
            Self::Nonscript => Node::Nonscript,
            Self::Adjust(v) => Node::Adjust(*v),
        }
    }
}

/// An immutable compact node-list span.
#[derive(Clone, Copy)]
pub struct NodeList<'a> {
    storage: &'a NodeStorage,
    start: usize,
    end: usize,
}

impl core::fmt::Debug for NodeList<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}
impl<const N: usize> PartialEq<&[Node; N]> for NodeList<'_> {
    fn eq(&self, rhs: &&[Node; N]) -> bool {
        self.to_vec().as_slice() == *rhs
    }
}
impl PartialEq<&[Node]> for NodeList<'_> {
    fn eq(&self, rhs: &&[Node]) -> bool {
        self.to_vec().as_slice() == *rhs
    }
}
impl PartialEq<Vec<Node>> for NodeList<'_> {
    fn eq(&self, rhs: &Vec<Node>) -> bool {
        self.to_vec() == *rhs
    }
}

impl<'a> NodeList<'a> {
    #[must_use]
    pub const fn len(self) -> usize {
        self.end - self.start
    }
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
    #[must_use]
    pub fn get(self, index: usize) -> Option<NodeRef<'a>> {
        (self.start + index < self.end).then(|| self.storage.decode(self.start + index))
    }
    #[must_use]
    pub fn first(self) -> Option<NodeRef<'a>> {
        self.get(0)
    }
    #[must_use]
    pub fn last(self) -> Option<NodeRef<'a>> {
        (!self.is_empty()).then(|| self.storage.decode(self.end - 1))
    }
    pub fn iter(self) -> NodeIter<'a> {
        NodeIter {
            storage: self.storage,
            next: self.start,
            end: self.end,
        }
    }
    /// Returns the maximal same-font run of inline byte-character words at
    /// `index`. Ligatures and every non-character word deliberately terminate
    /// a run so callers retain their ordinary semantic handling.
    #[must_use]
    pub fn char_run(self, index: usize) -> Option<CharRun<'a>> {
        if index >= self.len() {
            return None;
        }
        let first = *self.storage.words.get(self.start + index)?;
        if first.tag() != 0 {
            return None;
        }
        let font = crate::ids::FontId::new((first.payload() >> 21) as u32);
        let mut end = self.start + index + 1;
        while end < self.end {
            let word = self.storage.words[end];
            if word.tag() != 0 || (word.payload() >> 21) as u32 != font.raw() {
                break;
            }
            // TFM widths are defined only for the byte character domain.
            if word.payload() & 0x1f_ffff > u8::MAX as u64 {
                break;
            }
            end += 1;
        }
        if first.payload() & 0x1f_ffff > u8::MAX as u64 {
            return None;
        }
        Some(CharRun {
            words: &self.storage.words[self.start + index..end],
            font,
        })
    }

    /// Creates a lazy, single-pass iterator over the same-font byte-character
    /// run beginning at `index`.
    #[must_use]
    pub fn char_codes(self, index: usize) -> Option<CharCodes<'a>> {
        if index >= self.len() {
            return None;
        }
        let first = self.storage.words[self.start + index];
        let payload = first.payload();
        if first.tag() != 0 || payload & 0x1f_ffff > u8::MAX as u64 {
            return None;
        }
        Some(CharCodes {
            words: &self.storage.words[self.start + index..self.end],
            next: 0,
            font: crate::ids::FontId::new((payload >> 21) as u32),
        })
    }
    #[must_use]
    pub fn to_vec(self) -> Vec<Node> {
        self.iter().map(|node| node.to_owned()).collect()
    }
    /// Test/debug-only decoded view for legacy structural assertions.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    #[doc(hidden)]
    pub fn testing_decoded(self) -> &'static [Node] {
        Box::leak(self.to_vec().into_boxed_slice())
    }
}

/// Lazy byte codes from one contiguous same-font inline character run.
pub struct CharCodes<'a> {
    words: &'a [NodeWord],
    next: usize,
    font: crate::ids::FontId,
}

impl CharCodes<'_> {
    #[must_use]
    pub const fn font(&self) -> crate::ids::FontId {
        self.font
    }
}

impl Iterator for CharCodes<'_> {
    type Item = u8;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let word = *self.words.get(self.next)?;
        let payload = word.payload();
        if word.tag() != 0
            || (payload >> 21) as u32 != self.font.raw()
            || payload & 0x1f_ffff > u8::MAX as u64
        {
            return None;
        }
        self.next += 1;
        Some(payload as u8)
    }
}

/// Opaque zero-allocation view of a contiguous same-font byte-character run.
#[derive(Clone, Copy, Debug)]
pub struct CharRun<'a> {
    words: &'a [NodeWord],
    font: crate::ids::FontId,
}

impl<'a> CharRun<'a> {
    #[must_use]
    pub const fn font(self) -> crate::ids::FontId {
        self.font
    }
    #[must_use]
    pub const fn len(self) -> usize {
        self.words.len()
    }
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.words.is_empty()
    }
    pub fn codes(self) -> impl ExactSizeIterator<Item = u8> + 'a {
        self.words.iter().map(|word| word.payload() as u8)
    }
}

impl<'a> IntoIterator for NodeList<'a> {
    type Item = NodeRef<'a>;
    type IntoIter = NodeIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct NodeIter<'a> {
    storage: &'a NodeStorage,
    next: usize,
    end: usize,
}
impl<'a> Iterator for NodeIter<'a> {
    type Item = NodeRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            let node = self.storage.decode(self.next);
            self.next += 1;
            Some(node)
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.next;
        (n, Some(n))
    }
}
impl<'a> DoubleEndedIterator for NodeIter<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            None
        } else {
            self.end -= 1;
            Some(self.storage.decode(self.end))
        }
    }
}
impl ExactSizeIterator for NodeIter<'_> {}

impl NodeStorage {
    fn decode(&self, index: usize) -> NodeRef<'_> {
        let word = self.words[index];
        let payload = word.payload();
        let side = payload as usize;
        match word.tag() {
            0 => NodeRef::Char {
                font: crate::ids::FontId::new((payload >> 21) as u32),
                ch: char::from_u32((payload & 0x1f_ffff) as u32).expect("invalid stored scalar"),
            },
            1 => NodeRef::Lig {
                font: crate::ids::FontId::new((payload >> 24) as u32),
                ch: char::from_u32((payload & 0xff) as u32).expect("stored TFM byte is scalar"),
                orig: (
                    char::from_u32(((payload >> 8) & 0xff) as u32)
                        .expect("stored TFM byte is scalar"),
                    char::from_u32(((payload >> 16) & 0xff) as u32)
                        .expect("stored TFM byte is scalar"),
                ),
            },
            2 => NodeRef::Kern {
                amount: Scaled::from_raw(payload as u32 as i32),
                kind: decode_kern(((payload >> 32) & 3) as u8),
            },
            3 => NodeRef::Glue {
                spec: GlueId::new(payload as u32),
                kind: decode_glue(((payload >> 32) & 0x3f) as u8),
                leader: None,
            },
            4 => NodeRef::Penalty(payload as u32 as i32),
            5 => NodeRef::MathOn(Scaled::from_raw(payload as u32 as i32)),
            6 => NodeRef::MathOff(Scaled::from_raw(payload as u32 as i32)),
            7 => NodeRef::MathStyle(decode_style(payload as u8)),
            8 => NodeRef::Nonscript,
            9 | 10 => {
                let b = BoxNode::new(BoxNodeFields {
                    width: self.boxes.width[side],
                    height: self.boxes.height[side],
                    depth: self.boxes.depth[side],
                    shift: self.boxes.shift[side],
                    display: self.boxes.display[side],
                    glue_set: self.boxes.glue_set[side],
                    glue_sign: self.boxes.glue_sign[side],
                    glue_order: self.boxes.glue_order[side],
                    children: self.boxes.children[side],
                });
                if word.tag() == 9 {
                    NodeRef::HList(b)
                } else {
                    NodeRef::VList(b)
                }
            }
            11 => NodeRef::Unset(UnsetNode::new(UnsetNodeFields {
                kind: self.unsets.kind[side],
                width: self.unsets.width[side],
                height: self.unsets.height[side],
                depth: self.unsets.depth[side],
                span_count: self.unsets.span_count[side],
                stretch: self.unsets.stretch[side],
                stretch_order: self.unsets.stretch_order[side],
                shrink: self.unsets.shrink[side],
                shrink_order: self.unsets.shrink_order[side],
                children: self.unsets.children[side],
            })),
            12 => {
                let (width, height, depth) = self.rules[side];
                NodeRef::Rule {
                    width,
                    height,
                    depth,
                }
            }
            13 => {
                let (spec, kind, leader) = &self.leaders[side];
                NodeRef::Glue {
                    spec: *spec,
                    kind: *kind,
                    leader: Some(leader),
                }
            }
            14 => {
                let (kind, pre, post, replace) = self.discs[side];
                NodeRef::Disc {
                    kind,
                    pre,
                    post,
                    replace,
                }
            }
            15 => {
                let (class, tokens) = self.marks[side];
                NodeRef::Mark { class, tokens }
            }
            16 => NodeRef::Ins {
                class: self.insertions.class[side],
                size: self.insertions.size[side],
                split_top_skip: self.insertions.split_top_skip[side],
                split_max_depth: self.insertions.split_max_depth[side],
                floating_penalty: self.insertions.floating_penalty[side],
                content: self.insertions.content[side],
            },
            17 => NodeRef::Whatsit(&self.whatsits[side]),
            18 => NodeRef::MathNoad(crate::math::MathNoad {
                kind: self.noads.kind[side].clone(),
                nucleus: self.noads.nucleus[side].clone(),
                subscript: self.noads.subscript[side].clone(),
                superscript: self.noads.superscript[side].clone(),
            }),
            19 => NodeRef::FractionNoad(&self.fractions[side]),
            20 => NodeRef::MathChoice(&self.choices[side]),
            21 => NodeRef::MathList(self.math_lists[side]),
            22 => NodeRef::Adjust(self.adjusts[side]),
            _ => panic!("reserved node-word tag"),
        }
    }
}

/// Owned scratch buffer used by the aggregate freeze API.
#[derive(Clone, Debug)]
pub struct NodeListBuilder {
    buf: Vec<Node>,
}
impl NodeListBuilder {
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }
    pub fn push(&mut self, node: Node) {
        self.buf.push(node)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[Node] {
        &self.buf
    }
    pub fn clear(&mut self) {
        self.buf.clear()
    }
    pub(crate) fn finish(&mut self, arena: &mut NodeArena) -> NodeListId {
        let id = arena.append(&self.buf);
        self.buf.clear();
        id
    }
}

#[derive(Clone, Debug, Default)]
pub struct NodeArena {
    storage: NodeStorage,
}
impl NodeArena {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn builder() -> NodeListBuilder {
        NodeListBuilder::new()
    }
    pub(crate) fn get<'a>(&'a self, id: NodeListId, survivors: &'a SurvivorArena) -> NodeList<'a> {
        match id.arena() {
            ArenaRef::Epoch => self.storage.view(id.start(), id.len()),
            ArenaRef::Survivor(_) => survivors.get(id),
        }
    }
    pub(crate) fn get_epoch(&self, id: NodeListId) -> NodeList<'_> {
        assert!(matches!(id.arena(), ArenaRef::Epoch));
        self.storage.view(id.start(), id.len())
    }
    pub(crate) fn contains(&self, id: NodeListId) -> bool {
        matches!(id.arena(), ArenaRef::Epoch)
            && (id.start() as usize)
                .checked_add(id.len() as usize)
                .is_some_and(|e| e <= self.storage.len())
    }
    pub(crate) fn watermark(&self) -> NodeArenaMark {
        NodeArenaMark(self.storage.mark())
    }
    pub(crate) fn truncate_to(&mut self, mark: NodeArenaMark) {
        self.storage.truncate(mark.0)
    }
    #[cfg(feature = "node-stats")]
    pub(crate) fn memory_columns(&self) -> Vec<NodeMemoryColumn> {
        self.storage.memory_columns("epoch")
    }
    #[cfg(any(test, feature = "testing", feature = "shadow"))]
    pub(crate) fn testing_node_count(&self) -> usize {
        self.storage.len()
    }
    pub(crate) fn append(&mut self, nodes: &[Node]) -> NodeListId {
        let start = checked_len(self.storage.len(), "node arena exceeds u32 entries");
        self.debug_assert_bottom_up(nodes, start);
        #[cfg(feature = "node-stats")]
        for n in nodes {
            crate::node::record_node_append(n);
        }
        let (start, len) = self.storage.append(nodes);
        NodeListId::new_epoch(start, len)
    }
    #[cfg(debug_assertions)]
    fn debug_assert_bottom_up(&self, nodes: &[Node], new_start: u32) {
        let mut children = Vec::new();
        for node in nodes {
            node.child_lists(&mut children)
        }
        for child in children {
            if let ArenaRef::Epoch = child.arena() {
                let end = child
                    .start()
                    .checked_add(child.len())
                    .expect("child span overflow");
                debug_assert!(
                    end <= new_start,
                    "child node-list span must be frozen below the parent span"
                );
                debug_assert!(
                    end as usize <= self.storage.len(),
                    "child node-list id is not live"
                );
            }
        }
    }
    #[cfg(not(debug_assertions))]
    fn debug_assert_bottom_up(&self, _: &[Node], _: u32) {}
}

fn checked_len(value: usize, message: &str) -> u32 {
    u32::try_from(value).unwrap_or_else(|_| panic!("{message}"))
}
fn preflight_capacity(have: u32, add: u32, message: &str) -> u32 {
    have.checked_add(add).unwrap_or_else(|| panic!("{message}"))
}

#[cfg(test)]
mod tests;

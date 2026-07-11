use super::storage::NodeStorage;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

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

impl NodeStorage {
    #[cfg(feature = "node-stats")]
    pub(super) fn capacity_signature(&self) -> [usize; 39] {
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
    pub(super) fn retained_payload_bytes(&self) -> usize {
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
    pub(super) fn record_peak(&self) {
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
}

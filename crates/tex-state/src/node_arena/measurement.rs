use super::storage::NodeStorage;
#[cfg(feature = "profiling-stats")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "profiling-stats")]
use std::sync::{Mutex, OnceLock};

/// One allocator-backed compact-node column in a diagnostic memory report.
///
/// This is process-local measurement data. It is computed on demand and is
/// never stored in `Universe`, snapshots, hashes, or replay state.
#[cfg(feature = "profiling-stats")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeMemoryColumn {
    pub name: String,
    pub len: usize,
    pub capacity: usize,
    pub element_bytes: usize,
    pub logical_bytes: usize,
    pub retained_payload_bytes: usize,
}

#[cfg(feature = "profiling-stats")]
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

/// One complete canonical-node-storage observation.
///
/// The totals are sums of this record's columns, so consumers never receive
/// totals and column details from different storages.
#[cfg(feature = "profiling-stats")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeStorageObservation {
    pub logical_bytes: u64,
    pub retained_payload_bytes: u64,
    pub columns: Vec<NodeMemoryColumn>,
}

#[cfg(feature = "profiling-stats")]
impl NodeStorageObservation {
    fn from_columns(columns: Vec<NodeMemoryColumn>) -> Self {
        Self {
            logical_bytes: columns
                .iter()
                .map(|column| column.logical_bytes as u64)
                .sum(),
            retained_payload_bytes: columns
                .iter()
                .map(|column| column.retained_payload_bytes as u64)
                .sum(),
            columns,
        }
    }

    const fn order_key(&self) -> (u64, u64) {
        (self.logical_bytes, self.retained_payload_bytes)
    }
}

#[cfg(feature = "profiling-stats")]
#[derive(Debug, Default)]
struct PeakNodeStorageRecorder {
    logical_hint: AtomicU64,
    observation: Mutex<Option<NodeStorageObservation>>,
}

#[cfg(feature = "profiling-stats")]
impl PeakNodeStorageRecorder {
    fn observe(
        &self,
        (logical_bytes, retained_payload_bytes): (u64, u64),
        columns: impl FnOnce() -> Vec<NodeMemoryColumn>,
    ) {
        if logical_bytes < self.logical_hint.load(Ordering::Relaxed) {
            return;
        }

        let observation = NodeStorageObservation::from_columns(columns());
        debug_assert_eq!(
            observation.order_key(),
            (logical_bytes, retained_payload_bytes)
        );
        let mut peak = self
            .observation
            .lock()
            .expect("node measurement mutex poisoned");
        if peak
            .as_ref()
            .is_none_or(|current| observation.order_key() > current.order_key())
        {
            self.logical_hint
                .store(observation.logical_bytes, Ordering::Relaxed);
            *peak = Some(observation);
        }
    }

    fn snapshot(&self) -> Option<NodeStorageObservation> {
        self.observation
            .lock()
            .expect("node measurement mutex poisoned")
            .clone()
    }
}

#[cfg(feature = "profiling-stats")]
static PEAK_STORAGE: OnceLock<PeakNodeStorageRecorder> = OnceLock::new();

#[cfg(feature = "profiling-stats")]
pub(super) fn record_peak_observation(
    totals: (u64, u64),
    columns: impl FnOnce() -> Vec<NodeMemoryColumn>,
) {
    PEAK_STORAGE
        .get_or_init(PeakNodeStorageRecorder::default)
        .observe(totals, columns);
}

/// Largest individual canonical storage observed during this process.
/// Survivor scratch is reported separately; aggregate end-state storage is
/// available through `Universe::node_memory_columns`.
#[cfg(feature = "profiling-stats")]
#[must_use]
pub fn peak_node_storage_measurement() -> Option<NodeStorageObservation> {
    PEAK_STORAGE
        .get()
        .and_then(PeakNodeStorageRecorder::snapshot)
}

#[derive(Clone, Copy, Default)]
struct RetainedBytes {
    logical: usize,
    retained: usize,
}

impl RetainedBytes {
    fn string(value: &String) -> Self {
        Self {
            logical: value.len(),
            retained: value.capacity(),
        }
    }

    fn bytes(value: &Vec<u8>) -> Self {
        Self {
            logical: value.len(),
            retained: value.capacity(),
        }
    }

    fn boxed<T>(_: &T) -> Self {
        let bytes = core::mem::size_of::<T>();
        Self {
            logical: bytes,
            retained: bytes,
        }
    }

    #[cfg(feature = "profiling-stats")]
    fn add_assign(&mut self, other: Self) {
        self.logical += other.logical;
        self.retained += other.retained;
    }
}

#[derive(Default)]
struct WhatsitOwnedPayloads {
    strings: RetainedBytes,
    bytes: RetainedBytes,
    boxes: RetainedBytes,
}

/// Reports heap allocations owned directly by one whatsit sidecar value.
///
/// Referenced token lists, glue, fonts, and child lists remain shared storage
/// and are deliberately accounted for by their owning stores instead.
fn whatsit_owned_payloads(whatsit: &crate::node::Whatsit) -> WhatsitOwnedPayloads {
    use crate::node::Whatsit;

    match whatsit {
        Whatsit::OpenOut { path, .. } => WhatsitOwnedPayloads {
            strings: RetainedBytes::string(path),
            ..WhatsitOwnedPayloads::default()
        },
        Whatsit::Special { class, payload } => WhatsitOwnedPayloads {
            strings: RetainedBytes::string(class),
            bytes: RetainedBytes::bytes(payload),
            ..WhatsitOwnedPayloads::default()
        },
        Whatsit::PdfLiteral { payload, .. } | Whatsit::PdfSetMatrix { payload } => {
            WhatsitOwnedPayloads {
                bytes: RetainedBytes::bytes(payload),
                ..WhatsitOwnedPayloads::default()
            }
        }
        Whatsit::PdfColorStack { action, .. } => {
            let bytes = match action {
                crate::PdfColorStackAction::Set(payload)
                | crate::PdfColorStackAction::Push(payload) => RetainedBytes::bytes(payload),
                crate::PdfColorStackAction::Pop | crate::PdfColorStackAction::Current => {
                    RetainedBytes::default()
                }
            };
            WhatsitOwnedPayloads {
                bytes,
                ..WhatsitOwnedPayloads::default()
            }
        }
        Whatsit::PdfDestination(destination) => WhatsitOwnedPayloads {
            boxes: RetainedBytes::boxed(destination.as_ref()),
            ..WhatsitOwnedPayloads::default()
        },
        Whatsit::PdfThread(thread) => WhatsitOwnedPayloads {
            boxes: RetainedBytes::boxed(thread.as_ref()),
            ..WhatsitOwnedPayloads::default()
        },
        Whatsit::CloseOut { .. }
        | Whatsit::DeferredWrite { .. }
        | Whatsit::PdfReferenceObject { .. }
        | Whatsit::PdfAccessibility(_)
        | Whatsit::PdfAnnotation { .. }
        | Whatsit::PdfLinkStart { .. }
        | Whatsit::PdfLinkEnd { .. }
        | Whatsit::PdfRunningLink(_)
        | Whatsit::DeferredPdfLiteral { .. }
        | Whatsit::PdfSave
        | Whatsit::PdfRestore
        | Whatsit::PdfSavePos
        | Whatsit::PdfSnapRefPoint
        | Whatsit::PdfSnapY { .. }
        | Whatsit::PdfSnapYComp { .. }
        | Whatsit::PdfRefXForm { .. }
        | Whatsit::PdfRefXImage { .. }
        | Whatsit::PdfEndThread
        | Whatsit::Language { .. } => WhatsitOwnedPayloads::default(),
    }
}

impl NodeStorage {
    #[cfg(feature = "profiling-stats")]
    pub(super) fn capacity_signature(&self) -> [usize; 33] {
        [
            self.words.capacity(),
            self.origins.capacity(),
            self.ligatures.capacity(),
            self.boxes.rows.capacity(),
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

    pub(crate) fn retained_payload_bytes(&self) -> usize {
        usize::try_from(self.payload_bytes().1).expect("node storage retained bytes exceed usize")
    }

    pub(super) fn payload_bytes(&self) -> (u64, u64) {
        fn bytes<T>(values: &Vec<T>) -> (u64, u64) {
            (
                (values.len() * core::mem::size_of::<T>()) as u64,
                (values.capacity() * core::mem::size_of::<T>()) as u64,
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
        add!(self.origins);
        add!(self.ligatures);
        add!(self.boxes.rows);
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
        for (_, _, source, origins) in &self.ligatures {
            logical += (source.len() * core::mem::size_of::<char>()) as u64;
            retained += (source.capacity() * core::mem::size_of::<char>()) as u64;
            logical += (origins.len() * core::mem::size_of::<crate::token::OriginId>()) as u64;
            retained +=
                (origins.capacity() * core::mem::size_of::<crate::token::OriginId>()) as u64;
        }
        for whatsit in &self.whatsits {
            let owned = whatsit_owned_payloads(whatsit);
            for allocation in [owned.strings, owned.bytes, owned.boxes] {
                logical += allocation.logical as u64;
                retained += allocation.retained as u64;
            }
        }
        (logical, retained)
    }

    #[cfg(feature = "profiling-stats")]
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
        column!("origins", &self.origins);
        column!("ligatures", &self.ligatures);
        column!("boxes.rows", &self.boxes.rows);
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
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.ligatures.owned_sources"),
            self.ligatures
                .iter()
                .map(|(_, _, source, _)| source.len() * core::mem::size_of::<char>())
                .sum(),
            self.ligatures
                .iter()
                .map(|(_, _, source, _)| source.capacity() * core::mem::size_of::<char>())
                .sum(),
        ));

        let mut strings = RetainedBytes::default();
        let mut payloads = RetainedBytes::default();
        let mut boxes = RetainedBytes::default();
        for whatsit in &self.whatsits {
            let owned = whatsit_owned_payloads(whatsit);
            strings.add_assign(owned.strings);
            payloads.add_assign(owned.bytes);
            boxes.add_assign(owned.boxes);
        }
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.whatsits.owned_strings"),
            strings.logical,
            strings.retained,
        ));
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.whatsits.owned_payloads"),
            payloads.logical,
            payloads.retained,
        ));
        out.push(NodeMemoryColumn::byte_payload(
            format!("{prefix}.whatsits.owned_boxes"),
            boxes.logical,
            boxes.retained,
        ));
        out
    }

    #[cfg(feature = "profiling-stats")]
    pub(super) fn record_peak(&self) {
        record_peak_observation(self.payload_bytes(), || self.memory_columns("peak"));
    }
}

#[cfg(all(test, feature = "profiling-stats"))]
mod tests;

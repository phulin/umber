use std::mem::size_of;
use std::sync::Arc;

use tex_exec::{Mode, ModeNest, ModeNestSummary};
use tex_state::hyphenation::PatternSpec;
use tex_state::input::{InputFrameSummary, InputSummary, LexerState, SourceFrameSummary};
use tex_state::node::{KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::source_map::SourceDescriptor;
use tex_state::token::Catcode;
use tex_state::world::{PrintSink, StreamSlot};
use tex_state::{Snapshot, SourceId, Universe};

pub const RETAINED_BYTES_PER_CAPTURE_BUDGET: u64 = 32 * 1024;
pub const DETACHED_CODE_TABLE_WRITE_BUDGET: u64 = 8 * 1024;
pub const DEEP_GROUP_GLOBAL_WRITE_BUDGET: u64 = 8 * 1024;
pub const DEEP_GROUP_SMALL_DEPTH: usize = 8;
pub const DEEP_GROUP_LARGE_DEPTH: usize = 4_096;
pub const LATENCY_SCALE_BUDGET: u128 = 4;
pub const LATENCY_NOISE_ALLOWANCE_NS: u128 = 25_000;
pub const RETAINED_CAPTURES: usize = 32;

pub const WORKLOADS: [WorkloadKind; 7] = [
    WorkloadKind::Input,
    WorkloadKind::Page,
    WorkloadKind::Mode,
    WorkloadKind::Stream,
    WorkloadKind::Hyphenation,
    WorkloadKind::Provenance,
    WorkloadKind::UnicodeCodeTables,
];

#[derive(Clone, Copy, Debug)]
pub enum WorkloadKind {
    Input,
    Page,
    Mode,
    Stream,
    Hyphenation,
    Provenance,
    UnicodeCodeTables,
}

impl WorkloadKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Page => "page",
            Self::Mode => "mode",
            Self::Stream => "stream",
            Self::Hyphenation => "hyphenation",
            Self::Provenance => "provenance",
            Self::UnicodeCodeTables => "unicode_code_tables",
        }
    }

    #[must_use]
    pub const fn small_units(self) -> usize {
        match self {
            Self::Input | Self::Stream => 1_024,
            Self::Page | Self::Mode => 32,
            Self::Hyphenation | Self::Provenance | Self::UnicodeCodeTables => 16,
        }
    }

    #[must_use]
    pub const fn large_units(self) -> usize {
        match self {
            Self::Input | Self::Stream => 256 * 1024,
            Self::Page | Self::Mode => 4_096,
            Self::Hyphenation | Self::Provenance | Self::UnicodeCodeTables => 2_048,
        }
    }
}

// Keeping the measured values inline avoids adding benchmark-harness heap
// allocations to snapshot retention observations.
#[allow(clippy::large_enum_variant)]
pub enum Workload {
    Universe {
        universe: Universe,
        logical_live_bytes: u64,
    },
    Mode {
        nest: ModeNest,
        logical_live_bytes: u64,
    },
}

#[allow(clippy::large_enum_variant)]
pub enum CapturedState {
    Universe(Snapshot),
    Mode(ModeNestSummary),
}

impl Workload {
    #[must_use]
    pub const fn logical_live_bytes(&self) -> u64 {
        match self {
            Self::Universe {
                logical_live_bytes, ..
            }
            | Self::Mode {
                logical_live_bytes, ..
            } => *logical_live_bytes,
        }
    }

    pub fn warm_capture(&mut self) {
        drop(self.capture());
    }

    #[must_use]
    pub fn capture(&mut self) -> CapturedState {
        match self {
            Self::Universe { universe, .. } => CapturedState::Universe(universe.snapshot()),
            Self::Mode { nest, .. } => CapturedState::Mode(nest.summary()),
        }
    }

    pub fn set_unicode_catcode(&mut self, ch: char, value: Catcode) {
        let Self::Universe { universe, .. } = self else {
            panic!("Unicode code-table mutation requires a Universe workload");
        };
        universe.set_catcode(ch, value);
    }
}

#[must_use]
pub fn build_workload(kind: WorkloadKind, units: usize) -> Workload {
    match kind {
        WorkloadKind::Input => input_workload(units),
        WorkloadKind::Page => page_workload(units),
        WorkloadKind::Mode => mode_workload(units),
        WorkloadKind::Stream => stream_workload(units),
        WorkloadKind::Hyphenation => hyphenation_workload(units),
        WorkloadKind::Provenance => provenance_workload(units),
        WorkloadKind::UnicodeCodeTables => unicode_code_table_workload(units),
    }
}

#[must_use]
pub fn deep_group_code_table_workload(depth: usize) -> Universe {
    let mut universe = Universe::new();
    for _ in 0..depth {
        universe.enter_group();
    }
    universe
}

fn input_workload(bytes: usize) -> Workload {
    let mut universe = Universe::new();
    let line = "a".repeat(bytes);
    let registration = universe
        .register_input_source(
            SourceId::new(0),
            SourceDescriptor::generated(Arc::from(line.as_bytes())),
        )
        .expect("register benchmark input source");
    let frame = SourceFrameSummary::new(
        0,
        bytes,
        1,
        1,
        LexerState::MidLine,
        line,
        0,
        Vec::new(),
        false,
    )
    .with_registration(Some(registration));
    universe.set_input_summary(InputSummary::new(
        vec![InputFrameSummary::Source {
            source_id: SourceId::new(0),
            input_record: None,
            source: frame,
        }],
        None,
        None,
    ));
    Workload::Universe {
        universe,
        logical_live_bytes: bytes as u64,
    }
}

fn page_workload(nodes: usize) -> Workload {
    let mut universe = Universe::new();
    for index in 0..nodes {
        let node = Node::Kern {
            amount: Scaled::from_raw(index as i32),
            kind: KernKind::Explicit,
        };
        if index % 2 == 0 {
            universe.append_page_contribution(node);
        } else {
            universe.push_current_page_node(node);
        }
    }
    Workload::Universe {
        universe,
        logical_live_bytes: (nodes * size_of::<Node>()) as u64,
    }
}

fn mode_workload(nodes: usize) -> Workload {
    let mut nest = ModeNest::new();
    nest.push(Mode::Horizontal);
    for index in 0..nodes {
        nest.current_list_mut().push(Node::Kern {
            amount: Scaled::from_raw(index as i32),
            kind: KernKind::Explicit,
        });
    }
    Workload::Mode {
        nest,
        logical_live_bytes: (nodes * size_of::<Node>()) as u64,
    }
}

fn stream_workload(bytes: usize) -> Workload {
    let mut universe = Universe::new();
    universe
        .world_mut()
        .write_text(PrintSink::Stream(StreamSlot::new(0)), &"s".repeat(bytes));
    Workload::Universe {
        universe,
        logical_live_bytes: bytes as u64,
    }
}

fn hyphenation_workload(patterns: usize) -> Workload {
    let mut universe = Universe::new();
    let mut logical_live_bytes = 0_u64;
    for index in 0..patterns {
        let letters = unique_letters(index);
        let values = vec![0, 1, 0, 1, 0];
        logical_live_bytes += (letters.len() * size_of::<char>() + values.len()) as u64;
        universe.add_hyphenation_pattern(PatternSpec { letters, values });
    }
    Workload::Universe {
        universe,
        logical_live_bytes,
    }
}

fn provenance_workload(records: usize) -> Workload {
    let mut universe = Universe::new();
    let baseline = universe.provenance_stats();
    for index in 0..records {
        universe.source_origin(SourceId::new(7), index as u64, 1, index as u32 + 1);
    }
    let logical_live_bytes = universe
        .provenance_stats()
        .saturating_sub(baseline)
        .estimated_bytes() as u64;
    Workload::Universe {
        universe,
        logical_live_bytes,
    }
}

fn unicode_code_table_workload(entries: usize) -> Workload {
    let mut universe = Universe::new();
    for index in 0..entries {
        let scalar = ((index * 257) % 0x10_ff00) as u32;
        if let Some(ch) = char::from_u32(scalar) {
            universe.set_catcode(ch, Catcode::Letter);
        }
    }
    Workload::Universe {
        universe,
        logical_live_bytes: (entries * size_of::<u32>()) as u64,
    }
}

fn unique_letters(mut value: usize) -> Vec<char> {
    let mut letters = Vec::with_capacity(5);
    letters.push('.');
    for _ in 0..4 {
        letters.push(char::from(b'a' + (value % 26) as u8));
        value /= 26;
    }
    letters
}

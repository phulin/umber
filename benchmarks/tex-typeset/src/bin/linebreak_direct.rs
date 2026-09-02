use std::hint::black_box;
use std::time::Instant;

use tex_fonts::CharMetrics;
use tex_state::font::PdfFontCode;
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::node::{GlueKind, Node};
use tex_state::node_arena::{NodeCursor, PageListId};
use tex_state::node_region::NodePool;
use tex_state::page_node_arena::{PageMaterialArena, PageMaterialRegion, PageMaterialView};
use tex_state::scaled::Scaled;
use tex_typeset::TypesetState;
use tex_typeset::linebreak::{LineBreakParams, LineShape, ParagraphTape};

const PARAGRAPH_NODES: usize = 4_096;
const ITERATIONS: usize = 1_000;

struct DirectState {
    pool: NodePool,
    region: PageMaterialRegion,
    widths: [Scaled; 256],
    characters: [Option<CharMetrics>; 256],
}

impl DirectState {
    fn new() -> Self {
        let mut pool = NodePool::new();
        let region = PageMaterialRegion::new(&mut pool);
        Self {
            pool,
            region,
            widths: [Scaled::from_raw(0); 256],
            characters: [None; 256],
        }
    }

    fn publish(&mut self, nodes: Vec<Node>) -> PageListId {
        PageMaterialArena::new(&mut self.pool, &mut self.region)
            .publish_owned(nodes)
            .expect("focused paragraph is valid page material")
    }
}

impl TypesetState for DirectState {
    fn page_nodes(&self, list: PageListId) -> NodeCursor<'_> {
        PageMaterialView::new(&self.pool, &self.region)
            .node_cursor(list)
            .expect("focused paragraph remains live")
    }

    fn font_char_metrics(&self, _font: FontId, _code: u8) -> Option<CharMetrics> {
        None
    }

    fn font_widths(&self, _font: FontId) -> &[Scaled; 256] {
        &self.widths
    }

    fn font_characters(&self, _font: FontId) -> &[Option<CharMetrics>] {
        &self.characters
    }

    fn pdf_font_code(&self, _table: PdfFontCode, _font: FontId, _code: u8) -> i32 {
        0
    }
}

fn main() {
    let mut state = DirectState::new();
    let nodes = (0..PARAGRAPH_NODES)
        .map(|index| match index % 3 {
            0 => Node::Rule {
                width: Some(Scaled::from_raw(20)),
                height: Some(Scaled::from_raw(10)),
                depth: Some(Scaled::from_raw(0)),
            },
            1 => Node::Glue {
                spec: GlueSpec {
                    width: Scaled::from_raw(10),
                    stretch: Scaled::from_raw(5),
                    ..GlueSpec::ZERO
                },
                kind: GlueKind::Normal,
                leader: None,
            },
            _ => Node::Penalty(0),
        })
        .collect();
    let paragraph = state.publish(nodes);
    let params = LineBreakParams {
        pretolerance: 10_000,
        tolerance: 10_000,
        line_penalty: 10,
        hyphen_penalty: 50,
        ex_hyphen_penalty: 50,
        adj_demerits: 10_000,
        double_hyphen_demerits: 10_000,
        final_hyphen_demerits: 5_000,
        emergency_stretch: Scaled::from_raw(0),
        looseness: 0,
        last_line_fit: 0,
        pdf_adjust_spacing: 0,
        pdf_protrude_chars: 0,
        expansion_steps: None,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(Scaled::from_raw(1_000)),
    };

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        let tape = ParagraphTape::analyze_arena(&state, state.page_nodes(paragraph), &params);
        black_box(tape);
    }
    println!(
        "LINEBREAK_DIRECT nodes={} iterations={} visits={} elapsed_ns={}",
        PARAGRAPH_NODES,
        ITERATIONS,
        PARAGRAPH_NODES * ITERATIONS,
        started.elapsed().as_nanos()
    );
}

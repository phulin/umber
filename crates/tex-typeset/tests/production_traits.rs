use tex_fonts::metrics::ExtensibleRecipe;
use tex_fonts::{CharMetrics, LigKernChar, LigKernCommand};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::font::{NULL_FONT, PdfFontCode};
use tex_state::glue::GlueSpec;
use tex_state::ids::FontId;
use tex_state::math::{MathFontSize, MathStyle};
use tex_state::node::Node;
use tex_state::node_arena::{NodeCursor, PageListId, PageNodeArena};
use tex_state::scaled::Scaled;
use tex_typeset::linebreak::{LineBreakParams, LineShape, try_line_break_without_hyphenation};
use tex_typeset::math::{MathParamState, MathParams, MathTypesetState, Style, mlist_to_hlist};
use tex_typeset::{HpackParams, PackSpec, TypesetState, hpack};

struct KernelState {
    pages: PageNodeArena,
    widths: [Scaled; 256],
    characters: [Option<CharMetrics>; 256],
}

impl KernelState {
    fn new() -> Self {
        Self {
            pages: PageNodeArena::new(),
            widths: [Scaled::from_raw(0); 256],
            characters: [None; 256],
        }
    }

    fn publish(&mut self, nodes: Vec<Node>) -> PageListId {
        self.pages.publish(nodes).expect("valid page list")
    }
}

impl TypesetState for KernelState {
    fn page_nodes(&self, list: PageListId) -> NodeCursor<'_> {
        NodeCursor::compact(self.pages.get(list).expect("live page coordinate").nodes())
    }

    fn font_char_metrics(&self, _font: FontId, code: u8) -> Option<CharMetrics> {
        self.characters[usize::from(code)]
    }

    fn font_widths(&self, _font: FontId) -> &[Scaled; 256] {
        &self.widths
    }

    fn font_characters(&self, _font: FontId) -> &[Option<CharMetrics>] {
        &self.characters
    }

    fn pdf_font_code(&self, table: PdfFontCode, _font: FontId, _code: u8) -> i32 {
        if table == PdfFontCode::Ef { 1000 } else { 0 }
    }
}

impl MathTypesetState for KernelState {
    fn math_family_font(&self, _size: MathFontSize, _family: u8) -> FontId {
        NULL_FONT
    }

    fn font_parameter(&self, _font: FontId, _number: u16) -> Scaled {
        Scaled::from_raw(0)
    }

    fn font_next_larger(&self, _font: FontId, _code: u8) -> Option<u8> {
        None
    }

    fn font_extensible_recipe(&self, _font: FontId, _code: u8) -> Option<ExtensibleRecipe> {
        None
    }

    fn lig_kern_command(
        &self,
        _font: FontId,
        _left: LigKernChar,
        _right: LigKernChar,
    ) -> Option<LigKernCommand> {
        None
    }

    fn font_skew_char(&self, _font: FontId) -> i32 {
        0
    }
}

impl MathParamState for KernelState {
    fn int_param(&self, param: IntParam) -> i32 {
        if param == IntParam::DELIMITER_FACTOR {
            901
        } else {
            0
        }
    }

    fn dimen_param(&self, _param: DimenParam) -> Scaled {
        Scaled::from_raw(0)
    }

    fn glue_param(&self, _param: GlueParam) -> GlueSpec {
        GlueSpec::ZERO
    }
}

fn linebreak_params(width: Scaled) -> LineBreakParams {
    LineBreakParams {
        pretolerance: 100,
        tolerance: 1_000,
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
        expansion_steps: None,
        pdf_protrude_chars: 0,
        left_skip: GlueSpec::ZERO,
        right_skip: GlueSpec::ZERO,
        par_fill_skip: GlueSpec::ZERO,
        shape: LineShape::natural(width),
    }
}

#[test]
fn packing_resolves_a_typed_page_coordinate() {
    let mut state = KernelState::new();
    let list = state.publish(vec![Node::Rule {
        width: Some(Scaled::from_raw(12)),
        height: Some(Scaled::from_raw(3)),
        depth: Some(Scaled::from_raw(2)),
    }]);

    let packed = hpack(
        &state,
        list,
        PackSpec::Natural,
        HpackParams {
            hbadness: 1_000,
            hfuzz: Scaled::from_raw(0),
            overfull_rule: Scaled::from_raw(0),
        },
    );

    assert_eq!(packed.node.width, Scaled::from_raw(12));
    assert_eq!(packed.node.height, Scaled::from_raw(3));
    assert_eq!(packed.node.depth, Scaled::from_raw(2));
}

#[test]
fn linebreaking_uses_the_pure_adapter_without_a_runtime_owner() {
    let state = KernelState::new();
    let nodes = vec![
        Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(0)),
            depth: Some(Scaled::from_raw(0)),
        },
        Node::Penalty(-10_000),
    ];

    let plan =
        try_line_break_without_hyphenation(&state, &nodes, &linebreak_params(Scaled::from_raw(10)))
            .expect("forced one-line paragraph breaks");

    assert_eq!(plan.breaks.len(), 1);
}

#[test]
fn math_snapshot_and_empty_conversion_keep_only_plain_values() {
    let state = KernelState::new();
    let params = MathParams::read(&state);
    let layout = mlist_to_hlist(
        &state,
        PageListId::empty(),
        Style::from_math_style(MathStyle::Text),
        false,
        &params,
    );

    assert!(layout.root().is_empty());
    assert!(!layout.recovered());
    assert_eq!(params.delimiter_factor, 901);
    assert_eq!(params.thin_mu_skip, GlueSpec::ZERO);
}

use super::*;
use tex_lex::{InputStack, MemoryInput};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token};
use tex_state::{EffectRecord, ExpansionState, PrintSink};
use tex_state::{InteractionMode, Universe};

mod align;
mod assignments;
mod boxes;
mod core;
mod fonts;
mod grouping_parity;
mod groups;
mod hyphenation;

#[test]
fn paragraph_mutation_entry_class_distinguishes_root_from_live_groups() {
    assert!(crate::paragraph_memo::same_mutation_entry_class(false, 0));
    assert!(crate::paragraph_memo::same_mutation_entry_class(true, 1));
    assert!(crate::paragraph_memo::same_mutation_entry_class(true, 9));
    assert!(!crate::paragraph_memo::same_mutation_entry_class(false, 1));
    assert!(!crate::paragraph_memo::same_mutation_entry_class(true, 0));
}

/// pdftex.web's post-line-break dimension overrides first apply the per-line
/// values and then let the first/last special values win. The ignored sentinel
/// suppresses an individual assignment, including on a one-line paragraph.
#[test]
fn pdf_line_dimension_overrides_obey_ignored_and_edge_precedence() {
    use tex_state::node::{BoxNode, BoxNodeFields, Sign};
    use tex_state::scaled::Scaled;

    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let hbox = |width, height, depth| {
        BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(width),
            height: Scaled::from_raw(height),
            depth: Scaled::from_raw(depth),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: tex_state::scaled::GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: tex_state::glue::Order::Normal,
            children: empty,
        })
    };

    let ignored = stores.dimen_param(DimenParam::PDF_IGNORED_DIMEN);
    stores.set_dimen_param_global(DimenParam::PDF_EACH_LINE_HEIGHT, Scaled::from_raw(20));
    stores.set_dimen_param_global(DimenParam::PDF_EACH_LINE_DEPTH, Scaled::from_raw(30));
    stores.set_dimen_param_global(DimenParam::PDF_FIRST_LINE_HEIGHT, Scaled::from_raw(40));
    stores.set_dimen_param_global(DimenParam::PDF_LAST_LINE_DEPTH, ignored);
    let mut lines = vec![hbox(1, 2, 3), hbox(4, 5, 6)];

    crate::assignments::test_apply_pdf_line_dimensions(&stores, &mut lines);

    assert_eq!((lines[0].height.raw(), lines[0].depth.raw()), (40, 30));
    assert_eq!((lines[1].height.raw(), lines[1].depth.raw()), (20, 30));

    stores.set_dimen_param_global(DimenParam::PDF_LAST_LINE_DEPTH, Scaled::from_raw(50));
    let mut one = vec![hbox(7, 8, 9)];
    crate::assignments::test_apply_pdf_line_dimensions(&stores, &mut one);
    assert_eq!((one[0].height.raw(), one[0].depth.raw()), (40, 50));
}
mod io;
mod math;
pub(crate) mod support;

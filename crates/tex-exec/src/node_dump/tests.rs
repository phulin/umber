//! Regression coverage for `DumpConfig` and the `\showbox`/`\showlists`
//! node-list renderer.

use super::*;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::math::{LimitType, MathChar, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{BoxNodeFields, KernKind, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

/// Builds the `\hbox{\kern1pt}` box register from bd `umber2-alfh.6`'s
/// reproduction (`tests/corpus/command-semantic/main-control/show-box/show-box.tex`):
/// `\setbox0=\hbox{\kern1pt}`.
fn hbox_with_one_point_kern(stores: &mut Universe) -> NodeListId {
    let kern = Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    };
    let children = stores.freeze_node_list(&[kern]);
    let hbox = Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(Scaled::UNITY),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        display: false,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    stores.freeze_node_list(&[hbox])
}

/// bd `umber2-alfh.6`: with `\showboxbreadth`/`\showboxdepth` left at
/// INITEX's default of 0 (tex.web §240's `eqtb` zeroing loop), `\showbox`
/// printed `etc.` for a one-item box instead of the box line, because
/// `dump_nodes` used `show_box_breadth` directly as the per-level item limit.
/// TeX82 §198's `show_box` clamps a non-positive `breadth_max` to 5 before
/// `show_node_list` (§182) ever sees it; a limit of 0 is not "show nothing",
/// it is the sentinel for "the parameter was never set."
#[test]
fn default_show_box_breadth_renders_top_level_box_instead_of_etc() {
    let mut stores = Universe::new();
    assert_eq!(stores.int_param(IntParam::SHOW_BOX_BREADTH), 0);
    assert_eq!(stores.int_param(IntParam::SHOW_BOX_DEPTH), 0);

    let list = hbox_with_one_point_kern(&mut stores);
    let config = DumpConfig::read(&stores);
    let text = dump_node_list(&stores, list, config);

    // Real pdftex 1.40.27 writes exactly this line for `\showbox0` after
    // `\setbox0=\hbox{\kern1pt}` (confirmed against the pinned oracle).
    assert_eq!(text, "\\hbox(0.0+0.0)x1.0 []\n");
}

/// A user-set non-positive `\showboxbreadth` gets the same §198 fallback to
/// 5 as the untouched default, not just the value `0` INITEX happens to
/// leave behind.
#[test]
fn negative_show_box_breadth_also_falls_back_to_five() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::SHOW_BOX_BREADTH, -3);
    let list = hbox_with_one_point_kern(&mut stores);
    let config = DumpConfig::read(&stores);
    assert_eq!(config.breadth, 5);
    let text = dump_node_list(&stores, list, config);
    assert_eq!(text, "\\hbox(0.0+0.0)x1.0 []\n");
}

/// An explicit, still-positive breadth smaller than the item count must
/// keep truncating with `etc.`, per §182's `incr(n); if n>breadth_max then
/// ... print("etc.")`: the §198 fallback only replaces a non-positive
/// value, it does not disable truncation altogether.
#[test]
fn explicit_positive_breadth_still_truncates_with_etc() {
    let mut stores = Universe::new();
    let kern = |amount| Node::Kern {
        amount: Scaled::from_raw(amount),
        kind: KernKind::Explicit,
    };
    let list = stores.freeze_node_list(&[
        kern(Scaled::UNITY),
        kern(2 * Scaled::UNITY),
        kern(3 * Scaled::UNITY),
    ]);
    let config = DumpConfig {
        breadth: 2,
        depth: 0,
    };
    let text = dump_node_list(&stores, list, config);
    assert_eq!(text, "\\kern 1.0\n\\kern 2.0\netc.\n");
}

fn math_char(family: u8, character: char) -> MathChar {
    MathChar {
        family,
        character,
        origin: Default::default(),
    }
}

/// TeX82 §§691--697 (`tex.web:13623-13751`) assign a distinct exact
/// display to every noad kind and to the five math-field forms. Empty fields
/// must remain silent; the final noad exercises math-char, math-text-char,
/// sub-box, and sub-mlist fields in their nucleus/script positions.
#[test]
fn showlists_renders_all_math_noad_variants_and_empty_fields() {
    let mut stores = Universe::new();
    let sub_box = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let sub_mlist = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(math_char(3, 'm')),
    ))]);
    let mut nodes = Vec::new();
    for class in [
        NoadClass::Ord,
        NoadClass::Op,
        NoadClass::Bin,
        NoadClass::Rel,
        NoadClass::Open,
        NoadClass::Close,
        NoadClass::Punct,
        NoadClass::Inner,
    ] {
        nodes.push(Node::MathNoad(MathNoad::new(
            NoadKind::Normal(class),
            MathField::Empty,
        )));
    }
    for limits in [
        LimitType::DisplayLimits,
        LimitType::Limits,
        LimitType::NoLimits,
    ] {
        nodes.push(Node::MathNoad(MathNoad::new(
            NoadKind::Operator(limits),
            MathField::Empty,
        )));
    }
    for kind in [
        NoadKind::Radical { delimiter: 0x12345 },
        NoadKind::Accent {
            accent: math_char(2, '^'),
        },
        NoadKind::LeftDelimiter { delimiter: 0x28300 },
        NoadKind::RightDelimiter { delimiter: 0x29301 },
        NoadKind::MiddleDelimiter { delimiter: 0x26A00 },
        NoadKind::Underline,
        NoadKind::Overline,
        NoadKind::VCenter,
    ] {
        nodes.push(Node::MathNoad(MathNoad::new(kind, MathField::Empty)));
    }
    nodes.push(Node::MathNoad(MathNoad {
        kind: NoadKind::Normal(NoadClass::Ord),
        nucleus: MathField::MathTextChar(math_char(1, 'x')),
        superscript: MathField::SubBox(sub_box),
        subscript: MathField::SubMlist(sub_mlist),
    }));
    let list = stores.freeze_node_list(&nodes);

    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 100,
                depth: 100,
            },
        ),
        concat!(
            "\\mathord\n",
            "\\mathop\n",
            "\\mathbin\n",
            "\\mathrel\n",
            "\\mathopen\n",
            "\\mathclose\n",
            "\\mathpunct\n",
            "\\mathinner\n",
            "\\mathop\n",
            "\\mathop\\limits\n",
            "\\mathop\\nolimits\n",
            "\\radical\"12345\n",
            "\\accent\\fam2 ^\n",
            "\\left\"28300\n",
            "\\right\"29301\n",
            "\\middle\"26A00\n",
            "\\underline\n",
            "\\overline\n",
            "\\vcenter\n",
            "\\mathord\n",
            ".\\fam1 x\n",
            "^\\kern 1.0\n",
            "_\\mathord\n",
            "..\\fam3 m\n",
        ),
    );
}

/// TeX82 §692 (`tex.web:13639-13668`) prints ` []` for each nonempty
/// subsidiary field hidden by `\showboxdepth`, while an empty field prints no
/// marker. This is the exact `$a_b\showlists` shape at depth zero.
#[test]
fn showlists_depth_cutoff_prints_nonempty_math_field_marker() {
    let mut stores = Universe::new();
    let mut noad = MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(math_char(1, 'a')),
    );
    noad.subscript = MathField::MathChar(math_char(1, 'b'));
    let list = stores.freeze_node_list(&[Node::MathNoad(noad)]);

    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 100,
                depth: 0,
            },
        ),
        "\\mathord [] []\n",
    );
}

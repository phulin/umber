//! Regression coverage for `DumpConfig` and the `\showbox`/`\showlists`
//! node-list renderer.

use super::*;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::math::{LimitType, MathChar, MathChoice, MathField, MathNoad, NoadClass, NoadKind};
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
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }));
    stores.freeze_node_list(&[hbox])
}

#[test]
fn box_lr_has_exact_canonical_node_dump_evidence() {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    for (box_lr, suffix) in [
        (tex_state::node::BoxLr::Normal, ""),
        (tex_state::node::BoxLr::Reversed, ", reversed"),
        (tex_state::node::BoxLr::DList, ", display"),
    ] {
        let list = stores.freeze_node_list(&[Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: empty,
        }))]);
        assert_eq!(
            dump_node_list(&stores, list, DumpConfig::read(&stores)),
            format!("\\hbox(0.0+0.0)x0.0{suffix}\n"),
        );
    }
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

#[test]
fn etex_mlr_boundaries_dump_with_exact_identity() {
    // Merged e-TeX WEB §12 keeps all six M/L/R math-node subtypes distinct.
    let mut stores = Universe::new();
    let list = stores.freeze_node_list(&[
        Node::Direction(tex_state::node::Direction::BeginM),
        Node::Direction(tex_state::node::Direction::EndM),
        Node::Direction(tex_state::node::Direction::BeginL),
        Node::Direction(tex_state::node::Direction::EndL),
        Node::Direction(tex_state::node::Direction::BeginR),
        Node::Direction(tex_state::node::Direction::EndR),
    ]);
    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 10,
                depth: 10
            }
        ),
        "\\beginM\n\\endM\n\\beginL\n\\endL\n\\beginR\n\\endR\n"
    );
}

/// pdfTeX §§190/193 retain insertion and numbered-mark identity in list
/// diagnostics, in node order, without consuming or rewriting either frozen
/// source list. This is the exact body printed for the equivalent
/// `\vbox{\insert3{\hrule height5pt}\marks12{hello}}`.
#[test]
fn pdftex_insertion_and_numbered_mark_dump_exact_identity_in_source_order() {
    let mut stores = Universe::new_with_plain_catcodes();
    let zero = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    let insertion_content = stores.freeze_node_list(&[Node::Rule {
        width: None,
        height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
        depth: Some(Scaled::from_raw(0)),
    }]);
    let mark_tokens = stores.intern_token_list(&[
        tex_state::token::Token::Char {
            ch: 'h',
            cat: tex_state::token::Catcode::Letter,
        },
        tex_state::token::Token::Char {
            ch: 'i',
            cat: tex_state::token::Catcode::Letter,
        },
    ]);
    let nodes = [
        Node::Ins {
            class: 3,
            size: Scaled::from_raw(5 * Scaled::UNITY),
            split_top_skip: zero,
            split_max_depth: Scaled::MAX_DIMEN,
            floating_penalty: 17,
            content: insertion_content,
        },
        Node::Mark {
            class: 12,
            tokens: mark_tokens,
        },
    ];
    let source = stores.freeze_node_list(&nodes);

    let expected = concat!(
        "\\insert3, natural size 5.0; split(0.0,16383.99998); float cost 17\n",
        ".\\rule(5.0+0.0)x*\n",
        "\\marks12{hi}\n",
    );
    let config = DumpConfig {
        breadth: 100,
        depth: 100,
    };
    assert_eq!(dump_node_list(&stores, source, config), expected);
    assert_eq!(
        stores.nodes(source),
        nodes.as_slice(),
        "dumping is immutable"
    );
}

/// e-TeX `etex.web` change blocks 21 and 49 extend TeX82's mark node with a
/// class, while merged block 77 prints class zero in the legacy `\mark` form
/// and every sparse class as `\marks<n>`. The 255/256 boundary must therefore
/// preserve the integer itself, not an arena slot or a generic placeholder.
#[test]
fn etex_numbered_mark_dump_renders_dense_and_sparse_boundary_classes_exactly() {
    let mut stores = Universe::new_with_plain_catcodes();
    let tokens = stores.intern_token_list(&[tex_state::token::Token::Char {
        ch: 'x',
        cat: tex_state::token::Catcode::Letter,
    }]);
    let list = stores.freeze_node_list(&[
        Node::Mark { class: 0, tokens },
        Node::Mark { class: 255, tokens },
        Node::Mark { class: 256, tokens },
    ]);

    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 100,
                depth: 100,
            },
        ),
        "\\mark{x}\n\\marks255{x}\n\\marks256{x}\n"
    );
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

/// TeX82 §681 (`tex.web:13366-13426`) gives an empty sub-mlist a nonempty
/// math-field type with a null list pointer. Section 692 prints its subsidiary
/// marker and newline, unlike the wholly absent `empty` field.
#[test]
fn math_dump_distinguishes_empty_submlist() {
    let mut stores = Universe::new();
    let empty = stores.freeze_node_list(&[]);
    let list = stores.freeze_node_list(&[
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::Empty,
        )),
        Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::SubMlist(empty),
        )),
    ]);

    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 100,
                depth: 100,
            },
        ),
        "\\mathord\n\\mathord\n.\n",
    );
}

/// TeX82 §§689--690 (`tex.web:13581-13622`) visit all four choice arms;
/// §692 applies the same depth cutoff independently within each arm.
#[test]
fn math_dump_depth_and_choice_arms() {
    let mut stores = Universe::new();
    let arm = |stores: &mut Universe, ch| {
        stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
            NoadKind::Normal(NoadClass::Ord),
            MathField::MathChar(math_char(0, ch)),
        ))])
    };
    let display = arm(&mut stores, 'D');
    let text = arm(&mut stores, 'T');
    let script = arm(&mut stores, 'S');
    let script_script = arm(&mut stores, 's');
    let list = stores.freeze_node_list(&[Node::MathChoice(MathChoice {
        display,
        text,
        script,
        script_script,
    })]);

    assert_eq!(
        dump_node_list(
            &stores,
            list,
            DumpConfig {
                breadth: 100,
                depth: 0,
            },
        ),
        concat!(
            "\\mathchoice\n",
            "D\\mathord []\n",
            "T\\mathord []\n",
            "S\\mathord []\n",
            "s\\mathord []\n",
        ),
    );
}

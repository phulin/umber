//! Regression coverage for `DumpConfig` and the `\showbox`/`\showlists`
//! node-list renderer.

use super::*;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::math::{LimitType, MathChar, MathChoice, MathField, MathNoad, NoadClass, NoadKind};
use tex_state::node::{
    AdjustNode, BoxNodeFields, DiscKind, GlueKind, KernKind, LeaderPayload, Node, Sign, UnsetKind,
    UnsetNode, UnsetNodeFields,
};
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
fn glue_subtype_dump_matrix_preserves_pt_and_mu_identity() {
    let mut stores = Universe::new();
    let spec = stores.intern_glue(GlueSpec {
        width: Scaled::from_raw(Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    let empty = stores.freeze_node_list(&[]);
    let leader = LeaderPayload::HList(zero_sized_hbox(empty));
    let cases = [
        (GlueKind::Normal, None, "\\glue 1.0\n"),
        (GlueKind::TabSkip, None, "\\glue(\\tabskip) 1.0\n"),
        (GlueKind::BaselineSkip, None, "\\glue(\\baselineskip) 1.0\n"),
        (GlueKind::LineSkip, None, "\\glue(\\lineskip) 1.0\n"),
        (GlueKind::TopSkip, None, "\\glue(\\topskip) 1.0\n"),
        (GlueKind::SplitTopSkip, None, "\\glue(\\splittopskip) 1.0\n"),
        (GlueKind::LeftSkip, None, "\\glue(\\leftskip) 1.0\n"),
        (GlueKind::RightSkip, None, "\\glue(\\rightskip) 1.0\n"),
        (GlueKind::ParFillSkip, None, "\\glue(\\parfillskip) 1.0\n"),
        (
            GlueKind::AboveDisplaySkip,
            None,
            "\\glue(\\abovedisplayskip) 1.0\n",
        ),
        (
            GlueKind::BelowDisplaySkip,
            None,
            "\\glue(\\belowdisplayskip) 1.0\n",
        ),
        (
            GlueKind::AboveDisplayShortSkip,
            None,
            "\\glue(\\abovedisplayshortskip) 1.0\n",
        ),
        (
            GlueKind::BelowDisplayShortSkip,
            None,
            "\\glue(\\belowdisplayshortskip) 1.0\n",
        ),
        (
            GlueKind::Leaders,
            Some(leader),
            "\\leaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
        ),
        (
            GlueKind::Cleaders,
            Some(leader),
            "\\cleaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
        ),
        (
            GlueKind::Xleaders,
            Some(leader),
            "\\xleaders 1.0\n.\\hbox(0.0+0.0)x0.0\n",
        ),
        (GlueKind::MuSkip, None, "\\glue(\\mskip) 1.0mu\n"),
        (GlueKind::ThinMuSkip, None, "\\glue(\\thinmuskip) 1.0mu\n"),
        (GlueKind::MedMuSkip, None, "\\glue(\\medmuskip) 1.0mu\n"),
        (GlueKind::ThickMuSkip, None, "\\glue(\\thickmuskip) 1.0mu\n"),
        (GlueKind::NonScript, None, "\\glue(\\nonscript) 1.0\n"),
    ];

    for (kind, payload, expected) in cases {
        let node = Node::Glue {
            spec,
            kind,
            leader: payload,
        };
        assert_eq!(
            dump_node_slice(
                &stores,
                &[node],
                DumpConfig {
                    breadth: 10,
                    depth: 10
                }
            ),
            expected,
            "wrong dump for {kind:?}",
        );
        assert_eq!(stores.glue(spec).width, Scaled::from_raw(Scaled::UNITY));
        assert!(stores.nodes(empty).is_empty());
    }
}

#[test]
fn glue_unit_order_and_sign_matrix_is_exact_and_immutable() {
    let mut stores = Universe::new();
    let cases = [
        (
            Order::Normal,
            Order::Filll,
            "2.0 plus -3.0 minus 4.0filll",
            "2.0mu plus -3.0mu minus 4.0filll",
        ),
        (
            Order::Fil,
            Order::Fill,
            "2.0 plus -3.0fil minus 4.0fill",
            "2.0mu plus -3.0fil minus 4.0fill",
        ),
        (
            Order::Fill,
            Order::Fil,
            "2.0 plus -3.0fill minus 4.0fil",
            "2.0mu plus -3.0fill minus 4.0fil",
        ),
        (
            Order::Filll,
            Order::Normal,
            "2.0 plus -3.0filll minus 4.0",
            "2.0mu plus -3.0filll minus 4.0mu",
        ),
    ];
    for (stretch_order, shrink_order, ordinary, math) in cases {
        let value = GlueSpec {
            width: Scaled::from_raw(2 * Scaled::UNITY),
            stretch: Scaled::from_raw(-3 * Scaled::UNITY),
            stretch_order,
            shrink: Scaled::from_raw(4 * Scaled::UNITY),
            shrink_order,
        };
        let spec = stores.intern_glue(value);
        for (kind, expected) in [(GlueKind::Normal, ordinary), (GlueKind::MuSkip, math)] {
            let node = Node::Glue {
                spec,
                kind,
                leader: None,
            };
            let prefix = if kind == GlueKind::MuSkip {
                "\\glue(\\mskip) "
            } else {
                "\\glue "
            };
            assert_eq!(
                dump_node_slice(
                    &stores,
                    &[node],
                    DumpConfig {
                        breadth: 10,
                        depth: 10
                    }
                ),
                format!("{prefix}{expected}\n"),
            );
            assert_eq!(stores.glue(spec), value, "dumping must not rewrite glue");
        }
    }

    let signs = [
        (
            2,
            3,
            4,
            "2.0 plus 3.0 minus 4.0",
            "2.0mu plus 3.0mu minus 4.0mu",
        ),
        (
            2,
            3,
            -4,
            "2.0 plus 3.0 minus -4.0",
            "2.0mu plus 3.0mu minus -4.0mu",
        ),
        (
            2,
            -3,
            4,
            "2.0 plus -3.0 minus 4.0",
            "2.0mu plus -3.0mu minus 4.0mu",
        ),
        (
            2,
            -3,
            -4,
            "2.0 plus -3.0 minus -4.0",
            "2.0mu plus -3.0mu minus -4.0mu",
        ),
        (
            -2,
            3,
            4,
            "-2.0 plus 3.0 minus 4.0",
            "-2.0mu plus 3.0mu minus 4.0mu",
        ),
        (
            -2,
            3,
            -4,
            "-2.0 plus 3.0 minus -4.0",
            "-2.0mu plus 3.0mu minus -4.0mu",
        ),
        (
            -2,
            -3,
            4,
            "-2.0 plus -3.0 minus 4.0",
            "-2.0mu plus -3.0mu minus 4.0mu",
        ),
        (
            -2,
            -3,
            -4,
            "-2.0 plus -3.0 minus -4.0",
            "-2.0mu plus -3.0mu minus -4.0mu",
        ),
    ];
    for (width, stretch, shrink, ordinary, math) in signs {
        let value = GlueSpec {
            width: Scaled::from_raw(width * Scaled::UNITY),
            stretch: Scaled::from_raw(stretch * Scaled::UNITY),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(shrink * Scaled::UNITY),
            shrink_order: Order::Normal,
        };
        assert_eq!(format_glue(value, ""), ordinary);
        assert_eq!(format_glue(value, "mu"), math);
    }
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

/// TeX82 §186 (`tex.web:3740-3752`) decodes an unset node's quarterword
/// span count as one less than its displayed column count. A zero field has no
/// suffix; nonzero fields use the exact ` (N columns)` spelling before the
/// independently ordered stretch and shrink components.
#[test]
fn unset_box_prints_encoded_column_count_and_glue_fields() {
    let mut stores = Universe::new();
    let children = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    }]);
    let unset = |span_count| {
        Node::Unset(UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::HBox,
            width: Scaled::from_raw(4 * Scaled::UNITY),
            height: Scaled::from_raw(5 * Scaled::UNITY),
            depth: Scaled::from_raw(6 * Scaled::UNITY),
            span_count,
            stretch: Scaled::from_raw(2 * Scaled::UNITY),
            stretch_order: Order::Fil,
            shrink: Scaled::from_raw(3 * Scaled::UNITY),
            shrink_order: Order::Normal,
            children,
        }))
    };
    let source = [unset(0), unset(1), unset(2)];
    let list = stores.freeze_node_list(&source);
    let before_source = stores.nodes(list).to_vec();
    let before_children = stores.nodes(children).to_vec();

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
            "\\unsetbox(5.0+6.0)x4.0, stretch 2.0fil, shrink 3.0\n",
            ".\\kern 1.0\n",
            "\\unsetbox(5.0+6.0)x4.0 (2 columns), stretch 2.0fil, shrink 3.0\n",
            ".\\kern 1.0\n",
            "\\unsetbox(5.0+6.0)x4.0 (3 columns), stretch 2.0fil, shrink 3.0\n",
            ".\\kern 1.0\n",
        ),
    );
    assert_eq!(stores.nodes(list), before_source);
    assert_eq!(stores.nodes(children), before_children);

    assert_eq!(
        dump_node_slice(
            &stores,
            &source[1..2],
            DumpConfig {
                breadth: 100,
                depth: 0,
            },
        ),
        "\\unsetbox(5.0+6.0)x4.0 (2 columns), stretch 2.0fil, shrink 3.0 []\n",
    );
    assert_eq!(stores.nodes(list), before_source);
    assert_eq!(stores.nodes(children), before_children);
}

fn zero_sized_hbox(children: NodeListId) -> BoxNode {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: tex_state::node::BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    })
}

/// TeX82 §198 initializes `show_node_list` at depth -1. Thus depth zero
/// still prints the selected box itself, depth one admits exactly its child
/// box, and depth two admits that box's child. A negative threshold suppresses
/// the entire traversal. The ` []` marker records a nonempty hidden child list.
#[test]
fn showbox_depth_limits_nested_boxes_at_exact_thresholds() {
    let mut stores = Universe::new();
    let kern = Node::Kern {
        amount: Scaled::from_raw(Scaled::UNITY),
        kind: KernKind::Explicit,
    };
    let inner_children = stores.freeze_node_list(std::slice::from_ref(&kern));
    let inner = Node::HList(zero_sized_hbox(inner_children));
    let outer_children = stores.freeze_node_list(std::slice::from_ref(&inner));
    let outer = Node::HList(zero_sized_hbox(outer_children));
    let root = stores.freeze_node_list(std::slice::from_ref(&outer));

    let before_root = stores.nodes(root).to_vec();
    let before_outer = stores.nodes(outer_children).to_vec();
    let before_inner = stores.nodes(inner_children).to_vec();
    let render = |depth| dump_node_list(&stores, root, DumpConfig { breadth: 5, depth });

    assert_eq!(render(-1), "");
    assert_eq!(render(0), "\\hbox(0.0+0.0)x0.0 []\n");
    assert_eq!(render(1), "\\hbox(0.0+0.0)x0.0\n.\\hbox(0.0+0.0)x0.0 []\n");
    assert_eq!(
        render(2),
        concat!(
            "\\hbox(0.0+0.0)x0.0\n",
            ".\\hbox(0.0+0.0)x0.0\n",
            "..\\kern 1.0\n",
        )
    );
    assert_eq!(stores.nodes(root), before_root);
    assert_eq!(stores.nodes(outer_children), before_outer);
    assert_eq!(stores.nodes(inner_children), before_inner);
}

/// Subsidiary lists use the same exact depth and breadth accounting as box
/// children. This covers the non-box edges most prone to being accidentally
/// traversed with an independent limit: adjustments, leader payload boxes,
/// and discretionary pre/post/replacement lists.
#[test]
fn showbox_limits_side_lists_leaders_and_discretionaries_without_mutation() {
    let mut stores = Universe::new();
    let kern = |points| Node::Kern {
        amount: Scaled::from_raw(points * Scaled::UNITY),
        kind: KernKind::Explicit,
    };
    let two_kerns = stores.freeze_node_list(&[kern(1), kern(2)]);
    let empty = stores.freeze_node_list(&[]);
    let glue = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);

    let adjust = Node::Adjust(AdjustNode::ordinary(two_kerns));
    let leader = Node::Glue {
        spec: glue,
        kind: GlueKind::Leaders,
        leader: Some(LeaderPayload::HList(zero_sized_hbox(two_kerns))),
    };
    let disc = Node::Disc {
        kind: DiscKind::Discretionary,
        pre: two_kerns,
        post: two_kerns,
        replace: two_kerns,
    };
    let source = [adjust, leader, disc];
    let before = stores.nodes(two_kerns).to_vec();
    let render = |node: &Node| {
        dump_node_slice(
            &stores,
            std::slice::from_ref(node),
            DumpConfig {
                breadth: 1,
                depth: 10,
            },
        )
    };

    assert_eq!(render(&source[0]), "\\vadjust\n.\\kern 1.0\n.etc.\n");
    assert_eq!(
        render(&source[1]),
        concat!(
            "\\leaders 0.0\n",
            ".\\hbox(0.0+0.0)x0.0\n",
            "..\\kern 1.0\n",
            "..etc.\n",
        )
    );
    assert_eq!(
        render(&source[2]),
        concat!(
            "\\discretionary replacing 2\n",
            ".\\kern 1.0\n",
            ".etc.\n",
            ".|kern 1.0\n",
            ".etc.\n",
            "\\kern 1.0\n",
            "etc.\n",
        )
    );
    assert_eq!(stores.nodes(two_kerns), before, "dumping must be read-only");
    assert!(stores.nodes(empty).is_empty());
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

/// TeX82 §193's insertion-node arm prints every symbolic field before
/// recursively displaying the insertion list. Formatting is observational:
/// neither the insertion payload nor its child order may be consumed or
/// rewritten while `show_node_list` walks the frozen lists.
#[test]
fn insertion_node_dump_prints_all_web_fields() {
    let mut stores = Universe::new();
    let split_top_skip = stores.intern_glue(tex_state::glue::GlueSpec::ZERO);
    let children = [
        Node::Rule {
            width: Some(Scaled::from_raw(0)),
            height: Some(Scaled::from_raw(5 * Scaled::UNITY)),
            depth: Some(Scaled::from_raw(0)),
        },
        Node::Penalty(23),
    ];
    let content = stores.freeze_node_list(&children);
    let insertion = Node::Ins {
        class: 7,
        size: Scaled::from_raw(5 * Scaled::UNITY),
        split_top_skip,
        split_max_depth: Scaled::from_raw(4 * Scaled::UNITY),
        floating_penalty: 100,
        content,
    };
    let source = stores.freeze_node_list(std::slice::from_ref(&insertion));

    assert_eq!(
        dump_node_list(
            &stores,
            source,
            DumpConfig {
                breadth: 100,
                depth: 100,
            },
        ),
        concat!(
            "\\insert7, natural size 5.0; split(0.0,4.0); float cost 100\n",
            ".\\rule(5.0+0.0)x0.0\n",
            ".\\penalty 23\n",
        ),
    );
    assert_eq!(stores.nodes(source), std::slice::from_ref(&insertion));
    assert_eq!(stores.nodes(content), children.as_slice());
    let source_after = stores.nodes(source).to_vec();
    let [
        Node::Ins {
            content: attached_content,
            ..
        },
    ] = source_after.as_slice()
    else {
        panic!("insertion payload was detached");
    };
    assert_eq!(*attached_content, content);
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

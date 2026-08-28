use super::*;
use crate::math::tests::{math_char, noad, root_nodes, setup_universe};
use crate::test_state::TestState;
use tex_state::env::banks::IntParam;
use tex_state::glue::GlueSpec;
use tex_state::math::{MathChoice, NoadClass};

fn context<'a>(
    state: &'a TestState,
    params: &'a MathParams,
    style: Style,
) -> Context<'a, TestState> {
    Context {
        state,
        params,
        style,
        mu: math_unit(params, style),
        layout: NativeNodeTransaction::new(),
        converted: Default::default(),
        source_lists: Default::default(),
        conversion_events: Default::default(),
        capture_replay: false,
        pack_replays: Default::default(),
        event_replays: Default::default(),
        recovered: Default::default(),
        scratch: Default::default(),
    }
}

#[test]
fn first_pass_observes_check_dimensions_pack_for_every_noad() {
    // TeX82 §724 reaches `check_dimensions` for every noad and executes
    // `hpack(new_hlist(q), natural)`. Empty, unscripted noads therefore each
    // publish a distinct zero-size completion without relying on §754.
    let mut stores = setup_universe();
    let input = stores.publish_page_nodes(
        &(0..9)
            .map(|_| {
                Node::MathNoad(MathNoad::new(
                    NoadKind::Normal(NoadClass::Ord),
                    MathField::Empty,
                ))
            })
            .collect::<Vec<_>>(),
    );
    let params = MathParams::read(&stores);

    let layout = mlist_to_hlist(&stores, input, Style::TEXT, false, &params);

    assert_eq!(
        layout.pack_observations(),
        vec![
            MathPackObservation {
                axis: BoxAxis::Horizontal,
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
            };
            9
        ]
    );
}

/// TeX82 §§728--733 and §§761--767: both mlist passes must retain the full
/// eight-style state, perform rules 5/6 bin reclassification, suppress the
/// node following `\nonscript` only in script styles, and insert rule 21
/// penalties only at eligible boundaries.
#[test]
fn mlist_passes_cover_all_styles_bins_nonscript_spacing_and_penalties() {
    let mut stores = setup_universe();
    let arms = ['a', 'b', 'c', '+']
        .map(|ch| stores.publish_page_nodes(&[Node::MathNoad(noad(NoadClass::Ord, ch))]));
    let choice = stores.publish_page_nodes(&[Node::MathChoice(MathChoice {
        display: arms[0],
        text: arms[1],
        script: arms[2],
        script_script: arms[3],
    })]);
    let params = MathParams::read(&stores);
    for (style, expected) in [
        (Style::DISPLAY, 'a'),
        (Style::DISPLAY.cramped_style(), 'a'),
        (Style::TEXT, 'b'),
        (Style::TEXT.cramped_style(), 'b'),
        (Style::SCRIPT, 'c'),
        (Style::SCRIPT.cramped_style(), 'c'),
        (Style::SCRIPT_SCRIPT, '+'),
        (Style::SCRIPT_SCRIPT.cramped_style(), '+'),
    ] {
        let layout = mlist_to_hlist(&stores, choice, style, false, &params);
        let selected = root_nodes(&layout).into_iter().find_map(|node| match node {
            MathNode::Char { ch, .. } => Some(*ch),
            _ => None,
        });
        assert_eq!(selected, Some(expected), "choice arm for {style:?}");
    }

    let mut work = [
        WorkItem::Noad(WorkNoad {
            class: NoadClass::Ord,
            hlist: FrozenHList::default(),
            penalty: INF_PENALTY,
        }),
        WorkItem::Style(Style::SCRIPT),
        WorkItem::Noad(WorkNoad {
            class: NoadClass::Bin,
            hlist: FrozenHList::default(),
            penalty: 123,
        }),
    ];
    convert_final_bin_to_ord(&mut work);
    let WorkItem::Noad(last) = &work[2] else {
        panic!("last work item remains a noad");
    };
    assert_eq!(last.class, NoadClass::Ord);
    assert_eq!(last.penalty, INF_PENALTY);

    let zero = GlueSpec::ZERO;
    let nonscript = stores.publish_page_nodes(&[
        Node::Glue {
            spec: zero,
            kind: GlueKind::NonScript,
            leader: None,
        },
        Node::Kern {
            amount: Scaled::from_raw(2 * Scaled::UNITY),
            kind: KernKind::Mu,
        },
    ]);
    for style in [
        Style::DISPLAY,
        Style::DISPLAY.cramped_style(),
        Style::TEXT,
        Style::TEXT.cramped_style(),
        Style::SCRIPT,
        Style::SCRIPT.cramped_style(),
        Style::SCRIPT_SCRIPT,
        Style::SCRIPT_SCRIPT.cramped_style(),
    ] {
        let layout = mlist_to_hlist(&stores, nonscript, style, false, &params);
        let has_kern = root_nodes(&layout).iter().any(|node| {
            matches!(
                node,
                MathNode::Kern {
                    kind: KernKind::Explicit,
                    ..
                }
            )
        });
        assert_eq!(has_kern, !style.is_script_or_smaller(), "{style:?}");
    }

    let missing = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(math_char('\u{10ffff}')),
    ))]);
    let missing = mlist_to_hlist(&stores, missing, Style::TEXT, false, &params);
    assert!(missing.root().is_empty());

    let penalized = stores.publish_page_nodes(&[
        Node::MathNoad(noad(NoadClass::Ord, 'a')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::MathNoad(noad(NoadClass::Ord, 'b')),
        Node::MathNoad(noad(NoadClass::Rel, '=')),
        Node::MathNoad(noad(NoadClass::Ord, 'c')),
    ]);
    let layout = mlist_to_hlist(&stores, penalized, Style::TEXT, true, &params);
    let penalties: Vec<_> = root_nodes(&layout)
        .iter()
        .filter_map(|node| match node {
            MathNode::Penalty(value) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(penalties, [params.bin_op_penalty, params.rel_penalty]);
}

#[test]
fn middle_and_right_restore_base_style_before_nested_math_choices() {
    // e-TeX [36.727]: unlike a left noad, every middle/right noad resets
    // `cur_style` to the style supplied to `mlist_to_hlist`.
    let mut stores = setup_universe();
    let arms = ['a', 'b', 'c', '+']
        .map(|ch| stores.publish_page_nodes(&[Node::MathNoad(noad(NoadClass::Ord, ch))]));
    let choice = MathChoice {
        display: arms[0],
        text: arms[1],
        script: arms[2],
        script_script: arms[3],
    };
    let params = MathParams::read(&stores);
    let selected_char = |layout: &MathLayout| {
        let mut stack = vec![layout.root()];
        while let Some(list) = stack.pop() {
            for node in layout.logical_nodes(list) {
                match node {
                    MathNode::Char { ch, .. } => return Some(*ch),
                    MathNode::HList(boxed) | MathNode::VList(boxed) => stack.push(boxed.list),
                    _ => {}
                }
            }
        }
        None
    };

    for (base, expected) in [
        (Style::DISPLAY, 'a'),
        (Style::TEXT, 'b'),
        (Style::SCRIPT, 'c'),
        (Style::SCRIPT_SCRIPT, '+'),
    ] {
        for boundary in [
            NoadKind::MiddleDelimiter { delimiter: 0 },
            NoadKind::RightDelimiter { delimiter: 0 },
        ] {
            let nested = stores.publish_page_nodes(&[
                Node::MathStyle(tex_state::math::MathStyle::ScriptScript),
                Node::MathNoad(MathNoad::new(boundary.clone(), MathField::Empty)),
                Node::MathChoice(choice),
            ]);
            let input = stores.publish_page_nodes(&[Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubMlist(nested),
            ))]);
            let layout = mlist_to_hlist(&stores, input, base, false, &params);
            let selected = selected_char(&layout);
            assert_eq!(
                selected,
                Some(expected),
                "base={base:?}, boundary={boundary:?}"
            );
        }
    }

    let left = stores.publish_page_nodes(&[
        Node::MathStyle(tex_state::math::MathStyle::Script),
        Node::MathNoad(MathNoad::new(
            NoadKind::LeftDelimiter { delimiter: 0 },
            MathField::Empty,
        )),
        Node::MathChoice(choice),
    ]);
    let layout = mlist_to_hlist(&stores, left, Style::DISPLAY, false, &params);
    assert!(
        root_nodes(&layout)
            .iter()
            .any(|node| matches!(node, MathNode::Char { ch: 'c', .. }))
    );
}

#[test]
fn tex82_second_pass_spacing_delimiter_penalty_matrix() {
    let stores = setup_universe();
    let params = MathParams::read(&stores);
    let classes = [
        NoadClass::Ord,
        NoadClass::Op,
        NoadClass::Bin,
        NoadClass::Rel,
        NoadClass::Open,
        NoadClass::Close,
        NoadClass::Punct,
        NoadClass::Inner,
    ];
    for left in classes {
        for right in classes {
            let mut ctx = context(&stores, &params, Style::TEXT);
            let mut work = vec![
                WorkItem::Noad(WorkNoad {
                    class: left,
                    hlist: FrozenHList::default(),
                    penalty: INF_PENALTY,
                }),
                WorkItem::Noad(WorkNoad {
                    class: right,
                    hlist: FrozenHList::default(),
                    penalty: INF_PENALTY,
                }),
            ];
            let root = second_pass(
                &mut ctx,
                Style::TEXT,
                &mut work,
                false,
                Scaled::from_raw(0),
                Scaled::from_raw(0),
            );
            let layout = ctx.layout.finish(root);
            let actual = layout
                .logical_nodes(root)
                .into_iter()
                .find_map(|node| match node {
                    MathNode::Glue { kind, .. } => Some(*kind),
                    _ => None,
                });
            let spacing = spacing::inter_noad_spacing(left, right, Style::TEXT);
            let expected = spacing::spacing_glue(spacing, &params, math_unit(&params, Style::TEXT))
                .map(|_| math_glue_kind_for_spacing(spacing));
            assert_eq!(actual, expected, "spacing for {left:?} -> {right:?}");
        }
    }

    let mut ctx = context(&stores, &params, Style::TEXT);
    let mut work = vec![
        WorkItem::Delimiter(WorkDelimiter {
            left_class: NoadClass::Open,
            right_class: NoadClass::Open,
            delimiter: 0,
        }),
        WorkItem::Noad(WorkNoad {
            class: NoadClass::Bin,
            hlist: FrozenHList::default(),
            penalty: 321,
        }),
        WorkItem::Noad(WorkNoad {
            class: NoadClass::Ord,
            hlist: FrozenHList::default(),
            penalty: INF_PENALTY,
        }),
        WorkItem::Delimiter(WorkDelimiter {
            left_class: NoadClass::Close,
            right_class: NoadClass::Close,
            delimiter: 0,
        }),
    ];
    let root = second_pass(
        &mut ctx,
        Style::TEXT,
        &mut work,
        true,
        Scaled::from_raw(20),
        Scaled::from_raw(5),
    );
    let layout = ctx.layout.finish(root);
    let nodes = layout.logical_nodes(root);
    assert!(
        nodes
            .iter()
            .any(|node| matches!(node, MathNode::Penalty(321)))
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| matches!(node, MathNode::HList(_)))
            .count(),
        2
    );
}

#[test]
fn explicit_penalty_suppresses_preceding_bin_penalty() {
    // TeX82 §767: rule 21 inspects the physical node following a noad. A
    // source penalty remains canonical during the detached math transaction,
    // and it must still suppress the automatic bin-op penalty.
    let mut stores = setup_universe();
    stores.set_int_param(IntParam::BIN_OP_PENALTY, -3333);
    let input = stores.publish_page_nodes(&[
        Node::MathNoad(noad(NoadClass::Ord, 'A')),
        Node::MathNoad(noad(NoadClass::Bin, '+')),
        Node::Penalty(1000),
        Node::MathNoad(noad(NoadClass::Ord, 'A')),
    ]);
    let params = MathParams::read(&stores);

    let layout = mlist_to_hlist(&stores, input, Style::TEXT, true, &params);
    let penalties = root_nodes(&layout)
        .into_iter()
        .filter_map(|node| match node {
            MathNode::Penalty(value) => Some(*value),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(penalties, [1000]);
}

use super::*;
use crate::math::tests::{math_char, noad, root_nodes, setup_universe};
use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::math::{MathChoice, NoadClass};

fn context<'a>(state: &'a Universe, params: &'a MathParams, style: Style) -> Context<'a, Universe> {
    Context {
        state,
        params,
        style,
        mu: math_unit(params, style),
        layout: MathLayoutBuilder::new(),
        converted: Default::default(),
        source_lists: Default::default(),
    }
}

#[test]
fn tex82_first_pass_choice_bin_and_nonscript_matrix() {
    let mut stores = setup_universe();
    let arms = ['a', 'b', 'c', '+']
        .map(|ch| stores.freeze_node_list(&[Node::MathNoad(noad(NoadClass::Ord, ch))]));
    let choice = stores.freeze_node_list(&[Node::MathChoice(MathChoice {
        display: arms[0],
        text: arms[1],
        script: arms[2],
        script_script: arms[3],
    })]);
    let params = MathParams::read(&stores);
    for (style, expected) in [
        (Style::DISPLAY, 'a'),
        (Style::TEXT, 'b'),
        (Style::SCRIPT, 'c'),
        (Style::SCRIPT_SCRIPT, '+'),
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

    let zero = stores.intern_glue(GlueSpec::ZERO);
    let nonscript = stores.freeze_node_list(&[
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
    let text = mlist_to_hlist(&stores, nonscript, Style::TEXT, false, &params);
    assert!(root_nodes(&text).iter().any(|node| matches!(
        node,
        MathNode::Kern {
            kind: KernKind::Explicit,
            ..
        }
    )));
    let script = mlist_to_hlist(&stores, nonscript, Style::SCRIPT, false, &params);
    assert!(
        !root_nodes(&script)
            .iter()
            .any(|node| matches!(node, MathNode::Kern { .. }))
    );

    let missing = stores.freeze_node_list(&[Node::MathNoad(MathNoad::new(
        NoadKind::Normal(NoadClass::Ord),
        MathField::MathChar(math_char('\u{10ffff}')),
    ))]);
    let missing = mlist_to_hlist(&stores, missing, Style::TEXT, false, &params);
    assert!(missing.root().is_empty());
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
            let work = vec![
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
                work,
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
    let work = vec![
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
        work,
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

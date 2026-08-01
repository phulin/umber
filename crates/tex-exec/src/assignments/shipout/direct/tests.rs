use super::*;

fn permutation(directions_and_penalties: &[Node]) -> Option<Vec<usize>> {
    let mut stores = Universe::new();
    let list = stores.freeze_node_list(directions_and_penalties);
    normalize::direction_permutation_for_box(stores.nodes(list), tex_state::node::BoxLr::Normal)
}

fn test_hbox(stores: &mut Universe, children: &[Node], box_lr: tex_state::node::BoxLr) -> Node {
    let children = stores.freeze_node_list(children);
    Node::HList(StateBoxNode::new(tex_state::node::BoxNodeFields {
        width: tex_state::scaled::Scaled::from_raw(100),
        height: tex_state::scaled::Scaled::from_raw(20),
        depth: tex_state::scaled::Scaled::from_raw(3),
        shift: tex_state::scaled::Scaled::from_raw(0),
        box_lr,
        glue_set: tex_state::scaled::GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))
}

fn staged_artifact(stores: &mut Universe, root: Node) -> tex_out::PageArtifact {
    crate::assignments::test_stage_shipout_artifact(root, stores).expect("page stages")
}

#[test]
fn direction_permutation_preserves_nested_m_l_r_chunks_exactly() {
    use tex_state::node::MathBoundary::{BeginL, BeginM, BeginR, EndL, EndM, EndR};

    let cases = [
        (
            "ordinary LTR markers disappear",
            vec![
                Node::Direction(BeginL),
                Node::Penalty(1),
                Node::Penalty(2),
                Node::Direction(EndL),
            ],
            vec![1, 2],
        ),
        (
            "RTL reverses chunks",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Penalty(2),
                Node::Direction(EndR),
            ],
            vec![2, 1],
        ),
        (
            "LTR nested in RTL stays one ordered chunk",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Direction(BeginL),
                Node::Penalty(2),
                Node::Penalty(3),
                Node::Direction(EndL),
                Node::Penalty(4),
                Node::Direction(EndR),
            ],
            vec![6, 3, 4, 1],
        ),
        (
            "math nested in RTL stays one ordered chunk",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Direction(BeginM),
                Node::Penalty(2),
                Node::Penalty(3),
                Node::Direction(EndM),
                Node::Penalty(4),
                Node::Direction(EndR),
            ],
            vec![6, 3, 4, 1],
        ),
        (
            "RTL nested in LTR reverses only its own chunks",
            vec![
                Node::Direction(BeginL),
                Node::Penalty(1),
                Node::Direction(BeginR),
                Node::Penalty(2),
                Node::Penalty(3),
                Node::Direction(EndR),
                Node::Penalty(4),
                Node::Direction(EndL),
            ],
            vec![1, 4, 3, 6],
        ),
    ];

    for (reason, nodes, expected) in cases {
        assert_eq!(permutation(&nodes), Some(expected), "{reason}");
    }
}

#[test]
fn direction_permutation_recovers_from_unmatched_boundaries_exactly() {
    use tex_state::node::MathBoundary::{BeginL, BeginM, BeginR, EndL, EndM, EndR};

    let cases = [
        (
            "orphan ends are removed without changing ordinary order",
            vec![
                Node::Direction(EndR),
                Node::Penalty(1),
                Node::Direction(EndM),
                Node::Penalty(2),
            ],
            vec![1, 3],
        ),
        (
            "an unclosed RTL segment is finalized at list end",
            vec![
                Node::Penalty(1),
                Node::Direction(BeginR),
                Node::Penalty(2),
                Node::Penalty(3),
            ],
            vec![0, 3, 2],
        ),
        (
            "a mismatched end is ignored and does not close its parent",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Direction(EndL),
                Node::Penalty(2),
                Node::Direction(EndR),
            ],
            vec![3, 1],
        ),
        (
            "unclosed nested segments finalize from the inside out",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Direction(BeginM),
                Node::Penalty(2),
                Node::Penalty(3),
            ],
            vec![3, 4, 1],
        ),
        (
            "mismatched nesting leaves each opener to end recovery",
            vec![
                Node::Direction(BeginR),
                Node::Penalty(1),
                Node::Direction(BeginL),
                Node::Penalty(2),
                Node::Direction(EndR),
                Node::Penalty(3),
                Node::Direction(EndM),
            ],
            vec![3, 5, 1],
        ),
    ];

    for (reason, nodes, expected) in cases {
        assert_eq!(permutation(&nodes), Some(expected), "{reason}");
    }
}

#[test]
fn direction_normalization_has_one_exact_artifact_and_dvi_identity() {
    use tex_state::node::MathBoundary::{BeginM, BeginR, EndM, EndR};

    let marked = [
        Node::Direction(BeginR),
        Node::Penalty(1),
        Node::Direction(BeginM),
        Node::Penalty(2),
        Node::Penalty(3),
        Node::Direction(EndM),
        Node::Penalty(4),
        Node::Direction(EndR),
    ];
    let canonical = [
        Node::Penalty(4),
        Node::Penalty(2),
        Node::Penalty(3),
        Node::Penalty(1),
    ];

    let mut marked_stores = Universe::new();
    let marked_root = test_hbox(&mut marked_stores, &marked, tex_state::node::BoxLr::Normal);
    let marked_artifact = staged_artifact(&mut marked_stores, marked_root);

    let mut canonical_stores = Universe::new();
    let canonical_root = test_hbox(
        &mut canonical_stores,
        &canonical,
        tex_state::node::BoxLr::Reversed,
    );
    let canonical_artifact = staged_artifact(&mut canonical_stores, canonical_root);

    assert_eq!(
        marked_artifact.to_bytes(),
        canonical_artifact.to_bytes(),
        "direction markers and box_lr are normalization metadata, not artifact nodes"
    );
    assert!(matches!(
        marked_artifact.root,
        PageNode::HList(ref root)
            if matches!(root.children.as_slice(), [
                PageNode::Penalty(4), PageNode::Penalty(2),
                PageNode::Penalty(3), PageNode::Penalty(1),
            ])
    ));
    assert_eq!(
        tex_out::dvi::write_dvi(&[marked_artifact]).expect("marked DVI serializes"),
        tex_out::dvi::write_dvi(&[canonical_artifact]).expect("canonical DVI serializes"),
        "raw DVI shares the normalized artifact identity"
    );
}

#[test]
fn reversed_box_identity_prevents_a_second_shipout_permutation() {
    let mut stores = Universe::new();
    let list = stores.freeze_node_list(&[
        Node::Direction(Direction::BeginR),
        Node::Penalty(1),
        Node::Penalty(2),
        Node::Direction(Direction::EndR),
    ]);

    assert_eq!(
        normalize::direction_permutation_for_box(
            stores.nodes(list),
            tex_state::node::BoxLr::Normal,
        ),
        Some(vec![2, 1]),
    );
    assert_eq!(
        normalize::direction_permutation_for_box(
            stores.nodes(list),
            tex_state::node::BoxLr::Reversed,
        ),
        None,
        "merged e-TeX WEB §53a trusts box_lr instead of inferring reversal from children",
    );
}

#[test]
fn ordinary_page_effects_do_not_require_positioned_shipout() {
    assert!(!needs_positioned_shipout(&[
        PageEffect::Write {
            sink: EffectSink::Terminal,
            text: "ordinary".to_owned(),
        },
        PageEffect::PdfSave,
        PageEffect::PdfRestore,
        PageEffect::PdfSnapState {
            x: tex_state::scaled::Scaled::from_raw(17),
            y: tex_state::scaled::Scaled::from_raw(23),
        },
    ]));
}

#[test]
fn position_and_snap_effects_require_positioned_shipout() {
    let zero_glue = PageGlueSpec {
        width: tex_state::scaled::Scaled::from_raw(0),
        stretch: tex_state::scaled::Scaled::from_raw(0),
        stretch_order: PageGlueOrder::Normal,
        shrink: tex_state::scaled::Scaled::from_raw(0),
        shrink_order: PageGlueOrder::Normal,
    };
    for effect in [
        PageEffect::PdfSavePosition,
        PageEffect::PdfSnapRefPoint,
        PageEffect::PdfSnapY { spec: zero_glue },
        PageEffect::PdfSnapYComp { ratio: 500 },
    ] {
        assert!(needs_positioned_shipout(&[effect]));
    }
}

#[test]
fn dvi_accepts_only_canonical_deferred_whatsit_exceptions() {
    let effects = [
        PageEffect::Write {
            sink: EffectSink::Terminal,
            text: "write".to_owned(),
        },
        PageEffect::Special {
            class: "special".to_owned(),
            payload: b"payload".to_vec(),
        },
        PageEffect::PdfSavePosition,
    ];
    assert!(reject_pdf_nodes_in_dvi(&effects).is_ok());

    let rejected = reject_pdf_nodes_in_dvi(&[PageEffect::PdfLiteral {
        mode: tex_out::PdfLiteralMode::Direct,
        payload: b"q".to_vec(),
    }])
    .expect_err("a deferred PDF node must fail when DVI traversal reaches it");
    assert_eq!(
        rejected.to_string(),
        "pdfTeX error (ext4): \\pdfliteral used while \\pdfoutput is not set."
    );
}

#[test]
fn openout_sidecars_share_the_filtered_page_effect_index_space() {
    let mut world = tex_state::World::memory();
    world.record_pdf_object_placeholder("before");
    world.open_out(tex_state::StreamSlot::new(2), "same.out");
    world.record_pdf_object_placeholder("between");
    world.open_out(tex_state::StreamSlot::new(2), "same.out");

    let pending = pending_page_effects(&world, world.effect_records().len());
    assert_eq!(pending.effects.len(), 2);
    assert!(pending.effects.iter().all(|effect| matches!(
        effect,
        PageEffect::OpenOut {
            stream: 2,
            path
        } if path == "same.out"
    )));
    assert_eq!(
        pending
            .open_out_occurrences
            .iter()
            .map(|(page_index, position)| (*page_index, position.raw()))
            .collect::<Vec<_>>(),
        [(0, 2), (1, 4)],
        "omitted effects change absolute World positions, never page indices"
    );
}

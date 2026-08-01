use super::*;

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

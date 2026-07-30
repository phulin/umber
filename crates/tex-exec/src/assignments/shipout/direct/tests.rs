use super::*;

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
fn openout_sidecars_share_the_filtered_page_effect_index_space() {
    let mut world = tex_state::World::memory();
    world.record_pdf_object_placeholder("before");
    world.open_out(tex_state::StreamSlot::new(2), "same.out");
    world.record_pdf_object_placeholder("between");
    world.open_out(tex_state::StreamSlot::new(2), "same.out");

    let pending = pending_page_effects(&world);
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

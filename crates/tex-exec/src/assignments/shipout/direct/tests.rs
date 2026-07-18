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

use super::FontNumberTimeline;
use tex_fonts::FontSourceIdentity;

#[test]
fn later_engine_font_reuses_vf_local_font_number() {
    let identity = |byte| FontSourceIdentity::from_bytes([byte; 8]);
    let engine = [identity(0), identity(1), identity(3), identity(4)];
    let mut timeline = FontNumberTimeline::new(&engine);

    timeline
        .advance_engine_to(1)
        .expect("first engine font fits the timeline");
    assert_eq!(
        timeline.register(identity(2)).expect("first VF local fits"),
        2
    );
    assert_eq!(
        timeline
            .register(identity(3))
            .expect("future engine font fits as a VF local"),
        3
    );
    assert_eq!(
        timeline.register(identity(5)).expect("last VF local fits"),
        4
    );

    timeline
        .advance_engine_to(3)
        .expect("later engine fonts fit the timeline");
    assert_eq!(
        timeline.engine_number(2),
        3,
        "pdftex.web §32e reuses the VF-local number for a later equal engine font"
    );
    assert_eq!(
        timeline.engine_number(3),
        5,
        "only genuinely new engine fonts advance the shared font_ptr timeline"
    );
}

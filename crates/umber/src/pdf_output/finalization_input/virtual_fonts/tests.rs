use super::{FontNumberTimeline, LocalInstance, load_local_instance};
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

#[test]
fn detached_vf_local_instance_inherits_parent_expansion() {
    const FIX_ONE: i32 = 1 << 20;
    const CMR10: &[u8] = include_bytes!("../../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut vf = vec![247, 202, 0];
    vf.extend_from_slice(&0u32.to_be_bytes());
    vf.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    vf.extend_from_slice(&[243, 7]);
    vf.extend_from_slice(&0u32.to_be_bytes());
    vf.extend_from_slice(&FIX_ONE.to_be_bytes());
    vf.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    vf.extend_from_slice(&[0, 5]);
    vf.extend_from_slice(b"cmr10");
    vf.push(248);
    while !vf.len().is_multiple_of(4) {
        vf.push(248);
    }
    let mut resources = crate::PdfVirtualFontResources::default();
    resources.virtual_fonts.insert(
        "root".into(),
        crate::CachedVirtualFont {
            content_id: umber_vfs::FileContentId::for_bytes(&vf),
            program: tex_fonts::VfProgram::parse(&vf).expect("test VF"),
        },
    );
    resources.local_tfms.insert(
        "cmr10".into(),
        crate::CachedLocalTfm {
            content_id: umber_vfs::FileContentId::for_bytes(CMR10),
            bytes: CMR10.to_vec(),
            font: tex_fonts::TfmFont::parse(CMR10).expect("test TFM"),
        },
    );
    let parent = LocalInstance {
        identity: FontSourceIdentity::from_bytes([9; 8]),
        name: "root".into(),
        size: tex_arith::Scaled::from_raw(10 * tex_arith::Scaled::UNITY),
        expansion_ratio: 30,
    };

    let (instance, leaf, base) =
        load_local_instance(&resources, &parent, 7).expect("load expanded local font");
    let base = base.expect("expanded leaf retains its base font");

    assert_eq!(instance.expansion_ratio, 30);
    assert!(matches!(
        leaf.construction(),
        tex_fonts::FontConstruction::Expanded { source, ratio }
            if *source == base.source_identity() && *ratio == 30
    ));
}

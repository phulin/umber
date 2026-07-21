use super::*;
use crate::{CachedLocalTfm, CachedVirtualFont};
use tex_fonts::TfmFont;
use tex_out::positioned::PositionedTextRun;
use umber_vfs::FileContentId;

const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");
const FIX_ONE: i32 = 1 << 20;

fn vf(local: &[u8], commands: &[u8]) -> Vec<u8> {
    let mut bytes = vec![247, 202, 0];
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    bytes.extend_from_slice(&[243, 7]);
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&FIX_ONE.to_be_bytes());
    bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    bytes.push(0);
    bytes.push(local.len() as u8);
    bytes.extend_from_slice(local);
    bytes.push(commands.len() as u8);
    bytes.push(b'A');
    bytes.extend_from_slice(&[8, 0, 0]);
    bytes.extend_from_slice(commands);
    bytes.push(248);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(248);
    }
    bytes
}

fn loaded(name: &str, bytes: &[u8]) -> LoadedFont {
    let tfm = TfmFont::parse(bytes).expect("fixture TFM");
    LoadedFont::new(
        name,
        format!("{name}.tfm"),
        FileContentId::for_bytes(bytes).bytes(),
        tfm.header.checksum,
        tfm.header.design_size,
        tfm.font_size,
        tfm.parameters
            .values
            .iter()
            .map(|parameter| parameter.value)
            .collect(),
        tfm.font_metrics(),
    )
}

fn page(stores: &mut Universe, root: FontId) -> PositionedPage {
    stores
        .ensure_pdf_font_resource(root)
        .expect("PDF font resource");
    let font = stores.font(root);
    PositionedPage {
        page_index: 0,
        width: Scaled::from_raw(20 * Scaled::UNITY),
        height: Scaled::from_raw(20 * Scaled::UNITY),
        page_origin_x: Scaled::from_raw(0),
        page_origin_y: Scaled::from_raw(0),
        mag: 1000,
        counts: [0; 10],
        fonts: vec![FontResource {
            font_id: 0,
            name: font.name().to_owned(),
            tfm_content_hash: tex_out::ContentIdentity::new(font.content_hash()),
            tfm_checksum: font.checksum(),
            design_size: font.design_size(),
            at_size: font.size(),
            opentype: None,
            semantic_identity: font.source_identity(),
            construction: FontResourceConstruction::Loaded,
        }],
        events: vec![PositionedEvent::TextRun(PositionedTextRun {
            x: Scaled::from_raw(100),
            baseline: Scaled::from_raw(1_000),
            font_id: 0,
            units: vec![TextUnit::Code(b'A')],
            positions: vec![Scaled::from_raw(100)],
            physical_codes: vec![Some(b'A')],
            sources: vec![None],
        })],
        diagnostics: Vec::new(),
        last_saved_position: None,
        snap_reference: (Scaled::from_raw(0), Scaled::from_raw(0)),
    }
}

fn resources(root_vf: Vec<u8>, local_name: &str, local_tfm: &[u8]) -> PdfVirtualFontResources {
    let mut resources = PdfVirtualFontResources::default();
    resources.virtual_fonts.insert(
        "cmr10".to_owned(),
        CachedVirtualFont {
            content_id: FileContentId::for_bytes(&root_vf),
            program: tex_fonts::VfProgram::parse(&root_vf).expect("test VF"),
        },
    );
    resources.local_tfms.insert(
        local_name.to_owned(),
        CachedLocalTfm {
            content_id: FileContentId::for_bytes(local_tfm),
            bytes: local_tfm.to_vec(),
            font: TfmFont::parse(local_tfm).expect("local TFM"),
        },
    );
    resources
}

#[test]
fn lowers_movement_rule_pdf_special_and_real_font_selection() {
    let mut commands = vec![143, 2, 141, 157, 3, 132];
    commands.extend_from_slice(&FIX_ONE.to_be_bytes());
    commands.extend_from_slice(&FIX_ONE.to_be_bytes());
    commands.extend_from_slice(&[142, b'A', 239, 12]);
    commands.extend_from_slice(b"PDF:page:q Q");
    let root_vf = vf(b"cmsy10", &commands);
    let resources = resources(root_vf, "cmsy10", CMSY10);
    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];

    lower_pages(&mut stores, &mut pages, &resources, PdfVfLimits::default()).expect("lower VF");

    let moved_x = checked_add(
        Scaled::from_raw(100),
        scale_fix(2, stores.font(root).size()).expect("test movement scales"),
    )
    .expect("test movement fits");
    assert!(pages[0].fonts.iter().any(|font| font.name == "cmsy10"));
    assert!(pages[0].events.iter().any(|event| matches!(event,
        PositionedEvent::Rule(rule) if rule.x == moved_x
    )));
    assert!(pages[0].events.iter().any(|event| matches!(event,
        PositionedEvent::TextRun(run) if run.positions == [moved_x]
    )));
    assert!(pages[0].events.iter().any(|event| matches!(event,
        PositionedEvent::PdfGraphics(graphics)
            if matches!(&graphics.effect, PageEffect::PdfLiteral { mode: PdfLiteralMode::Page, payload } if payload == b"q Q")
    )));
    let root_resource = stores.pdf_font_resource(root).expect("root resource");
    assert!(
        stores
            .pdf_font_resources()
            .any(|resource| { resource.object_number() != root_resource.object_number() })
    );
}

#[test]
fn rejects_cycles_and_packet_work_overflow() {
    let cycle_vf = vf(b"cmr10", b"A");
    let cycle_resources = resources(cycle_vf, "cmr10", CMR10);
    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];
    assert!(matches!(
        lower_pages(
            &mut stores,
            &mut pages,
            &cycle_resources,
            PdfVfLimits::default()
        ),
        Err(PdfBuildError::VirtualFontCycle { .. })
    ));

    let root_vf = vf(b"cmsy10", b"A");
    let resources = resources(root_vf, "cmsy10", CMSY10);
    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];
    assert!(matches!(
        lower_pages(
            &mut stores,
            &mut pages,
            &resources,
            PdfVfLimits {
                max_packet_commands: 0,
                ..PdfVfLimits::default()
            },
        ),
        Err(PdfBuildError::VirtualFontWorkExceeded(0))
    ));
}

#[test]
fn lowers_nested_virtual_fonts_and_enforces_depth() {
    let root_vf = vf(b"cmsy10", b"A");
    let nested_vf = vf(b"cmex10", b"A");
    let mut resources = resources(root_vf, "cmsy10", CMSY10);
    resources.virtual_fonts.insert(
        "cmsy10".to_owned(),
        CachedVirtualFont {
            content_id: FileContentId::for_bytes(&nested_vf),
            program: tex_fonts::VfProgram::parse(&nested_vf).expect("nested VF"),
        },
    );
    resources.local_tfms.insert(
        "cmex10".to_owned(),
        CachedLocalTfm {
            content_id: FileContentId::for_bytes(CMEX10),
            bytes: CMEX10.to_vec(),
            font: TfmFont::parse(CMEX10).expect("leaf TFM"),
        },
    );

    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];
    lower_pages(&mut stores, &mut pages, &resources, PdfVfLimits::default())
        .expect("nested VF lowers");
    assert!(pages[0].fonts.iter().any(|font| font.name == "cmex10"));

    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];
    assert!(matches!(
        lower_pages(
            &mut stores,
            &mut pages,
            &resources,
            PdfVfLimits {
                max_recursion: 1,
                ..PdfVfLimits::default()
            },
        ),
        Err(PdfBuildError::VirtualFontDepthExceeded(1))
    ));
}

#[test]
fn empty_resource_set_leaves_non_virtual_page_unchanged() {
    let mut stores = Universe::new();
    let root = stores.intern_font(loaded("cmr10", CMR10));
    let mut pages = vec![page(&mut stores, root)];
    let before = pages.clone();
    lower_pages(
        &mut stores,
        &mut pages,
        &PdfVirtualFontResources::default(),
        PdfVfLimits::default(),
    )
    .expect("non-VF no-op");
    assert_eq!(pages, before);
}

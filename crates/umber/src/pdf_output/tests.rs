use std::path::Path;

use tex_arith::{FontSizeSpec, Scaled};
use tex_state::InteractionMode;

use super::{
    DEFAULT_PDF_PK_RESOLUTION, is_pdf_sfnt_program, pdf_finalization_input,
    pdf_from_accepted_artifacts_with_virtual_fonts,
};
use crate::{
    FileSessionResolvers, RetainedRootRequest, prepare_pdftex_run_stores,
    run_input_collecting_artifacts_with_profile,
};

#[test]
fn sfnt_program_classification_includes_supported_containers() {
    for name in [b"font.ttf".as_slice(), b"font.otf", b"font.woff2"] {
        assert!(is_pdf_sfnt_program(name));
    }
    assert!(!is_pdf_sfnt_program(b"font.pfb"));
}

#[test]
fn accepted_pdf_finalization_includes_the_unpublished_page_suffix() {
    fn setup() -> tex_state::DetachedPdfCompletion {
        crate::with_engine_universe(|stores| {
            stores.set_interaction_mode(InteractionMode::Nonstop);
            prepare_pdftex_run_stores(stores);
            let mut host =
                FileSessionResolvers::new(Path::new("prepared-pages.tex"), Vec::new(), Vec::new());
            run_input_collecting_artifacts_with_profile(
                stores,
                RetainedRootRequest::authored_job(
                    "prepared-pages",
                    concat!(
                        "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
                        "\\shipout\\vbox{\\hrule width1pt height1pt}",
                        "\\shipout\\vbox{\\hrule width2pt height2pt}",
                        "\\shipout\\vbox{\\hrule width3pt height3pt}\\end",
                    )
                    .as_bytes(),
                    tex_command::CommandProfile::PDFTEX14029,
                ),
                &mut host,
                tex_command::CommandProfile::PDFTEX14029,
            )
            .expect("three package-independent pages ship");
            stores
                .command_context()
                .expect("admit terminal PDF completion")
                .detach_pdf_completion()
                .expect("detach three-page PDF")
        })
        .expect("fresh PDF test universe")
    }

    let completion = setup();
    assert_eq!(completion.pages().len(), 3);
    let direct_pdf = pdf_from_accepted_artifacts_with_virtual_fonts(
        &completion,
        &crate::PdfVirtualFontResources::default(),
        &crate::PdfRawObjectFileReceipt::default(),
    )
    .expect("direct three-page PDF finalizes");
    let direct_query = test_support::pdf_query::PdfQuery::new(
        &direct_pdf,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent parser accepts direct PDF");
    assert_eq!(direct_query.pages().expect("direct page tree").len(), 3);
}

#[test]
fn detached_nested_vf_preserves_exact_local_tfm_identity_and_resources() {
    const FIX_ONE: i32 = 1 << 20;
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
    const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

    fn vf(local: &[u8], scaled_size: i32) -> Vec<u8> {
        let mut bytes = vec![247, 202, 0];
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
        bytes.extend_from_slice(&[243, 7]);
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&scaled_size.to_be_bytes());
        bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
        bytes.extend_from_slice(&[0, local.len() as u8]);
        bytes.extend_from_slice(local);
        bytes.extend_from_slice(&[1, b'A', 8, 0, 0, b'A', 248]);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(248);
        }
        bytes
    }

    fn setup() -> tex_state::DetachedPdfCompletion {
        crate::with_engine_universe(|stores| {
            stores.set_interaction_mode(InteractionMode::Nonstop);
            prepare_pdftex_run_stores(stores);
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed root TFM");
            stores
                .world_mut()
                .set_memory_file(
                    "cmr10.pfb",
                    include_bytes!("../../../../tests/corpus/pdf/embedded_type1/cmr10.pfb")
                        .to_vec(),
                )
                .expect("seed leaf program");
            let mut host =
                FileSessionResolvers::new(Path::new("pdf-test.tex"), Vec::new(), Vec::new());
            run_input_collecting_artifacts_with_profile(
                stores,
                RetainedRootRequest::authored_job(
                    "pdf-test",
                    concat!(
                        "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
                        "\\pdfmapline{=cmex10 CMR10 <cmr10.pfb}",
                        "\\font\\f=cmr10 at 12pt ",
                        "\\shipout\\hbox{\\f A}\\end",
                    )
                    .as_bytes(),
                    tex_command::CommandProfile::PDFTEX14029,
                ),
                &mut host,
                tex_command::CommandProfile::PDFTEX14029,
            )
            .expect("nested VF root page ships");
            stores
                .command_context()
                .expect("admit nested-VF completion")
                .detach_pdf_completion()
                .expect("detach nested-VF PDF")
        })
        .expect("fresh nested-VF universe")
    }

    let root_vf = vf(b"cmsy10", FIX_ONE / 2);
    let nested_vf = vf(b"cmex10", 3 * FIX_ONE / 2);
    let mut resources = crate::PdfVirtualFontResources::default();
    for (name, bytes) in [("cmr10", root_vf), ("cmsy10", nested_vf)] {
        resources.virtual_fonts.insert(
            name.to_owned(),
            crate::CachedVirtualFont {
                content_id: umber_vfs::FileContentId::for_bytes(&bytes),
                program: tex_fonts::VfProgram::parse(&bytes).expect("test VF"),
            },
        );
    }
    for (name, bytes) in [("cmsy10", CMSY10), ("cmex10", CMEX10)] {
        resources.local_tfms.insert(
            name.to_owned(),
            crate::CachedLocalTfm {
                content_id: umber_vfs::FileContentId::for_bytes(bytes),
                bytes: bytes.to_vec(),
                font: tex_fonts::TfmFont::parse(bytes).expect("test local TFM"),
            },
        );
    }

    let completion = setup();
    let before = completion.next_object();
    let input = pdf_finalization_input(&completion, DEFAULT_PDF_PK_RESOLUTION, &resources)
        .expect("nested VF detaches");
    assert_eq!(completion.next_object(), before);
    let intermediate = input
        .fonts
        .values()
        .find(|font| font.artifact_resource.name == "cmsy10")
        .expect("intermediate local instance");
    let leaf = input
        .fonts
        .values()
        .find(|font| font.artifact_resource.name == "cmex10")
        .expect("nested leaf instance");
    assert_eq!(intermediate.artifact_resource.at_size, pt(6));
    assert_eq!(leaf.artifact_resource.at_size, pt(9));
    assert!(intermediate.resource_number < leaf.resource_number);
    assert!(intermediate.object_number < leaf.object_number);
    let expected_leaf = tex_fonts::TfmFont::parse_with_size(CMEX10, FontSizeSpec::At(pt(9)))
        .expect("sized leaf TFM");
    assert_eq!(
        leaf.metrics.widths[usize::from(b'A')],
        expected_leaf
            .metrics()
            .character(b'A')
            .expect("leaf character")
            .width,
    );
    assert_eq!(
        input.virtual_fonts[b"cmsy10".as_slice()].local_tfms[b"cmex10".as_slice()]
            .bytes
            .as_ref(),
        CMEX10,
    );

    let mut bounded = input.clone();
    bounded.limits.max_virtual_font_recursion = 1;
    assert!(matches!(
        tex_out::pdf::finalize_pdf(&bounded),
        Err(tex_out::pdf::PdfBuildError::VirtualFontDepthExceeded(1))
    ));
    let mut bad_hash = input.clone();
    bad_hash
        .virtual_fonts
        .get_mut(b"cmsy10".as_slice())
        .expect("nested VF input")
        .local_tfms
        .get_mut(b"cmex10".as_slice())
        .expect("nested local TFM")
        .content_hash = [0; 32];
    assert!(matches!(
        tex_out::pdf::finalize_pdf(&bad_hash),
        Err(tex_out::pdf::PdfBuildError::InvalidVirtualLocalTfm { .. })
    ));
    let mut bad_bytes = input.clone();
    let local = bad_bytes
        .virtual_fonts
        .get_mut(b"cmsy10".as_slice())
        .expect("nested VF input")
        .local_tfms
        .get_mut(b"cmex10".as_slice())
        .expect("nested local TFM");
    local.bytes = CMR10.into();
    local.design_font = tex_fonts::TfmFont::parse(CMR10).expect("replacement TFM");
    assert!(matches!(
        tex_out::pdf::finalize_pdf(&bad_bytes),
        Err(tex_out::pdf::PdfBuildError::InvalidVirtualLocalTfm { .. })
    ));
    let mut cyclic = input.clone();
    cyclic
        .virtual_fonts
        .get_mut(b"cmsy10".as_slice())
        .expect("nested VF input")
        .program = tex_fonts::VfProgram::parse(&vf(b"cmsy10", FIX_ONE)).expect("cyclic VF");
    assert!(matches!(
        tex_out::pdf::finalize_pdf(&cyclic),
        Err(tex_out::pdf::PdfBuildError::VirtualFontCycle { .. })
    ));

    let pdf = pdf_from_accepted_artifacts_with_virtual_fonts(
        &completion,
        &resources,
        &crate::PdfRawObjectFileReceipt::default(),
    )
    .expect("detached nested VF finalizes");
    assert_eq!(completion.next_object(), input.allocation.next_object);
    let parsed = test_support::pdf_query::PdfQuery::new(
        &pdf,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent PDF parser accepts nested VF output");
    assert_eq!(parsed.pages().expect("nested VF page").len(), 1);
}

#[test]
fn detached_vf_retains_selected_default_resource_but_not_unreached_definitions() {
    const FIX_ONE: i32 = 1 << 20;
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
    const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");

    fn vf() -> Vec<u8> {
        let mut bytes = vec![247, 202, 0];
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
        for (number, name) in [
            (7, b"cmsy10".as_slice()),
            (9, b"cmex10".as_slice()),
            (11, b"unused10".as_slice()),
        ] {
            bytes.extend_from_slice(&[243, number]);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&FIX_ONE.to_be_bytes());
            bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
            bytes.extend_from_slice(&[0, name.len() as u8]);
            bytes.extend_from_slice(name);
        }
        // pdfTeX.web §32e do_vf_packet selects the first definition before
        // executing the packet. The packet immediately switches to font 9,
        // so font 7 is selected and checkpointed without painting a glyph.
        let commands = [235, 9, b'A'];
        bytes.extend_from_slice(&[commands.len() as u8, b'A', 8, 0, 0]);
        bytes.extend_from_slice(&commands);
        bytes.push(248);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(248);
        }
        bytes
    }

    crate::with_engine_universe(|stores| {
        stores.set_interaction_mode(InteractionMode::Nonstop);
        prepare_pdftex_run_stores(stores);
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("seed root TFM");
        stores
            .world_mut()
            .set_memory_file(
                "cmr10.pfb",
                include_bytes!("../../../../tests/corpus/pdf/embedded_type1/cmr10.pfb").to_vec(),
            )
            .expect("seed leaf program");
        let mut host = FileSessionResolvers::new(Path::new("pdf-test.tex"), Vec::new(), Vec::new());
        run_input_collecting_artifacts_with_profile(
            stores,
            RetainedRootRequest::authored_job(
                "pdf-test",
                concat!(
                    "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
                    "\\pdfmapline{=cmsy10 CMR10 <cmr10.pfb}",
                    "\\pdfmapline{=cmex10 CMR10 <cmr10.pfb}",
                    "\\font\\f=cmr10 at 12pt ",
                    "\\shipout\\hbox{\\f A}\\end",
                )
                .as_bytes(),
                tex_command::CommandProfile::PDFTEX14029,
            ),
            &mut host,
            tex_command::CommandProfile::PDFTEX14029,
        )
        .expect("VF root page ships");
        let completion = stores
            .command_context()
            .expect("admit selected-default completion")
            .detach_pdf_completion()
            .expect("detach selected-default PDF");
        let root_vf = vf();
        let mut resources = crate::PdfVirtualFontResources::default();
        resources.virtual_fonts.insert(
            "cmr10".to_owned(),
            crate::CachedVirtualFont {
                content_id: umber_vfs::FileContentId::for_bytes(&root_vf),
                program: tex_fonts::VfProgram::parse(&root_vf).expect("test VF"),
            },
        );
        for (name, bytes) in [("cmsy10", CMSY10), ("cmex10", CMEX10), ("unused10", CMSY10)] {
            resources.local_tfms.insert(
                name.to_owned(),
                crate::CachedLocalTfm {
                    content_id: umber_vfs::FileContentId::for_bytes(bytes),
                    bytes: bytes.to_vec(),
                    font: tex_fonts::TfmFont::parse(bytes).expect("test local TFM"),
                },
            );
        }

        let input = pdf_finalization_input(&completion, DEFAULT_PDF_PK_RESOLUTION, &resources)
            .expect("selected default font is retained at the detached boundary");
        let names = input
            .fonts
            .values()
            .map(|font| font.artifact_resource.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("cmsy10"), "selected default is checkpointed");
        assert!(names.contains("cmex10"), "painted leaf is checkpointed");
        assert!(
            !names.contains("unused10"),
            "an unselected VF definition is not an output resource"
        );

        let pdf = pdf_from_accepted_artifacts_with_virtual_fonts(
            &completion,
            &resources,
            &crate::PdfRawObjectFileReceipt::default(),
        )
        .expect("selected-default VF finalizes");
        test_support::pdf_query::PdfQuery::new(
            &pdf,
            test_support::pdf_query::QueryLimits::default(),
        )
        .expect("independent parser accepts selected-default PDF");
    })
    .expect("fresh selected-default universe");
}

fn pt(value: i32) -> Scaled {
    Scaled::from_raw(value * 65_536)
}

use std::path::Path;

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
fn external_pdf_resource_keeps_one_owner_through_finalization() {
    const IMAGE: &[u8] =
        include_bytes!("../../../../tests/corpus/pdf/minimal_rule/expected.umber.pdf");

    let (completion, acquired) = crate::with_engine_universe(|stores| {
        stores.set_interaction_mode(InteractionMode::Nonstop);
        prepare_pdftex_run_stores(stores);
        let image = IMAGE.to_vec();
        let acquired_address = image.as_ptr();
        stores
            .world_mut()
            .set_memory_file("figure.pdf", image)
            .expect("seed external PDF");
        let acquired = stores
            .world_mut()
            .read_file("figure.pdf")
            .expect("read acquired external PDF")
            .shared_bytes();
        assert_eq!(acquired.as_ptr(), acquired_address);

        let mut host =
            FileSessionResolvers::new(Path::new("image-owner.tex"), Vec::new(), Vec::new());
        run_input_collecting_artifacts_with_profile(
            stores,
            RetainedRootRequest::authored_job(
                "image-owner",
                concat!(
                    "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
                    "\\pdfobj reserveobjnum\\pdfobj reserveobjnum\\pdfobj reserveobjnum",
                    "\\pdfximage attr{/Group <</I false /K false /S /Transparency>>}{figure.pdf}",
                    "\\shipout\\hbox{\\pdfrefximage\\pdflastximage}\\end",
                )
                .as_bytes(),
                tex_command::CommandProfile::PDFTEX14029,
            ),
            &mut host,
            tex_command::CommandProfile::PDFTEX14029,
        )
        .expect("external PDF page ships");
        let completion = stores
            .command_context()
            .expect("admit external-image completion")
            .detach_pdf_completion()
            .expect("detach external-image PDF");
        (completion, acquired)
    })
    .expect("fresh external-image universe");

    let [image] = completion.images() else {
        panic!("expected exactly one external image");
    };
    // pdftex.web §1551 gives the image its own `pdf_ximage_count` resource
    // identity even when earlier allocations advanced the shared object table.
    assert_eq!(image.id().raw(), 4);
    assert_eq!(image.resource(), 1);
    assert_eq!(
        image.attributes(),
        b"/Group <</I false /K false /S /Transparency>>"
    );
    assert!(tex_state::SharedBytes::ptr_eq(
        &image.shared_bytes(),
        &acquired
    ));
    let input = pdf_finalization_input(
        &completion,
        DEFAULT_PDF_PK_RESOLUTION,
        &crate::PdfVirtualFontResources::default(),
    )
    .expect("external-image finalization input");
    let finalized_image = input.images.values().next().expect("final image input");
    assert_eq!(finalized_image.attributes, image.attributes());
    assert!(tex_state::SharedBytes::ptr_eq(
        &finalized_image.bytes,
        &acquired
    ));

    let first = tex_out::pdf::finalize_pdf(&input).expect("first deterministic finalization");
    let second = tex_out::pdf::finalize_pdf(&input).expect("second deterministic finalization");
    assert_eq!(first.bytes, second.bytes);
    let query = test_support::pdf_query::PdfQuery::new(
        &first.bytes,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent parser accepts external-image PDF");
    let pages = query.pages().expect("query external-image page");
    let xobjects = pages[0]
        .resources
        .categories
        .get(b"XObject".as_slice())
        .expect("page has an XObject resource");
    let entries: Vec<_> = xobjects[0].entries().collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, b"Im1");
    assert_eq!(
        entries[0]
            .1
            .referenced_id()
            .expect("image reference")
            .number,
        4
    );
    let form = query
        .dictionary(entries[0].1.referenced_id().expect("image reference"))
        .expect("imported image is a Form XObject");
    let group = form
        .get(b"Group")
        .and_then(|value| value.as_dictionary())
        .expect("image attributes preserve the transparency group");
    let subtype = group
        .get(b"S")
        .and_then(|value| value.name())
        .expect("transparency group has a subtype");
    assert_eq!(subtype.as_ref(), b"Transparency");
    assert_eq!(
        group.get(b"I").and_then(|value| value.boolean()),
        Some(false)
    );
    assert_eq!(
        group.get(b"K").and_then(|value| value.boolean()),
        Some(false)
    );
}

#[test]
fn detached_nested_vf_preserves_exact_local_tfm_identity_and_resources() {
    const FIX_ONE: i32 = 1 << 20;
    const CMR10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    const CMSY10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
    const CMEX10: &[u8] = include_bytes!("../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");
    const CMR10_PFB: &[u8] =
        include_bytes!("../../../../tests/corpus/pdf/embedded_type1/cmr10.pfb");

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
    resources.type1_programs.insert(
        b"cmr10.pfb".to_vec(),
        tex_fonts::PdfType1Program::from_pfb(CMR10_PFB).expect("test Type-1 program"),
    );

    let completion = setup();
    let before = completion.next_object();
    let input = pdf_finalization_input(&completion, DEFAULT_PDF_PK_RESOLUTION, &resources)
        .expect("nested VF detaches");
    assert_eq!(completion.next_object(), before);
    assert_eq!(
        input.virtual_fonts[b"cmsy10".as_slice()].local_tfms[b"cmex10".as_slice()]
            .bytes
            .as_ref(),
        CMEX10,
    );
    let output = tex_out::pdf::finalize_pdf(&input).expect("nested VF materializes atomically");
    test_support::pdf_query::PdfQuery::new(
        &output.bytes,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent parser accepts nested-VF PDF");

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
        .content_hash = [0; 8];
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
    assert_eq!(completion.next_object(), before);
    assert!(
        input.allocation.next_object > before,
        "document and local-font objects are allocated only in the detached destination"
    );
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
    const CMR10_PFB: &[u8] =
        include_bytes!("../../../../tests/corpus/pdf/embedded_type1/cmr10.pfb");

    fn vf() -> Vec<u8> {
        let mut bytes = vec![247, 202, 0];
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
        for (number, name) in [
            (7, b"cmsy10".as_slice()),
            (11, b"unreached10".as_slice()),
            (9, b"cmex10".as_slice()),
            (13, b"late10".as_slice()),
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
        for (character, commands) in [
            (b'A', &[235, 9, b'A'][..]),
            (b'B', &[235, 9, b'B', 235, 13, b'B'][..]),
        ] {
            bytes.extend_from_slice(&[commands.len() as u8, character, 8, 0, 0]);
            bytes.extend_from_slice(commands);
        }
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
                    "\\pdfmapline{=late10 CMR10 <cmr10.pfb}",
                    "\\font\\f=cmr10 at 12pt ",
                    "\\shipout\\hbox{\\f A}",
                    "\\font\\g=cmr10 at 10pt ",
                    "\\shipout\\hbox{\\g B}\\end",
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
        for (name, bytes) in [
            ("cmsy10", CMSY10),
            ("cmex10", CMEX10),
            ("unreached10", CMSY10),
            ("late10", CMSY10),
        ] {
            resources.local_tfms.insert(
                name.to_owned(),
                crate::CachedLocalTfm {
                    content_id: umber_vfs::FileContentId::for_bytes(bytes),
                    bytes: bytes.to_vec(),
                    font: tex_fonts::TfmFont::parse(bytes).expect("test local TFM"),
                },
            );
        }
        resources.type1_programs.insert(
            b"cmr10.pfb".to_vec(),
            tex_fonts::PdfType1Program::from_pfb(CMR10_PFB).expect("test Type-1 program"),
        );

        let input = pdf_finalization_input(&completion, DEFAULT_PDF_PK_RESOLUTION, &resources)
            .expect("selected default font is retained at the detached boundary");
        assert!(
            input.virtual_fonts[b"cmr10".as_slice()]
                .local_tfms
                .contains_key(b"cmsy10".as_slice())
        );
        assert!(
            input.virtual_fonts[b"cmr10".as_slice()]
                .local_tfms
                .contains_key(b"cmex10".as_slice())
        );
        let materialized_names = input
            .fonts
            .values()
            .map(|font| font.artifact_resource.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            materialized_names.contains("cmsy10"),
            "the packet's initially selected local font has a destination identity"
        );
        assert!(
            materialized_names.contains("cmex10"),
            "the packet's explicitly selected local font has a destination identity"
        );
        assert!(
            materialized_names.contains("late10"),
            "the later page's explicitly selected local font has a destination identity"
        );
        assert!(
            !materialized_names.contains("unreached10"),
            "an unselected VF definition has no destination identity"
        );
        let first_local_resource = input.pages[0].font_watermark + 1;
        let cmsy_resource = input
            .fonts
            .values()
            .find(|font| font.artifact_resource.name == "cmsy10")
            .expect("the default local font was materialized")
            .resource_number;
        let cmex_resource = input
            .fonts
            .values()
            .find(|font| font.artifact_resource.name == "cmex10")
            .expect("the explicitly selected local font was materialized")
            .resource_number;
        let late_resource = input
            .fonts
            .values()
            .find(|font| font.artifact_resource.name == "late10")
            .expect("the later page's selected local font was materialized")
            .resource_number;
        assert_eq!(
            [cmsy_resource, cmex_resource, late_resource],
            [
                first_local_resource,
                first_local_resource + 2,
                first_local_resource + 8,
            ],
            "pdftex.web §32e interleaves unselected VF definitions and later engine fonts in one internal-font timeline"
        );
        let cmex_instances = input
            .fonts
            .values()
            .filter(|font| font.artifact_resource.name == "cmex10")
            .collect::<Vec<_>>();
        assert_eq!(cmex_instances.len(), 2, "both selected sizes stay realized");
        assert!(
            cmex_instances
                .windows(2)
                .all(|pair| pair[0].resource_number == pair[1].resource_number
                    && pair[0].object_number == pair[1].object_number),
            "pdftex.web §32e shares one mapped VF-leaf resource across sizes"
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

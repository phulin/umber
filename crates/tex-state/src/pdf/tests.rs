use super::*;
use crate::interner::InternerBudget;
use crate::token::{Token, TokenWord};
use crate::universe::with_universe;

fn budget() -> InternerBudget {
    InternerBudget::new(64, 128, 4 * 1024).expect("test fixture is valid")
}

fn semantic_id(tag: u64) -> StateHashFragment {
    StateHasher::new(tag).finish_fragment()
}

fn output() -> PdfOutputParameters {
    PdfOutputParameters {
        output: 1,
        major_version: 1,
        minor_version: 7,
        compress_level: 9,
        object_compress_level: 2,
        decimal_digits: 4,
        gamma: 1_000,
        image_gamma: 1_000,
        image_hicolor: 1,
        image_apply_gamma: 1,
        draft_mode: 0,
        inclusion_copy_fonts: 1,
        pk_resolution: 600,
        unique_resource_names: 1,
    }
}

#[test]
fn aliased_pdf_fonts_enumerate_one_terminal_resource_object() {
    let mut state = PdfState::<()>::default();
    let identity = tex_fonts::PdfFontResourceIdentity::new([7; 32], None);
    let first = state
        .ensure_font_resource(
            crate::ids::FontId::testing_new(1),
            tex_fonts::FontSourceIdentity::from_bytes([1; 32]),
            identity,
        )
        .expect("first PDF font resource");
    let alias = state
        .ensure_font_resource(
            crate::ids::FontId::testing_new(2),
            tex_fonts::FontSourceIdentity::from_bytes([2; 32]),
            identity,
        )
        .expect("aliased PDF font resource");

    assert_eq!(alias.object_number(), first.object_number());
    assert_eq!(alias.resource_number(), first.resource_number());
    assert_eq!(state.font_resources().collect::<Vec<_>>(), vec![first]);
    assert_eq!(
        state.font_resource_records().collect::<Vec<_>>(),
        vec![first, alias],
        "terminal identity enumeration retains every artifact-facing alias"
    );
}

#[test]
fn terminal_pdf_completion_retains_every_scaled_font_alias_recipe() {
    let mut state = PdfState::<()>::default();
    let identity = tex_fonts::PdfFontResourceIdentity::new([7; 32], None);
    let base = crate::ids::FontId::testing_new(1);
    let scaled = crate::ids::FontId::testing_new(2);
    let base_identity = tex_fonts::FontSourceIdentity::from_bytes([1; 32]);
    let scaled_identity = tex_fonts::FontSourceIdentity::from_bytes([2; 32]);
    let first = state
        .ensure_font_resource(base, base_identity, identity)
        .expect("base PDF font resource");
    let alias = state
        .ensure_font_resource(scaled, scaled_identity, identity)
        .expect("scaled PDF font alias");

    let completion = completion::detach(
        &state,
        completion::PdfCompletionScalars {
            font_configuration: PdfFontConfiguration {
                adjust_spacing: 0,
                protrude_chars: 0,
                tracing_fonts: 0,
                adjust_interword_glue: 0,
                prepend_kern: 0,
                append_kern: 0,
                generate_to_unicode: 0,
                pk_resolution: 600,
                omit_charset: 0,
            },
            pages_entries: Vec::new(),
            include_info_dictionary: true,
            include_dates: true,
            suppress_ptex_info: 0,
            ptex_use_underscore: false,
            form_omit_procset: 0,
            suppress_page_group_warning: false,
            clock: crate::JobClock::DEFAULT,
        },
        |_| Ok(Vec::new()),
        |font| {
            let (name, at_size, semantic_identity) = if font == base {
                ("cmr10", 10, base_identity)
            } else {
                ("cmr10-scaled", 9, scaled_identity)
            };
            crate::FontArtifactRecipe {
                name: name.to_owned(),
                tfm_content_hash: [7; 32],
                tfm_checksum: 0,
                design_size: Scaled::from_raw(10 * Scaled::UNITY),
                at_size: Scaled::from_raw(at_size * Scaled::UNITY),
                layout_policy: tex_fonts::FontLayoutPolicy::ClassicTfmExact,
                mapping_fallback: None,
                opentype: None,
                semantic_identity,
                construction: crate::FontArtifactConstructionRecipe::Loaded,
            }
        },
        |_, _| None,
        |_, _| Scaled::from_raw(0),
        |_| Ok(None),
    )
    .expect("alias-only PDF ledger detaches");

    assert_eq!(completion.fonts().len(), 2);
    assert_eq!(
        completion
            .fonts()
            .iter()
            .map(|font| (
                font.recipe.semantic_identity,
                font.resource_number,
                font.object_number,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                base_identity,
                first.resource_number(),
                first.object_number()
            ),
            (
                scaled_identity,
                alias.resource_number(),
                alias.object_number(),
            ),
        ]
    );
}

#[test]
fn page_coordinates_are_generation_typed_and_checkpointed() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::frozen_relax())])
            .expect("test fixture is valid");
        let token = PdfTokenParameter {
            tokens: id.clone(),
            semantic_id: semantic_id(1),
        };
        let page = PdfPageParameters {
            h_origin: Scaled::from_raw(1),
            v_origin: Scaled::from_raw(2),
            width: Scaled::from_raw(3),
            height: Scaled::from_raw(4),
            link_margin: Scaled::from_raw(5),
            page_attr: token.clone(),
            resources: token.clone(),
            omit_procset: 0,
            space_font_name: 0,
        };
        let mut state = PdfState::default();
        state.enable();
        let checkpoint = state.snapshot();
        let checkpoint_copy = checkpoint.clone();
        state.commit_page(ContentHash::new([7; 32]), output(), page, token);
        assert_eq!(state.pages()[0].page_attr(), id);
        assert_eq!(state.pages()[0].resources(), id);
        state.rollback(checkpoint_copy);
        assert!(state.pages().is_empty());
    })
    .expect("test fixture is valid");
}

#[test]
fn action_annotation_outline_and_raw_object_copy_without_brand_traits() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::param(1))])
            .expect("test fixture is valid");
        let parameter = PdfTokenParameter {
            tokens: id.clone(),
            semantic_id: semantic_id(2),
        };
        let action = PdfActionSpec::User(id.clone());
        let mut state = PdfState::default();
        let annotation = state.reserve_annotation().expect("test fixture is valid");
        state
            .initialize_annotation(
                annotation.object(),
                PdfAnnotationData {
                    dimensions: PdfAnnotationDimensions::RUNNING,
                    entries: id.clone(),
                },
                semantic_id(3),
            )
            .expect("test fixture is valid");
        state
            .create_outline(id.clone(), action, 0, id.clone(), [semantic_id(4); 3])
            .expect("test fixture is valid");
        let raw = state.reserve_raw_object().expect("test fixture is valid");
        state
            .initialize_raw_object(
                raw,
                PdfRawObjectData::new(false, None, false, parameter),
                true,
            )
            .expect("test fixture is valid");
        assert_eq!(
            state.annotations()[0]
                .data()
                .expect("test fixture is valid")
                .entries,
            id
        );
        assert_eq!(state.outlines()[0].title(), id);
        assert_eq!(
            state
                .raw_object(raw)
                .expect("test fixture is valid")
                .data()
                .expect("test fixture is valid")
                .data(),
            id
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn snapshots_own_pdf_collections_and_rollback_them_atomically() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::frozen_relax())])
            .expect("test fixture is valid");
        let mut state = PdfState::default();
        let checkpoint = state.snapshot();
        state.append_document_fragment(
            PdfDocumentFragmentKind::Catalog,
            PdfTokenParameter {
                tokens: id.clone(),
                semantic_id: semantic_id(5),
            },
        );
        state
            .create_link(
                PdfAnnotationDimensions::RUNNING,
                id.clone(),
                PdfActionSpec::User(id.clone()),
                semantic_id(6),
                semantic_id(7),
                0,
            )
            .expect("test fixture is valid");
        assert_eq!(
            state
                .document_fragments(PdfDocumentFragmentKind::Catalog)
                .count(),
            1
        );
        assert_eq!(state.open_links().len(), 1);
        state.rollback(checkpoint);
        assert_eq!(
            state
                .document_fragments(PdfDocumentFragmentKind::Catalog)
                .count(),
            0
        );
        assert!(state.links().is_empty());
        assert!(state.open_links().is_empty());
    })
    .expect("test fixture is valid");
}

#[test]
fn external_image_payload_is_owned_not_shared() {
    let mut state = PdfState::<()>::default();
    let record = state
        .allocate_external_image(
            PdfExternalImageSource {
                identity: ContentHash::new([9; 32]),
                metadata: PdfExternalImageMetadata::Raster(PdfRasterImageMetadata {
                    format: PdfRasterFormat::Png,
                    width: 1,
                    height: 1,
                    bits_per_component: 8,
                    color_space: PdfRasterColorSpace::Gray,
                    alpha: false,
                    png_color_type: Some(0),
                }),
                natural_width: Scaled::from_raw(1),
                natural_height: Scaled::from_raw(1),
                bytes: vec![1, 2, 3],
            },
            PdfExternalImageDimensions {
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(1),
                depth: Scaled::from_raw(0),
            },
            0,
        )
        .expect("test fixture is valid");
    assert_eq!(record.bytes(), &[1, 2, 3]);
}

#[test]
fn format_pdf_ledger_detaches_and_materializes_before_publication() {
    with_universe(budget(), |universe| {
        let tokens = universe
            .allocate_token_list(&[TokenWord::pack(Token::param(4))])
            .expect("test fixture is valid");
        let parameter = PdfTokenParameter {
            tokens: tokens.clone(),
            semantic_id: semantic_id(8),
        };
        let mut source = PdfState::default();
        source.enable();
        let object = source.reserve_raw_object().expect("test fixture is valid");
        source
            .initialize_raw_object(
                object,
                PdfRawObjectData::new(false, None, false, parameter.clone()),
                true,
            )
            .expect("test fixture is valid");

        let bytes = source
            .capture_format_bytes(
                |id| {
                    assert_eq!(id, tokens);
                    Ok(vec![4])
                },
                |_| Err("unexpected node recipe".to_owned()),
            )
            .expect("test fixture is valid")
            .expect("format-compatible PDF state detaches");
        let restored = PdfState::restore_format_bytes(
            &bytes,
            crate::EngineCapacityProfile::Pdftex14029
                .configuration()
                .pdf,
            |recipe| {
                assert_eq!(recipe, [4]);
                Ok(parameter.clone())
            },
            |_| Err("unexpected node recipe".to_owned()),
        )
        .expect("detached PDF state materializes");
        assert!(restored.enabled());
        assert_eq!(
            restored
                .raw_object(object)
                .expect("test fixture is valid")
                .data()
                .expect("test fixture is valid")
                .data(),
            tokens
        );

        assert!(
            PdfState::<()>::restore_format_bytes(
                b"not a format",
                crate::EngineCapacityProfile::Pdftex14029
                    .configuration()
                    .pdf,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .is_err()
        );
    })
    .expect("test fixture is valid");
}

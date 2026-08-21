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
fn page_coordinates_are_generation_typed_and_checkpointed() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::frozen_relax())])
            .expect("test fixture is valid");
        let token = PdfTokenParameter {
            tokens: id,
            semantic_id: semantic_id(1),
        };
        let page = PdfPageParameters {
            h_origin: Scaled::from_raw(1),
            v_origin: Scaled::from_raw(2),
            width: Scaled::from_raw(3),
            height: Scaled::from_raw(4),
            link_margin: Scaled::from_raw(5),
            page_attr: token,
            resources: token,
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
            tokens: id,
            semantic_id: semantic_id(2),
        };
        let action = PdfActionSpec::User(id);
        let mut state = PdfState::default();
        let annotation = state.reserve_annotation().expect("test fixture is valid");
        state
            .initialize_annotation(
                annotation.object(),
                PdfAnnotationData {
                    dimensions: PdfAnnotationDimensions::RUNNING,
                    entries: id,
                },
                semantic_id(3),
            )
            .expect("test fixture is valid");
        state
            .create_outline(id, action, 0, id, [semantic_id(4); 3])
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
                tokens: id,
                semantic_id: semantic_id(5),
            },
        );
        state
            .create_link(
                PdfAnnotationDimensions::RUNNING,
                id,
                PdfActionSpec::User(id),
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
            tokens,
            semantic_id: semantic_id(8),
        };
        let mut source = PdfState::default();
        source.enable();
        let object = source.reserve_raw_object().expect("test fixture is valid");
        source
            .initialize_raw_object(
                object,
                PdfRawObjectData::new(false, None, false, parameter),
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
            |recipe| {
                assert_eq!(recipe, [4]);
                Ok(parameter)
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
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .is_err()
        );
    })
    .expect("test fixture is valid");
}

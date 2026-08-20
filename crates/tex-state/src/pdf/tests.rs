use super::*;
use crate::interner::InternerBudget;
use crate::token::{Token, TokenWord};
use crate::universe::with_universe;

fn budget() -> InternerBudget {
    InternerBudget::new(64, 128, 4 * 1024).unwrap()
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
            .unwrap();
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
    .unwrap();
}

#[test]
fn action_annotation_outline_and_raw_object_copy_without_brand_traits() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::param(1))])
            .unwrap();
        let parameter = PdfTokenParameter {
            tokens: id,
            semantic_id: semantic_id(2),
        };
        let action = PdfActionSpec::User(id);
        let mut state = PdfState::default();
        let annotation = state.reserve_annotation().unwrap();
        state
            .initialize_annotation(
                annotation.object(),
                PdfAnnotationData {
                    dimensions: PdfAnnotationDimensions::RUNNING,
                    entries: id,
                },
                semantic_id(3),
            )
            .unwrap();
        state
            .create_outline(id, action, 0, id, [semantic_id(4); 3])
            .unwrap();
        let raw = state.reserve_raw_object().unwrap();
        state
            .initialize_raw_object(
                raw,
                PdfRawObjectData::new(false, None, false, parameter),
                true,
            )
            .unwrap();
        assert_eq!(state.annotations()[0].data().unwrap().entries, id);
        assert_eq!(state.outlines()[0].title(), id);
        assert_eq!(state.raw_object(raw).unwrap().data().unwrap().data(), id);
    })
    .unwrap();
}

#[test]
fn snapshots_own_pdf_collections_and_rollback_them_atomically() {
    with_universe(budget(), |universe| {
        let id = universe
            .allocate_token_list(&[TokenWord::pack(Token::frozen_relax())])
            .unwrap();
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
            .unwrap();
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
    .unwrap();
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
        .unwrap();
    assert_eq!(record.bytes(), &[1, 2, 3]);
}

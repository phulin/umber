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
    let identity = tex_fonts::PdfFontResourceIdentity::new([7; 8], None);
    let first = state
        .ensure_font_resource(
            crate::ids::FontId::testing_new(1),
            tex_fonts::FontSourceIdentity::from_bytes([1; 8]),
            identity,
        )
        .expect("first PDF font resource");
    let alias = state
        .ensure_font_resource(
            crate::ids::FontId::testing_new(2),
            tex_fonts::FontSourceIdentity::from_bytes([2; 8]),
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
    let identity = tex_fonts::PdfFontResourceIdentity::new([7; 8], None);
    let base = crate::ids::FontId::testing_new(1);
    let scaled = crate::ids::FontId::testing_new(2);
    let base_identity = tex_fonts::FontSourceIdentity::from_bytes([1; 8]);
    let scaled_identity = tex_fonts::FontSourceIdentity::from_bytes([2; 8]);
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
                tfm_content_hash: [7; 8],
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
fn checkpoint_fork_and_restore_do_not_copy_image_or_form_payload_bytes() {
    let mut state = PdfState::<()>::default();
    state
        .allocate_external_image(
            PdfExternalImageSource {
                identity: ContentHash::new([3; 32]),
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
                bytes: vec![3; 1024 * 1024],
            },
            PdfExternalImageDimensions {
                width: Scaled::from_raw(1),
                height: Scaled::from_raw(1),
                depth: Scaled::from_raw(0),
            },
            0,
        )
        .expect("image fixture fits");
    state.set_form_artifact(
        41,
        PdfFormArtifact::new(
            vec![5; 1024 * 1024],
            Some((Scaled::from_raw(2), Scaled::from_raw(3))),
            (Scaled::from_raw(4), Scaled::from_raw(5)),
        ),
    );
    let checkpoint = state.snapshot();
    let image_id = state.external_images[0].payload;
    let form_id = state
        .form_artifact_payload(41)
        .expect("form payload exists");
    let image_address = state.payloads.get(image_id).as_ptr();
    let form_address = state.payloads.get(form_id).as_ptr();

    state.open_candidate_lineage(&checkpoint);
    assert_eq!(state.payloads.get(image_id).as_ptr(), image_address);
    assert_eq!(state.payloads.get(form_id).as_ptr(), form_address);
    state.rollback(checkpoint);
    assert_eq!(state.payloads.get(image_id).as_ptr(), image_address);
    assert_eq!(state.payloads.get(form_id).as_ptr(), form_address);
    state.reject_candidate_transaction();
}

#[test]
fn checkpoint_fork_reuses_append_only_metadata_prefix_allocations() {
    fn row_address<T>(row: Option<&T>) -> usize {
        row.expect("append-only fixture row exists") as *const T as usize
    }

    let mut state = PdfState::<()>::default();
    state.font_resources.push(PdfFontResourceRecord {
        font: FontId::testing_new(7),
        source_identity: tex_fonts::FontSourceIdentity::from_bytes([7; 8]),
        resource_number: 7,
        object_number: 11,
        identity: tex_fonts::PdfFontResourceIdentity::new([8; 8], None),
    });
    let payload = state.payloads.store(vec![1]);
    state.external_images.push(PdfExternalImageEntry {
        id: PdfExternalImageId(12),
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
        dimensions: PdfExternalImageDimensions {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(1),
            depth: Scaled::from_raw(0),
        },
        color_space_object: 0,
        payload,
        mask_object: None,
    });
    state.page_reservations.push(PdfPageReservation {
        number: 3,
        object: 13,
    });
    let checkpoint = state.snapshot();
    let addresses = [
        row_address(state.font_resources.get(0)),
        row_address(state.external_images.get(0)),
        row_address(state.page_reservations.get(0)),
    ];

    state.open_candidate_lineage(&checkpoint);
    assert_eq!(addresses[0], row_address(state.font_resources.get(0)));
    assert_eq!(addresses[1], row_address(state.external_images.get(0)));
    assert_eq!(addresses[2], row_address(state.page_reservations.get(0)));

    state.page_reservations.push(PdfPageReservation {
        number: 4,
        object: 14,
    });
    assert_eq!(state.page_reservations.len(), 2);
    state.reject_candidate_transaction();
    assert_eq!(state.page_reservations.len(), 1);
}

#[test]
fn candidate_acceptance_replaces_only_the_prior_pdf_suffix() {
    let mut state = PdfState::<()>::default();
    state.page_reservations.push(PdfPageReservation {
        number: 1,
        object: 11,
    });
    state.set_match(vec![1], vec![Some((0, 1))], 1, true);
    let base = state.snapshot();

    state.page_reservations.push(PdfPageReservation {
        number: 2,
        object: 12,
    });
    state.set_match(vec![2], vec![Some((0, 1))], 1, true);
    state.open_candidate_lineage(&base);
    state.page_reservations.push(PdfPageReservation {
        number: 3,
        object: 13,
    });
    state.set_match(vec![3], vec![Some((0, 1))], 1, true);
    state.accept_candidate_transaction();

    assert_eq!(
        state
            .page_reservations
            .iter()
            .map(|row| row.number)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(state.match_capture(0), Some((0, &[3][..])));
    assert_eq!(state.undo_base, base.undo_pos);
    assert_eq!(state.history_head().0, base.undo_pos + 1);
}

#[test]
fn rollback_exactly_replays_overwrite_delete_and_pop_then_push_mutations() {
    with_universe(budget(), |universe| {
        let tokens = universe
            .allocate_token_list(&[TokenWord::pack(Token::param(2))])
            .expect("token fixture");
        let parameter = PdfTokenParameter {
            tokens: tokens.clone(),
            semantic_id: semantic_id(20),
        };
        let mut state = PdfState::default();

        state.set_match(vec![1, 2, 3], vec![Some((0, 2))], 1, true);
        let raw = state.reserve_raw_object().expect("raw reservation");
        state
            .initialize_raw_object(
                raw,
                PdfRawObjectData::new(false, None, false, parameter),
                false,
            )
            .expect("raw initialization");
        let annotation = state.reserve_annotation().expect("annotation reservation");
        let destination = PdfDestinationIdentity::Name(b"before".to_vec());
        state
            .reserve_destination(destination.clone(), false)
            .expect("destination reservation");
        state
            .append_thread_bead(PdfDestinationIdentity::Number(7))
            .expect("first thread bead");
        state
            .create_link(
                PdfAnnotationDimensions::RUNNING,
                tokens.clone(),
                PdfActionSpec::User(tokens.clone()),
                semantic_id(21),
                semantic_id(22),
                3,
            )
            .expect("first open link");
        let color = state
            .allocate_color_stack(PdfColorStackMode::Direct, true, b"initial".to_vec())
            .expect("color stack");
        state
            .apply_color_stack(
                color,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Push(b"saved".to_vec()),
            )
            .expect("color push");
        state.set_form_artifact(
            51,
            PdfFormArtifact::new(
                vec![8; 4096],
                None,
                (Scaled::from_raw(1), Scaled::from_raw(2)),
            ),
        );
        let old_form_payload = state
            .form_artifact_payload(51)
            .expect("old form payload exists");
        let old_form_address = state.payloads.get(old_form_payload).as_ptr();
        let checkpoint = state.snapshot();

        state.set_match(vec![9, 8, 7], vec![Some((1, 3))], 1, true);
        state.reference_raw_object(raw).expect("raw reference");
        state
            .initialize_annotation(
                annotation.object(),
                PdfAnnotationData {
                    dimensions: PdfAnnotationDimensions::RUNNING,
                    entries: tokens.clone(),
                },
                semantic_id(23),
            )
            .expect("annotation initialization");
        state
            .define_destination(destination.clone(), None)
            .expect("destination definition");
        let original_open = state.end_link().expect("original link is open");
        let replacement_open = state
            .create_link(
                PdfAnnotationDimensions::RUNNING,
                tokens.clone(),
                PdfActionSpec::User(tokens),
                semantic_id(24),
                semantic_id(25),
                4,
            )
            .expect("replacement open link");
        state
            .apply_color_stack(color, PdfColorStackTarget::Page, &PdfColorStackAction::Pop)
            .expect("color pop");
        state
            .apply_color_stack(
                color,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Push(b"replacement".to_vec()),
            )
            .expect("color replacement push");
        state.set_form_artifact(
            51,
            PdfFormArtifact::new(
                vec![9; 8192],
                Some((Scaled::from_raw(3), Scaled::from_raw(4))),
                (Scaled::from_raw(5), Scaled::from_raw(6)),
            ),
        );
        state
            .append_thread_bead(PdfDestinationIdentity::Number(7))
            .expect("second thread bead");
        let accepted_form_payload = state
            .form_artifact_payload(51)
            .expect("accepted form payload exists");
        let accepted_form_address = state.payloads.get(accepted_form_payload).as_ptr();

        state.open_candidate_lineage(&checkpoint);
        assert_eq!(state.match_capture(0), Some((0, &[1, 2][..])));
        assert!(!state.raw_object(raw).expect("raw row").is_referenced());
        assert!(state.annotations()[0].data().is_none());
        assert!(
            !state
                .destination(&destination, false)
                .expect("destination row")
                .defined()
        );
        assert_eq!(
            state.open_links()[0].record.object(),
            original_open.record.object()
        );
        assert_eq!(state.thread_record(0).beads().len(), 1);
        assert_eq!(
            state.payloads.get(old_form_payload).as_ptr(),
            old_form_address
        );
        assert_eq!(
            state.form_artifact(51).expect("restored form").bytes(),
            &[8; 4096]
        );
        let current = state
            .apply_color_stack(
                color,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Current,
            )
            .expect("restored current color");
        assert_eq!(current.payload, b"saved");

        state.reject_candidate_transaction();
        assert_eq!(state.match_capture(0), Some((1, &[8, 7][..])));
        assert!(
            state
                .raw_object(raw)
                .expect("accepted raw row")
                .is_referenced()
        );
        assert!(state.annotations()[0].data().is_some());
        assert!(
            state
                .destination(&destination, false)
                .expect("accepted destination row")
                .defined()
        );
        assert_eq!(
            state.open_links()[0].record.object(),
            replacement_open.object()
        );
        assert_eq!(state.thread_record(0).beads().len(), 2);
        assert_eq!(
            state.payloads.get(accepted_form_payload).as_ptr(),
            accepted_form_address
        );
        assert_eq!(
            state.form_artifact(51).expect("accepted form").bytes(),
            &[9; 8192]
        );
        let current = state
            .apply_color_stack(
                color,
                PdfColorStackTarget::Page,
                &PdfColorStackAction::Current,
            )
            .expect("accepted current color");
        assert_eq!(current.payload, b"replacement");
    })
    .expect("test universe");
}

#[test]
fn pdf_history_pruning_keeps_exactly_the_two_live_rollback_positions() {
    let mut state = PdfState::<()>::default();
    let retired = state.snapshot();
    for value in 0..8 {
        state.set_match(vec![value], vec![Some((0, 1))], 1, true);
    }
    let prior = state.snapshot();
    for value in 8..16 {
        state.set_match(vec![value], vec![Some((0, 1))], 1, true);
    }
    let current = state.snapshot();

    state.prune_history(prior.history_position());
    assert!(!state.snapshot_is_retained(&retired));
    assert!(state.snapshot_is_retained(&prior));
    assert!(state.snapshot_is_retained(&current));
    assert_eq!(state.undo_base, prior.undo_pos);
    state.rollback(current);
    assert_eq!(state.match_capture(0), Some((0, &[15][..])));
    state.rollback(prior);
    assert_eq!(state.match_capture(0), Some((0, &[7][..])));
}

#[cfg(feature = "profiling")]
#[test]
fn pdf_checkpoint_capture_allocates_nothing_independent_of_payload_size() {
    use crate::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    fn measured_capture(payload_len: usize) -> (u64, u64) {
        let mut state = PdfState::<()>::default();
        state
            .allocate_external_image(
                PdfExternalImageSource {
                    identity: ContentHash::new([9; 8]),
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
                    bytes: vec![7; payload_len],
                },
                PdfExternalImageDimensions {
                    width: Scaled::from_raw(1),
                    height: Scaled::from_raw(1),
                    depth: Scaled::from_raw(0),
                },
                0,
            )
            .expect("image fixture fits the object ledger");
        let owner = HotCoreAllocationOwner::GenerationBoundary;
        let before = hot_core_thread_allocation_measurement(owner);
        {
            let _scope = hot_core_allocation_scope(owner);
            for _ in 0..100_000 {
                std::hint::black_box(state.snapshot());
            }
        }
        let after = hot_core_thread_allocation_measurement(owner);
        (
            after.calls.saturating_sub(before.calls),
            after.requested_bytes.saturating_sub(before.requested_bytes),
        )
    }

    assert_eq!(measured_capture(1), (0, 0));
    assert_eq!(measured_capture(16 * 1024 * 1024), (0, 0));
}

#[cfg(feature = "profiling")]
#[test]
fn pdf_candidate_begin_and_reject_allocate_nothing_with_exact_redo() {
    use crate::measurement::{
        HotCoreAllocationOwner, hot_core_allocation_scope, hot_core_thread_allocation_measurement,
    };

    let mut state = PdfState::<()>::default();
    state
        .page_reservations
        .extend((0..10_000).map(|row| PdfPageReservation {
            number: row,
            object: row + 1,
        }));
    let raw = state.reserve_raw_object().expect("raw reservation");
    let destination = PdfDestinationIdentity::Number(7);
    state
        .reserve_destination(destination.clone(), false)
        .expect("destination reservation");
    state
        .append_thread_bead(PdfDestinationIdentity::Number(9))
        .expect("base thread bead");
    let color = state
        .allocate_color_stack(PdfColorStackMode::Direct, true, b"base".to_vec())
        .expect("color stack");
    let base = state.snapshot();

    state.set_match(vec![3; 1024], vec![Some((0, 1))], 1, true);
    state.reference_raw_object(raw).expect("raw reference");
    state
        .define_destination(destination, None)
        .expect("destination definition");
    state
        .append_thread_bead(PdfDestinationIdentity::Number(9))
        .expect("accepted thread bead");
    state.set_form_artifact(
        500,
        PdfFormArtifact::new(
            vec![5; 4096],
            None,
            (Scaled::from_raw(0), Scaled::from_raw(0)),
        ),
    );
    state
        .apply_color_stack(
            color,
            PdfColorStackTarget::Page,
            &PdfColorStackAction::Push(b"accepted".to_vec()),
        )
        .expect("accepted color push");

    let owner = HotCoreAllocationOwner::GenerationBoundary;
    let before = hot_core_thread_allocation_measurement(owner);
    {
        let _scope = hot_core_allocation_scope(owner);
        state.open_candidate_lineage(&base);
        state.reject_candidate_transaction();
    }
    let after = hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
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
            crate::EngineCapacityProfile::Texlive2026
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
                crate::EngineCapacityProfile::Texlive2026
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

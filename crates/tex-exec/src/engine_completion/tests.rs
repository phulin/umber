use std::sync::Arc;

use tex_command::{RegisteredSourceKind, SourceRegistration};
use tex_state::{EffectRecord, World};

use super::*;
use crate::{MainControl, MainControlStep, OutputLedger};

fn capture(source: &[u8], demand: EngineCompletionDemand) -> DetachedEngineCompletion {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        universe
            .begin_retained_session()
            .expect("test execution retains host effects");
        let mut control = if demand.pdf() {
            universe.enable_pdf_output();
            tex_command::install_etex_expandable_primitives(universe);
            tex_command::install_pdftex_expandable_primitives(universe);
            crate::install_etex_unexpandable_primitives(universe);
            tex_command::install_pdftex_unexpandable_primitives(universe);
            let mut control =
                MainControl::prepared_initex(tex_command::CommandProfile::PDFTEX14029);
            control.set_engine_binary(crate::EngineBinaryIdentity::Pdftex14029);
            control
        } else {
            MainControl::tex82_initex(universe)
        };
        control.begin_job(universe, "completion.tex");
        let source = control
            .command_mut()
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("source registers");
        control
            .command_mut()
            .open_registered_source(source)
            .expect("source opens");
        let mut ledger = OutputLedger::default();
        let mut checkpoints = Vec::new();
        let cancellation = crate::Cancellation::new();
        let terminal = loop {
            match crate::CanonicalStepRunner::new(&mut control, universe, &mut ledger)
                .step(&mut checkpoints, &cancellation)
            {
                crate::CanonicalStepResult::Completed(step) => {
                    break ledger
                        .terminal_receipt(&control, step)
                        .expect("canonical terminal step arms its receipt");
                }
                crate::CanonicalStepResult::Progress(_)
                | crate::CanonicalStepResult::Committed(_) => {}
                other => panic!("unexpected completion step: {other:?}"),
            }
        };
        ledger
            .close_revision(&mut control, universe, &terminal, demand)
            .expect("terminal completion detaches")
    })
}

#[test]
fn partial_execution_cannot_detach_or_latch_terminal_state() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        universe
            .begin_retained_session()
            .expect("test execution retains host effects");
        let mut control = MainControl::tex82_initex(universe);
        control.begin_job(universe, "partial-completion.tex");
        let source = control
            .command_mut()
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(br"\relax\end".as_slice()),
            ))
            .expect("source registers");
        control
            .command_mut()
            .open_registered_source(source)
            .expect("source opens");
        let ledger = OutputLedger::default();
        let effects_before = universe.world().effect_records().to_vec();
        let artifacts_before = universe.world().committed_artifacts().to_vec();

        assert!(matches!(
            ledger.terminal_receipt(&control, MainControlStep::End),
            Err(EngineCompletionError::TerminalRevisionUnavailable)
        ));
        assert_eq!(universe.world().effect_records(), effects_before);
        assert_eq!(universe.world().committed_artifacts(), artifacts_before);
        assert_eq!(
            control
                .step(universe)
                .expect("rejection leaves execution live"),
            MainControlStep::Continue
        );
    });
}

#[test]
fn completion_aligns_effect_artifact_dvi_and_pdf_rows() {
    let completion = capture(
        br"\pdfoutput=1\shipout\hbox{\openout3=completion-aligned.tex\relax}\end",
        EngineCompletionDemand::with_pdf(),
    );
    assert_eq!(completion.pages().len(), 1);
    assert!(completion.pages()[0].dvi().is_none());
    let pdf = completion.pdf().expect("PDF was explicitly demanded");
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(
        pdf.pages()[0].artifact,
        completion.pages()[0].artifact().hash()
    );
    assert_eq!(
        pdf.pages()[0].artifact_bytes,
        completion.pages()[0].artifact().bytes()
    );
    assert!(completion.effects().iter().any(|effect| matches!(
        effect,
        EffectRecord::StreamOpen { slot, .. } if slot.raw() == 3
    )));

    let mut destination = World::memory();
    let publication = completion
        .into_publication()
        .expect("preflight accepts aligned rows")
        .publish(&mut destination)
        .expect("memory publication succeeds");
    let CompletionPublication::Committed(committed) = publication else {
        panic!("memory output cannot suspend")
    };
    assert_eq!(committed.pages().len(), 1);
    assert_eq!(destination.committed_artifacts().len(), 1);
    assert!(
        destination
            .memory_output("completion-aligned.tex")
            .is_some()
    );
}

#[test]
fn stream_open_retry_keeps_exact_suffix_and_retargets_page_and_pdf_once() {
    let temporary = tempfile::tempdir().expect("temporary publication root");
    let failed = temporary.path().join("missing").join("old.tex");
    let replacement = temporary.path().join("retry.tex");
    let later = temporary.path().join("later.tex");
    let source = format!(
        "\\pdfoutput=1\\shipout\\hbox{{\\openout3={}\\relax\\openout4={}\\relax}}\\end",
        failed.display(),
        later.display(),
    );
    let completion = capture(source.as_bytes(), EngineCompletionDemand::with_pdf());
    let original_hash = completion.pages()[0].artifact().hash();
    let mut destination = World::real_with_artifact_dir(temporary.path().join("artifacts"));
    let publication = completion
        .into_publication()
        .expect("capture preflight succeeds")
        .publish(&mut destination)
        .expect("unavailable open is retryable");
    let CompletionPublication::Retry { mut plan, failure } = publication else {
        panic!("missing parent must reject openout")
    };
    assert_eq!(failure.slot().map(|slot| slot.raw()), Some(3));
    assert_eq!(failure.path(), Some(failed.as_path()));
    assert!(failure.committed_prefix() > 0);
    assert!(destination.committed_artifacts().is_empty());
    assert!(
        !later.exists(),
        "later suffix effect must not overtake failure"
    );
    let remaining_before = plan.remaining_effects().len();
    plan.retarget(&failure, replacement.clone())
        .expect("exact retry head retargets");
    assert_ne!(plan.pages()[0].artifact().hash(), original_hash);
    let pdf = plan.pdf().expect("PDF remains owned by retry plan");
    assert_eq!(pdf.pages()[0].artifact, plan.pages()[0].artifact().hash());
    assert!(matches!(
        plan.retarget(&failure, temporary.path().join("stale.tex")),
        Err(EnginePublicationError::StaleRetarget)
    ));
    assert_eq!(plan.remaining_effects().len(), remaining_before);

    let publication = plan
        .publish(&mut destination)
        .expect("retargeted suffix publishes");
    let CompletionPublication::Committed(committed) = publication else {
        panic!("replacement path is available")
    };
    assert_eq!(committed.pages().len(), 1);
    assert_eq!(destination.committed_artifacts().len(), 1);
    assert!(replacement.exists());
    assert!(later.exists());
}

#[test]
fn retry_rejects_destination_divergence_without_publishing_artifacts() {
    let temporary = tempfile::tempdir().expect("temporary publication root");
    let failed = temporary.path().join("missing").join("old.tex");
    let source = format!(
        "\\shipout\\hbox{{\\openout3={}\\relax}}\\end",
        failed.display()
    );
    let completion = capture(source.as_bytes(), EngineCompletionDemand::without_pdf());
    let mut destination = World::real_with_artifact_dir(temporary.path().join("artifacts"));
    let CompletionPublication::Retry { plan, .. } = completion
        .into_publication()
        .expect("capture preflight")
        .publish(&mut destination)
        .expect("unavailable open is retryable")
    else {
        panic!("missing parent must reject openout")
    };
    destination.open_out(StreamSlot::new(4), temporary.path().join("other"));
    let pending_before = destination.effect_records().len();
    assert!(matches!(
        plan.publish(&mut destination),
        Err(EnginePublicationError::Destination(_))
    ));
    assert_eq!(destination.effect_records().len(), pending_before);
    assert!(destination.committed_artifacts().is_empty());
}

#[test]
fn omitted_pdf_demand_does_not_create_a_pdf_projection() {
    let completion = capture(
        br"\shipout\hbox{}\end",
        EngineCompletionDemand::without_pdf(),
    );
    assert!(completion.pdf().is_none());
    assert!(completion.pages()[0].dvi().is_some());
}

#[test]
fn invalid_artifact_preflight_and_dropped_plan_publish_nothing() {
    let mut completion = capture(
        br"\shipout\hbox{}\end",
        EngineCompletionDemand::without_pdf(),
    );
    let original = completion.pages[0].artifact.clone();
    completion.pages[0].artifact = original.with_testing_bytes_preserving_identity(vec![0xff]);
    assert!(matches!(
        completion.into_publication(),
        Err(EnginePublicationError::InvalidArtifactIdentity { page: 0 })
    ));

    let completion = capture(
        br"\shipout\hbox{}\end",
        EngineCompletionDemand::without_pdf(),
    );
    let plan = completion.into_publication().expect("valid completion");
    let destination = World::memory();
    drop(plan);
    assert!(destination.effect_records().is_empty());
    assert!(destination.committed_artifacts().is_empty());
}

#[test]
fn eager_and_retained_destinations_materialize_identical_effect_order() {
    let source = br"\immediate\write16{completion-order}\shipout\hbox{}\end";
    let eager = capture(source, EngineCompletionDemand::without_pdf())
        .into_publication()
        .expect("eager plan");
    let retained = capture(source, EngineCompletionDemand::without_pdf())
        .into_publication()
        .expect("retained plan");

    let mut eager_world = World::memory();
    let CompletionPublication::Committed(_) =
        eager.publish(&mut eager_world).expect("eager publication")
    else {
        panic!("memory open cannot suspend")
    };
    crate::test_harness::with_memory_universe(|retained_universe| {
        retained_universe
            .begin_retained_session()
            .expect("retained destination starts");
        let CompletionPublication::Committed(_) = retained
            .publish(retained_universe.world_mut())
            .expect("retained publication")
        else {
            panic!("retained destination cannot suspend")
        };
        retained_universe
            .export_retained_effects()
            .expect("retained destination exports");
        assert_eq!(
            retained_universe.world().memory_terminal_output(),
            eager_world.memory_terminal_output()
        );
        assert_eq!(
            retained_universe.world().memory_log_output(),
            eager_world.memory_log_output()
        );
        assert_eq!(
            retained_universe.world().committed_artifacts().len(),
            eager_world.committed_artifacts().len()
        );
    });
}

#[test]
fn pdf_completion_enumerates_owned_forms_objects_actions_and_navigation() {
    let completion = capture(
        br"\pdfoutput=1
           \pdfmapline{}
           \pdfglyphtounicode{A}{0041}
           \immediate\pdfobj{ordinary}
           \immediate\pdfobj stream file{payload.bin}
           \pdfcatalog{/PageMode /UseOutlines}
           \setbox7=\hbox{F}\pdfxform attr{/Subtype /Form} resources{/ProcSet [/PDF]} 7
           \pdfoutline attr{/C [1 0 0]} goto name{later} count 0 {Title}
           \shipout\hbox{\pdfrefxform\pdflastxform
             \pdfannot width 5pt height 6pt depth 1pt {/Subtype /Text}
             \pdfstartlink user{/S /URI /URI (https://example.invalid)}X\pdfendlink
             \pdfdest name{later} fit}
           \end",
        EngineCompletionDemand::with_pdf(),
    );
    let pdf = completion.pdf().expect("PDF projection demanded");
    assert_eq!(pdf.pages().len(), 1);
    assert_eq!(pdf.forms().len(), 1);
    assert_eq!(pdf.raw_objects().len(), 2);
    assert_eq!(pdf.raw_object_file_needs().len(), 1);
    assert!(!pdf.font_operations().is_empty());
    assert!(!pdf.annotations().is_empty());
    assert!(!pdf.links().is_empty());
    assert!(!pdf.destinations().is_empty());
    assert!(!pdf.outlines().is_empty());
    assert_eq!(pdf.raw_object_file_needs()[0].source_name, b"payload.bin");
    assert!(matches!(
        pdf.raw_objects()[1].payload,
        Some(tex_state::DetachedPdfRawObjectPayload::FileStream {
            ref source_name,
            ..
        }) if source_name == b"payload.bin"
    ));
    assert_eq!(pdf.document().fragments.catalog, b"/PageMode /UseOutlines");
}

#[test]
fn terminal_completion_values_forbid_live_and_publication_handles() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<DetachedEngineCompletion>();
    assert_send_static::<tex_state::DetachedPdfCompletion>();
    assert_send_static::<PreparedEnginePublication>();

    fn declaration_fields<'a>(source: &'a str, declaration: &str) -> &'a str {
        let start = source.find(declaration).expect("declaration exists");
        let body = &source[start + declaration.len()..];
        body.split_once("\n}").expect("field block closes").0
    }
    let engine_source = include_str!("../engine_completion.rs");
    let pdf_source = include_str!("../../../tex-state/src/pdf/completion.rs");
    for fields in [
        declaration_fields(engine_source, "pub struct DetachedEngineCompletion {"),
        declaration_fields(pdf_source, "pub struct DetachedPdfCompletion {"),
    ] {
        for forbidden in [
            "Universe",
            "World",
            "PdfState",
            "TokenListId",
            "FontId",
            "EffectPos",
            "ArtifactPublication",
            "Arc<",
            "&",
        ] {
            assert!(
                !fields.contains(forbidden),
                "terminal DTO field leaked {forbidden}: {fields}"
            );
        }
    }
}

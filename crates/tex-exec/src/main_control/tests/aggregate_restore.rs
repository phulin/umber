//! Observable full-root replay and ordinary named-restore contracts.

use super::*;
use crate::{
    CanonicalStepResult, CanonicalStepRunner, CheckpointSink, EngineBoundary, OutputLedger,
};

struct FirstParagraph<G> {
    root: Option<crate::EngineCheckpoint<G>>,
}

impl<G> CheckpointSink<G> for FirstParagraph<G> {
    fn wants_checkpoint(&self, boundary: EngineBoundary) -> bool {
        self.root.is_none() && boundary == EngineBoundary::OuterParagraphEnd
    }

    fn checkpoint(&mut self, checkpoint: crate::EngineCheckpoint<G>, _: &Universe<G>) {
        assert!(self.root.replace(checkpoint).is_none());
    }
}

fn next<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    output: &mut OutputLedger,
    sink: &mut FirstParagraph<G>,
) -> CanonicalStepResult {
    CanonicalStepRunner::new(control, stores, output).step(sink, &crate::Cancellation::new())
}

fn finish<G>(
    control: &mut MainControl<G>,
    stores: &mut Universe<G>,
    output: &mut OutputLedger,
    sink: &mut FirstParagraph<G>,
) -> crate::DetachedEngineCompletion {
    for _ in 0..128 {
        match next(control, stores, output, sink) {
            CanonicalStepResult::Completed(_) => {
                return output
                    .detach_checkpoint_prefix(control, stores)
                    .expect("detached output");
            }
            CanonicalStepResult::Progress(_) | CanonicalStepResult::Committed(_) => {}
            result => panic!("unexpected terminal result: {result:?}"),
        }
    }
    panic!("bounded test job must terminate");
}

#[test]
fn public_named_restore_replays_the_same_output_prefix_repeatedly() {
    // TeX82 §§1068/1211: shipout and assignment apply once per execution.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores
            .begin_retained_session()
            .expect("rollback-capable World timeline");
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f\count0=1 A\par\def\letters#1{ab#1cd}\toks0={ef}\shipout\hbox{\letters{gh}\the\toks0}\count0=2\end",
        );
        let mut output = OutputLedger::new();
        let mut sink = FirstParagraph { root: None };
        assert!(matches!(
            next(&mut control, stores, &mut output, &mut sink),
            CanonicalStepResult::Committed(_)
        ));
        let root = sink.root.take().expect("paragraph checkpoint");
        sink.root = Some(root);
        let first = finish(&mut control, stores, &mut output, &mut sink);
        assert_eq!(first.pages().len(), 2);
        let first_plans = first
            .pages()
            .iter()
            .map(|page| page.dvi().cloned())
            .collect::<Vec<_>>();
        for _ in 0..2 {
            let spent = control.fuel_burned();
            control
                .restore_checkpoint(
                    sink.root.as_ref().expect("retained root"),
                    stores,
                    &mut output,
                )
                .expect("public full named restore");
            assert_eq!(stores.count(0).expect("restored count"), 1);
            assert_eq!(control.fuel_burned(), spent, "restore cannot refund fuel");
            assert_eq!(
                output
                    .detach_checkpoint_prefix(&mut control, stores)
                    .expect("restored output")
                    .pages()
                    .len(),
                0
            );
            let replayed = finish(&mut control, stores, &mut output, &mut sink);
            assert_eq!(
                replayed
                    .pages()
                    .iter()
                    .map(|page| page.dvi().cloned())
                    .collect::<Vec<_>>(),
                first_plans
            );
            assert_eq!(stores.count(0).expect("replayed count"), 2);
            assert!(control.fuel_burned() > spent);
        }
    });
}

#[test]
fn resource_replay_discards_interrupted_groups_builders_effects_and_pages() {
    // TeX82 §§279--281/1068: abandoned host attempts do not run unsave or
    // aftergroup; ordinary replay later closes the real group exactly once.
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        stores
            .begin_retained_session()
            .expect("rollback-capable World timeline");
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\count0=1 \def\marker{\global\advance\count2 by1\message{AFTERGROUP}} A\par
            \immediate\openout1=result \immediate\write1{once}\immediate\closeout1
            \shipout\vbox{\hrule}\begingroup\count0=9\aftergroup\marker
            \def\letters#1{AB#1CD\input child}\setbox2=\hbox{\letters{EF}}\endgroup\end",
        );
        let mut output = OutputLedger::new();
        let mut sink = FirstParagraph { root: None };
        assert!(matches!(
            next(&mut control, stores, &mut output, &mut sink),
            CanonicalStepResult::Committed(_)
        ));
        let effect_prefix = stores.world().effect_pos();
        let artifact_prefix = stores.world().committed_artifacts().len();
        let mut need = None;
        for attempt in 0..2 {
            need = None;
            for _ in 0..128 {
                match next(&mut control, stores, &mut output, &mut sink) {
                    CanonicalStepResult::ResourceNeed(found) => {
                        need = Some(found);
                        break;
                    }
                    CanonicalStepResult::Progress(_) | CanonicalStepResult::Committed(_) => {}
                    result => panic!("unexpected interrupted result: {result:?}"),
                }
            }
            assert!(matches!(need, Some(ResourceNeed::Input { .. })));
            assert_eq!(stores.count(0).expect("local count"), 9);
            assert!(admitted!(stores, |context| context.execution_group_depth()) > 0);
            assert!(!control.execution.boxes.active_boxes.is_empty());
            let spent = control.fuel_burned();
            if attempt == 0 {
                let mut foreign_output = OutputLedger::new();
                assert!(matches!(
                    control.restore_resource_replay(
                        sink.root.as_ref().expect("retained root"),
                        stores,
                        &mut foreign_output,
                    ),
                    Err(crate::CheckpointRestoreError::Output(_))
                ));
                assert_eq!(stores.count(0).expect("unchanged local count"), 9);
                assert!(admitted!(stores, |context| context.execution_group_depth()) > 0);
                assert!(
                    !control.execution.boxes.active_boxes.is_empty(),
                    "failed admission cannot discard interrupted owners"
                );
                assert_eq!(control.fuel_burned(), spent);
            }
            control
                .restore_resource_replay(
                    sink.root.as_ref().expect("retained root"),
                    stores,
                    &mut output,
                )
                .expect("full aggregate replay restore");
            assert_eq!(stores.count(0).expect("restored count"), 1);
            assert_eq!(stores.count(2).expect("aftergroup count"), 0);
            assert_eq!(
                admitted!(stores, |context| context.execution_group_depth()),
                0
            );
            assert!(control.execution.boxes.active_boxes.is_empty());
            assert!(control.command.named_boundary_is_quiescent());
            assert_eq!(stores.world().effect_pos(), effect_prefix);
            assert_eq!(stores.world().committed_artifacts().len(), artifact_prefix);
            assert_eq!(
                output
                    .detach_checkpoint_prefix(&mut control, stores)
                    .expect("restored output")
                    .pages()
                    .len(),
                0
            );
            assert_eq!(control.fuel_burned(), spent);
        }
        let need = need.expect("input need");
        let ResourceNeed::Input { name, .. } = &need else {
            unreachable!()
        };
        output
            .fulfill(
                &mut control,
                &need,
                ResourceFulfillment::input(
                    name,
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&b"C"[..]),
                ),
            )
            .expect("verified input answer");
        let completed = finish(&mut control, stores, &mut output, &mut sink);
        assert_eq!(
            completed.pages().len(),
            2,
            "discarded shipouts cannot duplicate output"
        );
        assert_eq!(stores.count(0).expect("group restores count"), 1);
        assert_eq!(stores.count(2).expect("real aftergroup executes"), 1);
        assert_eq!(terminal_text(stores).matches("AFTERGROUP").count(), 1);
        assert_eq!(stores.world().effect_records().iter().filter(|effect| matches!(effect,
            tex_state::EffectRecord::StreamWrite { sink: tex_state::PrintSink::Stream(_), text } if text.contains("once")
        )).count(), 1, "discarded attempts cannot duplicate file writes");
    });
}

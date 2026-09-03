use tex_state::{DependencyRegionError, TrackedRegionBarrier};

use super::*;

#[derive(Debug, Eq, PartialEq)]
struct ParityOutcome {
    steps: Vec<StepResult>,
    count: i32,
    effects: Vec<tex_state::EffectRecord>,
    artifacts: Vec<tex_state::ContentHash>,
    dvi_pages: Vec<crate::dispatch::PreparedDviPage>,
    boundaries: Vec<crate::EngineBoundary>,
}

fn run_complete_job(tracked: bool) -> ParityOutcome {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_cmr10_as(&mut control, stores, "cmr10.tfm");
        register_source(
            &mut control,
            br"\font\f=cmr10 \f A\count0=17\message{tracked-region-parity}\end",
        );
        let mut steps = Vec::new();
        loop {
            let step = if tracked {
                control
                    .advance_with_tracked_region(stores)
                    .expect("tracked operation executes")
                    .step
            } else {
                control
                    .advance(stores)
                    .expect("ordinary operation executes")
            };
            let terminal = matches!(step, StepResult::Progress(MainControlStep::End));
            steps.push(step);
            if terminal {
                break;
            }
        }
        ParityOutcome {
            steps,
            count: stores.count(0).expect("count register"),
            effects: stores.world().effect_records().to_vec(),
            artifacts: stores.world().artifact_commits().to_vec(),
            dvi_pages: control.take_prepared_dvi_pages(),
            boundaries: Vec::new(),
        }
    })
}

#[derive(Debug, Eq, PartialEq)]
struct SuspensionOutcome {
    step: StepResult,
    region_was_published: bool,
    effects: Vec<tex_state::EffectRecord>,
    artifacts: Vec<tex_state::ContentHash>,
    boundaries: Vec<crate::EngineBoundary>,
}

fn run_missing_font(tracked: bool) -> SuspensionOutcome {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\font\missing=not-installed");
        let (step, region_was_published) = if tracked {
            let tracked = control
                .advance_with_tracked_region(stores)
                .expect("tracked suspension is returned");
            (tracked.step, tracked.region.is_some())
        } else {
            (
                control
                    .advance(stores)
                    .expect("ordinary suspension is returned"),
                false,
            )
        };
        SuspensionOutcome {
            step,
            region_was_published,
            effects: stores.world().effect_records().to_vec(),
            artifacts: stores.world().artifact_commits().to_vec(),
            boundaries: Vec::new(),
        }
    })
}

#[test]
fn recording_enabled_and_disabled_jobs_have_identical_committed_outcomes() {
    assert_eq!(run_complete_job(false), run_complete_job(true));
    assert_eq!(run_missing_font(false), run_missing_font(true));
}

fn projection_hashes_for_one_operation(tracked: bool) -> u64 {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\relax\end");
        crate::mode::reset_semantic_fingerprint_calls_for_test();
        if tracked {
            control
                .advance_with_tracked_region(stores)
                .expect("tracked operation executes");
        } else {
            control
                .advance(stores)
                .expect("ordinary operation executes");
        }
        crate::mode::semantic_fingerprint_calls_for_test()
    })
}

#[test]
fn ordinary_operations_skip_tracked_projection_hashing() {
    assert_eq!(projection_hashes_for_one_operation(false), 0);
    assert_eq!(projection_hashes_for_one_operation(true), 1);
}

#[test]
fn fatal_attempt_publishes_the_typed_partial_commit_barrier() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        register_source(&mut control, br"\noindent\discretionary{}{}{}");
        assert_eq!(
            control.advance(stores).expect("paragraph starts"),
            StepResult::Progress(MainControlStep::Continue)
        );
        while control.modes.depth() < 41 {
            control
                .modes
                .push(Mode::RestrictedHorizontal)
                .expect("fill the TeX82 semantic nest");
        }
        let fatal = control
            .advance_with_tracked_region(stores)
            .expect("fatal operation commits its terminal result");
        assert_eq!(fatal.step, StepResult::Progress(MainControlStep::End));
        assert_eq!(
            fatal.region,
            Some(Err(DependencyRegionError::Unsupported(
                TrackedRegionBarrier::FatalPartialCommit
            )))
        );
    });
}

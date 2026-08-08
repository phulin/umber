use std::panic::{AssertUnwindSafe, catch_unwind};

use tex_state::cell::{BankTag, CellId};
use tex_state::{DependencyKey, ObservedDependency, TrackedRegionBarrier, TrackedRegionError};

use super::*;

fn count_key(index: u16) -> DependencyKey {
    DependencyKey::Cell(CellId::new(BankTag::Count, u32::from(index)))
}

fn assert_relevant_perturbation_rejected(
    stores: &Universe,
    observations: &[ObservedDependency],
    expected: DependencyKey,
) {
    assert_eq!(
        stores.validate_dependencies_with_failure_readonly(observations, |key| {
            stores
                .semantic_dependency_value(key)
                .or_else(|| {
                    observations
                        .iter()
                        .find(|observation| observation.key == key)
                        .map(|observation| observation.value.clone())
                })
                .expect("changed dependency belongs to the detached record")
        }),
        Some(expected),
        "semantic perturbation falsely validated"
    );
}

#[test]
fn tracked_operation_perturbation_catches_an_omitted_command_read() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(&mut control, br"\ifnum\count7=0\relax\fi");

    let tracked = control
        .advance_with_tracked_region(&mut stores)
        .expect("conditional operation executes");
    let record = tracked
        .region
        .expect("committed operation publishes its attempt")
        .expect("conditional operation is supported");
    let key = count_key(7);
    assert!(
        record
            .observations()
            .iter()
            .any(|observation| observation.key == key),
        "the scanner's count-register read was not recorded"
    );

    let complete = record.observations().to_vec();
    stores.set_count(8, 1);
    assert!(
        stores
            .validate_dependencies_with_failure_readonly(&complete, |key| {
                stores
                    .semantic_dependency_value(key)
                    .or_else(|| {
                        complete
                            .iter()
                            .find(|observation| observation.key == key)
                            .map(|observation| observation.value.clone())
                    })
                    .expect("changed dependency belongs to the detached record")
            })
            .is_none(),
        "a nearby unrelated register invalidated the operation"
    );

    let omitted = complete
        .iter()
        .filter(|observation| observation.key != key)
        .cloned()
        .collect::<Vec<_>>();
    stores.set_count(7, 1);
    assert_relevant_perturbation_rejected(&stores, &complete, key);
    let omission_detected = catch_unwind(AssertUnwindSafe(|| {
        assert_relevant_perturbation_rejected(&stores, &omitted, key);
    }));
    assert!(
        omission_detected.is_err(),
        "the proof harness accepted a deliberately omitted semantic read"
    );
}

#[derive(Debug, Eq, PartialEq)]
struct ParityOutcome {
    steps: Vec<StepResult>,
    state_hash: u64,
    effects: Vec<tex_state::EffectRecord>,
    artifacts: Vec<tex_state::ContentHash>,
    dvi_pages: Vec<crate::dispatch::PreparedDviPage>,
    boundaries: Vec<crate::EngineBoundary>,
}

fn run_complete_job(tracked: bool) -> ParityOutcome {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_cmr10_as(&mut control, &mut stores, "cmr10.tfm");
    register_source(
        &mut control,
        br"\font\f=cmr10 \f A\count0=17\message{tracked-region-parity}\end",
    );
    let mut steps = Vec::new();
    loop {
        let step = if tracked {
            control
                .advance_with_tracked_region(&mut stores)
                .expect("tracked operation executes")
                .step
        } else {
            control
                .advance(&mut stores)
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
        state_hash: stores.snapshot().state_hash(),
        effects: stores.world().effect_records().to_vec(),
        artifacts: stores.world().artifact_commits().to_vec(),
        dvi_pages: control.take_prepared_dvi_pages(),
        boundaries: control.take_completed_boundaries(),
    }
}

#[test]
fn recording_enabled_and_disabled_jobs_have_identical_committed_outcomes() {
    assert_eq!(run_complete_job(false), run_complete_job(true));

    let mut plain_stores = Universe::new_with_plain_catcodes();
    let mut plain_control = MainControl::tex82_initex(&mut plain_stores);
    register_source(&mut plain_control, br"\font\missing=not-installed");
    let plain = plain_control
        .advance(&mut plain_stores)
        .expect("ordinary suspension is returned");

    let mut tracked_stores = Universe::new_with_plain_catcodes();
    let mut tracked_control = MainControl::tex82_initex(&mut tracked_stores);
    register_source(&mut tracked_control, br"\font\missing=not-installed");
    let tracked = tracked_control
        .advance_with_tracked_region(&mut tracked_stores)
        .expect("tracked suspension is returned");

    assert_eq!(plain, tracked.step);
    assert_eq!(tracked.region, None);
    assert_eq!(
        plain_stores.snapshot().state_hash(),
        tracked_stores.snapshot().state_hash()
    );
    assert_eq!(
        plain_stores.world().effect_records(),
        tracked_stores.world().effect_records()
    );
    assert_eq!(
        plain_stores.world().artifact_commits(),
        tracked_stores.world().artifact_commits()
    );
    assert_eq!(
        plain_control.take_completed_boundaries(),
        tracked_control.take_completed_boundaries()
    );
}

#[test]
fn nested_and_fatal_attempts_publish_no_partial_record() {
    let mut nested_stores = Universe::new_with_plain_catcodes();
    let mut nested_control = MainControl::tex82_initex(&mut nested_stores);
    register_source(&mut nested_control, br"\count0=1");
    let outer = nested_stores
        .begin_tracked_region()
        .expect("start caller-owned region");
    let nested = nested_control
        .advance_with_tracked_region(&mut nested_stores)
        .expect("operation survives rejected nested begin");
    assert_eq!(nested.region, Some(Err(TrackedRegionError::AlreadyActive)));
    assert!(nested_stores.dependency_region_is_active());
    nested_stores
        .abandon_tracked_region(outer)
        .expect("discard caller-owned partial record");
    let clean = nested_stores
        .begin_tracked_region()
        .expect("start clean replacement region");
    let clean = nested_stores
        .finish_tracked_region(clean)
        .expect("finish clean replacement region");
    assert!(clean.observations().is_empty());
    assert!(clean.environment_writes().is_empty());

    let mut fatal_stores = Universe::new_with_plain_catcodes();
    let mut fatal_control = MainControl::tex82_initex(&mut fatal_stores);
    register_source(&mut fatal_control, br"\noindent\discretionary{}{}{}");
    assert_eq!(
        fatal_control
            .advance(&mut fatal_stores)
            .expect("paragraph starts"),
        StepResult::Progress(MainControlStep::Continue)
    );
    while fatal_control.modes.depth() < 41 {
        fatal_control
            .modes
            .push(Mode::RestrictedHorizontal)
            .expect("fill the TeX82 semantic nest");
    }
    let fatal = fatal_control
        .advance_with_tracked_region(&mut fatal_stores)
        .expect("fatal operation commits its terminal result");
    assert_eq!(fatal.step, StepResult::Progress(MainControlStep::End));
    assert_eq!(
        fatal.region,
        Some(Err(TrackedRegionError::UnsupportedRegion(
            TrackedRegionBarrier::FatalPartialCommit
        )))
    );
    assert!(!fatal_stores.dependency_region_is_active());
    let clean = fatal_stores
        .begin_tracked_region()
        .expect("fatal path cleared the recorder");
    let clean = fatal_stores
        .finish_tracked_region(clean)
        .expect("finish post-fatal probe");
    assert!(clean.observations().is_empty());
    assert!(clean.environment_writes().is_empty());
}

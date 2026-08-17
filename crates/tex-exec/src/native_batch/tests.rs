use std::sync::Arc;

use tex_arith::Scaled;
use tex_command::{
    CommandHostCapabilities, CommandProfile, CommandState, NativeBatchProgram,
    RegisteredSourceKind, SourceRegistration,
};
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont, MetricCharTag};
use tex_out::ContentHash;

use super::{PackedEpisodeAttempt, execute_packed_episode};
use crate::{
    CoverageFallbackSafety, EpisodeCoverageFamily, MainControl, MainControlStep,
    RootCompletionPolicy, SemanticEpisodeBarrier,
};

const SOURCE: &[u8] = br"\count0=0\count1=0\count2=0\def\e#1{\advance\count0by#1\global\advance\count1by#1\ifnum#1<5\global\advance\count2by1\else\global\advance\count2by2\fi A\kern#1sp}\shipout\hbox{\e{1}\e{2}\e{3}\e{4}\e{5}\e{6}\e{7}\e{8}}\end";

fn test_font() -> LoadedFont {
    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(500),
        height: Scaled::from_raw(300),
        depth: Scaled::from_raw(100),
        italic_correction: Scaled::from_raw(0),
        tag: MetricCharTag::None,
    });
    LoadedFont::new(
        "batchfont",
        "batchfont.tfm",
        ContentHash::from_bytes(b"batchfont").bytes(),
        0x64b2_0010,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    )
}

fn compile(stores: &tex_state::Universe, source: &[u8]) -> NativeBatchProgram {
    let _ = stores;
    NativeBatchProgram::new(source.len().saturating_div(5).max(1))
}

fn direct_input(
    stores: &mut tex_state::Universe,
    source: &[u8],
) -> (CommandState, CommandHostCapabilities) {
    tex_command::install_tex82_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    let mut command = CommandState::new(CommandProfile::TEX82);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    (command, CommandHostCapabilities::default())
}

fn exact_profile_control(profile: CommandProfile, stores: &mut tex_state::Universe) -> MainControl {
    tex_command::install_tex82_expandable_primitives(stores);
    crate::install_unexpandable_primitives(stores);
    if profile != CommandProfile::TEX82 {
        tex_command::install_etex_expandable_primitives(stores);
        crate::install_etex_unexpandable_primitives(stores);
    }
    if profile == CommandProfile::PDFTEX14029 {
        tex_command::install_pdftex_expandable_primitives(stores);
        tex_command::install_pdftex_unexpandable_primitives(stores);
    }
    MainControl::prepared_initex(profile)
}

fn finish(control: &mut MainControl, stores: &mut tex_state::Universe, packed: bool) {
    if packed {
        loop {
            match control.advance_episode(stores).expect("episode advances") {
                crate::StepResult::Progress(MainControlStep::Continue) => {}
                crate::StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => {
                    break;
                }
                crate::StepResult::Suspended(need) => panic!("unexpected suspension: {need:?}"),
            }
        }
    } else {
        while let MainControlStep::Continue = control.step(stores).expect("canonical step advances")
        {
        }
    }
}

#[test]
fn main_control_packed_root_matches_canonical_artifact_dvi_effects_and_channels() {
    let mut canonical = tex_state::Universe::new_with_plain_catcodes();
    let canonical_font = canonical.intern_font(test_font());
    let mut canonical_control = MainControl::tex82_initex(&mut canonical);
    canonical.set_current_font_global(canonical_font);
    canonical_control.set_dvi_output(true);
    canonical_control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(SOURCE),
        ))
        .expect("canonical root registers");
    finish(&mut canonical_control, &mut canonical, false);

    let mut packed = tex_state::Universe::new_with_plain_catcodes();
    let packed_font = packed.intern_font(test_font());
    let mut packed_control = MainControl::tex82_initex(&mut packed);
    packed.set_current_font_global(packed_font);
    packed_control.set_dvi_output(true);
    packed_control
        .register_root_source_for_batch(
            &packed,
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(SOURCE)),
        )
        .expect("packed root registers");
    finish(&mut packed_control, &mut packed, true);

    assert_eq!(
        [packed.count(0), packed.count(1), packed.count(2)],
        [0, 36, 12]
    );
    assert_eq!(
        packed.world().committed_artifacts(),
        canonical.world().committed_artifacts()
    );
    assert_eq!(
        packed.world().effect_records(),
        canonical.world().effect_records()
    );
    assert_eq!(
        packed.world().memory_terminal_output(),
        canonical.world().memory_terminal_output()
    );
    assert_eq!(
        packed.world().memory_log_output(),
        canonical.world().memory_log_output()
    );
    assert_eq!(
        packed_control.command_work().fuel_charges,
        canonical_control.command_work().fuel_charges,
        "canonical-input episodes retain exact scalar fuel"
    );
    let mut packed_pages = packed_control.take_prepared_dvi_pages();
    let mut canonical_pages = canonical_control.take_prepared_dvi_pages();
    assert_eq!(packed_pages.len(), 1);
    assert_eq!(canonical_pages.len(), 1);
    assert_eq!(
        packed_pages.pop().expect("packed page").into_plan(),
        canonical_pages.pop().expect("canonical page").into_plan()
    );
}

#[test]
fn main_control_batch_resumes_after_scalar_definition_boundaries() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font_id = stores.intern_font(test_font());
    let mut control = MainControl::tex82_initex(&mut stores);
    stores.set_current_font_global(font_id);
    control.set_dvi_output(true);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(SOURCE)),
        )
        .expect("production root registers");

    while stores.world().artifact_commits().is_empty() {
        assert_eq!(
            control
                .advance_episode(&mut stores)
                .expect("episode advances toward output"),
            crate::StepResult::Progress(MainControlStep::Continue)
        );
    }
    assert_eq!(
        control
            .advance_episode(&mut stores)
            .expect("session resumes"),
        crate::StepResult::Progress(MainControlStep::End)
    );
    let telemetry = control.episode_telemetry();
    assert_eq!(
        telemetry.semantic_barriers(SemanticEpisodeBarrier::Output),
        1
    );
    assert_eq!(telemetry.terminals(), 1);
    for family in [
        EpisodeCoverageFamily::CharacterProfile,
        EpisodeCoverageFamily::SourceTokenization,
        EpisodeCoverageFamily::ScannerOrExpansion,
        EpisodeCoverageFamily::GroupLineage,
        EpisodeCoverageFamily::RollbackLineage,
    ] {
        assert_eq!(telemetry.coverage_fallbacks(family), 0);
    }
    assert!(
        telemetry.coverage_fallbacks(EpisodeCoverageFamily::CommandVocabulary) > 0,
        "definitions remain the next ordered migration family"
    );
}

#[test]
fn shipout_checkpoint_restores_the_packed_root_terminal_continuation() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font_id = stores.intern_font(test_font());
    let mut control = MainControl::tex82_initex(&mut stores);
    stores.set_current_font_global(font_id);
    control.set_dvi_output(true);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(SOURCE)),
        )
        .expect("production root registers");
    while stores.world().artifact_commits().is_empty() {
        assert_eq!(
            control
                .advance_episode(&mut stores)
                .expect("episode advances toward output"),
            crate::StepResult::Progress(MainControlStep::Continue)
        );
    }
    let checkpoint = control
        .capture_checkpoint(
            crate::EngineBoundary::ShipoutComplete,
            &mut stores,
            crate::ExecutionBudgetCounters::default(),
        )
        .expect("output checkpoint captures");

    let mut restored = MainControl::with_profile(CommandProfile::TEX82);
    restored
        .restore_checkpoint(&checkpoint, &mut stores)
        .expect("output checkpoint restores");
    assert_eq!(
        restored
            .advance_episode(&mut stores)
            .expect("restored job terminates"),
        crate::StepResult::Progress(MainControlStep::End)
    );
}

#[test]
fn execution_coverage_refusal_rolls_back_the_outer_main_control_transaction() {
    let source = br"\count0=41\shipout\hbox{A\end";
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    stores.set_count(0, 17);
    let program = compile(&stores, source);
    let (mut command, mut capabilities) = direct_input(&mut stores, source);
    let before_hash = stores.snapshot().state_hash();
    let rollback = stores.snapshot_for_local_retry();
    let attempt = execute_packed_episode(
        &mut stores,
        &mut command,
        &mut capabilities,
        &program,
        tex_state::font::NULL_FONT,
        &test_font(),
    )
    .expect("coverage refusal is typed");
    let PackedEpisodeAttempt::Coverage(protocol) = attempt else {
        panic!("malformed supported vocabulary must refuse coverage");
    };
    assert_eq!(
        protocol.safety(),
        CoverageFallbackSafety::ExactAggregateRollback
    );
    assert_eq!(protocol.family(), EpisodeCoverageFamily::ScannerOrExpansion);
    stores.rollback_local_retry_snapshot(rollback);
    assert_eq!(stores.count(0), 17);
    assert_eq!(stores.snapshot().state_hash(), before_hash);
}

#[test]
fn active_observer_is_a_required_barrier_before_mutation() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    stores.set_count(0, 31);
    let program = compile(&stores, br"\count0=99\end");
    let (mut command, mut capabilities) = direct_input(&mut stores, br"\count0=99\end");
    let tracked = stores.begin_tracked_region().expect("observer begins");
    let attempt = execute_packed_episode(
        &mut stores,
        &mut command,
        &mut capabilities,
        &program,
        tex_state::font::NULL_FONT,
        &test_font(),
    )
    .expect("observer refusal is typed");
    assert_eq!(
        attempt,
        PackedEpisodeAttempt::Barrier(SemanticEpisodeBarrier::Observer)
    );
    assert_eq!(stores.count(0), 31);
    let _ = stores.finish_tracked_region(tracked);
}

#[test]
fn schema_11_loaded_and_fresh_packed_state_are_exactly_equal() {
    let initializer = tex_state::Universe::new_with_plain_catcodes();
    let image = initializer.dump_format().expect("schema-11 format dumps");
    let mut loaded = tex_state::Universe::from_format(tex_state::World::memory(), &image)
        .expect("schema-11 format loads");
    let mut fresh = tex_state::Universe::new_with_plain_catcodes();
    let loaded_program = compile(&loaded, SOURCE);
    let fresh_program = compile(&fresh, SOURCE);
    let (mut loaded_command, mut loaded_capabilities) = direct_input(&mut loaded, SOURCE);
    let (mut fresh_command, mut fresh_capabilities) = direct_input(&mut fresh, SOURCE);

    let loaded_result = execute_packed_episode(
        &mut loaded,
        &mut loaded_command,
        &mut loaded_capabilities,
        &loaded_program,
        tex_state::font::NULL_FONT,
        &test_font(),
    )
    .expect("loaded episode executes");
    let fresh_result = execute_packed_episode(
        &mut fresh,
        &mut fresh_command,
        &mut fresh_capabilities,
        &fresh_program,
        tex_state::font::NULL_FONT,
        &test_font(),
    )
    .expect("fresh episode executes");
    assert_eq!(loaded_result, fresh_result);
    assert_eq!(
        loaded.dump_format().expect("loaded result redumps"),
        fresh.dump_format().expect("fresh result dumps")
    );
}

#[test]
fn every_exact_profile_reenters_canonical_input_after_live_catcode_assignment() {
    let source = br"\catcode65=12\count0=7\shipout\hbox{A}\end";
    for profile in [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
    ] {
        let mut stores = tex_state::Universe::new_with_plain_catcodes();
        let font = stores.intern_font(test_font());
        stores.set_current_font_global(font);
        let mut control = exact_profile_control(profile, &mut stores);
        control
            .register_root_source_for_batch(
                &stores,
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(&source[..]),
                ),
            )
            .expect("canonical root registers");
        control.flush_pending_file_framing(&mut stores);

        finish(&mut control, &mut stores, true);

        assert_eq!(stores.count(0), 7, "profile {profile:?}");
        assert_eq!(stores.catcode('A'), tex_state::token::Catcode::Other);
        let telemetry = control.episode_telemetry();
        assert_eq!(
            telemetry.coverage_fallbacks(EpisodeCoverageFamily::CharacterProfile),
            0,
            "profile {profile:?}"
        );
        assert_eq!(
            telemetry.coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
            0,
            "profile {profile:?}"
        );
        // The assignment itself may be absorbed by the scalar operation that
        // follows the episode's explicit command-vocabulary refusal. What
        // matters at this ownership boundary is that the next episode reads
        // `A` with the mutated live catcode and never re-tokenizes the root.
    }
}

#[test]
fn resource_retry_preserves_one_registered_source_stack_and_zero_source_fallback() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\input child\count0=2\shipout\hbox{A}\end"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    let suspended = control
        .advance_episode(&mut stores)
        .expect("missing child suspends");
    assert!(matches!(
        suspended,
        crate::StepResult::Suspended(crate::ResourceNeed::Input { ref name, .. })
            if name == "child.tex"
    ));
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&br"\count1=3 "[..]),
        ),
    );

    finish(&mut control, &mut stores, true);

    assert_eq!([stores.count(0), stores.count(1)], [2, 3]);
    assert_eq!(control.input_level_count(), 0);
    let telemetry = control.episode_telemetry();
    assert_eq!(
        telemetry.coverage_fallbacks(EpisodeCoverageFamily::CharacterProfile),
        0
    );
    assert_eq!(
        telemetry.coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
    assert_eq!(
        telemetry.semantic_barriers(SemanticEpisodeBarrier::Resource),
        1
    );
}

#[test]
fn noexpand_backup_uses_canonical_level_and_never_source_fallback() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\def\e{\relax}\noexpand\e\count0=4\shipout\hbox{A}\end"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    finish(&mut control, &mut stores, true);

    assert_eq!(stores.count(0), 4);
    assert_eq!(control.input_level_count(), 0);
    assert_eq!(
        control
            .episode_telemetry()
            .coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
}

#[test]
fn alignment_template_levels_return_to_the_same_canonical_episode_input() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\setbox0=\vbox{\halign{#\cr A\cr}}\count0=5\end"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    finish(&mut control, &mut stores, true);

    assert_eq!(stores.count(0), 5);
    assert_eq!(control.input_level_count(), 0);
    assert_eq!(
        control
            .episode_telemetry()
            .coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
}

#[test]
fn registered_read_stream_levels_return_to_the_same_canonical_episode_input() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::World,
            Arc::<[u8]>::from(&br"stream body"[..]),
        ),
    );
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\openin3=child \read3 to \line \closein3\count0=6\end"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    finish(&mut control, &mut stores, true);

    assert_eq!(stores.count(0), 6);
    let line = stores.symbol("line").expect("read target was scanned");
    assert!(stores.macro_meaning(line).is_some());
    assert_eq!(control.input_level_count(), 0);
    assert_eq!(
        control
            .episode_telemetry()
            .coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
}

#[test]
fn root_eof_returns_through_canonical_completion_without_source_fallback() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_root_completion_policy(RootCompletionPolicy::StopAtRootEof);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\count0=8"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    finish(&mut control, &mut stores, true);

    assert_eq!(stores.count(0), 8);
    assert_eq!(control.input_level_count(), 0);
    assert_eq!(
        control
            .episode_telemetry()
            .coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
}

#[test]
fn resource_barrier_rolls_back_canonical_input_before_scalar_resume() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font = stores.intern_font(test_font());
    stores.set_current_font_global(font);
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&br"\count0=11\input child\end"[..]),
            ),
        )
        .expect("canonical root registers");
    control.flush_pending_file_framing(&mut stores);

    assert!(matches!(
        control
            .advance_episode(&mut stores)
            .expect("rolled-back prefix resumes scalar and reaches the same input request"),
        crate::StepResult::Suspended(crate::ResourceNeed::Input { ref name, .. })
            if name == "child.tex"
    ));
    // Both the speculative episode and the scalar aggregate reached the
    // missing child after applying this assignment. Suspension restores both
    // aggregates, including their canonical input cursor and Universe state.
    assert_eq!(stores.count(0), 0);
    control.capabilities_mut().register_input(
        "child.tex",
        SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&br" "[..]),
        ),
    );
    finish(&mut control, &mut stores, true);
    assert_eq!(stores.count(0), 11);
    assert_eq!(
        control
            .episode_telemetry()
            .coverage_fallbacks(EpisodeCoverageFamily::SourceTokenization),
        0
    );
}

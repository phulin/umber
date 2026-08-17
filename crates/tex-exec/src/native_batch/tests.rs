use std::sync::Arc;

use tex_arith::Scaled;
use tex_command::{
    CommandProfile, NativeBatchProgram, RegisteredSourceKind, SourceRegistration,
};
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont, MetricCharTag};
use tex_out::ContentHash;

use super::{PackedEpisodeAttempt, execute_packed_episode};
use crate::{
    CoverageFallbackSafety, EpisodeCoverageFamily, MainControl, MainControlStep,
    SemanticEpisodeBarrier,
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
    NativeBatchProgram::compile(
        Arc::<[u8]>::from(source),
        CommandProfile::TEX82,
        stores.endlinechar(),
        |code| {
            let byte = code.to_byte().expect("TeX82 admission is exact byte");
            stores.catcode(char::from(byte))
        },
        source.len().saturating_div(5).max(1),
    )
    .expect("source admits")
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
        {}
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
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(SOURCE),
            ),
        )
        .expect("packed root registers");
    finish(&mut packed_control, &mut packed, true);

    assert_eq!([packed.count(0), packed.count(1), packed.count(2)], [0, 36, 12]);
    assert_eq!(packed.world().committed_artifacts(), canonical.world().committed_artifacts());
    assert_eq!(packed.world().effect_records(), canonical.world().effect_records());
    assert_eq!(packed.world().memory_terminal_output(), canonical.world().memory_terminal_output());
    assert_eq!(packed.world().memory_log_output(), canonical.world().memory_log_output());
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
fn main_control_batch_resumes_after_output_without_fallback() {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let font_id = stores.intern_font(test_font());
    let mut control = MainControl::tex82_initex(&mut stores);
    stores.set_current_font_global(font_id);
    control.set_dvi_output(true);
    control
        .register_root_source_for_batch(
            &stores,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(SOURCE),
            ),
        )
        .expect("production root registers");

    assert_eq!(
        control.advance_episode(&mut stores).expect("output commits"),
        crate::StepResult::Progress(MainControlStep::Continue)
    );
    assert_eq!(
        control.advance_episode(&mut stores).expect("session resumes"),
        crate::StepResult::Progress(MainControlStep::End)
    );
    let telemetry = control.episode_telemetry();
    assert_eq!(telemetry.semantic_barriers(SemanticEpisodeBarrier::Output), 1);
    assert_eq!(telemetry.terminals(), 1);
    for family in [
        EpisodeCoverageFamily::CharacterProfile,
        EpisodeCoverageFamily::SourceTokenization,
        EpisodeCoverageFamily::CommandVocabulary,
        EpisodeCoverageFamily::ScannerOrExpansion,
        EpisodeCoverageFamily::NodeOrFont,
        EpisodeCoverageFamily::GroupLineage,
        EpisodeCoverageFamily::RollbackLineage,
    ] {
        assert_eq!(telemetry.coverage_fallbacks(family), 0);
    }
}

#[test]
fn execution_coverage_refusal_rolls_back_the_outer_main_control_transaction() {
    let source = br"\count0=41\shipout\hbox{A\end";
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    stores.set_count(0, 17);
    let before_hash = stores.snapshot().state_hash();
    let program = compile(&stores, source);
    let rollback = stores.snapshot_for_local_retry();
    let attempt = execute_packed_episode(&mut stores, &program, 0, &test_font())
        .expect("coverage refusal is typed");
    let PackedEpisodeAttempt::Coverage(protocol) = attempt else {
        panic!("malformed supported vocabulary must refuse coverage");
    };
    assert_eq!(protocol.safety(), CoverageFallbackSafety::ExactAggregateRollback);
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
    let tracked = stores.begin_tracked_region().expect("observer begins");
    let attempt = execute_packed_episode(&mut stores, &program, 0, &test_font())
        .expect("observer refusal is typed");
    assert_eq!(attempt, PackedEpisodeAttempt::Barrier(SemanticEpisodeBarrier::Observer));
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

    let loaded_result = execute_packed_episode(&mut loaded, &loaded_program, 0, &test_font())
        .expect("loaded episode executes");
    let fresh_result = execute_packed_episode(&mut fresh, &fresh_program, 0, &test_font())
        .expect("fresh episode executes");
    assert_eq!(loaded_result, fresh_result);
    assert_eq!(
        loaded.dump_format().expect("loaded result redumps"),
        fresh.dump_format().expect("fresh result dumps")
    );
}

use std::sync::Arc;

use tex_arith::Scaled;
use tex_command::{CommandProfile, RegisteredSourceKind, SourceRegistration};
use tex_fonts::{CharMetrics, FontMetrics, LoadedFont, MetricCharTag};
use tex_out::ContentHash;

use super::{
    NativeBatchAttempt, NativeBatchFallback, NativeBatchRequest, run_native_batch_episode,
};
use crate::{MainControl, MainControlStep};

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

#[test]
fn production_episode_matches_canonical_state_artifact_dvi_effects_and_channels() {
    let font = test_font();
    let mut canonical_stores = tex_state::Universe::new_with_plain_catcodes();
    let font_id = canonical_stores.intern_font(font.clone());
    let mut control = MainControl::tex82_initex(&mut canonical_stores);
    canonical_stores.set_current_font_global(font_id);
    control.set_dvi_output(true);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(SOURCE),
        ))
        .expect("canonical source registers");
    while let MainControlStep::Continue = control
        .step(&mut canonical_stores)
        .expect("canonical step executes")
    {}
    let [committed] = canonical_stores.world().committed_artifacts() else {
        panic!("canonical run must ship exactly one page");
    };
    let canonical_artifact =
        tex_out::PageArtifact::from_bytes(committed.bytes()).expect("canonical artifact parses");
    let mut plans = control.take_prepared_dvi_pages();
    assert_eq!(plans.len(), 1);
    let plan = plans.pop().expect("one canonical DVI plan").into_plan();
    let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
    writer.write_page_plan(&plan).expect("DVI page writes");
    let canonical_dvi = writer.finish().expect("DVI stream finishes");

    let admission_stores = tex_state::Universe::new_with_plain_catcodes();
    let attempt = run_native_batch_episode(
        &admission_stores,
        NativeBatchRequest {
            source: Arc::<[u8]>::from(SOURCE),
            expected_calls: 8,
            profile: CommandProfile::TEX82,
            font_id: 0,
            font,
        },
    )
    .expect("production batch output succeeds");
    let NativeBatchAttempt::Completed(shared) = attempt else {
        panic!("supported source must enter the production batch episode");
    };

    assert_eq!(shared.counts, [0, 36, 12]);
    assert_eq!(shared.calls, 8);
    assert_eq!(shared.artifact, canonical_artifact);
    assert_eq!(shared.artifact_bytes, committed.bytes());
    assert_eq!(shared.dvi, canonical_dvi);
    assert_eq!(shared.effects, canonical_stores.world().effect_records());
    assert_eq!(
        shared.terminal,
        canonical_stores
            .world()
            .memory_terminal_output()
            .unwrap_or_default()
    );
    assert_eq!(
        shared.log,
        canonical_stores
            .world()
            .memory_log_output()
            .unwrap_or_default()
    );
}

#[test]
fn observable_command_falls_back_before_mutation() {
    let stores = tex_state::Universe::new_with_plain_catcodes();
    let before_counts = [stores.count(0), stores.count(1), stores.count(2)];
    let before_effects = stores.world().effect_records().len();
    let attempt = run_native_batch_episode(
        &stores,
        NativeBatchRequest {
            source: Arc::<[u8]>::from(&br"\message{barrier}\end"[..]),
            expected_calls: 0,
            profile: CommandProfile::TEX82,
            font_id: 0,
            font: test_font(),
        },
    )
    .expect("fallback is not an execution failure");

    assert!(matches!(
        attempt,
        NativeBatchAttempt::Fallback(NativeBatchFallback::Command(_))
    ));
    assert_eq!(
        [stores.count(0), stores.count(1), stores.count(2)],
        before_counts
    );
    assert_eq!(stores.world().effect_records().len(), before_effects);
    assert!(stores.world().committed_artifacts().is_empty());
}

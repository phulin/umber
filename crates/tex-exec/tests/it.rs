use std::sync::Arc;

use test_support::{CompileFailDependency, assert_compile_fail};
use tex_command::{
    CommandObservation, CommandObserver, CommandProfile, MutationTarget, ObservationValue,
    RegisteredSourceKind, SourceRegistration,
};
use tex_exec::{MainControl, MainControlStep, ResourceNeed, StepResult};
use tex_out::dvi::{DviPagePlan, DviStreamWriter};
use tex_state::{
    EffectRecord, InteractionMode, PrintSink, Universe,
    meaning::{Meaning, UnexpandablePrimitive},
};

fn run_tex82(source: &[u8], tracing_online: bool) -> String {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    if tracing_online {
        stores.set_int_param(tex_state::env::banks::IntParam::TRACING_ONLINE, 1);
    }
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("test source executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let committed = stores
        .world()
        .memory_log_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::Log | PrintSink::TerminalAndLog,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

#[derive(Default)]
struct ObservationCollector(Vec<CommandObservation>);

impl CommandObserver for ObservationCollector {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn observed_etex(source: &[u8]) -> (Universe, Vec<CommandObservation>) {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    tex_command::install_tex82_expandable_primitives(&mut stores);
    tex_command::install_etex_expandable_primitives(&mut stores);
    tex_exec::install_unexpandable_primitives(&mut stores);
    tex_exec::install_etex_unexpandable_primitives(&mut stores);
    let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");
    let mut observer = ObservationCollector::default();
    loop {
        match control
            .step_with_observer(&mut stores, &mut observer)
            .expect("observed e-TeX source executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    (stores, observer.0)
}

fn etex_session(source: &[u8]) -> (MainControl, Universe) {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(InteractionMode::Nonstop);
    tex_command::install_tex82_expandable_primitives(&mut stores);
    tex_command::install_etex_expandable_primitives(&mut stores);
    tex_exec::install_unexpandable_primitives(&mut stores);
    tex_exec::install_etex_unexpandable_primitives(&mut stores);
    let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");
    (control, stores)
}

fn serialize_dvi_page(plan: &DviPagePlan) -> Vec<u8> {
    let mut writer = DviStreamWriter::new(Vec::new());
    writer.write_page_plan(plan).expect("DVI page writes");
    writer.finish().expect("DVI file finishes")
}

#[test]
fn fresh_and_memo_shipouts_share_canonical_artifact_dvi() {
    let source: &[u8] = br"\setbox0=\hbox{\kern1pt}\shipout\copy0\shipout\copy0\end";
    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_shipout_memo();
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_dvi_output(true);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("shipouts execute") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let artifacts = stores.world().committed_artifacts();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(artifacts[0].bytes(), artifacts[1].bytes());
    assert_eq!(control.pure_memo_stats().shipout_hits, 1);
    let plans = control
        .take_prepared_dvi_pages()
        .into_iter()
        .map(tex_exec::PreparedDviPage::into_plan)
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0], plans[1]);
    assert_eq!(serialize_dvi_page(&plans[0]), serialize_dvi_page(&plans[1]));
}

#[test]
fn dvi_disabled_fresh_and_memo_shipouts_both_omit_plans() {
    let source: &[u8] = br"\setbox0=\hbox{\kern1pt}\shipout\copy0\shipout\copy0\end";
    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_shipout_memo();
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_dvi_output(false);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("shipouts execute") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    let artifacts = stores.world().committed_artifacts();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(artifacts[0].bytes(), artifacts[1].bytes());
    assert_eq!(control.pure_memo_stats().shipout_hits, 1);
    assert!(control.take_prepared_dvi_pages().is_empty());
}

#[test]
fn live_shipout_has_no_second_dvi_emitter() {
    let direct = include_str!("../src/shipout/direct.rs");
    assert!(!direct.contains("DviPagePlanBuilder"));
    assert!(!direct.contains("mod materialize"));
    assert_eq!(direct.matches("DviPagePlan::compile_v10").count(), 1);
}

#[test]
fn unified_operation_preserves_state_output_and_typed_evidence() {
    let source = br"\count0=7\afterassignment\relax\count1=9\setbox0=\hbox{A}\halign{#\cr B\cr}\write16{receipt}\end";
    let (mut ordinary, mut ordinary_stores) = etex_session(source);
    loop {
        match ordinary
            .advance_episode(&mut ordinary_stores)
            .expect("canonical episode execution")
        {
            StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
            StepResult::Progress(MainControlStep::Continue) => {}
            StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
        }
    }
    assert!(
        ordinary.episode_telemetry().operations() > ordinary.episode_telemetry().commits(),
        "the broad e-TeX fixture must exercise multi-operation canonical episodes"
    );

    let (mut observed, mut observed_stores) = etex_session(source);
    let mut evidence = ObservationCollector::default();
    loop {
        match observed
            .step_with_observer(&mut observed_stores, &mut evidence)
            .expect("observed execution")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(
        ordinary_stores.snapshot().state_hash(),
        observed_stores.snapshot().state_hash()
    );
    assert_eq!(
        ordinary_stores.world().effect_records(),
        observed_stores.world().effect_records()
    );
    assert_eq!(
        ordinary_stores.world().artifact_commits(),
        observed_stores.world().artifact_commits()
    );
    assert!(
        ordinary_stores
            .world()
            .memory_terminal_output()
            .is_some_and(|bytes| !bytes.is_empty()),
        "independent ordinary producer must commit world output"
    );
    assert!(
        !ordinary_stores.world().artifact_commits().is_empty(),
        "independent ordinary producer must exercise artifact publication"
    );
    assert!(
        evidence
            .0
            .iter()
            .any(|record| matches!(record, CommandObservation::Alignment(_)))
    );
    assert_eq!(register_mutation_keys(&evidence.0), ["count:0", "count:1"]);
    assert!(observed_stores.copy_box_to_page(0).is_some());
}

#[test]
fn unified_operation_resource_suspension_is_observation_independent() {
    let source = br"\input absent-resource";
    let (mut ordinary, mut ordinary_stores) = etex_session(source);
    let (mut observed, mut observed_stores) = etex_session(source);
    let mut evidence = ObservationCollector::default();

    let ordinary_need = loop {
        if let StepResult::Suspended(need) = ordinary
            .advance(&mut ordinary_stores)
            .expect("ordinary operation")
        {
            break need;
        }
    };
    let observed_need = loop {
        if let StepResult::Suspended(need) = observed
            .advance_with_observer(&mut observed_stores, &mut evidence)
            .expect("observed operation")
        {
            break need;
        }
    };

    assert_eq!(ordinary_need, observed_need);
    assert!(matches!(ordinary_need, ResourceNeed::Input { .. }));
    assert_eq!(
        ordinary_stores.snapshot().state_hash(),
        observed_stores.snapshot().state_hash()
    );
    assert!(
        evidence.0.is_empty(),
        "rolled-back evidence must not publish"
    );
}

#[test]
fn predecessor_operation_branches_are_absent() {
    let source = include_str!("../src/main_control.rs");
    let episode = include_str!("../src/episode.rs");
    let universe = include_str!("../../tex-state/src/universe.rs");
    let state_facade = include_str!("../../tex-state/src/lib.rs");
    assert!(source.contains("fn execute_operation("));
    for predecessor in [
        "fn step_once(",
        "fn alignment_step_once(",
        "fn step_with_observer_once(",
        "struct StepSnapshot",
        "fn execute_aggregate_operation(",
        "snapshot_for_local_retry(",
    ] {
        assert!(
            !source.contains(predecessor),
            "retained predecessor: {predecessor}"
        );
    }
    for predecessor in [
        "struct LocalRetrySnapshot",
        "snapshot_for_local_retry(",
        "rollback_for_local_retry(",
        "rollback_local_retry_snapshot(",
    ] {
        assert!(
            !universe.contains(predecessor),
            "retained state retry predecessor: {predecessor}"
        );
        assert!(
            !state_facade.contains(predecessor),
            "retained state retry export: {predecessor}"
        );
    }
    for predecessor in ["EpisodeInternalStop", "InternalStop("] {
        assert!(
            !episode.contains(predecessor),
            "retained episode-lineage predecessor: {predecessor}"
        );
    }
}

#[test]
fn fused_hot_and_typed_cold_dispatch_share_one_interpreter() {
    let control = include_str!("../src/main_control.rs");
    let interpreter = include_str!("../src/interpreter.rs");
    let hot = include_str!("../src/main_control/hot_apply.rs");
    let cold = include_str!("../src/main_control/cold/mod.rs");
    let cold_operation = include_str!("../src/main_control/cold/operation.rs");
    let cold_scan = include_str!("../src/main_control/cold/scan.rs");
    let cold_apply = include_str!("../src/main_control/cold/apply.rs");

    assert!(control.contains("mod cold;"));
    assert!(control.contains("mod hot_apply;"));
    assert_eq!(control.matches("fn command_processor<'a>(").count(), 1);
    assert_eq!(
        interpreter.matches("CommandProcessor::borrowed(").count(),
        1
    );
    assert!(!control.contains("enum ScannedStep"));
    assert!(!control.contains("struct PreparedOperation"));

    assert!(cold.contains("mod operation;"));
    assert!(cold.contains("mod scan;"));
    assert!(cold.contains("mod apply;"));
    assert!(cold_operation.contains("enum ColdOperation"));
    assert!(cold_scan.contains("fn scan("));
    assert!(cold_apply.contains("fn apply("));
    assert!(!hot.contains("ColdOperation::"));
    assert!(!hot.contains("PreparedColdOperation {"));
}

#[test]
fn canonical_episode_has_no_admission_executor_or_coverage_fallback() {
    let control = include_str!("../src/main_control.rs");
    let facade = include_str!("../src/lib.rs");
    let command_facade = include_str!("../../tex-command/src/lib.rs");
    let episode = include_str!("../src/episode.rs");

    assert!(control.contains("fn execute_operation("));
    assert!(control.contains("pub fn advance_episode("));
    for retired in [
        "NativeBatchProgram",
        "PackedRootEpisode",
        "advance_packed_root",
        "execute_packed_episode",
        "register_root_source_for_batch",
        "EpisodeCoverageFallback",
    ] {
        assert!(
            !control.contains(retired),
            "retained executor path: {retired}"
        );
        assert!(
            !facade.contains(retired),
            "retained executor export: {retired}"
        );
        assert!(
            !command_facade.contains(retired),
            "retained command executor export: {retired}"
        );
        assert!(
            !episode.contains(retired),
            "retained fallback protocol: {retired}"
        );
    }
}

#[test]
fn receipt_categories_are_append_bounded_consumed_and_closed_before_commit() {
    let receipt = include_str!("../src/execution_receipt.rs");
    for method in [
        "fn push_mutation",
        "fn push_diagnostic",
        "fn push_semantic_effect",
        "fn record_resource",
        "fn record_world_effect",
        "fn record_artifact",
    ] {
        let body = receipt
            .split_once(method)
            .unwrap_or_else(|| panic!("missing receipt append authority {method}"))
            .1
            .split_once("\n    }")
            .expect("bounded receipt method body")
            .0;
        assert!(body.contains("if !self.has_capacity()"), "{method}");
        assert!(
            body.find("has_capacity").expect("receipt capacity check")
                < body.find(".push(").expect("receipt vector append"),
            "{method} must reject before vector growth"
        );
    }
    let consume = receipt
        .split_once("pub(crate) fn consume(self)")
        .expect("active receipt consumer")
        .1;
    let consume = consume
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    for category in [
        "self.mutations.len()",
        "self.resources.len()",
        "self.effects.semantic.len()",
        "self.effects.world.len()",
        "self.artifacts.len()",
        "self.diagnostics.len()",
        "self.termination",
    ] {
        assert!(consume.contains(category), "unconsumed category {category}");
    }

    let control = include_str!("../src/main_control.rs");
    let operation = control
        .split_once("fn execute_direct_episode(")
        .expect("direct operation authority")
        .1
        .split_once("fn execute_operation(")
        .expect("operation authority boundary")
        .0;
    assert!(
        operation
            .find("admit_observed_receipt")
            .expect("receipt admission")
            < operation
                .rfind("commit_direct_operation")
                .expect("direct operation commit"),
        "world/artifact/geometry/termination receipt closes before commit"
    );
    let failed = control
        .split_once("fn finish_direct_failure(")
        .expect("failed-operation authority")
        .1
        .split_once("fn execute_direct_episode(")
        .expect("failed-operation authority boundary")
        .0;
    assert!(
        failed
            .find("admit_observed_receipt")
            .expect("fatal receipt")
            < failed
                .find("commit_direct_operation")
                .expect("fatal direct commit"),
        "fatal receipt closes before its direct operation commits"
    );
    assert!(control.contains("pending.consume_into(publish.then_some(observer))"));
}

fn register_mutation_keys(observations: &[CommandObservation]) -> Vec<&str> {
    observations
        .iter()
        .filter_map(|observation| match observation {
            CommandObservation::Mutation(record) if record.target == MutationTarget::Register => {
                match &record.key {
                    ObservationValue::Name(key) => Some(key.as_str()),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

#[test]
fn assignment_committer_owns_redundancy_glue_identity_and_afterassignment_order() {
    let (_, observations) = observed_etex(
        br"\count0=13{\count0=13\global\count0=13}\skip0=1pt{\skip0=1pt}\skip0=0pt{\skip0=0pt}\def\mark{\count1=7}\afterassignment\mark\count0=3\end",
    );
    let keys = register_mutation_keys(&observations);
    assert_eq!(keys.iter().filter(|key| **key == "count:0").count(), 3);
    assert_eq!(keys.iter().filter(|key| **key == "skip:0").count(), 3);
    let final_count = keys
        .iter()
        .rposition(|key| *key == "count:0")
        .expect("the assigned count register has a receipt");
    let after_count = keys
        .iter()
        .rposition(|key| *key == "count:1")
        .expect("the afterassignment body has a receipt");
    assert!(
        final_count < after_count,
        "afterassignment runs after its commit"
    );
}

#[test]
fn assignment_committer_emits_sparse_box_receipt_and_suppresses_overflow_write() {
    let (stores, observations) =
        observed_etex(br"\setbox32103=\hbox{}\count0=2147483647\advance\count0 by1\end");
    assert_eq!(stores.count(0), i32::MAX);
    let keys = register_mutation_keys(&observations);
    assert!(keys.contains(&"box:32103"));
    assert_eq!(keys.iter().filter(|key| **key == "count:0").count(), 1);
}

#[test]
fn let_endgroup_alias_runs_off_save_and_restores_the_primitive() {
    // TeX82 §§1215/1063--1066: `\let` copies the `end_group` command,
    // so the alias must take `off_save` inside a `simple_group`, regardless
    // of its spelling. The inserted right brace then runs §283 `unsave`
    // before the alias is replayed, restoring the locally redefined
    // `\endgroup`. Frozen alignment sentinels have distinct `EndV`/
    // `EndTemplate` meanings and continue through the alignment dispatch
    // covered by the tests below.
    let mut stores = Universe::new_with_plain_catcodes();
    let alias = stores.intern("alias");
    let restored = stores.intern("restored");
    stores.set_interaction_mode(InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_fuel_limit(128).expect("bounded command fuel");
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(
                br"\let\alias=\endgroup{\def\endgroup{\alias\alias}\alias\let\restored=\endgroup\count0=17"
                    .as_slice(),
            ),
        ))
        .expect("test source registers");

    loop {
        match control.step(&mut stores).expect("alias recovery executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }

    assert_eq!(
        stores.meaning(alias),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup)
    );
    assert_eq!(stores.meaning(restored), stores.meaning(alias));
    assert_eq!(stores.count(0), 17);
    assert!(control.fuel_burned() < 128);
    let transcript = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(transcript.matches("! Missing } inserted.").count(), 1);
    assert_eq!(transcript.matches("! Extra \\endgroup.").count(), 1);
}

#[test]
fn restricted_horizontal_hrule_reports_source_before_rule_spec_lookahead() {
    // TeX82 §1095 diagnoses this command in `head_for_vmode`, before §463
    // scans a rule specification. §82 must therefore display the physical
    // source line, not a token level created by keyword lookahead.
    let transcript = run_tex82(b"\\setbox0=\\hbox{\n\\hrule\n}\\end", true);
    let diagnostic = transcript
        .find("! You can't use `\\hrule' here except with leaders.")
        .unwrap_or_else(|| panic!("TeX82 §1095 diagnostic: {transcript:?}"));
    let context = &transcript[diagnostic..];
    assert!(context.contains("l.2 \\hrule"), "{transcript:?}");
    assert!(!context.contains("<to be read again>"), "{transcript:?}");
}

#[test]
fn restricted_horizontal_prevdepth_reports_before_scanning_an_operand() {
    // TeX82 §1243's `alter_aux` compares `cur_chr` with `abs(mode)` before
    // `scan_optional_equals` and `scan_normal_dimen`.
    let transcript = run_tex82(br"\setbox0=\hbox{\prevdepth\relax X}\end", false);
    assert!(
        transcript.contains("! You can't use `\\prevdepth' in restricted horizontal mode."),
        "{transcript}"
    );
    assert!(
        !transcript.contains("Missing number, treated as zero."),
        "{transcript}"
    );
}

#[test]
fn alignment_closing_brace_reports_inserted_cr_and_followup_brace() {
    let transcript = run_tex82(
        br"\long\def\l#1{}\let\PAR=\par\def\par{\relax\PAR}\halign{#&#&\l{#}\cr a&b&c&&&.}\par\cr}\end",
        true,
    );
    let diagnostic = transcript
        .find("! Missing \\cr inserted.")
        .unwrap_or_else(|| panic!("TeX82 §§82/1132 diagnostic: {transcript:?}"));
    let inserted = transcript[diagnostic..]
        .find("<inserted text>")
        .map(|offset| diagnostic + offset)
        .unwrap_or_else(|| panic!("inserted frozen \\cr context: {transcript:?}"));
    assert!(diagnostic < inserted, "{transcript:?}");
    let missing_left_brace = transcript[diagnostic..]
        .find("! Missing { inserted.")
        .map(|offset| diagnostic + offset)
        .unwrap_or_else(|| panic!("TeX82 §1127 diagnostic: {transcript:?}"));
    assert!(diagnostic < missing_left_brace, "{transcript:?}");
}

#[test]
fn misplaced_tab_in_v_template_retains_synchronous_error_context() {
    let transcript = run_tex82(
        br"\let\lb={\let\rb=}\halign\relax{\span\iffalse}\fi\cr#&\ifnum0=`{\fi\cr\cr}\end",
        false,
    );
    assert!(
        transcript.contains("<template> &\n            \\ifnum 0=`{\\fi \\endtemplate "),
        "TeX82 §§82,1128 diagnose before the retained v-template retires: {transcript}"
    );
}

#[test]
fn paragraph_start_page_build_reports_backed_up_context_before_help() {
    let transcript = run_tex82(
        br"\topskip=0pt \vsize=100pt \setbox1=\hbox{}\copy1 \vskip0pt minus 1fil$x$\end",
        true,
    );
    let error = transcript
        .find("! Infinite glue shrinkage found on current page.")
        .expect("error line");
    let context = transcript[error..]
        .find("<to be read again>")
        .map(|offset| error + offset)
        .unwrap_or_else(|| panic!("live command context: {transcript:?}"));
    let help = transcript[error..]
        .find("The page about to be output contains some infinitely")
        .map(|offset| error + offset)
        .expect("page-error help");
    assert!(error < context && context < help, "{transcript:?}");
}

#[test]
fn text_accent_in_math_reports_before_scanning_its_character() {
    let transcript = run_tex82(br"\setbox3=\hbox{x}$\unhcopy3\accent65x$\end", true);
    assert!(
        transcript.contains("Please use \\mathaccent for accents in math mode"),
        "{transcript:?}"
    );
    assert!(
        transcript.contains("\n<recently read> \\accent \n"),
        "TeX82 §§82,1110 retain the exhausted command level through the diagnostic: {transcript:?}"
    );
    assert!(
        !transcript.contains("<to be read again> 6"),
        "§436 must not consume the operand before §1110 reports"
    );
}

#[test]
fn resource_lookup_maps_available_values() {
    match tex_exec::ResourceLookup::Available(21_u8).map(u16::from) {
        tex_exec::ResourceLookup::Available(value) => assert_eq!(value, 21),
        _ => panic!("available execution resource must remain available after mapping"),
    }
}

#[test]
fn command_fuel_can_only_be_owned_by_a_session_ledger() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let tex_command_dir = manifest_dir.join("../tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &tex_command_dir)];
    assert_compile_fail(
        "command-fuel-construction-forbidden",
        &manifest_dir.join("tests/ui/command_fuel_construction_forbidden.rs"),
        &dependencies,
        &[
            "associated function `new` is private",
            "the trait bound `CommandFuel: Default` is not satisfied",
        ],
    );
    assert_compile_fail(
        "command-fuel-fields-forbidden",
        &manifest_dir.join("tests/ui/command_fuel_fields_forbidden.rs"),
        &dependencies,
        &["fields `limit` and `work` of struct `CommandFuel` are private"],
    );
}

#[test]
fn session_ledger_lends_typed_fuel_without_transferring_ownership() {
    fn leaf_operation(fuel: &mut tex_command::CommandFuel) {
        fuel.charge().expect("session funds leaf operation");
    }

    let mut session =
        tex_command::CommandFuelLedger::new(2).expect("valid top-level session limit");
    leaf_operation(session.fuel_mut());
    leaf_operation(session.fuel_mut());
    assert_eq!(session.burned(), 2);
}

#[test]
fn engine_checkpoint_cannot_be_forged_by_callers() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-exec", &manifest_dir),
        CompileFailDependency::path("tex-state", &tex_state_dir),
    ];
    assert_compile_fail(
        "engine-checkpoint-forgery-forbidden",
        &manifest_dir.join("tests/ui/engine_checkpoint_forgery_forbidden.rs"),
        &dependencies,
        &["cannot construct `EngineCheckpoint`", "private fields"],
    );
}

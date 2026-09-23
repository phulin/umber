use std::sync::Arc;

mod support;

use test_support::{CompileFailDependency, assert_compile_fail};
use tex_command::{
    CommandObservation, CommandObserver, CommandProfile, MutationTarget, ObservationValue,
    RegisteredSourceKind, SourceRegistration,
};
use tex_exec::{ExecError, MainControl, MainControlStep, ResourceNeed, StepResult};
use tex_out::dvi::{DviPagePlan, DviStreamWriter};
use tex_state::{
    EffectRecord, PrintSink, PureMemoConfig, PureMemoRecordingPolicy, PureMemoRuntime,
    ResolvedMeaning, Universe,
    env::AssignmentScope,
    meaning::{Meaning, UnexpandablePrimitive},
};

fn run_tex82(source: &[u8], tracing_online: bool) -> String {
    support::with_plain_universe(|stores| {
        if tracing_online {
            stores
                .command_context()
                .expect("command admission")
                .assign_int_param(
                    tex_state::env::banks::IntParam::TRACING_ONLINE,
                    1,
                    AssignmentScope::Global,
                )
                .expect("tracing assignment");
        }
        let mut control = MainControl::tex82_initex(stores);
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("test source registers");

        loop {
            match control.advance(stores).expect("test source executes") {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
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
    })
}

#[derive(Default)]
struct ObservationCollector(Vec<CommandObservation>);

impl CommandObserver for ObservationCollector {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn observed_etex(source: &[u8]) -> (i32, Vec<CommandObservation>) {
    support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        let mut observer = ObservationCollector::default();
        loop {
            match control
                .advance_with_observer(stores, &mut observer)
                .expect("observed e-TeX source executes")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }
        let count = stores
            .command_context()
            .expect("command admission")
            .count(0)
            .expect("count register");
        (count, observer.0)
    })
}

fn etex_session<G>(stores: &mut Universe<G>, source: &[u8]) -> MainControl<G> {
    tex_command::install_tex82_expandable_primitives(stores);
    tex_command::install_etex_expandable_primitives(stores);
    tex_exec::install_unexpandable_primitives(stores);
    tex_exec::install_etex_unexpandable_primitives(stores);
    let mut control = MainControl::prepared_initex(CommandProfile::ETEX26);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("test source registers");
    control
}

fn serialize_dvi_page(plan: &DviPagePlan) -> Vec<u8> {
    let mut writer = DviStreamWriter::new(Vec::new());
    writer.write_page_plan(plan).expect("DVI page writes");
    writer.finish().expect("DVI file finishes")
}

#[test]
fn fresh_and_memo_shipouts_share_canonical_artifact_dvi() {
    let source: &[u8] = br"\setbox0=\hbox{\kern1pt}\shipout\copy0\shipout\copy0\end";
    support::with_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.install_pure_memo_runtime(PureMemoRuntime::new(PureMemoConfig {
            recording: PureMemoRecordingPolicy::all(),
            ..PureMemoConfig::default()
        }));
        control.set_dvi_output(true);
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("test source registers");

        loop {
            match control.advance(stores).expect("shipouts execute") {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
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
    });
}

#[test]
fn dvi_disabled_fresh_and_memo_shipouts_both_omit_plans() {
    let source: &[u8] = br"\setbox0=\hbox{\kern1pt}\shipout\copy0\shipout\copy0\end";
    support::with_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control.install_pure_memo_runtime(PureMemoRuntime::new(PureMemoConfig {
            recording: PureMemoRecordingPolicy::all(),
            ..PureMemoConfig::default()
        }));
        control.set_dvi_output(false);
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("test source registers");

        loop {
            match control.advance(stores).expect("shipouts execute") {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let artifacts = stores.world().committed_artifacts();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(artifacts[0].bytes(), artifacts[1].bytes());
        assert_eq!(control.pure_memo_stats().shipout_hits, 1);
        assert!(control.take_prepared_dvi_pages().is_empty());
    });
}

#[test]
fn unified_operation_preserves_state_output_and_typed_evidence() {
    let source = br"\count0=7\afterassignment\relax\count1=9\setbox0=\hbox{A}\halign{#\cr B\cr}\write16{receipt}\end";
    let ordinary = support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        loop {
            match control
                .advance_episode(stores)
                .expect("canonical episode execution")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource need: {need:?}"),
            }
        }
        let telemetry = control.episode_telemetry();
        let context = stores.command_context().expect("command admission");
        let state = (
            context.count(0).expect("count zero"),
            context.count(1).expect("count one"),
            context.box_register(0).is_some(),
        );
        drop(context);
        (
            state,
            stores.world().effect_records().to_vec(),
            stores
                .world()
                .committed_artifacts()
                .iter()
                .map(|artifact| artifact.hash())
                .collect::<Vec<_>>(),
            stores
                .world()
                .memory_terminal_output()
                .is_some_and(|bytes| !bytes.is_empty()),
            telemetry,
        )
    });
    assert!(
        ordinary.4.operations() > ordinary.4.commits(),
        "the broad e-TeX fixture must exercise multi-operation canonical episodes"
    );

    let (observed, evidence) = support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        let mut evidence = ObservationCollector::default();
        loop {
            match control
                .advance_with_observer(stores, &mut evidence)
                .expect("observed execution")
            {
                StepResult::Progress(MainControlStep::End | MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }
        let context = stores.command_context().expect("command admission");
        let state = (
            context.count(0).expect("count zero"),
            context.count(1).expect("count one"),
            context.box_register(0).is_some(),
        );
        drop(context);
        (
            (
                state,
                stores.world().effect_records().to_vec(),
                stores
                    .world()
                    .committed_artifacts()
                    .iter()
                    .map(|artifact| artifact.hash())
                    .collect::<Vec<_>>(),
            ),
            evidence,
        )
    });
    let stepped = support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        loop {
            match control.advance(stores).expect("step execution") {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }
        let context = stores.command_context().expect("command admission");
        let state = (
            context.count(0).expect("count zero"),
            context.count(1).expect("count one"),
            context.box_register(0).is_some(),
        );
        drop(context);
        (
            state,
            stores.world().effect_records().to_vec(),
            stores
                .world()
                .committed_artifacts()
                .iter()
                .map(|artifact| artifact.hash())
                .collect::<Vec<_>>(),
        )
    });

    assert_eq!(ordinary.0, observed.0);
    assert_eq!(ordinary.1, observed.1);
    assert_eq!(ordinary.2, observed.2);
    assert_eq!(
        (ordinary.0, &ordinary.1, &ordinary.2),
        (stepped.0, &stepped.1, &stepped.2)
    );
    assert!(
        ordinary.3,
        "independent ordinary producer must commit world output"
    );
    assert!(
        !ordinary.2.is_empty(),
        "independent ordinary producer must exercise artifact publication"
    );
    assert!(
        evidence
            .0
            .iter()
            .any(|record| matches!(record, CommandObservation::Alignment(_)))
    );
    assert_eq!(register_mutation_keys(&evidence.0), ["count:0", "count:1"]);
    assert!(observed.0.2);
}

#[test]
fn unified_operation_resource_need_is_observation_independent() {
    let source = br"\count0=7\input absent-resource";
    let (ordinary_need, ordinary_effects) = support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        let need = loop {
            if let StepResult::Suspended(need) =
                control.advance(stores).expect("ordinary operation")
            {
                break need;
            }
        };
        assert_eq!(
            need,
            ResourceNeed::Input {
                name: "absent-resource.tex".to_owned(),
                original_name: "absent-resource".to_owned(),
            }
        );
        control.capabilities_mut().register_input(
            "absent-resource.tex",
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(&b""[..])),
        );
        assert!(matches!(
            control.advance(stores),
            Err(ExecError::ResourceReplayRequired)
        ));
        (need, stores.world().effect_records().to_vec())
    });
    let (observed_need, observed_effects, evidence) = support::with_plain_universe(|stores| {
        let mut control = etex_session(stores, source);
        let mut evidence = ObservationCollector::default();
        let need = loop {
            if let StepResult::Suspended(need) = control
                .advance_with_observer(stores, &mut evidence)
                .expect("observed operation")
            {
                break need;
            }
        };
        (need, stores.world().effect_records().to_vec(), evidence)
    });

    assert_eq!(ordinary_need, observed_need);
    assert!(matches!(ordinary_need, ResourceNeed::Input { .. }));
    assert_eq!(ordinary_effects, observed_effects);
    assert!(
        evidence.0.is_empty(),
        "rolled-back evidence must not publish"
    );
}

#[test]
fn command_host_facts_are_sampled_only_by_the_consuming_query() {
    fn facts(source: &[u8]) -> (u64, u64) {
        support::with_plain_universe(|stores| {
            let mut control = etex_session(stores, source);
            loop {
                match control.advance(stores).expect("host fact probe executes") {
                    StepResult::Progress(MainControlStep::End)
                    | StepResult::Progress(MainControlStep::EndOfInput) => break,
                    StepResult::Progress(MainControlStep::Continue) => {}
                    StepResult::Suspended(need) => {
                        panic!("unexpected resource suspension: {need:?}")
                    }
                }
            }
            let telemetry = control.episode_telemetry();
            (
                telemetry.host_fact_queries(),
                telemetry.effective_tail_traversals(),
            )
        })
    }

    let ordinary = facts(br"\relax\end");
    let mode_query = facts(br"\ifhmode\else\fi\end");
    let tail_query = facts(br"\xdef\seen{\the\lastnodetype}\end");
    assert_eq!(ordinary, (0, 0), "ordinary delivery needs no executor fact");
    assert!(mode_query.0 > ordinary.0);
    assert_eq!(mode_query.1, 0, "mode enquiry does not walk the tail");
    assert!(tail_query.0 > ordinary.0);
    assert!(tail_query.1 > 0, "lastnodetype reads the live tail");
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
    let (count, observations) =
        observed_etex(br"\setbox32103=\hbox{}\count0=2147483647\advance\count0 by1\end");
    assert_eq!(count, i32::MAX);
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
    support::with_plain_universe(|stores| {
        let (alias, restored) = {
            let mut context = stores.command_context().expect("command admission");
            (
                context.intern_control_sequence("alias"),
                context.intern_control_sequence("restored"),
            )
        };
        let mut control = MainControl::tex82_initex(stores);
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
            match control.advance(stores).expect("alias recovery executes") {
                StepResult::Progress(MainControlStep::End)
                | StepResult::Progress(MainControlStep::EndOfInput) => break,
                StepResult::Progress(MainControlStep::Continue) => {}
                StepResult::Suspended(need) => panic!("unexpected resource suspension: {need:?}"),
            }
        }

        let context = stores.command_context().expect("command admission");
        assert_eq!(
            context.meaning(alias),
            ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::EndGroup
            ))
        );
        assert_eq!(context.meaning(restored), context.meaning(alias));
        assert_eq!(context.count(0).expect("count register"), 17);
        drop(context);
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
    });
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
        &["of struct `CommandFuel` are private"],
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
        &[
            "cannot construct `EngineCheckpoint<_>` with struct literal syntax due to private fields",
        ],
    );
}

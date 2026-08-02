use super::support::*;
use super::*;
use tex_command::{
    CommandProfile, FontResource, RegisteredSourceKind, SourceRegistration,
    install_tex82_expandable_primitives,
};
use tex_state::InputOpenState;
use tex_state::ids::ArenaRef;
use tex_state::node::Node;
use tex_state::scaled::Scaled;

#[derive(Default)]
struct ObservationRecorder(Vec<tex_command::CommandObservation>);

impl tex_command::CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: tex_command::CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn mode_nest_projects_conditional_predicates_across_transitions() {
    let mut nest = ModeNest::new();
    assert_eq!(
        nest.conditional_state().mode(),
        tex_command::ConditionalMode::Vertical
    );
    assert!(!nest.conditional_state().is_inner());
    nest.push(Mode::Horizontal).expect("test mode push");
    assert_eq!(
        nest.conditional_state().mode(),
        tex_command::ConditionalMode::Horizontal
    );
    assert!(!nest.conditional_state().is_inner());
    nest.push(Mode::Math).expect("test mode push");
    assert_eq!(
        nest.conditional_state().mode(),
        tex_command::ConditionalMode::Math
    );
    assert!(nest.conditional_state().is_inner());
    nest.pop().expect("leave math");
    nest.push(Mode::InternalVertical).expect("test mode push");
    assert_eq!(
        nest.conditional_state().mode(),
        tex_command::ConditionalMode::Vertical
    );
    assert!(nest.conditional_state().is_inner());

    let executor = Executor::from_nest(nest.clone());
    let mut capabilities = tex_command::CommandHostCapabilities::default();
    executor.install_command_capabilities(&mut capabilities);
}

#[test]
fn owned_execution_run_advances_through_explicit_phases() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut checkpoints = Vec::new();
    let mut run = ExecutionRun::new("owned-job");
    let cancellation = Cancellation::new();

    let first = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores).with_checkpoints(&mut checkpoints),
        &cancellation,
    );
    let ExecutionStepResult::Progress(first) = first else {
        panic!("job start must commit progress")
    };
    assert_eq!(first.next_step, ExecutionStep::MainControl);
    assert_eq!(first.checkpoints.len(), 1);
    assert_eq!(checkpoints.len(), 1);

    let second = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores),
        &cancellation,
    );
    let ExecutionStepResult::Progress(second) = second else {
        panic!("end of input must advance to finalization")
    };
    assert_eq!(second.next_step, ExecutionStep::Finalize);

    let complete = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores),
        &cancellation,
    );
    let ExecutionStepResult::Complete(stats) = complete else {
        panic!("finalization must complete the run")
    };
    assert_eq!(stats, ExecutionStats::default());
    assert_eq!(run.lifecycle(), ExecutionLifecycle::Complete);
    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Failed(ExecError::ExecutionAlreadyTerminated)
    ));
}

#[test]
fn effect_budget_failure_rolls_back_the_entire_candidate_step() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\message{not published}\\end"));
    let mut run = ExecutionRun::new("budgeted").with_budgets(ExecutionBudgets {
        effects: 0,
        ..ExecutionBudgets::default()
    });
    let cancellation = Cancellation::new();
    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation
        ),
        ExecutionStepResult::Progress(_)
    ));
    let input_before = input.summary();
    let state_before = stores.snapshot().state_hash();
    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation
        ),
        ExecutionStepResult::Failed(ExecError::ResourceBudgetExceeded {
            resource: "pending effects",
            limit: 0,
            attempted,
        }) if attempted > 0
    ));
    assert_eq!(input.summary(), input_before);
    assert_eq!(stores.snapshot().state_hash(), state_before);
    assert!(stores.world().effect_records().is_empty());
}

#[test]
fn named_checkpoint_preserves_future_execution_accounting() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut checkpoints = Vec::new();
    let mut run = ExecutionRun::new("accounted");
    let cancellation = Cancellation::new();
    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores).with_checkpoints(&mut checkpoints),
            &cancellation
        ),
        ExecutionStepResult::Progress(_)
    ));
    assert_eq!(checkpoints[0].budget_counters().committed_steps, 1);

    let mut executor = Executor::new();
    executor
        .restore_checkpoint(&mut input, &mut stores, &checkpoints[0], |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(""))
        })
        .expect("checkpoint restores");
    assert_eq!(executor.budget_counters(), checkpoints[0].budget_counters());
}

#[test]
fn owned_execution_run_amortizes_savepoints_across_bounded_command_chunks() {
    let command_count = 257;
    let mut source = "\\count0=0 ".repeat(command_count);
    source.push_str("\\end");
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut run = ExecutionRun::new("chunked-job");
    let cancellation = Cancellation::new();

    loop {
        match run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ) {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::Complete(stats) => {
                assert_eq!(stats.main_control_dispatches, command_count + 1);
                break;
            }
            other => panic!("chunked run failed: {other:?}"),
        }
    }

    // JobStart, two bounded main-control chunks, FinishEnd, and Finalize.
    assert_eq!(run.telemetry().advance_calls, 5);
}

#[test]
fn owned_execution_run_observes_cancellation_before_mutation() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut input = InputStack::new(MemoryInput::new("ignored"));
    let mut run = ExecutionRun::new("cancelled-job");
    let cancellation = Cancellation::new();
    let input_before = input.summary();
    cancellation.cancel();

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Cancelled
    ));
    assert_eq!(run.lifecycle(), ExecutionLifecycle::Cancelled);
    assert_eq!(input.summary(), input_before);
    assert!(stores.input_summary().is_empty());
}

#[test]
fn injected_interrupt_enters_and_leaves_pause_dialog_without_token_loss() {
    struct InterruptDuringScan {
        interrupt: PendingInterrupt,
        requested: bool,
    }

    impl tex_expand::InputResolver for InterruptDuringScan {
        fn open_input(
            &mut self,
            _input: &mut dyn tex_state::InputReadState,
            _name: &str,
            _request_index: u64,
        ) -> tex_expand::ResourceResult<Box<dyn tex_lex::InputSource>> {
            assert!(!self.requested, "child input is opened exactly once");
            self.requested = true;
            self.interrupt.request();
            Ok(tex_expand::ResourceLookup::Available(Box::new(
                MemoryInput::new("0 "),
            )))
        }
    }

    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(tex_state::InteractionMode::ErrorStop);
    stores
        .world_mut()
        .push_memory_terminal_line("")
        .expect("memory terminal accepts the instruction-dialog answer");
    stores
        .world_mut()
        .push_memory_terminal_line("")
        .expect("memory terminal accepts the deferred interrupt answer");
    let mut input = InputStack::new(MemoryInput::new(
        r"\count0=1 \advance\count\input child by1 \end",
    ));
    let mut run = ExecutionRun::new("injected-interrupt");
    let interrupt = run.pending_interrupt();
    let cancellation = Cancellation::new();
    let mut resolver = InterruptDuringScan {
        interrupt: interrupt.clone(),
        requested: false,
    };

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Progress(_)
    ));
    interrupt.request();
    loop {
        match run.step(
            &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
            &cancellation,
        ) {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::Complete(_) => break,
            other => panic!("interrupted run must resume and complete: {other:?}"),
        }
    }

    assert!(resolver.requested);
    assert!(!interrupt.is_pending());
    assert_eq!(stores.count(0), 2);
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::ErrorStop
    );
    let output = terminal_effect_text(&stores);
    assert_eq!(output.matches("! Interruption.").count(), 2, "{output}");
    assert_eq!(output.matches("? ").count(), 2, "{output}");
}

struct SuspendInputOnce {
    suspensions_remaining: usize,
    request_indices: Vec<u64>,
}

struct SuspendScannerInputOnce {
    suspended: bool,
    request_indices: Vec<u64>,
}

impl tex_expand::InputResolver for SuspendScannerInputOnce {
    fn open_input(
        &mut self,
        _input: &mut dyn tex_state::InputReadState,
        _name: &str,
        request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn tex_lex::InputSource>> {
        self.request_indices.push(request_index);
        if !self.suspended {
            self.suspended = true;
            return Ok(tex_expand::ResourceLookup::NeedResource(
                tex_expand::ResourceNeed::new(request_index),
            ));
        }
        Ok(tex_expand::ResourceLookup::Available(Box::new(
            MemoryInput::new("0 "),
        )))
    }
}

#[test]
fn resource_suspension_inside_integer_scanning_rolls_back_and_resumes() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\count0=7 \\count\\input child =42 \\end",
    ));
    let mut resolver = SuspendScannerInputOnce {
        suspended: false,
        request_indices: Vec::new(),
    };
    let mut run = ExecutionRun::new("nested-scanner-rollback");
    let cancellation = Cancellation::new();

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Progress(_)
    ));
    let universe_before = stores.snapshot().state_hash();
    let input_before = input.summary();
    let nest_before = run.nest().clone();

    let ExecutionStepResult::AwaitingResources(suspension) = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
        &cancellation,
    ) else {
        panic!("resource request nested in integer scanning must suspend")
    };
    assert_eq!(suspension.requests, vec![tex_expand::ResourceNeed::new(0)]);
    assert_eq!(suspension.blocked_step, ExecutionStep::MainControl);
    assert_eq!(stores.snapshot().state_hash(), universe_before);
    assert_eq!(input.summary(), input_before);
    assert_eq!(run.nest(), &nest_before);
    assert_eq!(stores.count(0), 0);

    loop {
        match run.step(
            &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
            &cancellation,
        ) {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::Complete(_) => break,
            other => panic!("nested scanner replay must complete, got {other:?}"),
        }
    }
    assert_eq!(resolver.request_indices, vec![0, 0]);
    assert_eq!(stores.count(0), 42);
}

#[test]
fn high_segment_pgfkeys_call_preserves_second_argument_and_retires_condition() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.intern("low-slot-csname");
    for index in 0..65_536_u32 {
        stores.intern(&format!("padding-{index}"));
    }
    let mut input = InputStack::new(MemoryInput::new(concat!(
        r"\catcode`\@=11 ",
        r"\long\def\pgfkeys@@set#1#2{\gdef\result{#2}}",
        r"\csname low-slot-csname\endcsname ",
        r"\iftrue\pgfkeys@@set{/pgfplots/table}{second-argument}\fi\end",
    )));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("high-segment macro call inside a conditional should complete");

    assert_eq!(macro_text(&stores, "result"), "second-argument");
    assert!(input.current_condition().is_none());
}

#[test]
fn high_segment_package_let_state_survives_lower_meaning_writes() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    for index in 0..65_536_u32 {
        stores.intern(&format!("package-padding-{index}"));
    }
    let mut input = InputStack::new(MemoryInput::new(concat!(
        r"\catcode`\@=11 ",
        r"\def\markbooktabs{\gdef\booktabsresult{B}}",
        r"\def\markvoidbox{\gdef\voidboxresult{V}}",
        r"\def\markenumitem{\gdef\enumitemresult{E}}",
        r"\def\packagecall{",
        r"\let\@BTswitch\markbooktabs",
        r"\let\voidb\markvoidbox",
        r"\let\enit@resuming\markenumitem",
        r"\let\relax\relax",
        r"\@BTswitch\voidb\enit@resuming}",
        r"\packagecall",
    )));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("high-segment package-local aliases should remain defined");

    assert_eq!(macro_text(&stores, "booktabsresult"), "B");
    assert_eq!(macro_text(&stores, "voidboxresult"), "V");
    assert_eq!(macro_text(&stores, "enumitemresult"), "E");
    let output = terminal_effect_text(&stores);
    assert!(!output.contains("Undefined control sequence"), "{output}");
}

#[test]
fn resource_suspension_rolls_back_groups_entered_by_blocked_dispatch() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\halign{\\input child#\\cr a\\cr}\\end"));
    let mut resolver = SuspendScannerInputOnce {
        suspended: false,
        request_indices: Vec::new(),
    };
    let mut run =
        ExecutionRun::new("nested-group-resource-rollback").with_cumulative_fuel_limit(100_000);
    let cancellation = Cancellation::new();

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Progress(_)
    ));
    let universe_before = stores.snapshot().state_hash();
    let input_before = input.summary();

    let ExecutionStepResult::AwaitingResources(suspension) = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
        &cancellation,
    ) else {
        panic!("resource request after alignment group entry must suspend")
    };
    assert_eq!(suspension.requests, vec![tex_expand::ResourceNeed::new(0)]);
    assert_eq!(stores.snapshot().state_hash(), universe_before);
    assert_eq!(input.summary(), input_before);
    assert_eq!(tex_state::ExpansionState::execution_group_depth(&stores), 0);

    loop {
        match run.step(
            &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
            &cancellation,
        ) {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::Complete(_) => break,
            other => panic!("alignment replay must complete, got {other:?}"),
        }
    }
    assert_eq!(resolver.request_indices, vec![0, 0]);
}

#[test]
fn resource_suspension_preserves_local_box_state_only_until_box_exit() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\def\\expected{alive}\\setbox0=\\vbox{\\def\\currbox{alive}\\input child \\
         \\ifx\\currbox\\expected\\global\\count0=1\\else\\global\\count0=2\\fi}\\
         \\ifx\\currbox\\undefined\\global\\count1=1\\else\\global\\count1=2\\fi\\end",
    ));
    let mut resolver = SuspendScannerInputOnce {
        suspended: false,
        request_indices: Vec::new(),
    };
    let mut run =
        ExecutionRun::new("local-box-state-resource-rollback").with_cumulative_fuel_limit(100_000);
    let cancellation = Cancellation::new();

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Progress(_)
    ));
    let universe_before = stores.snapshot().state_hash();
    let currbox = stores.intern("currbox").symbol();

    let ExecutionStepResult::AwaitingResources(suspension) = run.step(
        &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
        &cancellation,
    ) else {
        panic!("resource request inside the open vbox must suspend")
    };
    assert_eq!(suspension.requests, vec![tex_expand::ResourceNeed::new(0)]);
    assert_eq!(stores.snapshot().state_hash(), universe_before);
    assert_eq!(tex_state::ExpansionState::execution_group_depth(&stores), 0);
    assert_eq!(
        stores.meaning(currbox),
        tex_state::meaning::Meaning::Undefined
    );

    loop {
        match run.step(
            &mut ExecutionServices::new(&mut input, &mut stores).with_input_resolver(&mut resolver),
            &cancellation,
        ) {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::Complete(_) => break,
            other => panic!("vbox replay must complete, got {other:?}"),
        }
    }

    assert_eq!(resolver.request_indices, vec![0, 0]);
    assert_eq!(
        stores.count(0),
        1,
        "local definition must survive the replay"
    );
    assert_eq!(
        stores.count(1),
        1,
        "local definition must end with the vbox group"
    );
    assert_eq!(
        stores.meaning(currbox),
        tex_state::meaning::Meaning::Undefined
    );
}

impl tex_expand::InputResolver for SuspendInputOnce {
    fn open_input(
        &mut self,
        _input: &mut dyn tex_state::InputReadState,
        _name: &str,
        request_index: u64,
    ) -> tex_expand::ResourceResult<Box<dyn tex_lex::InputSource>> {
        self.request_indices.push(request_index);
        if self.suspensions_remaining > 0 {
            self.suspensions_remaining -= 1;
            return Ok(tex_expand::ResourceLookup::NeedResource(
                tex_expand::ResourceNeed::new(request_index),
            ));
        }
        Ok(tex_expand::ResourceLookup::Available(Box::new(
            MemoryInput::new("\\count0=42"),
        )))
    }
}

#[test]
fn resource_suspension_rolls_back_the_aggregate_step_and_replays_stably() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 \\input child"));
    let mut resolver = SuspendInputOnce {
        suspensions_remaining: 2,
        request_indices: Vec::new(),
    };
    let mut recorder = TestRecorder::default();
    let mut run = ExecutionRun::new("rollback-job");
    let cancellation = Cancellation::new();

    assert!(matches!(
        run.step(
            &mut ExecutionServices::new(&mut input, &mut stores),
            &cancellation,
        ),
        ExecutionStepResult::Progress(_)
    ));

    let suspension = loop {
        let universe_before = stores.snapshot().state_hash();
        let input_before = input.summary();
        let nest_before = run.nest().clone();
        let recorder_before = recorder.meanings.len();
        let blocked_step = run.next_step();
        let result = run.step(
            &mut ExecutionServices::new(&mut input, &mut stores)
                .with_input_resolver(&mut resolver)
                .recording(&mut recorder),
            &cancellation,
        );
        if let ExecutionStepResult::AwaitingResources(suspension) = result {
            assert_eq!(stores.snapshot().state_hash(), universe_before);
            assert_eq!(input.summary(), input_before);
            assert_eq!(run.nest(), &nest_before);
            assert_eq!(run.next_step(), blocked_step);
            assert_eq!(recorder.meanings.len(), recorder_before);
            break suspension;
        }
        assert!(matches!(result, ExecutionStepResult::Progress(_)));
    };
    assert_eq!(run.lifecycle(), ExecutionLifecycle::Awaiting);
    assert_eq!(suspension.requests.len(), 1);
    assert_eq!(suspension.blocked_step, ExecutionStep::MainControl);

    let first_request_index = suspension.requests[0].request_index();
    let mut last_serial = suspension.serial;
    loop {
        let recorder_before = recorder.meanings.len();
        let result = run.step(
            &mut ExecutionServices::new(&mut input, &mut stores)
                .with_input_resolver(&mut resolver)
                .recording(&mut recorder),
            &cancellation,
        );
        match result {
            ExecutionStepResult::Progress(_) => {}
            ExecutionStepResult::AwaitingResources(repeated) => {
                assert_eq!(repeated.requests[0].request_index(), first_request_index);
                assert!(repeated.serial > last_serial);
                assert_eq!(recorder.meanings.len(), recorder_before);
                last_serial = repeated.serial;
            }
            ExecutionStepResult::Complete(_) => break,
            other => panic!("replayed run must complete, got {other:?}"),
        }
    }
    assert_eq!(
        resolver.request_indices,
        vec![
            first_request_index,
            first_request_index,
            first_request_index
        ]
    );
    assert_eq!(stores.count(0), 42);
}

#[test]
fn unsupported_typesetting_diagnostic_names_control_sequence() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let special = stores.intern("special");
    let error = ExecError::UnimplementedTypesetting {
        mode: Mode::DisplayMath,
        token: Token::Cs(special.symbol()),
        origin: OriginId::UNKNOWN,
        operation: "math primitive",
    };

    let rendered = error.format_with_provenance(&stores);

    assert!(rendered.contains("for token \\special"));
    assert!(!rendered.contains("Symbol("));
}

#[test]
fn detached_page_episode_replays_before_output_fire_up() {
    fn page_input(stores: &mut Universe, perturb_allocations: bool) -> OriginId {
        if perturb_allocations {
            let _ = stores.intern_token_list(&[Token::Char {
                ch: 'X',
                cat: Catcode::Other,
            }]);
            let _ = stores.source_origin(tex_state::input::SourceId::new(99), 9, 1, 10);
        }
        let origin = stores.source_origin(tex_state::input::SourceId::new(1), 4, 1, 5);
        let mark = stores.intern_token_list(&[Token::Char {
            ch: 'M',
            cat: Catcode::Other,
        }]);
        stores.append_page_contribution(Node::Mark {
            class: 0,
            tokens: mark,
        });
        let children = stores.freeze_node_list(&[Node::Char {
            font: tex_state::font::NULL_FONT,
            ch: 'A',
            origin,
        }]);
        stores.append_page_contribution(Node::HList(tex_state::node::BoxNode::new(
            tex_state::node::BoxNodeFields {
                width: Scaled::from_raw(10),
                height: Scaled::from_raw(20),
                depth: Scaled::from_raw(3),
                shift: Scaled::from_raw(0),
                box_lr: tex_state::node::BoxLr::Normal,
                glue_set: tex_state::scaled::GlueSetRatio::ZERO,
                glue_sign: tex_state::node::Sign::Normal,
                glue_order: tex_state::glue::Order::Normal,
                children,
            },
        )));
        stores.append_page_contribution(Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(20)),
            depth: Some(Scaled::from_raw(3)),
        });
        origin
    }

    let mut first = crate::test_harness::universe_with_plain_catcodes();
    first.enable_pure_memo(tex_state::PureMemoConfig::default());
    first.enable_page_memo();
    page_input(&mut first, false);
    crate::page_builder::build_page(&mut first).expect("cold page episode");
    let expected = first.page_memo_fingerprint();
    let runtime = first.take_pure_memo_runtime();

    let mut second = crate::test_harness::universe_with_plain_catcodes();
    let current_origin = page_input(&mut second, true);
    second.install_pure_memo_runtime(runtime);
    crate::page_builder::build_page(&mut second).expect("replayed page episode");

    let memo = second.pure_memo_stats();
    assert_eq!(second.page_memo_fingerprint(), expected);
    let replayed_box = second
        .current_page_nodes()
        .into_iter()
        .find_map(|node| match node {
            Node::HList(box_node) => Some(box_node),
            _ => None,
        })
        .expect("replayed page contains the valid box contribution");
    assert!(
        second
            .nodes(replayed_box.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::Char { origin, .. } if *origin == current_origin))
    );
    assert!(memo.page_hits >= 1, "{memo:?}");
    assert!(memo.page_contributions_skipped >= 2, "{memo:?}");
}

#[test]
fn page_episode_tracks_insertion_registers_and_detaches_insert_content() {
    const CLASS: u16 = 7;
    fn insertion_input(stores: &mut Universe, count: i32, perturb_allocations: bool) {
        stores.set_count(CLASS, count);
        stores.set_dimen(CLASS, Scaled::from_raw(1_000_000));
        if perturb_allocations {
            let _ = stores.freeze_node_list(&[Node::Penalty(123)]);
        }
        let content = stores.freeze_node_list(&[Node::Rule {
            width: Some(Scaled::from_raw(10)),
            height: Some(Scaled::from_raw(20)),
            depth: Some(Scaled::from_raw(3)),
        }]);
        stores.append_page_contribution(Node::Ins {
            class: CLASS,
            size: Scaled::from_raw(23),
            split_top_skip: stores.glue_param(GlueParam::SPLIT_TOP_SKIP),
            split_max_depth: Scaled::from_raw(100),
            floating_penalty: 17,
            content,
        });
    }

    let mut first = crate::test_harness::universe_with_plain_catcodes();
    first.enable_pure_memo(tex_state::PureMemoConfig::default());
    first.enable_page_memo();
    insertion_input(&mut first, 1_000, false);
    crate::page_builder::build_page(&mut first).expect("cold insertion episode");
    let expected = first.page_memo_fingerprint();
    let runtime = first.take_pure_memo_runtime();

    let mut same = crate::test_harness::universe_with_plain_catcodes();
    insertion_input(&mut same, 1_000, true);
    same.install_pure_memo_runtime(runtime);
    crate::page_builder::build_page(&mut same).expect("replayed insertion episode");
    let after_hit = same.pure_memo_stats();
    assert_eq!(same.page_memo_fingerprint(), expected);
    assert_eq!(after_hit.page_hits, 1, "{after_hit:?}");

    let runtime = same.take_pure_memo_runtime();
    let mut changed = crate::test_harness::universe_with_plain_catcodes();
    insertion_input(&mut changed, 500, true);
    changed.install_pure_memo_runtime(runtime);
    crate::page_builder::build_page(&mut changed).expect("changed insertion episode");
    let after_miss = changed.pure_memo_stats();
    assert_eq!(after_miss.page_hits, after_hit.page_hits, "{after_miss:?}");
    assert!(
        after_miss.page_lookups > after_hit.page_lookups,
        "{after_miss:?}"
    );
}

#[test]
fn finalized_shipout_artifact_reuses_while_output_routine_still_executes() {
    let source = "\\output={\\global\\advance\\count50 by1 \
        \\shipout\\box255}\\topskip=0pt \
        \\setbox0=\\hbox{\\vrule width1pt height1pt} \
        \\copy0\\penalty-10000 \\copy0\\penalty-10000\\end";
    let run = |memoized: bool| {
        let mut stores = crate::test_harness::universe_with_plain_catcodes();
        tex_expand::install_expandable_primitives(&mut stores);
        install_unexpandable_primitives(&mut stores);
        if memoized {
            stores.enable_pure_memo(tex_state::PureMemoConfig::default());
            stores.enable_shipout_memo();
        }
        let mut input = InputStack::new(MemoryInput::new(source));
        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("repeated output routine");
        let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
        for page in &stats.dvi_pages {
            writer.write_page_plan(page).expect("DVI page");
        }
        (
            writer.finish().expect("DVI finish"),
            stores.count(50),
            stores.pure_memo_stats(),
        )
    };

    let (cold_dvi, cold_count, _) = run(false);
    let (memo_dvi, memo_count, memo) = run(true);
    assert_eq!(memo_dvi, cold_dvi);
    assert_eq!(memo_count, cold_count);
    assert_eq!(memo_count, 2, "output routine must execute on every page");
    assert!(memo.shipout_hits >= 1, "{memo:?}");
}

#[test]
fn deferred_write_shipouts_are_counted_barriers_and_expand_each_time() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.enable_shipout_memo();
    let source = "\\setbox0=\\hbox{\\write16{p:\\the\\count1}} \
        \\count1=1\\shipout\\copy0 \\count1=2\\shipout\\copy0\\end";
    let mut input = InputStack::new(MemoryInput::new(source));
    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("deferred writes execute");

    assert_eq!(stats.shipped_artifacts.len(), 2);
    assert_ne!(stats.shipped_artifacts[0], stats.shipped_artifacts[1]);
    let memo = stores.pure_memo_stats();
    assert!(memo.shipout_barriers >= 2, "{memo:?}");
    assert_eq!(memo.shipout_hits, 0, "{memo:?}");
}

#[test]
fn expl3_primitive_alias_pattern_consumes_its_conditional_terminator() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    tex_expand::install_latex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let source = r"\long\def\useii#1#2{#2}
\long\def\usenone#1{}
\long\def\primitive#1#2{\ifdefined#1\expandafter\useii\fi\usenone{\global\let#2#1}}
\primitive\expanded\alias
\primitive\missing\missingalias
\end";
    let mut input = InputStack::new(MemoryInput::new(source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("expl3 primitive aliases execute");

    let alias = stores.symbol("alias").expect("defined primitive alias");
    let expanded = stores.intern("expanded");
    assert_eq!(stores.meaning(alias), stores.meaning(expanded));
    let missing = stores.intern("missingalias");
    assert_eq!(stores.meaning(missing), Meaning::Undefined);
    assert!(!terminal_effect_text(&stores).contains("Extra \\fi"));
}

#[test]
fn latex_token_loop_preserves_an_enclosing_conditional_frame() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let source = r"\def\space{ }
\def\nil{\nil}\def\nnil{\nil}
\long\def\fornoop#1\stop#2#3{}
\def\tfor#1:={\tforaux#1 }
\long\def\tforaux#1#2\do#3{\def\fortmp{#2}\ifx\fortmp\space\else\tforloop#2\nil\nil\stop#1{#3}\fi}
\long\def\tforloop#1#2\stop#3#4{\def#3{#1}\ifx#3\nnil\expandafter\fornoop\else#4\relax\expandafter\tforloop\fi#2\stop#3{#4}}
\def\outermarker{}
\ifdefined\outermarker
  \tfor\item:=ABC\do{\ifx\item\missing\fi}
\fi
\end";
    let mut input = InputStack::new(MemoryInput::new(source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("LaTeX token loop executes");

    assert!(!terminal_effect_text(&stores).contains("Extra \\fi"));
}

#[test]
fn deferred_write_preserves_unexpanded_tokens_through_shipout_collection() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let source = r"\def\payload{\endgroup \fi \bgroup \iffalse \else}
\setbox0=\hbox{\write16{\unexpanded\expandafter{\payload}}}
\shipout\box0\end";
    let mut input = InputStack::new(MemoryInput::new(source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("deferred write expands without executing unexpanded conditionals");

    assert!(!terminal_effect_text(&stores).contains("Extra \\fi"));
}

#[test]
fn trailing_hash_brace_is_appended_to_the_macro_replacement() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let source = r"\def\grab#1#{\message{ARG=[\detokenize{#1}]}}\grab #1 {closed}\end";
    let mut input = InputStack::new(MemoryInput::new(source));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("trailing hash-brace macro executes");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("ARG=[##1 ]"), "{output}");
    assert!(!output.contains("Too many }'s"), "{output}");
}

#[test]
fn nest_push_pop_and_summary_cover_all_modes() {
    let mut nest = ModeNest::new();
    for mode in [
        Mode::InternalVertical,
        Mode::Horizontal,
        Mode::RestrictedHorizontal,
        Mode::Math,
        Mode::DisplayMath,
    ] {
        nest.push(mode).expect("test mode push");
    }

    assert_eq!(nest.depth(), 6);
    assert_eq!(nest.current_mode(), Mode::DisplayMath);

    let summary = nest.summary();
    let restored = ModeNest::from_summary(summary.clone()).expect("valid summary");
    assert_eq!(restored.summary(), summary);

    assert_eq!(nest.pop().expect("display math").mode(), Mode::DisplayMath);
    assert_eq!(nest.pop().expect("math").mode(), Mode::Math);
    assert_eq!(
        nest.pop().expect("restricted h").mode(),
        Mode::RestrictedHorizontal
    );
    assert_eq!(nest.pop().expect("h").mode(), Mode::Horizontal);
    assert_eq!(
        nest.pop().expect("internal v").mode(),
        Mode::InternalVertical
    );
    assert_eq!(
        nest.pop().expect_err("base cannot pop").to_string(),
        "cannot pop the base vertical mode level"
    );
}

#[test]
fn engine_checkpoint_restores_input_modes_and_universe_atomically() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut executor = Executor::new();
    stores.set_count(3, 41);
    let mut checkpoints = Vec::new();
    executor
        .run_with_context_and_checkpoints(
            &mut input,
            &mut stores,
            &mut crate::ExecutionContext::new("texput"),
            &mut checkpoints,
        )
        .expect("empty job");
    let checkpoint = &checkpoints[0];
    assert_eq!(
        checkpoint.schema_version(),
        ENGINE_CHECKPOINT_SCHEMA_VERSION
    );

    executor
        .nest_mut()
        .push(Mode::Horizontal)
        .expect("test mode push");
    stores.set_count(3, 99);
    executor
        .restore_checkpoint(&mut input, &mut stores, checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(""))
        })
        .expect("published aggregate checkpoint");

    assert_eq!(stores.count(3), 41);
    assert_eq!(executor.nest().current_mode(), Mode::Vertical);
    assert_eq!(input.summary(), *checkpoint.input_summary());
}

#[test]
fn engine_session_publishes_named_outer_paragraph_boundary() {
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\f x\\par"));
    let mut checkpoints = Vec::new();
    Executor::new()
        .run_with_context_and_checkpoints(
            &mut input,
            &mut stores,
            &mut crate::ExecutionContext::new("texput"),
            &mut checkpoints,
        )
        .expect("paragraph job");
    assert_eq!(checkpoints[0].boundary(), EngineBoundary::JobStart);
    assert!(
        checkpoints
            .iter()
            .any(|checkpoint| checkpoint.boundary() == EngineBoundary::OuterParagraphEnd)
    );
}

#[test]
fn outer_paragraph_checkpoint_retains_survivor_pins_for_mode_restore() {
    let mut stores = support::stores_with_fonts();
    let source = "\\font\\f=cmr10 \\f \\setbox0=\\hbox{Q}\\copy0 X\\par";
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut checkpoints = Vec::new();
    let mut executor = Executor::new();
    executor
        .run_with_context_and_checkpoints(
            &mut input,
            &mut stores,
            &mut crate::ExecutionContext::new("texput"),
            &mut checkpoints,
        )
        .expect("paragraph with copied survivor box executes");
    let checkpoint = checkpoints
        .iter()
        .find(|checkpoint| checkpoint.boundary() == EngineBoundary::OuterParagraphEnd)
        .expect("outer paragraph checkpoint")
        .clone();
    let expected_mode_hash = checkpoint.mode_summary().semantic_fingerprint(&stores);

    let replacement = stores.freeze_node_list(&[Node::Kern {
        amount: Scaled::from_raw(0),
        kind: tex_state::node::KernKind::Explicit,
    }]);
    stores.set_box_reg_global(0, replacement);
    executor
        .restore_checkpoint(&mut input, &mut stores, &checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(source))
        })
        .expect("retained checkpoint restores survivor-backed mode roots");

    assert_eq!(
        executor.nest().summary().semantic_fingerprint(&stores),
        expected_mode_hash
    );
}

#[test]
fn shipout_checkpoint_restores_after_nested_work_has_unwound() {
    let source = "\\font\\f=cmr10 \\f \\setbox0=\\hbox{\\shipout\\hbox{A}B}\\end";
    let mut stores = support::stores_with_fonts();
    let mut input = InputStack::new(MemoryInput::new(source));
    let mut executor = Executor::new();
    let mut checkpoints = Vec::new();
    executor
        .run_with_context_and_checkpoints(
            &mut input,
            &mut stores,
            &mut crate::ExecutionContext::new("texput"),
            &mut checkpoints,
        )
        .expect("nested shipout job");
    let checkpoint = checkpoints
        .iter()
        .find(|checkpoint| checkpoint.boundary() == EngineBoundary::ShipoutComplete)
        .expect("outer executor publishes shipout completion");
    assert_eq!(checkpoint.mode_summary().levels().len(), 1);

    stores.set_count(7, 99);
    executor
        .restore_checkpoint(&mut input, &mut stores, checkpoint, |_, _, _| {
            Ok::<_, ()>(MemoryInput::new(source))
        })
        .expect("shipout checkpoint restores");
    assert_eq!(stores.count(7), 0);
    assert_eq!(executor.nest().current_mode(), Mode::Vertical);
}

#[test]
fn successful_execution_publishes_the_exact_final_input_cursor() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut executor = Executor::new();

    executor.run(&mut input, &mut stores).expect("empty run");

    assert_eq!(stores.input_summary(), &input.summary());
}

#[test]
fn virtualized_execution_trace_is_opt_in_and_semantically_neutral() {
    fn run(tracing: bool) -> (Universe, ExecutionStats) {
        let mut stores = support::stores_with_fonts();
        stores.world_mut().set_execution_tracing(tracing);
        let mut input = InputStack::new(MemoryInput::new(
            "\\font\\f=cmr10 \\f directtext \\def\\x{office-A}\\x\\par \\setbox0=\\hbox{\\x}",
        ));
        let stats = Executor::new()
            .run(&mut input, &mut stores)
            .expect("trace comparison source executes");
        (stores, stats)
    }

    let (mut ordinary, ordinary_stats) = run(false);
    let (mut traced, traced_stats) = run(true);
    assert!(ordinary_stats.source_text_span_tokens > 0);
    assert_eq!(traced_stats.source_text_span_tokens, 0);
    assert!(ordinary.world().execution_trace().is_empty());
    assert!(!traced.world().execution_trace().is_empty());
    assert!(
        traced
            .world()
            .execution_trace()
            .iter()
            .any(|event| event.subsystem() == "executor")
    );
    assert_eq!(
        ordinary.world().effect_records(),
        traced.world().effect_records()
    );
    assert_eq!(
        ordinary.snapshot().state_hash(),
        traced.snapshot().state_hash()
    );
}

#[test]
fn engine_snapshot_queries_are_backed_by_current_nest_level() {
    let mut executor = Executor::new();
    let stores = crate::test_harness::universe_with_plain_catcodes();
    let mut context = crate::ExecutionContext::new("texput");
    crate::executor::sync_engine_state(&mut context, executor.nest(), &stores);
    assert_eq!(context.engine.mode, tex_expand::EngineMode::Vertical);
    assert!(!context.engine.is_inner_mode);

    executor
        .nest_mut()
        .push(Mode::RestrictedHorizontal)
        .expect("test mode push");
    crate::executor::sync_engine_state(&mut context, executor.nest(), &stores);
    assert_eq!(context.engine.mode, tex_expand::EngineMode::Horizontal);
    assert!(context.engine.is_inner_mode);

    executor
        .nest_mut()
        .push(Mode::DisplayMath)
        .expect("test mode push");
    crate::executor::sync_engine_state(&mut context, executor.nest(), &stores);
    assert_eq!(context.engine.mode, tex_expand::EngineMode::Math);
    assert!(!context.engine.is_inner_mode);
}

#[test]
fn outer_lastskip_uses_page_glue_only_when_the_contribution_list_is_empty() {
    let executor = Executor::new();
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let page_glue = stores.intern_glue(GlueSpec {
        width: tex_state::scaled::Scaled::from_raw(7 * tex_state::scaled::Scaled::UNITY),
        ..GlueSpec::ZERO
    });
    stores.update_page_last_from_node(&Node::Glue {
        spec: page_glue,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    });
    let mut context = crate::ExecutionContext::new("texput");

    crate::executor::sync_engine_state(&mut context, executor.nest(), &stores);
    assert_eq!(context.engine.last_skip, stores.glue(page_glue));

    stores.append_page_contribution(Node::Rule {
        width: None,
        height: None,
        depth: None,
    });
    crate::executor::sync_engine_state(&mut context, executor.nest(), &stores);
    assert_eq!(context.engine.last_skip, GlueSpec::ZERO);
}

#[test]
fn dispatch_relax_continues_without_state_mutation() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.stop_at_end_of_input();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\relax".to_vec(),
        ))
        .expect("register relax source");

    assert_eq!(
        control.step(&mut stores).expect("relax dispatch"),
        MainControlStep::Continue
    );
    assert_eq!(control.current_mode(), Mode::Vertical);
    assert!(control.current_list().nodes().is_empty());
}

#[test]
fn dump_marks_format_stop_and_stops_before_following_input() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_page_dimension(tex_state::page::PageDimension::Goal, Scaled::from_raw(123));
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\dump\dump".to_vec(),
        ))
        .expect("register dump source");

    assert_eq!(
        control
            .step(&mut stores)
            .expect("dump stops canonical control"),
        MainControlStep::End
    );
    assert!(control.dumped_format());
    assert!(stores.input_summary().is_empty());
    stores
        .dump_format()
        .expect("dump should leave a quiescent serializable format boundary");
}

#[test]
fn incomplete_delimited_macro_at_root_eof_recovers_once_with_par() {
    let stores = run_canonical_tex82(r"\def\runaway#1\stop{}\runaway missing");

    let transcript = terminal_effect_text(&stores);
    let heading = transcript.find("Runaway argument?").expect(&transcript);
    let partial = transcript[heading..].find("missing").expect(&transcript) + heading;
    let report = transcript
        .find("File ended while scanning use of \\runaway")
        .expect(&transcript);
    assert!(heading < partial && partial < report, "{transcript}");
    assert!(
        transcript.contains("File ended while scanning use of \\runaway"),
        "{transcript}"
    );
    assert!(stores.input_summary().is_empty());
}

#[test]
fn incomplete_delimited_macro_at_outer_token_recovers_once_with_par() {
    let stores = run_canonical_tex82(
        r"\outer\def\forbidden{}\def\runaway#1\stop{}\runaway missing\forbidden\end",
    );

    let transcript = terminal_effect_text(&stores);
    let heading = transcript.find("Runaway argument?").expect(&transcript);
    let partial = transcript[heading..].find("missing").expect(&transcript) + heading;
    let report = transcript
        .find("Forbidden control sequence found while scanning use of \\runaway")
        .expect(&transcript);
    assert!(heading < partial && partial < report, "{transcript}");
    assert_eq!(
        transcript.matches("Runaway argument?").count(),
        1,
        "{transcript}"
    );
    assert_eq!(
        transcript
            .matches("Forbidden control sequence found while scanning use of \\runaway")
            .count(),
        1,
        "{transcript}"
    );
}

#[test]
fn incomplete_delimited_macro_from_inserted_replay_retains_clean_eof_recovery() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.stop_at_end_of_input();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\def\runaway#1\stop{}\runaway X".to_vec(),
        ))
        .expect("register runaway replay source");
    for _ in 0..64 {
        if control
            .step(&mut stores)
            .expect("inserted recovery preserves clean EOF")
            == MainControlStep::EndOfInput
        {
            assert!(stores.input_summary().is_empty());
            return;
        }
    }
    panic!("canonical inserted recovery did not reach clean EOF");
}

#[test]
fn format_loaded_job_replays_everyjob_before_root_input() {
    let mut initex = crate::test_harness::universe_with_plain_catcodes();
    let mut initex_control = CanonicalMainControl::tex82_initex(&mut initex);
    run_registered_canonical_tex82(
        &mut initex_control,
        &mut initex,
        r"\everyjob{\count0=42\message{EVERYJOB}}\dump",
    );
    let format = initex.dump_format().expect("dump format");

    let mut stores =
        Universe::from_format(tex_state::World::memory(), &format).expect("load format");
    let mut control = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    run_registered_canonical_tex82(
        &mut control,
        &mut stores,
        r"\message{COUNT=\the\count0}\end",
    );

    let terminal = terminal_effect_text(&stores);
    let every_job = terminal.find("EVERYJOB").expect("everyjob message");
    let root = terminal.find("COUNT=42").expect("root-input message");
    assert!(every_job < root, "{terminal:?}");
}

#[test]
fn format_loaded_message_keeps_the_token_register_output_unexpanded() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut initial_control = CanonicalMainControl::tex82_initex(&mut stores);
    run_registered_canonical_tex82(
        &mut initial_control,
        &mut stores,
        r"\toksdef\tokens=256
          \def\settest#1{\let\test= }\settest. \relax
          \def\a#1{\ifcat#1 \message\ifx#1 {\iffalse\fi\the\tokens\fi\fi}}
          \tokens={\a\test}\dump",
    );
    let format = stores.dump_format().expect("dump reduced TRIP format");

    let mut loaded = Universe::from_format(tex_state::World::memory(), &format)
        .expect("load reduced TRIP format");
    let mut loaded_control = CanonicalMainControl::with_profile(CommandProfile::TEX82);
    run_registered_canonical_tex82(&mut loaded_control, &mut loaded, "\\the\\tokens\\end");

    assert!(terminal_effect_text(&loaded).contains("\\a \\test"));
}

#[test]
fn immediate_puts_back_non_io_extension_tokens() {
    let stores = run_canonical_tex82(r"\immediate\catcode`A=12\message{C=\the\catcode`A}\end");

    assert!(terminal_effect_text(&stores).contains("C=12"));
}

#[test]
fn interaction_mode_primitives_update_checkpointed_engine_state() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let snapshot = stores.snapshot();
    run_registered_canonical_tex82(&mut control, &mut stores, r"\nonstopmode\end");
    assert_eq!(stores.interaction_mode(), InteractionMode::Nonstop);

    stores.rollback(&snapshot);
    assert_eq!(stores.interaction_mode(), InteractionMode::ErrorStop);
}

#[test]
fn message_applies_newlinechar_to_raw_expanded_character_tokens() {
    // tex.web's issue_message builds a string with selector=new_string, so
    // character tokens remain raw until newlinechar is applied.
    let stores = run_canonical_tex82("\\newlinechar=10\\message{LEFT^^JRIGHT}\\end");

    assert!(terminal_effect_text(&stores).contains("LEFT\nRIGHT"));
}

#[test]
fn bare_internal_quantity_reports_illegal_mode_and_continues() {
    let stores = run_canonical_tex82(r"\badness\message{continued}\end");

    let output = terminal_effect_text(&stores);
    assert!(output.contains("You can't use `\\badness' in vertical mode"));
    assert!(output.contains("continued"));
}

#[test]
fn inputlineno_reports_current_physical_source_line() {
    let stores = run_canonical_tex82("\\relax\n\\message{L=\\the\\inputlineno}\\end");

    assert!(terminal_effect_text(&stores).contains("L=2"));
}

#[test]
fn setlanguage_appends_normalized_language_whatsit_in_hmode() {
    let stores =
        run_canonical_tex82(r"\lefthyphenmin=0 \righthyphenmin=99 \setbox0=\hbox{\setlanguage7}");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(box_node.children).testing_decoded(),
        [tex_state::node::Node::Whatsit(
            tex_state::node::Whatsit::Language {
                language: 7,
                left_hyphen_min: 1,
                right_hyphen_min: 63,
            }
        )]
    ));
}

#[test]
fn internal_integer_assignment_leaves_following_expandafter_unexpanded() {
    let source = r#"
        \catcode`@=11
        \countdef\m@ne=22 \m@ne=-1
        \countdef\count@=255
        {\uccode`1=`i \uccode`2=`f \uppercase{\gdef\if@12{\message{ok}}}}
        \escapechar\m@ne
        \expandafter\if@\string\ifplain
        \end
    "#;
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical source");
    let mut observations = CoreObservationRecorder(Vec::new());
    let mut terminated_at = None;
    for step in 0..64 {
        let result = control
            .step_with_observer(&mut stores, &mut observations)
            .expect("canonical step");
        if matches!(result, MainControlStep::End | MainControlStep::EndOfInput) {
            terminated_at = Some(step);
            break;
        }
    }
    assert!(
        terminated_at.is_some(),
        "canonical source did not terminate in 64 steps"
    );
    let raw_expandafter = observations.0.iter().position(|event| {
        matches!(event, tex_command::CommandObservation::Command(command)
            if command.boundary == tex_command::CommandDeliveryBoundary::Raw
                && command.spelling == tex_command::ObservedToken::ControlSequence("expandafter".into()))
    });
    assert!(
        raw_expandafter.is_some(),
        "following expandafter remains available to main control"
    );
    assert!(
        !observations.0.iter().any(|event| {
            matches!(event, tex_command::CommandObservation::Command(command)
                if command.boundary == tex_command::CommandDeliveryBoundary::Expanded
                    && command.spelling == tex_command::ObservedToken::ControlSequence("expandafter".into()))
        }),
        "assignment scanning must not publish the following expandafter as expanded"
    );
    assert!(terminal_effect_text(&stores).contains("OK"));
}

struct CoreObservationRecorder(Vec<tex_command::CommandObservation>);

impl tex_command::CommandObserver for CoreObservationRecorder {
    fn committed(&mut self, observation: tex_command::CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn uppercase_expands_tokens_until_its_opening_brace() {
    let stores =
        run_canonical_tex82("\\def\\body{\\message{ok}}\\uppercase\\expandafter{\\body}\\end");

    assert!(terminal_effect_text(&stores).contains("OK"));
}

#[test]
fn uppercase_retargets_active_character_definitions() {
    let stores = run_canonical_tex82(
        "\\catcode126=13 \\uccode126=239 \\uppercase{\\gdef~{\\message{ok}}}\\uppercase{~}\\end",
    );

    assert!(terminal_effect_text(&stores).contains("OK"));
}

#[test]
fn protected_active_macro_expands_from_classic_utf8_input() {
    let stores = run_canonical_etex(
        "\\catcode126=13 \\catcode239=13 \\catcode172=13 \\catcode128=13 \\uccode126=239 \\uppercase{\\protected\\def~#1#2{\\message{OK}}}ﬀ\\end",
    );

    assert!(terminal_effect_text(&stores).contains("OK"));
}

#[test]
fn dispatch_character_hits_loud_typesetting_stub() {
    let stores = run_canonical_tex82(r"\setbox0=\hbox{x}\end");

    assert!(stores.box_reg(0).is_some());
}

#[test]
fn dispatch_undefined_control_sequence_reports_and_continues() {
    let stores = run_canonical_tex82(r"\undefined\message{continued}\end");
    let output = terminal_effect_text(&stores);

    // tex.web §370's message never names the control sequence; §82's context
    // display is what ends with it.
    assert!(output.contains("! Undefined control sequence."));
    assert!(output.contains("continued"));
}

#[test]
fn edef_reports_undefined_control_sequence_and_completes_definition() {
    let stores = run_canonical_tex82("\\edef\\foo{a\\missing b}\\message{RESULT=\\foo}\\end");

    let output = terminal_effect_text(&stores);
    // tex.web §370's message names no control sequence; §82's context display
    // is what identifies it, and that goes to the transcript alone.
    assert!(output.contains("Undefined control sequence"), "{output}");
    assert!(output.contains("RESULT=ab"));
}

#[test]
fn execution_error_capture_retains_macro_trace_after_frame_pop() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let mut recorder = ObservationRecorder::default();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\def\m{\bad}\m\relax\end".to_vec(),
        ))
        .expect("register canonical source");
    for _ in 0..32 {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut recorder)
                .expect("canonical observed step"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let bad = recorder
        .0
        .iter()
        .find_map(|record| match record {
            tex_command::CommandObservation::Command(command)
                if command.spelling
                    == tex_command::ObservedToken::ControlSequence("bad".into()) =>
            {
                Some(command)
            }
            _ => None,
        })
        .expect("undefined macro-body command is observed");
    assert!(bad.provenance.has_origin);
    assert!(recorder.0.iter().any(|record| matches!(
        record,
        tex_command::CommandObservation::Input(tex_command::InputRecord {
            transition: tex_command::InputTransition::Retire,
            reason: tex_command::InputReason::Macro,
            ..
        })
    )));
    assert!(terminal_effect_text(&stores).contains("Undefined control sequence"));
}

#[test]
fn extra_endcsname_delivery_reports_and_continues() {
    let stores = run_canonical_tex82(r"\endcsname\message{continued}\end");
    let output = terminal_effect_text(&stores);

    assert!(output.contains("Extra \\endcsname"));
    assert!(output.contains("continued"));
}

#[test]
fn illegal_prefix_replays_scanned_token_with_its_origin() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let mut recorder = ObservationRecorder::default();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\global x\end".to_vec(),
        ))
        .expect("register canonical source");
    for _ in 0..32 {
        if matches!(
            control
                .step_with_observer(&mut stores, &mut recorder)
                .expect("canonical observed step"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            break;
        }
    }

    let recovery = recorder
        .0
        .iter()
        .find_map(|record| match record {
            tex_command::CommandObservation::Recovery(recovery)
                if recovery.kind == tex_command::RecoveryKind::Backup =>
            {
                Some(recovery)
            }
            _ => None,
        })
        .expect("non-assignment token is backed up");
    assert_eq!(
        recovery.tokens,
        [tex_command::ObservedToken::Character {
            character: 'x',
            catcode: Catcode::Letter,
        }]
    );
    let replayed = recorder
        .0
        .iter()
        .filter_map(|record| match record {
            tex_command::CommandObservation::Command(command)
                if command.spelling
                    == tex_command::ObservedToken::Character {
                        character: 'x',
                        catcode: Catcode::Letter,
                    } =>
            {
                Some(command)
            }
            _ => None,
        })
        .last()
        .expect("backed-up token is replayed");
    assert!(replayed.provenance.has_origin);
    assert!(terminal_effect_text(&stores).contains("You can't use a prefix"));
}

#[test]
fn main_control_uses_get_x_token_and_expands_macros_before_dispatch() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    let mut recorder = ObservationRecorder::default();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\relax".to_vec(),
        ))
        .expect("register canonical source");
    while control
        .step_with_observer(&mut stores, &mut recorder)
        .expect("canonical observed step")
        == MainControlStep::Continue
    {}

    let expanded: Vec<_> = recorder
        .0
        .iter()
        .filter_map(|record| match record {
            tex_command::CommandObservation::Command(command)
                if command.boundary == tex_command::CommandDeliveryBoundary::Expanded =>
            {
                Some(command)
            }
            _ => None,
        })
        .collect();
    assert_eq!(expanded.len(), 1);
    assert_eq!(expanded[0].command, "relax");
    assert!(expanded[0].provenance.has_origin);
}

#[test]
fn horizontal_main_control_batches_inactive_alignment_macro_text() {
    let run = observed_canonical_font_run(false, br"\font\f=cmr10 \f\def\x{abcdefgh}\x");
    let run = horizontal_character_run(&run.steps, 'a'..='h');

    assert_eq!(run.len(), 8);
    assert!(!run[0].main_loop_before);
    assert!(run.iter().all(|step| step.main_loop_after));
    assert_eq!(run[0].delivery.provenance.position, 0);
    assert_eq!(run[7].delivery.provenance.position, 7);
}

#[test]
fn horizontal_main_control_batches_direct_physical_source_text() {
    let run = observed_canonical_font_run(false, br"\font\f=cmr10 \f abcdef");
    let run = horizontal_character_run(&run.steps, 'a'..='f');

    assert_eq!(run.len(), 6);
    assert!(!run[0].main_loop_before);
    assert!(run[1..].iter().all(|step| step.main_loop_before));
    assert_eq!(run.iter().filter(|step| step.main_loop_before).count(), 5);
    assert!(run.iter().all(|step| step.delivery.provenance.has_origin));
}

#[test]
fn paragraph_recording_preserves_source_text_batching() {
    fn run(memo: bool) -> Vec<tex_command::CommandObservation> {
        observed_canonical_font_run(memo, br"\font\f=cmr10 \f abcdef\par").records
    }

    let ordinary = run(false);
    let memo_miss = run(true);
    assert_eq!(
        ordinary, memo_miss,
        "paragraph recording must preserve delivery batching, provenance, and dispatch count"
    );
}

#[test]
fn horizontal_main_control_deopts_macro_text_when_alignment_scanner_is_active() {
    let run = observed_canonical_font_run(
        false,
        br"\font\f=cmr10 \f\def\x{abcdefgh}\setbox0=\vbox{\halign{#\cr\omit\x\cr}}\end",
    );
    let run = horizontal_character_run(&run.steps, 'a'..='h');

    assert_eq!(run.len(), 8);
    assert!(
        run.iter()
            .all(|step| !step.main_loop_before && !step.main_loop_after)
    );
    assert_eq!(
        run[0].delivery.provenance.input_level,
        run[7].delivery.provenance.input_level
    );
    assert_eq!(run[0].delivery.provenance.position, 0);
    assert_eq!(run[7].delivery.provenance.position, 7);
}

struct ObservedCanonicalRun {
    records: Vec<tex_command::CommandObservation>,
    steps: Vec<ObservedCanonicalStep>,
}

struct ObservedCanonicalStep {
    records: Vec<tex_command::CommandObservation>,
    main_loop_before: bool,
    main_loop_after: bool,
}

fn observed_canonical_font_run(memo: bool, source: &[u8]) -> ObservedCanonicalRun {
    let mut stores = stores_with_fonts();
    if memo {
        stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    }
    let metrics = tex_state::InputReadState::read_input_file(
        &mut stores.input_open_context(),
        std::path::Path::new("cmr10.tfm"),
    )
    .expect("seeded font fixture reads");
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.capabilities_mut().register_font(
        "cmr10.tfm",
        FontResource::Tfm {
            metrics,
            opentype: None,
        },
    );
    observed_canonical_run_with_control(&mut control, &mut stores, source)
}

fn observed_canonical_run_with_control(
    control: &mut CanonicalMainControl,
    stores: &mut Universe,
    source: &[u8],
) -> ObservedCanonicalRun {
    let mut recorder = ObservationRecorder::default();
    let mut steps = Vec::new();
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.to_vec(),
        ))
        .expect("register canonical source");
    for _ in 0..128 {
        let record_start = recorder.0.len();
        let main_loop_before = control.main_loop_active_for_test();
        let result = control
            .step_with_observer(stores, &mut recorder)
            .expect("canonical observed step");
        steps.push(ObservedCanonicalStep {
            records: recorder.0[record_start..].to_vec(),
            main_loop_before,
            main_loop_after: control.main_loop_active_for_test(),
        });
        if matches!(result, MainControlStep::End | MainControlStep::EndOfInput) {
            return ObservedCanonicalRun {
                records: recorder.0,
                steps,
            };
        }
    }
    panic!("canonical source did not stop consuming input");
}

struct HorizontalCharacterStep<'a> {
    delivery: &'a tex_command::CommandDeliveryRecord,
    main_loop_before: bool,
    main_loop_after: bool,
}

fn horizontal_character_run(
    steps: &[ObservedCanonicalStep],
    characters: std::ops::RangeInclusive<char>,
) -> Vec<HorizontalCharacterStep<'_>> {
    let expected: Vec<_> = characters.clone().collect();
    let candidates: Vec<_> = steps
        .iter()
        .filter_map(|step| {
            step.records.iter().rev().find_map(|record| match record {
                tex_command::CommandObservation::Command(delivery)
                    if matches!(delivery.spelling, tex_command::ObservedToken::Character { character, .. } if characters.contains(&character)) =>
                {
                    Some(HorizontalCharacterStep {
                        delivery,
                        main_loop_before: step.main_loop_before,
                        main_loop_after: step.main_loop_after,
                    })
                }
                _ => None,
            })
        })
        .collect();
    let start = candidates
        .windows(expected.len())
        .rposition(|window| {
            window.iter().zip(&expected).all(|(step, expected)| {
                matches!(step.delivery.spelling, tex_command::ObservedToken::Character { character, .. } if character == *expected)
            })
        })
        .expect("ordered horizontal character run is observed");
    candidates
        .into_iter()
        .skip(start)
        .take(expected.len())
        .collect()
}

#[test]
fn main_control_recovers_from_undefined_control_sequence() {
    let stores = run_canonical_tex82("\\missing\\count0=7\\end");

    assert_eq!(stores.count(0), 7);
    // §370 reports the message alone; §82's context is what shows `\missing`.
    let output = terminal_effect_text(&stores);
    assert!(output.contains("! Undefined control sequence."), "{output}");
    assert!(output.contains("\\missing"), "{output}");
}

#[test]
fn register_index_scanner_recovers_from_undefined_control_sequence() {
    let stores = run_canonical_tex82("\\setbox\\missing\\hbox{x}\\global\\count2=7\\end");

    assert_eq!(stores.count(2), 7);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("! Undefined control sequence."), "{output}");
    assert!(output.contains("\\missing"), "{output}");
    assert!(output.contains("Missing number, treated as zero"));
}

#[test]
fn recursively_expanded_dimension_scanner_recovers_from_undefined_control_sequence() {
    let stores = run_canonical_tex82("\\ifdim\\dimen\\missing=0pt \\global\\count2=7\\fi\\end");

    assert_eq!(stores.count(2), 7);
    let output = terminal_effect_text(&stores);
    assert!(output.contains("! Undefined control sequence."), "{output}");
    assert!(output.contains("\\missing"), "{output}");
}

#[test]
fn main_control_keeps_replaying_macro_after_undefined_control_sequence() {
    let stores = run_canonical_tex82("\\def\\resume{\\missing\\let\\x\\relax}\\resume\\end");

    let x = stores.symbol("x").expect("let target exists");
    assert_eq!(stores.meaning(x), Meaning::Relax);
}

#[test]
fn main_control_consumes_invalid_category_character() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('@', Catcode::Invalid);
    let stores = run_canonical_tex82_with_universe(stores, "@\\count0=7\\end");

    assert_eq!(stores.count(0), 7);
}

#[test]
fn main_control_aborts_nonlong_macro_argument_at_par_and_replays_par() {
    let stores = run_canonical_tex82("\\def\\b#1\\par{}\\b{x\\par\\count0=7\\end");

    assert_eq!(stores.count(0), 7);
    assert!(terminal_effect_text(&stores).contains("Runaway argument"));
}

#[test]
fn main_control_ignores_extra_conditional_terminator() {
    let stores = run_canonical_tex82("\\else\\count0=7\\end");

    assert_eq!(stores.count(0), 7);
    assert!(terminal_effect_text(&stores).contains("Extra \\else"));
}

#[test]
fn def_and_gdef_assign_macro_meanings_through_group_barrier() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            br"\def\a{A}\gdef\b{B}".to_vec(),
        ))
        .expect("register canonical definition source");
    stores.enter_group();

    for _ in 0..16 {
        if control
            .step(&mut stores)
            .expect("canonical definition step")
            == MainControlStep::EndOfInput
        {
            break;
        }
    }
    let a = stores.symbol("a").expect("a was interned");
    let b = stores.symbol("b").expect("b was interned");
    assert!(matches!(stores.meaning(a), Meaning::Macro { .. }));
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));

    let _ = stores.leave_group();
    assert_eq!(stores.meaning(a), Meaning::Undefined);
    assert!(matches!(stores.meaning(b), Meaning::Macro { .. }));
}

#[test]
fn edef_omits_noexpand_command_and_freezes_the_output() {
    let stores = run_canonical_tex82(r"\let\b=\relax\toks0={\b}\edef\e{\noexpand\a\the\toks0}\end");
    let a = stores.symbol("a").expect("a was interned");
    let b = stores.symbol("b").expect("b was interned");
    let e = stores.symbol("e").expect("e was interned");
    let meaning = stores.macro_meaning(e).expect("e is a macro");

    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[Token::Cs(a.symbol()), Token::Cs(b.symbol())]
    );
}

#[test]
fn edef_expandafter_expands_a_target_preserved_by_prior_unexpanded() {
    // TeX.web section 366 expands the second raw token once; e-TeX manual
    // section 3.1 limits `\unexpanded` suppression to construction of the
    // expanded token list, not a later invocation of that stored list.
    let stores = run_canonical_etex(
        r"\def\a{A}\def\b{B}\edef\holder{\unexpanded{\expandafter\a\b}}\edef\result{\holder}",
    );

    assert_eq!(macro_text(&stores, "result"), "AB");
}

#[test]
fn edef_expansion_uses_active_input_resolver() {
    let stores = run_canonical_tex82_with_inputs(
        r"\endlinechar=-1 \edef\e{A\input{outer}E}\count0=9\end",
        &[("outer", b"B\\input{inner}D"), ("inner", b"C")],
    );
    let e = stores.symbol("e").expect("e was interned");
    let meaning = stores.macro_meaning(e).expect("e is a macro");

    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'B',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'C',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'D',
                cat: Catcode::Letter
            },
            Token::Char {
                ch: 'E',
                cat: Catcode::Letter
            },
        ]
    );
    assert_eq!(
        stores.count(0),
        9,
        "the command after the edef remains unread"
    );
}

#[test]
fn input_expands_while_scanning_assignment_values() {
    let stores = run_canonical_tex82_with_inputs(
        "\\endlinechar=-1 \\dimen0=\\input{dim}\\skip0=\\input{glue}\\count0=41\\end",
        &[
            ("dim", b"\\input{number}pt"),
            ("number", b"12"),
            ("glue", b"3pt plus \\input{stretch}pt"),
            ("stretch", b"2"),
        ],
    );

    assert_eq!(
        stores.dimen(0),
        tex_state::scaled::Scaled::from_raw(12 * 65_536)
    );
    let glue = stores.glue(stores.skip(0));
    assert_eq!(glue.width, tex_state::scaled::Scaled::from_raw(3 * 65_536));
    assert_eq!(
        glue.stretch,
        tex_state::scaled::Scaled::from_raw(2 * 65_536)
    );
    assert_eq!(
        stores.count(0),
        41,
        "the command after both scans remains unread"
    );
}

#[test]
fn input_expands_while_scanning_conditional_operands() {
    let stores = run_canonical_tex82_with_inputs(
        "\\endlinechar=-1\\ifdim\\input{left}<\\input{right}\\count0=1\\fi\
         \\ifcat\\input{a}\\input{b}\\count1=1\\fi\
         \\ifnum 1 \\input{relation} 2\\count2=1\\fi\
         \\ifeof\\input{stream}\\count3=1\\fi\\end",
        &[
            ("left", b"1pt"),
            ("right", b"2pt"),
            ("a", b"a"),
            ("b", b"b"),
            ("relation", b"<"),
            ("stream", b"15"),
        ],
    );

    assert_eq!(stores.count(0), 1);
    assert_eq!(stores.count(1), 1);
    assert_eq!(stores.count(2), 1);
    assert_eq!(stores.count(3), 1);
}

#[test]
fn input_expands_while_scanning_register_indices_and_the_operands() {
    let stores = run_canonical_tex82_with_inputs(
        "\\endlinechar=-1\\count\\input{idx}=9\\edef\\e{\\the\\count\\input{idx}}\\end",
        &[("idx", b"5")],
    );

    assert_eq!(stores.count(5), 9);
    let e = stores.symbol("e").expect("macro was defined");
    let meaning = stores.macro_meaning(e).expect("e is a macro");
    assert_eq!(
        stores.tokens(meaning.replacement_text()),
        &[Token::Char {
            ch: '9',
            cat: Catcode::Other
        }]
    );
}

#[test]
fn let_assigns_control_sequence_and_implicit_character_meanings() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let a = stores.intern("a");
    stores.set_meaning(a, Meaning::CharGiven('Q'));
    let stores = run_canonical_tex82_with_universe(stores, "\\let\\b=\\a\\let\\c = Z\\end");
    assert_eq!(
        stores.meaning(stores.symbol("b").expect("b was interned")),
        Meaning::CharGiven('Q')
    );
    assert_eq!(
        stores.meaning(stores.symbol("c").expect("c was interned")),
        Meaning::CharToken {
            ch: 'Z',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn let_skips_spaces_before_optional_equals_and_aliases_control_symbol() {
    let stores = run_canonical_tex82("\\def\\\\#1{#1}\\let\\alias   = \\\\ \\end");

    let control_symbol = stores.symbol("\\").expect("control symbol");
    let alias = stores.symbol("alias").expect("alias");
    assert_eq!(stores.meaning(alias), stores.meaning(control_symbol));
}

#[test]
fn plain_getf_ctor_setup_restores_catcodes_before_control_symbol_alias() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('@', Catcode::Letter);
    let stores = run_canonical_tex82_with_universe(
        stores,
        "{\\catcode`p=12 \\catcode`t=12 \\gdef\\\\#1pt{#1}} \\let\\getf@ctor=\\\\\\end",
    );

    assert_eq!(stores.catcode('p'), Catcode::Letter);
    assert_eq!(stores.catcode('t'), Catcode::Letter);
    let control_symbol = stores.symbol("\\").expect("control symbol");
    let alias = stores.symbol("getf@ctor").expect("getf@ctor alias");
    assert_eq!(stores.meaning(alias), stores.meaning(control_symbol));
}

#[test]
fn futurelet_assigns_second_token_meaning_and_preserves_order() {
    let stores =
        run_canonical_tex82("\\def\\first#1{\\global\\count0=`#1}\\futurelet\\n\\first x\\end");

    let n = stores.symbol("n").expect("n was interned");
    assert_eq!(
        stores.meaning(n),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
    assert_eq!(
        stores.count(0),
        'x' as i32,
        "the first token executes before the preserved lookahead token"
    );
}

#[test]
fn let_copies_frozen_endv_alignment_meaning() {
    let stores = run_canonical_tex82(
        "\\def\\capture{\\afterassignment\\relax\\global\\let\\endvalias=}\\halign{#\\cr x\\capture\\cr}\\end",
    );
    let alias = stores.symbol("endvalias").expect("global end-v alias");

    assert_eq!(
        stores.meaning(alias.symbol()),
        Meaning::ExpandablePrimitive(ExpandablePrimitive::EndTemplate)
    );
}

#[test]
fn def_accepts_active_character_target_and_expands_it() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores = run_canonical_tex82_with_universe(stores, "\\def~{OK}\\edef\\x{~}\\end");

    assert!(
        stores
            .macro_meaning(stores.active_character_symbol('~').expect("active symbol"))
            .is_some()
    );
    assert_eq!(macro_text(&stores, "x"), "OK");
}

#[test]
fn active_character_and_same_spelling_control_symbol_expand_independently() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores = run_canonical_tex82_with_universe(
        stores,
        "\\def~{ACTIVE}\\def\\~{NAMED}\\edef\\a{~}\\edef\\b{\\~}\\end",
    );

    let named = stores.symbol("~").expect("named control symbol");
    let active = stores.active_character_symbol('~').expect("active symbol");
    assert_ne!(named, active);
    assert_eq!(macro_text(&stores, "a"), "ACTIVE");
    assert_eq!(macro_text(&stores, "b"), "NAMED");
}

#[test]
fn let_accepts_active_character_target() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores =
        run_canonical_tex82_with_universe(stores, "\\def\\a{A}\\let~=\\a\\edef\\x{~}\\end");

    assert_eq!(macro_text(&stores, "x"), "A");
}

#[test]
fn futurelet_accepts_active_character_target() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores =
        run_canonical_tex82_with_universe(stores, "\\def\\first#1{}\\futurelet~\\first x\\end");

    assert_eq!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::CharToken {
            ch: 'x',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn countdef_accepts_active_character_target_and_assigns_through_it() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores = run_canonical_tex82_with_universe(stores, "\\countdef~=12 ~=7\\end");

    assert_eq!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::CountRegister(12)
    );
    assert_eq!(stores.count(12), 7);
}

#[test]
fn outer_def_accepts_active_character_target() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_catcode('~', Catcode::Active);
    let stores = run_canonical_tex82_with_universe(stores, "\\outer\\def~{A}\\end");

    assert!(matches!(
        stores.meaning(stores.active_character_symbol('~').expect("active symbol")),
        Meaning::Macro { flags, .. } if flags.contains(tex_state::meaning::MeaningFlags::OUTER)
    ));
}

#[test]
fn box_primitives_round_trip_through_registers() {
    let stores = run_canonical_tex82("\\setbox0=\\hbox to 10pt{}\\setbox1=\\copy0\\box0");

    assert!(stores.box_reg(0).is_none(), "\\box should void register 0");
    let box1 = stores.box_reg(1).expect("copy should preserve register 1");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box1).testing_decoded() else {
        panic!("register 1 should hold an hbox");
    };
    assert_eq!(box_node.width.raw(), 10 * tex_state::scaled::Scaled::UNITY);
    let current_page = stores.current_page_nodes();
    let Some(tex_state::node::Node::HList(appended)) = current_page
        .iter()
        .find(|node| matches!(node, tex_state::node::Node::HList(_)))
    else {
        panic!("current page should contain copied-out hbox");
    };
    assert_eq!(appended.width.raw(), 10 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn box_scanner_inserts_missing_left_brace_and_replays_body_token() {
    let stores = run_canonical_tex82("\\setbox0=\\hbox \\global\\count0=7}");

    assert_eq!(stores.count(0), 7);
    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("Missing { inserted"));
}

#[test]
fn box_scanner_closes_by_execution_group_after_message_argument() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\\message{x}\\vbox{\\hrule height2pt}}\\hrule height3pt",
    );

    let box0 = stores.box_reg(0).expect("setbox destination remains owned");
    let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("box0 should contain the outer hbox");
    };
    assert!(
        stores
            .nodes(hbox.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_))),
        "box0 should own the nested vbox"
    );
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .all(|node| !matches!(node, Node::VList(_))),
        "the nested vbox must not escape to the outer vertical list"
    );
    assert!(
        stores.page_contributions().iter().any(
            |node| matches!(node, Node::Rule { height: Some(height), .. } if height.raw() == 3 * Scaled::UNITY)
        ),
        "outer material should remain outside box0"
    );
}

#[test]
fn trip_math_mode_box_closure_preserves_ownership_and_replays() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let checkpoint = stores.snapshot();
    // This is the decisive topology reduced from the malformed tail of
    // trip.tex: Box is innermost when its brace arrives, but Math is still
    // the current mode. The vbox corresponds to the material box9 must own.
    let source = "\\setbox0=\\hbox{\x24x}\x24\\vbox{\\hrule height2pt}}\\hrule height3pt";
    let mut first_hash = None;

    for pass in 0..2 {
        stores = run_canonical_tex82_with_universe(stores, source);

        assert_eq!(stores.execution_group_depth(), 0, "pass {pass}");
        let box0 = stores.box_reg(0).expect("recovered setbox remains nonvoid");
        let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
            panic!("box0 should own the recovered outer hbox");
        };
        assert!(
            stores
                .nodes(hbox.children)
                .testing_decoded()
                .iter()
                .any(|node| matches!(node, Node::HList(_) | Node::VList(_))),
            "recovered nested material remains owned by box0"
        );
        assert!(
            stores
                .current_page_nodes()
                .iter()
                .all(|node| !matches!(node, Node::VList(_))),
            "nested vbox must not leak to the outer page"
        );
        assert!(stores.page_contributions().iter().any(
            |node| matches!(node, Node::Rule { height: Some(height), .. } if height.raw() == 3 * Scaled::UNITY)
        ));

        let hash = stores.snapshot().state_hash();
        if let Some(expected) = first_hash {
            assert_eq!(hash, expected, "rollback replay must converge");
        } else {
            first_hash = Some(hash);
            stores.rollback(&checkpoint);
        }
    }
}

#[test]
fn recoverable_assignment_error_inside_box_preserves_box_ownership() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\\afterassignment\\relax\\advance\\prevdepth\\undefined\\vbox{\\hrule height2pt}}",
    );

    let box0 = stores
        .box_reg(0)
        .expect("setbox must not roll back to void");
    let [Node::HList(hbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("box0 should contain the recovered hbox");
    };
    assert!(
        stores
            .nodes(hbox.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::VList(_))),
        "the remaining box body must stay owned by box0"
    );
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .all(|node| !matches!(node, Node::VList(_))),
        "the nested vbox must not leak onto the outer page"
    );
}

#[test]
fn last_box_assignment_replays_with_identical_state_hash() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let checkpoint = stores.snapshot();
    let source = "\\setbox0=\\hbox{\\raise2pt\\hbox to7pt{}\\global\\setbox1=\\lastbox}";

    stores = run_canonical_tex82_with_universe(stores, source);
    let first_box = stores.box_reg(1).expect("global lastbox destination");
    let [Node::HList(first_node)] = stores.nodes(first_box).testing_decoded() else {
        panic!("lastbox destination should contain an hbox");
    };
    assert_eq!(first_node.shift.raw(), 0, "lastbox clears box shift");
    let first_hash = stores.snapshot().state_hash();
    stores.rollback(&checkpoint);
    stores = run_canonical_tex82_with_universe(stores, source);

    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn control_space_uses_space_skip_without_space_factor_scaling() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \\f\
         \\fontdimen2\\f=10pt \\fontdimen3\\f=2pt \\fontdimen4\\f=3pt \
         \\spaceskip=20pt \\xspaceskip=30pt \
         \\setbox0=\\hbox{A\\spacefactor=3000\\ B}",
    );

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children).testing_decoded();
    assert!(matches!(
        children,
        [
            tex_state::node::Node::Char { ch: 'A', .. },
            tex_state::node::Node::Glue { spec, kind: tex_state::node::GlueKind::SpaceSkip, leader: None },
            tex_state::node::Node::Char { ch: 'B', .. },
        ] if stores.glue(*spec) == GlueSpec {
            width: Scaled::from_raw(20 * Scaled::UNITY),
            stretch: Scaled::from_raw(0),
            stretch_order: tex_state::glue::Order::Normal,
            shrink: Scaled::from_raw(0),
            shrink_order: tex_state::glue::Order::Normal,
        }
    ));
}

#[test]
fn sentence_space_preserves_xspaceskip_and_spaceskip_node_subtypes() {
    // TeX82 §§182/1042: a nonzero `\xspaceskip` has its own node subtype;
    // when it is zero, sentence spacing falls back to scaled `\spaceskip`
    // and retains that distinct subtype for `show_node_list`.
    let stores = run_canonical_tex82_with_fonts(
        r"\font\f=cmr10 \relax \f\spaceskip=20pt plus 2pt minus 3pt
          \xspaceskip=30pt \setbox0=\hbox{A\spacefactor=3000{} B}\xspaceskip=0pt
          \setbox1=\hbox{A\spacefactor=3000{} B}",
    );

    let glue = |stores: &Universe, register| {
        let root = stores.box_reg(register).expect("box is assigned");
        let [Node::HList(box_node)] = stores.nodes(root).testing_decoded() else {
            panic!("register holds an hbox");
        };
        stores
            .nodes(box_node.children)
            .testing_decoded()
            .iter()
            .find_map(|node| match node {
                Node::Glue { spec, kind, .. } => Some((*spec, *kind)),
                _ => None,
            })
            .expect("sentence contains interword glue")
    };
    let (xspace_spec, xspace_kind) = glue(&stores, 0);
    assert_eq!(xspace_kind, tex_state::node::GlueKind::XSpaceSkip);
    assert_eq!(stores.glue(xspace_spec).width.raw(), 30 * Scaled::UNITY);

    let (space_spec, space_kind) = glue(&stores, 1);
    assert_eq!(space_kind, tex_state::node::GlueKind::SpaceSkip);
    let space = stores.glue(space_spec);
    assert_eq!(space.width.raw(), 20 * Scaled::UNITY);
    assert_eq!(space.stretch.raw(), 6 * Scaled::UNITY);
    assert_eq!(space.shrink.raw(), Scaled::UNITY);
}

#[test]
fn invalid_space_factor_reports_and_preserves_the_previous_value() {
    let stores =
        run_canonical_tex82(r"\noindent\spacefactor=2000\spacefactor=0\count0=\spacefactor");

    assert_eq!(stores.count(0), 2000);
    assert!(support::terminal_effect_text(&stores).contains("Bad space factor (0)"));
}

#[test]
fn adjacent_cmr10_characters_emit_tfm_kern() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \\f \\everypar{\\penalty10000}\\setbox0=\\vbox{Yo\\par}",
    );

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [Node::VList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(box_node.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let children = stores.nodes(line.children).testing_decoded();
    assert!(
        children.windows(3).any(|nodes| matches!(
            nodes,
            [
                Node::Char { ch: 'Y', .. },
                Node::Kern {
                    amount,
                    kind: tex_state::node::KernKind::Font,
                },
                Node::Char { ch: 'o', .. },
            ] if amount.raw() == -54_614
        )),
        "unexpected Yo nodes: {children:?}"
    );
}

#[test]
fn literal_groups_break_ligature_runs_and_preserve_natural_width() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \\f \\setbox0=\\hbox{first}\\setbox1=\\hbox{{f}irst}",
    );

    let ligated = stores.box_reg(0).expect("ligated box should be assigned");
    let grouped = stores.box_reg(1).expect("grouped box should be assigned");
    let [Node::HList(ligated_box)] = stores.nodes(ligated).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [Node::HList(grouped_box)] = stores.nodes(grouped).testing_decoded() else {
        panic!("register 1 should hold an hbox");
    };

    assert!(matches!(
        stores.nodes(ligated_box.children).testing_decoded().first(),
        Some(Node::Lig {
            ch: '\u{c}',
            orig,
            ..
        }) if orig == &['f', 'i']
    ));
    assert!(matches!(
        stores.nodes(grouped_box.children).testing_decoded(),
        [Node::Char { ch: 'f', .. }, Node::Char { ch: 'i', .. }, ..]
    ));
    assert_eq!(
        grouped_box.width.raw() - ligated_box.width.raw(),
        18_205,
        "cmr10's unligated f+i pair has TeX82's larger natural width"
    );
}

#[test]
fn appended_box_resets_space_factor_before_sentence_punctuation() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \\f \\sfcode46=3000 A\\hbox{}.\\message{S=\\the\\spacefactor}\\end",
    );

    let output = terminal_effect_text(&stores);
    assert!(output.contains("S=3000"), "unexpected output: {output:?}");
}

#[test]
fn overfull_hbox_appends_running_rule_when_enabled() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_dimen_param(
        DimenParam::OVERFULL_RULE,
        tex_state::scaled::Scaled::from_raw(3 * tex_state::scaled::Scaled::UNITY),
    );
    let stores = run_canonical_tex82_with_universe(stores, "\\setbox0=\\hbox to 10pt{\\kern20pt}");

    let box0 = stores.box_reg(0).expect("box should be assigned");
    let [tex_state::node::Node::HList(box_node)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let children = stores.nodes(box_node.children).testing_decoded();
    assert!(matches!(
        children.last(),
        Some(tex_state::node::Node::Rule {
            width: Some(width),
            height: None,
            depth: None,
        }) if width.raw() == 3 * tex_state::scaled::Scaled::UNITY
    ));
}

#[test]
fn box_dimension_writes_are_readable_by_the() {
    let stores = run_canonical_tex82("\\setbox0=\\hbox{}");
    let before = stores.testing_epoch_clone_counts();
    let stores = run_canonical_tex82_with_universe(stores, "\\wd0=12pt\\edef\\x{\\the\\wd0}");
    assert_eq!(stores.testing_epoch_clone_counts(), before);

    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Width)
            .expect("box dimension")
            .raw(),
        12 * tex_state::scaled::Scaled::UNITY
    );
    let x = stores.symbol("x").expect("x was interned");
    let meaning = stores.macro_meaning(x).expect("x is a macro");
    let rendered: String = stores
        .tokens(meaning.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect();
    assert_eq!(rendered, "12.0pt");
}

#[test]
fn box_dimension_writes_mutate_the_visible_box_binding() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{} {\\ht0=12pt}\\setbox1=\\hbox{} {\\setbox1=\\hbox{}\\global\\ht1=9pt}",
    );

    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("inherited box survives")
            .raw(),
        12 * tex_state::scaled::Scaled::UNITY,
        "an inherited box is mutated across the current group"
    );
    assert_eq!(
        stores
            .box_dimension(1, tex_state::BoxDimension::Height)
            .expect("outer box is restored")
            .raw(),
        0,
        "a dimension prefix does not globalize a locally bound box"
    );
}

#[test]
fn uncopy_primitives_unbox_without_clearing_registers() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\\kern1pt}\
         \\setbox1=\\hbox{\\unhcopy0}\
         \\setbox2=\\vbox{\\kern2pt}\
         \\setbox3=\\vbox{\\unvcopy2}",
    );

    assert!(stores.box_reg(0).is_some(), "\\unhcopy should not clear");
    assert!(stores.box_reg(2).is_some(), "\\unvcopy should not clear");

    let hcopy = stores.box_reg(1).expect("hcopy destination");
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hcopy).testing_decoded() else {
        panic!("register 1 should hold an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vcopy = stores.box_reg(3).expect("vcopy destination");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vcopy).testing_decoded() else {
        panic!("register 3 should hold a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));
}

#[test]
fn etex_lastnodetype_tracks_effective_outer_vertical_tail() {
    // e-TeX short reference manual section 3.3 assigns -1 to an empty list
    // and the e-TRIP node codes 1, 12, and 13 to hlist, kern, and penalty.
    for (material, expected) in [("\\hbox{}", "1"), ("\\kern1pt", "12"), ("\\penalty7", "13")] {
        let stores = run_canonical_etex(&format!(
            "\\relax{material}\\edef\\result{{\\the\\lastnodetype}}"
        ));

        assert_eq!(macro_text(&stores, "result"), expected);
    }
}

#[test]
fn etex_tracingscantokens_closes_after_everyeof() {
    // The e-TeX manual sections 3.2 and 3.6 require `( ` on pseudo-file
    // entry and the matching `)` only when scanning, including everyeof, ends.
    let stores = run_canonical_etex(
        "\\tracingscantokens=1\\everyeof{\\message{EOF}}\\scantokens{\\message{BODY}}\\end",
    );

    let output = terminal_effect_text(&stores);
    let open = output.find('(').expect("pseudo-file opening trace");
    let body = output.find("BODY").expect("pseudo-file body");
    let eof = output.find("EOF").expect("everyeof body");
    let close = output.find(')').expect("pseudo-file closing trace");
    assert!(open < body && body < eof && eof < close, "{output:?}");
}

#[test]
fn etex_glue_component_and_conversion_enquiries_match_manual_types() {
    let stores = run_canonical_etex(
        "\\skip0=1pt plus 2fill minus 3fil\\muskip0=4mu plus 5fil\
         \\edef\\result{\\the\\gluestretch\\skip0/\\the\\glueshrink\\skip0/\
         \\the\\gluestretchorder\\skip0,\\the\\glueshrinkorder\\skip0/\
         \\the\\gluetomu\\skip0/\\the\\mutoglue\\muskip0}",
    );
    assert_eq!(
        macro_text(&stores, "result"),
        "2.0pt/3.0pt/2,1/1.0mu plus 2.0fill minus 3.0fil/4.0pt plus 5.0fil"
    );
}

#[test]
fn etex_showtokens_decomposes_unexpanded_balanced_text() {
    let stores = run_canonical_etex("\\def\\foo#1{X#1}\\showtokens{a \\foo{b}}\\end");
    assert!(terminal_effect_text(&stores).contains("> a \\foo {b}."));
}

#[test]
fn etex_showtokens_expands_only_to_find_its_opening_brace() {
    let stores =
        run_canonical_etex("\\def\\payload{kept}\\showtokens\\expandafter{\\payload}\\end");
    assert!(terminal_effect_text(&stores).contains("> kept."));
}

#[test]
fn etex_showgroups_and_showifs_report_live_checkpointed_stacks() {
    let stores = run_canonical_etex("\\begingroup\\iftrue\\showgroups\\showifs\\fi\\endgroup\\end");
    let output = terminal_effect_text(&stores);
    assert!(
        output.contains("### semi simple group (level 1)"),
        "{output:?}"
    );
    assert!(output.contains("### bottom level"), "{output:?}");
    assert!(output.contains("### level 1: \\iftrue"), "{output:?}");
}

#[test]
fn etex_showifs_is_available_inside_math_mode() {
    let stores = run_canonical_etex("\\iftrue$\\showifs$\\fi\\end");
    assert!(terminal_effect_text(&stores).contains("\\iftrue"));
}

#[test]
fn leaders_parse_box_and_rule_payloads_on_glue_nodes() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\\leaders\\hbox{\\kern1pt}\\hskip10pt}\
         \\setbox1=\\vbox{\\cleaders\\hrule height2pt\\vskip5pt}",
    );

    let hbox = stores.box_reg(0).expect("hbox register");
    let [tex_state::node::Node::HList(hbox)] = stores.nodes(hbox).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [
        tex_state::node::Node::Glue {
            spec,
            kind,
            leader: Some(tex_state::node::LeaderPayload::HList(payload)),
        },
    ] = stores.nodes(hbox.children).testing_decoded()
    else {
        panic!("hbox should contain leader glue with hlist payload");
    };
    assert_eq!(*kind, tex_state::node::GlueKind::Leaders);
    assert_eq!(
        stores.glue(*spec).width.raw(),
        10 * tex_state::scaled::Scaled::UNITY
    );
    assert!(matches!(
        stores.nodes(payload.children).testing_decoded(),
        [tex_state::node::Node::Kern { .. }]
    ));

    let vbox = stores.box_reg(1).expect("vbox register");
    let [tex_state::node::Node::VList(vbox)] = stores.nodes(vbox).testing_decoded() else {
        panic!("register 1 should hold a vbox");
    };
    let [
        tex_state::node::Node::Glue {
            spec,
            kind,
            leader:
                Some(tex_state::node::LeaderPayload::Rule {
                    height: Some(height),
                    ..
                }),
        },
    ] = stores.nodes(vbox.children).testing_decoded()
    else {
        panic!("vbox should contain leader glue with rule payload");
    };
    assert_eq!(*kind, tex_state::node::GlueKind::Cleaders);
    assert_eq!(
        stores.glue(*spec).width.raw(),
        5 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(height.raw(), 2 * tex_state::scaled::Scaled::UNITY);
}

#[test]
fn leaders_report_missing_payload_and_glue_diagnostics() {
    let missing_payload = run_canonical_tex82("\\setbox0=\\hbox{\\leaders x\\hskip10pt}");
    assert!(terminal_effect_text(&missing_payload).contains("A <box> was supposed to be here."));
}

#[test]
fn leaders_missing_glue_diagnostic_recovers_into_following_assignment() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\\leaders\\hbox{}\\global\\count0=7}+         \\setbox1=\\vbox{\\cleaders\\hrule\\global\\count1=8}",
    );

    assert_eq!(stores.count(0), 7, "box-payload recovery replays \\global");
    assert_eq!(stores.count(1), 8, "rule-payload recovery replays \\global");
    let terminal = support::terminal_effect_text(&stores);
    assert_eq!(
        terminal
            .matches("Leaders not followed by proper glue")
            .count(),
        2,
        "each malformed leader reports TeX82 §1078's diagnostic: {terminal:?}"
    );

    for register in [0, 1] {
        let boxed = stores.box_reg(register).expect("recovered box register");
        let [node] = stores.nodes(boxed).testing_decoded() else {
            panic!("register {register} should contain its outer box");
        };
        let children = match node {
            tex_state::node::Node::HList(node) => node.children,
            tex_state::node::Node::VList(node) => node.children,
            other => panic!("register {register} should contain a box, got {other:?}"),
        };
        assert!(
            stores.nodes(children).is_empty(),
            "a malformed leader must not append payload or glue"
        );
    }
}

#[test]
fn leader_payloads_participate_in_state_hash_and_rollback() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let snapshot = stores.snapshot();
    let before = snapshot.state_hash();

    stores = run_canonical_tex82_with_universe(
        stores,
        "\\setbox0=\\hbox{\\xleaders\\hbox{\\kern1pt}\\hskip10pt}",
    );
    let with_one_point_payload = stores.snapshot().state_hash();
    assert_ne!(with_one_point_payload, before);

    stores.rollback(&snapshot);
    assert_eq!(stores.snapshot().state_hash(), before);

    stores = run_canonical_tex82_with_universe(
        stores,
        "\\setbox0=\\hbox{\\xleaders\\hbox{\\kern2pt}\\hskip10pt}",
    );
    assert_ne!(stores.snapshot().state_hash(), with_one_point_payload);
}

#[test]
fn showbox_dumps_leader_glue_payloads_like_reference() {
    let stores = run_canonical_tex82(
        "\\showboxbreadth=100 \\showboxdepth=100 \
         \\setbox0=\\hbox{\\leaders\\hbox{\\kern1pt}\\hskip10pt}\\showbox0",
    );

    let log = terminal_effect_text(&stores);
    assert!(log.contains(".\\leaders 10.0"), "{log}");
    assert!(log.contains("..\\hbox"), "{log}");
    assert!(log.contains("...\\kern 1.0"), "{log}");
}

#[test]
fn showbox_and_showeqtb_render_exact_assigned_regions_without_mutation() {
    let mut stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \
         \\f \\textfont1=\\f \\scriptfont2=\\f \\scriptscriptfont3=\\f \
         \\catcode64=11 \\lccode64=97 \\uccode64=65 \
         \\sfcode64=2345 \\mathcode64=12345 \
         \\setbox0=\\hbox{A\\hbox{B}} \\end",
    );

    let box_before = stores.box_reg(0).expect("box register 0");
    let catcode_before = stores.catcode('@');
    let lccode_before = stores.lccode('@');
    let uccode_before = stores.uccode('@');
    let sfcode_before = stores.sfcode('@');
    let mathcode_before = stores.mathcode('@');

    stores = run_canonical_tex82_with_universe(
        stores,
        "\\f \\showboxbreadth=100 \\showboxdepth=100 \\showbox0 \
         \\showboxbreadth=1 \\showboxdepth=0 \\showbox0 \
         \\showthe\\font \\showthe\\textfont1 \\showthe\\scriptfont2 \
         \\showthe\\scriptscriptfont3 \
         \\showthe\\catcode64 \\showthe\\lccode64 \\showthe\\uccode64 \
         \\showthe\\sfcode64 \\showthe\\mathcode64 \\end",
    );

    let log = terminal_effect_text(&stores);
    assert!(log.contains("> \\box0=\n\\hbox"), "{log}");
    assert!(log.contains(".\\f A"), "{log}");
    assert!(log.contains(".\\hbox"), "{log}");
    assert!(log.contains("..\\f B"), "{log}");
    assert!(log.contains(" []"), "{log}");
    for expected in ["> \\f .", "> 11.", "> 97.", "> 65.", "> 2345.", "> 12345."] {
        assert!(log.contains(expected), "missing {expected:?} in {log}");
    }

    assert_eq!(stores.box_reg(0), Some(box_before));
    assert_eq!(stores.catcode('@'), catcode_before);
    assert_eq!(stores.lccode('@'), lccode_before);
    assert_eq!(stores.uccode('@'), uccode_before);
    assert_eq!(stores.sfcode('@'), sfcode_before);
    assert_eq!(stores.mathcode('@'), mathcode_before);
}

#[test]
fn box_motion_uses_tex_web_shift_amount_signs_and_diagnostics() {
    let stores = run_canonical_tex82(
        "\\showboxbreadth=100 \\showboxdepth=100 \
         \\setbox0=\\hbox{\\raise2pt\\hbox{}\\lower3pt\\hbox{}} \
         \\setbox1=\\vbox{\\moveleft4pt\\hbox{}\\moveright5pt\\hbox{}} \
         \\showbox0 \\showbox1",
    );

    let hbox = stores.box_reg(0).expect("hbox register");
    let [Node::HList(hbox)] = stores.nodes(hbox).testing_decoded() else {
        panic!("register 0 should hold an hbox");
    };
    let [Node::HList(raised), Node::HList(lowered)] = stores.nodes(hbox.children).testing_decoded()
    else {
        panic!("hbox should contain raised and lowered boxes");
    };
    assert_eq!(raised.shift.raw(), -2 * Scaled::UNITY);
    assert_eq!(lowered.shift.raw(), 3 * Scaled::UNITY);

    let vbox = stores.box_reg(1).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(vbox).testing_decoded() else {
        panic!("register 1 should hold a vbox");
    };
    let horizontal_shifts: Vec<_> = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::HList(boxed) => Some(boxed.shift.raw()),
            _ => None,
        })
        .collect();
    assert_eq!(horizontal_shifts, [-4 * Scaled::UNITY, 5 * Scaled::UNITY]);

    let log = terminal_effect_text(&stores);
    for shift in ["shifted -2.0", "shifted 3.0", "shifted -4.0", "shifted 5.0"] {
        assert!(log.contains(shift), "missing {shift:?} in {log}");
    }
}

#[test]
fn everypar_replays_through_input_stack_and_mutates_state() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let global = stores.intern("global");
    let count = stores.intern("count");
    let everypar = stores.intern_token_list(&[
        Token::Cs(global.symbol()),
        Token::Cs(count.symbol()),
        Token::Char {
            ch: '0',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '=',
            cat: Catcode::Other,
        },
        Token::Char {
            ch: '7',
            cat: Catcode::Other,
        },
    ]);
    stores.set_tok_param(TokParam::EVERY_PAR, everypar);
    let stores = run_canonical_tex82_with_universe(stores, "x\\par\\end");

    assert_eq!(stores.count(0), 7);
}

#[test]
fn paragraph_end_appends_single_line_through_vertical_spacing() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.set_dimen_param(
        DimenParam::PAR_INDENT,
        tex_state::scaled::Scaled::from_raw(0),
    );
    let baseline = stores.intern_glue(GlueSpec {
        width: tex_state::scaled::Scaled::from_raw(12 * 65_536),
        ..GlueSpec::ZERO
    });
    stores.set_glue_param(GlueParam::BASELINE_SKIP, baseline);
    let stores = run_canonical_tex82_with_universe(
        stores,
        "\\setbox0=\\hbox{}\\ht0=4pt\\dp0=1pt\\setbox1=\\vbox{\\copy0\\par\\copy0}\\end",
    );
    let root = stores.box_reg(1).expect("box1");
    let [Node::VList(vbox)] = stores.nodes(root).testing_decoded() else {
        panic!("box1 should contain a vbox");
    };
    let nodes = stores.nodes(vbox.children).testing_decoded();
    assert!(nodes.iter().any(|node| matches!(
        node,
        tex_state::node::Node::Glue {
            kind: tex_state::node::GlueKind::BaselineSkip,
            ..
        }
    )));
}

#[test]
fn paragraph_hpack_appends_overfull_rule_for_insufficient_normal_shrink() {
    let stores = run_canonical_tex82(concat!(
        "\\setbox0=\\vbox{\\hsize=10pt \\overfullrule=5pt ",
        "\\leftskip=8pt minus4pt \\noindent\\kern9pt\\par}\\end"
    ));

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    let has_rule = stores.nodes(vbox.children).iter().any(|node| {
        let tex_state::node_arena::NodeRef::HList(line) = node else {
            return false;
        };
        stores.nodes(line.children).iter().any(|node| {
            matches!(
                node,
                tex_state::node_arena::NodeRef::Rule {
                    width: Some(width),
                    height: None,
                    depth: None,
                } if width.raw() == 5 * Scaled::UNITY
            )
        })
    });
    assert!(
        has_rule,
        "overfull paragraph line should end in a five-point rule"
    );
}

#[test]
fn paragraph_end_ignores_empty_unindented_paragraph() {
    let stores = run_canonical_tex82("\\setbox0=\\vbox{\\noindent\\par\\indent\\par}\\end");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [Node::HList(_)]
    ));
}

#[test]
fn paragraph_start_resets_prevgraf_and_inserts_parskip_only_at_a_nonempty_boundary() {
    let stores = run_canonical_tex82(concat!(
        "\\parskip=7pt \\everypar{\\global\\count0=\\prevgraf}",
        "\\setbox0=\\vbox{\\prevgraf=9 \\indent\\par \\prevgraf=8 \\indent\\par}\\end"
    ));

    assert_eq!(
        stores.count(0),
        0,
        "every fresh paragraph starts at line zero"
    );
    let root = stores.box_reg(0).expect("box0");
    let [Node::VList(vbox)] = stores.nodes(root).testing_decoded() else {
        panic!("box0 should contain a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
    let paragraph_lines: Vec<_> = children
        .iter()
        .filter_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .collect();
    assert_eq!(paragraph_lines.len(), 2);
    assert!(paragraph_lines.iter().all(|line| matches!(
        stores.nodes(line.children).testing_decoded().first(),
        Some(Node::HList(indent))
            if indent.width == stores.dimen_param(DimenParam::PAR_INDENT)
                && indent.height.raw() == 0
                && indent.depth.raw() == 0
                && stores.nodes(indent.children).is_empty()
    )));
    let parskip_positions: Vec<_> = children
        .iter()
        .enumerate()
        .filter_map(|(index, node)| match node {
            Node::Glue {
                spec,
                kind: tex_state::node::GlueKind::ParSkip,
                ..
            } if stores.glue(*spec).width.raw() == 7 * Scaled::UNITY => Some(index),
            _ => None,
        })
        .collect();
    assert_eq!(parskip_positions.len(), 1);
    let first_line = children
        .iter()
        .position(|node| matches!(node, Node::HList(_)))
        .expect("first paragraph line");
    let second_line = children
        .iter()
        .rposition(|node| matches!(node, Node::HList(_)))
        .expect("second paragraph line");
    assert!(first_line < parskip_positions[0] && parskip_positions[0] < second_line);
}

#[test]
fn paragraph_indent_is_a_null_box_without_a_pack_transition() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.enable_geometry_observation();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control.stop_at_end_of_input();
    run_registered_canonical_tex82(&mut control, &mut stores, "\\parindent=2pt\\indent");

    let [Node::HList(indent)] = control.current_list().nodes() else {
        panic!("paragraph indent should be an hlist");
    };

    assert_eq!(indent.width.raw(), 2 * Scaled::UNITY);
    assert_eq!(indent.height.raw(), 0);
    assert_eq!(indent.depth.raw(), 0);
    assert!(stores.nodes(indent.children).is_empty());
    assert_eq!(
        stores.geometry_observation_len(),
        0,
        "TeX82 §1090 new_null_box is not §649 hpack"
    );
}

#[test]
fn vbox_closing_brace_ends_paragraph_resumed_after_display() {
    let stores = run_canonical_tex82("\\setbox0=\\vbox{\\hrule $$\\hbox{}$$}\\end");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
    assert!(matches!(children.first(), Some(Node::Rule { .. })));
    assert!(children.iter().any(|node| matches!(node, Node::HList(_))));
}

#[test]
fn paragraph_end_removes_only_the_final_trailing_glue() {
    let stores =
        run_canonical_tex82("\\setbox0=\\vbox{\\noindent x\\hskip1pt\\hskip2pt\\par}\\end");

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph should produce a line");
    let explicit_glue: Vec<_> = stores
        .nodes(line.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::Glue {
                spec,
                kind: tex_state::node::GlueKind::Normal,
                ..
            } => Some(stores.glue(*spec).width.raw()),
            _ => None,
        })
        .collect();

    assert_eq!(explicit_glue, [65_536]);
}

#[test]
fn last_items_read_current_horizontal_tail_by_type() {
    let stores = run_canonical_tex82(
        "\\setbox0=\\hbox{\
         \\kern3pt\\xdef\\lk{\\the\\lastkern}\
         \\penalty42\\xdef\\lp{\\the\\lastpenalty}\
         \\hskip1pt plus 2fil\\xdef\\ls{\\the\\lastskip}\
         \\kern4pt\\xdef\\lszero{\\the\\lastskip}}",
    );

    assert_eq!(macro_text(&stores, "lk"), "3.0pt");
    assert_eq!(macro_text(&stores, "lp"), "42");
    assert_eq!(macro_text(&stores, "ls"), "1.0pt plus 2.0fil");
    assert_eq!(macro_text(&stores, "lszero"), "0.0pt");
}

#[test]
fn delete_last_removes_only_matching_current_list_tail() {
    let (stores, nodes) = run_canonical_tex82_current_list(
        "\\vskip1pt\\unpenalty\\edef\\stillglue{\\the\\lastskip}\
         \\unskip\\edef\\noglue{\\the\\lastskip}",
    );

    assert_eq!(macro_text(&stores, "stillglue"), "1.0pt");
    assert_eq!(macro_text(&stores, "noglue"), "0.0pt");
    assert!(nodes.is_empty());
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn vertical_infinite_skip_primitives_preserve_glue_orders() {
    let stores = run_canonical_tex82(
        "\\vfil\\edef\\vfilglue{\\the\\lastskip}\
         \\vfill\\edef\\vfillglue{\\the\\lastskip}\
         \\vss\\edef\\vssglue{\\the\\lastskip}\
         \\vfilneg\\edef\\vfilnegglue{\\the\\lastskip}",
    );

    assert_eq!(macro_text(&stores, "vfilglue"), "0.0pt plus 1.0fil");
    assert_eq!(macro_text(&stores, "vfillglue"), "0.0pt plus 1.0fill");
    assert_eq!(
        macro_text(&stores, "vssglue"),
        "0.0pt plus 1.0fil minus 1.0fil"
    );
    assert_eq!(macro_text(&stores, "vfilnegglue"), "0.0pt plus -1.0fil");
}

#[test]
fn vertical_skip_in_hbox_closes_box_and_retries_in_outer_mode() {
    let stores = run_canonical_tex82("\\setbox0=\\hbox{\\vfill}");

    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("Missing } inserted"));
}

#[test]
fn vertical_skip_in_horizontal_mode_ends_the_paragraph_before_appending_glue() {
    let stores = run_canonical_tex82("\\setbox0=\\vbox{\\noindent\\kern1pt\\vskip2pt\\kern3pt}");

    let box0 = stores.box_reg(0).expect("vbox exists");
    let [Node::VList(outer)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let children = stores.nodes(outer.children).testing_decoded();
    assert!(matches!(children.first(), Some(Node::HList(_))));
    assert!(children.windows(2).any(|nodes| {
        matches!(
            nodes,
            [
                Node::HList(_),
                Node::Glue {
                    spec,
                    kind: tex_state::node::GlueKind::Normal,
                    ..
                }
            ] if stores.glue(*spec).width.raw() == 2 * Scaled::UNITY
        )
    }));
}

#[test]
fn delete_last_outer_vertical_empty_matches_tex_error_asymmetry() {
    let _stores = run_canonical_tex82("\\unskip");

    for (source, command) in [("\\unpenalty", "\\unpenalty"), ("\\unkern", "\\unkern")] {
        let stores = run_canonical_tex82(&format!("{source}\\count0=23"));
        assert!(
            support::terminal_effect_text(&stores)
                .contains(&format!("You can't use `{command}' in vertical mode")),
            "{}",
            support::terminal_effect_text(&stores)
        );
        assert_eq!(stores.count(0), 23, "following assignment must execute");
    }
}

#[test]
fn new_paragraph_resets_prevgraf_before_tracking_finished_lines() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm\
         \\parindent=0pt \\hsize=20pt \\parfillskip=0pt\
         \\prevgraf=5 \\edef\\pg{\\the\\prevgraf}\
         a\\penalty-10000 b\\penalty-10000 c\\par\
         \\edef\\finishedpg{\\the\\prevgraf}",
    );

    assert_eq!(macro_text(&stores, "pg"), "5");
    assert_eq!(macro_text(&stores, "finishedpg"), "3");
}

#[test]
fn negative_prevgraf_is_recoverable_and_leaves_value_unchanged() {
    let stores = run_canonical_tex82("\\prevgraf=3\\prevgraf=-1\\edef\\pg{\\the\\prevgraf}");

    assert_eq!(macro_text(&stores, "pg"), "3");
    assert!(support::terminal_effect_text(&stores).contains("Bad \\prevgraf"));
}

#[test]
fn fresh_hanging_paragraph_keeps_its_first_item_line_at_full_width() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm\
         \\setbox0=\\vbox{\\hsize=100pt \\parindent=20pt \\parfillskip=0pt plus 1fil\
         \\noindent previous\\par\
         \\hangindent=20pt \\indent\
         \\hbox to 0pt{\\hss X\\hskip10pt}first\\penalty-10000 second\\par}",
    );

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let lines = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1].shift.raw(), 0);
    assert_eq!(lines[1].width.raw(), 100 * Scaled::UNITY);
    assert_eq!(lines[2].shift.raw(), 20 * Scaled::UNITY);
    assert_eq!(lines[2].width.raw(), 80 * Scaled::UNITY);
}

#[test]
fn paragraph_hfill_sets_the_line_at_fill_order() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\relax \\tenrm\
         \\setbox0=\\vbox{\\hsize=345pt \\hfill lorem\\par}",
    );

    let box0 = stores.box_reg(0).expect("vbox register");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    let line = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(line),
            _ => None,
        })
        .expect("paragraph line");
    assert_eq!(
        line.glue_sign,
        tex_state::node::Sign::Stretching,
        "line={line:?}, children={:?}",
        stores.nodes(line.children).testing_decoded()
    );
    assert_eq!(
        line.glue_order,
        tex_state::glue::Order::Fill,
        "line={line:?}, children={:?}",
        stores.nodes(line.children).testing_decoded()
    );
    assert!(!line.glue_set.is_zero());
}

#[test]
fn vertical_hrule_uses_defaults_and_sets_prevdepth_ignore_sentinel() {
    let stores = run_canonical_tex82("\\hrule width7pt\\edef\\pd{\\the\\prevdepth}");

    assert_eq!(macro_text(&stores, "pd"), "-1000.0pt");
    let Some(tex_state::node::Node::Rule {
        width,
        height,
        depth,
    }) = stores.page_contributions().front()
    else {
        panic!("recent contributions should contain one rule");
    };
    assert_eq!(stores.page_contributions().len(), 1);
    assert_eq!(width.map(tex_state::scaled::Scaled::raw), Some(7 * 65_536));
    assert_eq!(height.map(tex_state::scaled::Scaled::raw), Some(26_214));
    assert_eq!(depth.map(tex_state::scaled::Scaled::raw), Some(0));
}

#[test]
fn vertical_vrule_runs_everypar_before_scanning_rule_dimensions() {
    let stores = run_canonical_tex82_with_fonts(
        "\\vsize=1000pt \\everypar{\\hangindent=30pt}\\vrule width0pt X\\par",
    );

    let rule = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => stores
                .nodes(line.children)
                .testing_decoded()
                .iter()
                .find_map(|child| match child {
                    Node::Rule { width, .. } => Some(*width),
                    _ => None,
                }),
            _ => None,
        })
        .expect("paragraph line should contain the vertical rule");
    assert_eq!(rule.map(Scaled::raw), Some(0));
    assert!(!terminal_effect_text(&stores).contains("Missing number"));
}

#[test]
fn vertical_char_runs_everypar_before_scanning_and_appending_the_character() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\f=cmr10 \\relax \\f \\vsize=1000pt \\everypar{\\char66 }\\char65 \\par",
    );

    let chars = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            Node::HList(line) => Some(
                stores
                    .nodes(line.children)
                    .testing_decoded()
                    .iter()
                    .filter_map(|child| match child {
                        Node::Char { ch, .. } | Node::Lig { ch, .. } => Some(*ch),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .expect("paragraph should contribute a line");
    assert_eq!(chars, "BA", "page={:?}", stores.current_page_nodes());
}

#[test]
fn hrule_in_restricted_horizontal_mode_reports_and_is_ignored() {
    let stores = run_canonical_tex82("\\setbox0=\\hbox{\\hrule}");

    assert!(stores.box_reg(0).is_some());
    assert!(support::terminal_effect_text(&stores).contains("hrule' here except with leaders"));
}

#[test]
fn showlists_reports_vertical_rule_and_ignored_prevdepth() {
    let stores =
        run_canonical_tex82("\\showboxbreadth=100 \\showboxdepth=100 \\hrule width7pt\\showlists");

    let log = terminal_effect_text(&stores);
    assert!(log.contains("### recent contributions:"));
    assert!(log.contains("\\rule(0.4+0.0)x7.0"));
    assert!(log.contains("prevdepth ignored"));
}

#[test]
fn showlists_preserves_page_goal_row_leading_space() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\vsize=100pt \\topskip=0pt \\setbox0=\\hbox{}\\ht0=2pt \\copy0\\penalty0\\showlists\\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains("total height 2.0\n goal height 100.0\nprevdepth 0.0"),
        "{log}"
    );
}

#[test]
fn showlists_reports_count_scaled_split_page_insertions() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\vsize=5pt \\count7=500 \\dimen7=100pt \\skip7=0pt \\insert7{\\hrule height20pt}\\showlists\\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains("\\insert7 adds 9.9945, #1 might split\n"),
        "{log}"
    );
}

#[test]
fn tracingpages_reports_insertion_split_capacity_height_and_penalty() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\tracingpages=1 \\vsize=5pt \\count7=500 \\dimen7=100pt \\skip7=0pt \\insert7{\\hrule height20pt}\\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(log.contains("% split7 to 9.9945,20.0 p=-10000\n"), "{log}");
}

#[test]
fn end_job_resumes_the_suffix_left_by_a_split_insertion_break() {
    // TeX82 §§1014--1015 resume the page builder after default output before
    // §1054 retries the backed-up `\end`. A null insertion split contributes
    // `-10000`, allowing the filler glue to fire output while the forced
    // penalty is still queued; that suffix must be consumed, not accompanied
    // by another end-job filler trio on every retry.
    let stores = run_canonical_tex82(
        "\\nonstopmode\\vsize=5pt \\count7=500 \\dimen7=100pt \\skip7=0pt \\insert7{\\hrule height20pt}\\end",
    );

    assert_eq!(stores.current_page_len(), 0);
    assert!(stores.page_contributions().is_empty());
    assert!(stores.page_fire_up().is_none());
    assert!(stores.page_insertions().is_empty());
}

#[test]
fn showlists_reports_source_entry_line_and_hyphenation_context() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\language=7\\lefthyphenmin=2\\righthyphenmin=5\nX\\showlists\\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains("### horizontal mode entered at line 2 (language7:hyphenmin2,5)"),
        "{log}"
    );
    assert!(
        log.contains("spacefactor 1000, current language 7"),
        "{log}"
    );
}

#[test]
fn showlists_omits_zero_hyphenation_context() {
    // TeX82 §218's mode-entry line has no parenthetical suffix when the
    // horizontal list carries only the zero language/minima defaults.
    let stores = run_canonical_tex82("\\nonstopmode\\setbox0=\\hbox{\\showlists}\\end");
    let log = terminal_effect_text(&stores);

    assert!(
        log.contains("### restricted horizontal mode entered at line 1\n"),
        "{log}"
    );
    assert!(!log.contains("(language0:hyphenmin0,0)"), "{log}");
}

#[test]
fn showlists_marks_only_the_active_output_routine_context() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\output={\\showlists\\shipout\\box255}\n\\topskip=0pt\\vsize=1pt\\setbox0=\\hbox{}\\ht0=2pt\\copy0\\penalty-10000\n\\showlists\\end",
    );
    let log = terminal_effect_text(&stores);

    assert_eq!(
        log.matches("### internal vertical mode entered at line 2 (\\output routine)")
            .count(),
        1,
        "{log}"
    );
    let active = log
        .find("(\\output routine)")
        .expect("active output-routine marker");
    let later = log[active..]
        .rfind("### vertical mode entered at line 0")
        .expect("post-unwind outer vertical report");
    assert!(later > 0, "{log}");
}

#[test]
fn showlists_marks_a_page_held_during_an_output_routine() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    stores.push_current_page_node(Node::Penalty(0));
    stores.set_output_routine_active(true);
    let stores = run_canonical_tex82_with_universe(
        stores,
        "\\nonstopmode\\showlists",
    );
    let log = terminal_effect_text(&stores);
    assert!(
        log.contains("### current page: (held over for next output)\n\\penalty 0"),
        "{log}"
    );
}

#[test]
fn output_routine_observes_completed_page_shrink() {
    let stores = run_canonical_tex82(
        "\\nonstopmode\\output={\\showthe\\pageshrink\\shipout\\box255}\\topskip=0pt\\vsize=1pt\\hrule height0pt\\vskip0pt minus51pt\\hrule height2pt\\penalty-10000\\end",
    );
    let log = terminal_effect_text(&stores);

    assert!(log.contains("\n> 51.0pt.\n<output>"), "{log}");
}

#[test]
fn macro_parameter_in_vertical_mode_does_not_build_recent_rule() {
    let stores = run_canonical_tex82("\\hrule width7pt#\\showlists");

    assert!(stores.current_page_nodes().is_empty());
    assert_eq!(stores.page_contributions().len(), 1);
    assert!(matches!(
        stores.page_contributions().front(),
        Some(Node::Rule { .. })
    ));
    let log = terminal_effect_text(&stores);
    assert!(log.contains("You can't use `macro parameter character #' in vertical mode"));
    assert!(log.contains("### recent contributions:"));
}

#[test]
fn outer_paragraph_retains_zero_parskip_after_existing_material() {
    let stores = run_canonical_tex82(
        "\\vsize=100pt \\parskip=0pt \\hrule \\noindent\\vrule\\par",
    );

    let page = stores.current_page_nodes();
    assert!(page.windows(2).any(|nodes| {
        matches!(
            nodes,
            [
                Node::Rule { .. },
                Node::Glue {
                    spec,
                    kind: tex_state::node::GlueKind::ParSkip,
                    leader: None,
                },
            ] if stores.glue(*spec) == GlueSpec::ZERO
        )
    }));
}

#[test]
fn vertical_unhbox_of_void_box_still_builds_indented_empty_line() {
    let stores = run_canonical_tex82(
        "\\vsize=100pt \\parskip=0pt \\hrule \\vskip12pt \\unhbox0 \\par",
    );

    assert!(stores.current_page_nodes().iter().any(|node| {
        matches!(
            node,
            Node::HList(line)
                if line.height.raw() == 0
                    && line.depth.raw() == 0
                    && matches!(stores.nodes(line.children).testing_decoded(), [Node::HList(indent), ..] if indent.width == stores.dimen_param(DimenParam::PAR_INDENT))
        )
    }));
}

#[test]
fn page_builder_moves_box_and_updates_page_scalars() {
    let stores = run_canonical_tex82(
        "\\topskip=10pt \\vsize=100pt \\maxdepth=2pt \
         \\setbox0=\\hbox{}\\ht0=7pt \\dp0=3pt \
         \\copy0 \\edef\\snapshot{\\the\\pagegoal,\\the\\pagetotal,\\the\\pagedepth}",
    );

    assert!(stores.page_contributions().is_empty());
    assert_eq!(stores.current_page_nodes().len(), 2);
    assert_eq!(macro_text(&stores, "snapshot"), "100.0pt,11.0pt,2.0pt");
}

/// Concatenates only the text routed to one sink, so a diagnostic's
/// destination is asserted rather than assumed.
fn sink_text(stores: &Universe, wanted: PrintSink) -> String {
    let mut text = String::new();
    for record in stores.world().effect_records() {
        if let EffectRecord::StreamWrite {
            sink,
            text: written,
        } = record
            && *sink == wanted
        {
            text.push_str(written);
        }
    }
    text
}

/// tex.web §987's `%% goal height=` line and §1006's `% t=... c=...#` line.
const TRACING_PAGES_SOURCE: &str = "\\tracingpages=1 \\topskip=10pt \\vsize=100pt \\maxdepth=2pt \
     \\setbox0=\\hbox{}\\ht0=7pt \\dp0=3pt \\copy0 \\penalty100 ";

fn canonical_trace_text(stores: &Universe, sink: PrintSink) -> String {
    let suffix = match sink {
        PrintSink::Log => "\n*** (job aborted, no legal \\end found)\n",
        PrintSink::TerminalAndLog => "! Emergency stop.\n<*> \n    \n",
        other => panic!("unexpected tracing-pages sink {other:?}"),
    };
    sink_text(stores, sink)
        .strip_suffix(suffix)
        .unwrap_or_else(|| panic!("canonical source-exhaustion diagnostic missing from {sink:?}"))
        .to_owned()
}

#[test]
fn tracingpages_reports_the_page_specs_and_break_cost_like_tex_web() {
    let (stores, _) = run_canonical_tex82_current_list(TRACING_PAGES_SOURCE);

    assert_eq!(
        canonical_trace_text(&stores, PrintSink::Log),
        "%% goal height=100.0, max depth=2.0\n% t=11.0 g=100.0 b=10000 p=100 c=100000#\n"
    );
}

#[test]
fn tracingpages_is_off_by_default_and_online_routing_follows_tracingonline() {
    for (prefix, log, terminal_and_log) in [
        ("", "", ""),
        (
            "\\tracingonline=1 ",
            "",
            "%% goal height=100.0, max depth=2.0\n% t=11.0 g=100.0 b=10000 p=100 c=100000#\n",
        ),
    ] {
        let source = if prefix.is_empty() {
            TRACING_PAGES_SOURCE.replace("\\tracingpages=1 ", "")
        } else {
            format!("{prefix}{TRACING_PAGES_SOURCE}")
        };
        let (stores, _) = run_canonical_tex82_current_list(&source);

        assert_eq!(
            canonical_trace_text(&stores, PrintSink::Log),
            log,
            "{source}"
        );
        assert_eq!(
            canonical_trace_text(&stores, PrintSink::TerminalAndLog),
            terminal_and_log,
            "{source}"
        );
    }
}

#[test]
fn page_builder_discards_glue_before_first_box() {
    let (stores, _) = run_canonical_tex82_current_list("\\vskip 5pt\\setbox0=\\hbox{}\\copy0");

    assert!(stores.page_contributions().is_empty());
    assert!(stores.current_page_nodes().iter().all(|node| {
        !matches!(node, tex_state::node::Node::Glue { spec, .. }
        if stores.glue(*spec).width.raw() == 5 * tex_state::scaled::Scaled::UNITY)
    }));
}

#[test]
fn etex_page_discards_save_splice_and_clear_discarded_material() {
    let (stores, _) = run_canonical_etex_current_list(
        "\\savingvdiscards=1 \\vskip5pt \
         \\setbox0=\\hbox{}\\copy0 \
         \\setbox1=\\vbox{\\pagediscards} \
         \\setbox2=\\vbox{\\pagediscards}",
    );

    let first = stores.box_reg(1).expect("first discard box");
    let Node::VList(first) = stores
        .nodes(first)
        .first()
        .expect("first discard vbox")
        .to_owned()
    else {
        panic!("expected vbox");
    };
    assert!(stores.nodes(first.children).into_iter().any(|node| {
        matches!(node.to_owned(), Node::Glue { spec, .. }
            if stores.glue(spec).width.raw() == 5 * Scaled::UNITY)
    }));

    let second = stores.box_reg(2).expect("second discard box");
    let Node::VList(second) = stores
        .nodes(second)
        .first()
        .expect("second discard vbox")
        .to_owned()
    else {
        panic!("expected vbox");
    };
    assert!(stores.nodes(second.children).is_empty());
    assert!(stores.page_discards().is_empty());
}

#[test]
fn etex_vsplit_updates_mark_classes_and_consumes_saved_discards() {
    // e-TeX manual sections 3.4 and 3.7 require classed split marks and make
    // \splitdiscards a destructive splice when \savingvdiscards is positive.
    let (stores, _) = run_canonical_etex_current_list(concat!(
        "\\savingvdiscards=1 ",
        "\\setbox0=\\vbox{\\marks7{A}\\hbox{}\\marks7{B}\\hbox{}\\vskip5pt\\hbox{}} ",
        "\\setbox1=\\vsplit0 to0pt ",
        "\\edef\\splitresult{\\splitfirstmarks7/\\splitbotmarks7} ",
        "\\setbox2=\\vbox{\\splitdiscards} ",
        "\\setbox3=\\vbox{\\splitdiscards}",
    ));

    assert_eq!(macro_text(&stores, "splitresult"), "A/B");
    let first = stores.box_reg(2).expect("first split-discard box");
    let Node::VList(first) = stores
        .nodes(first)
        .first()
        .expect("first split-discard vbox")
        .to_owned()
    else {
        panic!("expected vbox");
    };
    assert!(stores.nodes(first.children).into_iter().any(|node| {
        matches!(node.to_owned(), Node::Glue { spec, .. }
            if stores.glue(spec).width.raw() == 5 * Scaled::UNITY)
    }));
    let second = stores.box_reg(3).expect("second split-discard box");
    let Node::VList(second) = stores
        .nodes(second)
        .first()
        .expect("second split-discard vbox")
        .to_owned()
    else {
        panic!("expected vbox");
    };
    assert!(stores.nodes(second.children).is_empty());
    assert!(stores.split_discards().is_empty());
}

#[test]
fn page_builder_reports_and_normalizes_infinite_shrink_glue() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\topskip=0pt \\vsize=100pt \\setbox0=\\hbox{}\\copy0\
         \\vskip0pt minus 1fil\\copy0",
    );

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found on current page."));
    let page_glue = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => {
                let spec = stores.glue(*spec);
                (spec.shrink.raw() != 0).then_some(spec)
            }
            _ => None,
        })
        .expect("page glue");
    assert_eq!(page_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(page_glue.shrink_order, tex_state::glue::Order::Normal);
}

/// tex.web §825: all offending glue in one paragraph is copied with finite
/// shrink order, but the recovery error is issued only once for the paragraph.
#[test]
fn paragraph_infinite_shrink_reports_once_per_paragraph_and_normalizes_glue() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\tracingparagraphs=1\\tracingonline=1 \\hsize=100pt \\parindent=0pt \\noindent\\vrule width1pt\
         \\hskip1pt minus 1fil\\vrule width1pt\
         \\hskip2pt minus 2fill\\vrule width1pt\\par",
    );

    let terminal = terminal_effect_text(&stores);
    assert_eq!(
        terminal
            .matches("Infinite glue shrinkage found in a paragraph")
            .count(),
        1
    );
    assert!(
        terminal.contains("\n! Infinite glue shrinkage found in a paragraph."),
        "{terminal:?}"
    );
    let line_lists: Vec<_> = stores
        .current_page_nodes()
        .into_iter()
        .filter_map(|node| match node {
            Node::HList(line) => Some(line.children),
            _ => None,
        })
        .collect();
    assert!(!line_lists.is_empty(), "materialized paragraph lines");
    let mut shrinking = Vec::new();
    for children in line_lists {
        for node in stores.nodes(children) {
            if let tex_state::node_arena::NodeRef::Glue { spec, .. } = node {
                let spec = stores.glue(spec);
                if spec.shrink.raw() != 0 {
                    shrinking.push(spec);
                }
            }
        }
    }
    assert_eq!(shrinking.len(), 2);
    assert!(
        shrinking
            .iter()
            .all(|spec| spec.shrink_order == tex_state::glue::Order::Normal)
    );
}

#[test]
fn page_scalars_read_after_page_freeze_and_reject_register_arithmetic() {
    // TeX82 §1236 accepts only assign_int..assign_mu_glue as named
    // arithmetic targets. Page scalars are readable internal quantities, but
    // their direct writes instead belong to §1245/§1246, so the rejected
    // \advance leaves the frozen live page value unchanged.
    let (stores, _) = run_canonical_tex82_current_list(
        "\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \
         \\pagegoal=12pt \\advance\\pagegoal by 3pt \
         \\insertpenalties=4 \\edef\\snapshot{\\the\\pagegoal/\\the\\insertpenalties}",
    );

    assert_eq!(macro_text(&stores, "snapshot"), "12.0pt/4");
    assert!(
        terminal_effect_text(&stores).contains("! You can't use `\\pagegoal' after \\advance.")
    );
}

#[test]
fn insert_node_captures_split_parameters_and_natural_size() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\count7=1000 \\dimen7=100pt \
         \\splittopskip=9pt \\splitmaxdepth=3pt \\floatingpenalty=77 \
         \\insert7{\\vskip2pt\\hrule height5pt depth1pt}",
    );

    let insert = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Some((
                *class,
                *size,
                stores.glue(*split_top_skip),
                *split_max_depth,
                *floating_penalty,
                *content,
            )),
            _ => None,
        })
        .expect("insert node");
    assert_eq!(insert.0, 7);
    assert_eq!(insert.1.raw(), 8 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.2.width.raw(), 9 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.3.raw(), 3 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(insert.4, 77);
    assert_eq!(stores.nodes(insert.5).testing_decoded().len(), 2);
}

#[test]
fn insert_node_snapshots_body_local_split_parameters_before_scope_restoration() {
    let (stores, _) = run_canonical_tex82_current_list(concat!(
        "\\splittopskip=1pt \\splitmaxdepth=2pt \\floatingpenalty=3 ",
        "\\insert7{\\splittopskip=9pt \\splitmaxdepth=4pt ",
        "\\floatingpenalty=77 \\hrule height5pt}"
    ));

    let (split_top_skip, split_max_depth, floating_penalty) = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            Node::Ins {
                split_top_skip,
                split_max_depth,
                floating_penalty,
                ..
            } => Some((*split_top_skip, *split_max_depth, *floating_penalty)),
            _ => None,
        })
        .expect("insert node");
    assert_eq!(stores.glue(split_top_skip).width.raw(), 9 * Scaled::UNITY);
    assert_eq!(split_max_depth.raw(), 4 * Scaled::UNITY);
    assert_eq!(floating_penalty, 77);
    assert_eq!(
        stores
            .glue(stores.glue_param(GlueParam::SPLIT_TOP_SKIP))
            .width
            .raw(),
        Scaled::UNITY
    );
    assert_eq!(
        stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH).raw(),
        2 * Scaled::UNITY
    );
    assert_eq!(stores.int_param(IntParam::FLOATING_PENALTY), 3);
}

#[test]
fn vertical_list_preserves_structured_mark_penalty_and_material_order() {
    let (stores, _) = run_canonical_tex82_current_list(concat!(
        "\\setbox0=\\vbox{\\mark{A}\\penalty11\\kern2pt",
        "\\hrule height3pt\\insert7{\\penalty22}\\mark{B}\\penalty33}"
    ));

    let root = stores.box_reg(0).expect("box0");
    let [Node::VList(vbox)] = stores.nodes(root).testing_decoded() else {
        panic!("box0 should contain a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
    assert!(matches!(
        children,
        [
            Node::Mark { class: 0, .. },
            Node::Penalty(11),
            Node::Kern { amount, .. },
            Node::Rule { height: Some(height), .. },
            Node::Ins { class: 7, .. },
            Node::Mark { class: 0, .. },
            Node::Penalty(33),
        ] if amount.raw() == 2 * Scaled::UNITY && height.raw() == 3 * Scaled::UNITY
    ));
    let [Node::Ins { content, .. }] = &children[4..5] else {
        unreachable!("fifth node is the insertion")
    };
    assert!(matches!(
        stores.nodes(*content).testing_decoded(),
        [Node::Penalty(22)]
    ));
}

#[test]
fn explicit_hbox_migrates_vadjust_material_to_enclosing_vlist() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\setbox0=\\vbox{\\hbox{\\vadjust{\\penalty123}}}",
    );

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    let children = stores.nodes(vbox.children).testing_decoded();
    assert!(matches!(children, [Node::HList(_), Node::Penalty(123)]));
    let Node::HList(hbox) = &children[0] else {
        unreachable!()
    };
    assert!(stores.nodes(hbox.children).is_empty());
}

#[test]
fn nested_hbox_retains_vadjust_through_incompatible_unhbox() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\setbox10=\\vbox to8192pt{\\hbox{\\hbox{\\vadjust{A}}}}%\n\\vrule\\unhbox10\\hrule",
    );

    let root = stores
        .box_reg(10)
        .expect("incompatible unhbox leaves box10 intact");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box10 should remain a vbox");
    };
    let Some(tex_state::node_arena::NodeRef::HList(outer)) = stores.nodes(vbox.children).first()
    else {
        panic!("vbox should retain its outer hbox");
    };
    let Some(tex_state::node_arena::NodeRef::HList(inner)) = stores.nodes(outer.children).first()
    else {
        panic!("outer hbox should retain its inner hbox");
    };
    assert!(matches!(
        stores.nodes(inner.children).first(),
        Some(tex_state::node_arena::NodeRef::Adjust(_))
    ));
    assert!(
        !stores
            .current_page_nodes()
            .iter()
            .any(|node| matches!(node, Node::VList(_)))
    );
}

#[test]
fn empty_negative_width_hbox_does_not_gain_an_overfull_rule() {
    let (stores, _) = run_canonical_tex82_current_list(
        "\\overfullrule=5pt \\setbox0=\\hbox to -10pt{}",
    );

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert_eq!(hbox.width.raw(), -655_360);
    assert!(stores.nodes(hbox.children).is_empty());
}

#[test]
fn vertical_mode_discretionary_hyphen_starts_a_paragraph() {
    let (stores, _) = run_canonical_tex82_current_list("\\setbox0=\\vbox{\\-\\par}");

    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vbox");
    };
    assert!(matches!(
        stores.nodes(vbox.children).testing_decoded(),
        [Node::HList(_)]
    ));
}

#[test]
fn insertion_starts_with_normal_paragraph_parameters() {
    let stores = run_canonical_tex82(concat!(
        "\\hsize=100pt ",
        "\\hangindent=99pt \\hangafter=0 \\looseness=2 ",
        "\\insert7{a b c d e f g h i j}"
    ));

    let (size, content) = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { size, content, .. } => Some((*size, *content)),
            _ => None,
        })
        .expect("insert node");
    assert!(matches!(
        stores.nodes(content).testing_decoded(),
        [tex_state::node::Node::HList(_)]
    ));
    assert!(size.raw() < 20 * tex_state::scaled::Scaled::UNITY);
    assert_eq!(
        stores.dimen_param(DimenParam::HANG_INDENT).raw(),
        99 * tex_state::scaled::Scaled::UNITY
    );
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 0);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), 2);
}

#[test]
fn vtop_normalizes_paragraph_parameters_locally_before_display() {
    let mut stores = stores_with_fonts();
    let source = concat!(
        "\\font\\f=cmr10 \\f \\hsize=100pt ",
        "\\parshape=1 3pt 13pt \\hangindent=-10pt \\hangafter=-12 \\looseness=-2 ",
        "\\setbox0=\\vtop{\\noindent$$$$}\\end"
    );
    let checkpoint = stores.snapshot();

    stores = run_canonical_tex82_with_fonts_and_universe(stores, source);

    let first_hash = stores.snapshot().state_hash();
    let root = stores.box_reg(0).expect("box0");
    let Some(tex_state::node_arena::NodeRef::VList(vtop)) = stores.nodes(root).first() else {
        panic!("box0 should contain a vtop");
    };
    assert_eq!(vtop.width.raw(), 50 * Scaled::UNITY);
    let display = stores
        .nodes(vtop.children)
        .iter()
        .find_map(|node| match node {
            tex_state::node_arena::NodeRef::HList(node)
                if node.box_lr == tex_state::node::BoxLr::DList =>
            {
                Some(node)
            }
            _ => None,
        })
        .expect("display box");
    assert_eq!(display.width.raw(), 0);
    assert_eq!(display.shift.raw(), 50 * Scaled::UNITY);

    // begin_box's normal_paragraph assignments are local to the box group.
    assert_eq!(stores.paragraph_shape()[0].indent.raw(), 3 * Scaled::UNITY);
    assert_eq!(
        stores.dimen_param(DimenParam::HANG_INDENT).raw(),
        -10 * Scaled::UNITY
    );
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), -12);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), -2);

    stores.rollback(&checkpoint);
    stores = run_canonical_tex82_with_fonts_and_universe(stores, source);
    assert_eq!(stores.snapshot().state_hash(), first_hash);
}

#[test]
fn insertion_omits_parskip_before_first_internal_vlist_paragraph() {
    let stores = run_canonical_tex82_with_fonts("\\parskip=12pt \\insert7{x}");

    let content = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { content, .. } => Some(*content),
            _ => None,
        })
        .expect("insert node");
    assert!(matches!(
        stores.nodes(content).testing_decoded(),
        [tex_state::node::Node::HList(_)]
    ));
}

#[test]
fn insertion_skip_reports_infinite_shrink_correction() {
    let stores = run_canonical_tex82(
        "\\topskip=0pt \\vsize=100pt \\count7=1000 \\dimen7=100pt \
         \\skip7=0pt minus 1fil \\insert7{\\hrule height1pt}",
    );

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage inserted from \\skip7."));
    assert_eq!(
        stores
            .page_dimension(tex_state::page::PageDimension::Shrink)
            .raw(),
        tex_state::scaled::Scaled::UNITY
    );
}

#[test]
fn split_insertion_reports_and_normalizes_infinite_shrink_content() {
    let stores = run_canonical_tex82(
        "\\topskip=0pt \\vsize=20pt \\count7=1000 \\dimen7=12pt \
         \\insert7{\\hrule height8pt\\vskip0pt minus 1fil\\penalty123\\hrule height8pt}",
    );

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found in box being split."));
    let content = stores
        .current_page_nodes()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Ins { content, .. } => Some(*content),
            _ => None,
        })
        .expect("insert content");
    let split_glue = stores
        .nodes(content)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => Some(stores.glue(*spec)),
            _ => None,
        })
        .expect("split glue");
    assert_eq!(split_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(split_glue.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn vsplit_reports_and_normalizes_infinite_shrink_glue() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    run_registered_canonical_tex82(
        &mut control,
        &mut stores,
        "\\setbox0=\\vbox{\\hrule height10pt\\vskip0pt minus 1fil\\hrule height10pt}",
    );
    let before = stores.testing_epoch_clone_counts();
    run_registered_canonical_tex82(
        &mut control,
        &mut stores,
        "\\setbox1=\\vsplit0 to 30pt\\end",
    );
    assert_eq!(stores.testing_epoch_clone_counts(), before);

    let log = terminal_effect_text(&stores);
    assert!(log.contains("! Infinite glue shrinkage found in box being split."));
    let box1 = stores.box_reg(1).expect("split box");
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box1).testing_decoded() else {
        panic!("box1 should be a vbox");
    };
    let split_glue = stores
        .nodes(box_node.children)
        .testing_decoded()
        .iter()
        .find_map(|node| match node {
            tex_state::node::Node::Glue { spec, .. } => Some(stores.glue(*spec)),
            _ => None,
        })
        .expect("split glue");
    assert_eq!(split_glue.shrink.raw(), tex_state::scaled::Scaled::UNITY);
    assert_eq!(split_glue.shrink_order, tex_state::glue::Order::Normal);
}

#[test]
fn vsplit_recovers_a_missing_to_keyword() {
    let stores = run_canonical_tex82("\\setbox0=\\vbox{}\\setbox1=\\vsplit0 0pt\\end");

    assert!(support::terminal_effect_text(&stores).contains("Missing `to' inserted"));
}

#[test]
fn vsplit_leaves_hbox_source_untouched_and_returns_void() {
    let stores = run_canonical_tex82("\\setbox3=\\hbox{}\\setbox4=\\vsplit3 to 0pt\\end");

    assert!(stores.box_reg(3).is_some());
    assert!(stores.box_reg(4).is_none());
    assert!(support::terminal_effect_text(&stores).contains("vsplit needs a \\vbox"));
}

#[test]
fn show_macro_renders_parameter_tokens_and_replacement_exactly_without_mutation() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    run_registered_canonical_tex82(
        &mut control,
        &mut stores,
        "\\def\\pair#1#2{#1 #2}\\def\\nine#1#2#3#4#5#6#7#8#9{#9}\
         \\def\\hash{##}\\def\\empty{}\\def\\prefix abc#1{[#1]}",
    );
    let before: Vec<_> = ["pair", "nine", "hash", "empty", "prefix"]
        .into_iter()
        .map(|name| {
            let meaning = stores.meaning(stores.symbol(name).expect("defined macro"));
            let Meaning::Macro { definition, .. } = meaning else {
                panic!("{name} should be a macro")
            };
            (meaning, stores.macro_definition(definition))
        })
        .collect();

    run_registered_canonical_tex82(
        &mut control,
        &mut stores,
        "\\show\\pair\\show\\nine\\show\\hash\\show\\empty\\show\\prefix",
    );

    let output = support::terminal_effect_text_unbroken(&stores);
    for exact in [
        "> \\pair=macro:\n#1#2->#1 #2.",
        "> \\nine=macro:\n#1#2#3#4#5#6#7#8#9->#9.",
        "> \\hash=macro:\n->##.",
        "> \\empty=macro:\n->.",
        "> \\prefix=macro:\nabc#1->[#1].",
    ] {
        assert!(
            output.contains(exact),
            "missing exact meaning {exact:?} in {output:?}"
        );
    }

    let after: Vec<_> = ["pair", "nine", "hash", "empty", "prefix"]
        .into_iter()
        .map(|name| {
            let meaning = stores.meaning(stores.symbol(name).expect("defined macro"));
            let Meaning::Macro { definition, .. } = meaning else {
                panic!("{name} should remain a macro")
            };
            (meaning, stores.macro_definition(definition))
        })
        .collect();
    assert_eq!(
        after, before,
        "show must retain the definition and both token-list handles"
    );
}

/// TeX82 §295 can print `CLOBBERED.` after `show_token_list` follows a damaged
/// linked-memory pointer. Umber has no corresponding traversal: a macro owns
/// two generation-tagged `TokenListId`s, `intern_macro` rejects either handle
/// unless it is live in the same `Universe`, and diagnostic rendering resolves
/// each live handle directly to an immutable token slice.
///
/// This is permanently unrepresentable by the safe state model. A constructor
/// for a stale/foreign handle or a mutable corruption hook would weaken the
/// production invariant. Injecting a synthetic failure only into the renderer
/// would instead test a branch that no production state can reach, rather than
/// TeX82's damaged-link behavior. Keep this exact-output case as an explicit
/// xfail unless the production token-list representation itself gains a safe,
/// fallible traversal corresponding to tex.web's links.
#[test]
#[ignore = "permanent xfail: validated immutable TokenListId has no malformed link traversal"]
fn show_macro_renders_clobbered_token_list_marker() {
    panic!("unrepresentable TeX82 §295 output: CLOBBERED.")
}

#[test]
fn insertion_page_goal_uses_skip_once_and_count_scaling() {
    let stores = run_canonical_tex82(
        "\\topskip=0pt \\vsize=100pt \
         \\count7=500 \\dimen7=100pt \\skip7=10pt \
         \\insert7{\\hrule height20pt depth0pt}\
         \\edef\\firstpenalties{\\the\\insertpenalties}\
         \\insert7{\\hrule height10pt depth0pt}\
         \\edef\\secondpenalties{\\the\\insertpenalties}",
    );

    assert_eq!(macro_text(&stores, "firstpenalties"), "0");
    assert_eq!(macro_text(&stores, "secondpenalties"), "0");
    assert_eq!(
        stores
            .page_dimension(tex_state::page::PageDimension::Goal)
            .raw(),
        75 * tex_state::scaled::Scaled::UNITY + 540
    );
}

#[test]
fn split_insertion_penalty_is_mainline_then_heldover_count_in_output() {
    let stores = run_canonical_tex82(
        "\\topskip=0pt \\vsize=20pt \\count7=1000 \\dimen7=12pt \
         \\output={\\xdef\\held{\\the\\insertpenalties}\\shipout\\box255}\
         \\insert7{\\hrule height8pt depth0pt\\penalty123\\hrule height8pt depth0pt}\
         \\edef\\main{\\the\\insertpenalties}\
         \\setbox0=\\hbox{}\\copy0\\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(macro_text(&stores, "main"), "123");
    assert_eq!(macro_text(&stores, "held"), "1");

    let box7 = stores.box_reg(7).expect("insertion box");
    let [tex_state::node::Node::VList(box_node)] = stores.nodes(box7).testing_decoded() else {
        panic!("box7 should be a vbox");
    };
    assert!(
        stores
            .nodes(box_node.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, tex_state::node::Node::Rule { .. })),
        "split-off insertion material should be appended to box7"
    );
}

#[test]
fn forced_page_penalty_runs_default_output() {
    let stores = run_canonical_tex82("\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000");

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(stores.box_reg(255).is_none());
    assert!(stores.page_fire_up().is_none());
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert_eq!(
        stores.page_dimension(tex_state::page::PageDimension::Goal),
        tex_state::scaled::Scaled::MAX_DIMEN
    );
}

#[test]
fn page_output_promotes_nested_survivor_children_into_one_root() {
    let stores = run_canonical_tex82(
        "\\output={\\global\\setbox2=\\copy255 \\shipout\\box255}\
         \\topskip=0pt \\setbox0=\\hbox{X}\\copy0 \\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 1);
    let root = stores
        .box_reg(2)
        .expect("output routine should retain page copy");
    let ArenaRef::Survivor(root_id) = root.arena() else {
        panic!("retained page should be survivor-owned");
    };
    let mut pending = vec![root];
    let mut nested_boxes = 0;
    while let Some(list) = pending.pop() {
        for node in stores.nodes(list).testing_decoded() {
            if let Node::HList(box_node) | Node::VList(box_node) = node {
                let ArenaRef::Survivor(child_root) = box_node.children.arena() else {
                    panic!("promoted page contains an epoch child");
                };
                assert_eq!(child_root, root_id);
                nested_boxes += 1;
                pending.push(box_node.children);
            }
        }
    }
    assert!(
        nested_boxes >= 2,
        "page should retain packed and source boxes"
    );
}

#[test]
fn page_output_keeps_locally_moved_box_children_live() {
    let stores = run_canonical_tex82("\\topskip=0pt {\\setbox0=\\hbox{X}\\box0} \\penalty-10000");

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(stores.box_reg(0).is_none());
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn page_output_keeps_shifted_copy_children_live_after_source_replacement() {
    let stores = run_canonical_tex82(
        "\\topskip=0pt \\setbox0=\\hbox{X} \\setbox1=\\hbox{\\raise1pt\\copy0} \\box1 \
         \\setbox0=\\hbox{Y} \\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn mark_scans_raw_general_text_then_expands_payload() {
    let stores = run_canonical_tex82("\\def\\a{A}\\mark{#\\a}");

    let current_page = stores.current_page_nodes();
    let mark = current_page
        .iter()
        .chain(stores.page_contributions())
        .find_map(|node| match node {
            tex_state::node::Node::Mark { tokens, .. } => Some(*tokens),
            _ => None,
        })
        .expect("mark node");
    assert_eq!(
        stores.tokens(mark),
        &[
            Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            },
            Token::Char {
                ch: 'A',
                cat: Catcode::Letter,
            },
        ]
    );
}

#[test]
fn etex_marks_appends_the_scanned_mark_class() {
    let stores = run_canonical_etex(r"\marks27{classed}\marks-1{class-zero}");

    let current_page = stores.current_page_nodes();
    let mark = current_page
        .iter()
        .chain(stores.page_contributions())
        .find(|node| matches!(node, Node::Mark { class: 27, .. }));
    assert!(mark.is_some());
    assert!(
        stores
            .current_page_nodes()
            .iter()
            .chain(stores.page_contributions())
            .any(|node| matches!(node, Node::Mark { class: 0, .. }))
    );
    assert!(terminal_effect_text(&stores).contains("Bad register code (-1)"));
}

#[test]
fn fire_up_updates_top_first_bot_marks_across_no_mark_page() {
    let stores = run_canonical_tex82(
        "\\output={\\global\\advance\\count0 by 1 \
         \\ifnum\\count0=1 \\xdef\\pagea{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=2 \\xdef\\pageb{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=3 \\xdef\\pagec{\\topmark/\\firstmark/\\botmark}\
         \\else\\ifnum\\count0=4 \\xdef\\paged{\\topmark/\\firstmark/\\botmark}\
         \\else \\xdef\\pagee{\\topmark/\\firstmark/\\botmark}\\fi\\fi\\fi\\fi \
         \\shipout\\box255}\
         \\topskip=0pt \\vsize=1pt \\setbox0=\\hbox{}\\ht0=2pt \
         \\mark{A}\\copy0\\penalty-10000 \
         \\copy0\\penalty-10000 \
         \\mark{B}\\copy0\\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 5);
    assert_eq!(macro_text(&stores, "pagea"), "/A/A");
    assert_eq!(macro_text(&stores, "pageb"), "A/A/A");
    assert_eq!(macro_text(&stores, "pagec"), "A/A/A");
    assert_eq!(macro_text(&stores, "paged"), "A/B/B");
    assert_eq!(macro_text(&stores, "pagee"), "B/B/B");
}

#[test]
fn fire_up_tracks_etex_mark_classes_independently() {
    let stores = run_canonical_etex(
        "\\output={\\global\\advance\\count0 by 1 \\
         \\ifnum\\count0=1 \\xdef\\pagea{\\topmarks7/\\firstmarks7/\\botmarks7}\\else \\
         \\xdef\\pageb{\\topmarks7/\\firstmarks7/\\botmarks7}\\fi \\shipout\\box255}\\
         \\topskip=0pt \\vsize=1pt \\setbox0=\\hbox{}\\ht0=2pt \\
         \\marks7{A}\\copy0\\penalty-10000",
    );

    assert_eq!(macro_text(&stores, "pagea"), "/A/A");
    assert_eq!(macro_text(&stores, "pageb"), "A/A/A");
}

#[test]
fn output_routine_replays_in_implicit_group_and_consumes_box255() {
    let stores = run_canonical_tex82(
        "\\output={\\advance\\count0 by 1 \\global\\advance\\count1 by 1 \\shipout\\box255}\
         \\count0=10 \\count1=20 \
         \\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(
        stores.count(0),
        10,
        "plain assignments in \\output are local"
    );
    assert_eq!(
        stores.count(1),
        21,
        "global assignments in \\output survive"
    );
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
    assert!(stores.box_reg(255).is_none());
}

#[test]
fn expandable_output_tail_cannot_consume_following_float_group() {
    let stores = run_canonical_tex82(
        "\\catcode64=11 \
         \\def\\outputtail{} \
         \\output={\\let\\relax\\outputtail\\shipout\\box255} \
         \\topskip=0pt \\vsize=1pt \\setbox0=\\hbox{}\\ht0=2pt \
         \\copy0\\penalty-10000 \
         \\begingroup\\def\\@currbox{alive}\\def\\expected{alive}\
         \\ifx\\@currbox\\expected\\global\\count2=1\\else\\global\\count2=2\\fi\
         \\endgroup\\end",
    );

    assert_eq!(stores.count(2), 1);
    let currbox = stores.symbol("@currbox").expect("float-like symbol");
    assert_eq!(
        stores.meaning(currbox),
        tex_state::meaning::Meaning::Undefined
    );
}

#[test]
fn output_routine_emits_one_checkpoint_only_after_teardown() {
    let source = "\\output={\\advance\\count0 by 1 \\
                  \\global\\advance\\count1 by 1 \\shipout\\hbox{}\\shipout\\box255}
                  \\count0=10 \\count1=20
                  \\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000";
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical source");
    let mut checkpoints = Vec::new();
    let mut pending_boundaries = Vec::new();
    for _ in 0..1024 {
        let step = control.step(&mut stores).expect("canonical output step");
        pending_boundaries.extend(control.take_completed_boundaries());
        while let Some(&boundary) = pending_boundaries.first() {
            let Ok(checkpoint) = control.capture_checkpoint(
                boundary,
                &mut stores,
                ExecutionBudgetCounters::default(),
            ) else {
                break;
            };
            checkpoints.push(checkpoint);
            pending_boundaries.remove(0);
        }
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            break;
        }
    }
    assert!(pending_boundaries.is_empty());

    let shipouts = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.boundary() == EngineBoundary::ShipoutComplete)
        .collect::<Vec<_>>();
    assert_eq!(shipouts.len(), 1);
    let checkpoint = shipouts[0];
    assert_eq!(checkpoint.mode_summary().levels().len(), 1);
    assert_eq!(stores.count(0), 10, "output local was restored");
    assert_eq!(stores.count(1), 21, "output global survived");
    assert!(stores.box_reg(255).is_none(), "output box was consumed");

    stores.set_count(1, 99);
    control
        .restore_checkpoint(checkpoint, &mut stores)
        .expect("post-output checkpoint restores");
    assert_eq!(stores.count(1), 21);
}

#[test]
fn shipout_without_output_routine_publishes_at_its_own_quiescent_boundary() {
    let source = "\\shipout\\hbox{}\\count2=7";
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical source");
    let mut checkpoints = Vec::new();
    loop {
        let step = control.step(&mut stores).expect("canonical shipout step");
        for boundary in control.take_completed_boundaries() {
            checkpoints.push(
                control
                    .capture_checkpoint(boundary, &mut stores, ExecutionBudgetCounters::default())
                    .expect("direct shipout boundary is immediately quiescent"),
            );
        }
        if matches!(step, MainControlStep::End | MainControlStep::EndOfInput) {
            break;
        }
    }

    let shipouts = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.boundary() == EngineBoundary::ShipoutComplete)
        .collect::<Vec<_>>();
    assert_eq!(shipouts.len(), 1);
    assert_eq!(shipouts[0].mode_summary().levels().len(), 1);
    control
        .restore_checkpoint(shipouts[0], &mut stores)
        .expect("direct shipout checkpoint restores");
    assert_eq!(stores.count(2), 0, "checkpoint precedes the next command");
}

#[test]
fn lastbox_reappend_runs_page_builder_before_enclosing_group_ends() {
    let stores = run_canonical_tex82_with_fonts(
        "\\font\\tenrm=cmr10 \\font\\tt=cmtt10 \\tenrm \
         \\topskip=0pt \\vsize=1pt \
         \\output={\\global\\advance\\count1 by 1 \
           \\ifnum\\count1=1 \\global\\dimen1=1em\\fi \
           \\shipout\\box255} \
         \\setbox0=\\vbox{\\hbox{}\\penalty-10000\\hbox{}} \
         {\\tt \\unvbox0\\lastbox} \
         \\end",
    );

    assert!(!stores.world().artifact_commits().is_empty());
    let typewriter = support::font_meaning(&stores, "tt");
    assert_eq!(stores.dimen(1), stores.font_parameter(typewriter, 6));
}

#[test]
fn output_routine_reports_nonvoid_box255_after_output() {
    let stores = run_canonical_tex82(
        "\\output={\\relax}\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\penalty-10000",
    );

    assert!(
        support::terminal_effect_text(&stores)
            .contains("Output routine didn't use all of \\box255")
    );
}

#[test]
fn deadcycles_overflow_reports_output_loop() {
    let stores = run_canonical_tex82(
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\
         \\topskip=0pt \\setbox0=\\hbox{}\
         \\copy0 \\penalty-10000 \
         \\copy0 \\penalty-10000",
    );

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(
        String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("memory output")
        )
        .contains("Output loop---1 consecutive dead cycles")
    );
}

#[test]
fn output_unbalanced_group_reports_and_recovers_canonically() {
    // TeX82 §§1026--1028: §1226's safety brace reaches `off_save` while the
    // routine's simple group is still open. The inserted closer ends that
    // group, then the backed-up safety brace ends the output group normally.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\output={\\begingroup\\global\\count1=37\\shipout\\box255}\\
              topskip=0pt\\setbox0=\\hbox{}\\copy0\\penalty-10000\\end"
                .to_vec(),
        ))
        .expect("register canonical source");

    for _ in 0..256 {
        if control.step(&mut stores).expect("canonical recovery step") == MainControlStep::End {
            break;
        }
    }

    assert_eq!(stores.count(1), 37);
    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(stores.box_reg(255).is_none());
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
    assert!(
        terminal_effect_text(&stores).contains("Extra }, or forgotten \\endgroup"),
        "canonical off_save diagnostic missing: {}",
        terminal_effect_text(&stores)
    );
}

#[test]
fn end_cleanup_ejects_residual_page() {
    let stores = run_canonical_tex82("\\topskip=0pt \\setbox0=\\hbox{}\\copy0 \\end");

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(stores.world().committed_artifacts().len(), 1);
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert!(stores.page_fire_up().is_none());
    assert!(stores.box_reg(255).is_none());
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
}

#[test]
fn end_cleanup_exposes_tex_its_all_over_penalty_to_output_routine() {
    let stores = run_canonical_tex82(
        "\\output={\\global\\count0=\\outputpenalty \\shipout\\box255}\\
         \\setbox0=\\hbox{}\\copy0\\end",
    );

    assert_eq!(stores.count(0), -1_073_741_824);
    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(stores.world().committed_artifacts().len(), 1);
    assert!(stores.box_reg(255).is_none());
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert!(stores.page_fire_up().is_none());
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
}

#[test]
fn canonical_dead_cycle_escape_ships_the_selected_residual_page() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"\\maxdeadcycles=0 \\output={\\relax} \\topskip=0pt \\
              \\setbox0=\\hbox{}\\copy0\\end"
                .to_vec(),
        ))
        .expect("register canonical source");

    for _ in 0..128 {
        if control.step(&mut stores).expect("canonical step") == MainControlStep::End {
            break;
        }
    }

    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert_eq!(stores.world().committed_artifacts().len(), 1);
    let artifact = stores
        .world()
        .committed_artifacts()
        .first()
        .expect("canonical dead-cycle escape commits one page");
    let page = tex_out::PageArtifact::from_bytes(artifact.bytes()).expect("artifact parses");
    assert!(matches!(page.root, tex_out::PageNode::VList(_)));
    assert!(stores.box_reg(255).is_none());
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
    assert!(stores.page_fire_up().is_none());
    assert_eq!(
        stores.page_integer(tex_state::page::PageInteger::DeadCycles),
        0
    );
    assert!(terminal_effect_text(&stores).contains("Output loop---0 consecutive dead cycles"));
}

/// Runs `source` through canonical main control until it stops asking for
/// tokens, then returns the mode nest's current list unfinished.
///
/// `run_canonical_tex82` only exposes state that survives `\end`, which cannot
/// show the shape of a list still under construction -- an mlist, for
/// instance, is converted away the moment its formula closes.
pub(super) fn run_canonical_tex82_current_list(source: &str) -> (Universe, Vec<Node>) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical source");
    for _ in 0..1024 {
        if control.step(&mut stores).expect("canonical step") != MainControlStep::Continue {
            let nodes = control.current_list().nodes().to_vec();
            return (stores, nodes);
        }
    }
    panic!("canonical source did not stop consuming input");
}

pub(super) fn run_canonical_tex82(source: &str) -> Universe {
    run_canonical_tex82_with_universe(crate::test_harness::universe_with_plain_catcodes(), source)
}

pub(super) fn run_canonical_tex82_with_fonts(source: &str) -> Universe {
    run_canonical_tex82_with_fonts_and_universe(stores_with_fonts(), source)
}

fn run_canonical_tex82_with_fonts_and_universe(mut stores: Universe, source: &str) -> Universe {
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    for name in ["cmr10.tfm", "cmmi10.tfm", "cmtt10.tfm"] {
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
            std::path::Path::new(name),
        )
        .expect("seeded font fixture reads");
        control.capabilities_mut().register_font(
            name,
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
    }
    run_registered_canonical_tex82(&mut control, &mut stores, source);
    stores
}

pub(super) fn run_canonical_tex82_with_universe(mut stores: Universe, source: &str) -> Universe {
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    run_registered_canonical_tex82(&mut control, &mut stores, source);
    stores
}

fn run_canonical_tex82_with_inputs(source: &str, inputs: &[(&str, &[u8])]) -> Universe {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::tex82_initex(&mut stores);
    for (name, contents) in inputs {
        // TeX82 §537 applies the default `.tex` extension before asking the
        // host for a source. Register the exact request key while keeping the
        // fixture table concise.
        let requested_name = if name.ends_with(".tex") {
            (*name).to_owned()
        } else {
            format!("{name}.tex")
        };
        control.capabilities_mut().register_input(
            requested_name,
            SourceRegistration::new(RegisteredSourceKind::World, contents.to_vec()),
        );
    }
    run_registered_canonical_tex82(&mut control, &mut stores, source);
    stores
}

fn run_registered_canonical_tex82(
    control: &mut CanonicalMainControl,
    stores: &mut Universe,
    source: &str,
) {
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical source");
    for _ in 0..1024 {
        if matches!(
            control.step(stores).expect("canonical step"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            return;
        }
    }
    panic!("canonical source did not terminate");
}

#[test]
fn canonical_valign_noalign_preserves_the_alignment_mode() {
    let stores =
        run_canonical_tex82(r"\valign{#\cr a\cr\noalign{\spacefactor=1}}\global\count0=7\end");

    assert_eq!(stores.count(0), 7);
}

pub(super) fn run_canonical_etex(source: &str) -> Universe {
    run_canonical_extended_profile(source, CommandProfile::ETEX26)
}

fn run_canonical_extended_profile(source: &str, profile: CommandProfile) -> Universe {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::prepared_initex(profile);
    install_tex82_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    match profile {
        CommandProfile::ETEX26 => {}
        CommandProfile::PDFTEX14027 => {
            tex_expand::install_pdftex_expandable_primitives(&mut stores);
        }
        _ => panic!("extended-profile helper requires e-TeX or pdfTeX"),
    }
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical e-TeX source");
    for _ in 0..1024 {
        if matches!(
            control.step(&mut stores).expect("canonical e-TeX step"),
            MainControlStep::End | MainControlStep::EndOfInput
        ) {
            return stores;
        }
    }
    panic!("canonical extended-profile source did not terminate");
}

#[test]
fn canonical_etex_glue_component_enquiries_recover_standalone_in_every_mode() {
    let stores = run_canonical_etex(
        r"\nonstopmode
          \count0=0
          \gluestretchorder \advance\count0 by1
          \hbox{\glueshrinkorder \global\advance\count0 by2}
          \vbox{\gluestretch \global\advance\count0 by4}
          $\glueshrink \global\advance\count0 by8$
          \end",
    );

    assert_eq!(stores.count(0), 15);
    let output = terminal_effect_text(&stores);
    for name in [
        "gluestretchorder",
        "glueshrinkorder",
        "gluestretch",
        "glueshrink",
    ] {
        assert!(
            output.contains(&format!("You can't use `\\{name}' in")),
            "missing standalone last_item recovery for \\{name}: {output:?}"
        );
    }
}

pub(super) fn run_canonical_etex_current_list(source: &str) -> (Universe, Vec<Node>) {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    let mut control = CanonicalMainControl::prepared_initex(CommandProfile::ETEX26);
    install_tex82_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            source.as_bytes().to_vec(),
        ))
        .expect("register canonical e-TeX source");
    for _ in 0..1024 {
        if control.step(&mut stores).expect("canonical e-TeX step") != MainControlStep::Continue {
            return (stores, control.current_list().nodes().to_vec());
        }
    }
    panic!("canonical e-TeX source did not stop consuming input");
}

#[test]
fn canonical_etex_text_directions_follow_texxet_state() {
    // e-TeX 2.6 `etex.ch` [17.3822--3880]: the nonzero `valign`
    // modifiers append direction nodes in hmode exactly when
    // `TeXXeT_state>0`.
    let (enabled, nodes) = run_canonical_etex_current_list(
        r"\TeXXeTstate=1\noindent
          \beginL\endL\beginR\endR",
    );
    assert_eq!(enabled.int_param(IntParam::TEX_XET_STATE), 1);
    assert_eq!(
        nodes,
        [
            Node::Direction(tex_state::node::Direction::BeginL),
            Node::Direction(tex_state::node::Direction::EndL),
            Node::Direction(tex_state::node::Direction::BeginR),
            Node::Direction(tex_state::node::Direction::EndR),
        ]
    );
}

#[test]
fn disabled_text_directions_diagnose_exactly_without_consuming_or_mutating() {
    // e-TeX 2.6 etex.ch [17.3822--3880]: every nonzero `valign` modifier
    // passes through `eTeX_enabled`, whose false branch reports exactly one
    // Improper diagnostic with help1 and appends no direction node.
    const PRIMITIVES: [(&str, usize); 4] = [("beginL", 2), ("endL", 1), ("beginR", 1), ("endR", 1)];
    for profile in [CommandProfile::ETEX26, CommandProfile::PDFTEX14027] {
        let stores = run_canonical_extended_profile(
            r"\nonstopmode
              \TeXXeTstate=0 \count0=0
              \setbox0=\hbox{\beginL \global\advance\count0 by1
                {\TeXXeTstate=-1 \endL \global\advance\count0 by2}
                \beginR \global\advance\count0 by4
                \endR \global\advance\count0 by8}
              \setbox1=\vbox{\beginL \global\count2=1 \par}
              \shipout\copy0
              \global\count1=\TeXXeTstate
              \end",
            profile,
        );

        assert_eq!(stores.count(0), 15, "profile {profile:?}");
        assert_eq!(stores.count(1), 0, "profile {profile:?}");
        assert_eq!(stores.count(2), 1, "profile {profile:?}");
        assert_eq!(
            stores.int_param(IntParam::TEX_XET_STATE),
            0,
            "profile {profile:?}"
        );
        let box0 = stores.box_reg(0).expect("copy shipout must retain box 0");
        assert!(
            stores
                .nodes(box0)
                .testing_decoded()
                .iter()
                .all(|node| !matches!(node, Node::Direction(_))),
            "profile {profile:?} inserted a disabled direction node"
        );

        let log = String::from_utf8_lossy(
            stores
                .world()
                .memory_log_output()
                .expect("completed job must publish its transcript"),
        );
        for (primitive, expected_count) in PRIMITIVES {
            assert_eq!(
                log.lines()
                    .filter(|line| *line == format!("! Improper \\{primitive}."))
                    .count(),
                expected_count,
                "profile {profile:?}, primitive \\{primitive}: {log:?}"
            );
        }
        assert_eq!(
            log.lines()
                .filter(|line| *line == "Sorry, this optional e-TeX feature has been disabled.")
                .count(),
            PRIMITIVES.iter().map(|(_, count)| count).sum(),
            "profile {profile:?}: {log:?}"
        );
    }
}

#[test]
fn canonical_etex_direction_in_vbox_starts_a_paragraph() {
    // TeX82 §1090 backs up a vertical-mode `valign` command before
    // `new_graf`; e-TeX 2.6 [53a.3826--3883] gives `\beginL` that command
    // code, so the direction and following kern belong to a paragraph line.
    let stores = run_canonical_etex(r"\setbox0=\vbox{\TeXXeTstate=1\beginL\kern1pt\par}\end");
    let box0 = stores.box_reg(0).expect("vbox should be assigned");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 should hold a vbox");
    };
    assert!(
        stores
            .nodes(vbox.children)
            .testing_decoded()
            .iter()
            .any(|node| matches!(node, Node::HList(_))),
        "vertical-mode direction must be retried inside a paragraph"
    );
}

#[test]
fn canonical_texxet_hpack_reports_and_recovers_lr_anomalies() {
    // e-TeX [33.649]: hpack converts unmatched closing LR nodes to explicit
    // zero kerns, appends missing closers, and runs the ordinary hbox
    // short-display diagnostic tail for both paragraph and direct boxes.
    let stores = run_canonical_etex(
        r"\nonstopmode\showboxdepth=10\showboxbreadth=20
          \setbox0=\vbox{\hsize=10pt\parindent=0pt\TeXXeTstate=1
            \beginL\beginR\kern1pt\endL\endR\endL\par
            \endL\kern2pt\endR\par}
          \setbox1=\hbox{\TeXXeTstate=1\beginL}\end",
    );
    let log = terminal_effect_text(&stores);
    assert!(
        log.contains("\\endL or \\endR problem (0 missing, 1 extra) in paragraph at lines"),
        "{log}"
    );
    assert!(
        log.contains("\\endL or \\endR problem (0 missing, 2 extra) in paragraph at lines"),
        "{log}"
    );
    assert!(
        log.contains("\\endL or \\endR problem (1 missing, 0 extra) detected at line"),
        "{log}"
    );

    let box0 = stores.box_reg(0).expect("paragraph vbox");
    let [Node::VList(vbox)] = stores.nodes(box0).testing_decoded() else {
        panic!("register 0 must contain the paragraph vbox");
    };
    let lines = stores
        .nodes(vbox.children)
        .testing_decoded()
        .iter()
        .filter_map(|node| match node {
            Node::HList(line) => Some(*line),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    for (line, expected) in lines.iter().zip([1, 2]) {
        let recovered = stores
            .nodes(line.children)
            .testing_decoded()
            .iter()
            .filter(|node| {
                matches!(
                    node,
                    Node::Kern {
                        amount,
                        kind: tex_state::node::KernKind::Explicit,
                    } if amount.raw() == 0
                )
            })
            .count();
        assert_eq!(recovered, expected);
    }

    let box1 = stores.box_reg(1).expect("direct hbox");
    let [Node::HList(hbox)] = stores.nodes(box1).testing_decoded() else {
        panic!("register 1 must contain the direct hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).testing_decoded(),
        [
            Node::Direction(tex_state::node::Direction::BeginL),
            Node::Direction(tex_state::node::Direction::EndL)
        ]
    ));
}

#[test]
fn canonical_showtokens_scans_unexpanded_balanced_general_text() {
    // e-TeX 2.6 etex.ch [17.3623--3671] uses scan_general_text, then the
    // TeX82 §1297 token_show/common-ending diagnostic path.
    let stores = run_canonical_etex(
        r"\nonstopmode
          \def\boom{\global\count7=99}\def~{\global\count8=99}
          \showtokens{A {B} \boom ~ ##  x}
          \global\count0=1\end",
    );
    let terminal = terminal_effect_text(&stores);
    assert!(terminal.contains("> A {B} \\boom ~ #### x."), "{terminal}");
    assert_eq!(
        stores.count(0),
        1,
        "execution continues after the diagnostic"
    );
    assert_eq!(stores.count(7), 0, "control sequences remain unexpanded");
    assert_eq!(stores.count(8), 0, "active characters remain unexpanded");
}

#[test]
fn canonical_showtokens_is_mode_independent_and_rejects_prefixes_without_mutation() {
    let stores = run_canonical_etex(
        r"\nonstopmode
          \showtokens{V}
          \setbox0=\hbox{\showtokens{H}}
          $\showtokens{M}$
          \global\showtokens{P}
          \global\count0=1\end",
    );
    let terminal = terminal_effect_text(&stores);
    for rendered in ["> V.", "> H.", "> M."] {
        assert!(
            terminal.contains(rendered),
            "missing {rendered:?}: {terminal}"
        );
    }
    assert!(
        terminal.contains("You can't use a prefix with `\\showtokens'"),
        "{terminal}"
    );
    assert_eq!(stores.count(0), 1);
}

#[test]
fn canonical_showifs_renders_live_stack_in_etex_order_without_mutation() {
    // e-TeX 2.6 etex.ch [17.3703--3732]: the current frame is printed first,
    // `fi_code` appends `\else`, and saved source lines belong to each frame.
    let stores = run_canonical_etex(
        "\\nonstopmode\n\
         \\iftrue\n\
         \\unless\\iftrue\\else\n\
         \\iffalse\\else\\showifs\\fi\n\
         \\fi\\fi\n\
         \\global\\count0=1\\end",
    );
    let terminal = terminal_effect_text(&stores);
    let inner = terminal
        .find("### level 3: \\iffalse\\else entered on line 4")
        .expect("innermost conditional");
    let middle = terminal
        .find("### level 2: \\unless\\iftrue\\else entered on line 3")
        .unwrap_or_else(|| panic!("middle conditional: {terminal}"));
    let outer = terminal
        .find("### level 1: \\iftrue entered on line 2")
        .expect("outermost conditional");
    assert!(inner < middle && middle < outer, "{terminal}");
    assert_eq!(stores.count(0), 1, "diagnostic does not alter execution");
}

#[test]
fn canonical_showifs_handles_empty_stack_modes_and_prefix_recovery() {
    let stores = run_canonical_etex(
        r"\nonstopmode
          \showifs
          \setbox0=\hbox{\iftrue\showifs\fi}
          $\iftrue\showifs\fi$
          \global\showifs
          \global\count0=1\end",
    );
    let terminal = terminal_effect_text(&stores);
    assert!(
        terminal.contains("### no active conditionals"),
        "{terminal}"
    );
    assert!(
        terminal.matches("### level 1: \\iftrue").count() >= 2,
        "{terminal}"
    );
    assert!(
        terminal.contains("You can't use a prefix with `\\showifs'"),
        "{terminal}"
    );
    assert_eq!(stores.count(0), 1);
}

#[test]
fn canonical_named_register_aliases_match_primitive_assignments() {
    let stores = run_canonical_tex82(
        r"\countdef\countalias=1 \dimendef\dimenalias=2
          \skipdef\skipalias=3 \muskipdef\muskipalias=4 \toksdef\toksalias=5
          \countalias=7 \dimenalias=8pt \skipalias=9pt plus 1fil
          \muskipalias=10mu plus 2fill \toksalias={A{B}} \end",
    );

    assert_eq!(stores.count(1), 7);
    assert_eq!(stores.dimen(2).raw(), 8 * 65_536);
    assert_eq!(stores.glue(stores.skip(3)).width.raw(), 9 * 65_536);
    assert_eq!(stores.glue(stores.muskip(4)).width.raw(), 10 * 65_536);
    assert_eq!(stores.tokens(stores.toks(5)).len(), 4);
}

#[test]
fn canonical_etex_sparse_registers_support_assignment_and_arithmetic() {
    let stores = run_canonical_etex(
        r"\count2000=5 \advance\count2000 by 5 \multiply\count2000 by 10
          \divide\count2000 by 5
          \dimen2001=2pt \advance\dimen2001 by 3pt
          \skip2002=1pt \advance\skip2002 by 2pt
          \muskip2003=1mu \advance\muskip2003 by 3mu \end",
    );

    // e-TeX 2.6 change [49.1237] scans explicit register selectors through
    // `scan_register_num` for assignment and `do_register_command` alike.
    assert_eq!(stores.count(2000), 20);
    assert_eq!(stores.dimen(2001).raw(), 5 * 65_536);
    assert_eq!(stores.glue(stores.skip(2002)).width.raw(), 3 * 65_536);
    assert_eq!(stores.glue(stores.muskip(2003)).width.raw(), 4 * 65_536);
    assert_eq!(stores.count(0), 0);
}

#[test]
fn canonical_register_definitions_honor_nested_scope_and_globaldefs() {
    let stores = run_canonical_tex82(
        r"\countdef\local=1
          {\countdef\local=2 \local=22}
          \local=11
          \globaldefs=1 {\countdef\globalalias=3}
          \globaldefs=0
          {\globaldefs=-1 \global\countdef\suppressed=4}
          \end",
    );

    assert_eq!(
        stores.meaning(stores.symbol("local").expect("local symbol")),
        Meaning::CountRegister(1)
    );
    assert_eq!(stores.count(1), 11);
    assert_eq!(
        stores.meaning(stores.symbol("globalalias").expect("global alias symbol")),
        Meaning::CountRegister(3)
    );
    assert_eq!(
        stores.meaning(stores.symbol("suppressed").expect("suppressed symbol")),
        Meaning::Undefined
    );
}

#[test]
fn canonical_register_definitions_recover_out_of_range_indices_to_zero() {
    let stores = run_canonical_tex82(
        r"\countdef\badcount=-1 \dimendef\baddimen=256
          \skipdef\badskip=-2 \muskipdef\badmuskip=999 \toksdef\badtoks=256
          \badcount=7 \baddimen=8pt \badskip=9pt \badmuskip=10mu \badtoks={Z}\end",
    );

    assert_eq!(
        stores.meaning(stores.symbol("badcount").expect("bad count symbol")),
        Meaning::CountRegister(0)
    );
    assert_eq!(
        stores.meaning(stores.symbol("baddimen").expect("bad dimen symbol")),
        Meaning::DimenRegister(0)
    );
    assert_eq!(
        stores.meaning(stores.symbol("badskip").expect("bad skip symbol")),
        Meaning::SkipRegister(0)
    );
    assert_eq!(
        stores.meaning(stores.symbol("badmuskip").expect("bad muskip symbol")),
        Meaning::MuskipRegister(0)
    );
    assert_eq!(
        stores.meaning(stores.symbol("badtoks").expect("bad toks symbol")),
        Meaning::ToksRegister(0)
    );
    assert_eq!(stores.count(0), 7);
    assert_eq!(stores.dimen(0).raw(), 8 * 65_536);
    assert_eq!(stores.glue(stores.skip(0)).width.raw(), 9 * 65_536);
    assert_eq!(stores.glue(stores.muskip(0)).width.raw(), 10 * 65_536);
    assert_eq!(stores.tokens(stores.toks(0)).len(), 1);
}

#[test]
fn canonical_named_toks_assignment_collects_and_copies_token_lists() {
    let stores = run_canonical_tex82(r"\toksdef\tokens=12 \tokens={A{B}} \toks13=\tokens \end");

    assert_eq!(
        stores.tokens(stores.toks(12)),
        stores.tokens(stores.toks(13))
    );
    assert_eq!(stores.tokens(stores.toks(12)).len(), 4);
}

#[test]
fn end_inside_unterminated_box_reaches_outer_cleanup() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\hbox{A\\end"));

    let stats = Executor::new()
        .run(&mut input, &mut stores)
        .expect("stop command is reconsidered after box recovery");

    assert_eq!(stats.shipped_artifacts.len(), 1);
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn parshape_and_hanging_parameters_reset_after_paragraph() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=1 3pt 40pt\\hangindent=5pt\\hangafter=2\\looseness=2 x\\par",
    ));
    let mut executor = Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph executes");

    assert_eq!(stores.dimen_param(DimenParam::HANG_INDENT).raw(), 0);
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), 0);
    assert!(stores.paragraph_shape().is_empty());
}

#[test]
fn vertical_par_resets_normal_paragraph_parameters_without_material() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=1 3pt 40pt\\hangindent=5pt\\hangafter=2\\looseness=2\\par",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("vertical par executes normal_paragraph");

    assert_eq!(stores.dimen_param(DimenParam::HANG_INDENT).raw(), 0);
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);
    assert_eq!(stores.int_param(IntParam::LOOSENESS), 0);
    assert!(stores.paragraph_shape().is_empty());
    assert!(stores.current_page_nodes().is_empty());
    assert!(stores.page_contributions().is_empty());
}

#[test]
fn parshape_assignment_obeys_local_and_global_grouping() {
    let mut local_stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut local_stores);
    let mut local_input =
        InputStack::new(MemoryInput::new("\\parshape=1 3pt 40pt{\\parshape=0}\\end"));
    Executor::new()
        .run(&mut local_input, &mut local_stores)
        .expect("locally grouped parshape executes");
    assert_eq!(local_stores.paragraph_shape().len(), 1);
    assert_eq!(local_stores.paragraph_shape()[0].indent.raw(), 3 * 65_536);

    let mut global_stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut global_stores);
    let mut global_input =
        InputStack::new(MemoryInput::new("{\\global\\parshape=1 7pt 80pt}\\end"));
    Executor::new()
        .run(&mut global_input, &mut global_stores)
        .expect("globally grouped parshape executes");
    assert_eq!(global_stores.paragraph_shape().len(), 1);
    assert_eq!(global_stores.paragraph_shape()[0].indent.raw(), 7 * 65_536);
}

#[test]
fn etex_parshape_enquiries_return_explicit_and_repeated_components() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\parshape=2 1pt 2pt 3pt 4pt \
         \\edef\\result{\\the\\parshapeindent1/\\the\\parshapelength1/\
         \\the\\parshapedimen3/\\the\\parshapedimen4/\
         \\the\\parshapeindent8/\\the\\parshapelength8/\\the\\parshapeindent0}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("parshape enquiries execute");
    assert_eq!(
        macro_text(&stores, "result"),
        "1.0pt/2.0pt/3.0pt/4.0pt/3.0pt/4.0pt/0.0pt"
    );
}

#[test]
fn etex_penalty_arrays_assign_query_restore_and_reset_interline_at_par() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\clubpenalties=2 200 100 \
         \\widowpenalties=2 300 400 \
         \\displaywidowpenalties=1 500 \
         {\\clubpenalties=1 7} \
         \\interlinepenalties=2 8 7 \
         \\edef\\before{\\number\\clubpenalties0/\\the\\clubpenalties1/\\the\\clubpenalties8/\
         \\the\\widowpenalties1/\\the\\widowpenalties8/\
         \\the\\displaywidowpenalties0/\\the\\displaywidowpenalties8/\\the\\interlinepenalties0} \
         \\noindent\\par \
         \\edef\\after{\\the\\interlinepenalties0}\\end",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("penalty array assignments and enquiries execute");
    assert_eq!(macro_text(&stores, "before"), "2/200/100/300/400/1/500/2");
    assert_eq!(macro_text(&stores, "after"), "0");
}

fn macro_text(stores: &Universe, name: &str) -> String {
    let symbol = stores.symbol(name).expect("macro control sequence");
    let meaning = stores.macro_meaning(symbol).expect("macro meaning");
    stores
        .tokens(meaning.replacement_text())
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => None,
        })
        .collect()
}

#[test]
fn long_prefix_on_let_reports_tex_prefix_error() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\long\\let\\a=b"));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("irrelevant long prefix is reported, discarded, and let continues");
    assert!(support::terminal_effect_text(&stores).contains("You can't use `\\long'"));
    let a = stores.symbol("a").expect("let target exists");
    assert_eq!(
        stores.meaning(a),
        Meaning::CharToken {
            ch: 'b',
            cat: Catcode::Letter
        }
    );
}

#[test]
fn interactionmode_reads_and_assigns_globally() {
    let mut stores = Universe::new_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\edef\\before{\\the\\interactionmode}\
         \\begingroup\\interactionmode=1\\endgroup\
         \\edef\\after{\\the\\interactionmode}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("interaction mode assignment");

    assert_eq!(macro_text(&stores, "before"), "3");
    assert_eq!(macro_text(&stores, "after"), "1");
    assert_eq!(
        stores.interaction_mode(),
        tex_state::InteractionMode::Nonstop
    );
}

#[test]
fn interactionmode_rejects_out_of_range_values_without_changing_mode() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut input = InputStack::new(MemoryInput::new(
        "\\interactionmode=-1\\edef\\result{\\the\\interactionmode}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("bad mode recovers");
    assert_eq!(macro_text(&stores, "result"), "1");
    assert!(terminal_effect_text(&stores).contains("Bad interaction mode (-1)"));
}

#[test]
fn etex_showgroups_and_showifs_render_live_nested_stacks() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        "\\begingroup\\iftrue\\showgroups\\showifs\\fi\\endgroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("stack diagnostics execute");

    let output = support::terminal_effect_text(&stores);
    assert!(output.contains("### semi simple group (level 1) (\\begingroup)"));
    assert!(output.contains("### bottom level"));
    assert!(output.contains("### level 1: \\iftrue"));
}

#[test]
fn protected_prefix_resumes_command_demand_after_unexpanded_tokens() {
    // e-TeX manual section 3.1 / e-TRIP's protected-macro check: tokens
    // returned by `\unexpanded` are suppressed for that expansion step, but
    // protected macros encountered while the prefix scanner continues are
    // expanded before the eventual definition command.
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    crate::install_etex_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\let\bgroup={\protected\def\two{}\let\three=\two\protected\unexpanded\bgroup\two\protected\three\protected\def\one{\two}}",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("protected prefix chain executes");

    let one = stores.intern("one");
    let Meaning::Macro { definition, flags } = stores.meaning(one) else {
        panic!("one is defined")
    };
    assert!(flags.contains(tex_state::meaning::MeaningFlags::PROTECTED));
    let replacement = stores.macro_definition(definition).replacement_text();
    assert_eq!(stores.tokens(replacement).len(), 1);
    assert!(!terminal_effect_text(&stores).contains("You can't use a prefix"));
}

#[test]
fn global_prefix_resumes_command_demand_inside_unexpanded_tokens() {
    let mut stores = crate::test_harness::universe_with_plain_catcodes();
    tex_expand::install_expandable_primitives(&mut stores);
    tex_expand::install_etex_expandable_primitives(&mut stores);
    install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new(
        r"\let\flag\iftrue\def\setfalse{\let\flag\iffalse}\begingroup\global\unexpanded{\setfalse}\endgroup",
    ));

    Executor::new()
        .run(&mut input, &mut stores)
        .expect("unexpanded command demand executes");

    let flag = stores.intern("flag");
    assert_eq!(
        stores.meaning(flag),
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::IfFalse)
    );
    assert!(!terminal_effect_text(&stores).contains("You can't use a prefix"));
}

/// tex.web §1079's `begin_box` ends every immediately resolved case --
/// `box_code`, `copy_code`, `last_box_code`, `vsplit_code` -- with the same
/// `box_end(box_context)` call, so §1077's "Store `cur_box` in a box
/// register" applies to all of them and not only to the `\hbox`/`\vbox`
/// bodies §1083 defers to a group end.
#[test]
fn canonical_setbox_stores_every_immediately_resolved_box_source() {
    let stores = run_canonical_tex82(
        r"\setbox12\hbox to 10pt{}\setbox1\copy12 \setbox2\box12
          \setbox14\vbox to 8pt{\hbox{}}\setbox3\vsplit14 to 1pt \end",
    );

    assert!(stores.box_reg(1).is_some(), "\\setbox from \\copy stores");
    assert!(stores.box_reg(2).is_some(), "\\setbox from \\box stores");
    assert!(stores.box_reg(3).is_some(), "\\setbox from \\vsplit stores");
    assert!(
        stores.box_reg(12).is_none(),
        "\\box still voids its source register at the same level"
    );
}

/// §1080's `\lastbox` removes the tail box from the current list; §1075 then
/// disposes of it by context. Under `\setbox` that means the register, not a
/// re-append: plain.tex's `\t@bb@x` reads `\global\setbox\@ne\lastbox` inside
/// the very `\hbox` it is shortening, and a re-append would both void the
/// destination and leave the box in place (`umber2-johp.263`).
#[test]
fn canonical_setbox_from_lastbox_removes_the_box_instead_of_reappending_it() {
    let stores = run_canonical_tex82(r"\setbox13\hbox{\hbox to 10pt{}\global\setbox1\lastbox}\end");

    assert!(
        stores.box_reg(1).is_some(),
        "\\global\\setbox from \\lastbox stores"
    );
    assert_eq!(
        stores
            .box_dimension(13, tex_state::BoxDimension::Width)
            .map(tex_state::scaled::Scaled::raw),
        Some(0),
        "the enclosing hbox is packaged without the box \\lastbox took"
    );
}

/// §1077 is `eq_define(box_base-box_flag+box_context,box_ref,cur_box)` with
/// no `cur_box<>null` guard -- unlike §1076's append and §1075's `ship_out`.
/// A void source therefore voids the destination rather than leaving its
/// previous value in place.
#[test]
fn canonical_setbox_from_a_void_source_voids_the_destination() {
    let stores = run_canonical_tex82(
        r"\setbox1\hbox to 3pt{}\setbox2\hbox to 3pt{}\setbox3\hbox to 3pt{}
          \setbox1\box12 \setbox2\copy12 \setbox3\vsplit12 to 1pt \end",
    );

    assert!(stores.box_reg(1).is_none(), "void \\box voids the target");
    assert!(stores.box_reg(2).is_none(), "void \\copy voids the target");
    assert!(
        stores.box_reg(3).is_none(),
        "void \\vsplit voids the target"
    );
}

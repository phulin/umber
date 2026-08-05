use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile, InputReason,
    InputTransition, RegisteredSourceKind, SourceRegistration,
};
use tex_exec::{
    EngineBoundary, ExecutionBudgetCounters, MainControl, MainControlStep, ParagraphRegion,
};
use tex_state::Universe;

fn terminal_text(stores: &Universe) -> String {
    let committed = stores
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            tex_state::EffectRecord::StreamWrite {
                sink:
                    tex_state::PrintSink::Terminal
                    | tex_state::PrintSink::TerminalAndLog
                    | tex_state::PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

fn register_source(control: &mut MainControl, bytes: &[u8]) {
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(bytes),
        ))
        .expect("source registers");
    control
        .command_mut()
        .open_registered_source(source)
        .expect("source opens");
}

fn run_to_end(control: &mut MainControl, stores: &mut Universe) {
    loop {
        match control.step(stores).expect("canonical program executes") {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[test]
fn overfull_rule_requires_excess_beyond_hfuzz() {
    // TeX82 §§663/174: `hbadness<100` can request an overfull diagnostic
    // within `\hfuzz`, but §663 appends `\overfullrule` only when the excess
    // is greater than `\hfuzz`. The glue therefore remains §174's space.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingonline=1\hbadness=0\hfuzz=2pt\overfullrule=5pt
           \setbox0=\hbox to0pt{\hskip10pt minus9pt}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains("1.0pt too wide) detected at line 2\n \n"),
        "unexpected short display: {terminal}"
    );
    let root = stores.box_reg(0).expect("box0 exists");
    let Some(tex_state::node_arena::NodeRef::HList(hbox)) = stores.nodes(root).first() else {
        panic!("box0 should contain an hbox");
    };
    assert!(matches!(
        stores.nodes(hbox.children).first(),
        Some(tex_state::node_arena::NodeRef::Glue { .. })
    ));
    assert_eq!(stores.nodes(hbox.children).len(), 1);
}

#[test]
fn explicit_rule_remains_a_short_display_rule_marker() {
    // TeX82 §174 prints `|` for a real rule node independently of §663's
    // overfull-rule insertion condition.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingonline=1\hbadness=0\hfuzz=2pt\overfullrule=0pt
           \setbox0=\hbox to0pt{\vrule width1pt}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(
        terminal.contains("1.0pt too wide) detected at line 2\n|\n"),
        "unexpected short display: {terminal}"
    );
}

#[test]
fn vbox_restores_local_parameters_before_reporting_outer_overfull_box() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingonline=1\tracingrestores=1\vbadness=10000\vfuzz=0pt
           \setbox0=\vbox to0pt{\vfuzz=100pt\hrule height10pt}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    let restore = terminal
        .find("{restoring \\vfuzz=0.0pt}")
        .expect("vbox-local vfuzz restoration is traced");
    let diagnostic = terminal
        .find("Overfull \\vbox (10.0pt too high)")
        .expect("restored enclosing vfuzz requests the diagnostic");
    assert!(restore < diagnostic, "{terminal}");
}

#[test]
fn vbox_diagnostic_uses_restored_enclosing_vfuzz() {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingonline=1\global\vbadness=10000\global\vfuzz=100pt
           \setbox0=\vbox to0pt{\vfuzz=0pt\hrule height10pt}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(!terminal.contains("Overfull \\vbox"), "{terminal}");
    assert_eq!(
        stores
            .box_dimension(0, tex_state::BoxDimension::Height)
            .expect("packed vbox height")
            .raw(),
        0,
        "diagnostic sequencing does not alter the requested box dimension"
    );
}

fn run_to_end_observed(control: &mut MainControl, stores: &mut Universe) {
    struct Observer;
    impl tex_command::CommandObserver for Observer {
        fn committed(&mut self, _observation: tex_command::CommandObservation) {}
    }
    let mut observer = Observer;
    loop {
        match control
            .step_with_observer(stores, &mut observer)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
}

#[derive(Debug, Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn collect_to_end(control: &mut MainControl, stores: &mut Universe) -> ObservationRecorder {
    let mut observations = ObservationRecorder::default();
    loop {
        match control
            .step_with_observer(stores, &mut observations)
            .expect("canonical program executes")
        {
            MainControlStep::End | MainControlStep::EndOfInput => break,
            MainControlStep::Continue => {}
        }
    }
    observations
}

#[test]
fn one_token_everydisplay_traces_its_named_context_before_final_token_execution() {
    // TeX82 §§323 and 1145: begin_token_list(every_display,
    // every_display_text) prints the named list while that input level owns
    // its sole token. The following assignment executes normally, and §357
    // retires the exhausted level on the next input demand.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingmacros=2\tracingcommands=3\tracingonline=1\everydisplay{\global}\noindent$$\count7=19$$\end",
    );

    run_to_end_observed(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert_eq!(
        terminal.matches("\\everydisplay->\\global").count(),
        1,
        "{terminal:?}"
    );
    let hook_trace = terminal
        .find("\\everydisplay->\\global")
        .expect("the named hook trace is present");
    let final_token_trace = terminal
        .find("{display math mode: \\global}")
        .expect("the hook's final token reaches main control");
    assert!(
        hook_trace < final_token_trace,
        "§323 traces begin_token_list before §1030 executes the hook's final token: {terminal:?}"
    );
    assert_eq!(stores.count(7), 19);
}

#[test]
fn exhausted_ordinary_token_replay_does_not_gain_a_named_hook_trace() {
    // Negative control: §§323/307 name the every... token_type family, not
    // every arbitrary stored or transient token list that reaches loc=null.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingmacros=2\tracingonline=1\toks0{\global}\the\toks0\count7=23\end",
    );

    run_to_end_observed(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(!terminal.contains("everydisplay->"), "{terminal}");
    assert_eq!(stores.count(7), 23);
    assert_eq!(control.command_mut().input_level_count(), 0);
}

#[test]
fn deferred_write_trace_precedes_improper_spacefactor_report_with_live_context() {
    // TeX82 §§314, 418, and 1370: write_out begins its write_text input level
    // before expanded scan_toks. The named-list trace therefore precedes the
    // error, whose §82 context still displays that same live write level.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\tracingmacros=2\tracingonline=1 \shipout\vbox{\write16{\the\spacefactor}}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    let trace = terminal
        .find("\\write->\\the \\spacefactor ")
        .unwrap_or_else(|| panic!("write trace is visible: {terminal:?}"));
    let improper = terminal
        .find("Improper \\spacefactor")
        .unwrap_or_else(|| panic!("improper auxiliary report is visible: {terminal:?}"));
    let context = terminal[improper..]
        .find("<write> ")
        .map(|offset| improper + offset)
        .unwrap_or_else(|| panic!("write context is live: {terminal:?}"));
    let recovered_zero = terminal[context..]
        .find("\n0\n")
        .map(|offset| context + offset)
        .unwrap_or_else(|| panic!("recovered write value is published: {terminal:?}"));
    assert!(
        trace < improper && improper < context && context < recovered_zero,
        "{terminal}"
    );
}

#[test]
fn immediate_write_reads_horizontal_spacefactor_without_improper_report() {
    // Negative control: §1370's temporary mode zero, rather than the box's
    // surrounding horizontal mode, makes the deferred read improper.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\setbox0=\hbox{A\spacefactor=2345\immediate\write16{\the\spacefactor}}\end",
    );
    run_to_end(&mut control, &mut stores);

    let terminal = terminal_text(&stores);
    assert!(terminal.contains("2345"), "{terminal}");
    assert!(!terminal.contains("Improper \\spacefactor"), "{terminal}");
}

#[test]
fn output_token_list_push_precedes_its_scanner_owned_opening_brace() {
    // TeX82/pdfTeX §§1025/323: `begin_token_list(output_routine,
    // output_text)` publishes the named input level before `scan_left_brace`
    // consumes the routine's opening brace.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\maxdeadcycles=1\output={\dimen0=1pt}
           \topskip=0pt\setbox0=\vbox to1pt{}\copy0\penalty-10000\end",
    );

    let observations = collect_to_end(&mut control, &mut stores);
    let (push, output_level) = observations
        .0
        .iter()
        .enumerate()
        .find_map(|(index, observation)| match observation {
            CommandObservation::Input(record)
                if record.transition == InputTransition::Push
                    && record.reason == InputReason::OutputRoutine =>
            {
                Some((index, record.level))
            }
            _ => None,
        })
        .expect("the output token list is published");
    let opening_brace = observations
        .0
        .iter()
        .enumerate()
        .find_map(|(index, observation)| match observation {
            CommandObservation::Command(record)
                if record.boundary == CommandDeliveryBoundary::Raw
                    && record.command == "left_brace"
                    && record.provenance.input_level == output_level =>
            {
                Some(index)
            }
            _ => None,
        })
        .expect("the output scanner consumes its opening brace");
    assert!(
        push < opening_brace,
        "the §323 publication precedes the §1025 brace: {:?}",
        observations.0
    );
}

#[test]
fn pending_every_par_push_precedes_later_output_push_and_output_brace() {
    // TeX82 §§1091/1025/323: display resumption starts the following
    // paragraph (and therefore its `every_par` list) before its final
    // `build_page` can enter the output routine. All three publications are
    // one ordered transaction: every_par, output_text, then output_text's
    // scanner-owned opening brace.
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\maxdeadcycles=1\vsize=1pt\everypar{\relax}\output={\dimen0=1pt}
           \noindent X\par $$x$$Y\penalty-10000\end",
    );

    let observations = collect_to_end(&mut control, &mut stores);
    let pushes: Vec<_> = observations
        .0
        .iter()
        .enumerate()
        .filter_map(|(index, observation)| match observation {
            CommandObservation::Input(record)
                if record.transition == InputTransition::Push
                    && matches!(
                        record.reason,
                        InputReason::EveryPar | InputReason::OutputRoutine
                    ) =>
            {
                Some((index, record.reason, record.level))
            }
            _ => None,
        })
        .collect();
    let pair = pushes
        .windows(2)
        .find(|pair| pair[0].1 == InputReason::EveryPar && pair[1].1 == InputReason::OutputRoutine);
    let pair = pair.unwrap_or_else(|| panic!("expected adjacent ordered pushes: {pushes:?}"));
    let opening_brace = observations
        .0
        .iter()
        .enumerate()
        .find_map(|(index, observation)| match observation {
            CommandObservation::Command(record)
                if index > pair[1].0
                    && record.boundary == CommandDeliveryBoundary::Raw
                    && record.command == "left_brace"
                    && record.provenance.input_level == pair[1].2 =>
            {
                Some(index)
            }
            _ => None,
        })
        .expect("the later output list consumes its opening brace");
    assert!(pair[1].0 < opening_brace, "{pushes:?}");
}

#[test]
fn default_page_output_publishes_no_output_token_list_push() {
    // TeX82 §§1023--1025: the default output path ships box 255 without
    // entering `output_text`; only a selected non-null \output routine owns
    // the named token-list publication.
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(
        &mut control,
        br"\topskip=0pt\setbox0=\vbox to1pt{}\copy0\penalty-10000\end",
    );

    let observations = collect_to_end(&mut control, &mut stores);
    assert!(!observations.0.iter().any(|observation| matches!(
        observation,
        CommandObservation::Input(record)
            if record.transition == InputTransition::Push
                && record.reason == InputReason::OutputRoutine
    )));
}

fn editor_layout_for(bytes: &[u8]) -> (tex_state::FragmentStore, tex_state::EditorLayout) {
    let mut fragments = tex_state::FragmentStore::new();
    let (fragment, _) = fragments
        .append(Arc::from(bytes), 2)
        .expect("editor fragment installs");
    let length = u32::try_from(bytes.len()).expect("fixture fits editor layout");
    let layout = tex_state::EditorLayout::new(
        "<editor>",
        tex_state::LayoutGeneration::new(2),
        vec![tex_state::Piece::new(fragment, 0, length)],
        &fragments,
    )
    .expect("editor layout installs");
    (fragments, layout)
}

fn fork_after_first_paragraph(
    old: &[u8],
    revised: Arc<[u8]>,
) -> (MainControl, Universe, ParagraphRegion) {
    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut control = MainControl::tex82_initex(&mut stores);
    register_source(&mut control, old);
    let checkpoint = loop {
        assert!(
            !matches!(
                control.step(&mut stores).expect("cold source executes"),
                MainControlStep::End | MainControlStep::EndOfInput
            ),
            "first paragraph boundary must precede end"
        );
        if control
            .take_completed_boundaries()
            .contains(&EngineBoundary::OuterParagraphEnd)
        {
            break control
                .capture_checkpoint_with_exact_identity(
                    EngineBoundary::OuterParagraphEnd,
                    &mut stores,
                    ExecutionBudgetCounters::default(),
                )
                .expect("paragraph boundary checkpoints");
        }
    };
    let _ = control.take_finished_paragraph_regions();
    run_to_end(&mut control, &mut stores);
    let suffix = control.take_finished_paragraph_regions();
    let edit_start = old
        .iter()
        .zip(revised.iter())
        .position(|(old, new)| old != new)
        .expect("fixture has one edit");
    let region = suffix
        .last()
        .expect("stable suffix paragraph records")
        .rehome_edited_root(old, Arc::clone(&revised), edit_start..edit_start + 4)
        .expect("stable suffix rehomes");
    let memo = control.take_pure_memo_runtime();
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = MainControl::with_profile(CommandProfile::TEX82);
    replay.install_pure_memo_runtime(memo);
    let (forked, _) = checkpoint
        .fork_editor(&mut replay, &substrate, old, revised, &fragments, &layout)
        .expect("canonical editor checkpoint forks");
    (replay, forked, region)
}

#[test]
fn checkpoint_fork_keeps_rehomed_suffix_replay_key() {
    let old = br"first\par
beta\par
stable suffix\par
\end";
    let revised: Arc<[u8]> = Arc::from(
        &br"first\par
delta\par
stable suffix\par
\end"[..],
    );
    let (mut replay, mut stores, region) = fork_after_first_paragraph(old, Arc::clone(&revised));
    replay.install_paragraph_replay_regions([region]);
    run_to_end(&mut replay, &mut stores);
    assert_eq!(replay.pure_memo_stats().paragraph_hits, 1);
    assert!(
        replay
            .take_finished_paragraph_regions()
            .iter()
            .any(|region| region.finished_lines().is_some())
    );
}

#[test]
fn job_start_fork_replays_after_unrelated_prefix_assignment() {
    let old = br"stateful \count5=41 paragraph text\par
stateful \count5=42 paragraph text\par
\end";
    let prefix = br"\count99=3 ";
    let mut revised = prefix.to_vec();
    revised.extend_from_slice(old);
    let revised: Arc<[u8]> = revised.into();

    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut cold = MainControl::tex82_initex(&mut stores);
    register_source(&mut cold, old);
    let checkpoint = cold
        .capture_checkpoint_with_exact_identity(
            EngineBoundary::JobStart,
            &mut stores,
            ExecutionBudgetCounters::default(),
        )
        .expect("job start checkpoints");
    run_to_end(&mut cold, &mut stores);
    let regions = cold
        .take_finished_paragraph_regions()
        .into_iter()
        .map(|region| {
            region
                .rehome_edited_root(old, Arc::clone(&revised), 0..0)
                .expect("unchanged paragraph rehomes after prefix insertion")
        })
        .collect::<Vec<_>>();
    let memo = cold.take_pure_memo_runtime();
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = MainControl::with_profile(CommandProfile::TEX82);
    replay.install_pure_memo_runtime(memo);
    let (mut stores, _) = checkpoint
        .fork_editor(
            &mut replay,
            &substrate,
            old,
            Arc::clone(&revised),
            &fragments,
            &layout,
        )
        .expect("job-start editor checkpoint forks");
    replay.install_paragraph_replay_regions(regions);
    run_to_end(&mut replay, &mut stores);
    assert_eq!(replay.pure_memo_stats().paragraph_hits, 2);
    assert_eq!(stores.count(99), 3);
}

#[test]
fn job_start_fork_rejects_changed_mutation_precondition() {
    let old = br"stateful \count5=41 paragraph text\par
\end";
    let prefix = br"\count5=99 ";
    let mut revised = prefix.to_vec();
    revised.extend_from_slice(old);
    let revised: Arc<[u8]> = revised.into();

    let mut stores = Universe::new_with_plain_catcodes();
    stores.enable_pure_memo(tex_state::PureMemoConfig::default());
    stores.set_root_editor_content_hash(tex_state::ContentHash::from_bytes(old));
    let mut cold = MainControl::tex82_initex(&mut stores);
    register_source(&mut cold, old);
    let checkpoint = cold
        .capture_checkpoint_with_exact_identity(
            EngineBoundary::JobStart,
            &mut stores,
            ExecutionBudgetCounters::default(),
        )
        .expect("job start checkpoints");
    run_to_end(&mut cold, &mut stores);
    let region = cold
        .take_finished_paragraph_regions()
        .pop()
        .expect("stateful paragraph records")
        .rehome_edited_root(old, Arc::clone(&revised), 0..0)
        .expect("unchanged paragraph input rehomes");
    let memo = cold.take_pure_memo_runtime();
    let substrate = stores.freeze_generation();
    let (fragments, layout) = editor_layout_for(&revised);
    let mut replay = MainControl::with_profile(CommandProfile::TEX82);
    replay.install_pure_memo_runtime(memo);
    let (mut stores, _) = checkpoint
        .fork_editor(
            &mut replay,
            &substrate,
            old,
            Arc::clone(&revised),
            &fragments,
            &layout,
        )
        .expect("job-start editor checkpoint forks");
    replay.install_paragraph_replay_regions([region]);
    run_to_end(&mut replay, &mut stores);
    let stats = replay.pure_memo_stats();
    assert_eq!(stats.paragraph_hits, 0);
    assert_eq!(stats.paragraph.key_misses, 1);
    assert_eq!(stores.count(5), 41, "cold execution applies the paragraph");
}

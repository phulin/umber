use std::{collections::BTreeSet, fs};

use test_support::{CompileFailDependency, assert_compile_fail};

fn count_outer_validity_entry_calls(source: &str) -> usize {
    [
        "self.check_outer_validity_entry(&mut rich)",
        "processor.check_outer_validity_entry(&mut rich)",
    ]
    .into_iter()
    .map(|call| source.matches(call).count())
    .sum()
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn crate_production_dependencies_match_the_command_boundary_allowlist() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let command_manifest = fs::read_to_string(manifest_dir.join("Cargo.toml"))
        .unwrap_or_else(|error| panic!("failed to read tex-command manifest: {error}"));
    let state_manifest = fs::read_to_string(manifest_dir.join("../tex-state/Cargo.toml"))
        .unwrap_or_else(|error| panic!("failed to read tex-state manifest: {error}"));

    let command_dependencies = dependency_names(&command_manifest);
    let state_dependencies = dependency_names(&state_manifest);
    assert_eq!(
        command_dependencies,
        BTreeSet::from([
            "md-5.workspace",
            "posix-regex.workspace",
            "smallvec.workspace",
            "tex-fonts",
            "tex-state",
        ]),
        "tex-command's production dependency boundary must remain explicit"
    );
    assert!(
        !state_dependencies.contains("tex-command"),
        "tex-state must remain unaware of command interpretation"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn hot_character_delivery_has_no_host_lookup_surface() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    for relative in [
        "src/state.rs",
        "src/input/source.rs",
        "src/input/lines.rs",
        "src/input/tokenizer.rs",
    ] {
        let source = fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"));
        for forbidden in [
            "std::fs",
            "std::net",
            "File::open",
            "CommandHostContext",
            "CommandHostCapabilities",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} must not acquire host resources through {forbidden}"
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn source_checkpoint_and_probe_paths_cannot_clone_variable_owners() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let source = fs::read_to_string(manifest_dir.join("src/input/source.rs"))
        .expect("read source owner implementation");
    let lines = fs::read_to_string(manifest_dir.join("src/input/lines.rs"))
        .expect("read source line implementation");
    let tokenizer = fs::read_to_string(manifest_dir.join("src/input/tokenizer.rs"))
        .expect("read source tokenizer implementation");
    let levels = fs::read_to_string(manifest_dir.join("src/input/levels.rs"))
        .expect("read input checkpoint implementation");
    let history = fs::read_to_string(manifest_dir.join("src/input/history.rs"))
        .expect("read dedicated input history implementation");
    let owners = format!("{source}\n{lines}\n{levels}");

    for owner in ["SourceCursor", "SourceLineState", "SourceOpenDepths"] {
        let declaration = format!("struct {owner}");
        let prefix = owners.split(&declaration).next().unwrap_or_default();
        let derive = prefix.rsplit("#[derive(").next().unwrap_or_default();
        assert!(
            !derive
                .split(')')
                .next()
                .unwrap_or_default()
                .contains("Clone"),
            "{owner} must remain a move-only variable owner"
        );
    }
    assert!(tokenizer.contains("struct LineProbe"));
    assert!(tokenizer.contains("LineProbe::new(line.cursor)"));
    for forbidden in [
        "line.clone()",
        "self.line.clone()",
        "trial.clone()",
        "Arc::clone",
    ] {
        assert!(
            !tokenizer.contains(forbidden),
            "source probes must not clone owner state through {forbidden}"
        );
    }
    assert!(!levels.contains("source.slot.cursor.clone()"));
    assert!(levels.contains("struct SourceLexExecutionState"));
    assert!(levels.contains("position: u32"));
    assert!(!levels.contains("LogicalStackElement for InputLevel"));
    assert!(history.contains("enum InputUndo"));
    assert!(history.contains("pub(crate) struct InputStack"));
    assert!(history.contains("source_slots: PayloadSlab<SourceSlot<G>>"));
    assert!(levels.contains("struct RowRollbackMarker"));
    assert_eq!(
        levels
            .matches("pub(crate) rollback: RowRollbackMarker")
            .count(),
        2
    );
    assert!(!history.contains("rollback_markers"));
    for retired_lane in [
        "touched: Vec<u64>",
        "partially_captured",
        "cold_state_captured",
    ] {
        assert!(
            !history.contains(retired_lane),
            "input rollback must not restore the parallel {retired_lane} lane"
        );
    }
    assert!(history.contains("mutate_top_source_lex"));
    assert!(!history.contains("fn last_mut"));
    assert!(!levels.contains("slot: Box<SourceSlot"));
    assert!(!history.contains("LogicalStack<InputLevel"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn command_delivery_keeps_one_profile_shared_input_path_and_semantic_free_levels() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read typed delivery driver");
    let input_stack = fs::read_to_string(manifest_dir.join("src/input/stack.rs"))
        .expect("read input-top transition");
    let input_history = fs::read_to_string(manifest_dir.join("src/input/history.rs"))
        .expect("read resident input owner");
    let levels = fs::read_to_string(manifest_dir.join("src/input/levels.rs"))
        .expect("read input-level representation");
    let execution_scratch = fs::read_to_string(manifest_dir.join("src/execution_scratch.rs"))
        .expect("read macro-argument storage");
    let command = fs::read_to_string(manifest_dir.join("src/command.rs"))
        .expect("read current-command typestate");

    assert!(
        !next.contains("fn next_command_into("),
        "raw entry points must not own a second next-command pipeline"
    );
    assert_eq!(expansion.matches("fn raw_delivery_entry(").count(), 0);
    assert!(!expansion.contains("fn raw_destination_loop("));
    assert_eq!(expansion.matches("fn expanded_delivery_entry(").count(), 0);
    assert!(!expansion.contains("fn expanded_destination_loop("));
    assert!(!expansion.contains("fn command_delivery_entry("));
    for entry in [
        "pub(super) fn raw_next(",
        "pub(super) fn expanded_next(",
        "pub(super) fn main_character_run(",
    ] {
        assert!(
            expansion.contains(entry),
            "delivery must expose the concrete {entry} loop"
        );
    }
    assert!(!input_history.contains("fn advance_resident_row_into("));
    assert!(!input_stack.contains("fn deliver_top_into("));
    for retired in [
        "fn take_input_token(",
        "fn deliver_raw_input_into(",
        "fn get_next_canonical(",
        "ActiveInput",
        "DeliveredToken",
    ] {
        assert!(
            !next.contains(retired),
            "raw delivery must not retain the retired {retired} envelope"
        );
    }
    assert!(expansion.contains("(HotCommand::empty(), true, false)"));
    let delivery_entry = expansion
        .split("pub(super) fn expanded_next(")
        .nth(1)
        .and_then(|tail| tail.split("/// Completes a source or synthetic").next())
        .expect("locate expanded delivery loop");
    for forbidden in [
        ".recording",
        ".interval",
        ".rollback",
        "InputLevelInlineState",
        "record_resident_first_touch",
    ] {
        assert!(
            !delivery_entry.contains(forbidden),
            "token delivery must not perform rollback bookkeeping through {forbidden}"
        );
    }
    assert!(!delivery_entry.contains("destination.as_ref()"));
    assert_eq!(delivery_entry.matches("destination.as_mut()").count(), 0);
    assert!(!expansion.contains(".advance_resident_row_into("));
    assert!(!next.contains("fn apply_delivery_rules("));
    assert!(delivery_entry.contains("roots.alignment.account_literal_brace("));
    assert!(delivery_entry.contains("resolution.literal_catcode()"));
    assert_eq!(
        delivery_entry.matches("requires_slow_settlement()").count(),
        1,
        "ordinary delivery has one exceptional-mode branch"
    );
    assert!(input_history.contains("true"));
    assert!(!input_history.contains("fn advance_resident_top_into("));
    assert!(!levels.contains("let frame = self.frame;"));
    assert!(command.contains("struct HotToken"));
    assert!(command.contains("struct CommandWord<G>"));
    assert!(!command.contains("struct EmptyCommand"));
    assert!(!command.contains("ResolvedCommand"));
    assert!(delivery_entry.contains("command.write_resolved_delivery("));
    assert!(!input_stack.contains("enum InputTopTransition {"));
    assert!(!input_history.contains("InputTopTransition"));
    assert!(!input_history.contains("fn select_resident_top("));
    assert!(!input_history.contains("enum ResidentInputTop<'a, G>"));
    assert!(!input_history.contains("match &cursor.span"));
    assert!(!input_history.contains("ResidentStoredTokenTop"));
    assert!(!format!("{input_history}\n{levels}").contains("StoredTokenAdvance"));
    assert!(!input_history.contains("ResidentMacroBodyTop"));
    assert!(!input_history.contains("ResidentMacroArgumentTop"));
    assert!(!input_history.contains("fn settle_resident_delivery("));
    assert_eq!(
        command.matches("fn write_resolved_delivery(").count(),
        1,
        "resident input words must resolve through one final-slot write"
    );
    assert_eq!(
        delivery_entry.matches(".write_resolved_delivery(").count(),
        1,
        "all resident variants must share one final-slot admission tail"
    );
    assert!(!levels.contains("destination.write_resolved_delivery("));
    for retired in [
        "RawDeliverySlot",
        "struct RawCommand<'slot, G>",
        "resolve_in_place(",
    ] {
        assert!(
            !format!("{next}\n{input_history}\n{levels}\n{command}").contains(retired),
            "input delivery must resolve the canonical command directly, without {retired}"
        );
    }
    let resident_front = expansion
        .split("fn next_resident_word(&mut self)")
        .nth(1)
        .and_then(|tail| {
            tail.split("/// Substitution changes the input stack")
                .next()
        })
        .expect("locate resident-word reader");
    let expanded_delivery = delivery_entry;
    assert_eq!(expanded_delivery.matches("'delivery: loop").count(), 1);
    assert_eq!(expanded_delivery.matches("'frame: loop").count(), 0);
    assert!(expanded_delivery.contains("ExpandedCommandAction::Expand(dispatch)"));
    assert_eq!(
        resident_front
            .matches("let InputLevel::Resident(row) =")
            .count(),
        1,
        "the owning input row must dispatch directly without a universal top carrier"
    );
    assert!(expansion.contains("InputLevel::Source(source)"));
    assert!(expansion.contains("let InputLevel::Resident(row) ="));
    assert_eq!(resident_front.matches("match &mut row.storage").count(), 1);
    for storage in ["Replay", "Attempt", "Durable", "MacroBody", "MacroArgument"] {
        assert!(
            resident_front.contains(&format!("ResidentTokenStorage::{storage}")),
            "resident storage choice must include {storage}"
        );
    }
    for retired in [
        "advance_resident_top_into",
        "ResidentInputTop",
        "select_resident_top",
        "let inline_state = match",
        "let transition = match",
        "match transition",
        "InputTopTransition",
        "ResidentDeliveryCarrier",
        "deliver_stored_word",
        "fallback",
        "cache",
        "threshold",
    ] {
        assert!(
            !resident_front.contains(retired),
            "resident front must not retain alternate machinery through {retired}"
        );
    }
    assert!(expanded_delivery.contains("break 'fetch literal_catcode;"));
    assert!(expanded_delivery.contains("resolution.literal_catcode()"));
    assert!(resident_front.contains("argument.advance_delivery("));
    assert!(resident_front.contains("next_word_from_current_frame("));
    let frame_read = expansion
        .split("fn next_word_from_current_frame(")
        .nth(1)
        .and_then(|tail| {
            tail.split("/// Reads one word from an admitted macro")
                .next()
        })
        .expect("locate tiny current-frame read");
    for cold_transition in [
        "advance_character_run",
        "finish_resident_exhaustion",
        "push_resident_parameter_cursor",
        "acquire_source_line",
        "finish_exhausted_source",
        "report_recoverable",
        "retire_input_top",
    ] {
        assert!(
            !frame_read.contains(cold_transition),
            "tiny current-frame read must not contain {cold_transition}"
        );
        assert!(
            !resident_front.contains(cold_transition),
            "generated hot loop must leave {cold_transition} to transition_input_frame"
        );
    }
    assert!(frame_read.contains("frame.position()"));
    assert!(frame_read.contains("frame.limit()"));
    assert!(frame_read.contains("load(position)?"));
    assert!(frame_read.contains("frame.advance_resident()"));
    let macro_argument_cursor = levels
        .split("impl<G> MacroArgumentCursor<G>")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> TokenRowHeader<G>").next())
        .expect("locate macro-argument cursor implementation");
    assert!(!macro_argument_cursor.contains("fn advance_word("));
    assert!(macro_argument_cursor.contains("fn advance_delivery("));
    assert_eq!(
        execution_scratch
            .matches("fn admitted_argument_parts_at_sequential(")
            .count(),
        1
    );
    assert!(!execution_scratch.contains("fn admitted_argument_word_at_sequential("));
    let admitted_argument_read = execution_scratch
        .split("fn admitted_argument_parts_at_sequential(")
        .nth(1)
        .and_then(|tail| tail.split("fn validate_admitted_argument_range(").next())
        .expect("locate admitted sequential argument read");
    assert!(!admitted_argument_read.contains("range:"));
    assert!(!admitted_argument_read.contains("TracedTokenWord"));
    assert!(!input_stack.contains("fn deliver_top_into("));
    assert!(!input_history.contains("let Some(level) = roots.input.levels.last()"));
    let input_top_transition = expanded_delivery;
    for forbidden in [
        "register_source",
        "backing_registered",
        "line_backing_registered",
    ] {
        assert!(
            !input_top_transition.contains(forbidden),
            "warmed input-top transition must not inspect or call source registration through {forbidden}"
        );
    }
    assert!(input_history.contains("CharacterMode::EightBitExact"));
    assert!(input_history.contains("CharacterMode::UnicodeExtended"));
    assert!(
        !expansion.contains("ControlSequenceCreation"),
        "canonical command delivery must not carry source-name creation policy"
    );
    assert!(!expansion.contains("self.next_command_into("));
    assert!(expansion.contains("fn transition_input_frame("));
    assert!(!expansion.contains("self.expanded_destination_loop("));
    assert!(!expansion.contains("fn expanded_delivery_loop("));
    assert!(!expansion.contains("delivery_state_machine::<"));
    assert!(next.contains("create_source_control_sequences"));
    assert!(input_stack.contains("CompactSourceStepQueries for LiveSourceQueries"));
    assert!(
        !next.contains(".trace"),
        "diagnostic replay explanations must not select raw delivery semantics"
    );

    let level_definition = levels
        .split("pub(crate) enum InputLevel")
        .nth(1)
        .and_then(|tail| tail.split("/// One registered-source level").next())
        .expect("locate InputLevel definition");
    for forbidden in ["Condition", "Cache", "Scanner", "Expansion", "Paragraph"] {
        assert!(
            !level_definition.contains(forbidden),
            "input levels must not retain {forbidden} state"
        );
    }
    assert!(levels.contains("This value is diagnostic/provenance state."));
    assert!(levels.contains("cannot select expansion"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn migrated_production_delivery_callers_own_their_command_destinations() {
    let repository = test_support::repository_root();
    let roots = [
        repository.join("crates/tex-command/src"),
        repository.join("crates/tex-exec/src"),
    ];
    let value_returning_calls = [
        ".get_next()",
        ".get_token()",
        ".get_x_token()",
        ".get_x_or_protected()",
        ".get_next_with_replay_completion(",
        ".get_x_token_with_replay_completion(",
        ".get_x_or_protected_with_replay_completion(",
        ".get_x_alignment_delivery(",
        ".next_non_blank_x_token()",
        ".next_non_blank_non_relax_x_token()",
    ];
    let inferred_or_redispatched = [
        "infer_command_destination",
        "search_command_destination",
        "redispatch_command",
    ];

    for root in roots {
        for path in production_rust_sources(&root) {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            let relative = path
                .strip_prefix(&repository)
                .expect("production source is below repository root");
            for forbidden in value_returning_calls {
                assert!(
                    !source.contains(forbidden),
                    "{} must write command delivery directly into its caller-owned destination; found {forbidden}",
                    relative.display()
                );
            }
            for forbidden in inferred_or_redispatched {
                assert!(
                    !source.contains(forbidden),
                    "{} must not infer a destination or redispatch a delivered command through {forbidden}",
                    relative.display()
                );
            }
        }
    }

    // The diagnostic-only undefined-preserving convenience remains a distinct
    // cold host boundary. It is not part of ordinary command delivery and may
    // not spread into another production caller.
    let main_control = fs::read_to_string(repository.join("crates/tex-exec/src/main_control.rs"))
        .expect("read main-control implementation");
    assert_eq!(
        main_control
            .matches(".get_x_token_preserving_undefined()")
            .count(),
        1,
        "only diagnostic_expand_step may retain the undefined-preserving convenience"
    );
    let diagnostic = main_control
        .split("pub fn diagnostic_expand_step(")
        .nth(1)
        .and_then(|tail| tail.split("pub fn ").next())
        .expect("locate diagnostic-only expansion entry point");
    assert!(diagnostic.contains(".get_x_token_preserving_undefined()"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn outer_validity_and_runaway_recovery_have_one_raw_delivery_owner() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read fused delivery implementation");
    let outer = fs::read_to_string(manifest_dir.join("src/processor/outer_recovery.rs"))
        .expect("read outer recovery implementation");

    assert_eq!(
        outer.matches("fn check_outer_validity_entry(").count(),
        1,
        "outer-command detection must have one raw-delivery entry point"
    );
    assert_eq!(
        outer.matches("fn recover_runaway_eof(").count(),
        1,
        "EOF legality must have one raw-delivery entry point"
    );
    assert_eq!(
        outer.matches("fn install_outer_recovery(").count(),
        1,
        "outer commands and runaway EOF must share one recovery table"
    );
    // The boolean argument is §336's `cur_cs<>0` test, which selects the
    // first help line; both entry points still share the one recovery table.
    assert_eq!(
        outer
            .matches("self.install_outer_recovery(recovery, ")
            .count(),
        2,
        "only outer-command and runaway-EOF entry points may install recovery"
    );
    assert!(outer.contains("self.back_input(command.copy_for_backup())?;"));
    assert!(outer.contains("self.command.clear_scanner_for_recovery();"));
    assert_eq!(
        count_outer_validity_entry_calls(&expansion),
        1,
        "the singular cold settlement helper must own recovery"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn raw_delivery_handlers_are_private_direct_call_siblings() {
    let processor = test_support::repository_root().join("crates/tex-command/src/processor");
    let module = fs::read_to_string(processor.join("mod.rs")).expect("read processor module");
    let next = fs::read_to_string(processor.join("next.rs")).expect("read raw delivery");
    let expansion = fs::read_to_string(processor.join("expand.rs")).expect("read fused delivery");
    let input = test_support::repository_root().join("crates/tex-command/src/input");
    let history = fs::read_to_string(input.join("history.rs")).expect("read resident input");
    let stack = fs::read_to_string(input.join("stack.rs")).expect("read input retirement");
    let end_input =
        fs::read_to_string(processor.join("end_input.rs")).expect("read end-input handling");
    let alignment = fs::read_to_string(processor.join("alignment_interception.rs"))
        .expect("read alignment interception");
    let backup = fs::read_to_string(processor.join("backup.rs")).expect("read input backup");
    let recovery =
        fs::read_to_string(processor.join("recovery.rs")).expect("read command recovery");
    let outer =
        fs::read_to_string(processor.join("outer_recovery.rs")).expect("read outer recovery");

    for sibling in [
        "alignment_interception",
        "backup",
        "end_input",
        "outer_recovery",
        "recovery",
    ] {
        assert!(module.contains(&format!("mod {sibling};")));
        assert!(!module.contains(&format!("pub mod {sibling};")));
    }
    for handler in [
        "fn retire_input_top(",
        "fn check_outer_validity_entry(",
        "fn begin_alignment_v_template(",
        "fn back_input_unchecked(",
    ] {
        assert!(
            !next.contains(handler),
            "next.rs must only orchestrate {handler}"
        );
    }
    assert!(end_input.contains("fn retire_input_top("));
    assert!(alignment.contains("fn begin_alignment_v_template("));
    assert!(backup.contains("fn back_input_unchecked("));
    assert!(outer.contains("fn check_outer_validity_entry("));
    assert!(recovery.contains("fn recover_off_save("));
    assert!(expansion.contains("self.retire_input_top(identity)"));
    assert!(stack.contains("fn retire_resident_ordinary_input("));
    assert!(history.contains("fn finish_resident_exhaustion("));
    assert!(history.contains("fn settle_resident_retirement("));
    assert!(history.contains("fn pop_resident("));
    assert!(!stack.contains("RetiredInputLevel"));
    assert!(!history.contains("pop_resident_project"));
    assert!(expansion.contains("command.write_resolved_delivery("));
    assert!(!history.contains("self.retire_input_top("));
    assert_eq!(
        count_outer_validity_entry_calls(&expansion),
        1,
        "outer validity must remain one direct call from the destination loop"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn macro_stack_conservation_reads_only_admitted_cursor_bounds() {
    let end_input = fs::read_to_string(
        test_support::repository_root().join("crates/tex-command/src/processor/end_input.rs"),
    )
    .expect("read stack-conservation owner");
    let conservation = end_input
        .split("fn conserve_input_stack_with_owner(")
        .nth(1)
        .and_then(|tail| tail.split("/// Names the tex.web").next())
        .expect("locate stack-conservation transition");

    assert!(conservation.contains("level.stored_is_exhausted()"));
    assert_eq!(conservation.matches("cursor.is_exhausted()").count(), 0);
    assert!(!conservation.contains("stored_indexed_token_at_cold"));
    assert!(!conservation.contains("cursor.token_at("));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn scalar_macro_call_keeps_one_raw_fallback_matcher() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let matcher = fs::read_to_string(manifest_dir.join("src/macro_call.rs"))
        .expect("read scalar macro matcher implementation");

    assert_eq!(
        matcher.matches("fn macro_call_scalar(").count(),
        1,
        "macro calls must retain one scalar semantic fallback"
    );
    assert_eq!(
        matcher.matches("fn scan_undelimited_argument(").count(),
        1,
        "undelimited matching must have one canonical scanner"
    );
    assert_eq!(
        matcher.matches("fn scan_delimited_argument(").count(),
        1,
        "delimited matching must have one canonical scanner"
    );
    assert!(
        matcher.contains("self.get_token_into(&mut delivered)?"),
        "the scalar matcher must consume raw tokens into its request-local destination"
    );
    for forbidden in ["CompiledMacroMatcher", "MacroBytecode", "FastMacroMatcher"] {
        assert!(
            !matcher.contains(forbidden),
            "alternate macro matcher {forbidden} would bypass the scalar fallback"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn command_delivery_has_separate_concrete_loops_and_direct_input_mutation() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read ordinary expansion implementation");
    let policies = fs::read_to_string(manifest_dir.join("src/processor/mod.rs"))
        .expect("read processor module definitions");
    let raw = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery entry points");
    let structural = fs::read_to_string(manifest_dir.join("src/processor/expand_structural.rs"))
        .expect("read structural expansion primitives");
    let pdf_string = fs::read_to_string(manifest_dir.join("src/processor/expand_pdf_string.rs"))
        .expect("read pdfTeX string expansion primitives");
    let input_history = fs::read_to_string(manifest_dir.join("src/input/history.rs"))
        .expect("read resident input transition");
    let command_state = fs::read_to_string(manifest_dir.join("src/state.rs"))
        .expect("read command-state ownership");

    assert_eq!(expansion.matches("fn raw_delivery_entry(").count(), 0);
    assert!(!expansion.contains("fn raw_destination_loop("));
    assert_eq!(expansion.matches("fn expanded_delivery_entry(").count(), 0);
    assert!(!expansion.contains("fn expanded_destination_loop("));
    assert!(!expansion.contains("fn command_delivery_entry("));
    assert!(policies.contains("mod delivery_mode;"));
    assert!(expansion.contains("delivery_mode.requires_slow_settlement()"));
    for deleted in ["DeliveryPolicy", "ExpandedDeliveryPolicy"] {
        assert!(
            !format!("{expansion}\n{policies}").contains(deleted),
            "generic delivery shell {deleted} must stay deleted"
        );
    }
    assert!(!expansion.contains("fn raw_delivery_driver("));
    assert!(!expansion.contains("fn expanded_delivery_driver("));
    assert_eq!(expansion.matches("macro_rules!").count(), 0);
    for loop_name in ["raw_next", "expanded_next", "main_character_run"] {
        assert_eq!(
            expansion
                .matches(&format!("pub(super) fn {loop_name}("))
                .count(),
            1,
            "delivery must expose one concrete {loop_name} loop"
        );
    }
    let raw_loop = expansion
        .split("pub(super) fn raw_next(")
        .nth(1)
        .and_then(|tail| tail.split("/// The concrete TeX82 §380").next())
        .expect("locate raw command-delivery loop");
    let expanded_loop = expansion
        .split("pub(super) fn expanded_next(")
        .nth(1)
        .and_then(|tail| tail.split("/// Completes a source or synthetic").next())
        .expect("locate expanded command-delivery loop");
    let character_loop = expansion
        .split("pub(super) fn main_character_run(")
        .nth(1)
        .and_then(|tail| tail.split("/// Replay-aware raw delivery").next())
        .expect("locate character-run delivery loop");
    for delivery_loop in [raw_loop, expanded_loop, character_loop] {
        assert!(!delivery_loop.contains("DeliveryErrorSlot"));
        assert!(!delivery_loop.contains("DeliveryFailed"));
        assert!(!delivery_loop.contains("Result<DeliveryStatus, DeliveryFailed>"));
        assert!(!delivery_loop.contains("destination.as_ref()"));
        assert_eq!(delivery_loop.matches("destination.as_mut()").count(), 0);
        assert!(delivery_loop.contains("roots.alignment.account_literal_brace("));
        assert!(delivery_loop.contains("resolution.literal_catcode()"));
        assert_eq!(
            delivery_loop.matches("requires_slow_settlement()").count(),
            1,
            "each concrete loop has one exceptional-mode branch"
        );
        assert_eq!(
            delivery_loop.matches("write_resolved_delivery(").count(),
            1,
            "each concrete loop has one final-slot resolution"
        );
    }
    assert!(!format!("{expansion}\n{policies}").contains("DeliveryErrorSlot"));
    assert!(!format!("{expansion}\n{policies}").contains("DeliveryFailed"));
    assert!(!input_history.contains("take_ready_replay_completion"));
    assert!(!command_state.contains("pending_replay_completions"));
    assert!(!command_state.contains("replay_completions.iter()"));
    for slow_entry in [
        "pub(super) fn raw_next_with_replay_completion(",
        "pub(super) fn protected_expanded_next_with_replay_completion(",
        "pub(super) fn tex_alignment_lookahead_next(",
        "pub(super) fn resumed_expanded_next(",
        "pub(super) fn alignment_expanded_next(",
    ] {
        assert!(
            expansion.contains(slow_entry),
            "rare delivery semantics require the dedicated {slow_entry} entry"
        );
    }
    assert!(
        expansion.matches("#[cold]").count() >= 5,
        "rare delivery entries must stay out of line"
    );
    for loop_name in ["raw_next", "expanded_next", "main_character_run"] {
        let signature = expansion
            .split(&format!("pub(super) fn {loop_name}("))
            .nth(1)
            .and_then(|tail| {
                tail.split(") -> Result<DeliveryStatus, CommandError>")
                    .next()
            })
            .expect("locate concrete delivery signature");
        for retired_policy_parameter in [
            "expanded_fetch:",
            "protected_macros:",
            "undefined:",
            "observation:",
            "first_command:",
            "replay_completion:",
            "alignment_interception:",
            "character_run: Option",
        ] {
            assert!(
                !signature.contains(retired_policy_parameter),
                "hot entry must not receive runtime policy {retired_policy_parameter}"
            );
        }
    }
    assert!(
        !policies.contains("ControlSequenceCreation"),
        "name creation belongs to source tokenization, not delivery policy"
    );
    assert!(!expansion.contains("ProtectedMacroHandling"));
    assert!(!expansion.contains("UndefinedHandling"));
    assert!(
        !format!("{expansion}\n{raw}").contains("pending_expanded_delivery"),
        "pending observation ownership must be typed, never a boolean"
    );
    assert!(expansion.contains("pub(crate) fn expand_into("));
    assert!(expansion.contains("destination: &mut Option<CurrentCommand<G>>"));
    let classification = expansion
        .split("fn classify_expanded_command<G>(")
        .nth(1)
        .and_then(|tail| tail.split("/// The finite expansion set").next())
        .expect("locate expanded-command classification");
    assert_eq!(
        classification
            .matches("match command.meaning_ref()")
            .count(),
        1
    );
    for exact_action in [
        "ExpansionDispatch::Macro",
        "ExpansionDispatch::Undefined",
        "ExpansionDispatch::Primitive(*primitive)",
    ] {
        assert!(
            classification.contains(exact_action),
            "classification must select {exact_action} directly"
        );
    }
    let expansion_dispatch = expansion
        .split("fn expand_classified_occupied(")
        .nth(1)
        .and_then(|tail| tail.split("pub(super) fn retain_expansion_scalar").next())
        .expect("locate exact expansion dispatch");
    assert!(
        expansion
            .contains("self.expand_classified_into(destination, dispatch, report_trace, false)")
    );
    assert!(expansion.contains("match self.expand_classified_occupied("));
    assert!(
        !expansion_dispatch.contains("Option<ExpansionDispatch>"),
        "an already-classified delivery must not wrap its dispatch for handoff"
    );
    assert!(
        !expansion_dispatch.contains("command.meaning_ref()"),
        "the selected macro/undefined/primitive dispatch must not reread the resident meaning"
    );
    assert!(
        expansion_dispatch.contains("match dispatch"),
        "the borrowed classification must drive exact expansion dispatch"
    );
    assert!(expansion.contains("let _activated = self.macro_call_hot(command)?;"));
    assert!(!expansion.contains("MacroCallOutcome"));
    assert!(expansion.contains("suppress_first_expansion_trace"));
    assert!(expansion.contains(".store_expansion_frame(pending)"));
    assert!(expansion.contains("(HotCommand::empty(), true, false)"));
    assert!(expansion.contains("std::mem::replace(command, CurrentCommand::empty())"));
    assert!(!expansion.contains("fn expand_with_trace("));
    assert!(!expansion.contains("expand_owned_with_trace("));
    assert!(!expansion.contains("delivery_driver_inner("));
    assert!(policies.contains("take_pending_expansion_work"));
    assert!(expansion.contains("ChildContinuation::capture("));
    assert!(expansion.contains("PendingExpansionChildDestination::Dispatch"));
    assert!(structural.contains(".store_expandafter_frame(PendingExpandAfter"));
    assert!(pdf_string.contains(".store_pdf_string_compare_frame(PendingPdfStringCompare"));
    assert!(pdf_string.contains("PdfStringComparePhase::Right { left }"));
    assert!(structural.contains("PendingExpansionResume::CsName { name }"));
    let conditionals = fs::read_to_string(manifest_dir.join("src/conditionals.rs"))
        .expect("read conditional continuation ownership");
    assert!(conditionals.contains("PendingExpansionResume::IfCsName"));
    let state = fs::read_to_string(manifest_dir.join("src/state.rs"))
        .expect("read command-state ownership");
    for forbidden in [
        "pending_csnames",
        "pending_integer_scans",
        "pending_file_enquiry:",
        "Vec<crate::scanners::Pending",
        "Vec<PendingExpansion",
    ] {
        assert!(
            !state.contains(forbidden),
            "caller-order continuation mailbox {forbidden} must not return to command state"
        );
    }
    assert!(
        !format!("{expansion}\n{structural}\n{pdf_string}").contains("retain_pending_expansion")
            && !structural.contains("retain_pending_expandafter"),
        "resource retry ownership must stay in the typed scratch continuation chain"
    );
    assert!(
        !expansion.contains("let retry = command.clone();"),
        "ordinary expansion must move the live command only at a typed retry barrier"
    );
    assert!(!expansion.contains("fn expand_noexpand("));
    assert!(!expansion.contains("fn expand_expandafter("));
    assert!(structural.contains("fn expand_noexpand("));
    assert!(structural.contains("fn expand_expandafter("));
    for forbidden in ["Dispatch::Push", "Dispatch::PushTransient", "ExpansionMode"] {
        assert!(
            !format!("{expansion}\n{structural}\n{pdf_string}").contains(forbidden),
            "ordinary expansion must not introduce {forbidden}"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn scan_toks_keeps_its_one_step_collector_and_direct_splice_boundary() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let scanner = fs::read_to_string(manifest_dir.join("src/scan_toks.rs"))
        .expect("read token-list scanner implementation");
    let token_collector = fs::read_to_string(manifest_dir.join("src/token_collector.rs"))
        .expect("read shared token collector implementation");
    let collector = scanner
        .split("fn collect_replacement(")
        .nth(1)
        .and_then(|tail| tail.split("/// Splices a token-list result").next())
        .expect("locate expanded token-list collector");
    let expansion = scanner
        .split("fn drive_collector_expansion(")
        .nth(1)
        .and_then(|tail| tail.split("/// TeX82 §477").next())
        .expect("locate collector expansion step");
    let splice = scanner
        .split("fn append_direct_the_toks(")
        .nth(1)
        .and_then(|tail| tail.split("/// e-TeX").next())
        .expect("locate direct token-list splice");
    let unexpanded = scanner
        .split("fn append_unexpanded(")
        .nth(1)
        .and_then(|tail| tail.split("/// e-TeX 2.6").next())
        .expect("locate unexpanded child splice");
    let standalone_unexpanded = scanner
        .split("fn expand_unexpanded(")
        .nth(1)
        .expect("locate standalone unexpanded replay");

    assert_eq!(scanner.matches("fn scan_toks_inner(").count(), 1);
    assert!(scanner.contains("let mut pending = match resumed"));
    assert!(scanner.contains("phase: &mut PendingScanToksPhase<G>"));
    assert!(collector.contains("progress: &mut ReplacementProgress<G>"));
    for retired_carrier in [
        "struct ScanToksFailure",
        "struct ReplacementFailure",
        "fn replacement_failure",
    ] {
        assert!(
            !scanner.contains(retired_carrier),
            "scan_toks must not rebuild the stationary phase through {retired_carrier}"
        );
    }
    assert_eq!(
        scanner.matches("progress: ReplacementProgress<G>").count(),
        1,
        "only the stationary phase row may own replacement progress"
    );
    assert!(collector.contains("self.get_next_into(&mut destination)"));
    assert!(expansion.contains(".as_mut()"));
    assert!(collector.contains("clear_command_destination(&mut destination)"));
    assert!(collector.contains("pending_expansion.take()"));
    let restore = collector
        .find("pending_expansion.take()")
        .expect("collector restores its parked expansion");
    let steady_loop = collector.find("loop {").expect("steady collection loop");
    assert!(restore < steady_loop);
    assert!(
        !collector[steady_loop..].contains("pending_expansion.take()"),
        "steady replacement collection must not probe parked suspension state"
    );
    assert_eq!(scanner.matches("fn drive_collector_expansion(").count(), 1);
    assert!(expansion.contains("PendingCollectorExpansion"));
    assert!(expansion.contains("error.is_resource_suspension()"));
    assert!(expansion.contains("self.expand_into(destination, true)"));
    assert!(expansion.contains("command: destination.take()"));
    assert!(expansion.contains("self.append_direct_the_toks(collector, expansion_operand)"));
    assert!(
        !collector.contains("self.get_x_token()?"),
        "the replacement collector must not enter a second ordinary expansion loop"
    );
    assert!(
        splice.contains("self.get_x_token_into(target)?"),
        "\\the must retain its expanded internal-value target before selecting a token list"
    );
    assert!(splice.contains("self.push_scan_toks_word(collector, word)?"));
    assert!(scanner.contains(".push_definition_replacement(*definition, word.token_word())"));
    for retired_route in ["ScanToksSinks", "ScanToksSink", "ScannedToksPart"] {
        assert!(
            !scanner.contains(retired_route),
            "resident collector must not retain retired route {retired_route}"
        );
    }
    assert!(scanner.contains("collector: TokenCollector<G>"));
    assert!(scanner.contains("collector: &mut TokenCollector<G>"));
    assert!(!scanner.contains("struct ScanToksCollector"));
    assert!(token_collector.contains("pub(crate) struct TokenCollector<G>"));
    assert!(!token_collector.contains("TokenCollectorDestination::MacroArgument"));
    assert!(!token_collector.contains("MacroArgumentFacts"));
    assert!(token_collector.contains("TokenCollectorDestination::TokenBuffers"));
    assert!(token_collector.contains("TokenCollectorDestination::Definition"));
    assert!(token_collector.contains("TokenCollectorDestination::ReplayInput"));
    assert!(unexpanded.contains("consume_token_list_into_buffer"));
    assert!(standalone_unexpanded.contains("ScanToksMode::EscapingGeneralText"));
    assert!(standalone_unexpanded.contains("ScannedToksStorage::ReplayInput"));
    assert!(standalone_unexpanded.contains("PackedTokenSpanHandle::Replay"));
    assert!(!standalone_unexpanded.contains(".to_vec()"));
    assert!(!standalone_unexpanded.contains("PackedTokenSpanHandle::transient"));
    assert!(!standalone_unexpanded.contains("PackedTokenSpanHandle::AttemptList"));
    assert!(
        !splice.contains("self.expand("),
        "direct token-list splicing must not recursively expand its contents"
    );
    assert!(
        !splice.contains("self.get_next()?"),
        "direct token-list splicing must not redeliver its contents through the collector"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn attempt_promotion_preflights_then_writes_the_resident_destination_once() {
    let repository = test_support::repository_root();
    let attempt = fs::read_to_string(repository.join("crates/tex-command/src/attempt.rs"))
        .expect("read attempt promotion implementation");
    let stores = fs::read_to_string(repository.join("crates/tex-state/src/stores.rs"))
        .expect("read destination promotion implementation");
    let operation =
        fs::read_to_string(repository.join("crates/tex-exec/src/main_control/cold/operation.rs"))
            .expect("read resident cold-operation promotion implementation");
    let promotion = attempt
        .split("pub(crate) fn promote_into<D>(")
        .nth(1)
        .and_then(|tail| tail.split("pub(crate) fn promote_definition(").next())
        .expect("locate generic attempt promotion");
    let destination = stores
        .split("pub(crate) fn promote_resident_batch<B>(")
        .nth(1)
        .and_then(|tail| tail.split("fn promote_value_streams_from<").next())
        .expect("locate resident destination promotion");
    let resident_writer = operation
        .split("struct ColdOperationPromotion<'a, G>")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> ColdOperation<G>").next())
        .expect("locate cold-operation resident writer");
    let preparation = operation
        .split("pub(in crate::main_control) fn prepare_cold_operation<G>(")
        .nth(1)
        .and_then(|tail| tail.split("struct ColdOperationPromotion<'a, G>").next())
        .expect("locate resident cold-operation preparation");

    assert!(promotion.contains("universe.promote_resident_batch(&mut batch)?"));
    assert!(promotion.contains("AttemptResidentPromotion"));
    assert!(!promotion.contains("SmallVec"));
    assert!(!promotion.contains("AttemptPromotionReceipt"));
    assert!(!promotion.contains("AttemptPromotionRoots"));
    assert!(!promotion.contains("promote_value_streams"));
    assert!(!promotion.contains("definitions.into_iter()"));
    assert!(!promotion.contains("DefinitionPromotion::new("));
    assert!(!promotion.contains("Vec<DefinitionBuilder>"));
    assert!(!promotion.contains("Vec<TokenWord>"));
    assert!(!promotion.contains("parameter_text().to_vec()"));
    assert!(!promotion.contains("replacement_text().to_vec()"));
    assert!(!destination.contains("DefinitionBuilder::from_slices"));
    let validation = destination
        .find("definitions_arena.validate_builder(batch.definition(index))")
        .expect("destination-policy preflight");
    let publication = destination
        .find(".publish_prevalidated(batch.next_definition_mut())")
        .expect("infallible checked builder transfer");
    assert!(validation < publication);
    assert!(destination.contains("reserve_batch(definition_count, definition_words)?"));
    assert!(destination.contains(".allocate_from_iter(words)"));
    assert!(destination.contains("batch.settle_next_definition(definition)"));
    assert!(destination.contains("batch.settle_next_token_list(tokens)"));
    assert!(!destination.contains("PromotionReceipt"));
    assert!(!resident_writer.contains("AttemptPromotionReceipt"));
    assert!(!resident_writer.contains("AttemptPromotionRoots"));
    assert!(!resident_writer.contains("receipt.token_lists"));
    assert!(!resident_writer.contains("receipt.definitions"));
    assert!(preparation.contains(") -> Result<(), ColdPreparationError>"));
    assert!(preparation.contains("command.promote_attempt_roots_into(stores, &mut destination)?"));
    assert!(!preparation.contains("Result<Vec<"));
    assert!(!preparation.contains("let mut roots"));
    assert!(!preparation.contains("let mut definitions"));
    assert!(!preparation.contains("collect::<Vec"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn resource_capable_scalar_scans_have_one_inline_owned_continuation_surface() {
    let repository = test_support::repository_root();
    let manifest_dir = repository.join("crates/tex-command");
    let scalar = fs::read_to_string(manifest_dir.join("src/scanners/scalar.rs"))
        .expect("read scalar scanner implementation");
    let font = fs::read_to_string(manifest_dir.join("src/scanners/font.rs"))
        .expect("read font scanner implementation");
    let structured = fs::read_to_string(manifest_dir.join("src/scanners/structured.rs"))
        .expect("read structured scanner implementation");

    for forbidden in [
        "pub fn scan_optional_equals(",
        "pub fn scan_keyword(",
        "pub fn scan_integer(",
        "pub fn scan_dimension(",
        "pub fn scan_mu_dimension(",
        "pub fn scan_glue(",
        "pub fn scan_internal_value_or_zero(",
        "pub fn scan_the_internal_value(",
        "pub fn scan_character_number(",
        "pub fn scan_eight_bit_register_index(",
        "pub fn scan_profile_register_index(",
        "pub fn scan_extended_register_index(",
    ] {
        assert!(
            !scalar.contains(forbidden),
            "resource-capable scalar entry must stay private: {forbidden}"
        );
    }
    assert!(!font.contains("pub fn scan_font_selector("));
    assert!(!structured.contains("pub fn scan_file_name("));

    let scalar_frame = scalar
        .split("pub(crate) enum PendingScalarFrame")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> PendingScalarFrame").next())
        .expect("locate scalar continuation variants");
    for forbidden in ["Box<", "Vec<", "Arc<", "VecDeque", "HashMap"] {
        assert!(
            !scalar_frame.contains(forbidden),
            "scalar continuation must remain inline and allocation-free: {forbidden}"
        );
    }

    let raw_callers = [
        "src/conditionals.rs",
        "src/processor/expand.rs",
        "src/scan_toks.rs",
        "src/scanners/expression.rs",
        "src/scanners/hyphenation.rs",
        "src/scanners/restricted.rs",
        "src/scanners/structured.rs",
        "src/scanners/token_list.rs",
    ];
    let forbidden_calls = [
        ".scan_optional_equals()",
        ".scan_keyword(",
        ".scan_integer()",
        ".scan_dimension()",
        ".scan_mu_dimension()",
        ".scan_glue(",
        ".scan_internal_value_or_zero()",
        ".scan_the_internal_value(",
        ".scan_character_number()",
        ".scan_eight_bit_register_index()",
        ".scan_profile_register_index()",
        ".scan_extended_register_index()",
        ".scan_font_selector()",
    ];
    for relative in raw_callers {
        let source = fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"));
        for forbidden in forbidden_calls {
            assert!(
                !source.contains(forbidden),
                "{relative} bypasses an owned scalar parent through {forbidden}"
            );
        }
    }

    let main_control =
        fs::read_to_string(repository.join("crates/tex-exec/src/main_control/command_episode.rs"))
            .expect("read main-control continuation architecture");
    let command_episode = main_control
        .split("struct CommandEpisode<G>")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> Default for CommandEpisode<G>").next())
        .expect("locate singular resident command episode");
    assert!(command_episode.contains("command: Option<tex_command::CurrentCommand<G>>"));
    assert!(command_episode.contains("phase: Option<PreflightCommandPhase>"));
    assert!(command_episode.contains("scanner: Option<tex_command::ScannerFrameKey<G>>"));
    assert!(command_episode.contains("operation_scan: Option<PendingOperationScanPhase>"));
    assert!(!command_episode.contains("OperationPayload"));
    for forbidden in ["Box<", "Vec<", "Arc<", "VecDeque", "HashMap"] {
        assert!(!command_episode.contains(forbidden));
    }
    let operation_frame = main_control
        .split("struct OperationFrame<G>")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> OperationFrame<G>").next())
        .expect("locate suspension-only operation frame");
    assert!(operation_frame.contains("episode: Option<CommandEpisode<G>>"));
    assert!(operation_frame.contains("cold: Option<ColdOperationSlot<G>>"));
    assert!(!operation_frame.contains("CurrentCommand"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn condition_delivery_and_alignment_lifecycle_remain_on_the_canonical_seams() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let conditionals = fs::read_to_string(manifest_dir.join("src/conditionals.rs"))
        .expect("read conditional implementation");
    let input =
        fs::read_to_string(manifest_dir.join("src/input/mod.rs")).expect("read input facade");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read frame-owned delivery loop");
    let state = fs::read_to_string(manifest_dir.join("src/state.rs")).expect("read command state");
    let alignment = fs::read_to_string(manifest_dir.join("src/processor/alignment.rs"))
        .expect("read alignment delivery implementation");
    let interception =
        fs::read_to_string(manifest_dir.join("src/processor/alignment_interception.rs"))
            .expect("read alignment interception implementation");
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");

    let pass_text = conditionals
        .split("fn pass_text_scalar(")
        .nth(1)
        .and_then(|tail| tail.split("fn classify_pass_text_command(").next())
        .expect("locate canonical skipped-text loop");
    assert!(pass_text.contains("self.get_next_into(&mut destination)?"));
    assert!(pass_text.contains("nested_conditions"));
    assert!(conditionals.contains("fn evaluate_ifx("));
    let ifx = conditionals
        .split("fn evaluate_ifx(")
        .nth(1)
        .and_then(|tail| tail.split("fn ifx_meaning_eq(").next())
        .expect("locate raw ifx operand comparison");
    // TeX82 §507 reads both operands with `get_next`. The distinction is not
    // cosmetic: §365 clears `no_new_control_sequence` only inside
    // `get_token`, so reading an `\ifx` operand must not enter a new name in
    // the hash table.
    assert_eq!(ifx.matches("self.get_next_into(").count(), 2);
    assert!(!ifx.contains("self.get_token()?"));
    assert!(!ifx.contains("self.get_x_token()?"));
    assert!(ifx.contains("begin_scanner_episode(ScannerStatus::Normal"));
    assert!(ifx.contains("finish_scanner_episode(episode)"));
    assert_eq!(ifx.matches(".meaning_ref()").count(), 2);
    assert!(!ifx.contains(".meaning()"));
    assert!(ifx.contains("Ok::<_, CommandError>(self.ifx_meaning_eq("));
    assert!(!ifx.contains("(first, second)"));
    let ifx_comparison = conditionals
        .split("fn ifx_meaning_eq(")
        .nth(1)
        .and_then(|tail| tail.split("fn scan_if_relation(").next())
        .expect("locate borrowed ifx meaning comparison");
    assert!(ifx_comparison.contains(".definition_contents_equal("));
    assert!(!ifx_comparison.contains(".clone()"));
    assert!(!ifx_comparison.contains("self.state.definition("));
    assert!(conditionals.contains("fn expand_unless("));
    assert!(conditionals.contains("inverted"));
    assert!(!input.contains("ConditionStack"));

    // TeX82 §331 gives the run exactly one `align_state`; §772's
    // `push_alignment`/`pop_alignment` save and restore copies of it on the
    // alignment stack rather than giving any other record its own field.
    assert_eq!(alignment.matches("align_state: i32").count(), 1);
    assert_eq!(
        alignment
            .matches("align_stack: crate::timeline::LogicalStack<i32>")
            .count(),
        1
    );
    assert_eq!(alignment.matches("fn account_literal_brace(").count(), 1);
    assert_eq!(alignment.matches("fn classify_delimiter(").count(), 1);
    assert_eq!(next.matches(".classify_alignment_delivery(").count(), 0);
    assert_eq!(
        expansion
            .matches("roots.alignment.account_literal_brace(")
            .count(),
        4,
        "three hot loops and the cold synthetic tail share the one alignment authority"
    );
    assert!(!next.contains("record_alignment_phase"));
    let classifier = alignment
        .split("fn account_literal_brace(")
        .nth(1)
        .and_then(|tail| tail.split("/// Applies TeX82 §1127").next())
        .expect("locate the singular alignment delivery classifier");
    assert_eq!(classifier.matches("semantic_token()").count(), 0);
    assert!(classifier.contains("match literal_catcode"));
    assert_eq!(
        classifier.matches("record_delivery_align_state(").count(),
        2
    );
    assert!(!classifier.contains(".clone()"));
    assert!(!classifier.contains("Vec<"));
    assert!(!classifier.contains("Box<"));
    assert!(state.contains("pub fn apply_alignment_request("));
    assert!(state.contains("Starting a v-template is intentionally absent"));
    assert!(interception.contains("pub fn begin_alignment_v_template("));
    assert!(interception.contains("AlignmentDeliveryEvent::EndTemplate"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn scanner_status_lifetimes_have_one_processor_episode_mechanism() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command/src");
    let status = fs::read_to_string(manifest_dir.join("processor/status.rs"))
        .expect("read scanner-status implementation");
    assert!(status.contains("fn begin_scanner_episode("));
    assert!(status.contains("fn finish_scanner_episode("));
    assert!(status.contains("fn resume_scanner_episode_after_recovery("));

    for relative in [
        "scan_toks.rs",
        "macro_call.rs",
        "conditionals.rs",
        "processor/expand.rs",
        "scanners/structured.rs",
    ] {
        let source = fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        assert!(
            !source.contains(".begin_scanner_status("),
            "{relative} bypasses the processor scanner episode"
        );
    }

    let scan_toks = fs::read_to_string(manifest_dir.join("scan_toks.rs")).expect("read scan_toks");
    assert!(scan_toks.contains("let config = ScanToksConfig::parse(mode);"));
    assert_eq!(scan_toks.matches("match mode {").count(), 1);
    assert!(scan_toks.contains("`read_toks` is deliberately not a `scan_toks` mode"));
}

fn dependency_names(manifest: &str) -> BTreeSet<&str> {
    let mut in_dependencies = false;
    let mut names = BTreeSet::new();

    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if in_dependencies
            && !line.is_empty()
            && !line.starts_with('#')
            && let Some((name, _)) = line.split_once('=')
        {
            names.insert(name.trim());
        }
    }

    names
}

#[allow(clippy::disallowed_methods)] // host-side architecture test helper
fn production_rust_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        {
            let path = entry.expect("read production source entry").path();
            if path.is_dir() {
                if path.file_name().is_none_or(|name| name != "tests") {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                sources.push(path);
            }
        }
    }
    sources.sort();
    sources
}

#[test]
fn command_state_machines_are_private() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &manifest_dir)];

    assert_compile_fail(
        "command-private-modules",
        &manifest_dir.join("tests/ui/private_modules.rs"),
        &dependencies,
        &[
            "E0603",
            "module `conditionals` is private",
            "module `input` is private",
            "module `macro_call` is private",
            "module `primitives` is private",
            "module `processor` is private",
            "module `scan_toks` is private",
            "module `scanners` is private",
        ],
    );
}

#[test]
fn lexical_attempt_ids_cannot_escape_their_scope() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &manifest_dir)];

    assert_compile_fail(
        "attempt-scope-escape",
        &manifest_dir.join("tests/ui/attempt_scope_escape.rs"),
        &dependencies,
        &["lifetime may not live long enough"],
    );
}

#[test]
fn command_attempt_operation_cannot_be_forged() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &manifest_dir)];

    assert_compile_fail(
        "attempt-operation-forgery",
        &manifest_dir.join("tests/ui/attempt_operation_forgery.rs"),
        &dependencies,
        &[
            "E0451",
            "field `_private` of struct `CommandAttemptOperation` is private",
        ],
    );
}

#[test]
fn semantic_and_runtime_fields_are_opaque() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &manifest_dir)];

    assert_compile_fail(
        "command-opaque-state",
        &manifest_dir.join("tests/ui/opaque_state.rs"),
        &dependencies,
        &["E0616", "field `input`", "field `generation`"],
    );
}

#[test]
fn command_profile_and_installed_mode_are_immutable() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [CompileFailDependency::path("tex-command", &manifest_dir)];

    assert_compile_fail(
        "command-immutable-profile",
        &manifest_dir.join("tests/ui/immutable_profile.rs"),
        &dependencies,
        &[
            "E0616",
            "field `dialect`",
            "field `characters`",
            "field `expansion`",
        ],
    );
}

#[test]
fn host_context_cannot_be_serialized() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [
        CompileFailDependency::path("tex-command", &manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "command-host-serialization",
        &manifest_dir.join("tests/ui/host_serialization.rs"),
        &dependencies,
        &[
            "CommandHostCapabilities",
            "CommandHostContext",
            "Serialize",
            "DeserializeOwned",
            "Clone",
        ],
    );
}

#[test]
fn ephemeral_command_types_cannot_be_serialized() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let dependencies = [
        CompileFailDependency::path("tex-command", &manifest_dir),
        CompileFailDependency::registry("serde", "1"),
    ];

    assert_compile_fail(
        "command-ephemeral-serialization",
        &manifest_dir.join("tests/ui/ephemeral_serialization.rs"),
        &dependencies,
        &[
            "CurrentCommand",
            "CommandProcessor",
            "Serialize",
            "DeserializeOwned",
        ],
    );
}

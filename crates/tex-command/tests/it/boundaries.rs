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

    assert!(scalar.contains("pub enum RetainedScalarScan<T>"));
    assert!(scalar.contains("pub struct ScalarScanFrame"));
    let scalar_frame = scalar
        .split("pub struct ScalarScanFrame")
        .nth(1)
        .and_then(|tail| tail.split("impl ScalarScanFrame").next())
        .expect("locate scalar result slot");
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
    assert!(!command_episode.contains("OperationPayload"));
    for forbidden in ["Box<", "Vec<", "Arc<", "VecDeque", "HashMap"] {
        assert!(!command_episode.contains(forbidden));
    }
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

// The former control-lane source guard prescribed stored caller phases that no
// longer exist; concrete-loop and nested scanner tests cover the live owner.
#[test]
#[allow(clippy::disallowed_methods)] // host-side architectural source guard
fn expansion_primitives_and_scanners_use_typed_delivery_requests() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let production = [
        "src/conditionals.rs",
        "src/scan_toks.rs",
        "src/scanners/expression.rs",
        "src/scanners/font.rs",
        "src/scanners/hyphenation.rs",
        "src/scanners/scalar.rs",
        "src/scanners/structured.rs",
        "src/scanners/token_list.rs",
        "src/processor/alignment_interception.rs",
        "src/processor/backup.rs",
        "src/processor/expand_structural.rs",
        "src/processor/expand_convert.rs",
        "src/processor/expand_input.rs",
        "src/processor/expand_render.rs",
        "src/processor/expand_replay.rs",
        "src/processor/expand_pdf.rs",
        "src/processor/expand_pdf_file.rs",
        "src/processor/expand_pdf_string.rs",
    ];
    let forbidden = [
        "self.expanded_next(",
        "self.get_x_token(",
        "self.get_x_token_into(",
        "self.x_token_next(",
        "self.expand_into(",
    ];
    for relative in production {
        let source = fs::read_to_string(manifest_dir.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"));
        // Keep this guard focused on executable call edges. Documentation in
        // these modules names the TeX routines being replaced, but cannot
        // create a Rust re-entry edge.
        let executable = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for call in forbidden {
            assert!(
                !executable.contains(call),
                "{relative} must return a typed request instead of recursively entering delivery through {call}"
            );
        }
    }

    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read typed request boundary");
    let request_start = expansion
        .find("pub(crate) fn request_expanded_token(")
        .expect("locate typed expanded-token request");
    let request_tail = &expansion[request_start..];
    let request_body = request_tail
        .split("    /// Requests one already-delivered command's expansion")
        .next()
        .expect("typed expanded-token request has a bounded body");
    assert_eq!(
        request_body
            .matches("self.get_x_token_into(destination)")
            .count(),
        1,
        "the typed expanded-token request must have one canonical driver bridge"
    );
}

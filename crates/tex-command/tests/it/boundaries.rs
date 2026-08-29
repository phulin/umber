use std::{collections::BTreeSet, fs};

use test_support::{CompileFailDependency, assert_compile_fail};

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
    assert!(levels.contains("CapturedStackState::Compact"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn raw_delivery_keeps_one_profile_shared_input_path_and_semantic_free_levels() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read typed delivery driver");
    let input_stack = fs::read_to_string(manifest_dir.join("src/input/stack.rs"))
        .expect("read input-top transition");
    let levels = fs::read_to_string(manifest_dir.join("src/input/levels.rs"))
        .expect("read input-level representation");

    assert_eq!(
        next.matches("fn deliver_raw_input_into(").count(),
        1,
        "the command core must have exactly one destination-directed input delivery loop"
    );
    assert_eq!(
        next.matches("fn get_next_canonical(").count(),
        1,
        "the command core must have exactly one canonical raw-command loop"
    );
    for retired in ["fn take_input_token(", "ActiveInput", "DeliveredToken"] {
        assert!(
            !next.contains(retired),
            "raw delivery must not retain the retired {retired} envelope"
        );
    }
    assert!(next.contains("Some(CurrentCommand::empty())"));
    assert!(next.contains("self.deliver_raw_input_into(command)"));
    assert!(next.contains("command.resolve_raw_delivery("));
    assert!(levels.contains("destination: &mut crate::CurrentCommand<G>"));
    for retired in ["RawDeliverySlot", "resolve_into"] {
        assert!(
            !format!("{next}\n{levels}").contains(retired),
            "raw delivery must write the canonical command directly, without {retired}"
        );
    }
    assert_eq!(
        input_stack.matches("fn transition_input_top_into(").count(),
        1,
        "source and stored input must share one destination-directed top transition"
    );
    let input_top_transition = input_stack
        .split("fn transition_input_top_into(")
        .nth(1)
        .and_then(|tail| tail.split("/// Acquires, firms, registers").next())
        .expect("locate warmed input-top transition");
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
    assert!(input_stack.contains("CharacterMode::EightBitExact"));
    assert!(input_stack.contains("CharacterMode::UnicodeExtended"));
    assert!(
        !expansion.contains("ControlSequenceCreation"),
        "canonical command delivery must not carry source-name creation policy"
    );
    assert_eq!(
        expansion.matches("self.get_next_canonical(").count(),
        2,
        "raw and expanded drivers must share canonical ID delivery"
    );
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
        ".settle_current_command(",
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
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");

    assert_eq!(
        next.matches("fn check_outer_validity_entry(").count(),
        1,
        "outer-command detection must have one raw-delivery entry point"
    );
    assert_eq!(
        next.matches("fn recover_runaway_eof(").count(),
        1,
        "EOF legality must have one raw-delivery entry point"
    );
    assert_eq!(
        next.matches("fn install_outer_recovery(").count(),
        1,
        "outer commands and runaway EOF must share one recovery table"
    );
    // The boolean argument is §336's `cur_cs<>0` test, which selects the
    // first help line; both entry points still share the one recovery table.
    assert_eq!(
        next.matches("self.install_outer_recovery(recovery, ")
            .count(),
        2,
        "only outer-command and runaway-EOF entry points may install recovery"
    );
    assert!(next.contains("self.back_input(command.copy_for_backup())?;"));
    assert!(next.contains("self.command.scanner.clear_for_recovery();"));
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
fn command_delivery_has_specialized_typed_loops_and_direct_input_mutation() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read ordinary expansion implementation");
    let policies = fs::read_to_string(manifest_dir.join("src/processor/mod.rs"))
        .expect("read delivery policy definitions");
    let raw = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery entry points");

    assert_eq!(
        expansion.matches("fn raw_delivery_driver(").count(),
        1,
        "raw production delivery must have one policy loop"
    );
    assert_eq!(
        expansion.matches("fn expanded_delivery_driver(").count(),
        1,
        "expanded production delivery must have one policy loop"
    );
    for (policy_axis, variants) in [
        ("ReplayCompletionPolicy", &["Consume", "Surface"][..]),
        (
            "ExpandedObservationPolicy",
            &["Commit", "RawOnly", "DeferIfExpanded"][..],
        ),
        ("FirstCommandPolicy", &["Ordinary", "MainLoopCharacter"][..]),
        (
            "AlignmentInterceptionPolicy",
            &["Scalar", "Surface", "None"][..],
        ),
    ] {
        assert!(
            policies.contains(&format!("enum {policy_axis}")),
            "typed delivery must select the {policy_axis} axis explicitly"
        );
        for variant in variants {
            assert!(
                policies.contains(&format!("    {variant},")),
                "{policy_axis} must retain its {variant} policy"
            );
        }
    }
    assert!(
        !policies.contains("ControlSequenceCreation"),
        "name creation belongs to source tokenization, not delivery policy"
    );
    assert!(expansion.contains("ProtectedMacroHandling"));
    assert!(expansion.contains("UndefinedHandling"));
    assert!(
        !format!("{expansion}\n{raw}").contains("pending_expanded_delivery"),
        "pending observation ownership must be typed, never a boolean"
    );
    assert!(expansion.contains("fn expand(&mut self, command: &CurrentCommand<G>)"));
    assert!(expansion.contains("match self.macro_call(command)?"));
    assert!(expansion.contains("MacroCallOutcome::Activated"));
    assert!(expansion.contains("MacroCallOutcome::PrefixMismatchRecovered"));
    assert!(expansion.contains("match self.expand_with_trace("));
    assert!(expansion.contains("suppress_first_expansion_trace"));
    assert!(expansion.contains(".store_expansion_frame(pending)"));
    assert!(expansion.contains("expand_owned_with_trace("));
    assert!(policies.contains("take_pending_expansion_work"));
    assert!(expansion.contains("ChildContinuation::capture("));
    assert!(expansion.contains("PendingExpansionChildDestination::Dispatch"));
    assert!(expansion.contains(".store_expandafter_frame(PendingExpandAfter"));
    assert!(expansion.contains(".store_pdf_string_compare_frame(PendingPdfStringCompare"));
    assert!(expansion.contains("PdfStringComparePhase::Right { left }"));
    assert!(expansion.contains("PendingExpansionResume::CsName { name }"));
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
        !expansion.contains("retain_pending_expansion")
            && !expansion.contains("retain_pending_expandafter"),
        "resource retry ownership must stay in the typed scratch continuation chain"
    );
    assert!(
        !expansion.contains("let retry = command.clone();"),
        "ordinary expansion must move the live command only at a typed retry barrier"
    );
    assert!(expansion.contains("fn expand_noexpand("));
    assert!(expansion.contains("fn expand_expandafter("));
    for forbidden in ["Dispatch::Push", "Dispatch::PushTransient", "ExpansionMode"] {
        assert!(
            !expansion.contains(forbidden),
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
    let collector = scanner
        .split("fn collect_replacement(")
        .nth(1)
        .and_then(|tail| tail.split("/// Splices a token-list result").next())
        .expect("locate expanded token-list collector");
    let splice = scanner
        .split("fn append_direct_the_toks(")
        .nth(1)
        .and_then(|tail| tail.split("/// e-TeX").next())
        .expect("locate direct token-list splice");

    assert_eq!(scanner.matches("fn scan_toks_inner(").count(), 1);
    assert!(collector.contains("self.get_next_into(&mut destination)"));
    assert!(collector.contains(".as_mut()"));
    assert!(collector.contains("clear_command_destination(&mut destination)"));
    assert!(collector.contains("pending_expansion.take()"));
    assert!(collector.contains("PendingCollectorExpansion"));
    assert!(collector.contains("self.expand(command)"));
    assert!(collector.contains("self.append_direct_the_toks(output, &mut expansion_operand)"));
    assert!(
        !collector.contains("self.get_x_token()?"),
        "the replacement collector must not enter a second ordinary expansion loop"
    );
    assert!(
        splice.contains("self.get_x_token_into(target)?"),
        "\\the must retain its expanded internal-value target before selecting a token list"
    );
    assert!(splice.contains("self.push_scan_toks_word(output, token)?"));
    assert!(scanner.contains("arena.push_definition_replacement(definition, word.token_word())"));
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
fn definition_promotion_moves_the_checked_builder_and_preflights_its_policy() {
    let repository = test_support::repository_root();
    let attempt = fs::read_to_string(repository.join("crates/tex-command/src/attempt.rs"))
        .expect("read attempt promotion implementation");
    let stores = fs::read_to_string(repository.join("crates/tex-state/src/stores.rs"))
        .expect("read destination promotion implementation");
    let promotion = attempt
        .split("pub(crate) fn promote(")
        .nth(1)
        .and_then(|tail| tail.split("pub(crate) fn promote_definition(").next())
        .expect("locate generic attempt promotion");
    let destination = stores
        .split("pub(crate) fn promote_values(")
        .nth(1)
        .and_then(|tail| tail.split("pub(crate) fn promote_format_values(").next())
        .expect("locate generic destination promotion");

    assert!(promotion.contains("DefinitionPromotion::new("));
    assert!(promotion.contains(".builder\n                    .take()"));
    assert!(!promotion.contains("parameter_text().to_vec()"));
    assert!(!promotion.contains("replacement_text().to_vec()"));
    assert!(!destination.contains("DefinitionBuilder::from_slices"));
    let validation = destination
        .find("definitions_arena.validate_builder(definition.builder())")
        .expect("destination-policy preflight");
    let publication = destination
        .find(".publish(definition.builder())")
        .expect("checked builder publication");
    assert!(validation < publication);
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

    let main_control = fs::read_to_string(repository.join("crates/tex-exec/src/main_control.rs"))
        .expect("read main-control continuation architecture");
    let operation_frame = main_control
        .split("struct PreflightCommand<G>")
        .nth(1)
        .and_then(|tail| tail.split("impl<G> PreflightCommand<G>").next())
        .expect("locate singular preflight command owner");
    assert!(operation_frame.contains("command: Option<tex_command::CurrentCommand<G>>"));
    assert!(operation_frame.contains("phase: PreflightCommandPhase"));
    assert!(operation_frame.contains("scanner: Option<tex_command::ScannerFrameKey<G>>"));
    assert!(operation_frame.contains("operation_scan: Option<PendingOperationScanPhase>"));
    for forbidden in ["Box<", "Vec<", "Arc<", "VecDeque", "HashMap"] {
        assert!(!operation_frame.contains(forbidden));
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn condition_delivery_and_alignment_lifecycle_remain_on_the_canonical_seams() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let conditionals = fs::read_to_string(manifest_dir.join("src/conditionals.rs"))
        .expect("read conditional implementation");
    let input =
        fs::read_to_string(manifest_dir.join("src/input/mod.rs")).expect("read input facade");
    let state = fs::read_to_string(manifest_dir.join("src/state.rs")).expect("read command state");
    let alignment = fs::read_to_string(manifest_dir.join("src/processor/alignment.rs"))
        .expect("read alignment delivery implementation");
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
    let ifx_comparison = conditionals
        .split("fn ifx_meaning_eq(")
        .nth(1)
        .and_then(|tail| tail.split("fn scan_if_relation(").next())
        .expect("locate borrowed ifx meaning comparison");
    assert!(ifx_comparison.contains("first_definition.parameter_text()"));
    assert!(ifx_comparison.contains("second_definition.replacement_text()"));
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
    assert_eq!(alignment.matches("fn classify_delivery(").count(), 1);
    assert_eq!(
        next.matches("self.command.alignment.classify_delivery(")
            .count(),
        1
    );
    assert!(state.contains("pub fn apply_alignment_request("));
    assert!(state.contains("Starting a v-template is intentionally absent"));
    assert!(next.contains("pub fn begin_alignment_v_template("));
    assert!(next.contains("AlignmentDeliveryEvent::EndTemplate"));
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

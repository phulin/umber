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
fn raw_delivery_keeps_one_profile_shared_input_path_and_semantic_free_levels() {
    let manifest_dir = test_support::repository_root().join("crates/tex-command");
    let next = fs::read_to_string(manifest_dir.join("src/processor/next.rs"))
        .expect("read raw delivery implementation");
    let expansion = fs::read_to_string(manifest_dir.join("src/processor/expand.rs"))
        .expect("read typed delivery driver");
    let levels = fs::read_to_string(manifest_dir.join("src/input/levels.rs"))
        .expect("read input-level representation");

    assert_eq!(
        next.matches("fn take_input_token(").count(),
        1,
        "the command core must have exactly one scalar input-level delivery loop"
    );
    assert_eq!(
        next.matches("fn next_source_step(").count(),
        1,
        "exact and Unicode profiles must select beneath the shared delivery loop"
    );
    assert!(next.contains("CharacterMode::EightBitExact"));
    assert!(next.contains("CharacterMode::UnicodeExtended"));
    assert!(expansion.contains("ControlSequenceCreation::Allow"));
    assert!(expansion.contains("ControlSequenceCreation::Forbid"));
    assert_eq!(
        expansion
            .matches("self.get_next_with_control_sequence_creation(")
            .count(),
        2,
        "the raw and expanded typed drivers must be the only scalar-delivery callers"
    );
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
        matcher.contains("self.get_token()?"),
        "the scalar matcher must consume raw tokens through get_token"
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
        ("ControlSequenceCreation", &["Forbid", "Allow"][..]),
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
    assert!(expansion.contains("self.command.retain_pending_expansion(command);"));
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
    assert!(collector.contains("match self.get_next()"));
    assert!(collector.contains("pending_expansion.take()"));
    assert!(collector.contains("PendingCollectorExpansion"));
    assert!(collector.contains("self.expand(&command)"));
    assert!(collector.contains("self.append_direct_the_toks(output)"));
    assert!(
        !collector.contains("self.get_x_token()?"),
        "the replacement collector must not enter a second ordinary expansion loop"
    );
    assert!(
        splice.contains("let target = self.get_x_token()?"),
        "\\the must expand its internal-value target before selecting a token list"
    );
    assert!(splice.contains("self.push_attempt_token(output, token)?"));
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
    assert!(pass_text.contains("self.get_next()?"));
    assert!(pass_text.contains("nested_conditions"));
    assert!(conditionals.contains("fn evaluate_ifx("));
    let ifx = conditionals
        .split("fn evaluate_ifx(")
        .nth(1)
        .and_then(|tail| tail.split("fn evaluate_numeric_comparison(").next())
        .expect("locate raw ifx operand comparison");
    // TeX82 §507 reads both operands with `get_next`. The distinction is not
    // cosmetic: §365 clears `no_new_control_sequence` only inside
    // `get_token`, so reading an `\ifx` operand must not enter a new name in
    // the hash table.
    assert_eq!(ifx.matches("self.get_next()").count(), 2);
    assert!(!ifx.contains("self.get_token()?"));
    assert!(!ifx.contains("self.get_x_token()?"));
    assert!(ifx.contains("begin_scanner_episode(ScannerStatus::Normal"));
    assert!(ifx.contains("finish_scanner_episode(episode)"));
    assert!(conditionals.contains("fn expand_unless("));
    assert!(conditionals.contains("inverted"));
    assert!(!input.contains("ConditionStack"));

    // TeX82 §331 gives the run exactly one `align_state`; §772's
    // `push_alignment`/`pop_alignment` save and restore copies of it on the
    // alignment stack rather than giving any other record its own field.
    assert_eq!(alignment.matches("align_state: i32").count(), 1);
    assert_eq!(alignment.matches("align_stack: Vec<i32>").count(), 1);
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

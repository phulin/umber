use std::fs;

use test_support::{CompileFailDependency, assert_compile_fail};

fn production_rust_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).expect("read production source directory") {
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
    sources
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_token_rendering_stays_on_the_state_owner() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    for path in production_rust_sources(&manifest_dir.join("src")) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::append_token_show_text",
            "tex_expand::append_token_string_text",
            "tex_expand::append_token_selector_text",
            "tex_expand::token_text",
            "tex_expand::semantic_token",
            "tex_expand::meaning_text",
            "tex_expand::bounded_meaning_text",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use tex_state::token_show instead of `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_replay_kinds_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_lex::TokenListReplayKind"),
            "{} must use tex_state::TokenListReplayKind",
            path.display()
        );
        assert!(
            !source
                .lines()
                .any(|line| line.contains("tex_lex::{") && line.contains("TokenListReplayKind")),
            "{} must not import TokenListReplayKind through tex-lex",
            path.display()
        );
        assert!(
            !source.contains("tex_lex::TokenListReplayMarker"),
            "{} must use tex_state::TokenListReplayMarker",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_lex::{") && line.contains("TokenListReplayMarker")
            }),
            "{} must not import TokenListReplayMarker through tex-lex",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_page_output_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_page_output.rs"))
        .expect("read canonical page-output module");
    for forbidden in [
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_output",
        "run_main_control_until",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_page_output.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn legacy_output_has_no_shipped_command_control_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_output") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some("lib.rs" | "executor.rs" | "align/execution.rs" | "legacy_output.rs")
                ),
                "{} must not call the retired output front",
                relative.display()
            );
        }
    }
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_output"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_diagnostics_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_diagnostics.rs"))
        .expect("read canonical diagnostics module");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "legacy_diagnostics",
        "raw_delivery",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_diagnostics.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn legacy_diagnostics_has_no_canonical_command_control_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("legacy_diagnostics") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "executor.rs"
                            | "assignments/mod.rs"
                            | "assignments/scanning.rs"
                            | "assignments/boxes/packaging.rs"
                            | "legacy_diagnostics.rs"
                    )
                ),
                "{} must not call the retired diagnostic scanner front",
                relative.display()
            );
        }
    }
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("legacy_diagnostics"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_assignment_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let owner_root = source_root.join("canonical_assignments");
    for path in production_rust_sources(&owner_root) {
        let source = fs::read_to_string(&path).expect("read canonical assignment source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "crate::assignments",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must not reference legacy boundary `{forbidden}`",
                path.strip_prefix(&source_root)
                    .expect("canonical assignment source below root")
                    .display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_assignment_owner_has_only_declared_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("canonical_assignments") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some(
                        "lib.rs"
                            | "canonical_main_control.rs"
                            | "assignments/mod.rs"
                            | "assignments/variables.rs"
                            | "canonical_assignments/mod.rs"
                    )
                ),
                "{} must not bypass the canonical assignment owner",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_paragraph_memo_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let source = fs::read_to_string(source_root.join("canonical_paragraph_memo.rs"))
        .expect("read canonical paragraph-memo module");
    for forbidden in [
        "tex_expand",
        "tex_lex",
        "InputStack",
        "ExecutionContext",
        "crate::executor",
        "paragraph_memo::",
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical_paragraph_memo.rs must not reference legacy boundary `{forbidden}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_paragraph_replay_bypasses_the_legacy_memo_front() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    for helper in [
        "validate_dependencies",
        "same_mutation_entry_class",
        "validate_mutations",
        "replay_mutations",
    ] {
        assert!(
            canonical.contains(&format!("canonical_paragraph_memo::{helper}")),
            "canonical command control must call canonical paragraph helper `{helper}`"
        );
    }
    for retired in [
        "crate::paragraph_memo::validate_canonical",
        "crate::paragraph_memo::same_mutation_entry_class",
        "crate::paragraph_memo::replay_canonical",
    ] {
        assert!(
            !canonical.contains(retired),
            "canonical command control must bypass retired paragraph front `{retired}`"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_math_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src/math");
    for relative in ["mod.rs", "display.rs", "lower.rs", "support.rs"] {
        let source = fs::read_to_string(source_root.join(relative))
            .expect("read canonical math-family source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "legacy_front::",
            "legacy_scan::",
        ] {
            assert!(
                !source.contains(forbidden),
                "canonical math source {relative} must not reference legacy boundary `{forbidden}`"
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_math_front_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("math::legacy_front"));
    assert!(!canonical.contains("math::legacy_scan"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("math::legacy_front") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some("dispatch.rs" | "paragraph_memo.rs" | "assignments/mod.rs")
                ),
                "{} must not call the retired math front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_alignment_family_has_no_legacy_dependencies() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src/align");
    let mut canonical = vec![
        source_root.join("mod.rs"),
        source_root.join("canonical_execution.rs"),
        source_root.join("packaging.rs"),
        source_root.join("support.rs"),
        source_root.join("transitions.rs"),
    ];
    canonical.extend(production_rust_sources(&source_root.join("widths")));
    for path in canonical {
        let source = fs::read_to_string(&path).expect("read canonical alignment source");
        for forbidden in [
            "tex_expand",
            "tex_lex",
            "InputStack",
            "ExecutionContext",
            "crate::executor",
            "legacy_front::",
            "legacy_execution::",
        ] {
            assert!(
                !source.contains(forbidden),
                "canonical alignment source {} must not reference legacy boundary `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_command_control_has_no_legacy_alignment_callers() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let canonical = fs::read_to_string(source_root.join("canonical_main_control.rs"))
        .expect("read canonical command control");
    assert!(!canonical.contains("align::legacy_front"));
    assert!(!canonical.contains("align::legacy_execution"));

    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        if source.contains("align::legacy_front") || source.contains("align::legacy_execution") {
            let relative = path.strip_prefix(&source_root).expect("source below root");
            assert!(
                matches!(
                    relative.to_str(),
                    Some("dispatch.rs" | "assignments/mod.rs" | "math/legacy_front.rs")
                ),
                "{} must not call the retired alignment front",
                relative.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_raw_token_delivery_bypasses_the_expand_compatibility_boundary() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::next_semantic_raw_token",
            "tex_expand::get_token",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use the input owner's raw delivery instead of `{forbidden}`",
                path.display()
            );
        }
        if path
            .file_name()
            .is_none_or(|name| name != "raw_delivery.rs")
        {
            assert!(
                !source.contains("tex_lex::next_semantic_raw_token"),
                "{} must cross raw delivery only through raw_delivery.rs",
                path.display()
            );
        }
    }
    let bridge = fs::read_to_string(source_root.join("raw_delivery.rs"))
        .expect("read retired raw-delivery bridge");
    assert_eq!(
        bridge.matches("tex_lex::next_semantic_raw_token").count(),
        1
    );
    let expand =
        fs::read_to_string(test_support::repository_root().join("crates/tex-expand/src/lib.rs"))
            .expect("read expansion public surface");
    assert!(!expand.contains("pub fn next_semantic_raw_token("));
    assert!(!expand.contains("pub fn get_token("));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_mode_snapshots_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in ["tex_expand::EngineMode", "tex_expand::EngineStateSnapshot"] {
            assert!(
                !source.contains(forbidden),
                "{} must use the tex-state-owned mode snapshot instead of `{forbidden}`",
                path.display()
            );
        }
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{")
                    && (line.contains("EngineMode") || line.contains("EngineStateSnapshot"))
            }),
            "{} must not import mode snapshot types through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_dimension_diagnostics_stay_on_the_command_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::scan_dimen::DimensionDiagnostic"),
            "{} must use tex_command::DimensionDiagnostic",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{") && line.contains("DimensionDiagnostic")
            }),
            "{} must not import DimensionDiagnostic through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_recoverable_expansion_diagnostics_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::RecoverableExpansionDiagnostic"),
            "{} must use tex_state::RecoverableExpansionDiagnostic",
            path.display()
        );
        assert!(
            !source.lines().any(|line| {
                line.contains("tex_expand::{") && line.contains("RecoverableExpansionDiagnostic")
            }),
            "{} must not import RecoverableExpansionDiagnostic through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_paragraph_barriers_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::ParagraphExpansionBarrier",
            "tex_expand::PARAGRAPH_SCANTOKENS_BARRIER_DOMAIN",
            "tex_expand::PARAGRAPH_INPUT_OPEN_BARRIER_DOMAIN",
            "tex_expand::PARAGRAPH_END_INPUT_BARRIER_DOMAIN",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use the tex-state-owned paragraph barrier contract instead of `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_inserted_input_stays_on_the_input_stack_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        assert!(
            !source.contains("tex_expand::insert_input"),
            "{} must insert never-delivered tokens through InputStack instead of tex-expand",
            path.display()
        );
        assert!(
            !source
                .lines()
                .any(|line| line.contains("use tex_expand::{") && line.contains("insert_input")),
            "{} must not import insert_input through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_backed_up_input_stays_on_the_input_stack_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in ["tex_expand::back_input", "tex_expand::back_error_input"] {
            assert!(
                !source.contains(forbidden),
                "{} must return delivered tokens through InputStack instead of `{forbidden}`",
                path.display()
            );
        }
        assert!(
            !source.lines().any(|line| {
                line.contains("use tex_expand::{")
                    && (line.contains("back_input") || line.contains("back_error_input"))
            }),
            "{} must not import token-backup helpers through tex-expand",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn executor_resource_results_stay_on_the_execution_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let executor = fs::read_to_string(source_root.join("executor.rs")).expect("read executor");
    let public_surface =
        fs::read_to_string(source_root.join("lib.rs")).expect("read public surface");

    assert!(executor.contains("pub enum ResourceLookup<T>"));
    assert!(executor.contains("pub struct ResourceNeed"));
    for (source_name, source) in [("executor", executor), ("public surface", public_surface)] {
        for forbidden in [
            "tex_expand::ResourceLookup",
            "tex_expand::ResourceResult",
            "pub use tex_expand::ResourceNeed",
            "pub use tex_expand::{ResourceLookup",
        ] {
            assert!(
                !source.contains(forbidden),
                "{source_name} must not regain the retired resource-result owner through `{forbidden}`"
            );
        }
    }

    match tex_exec::ResourceLookup::Available(21_u8).map(u16::from) {
        tex_exec::ResourceLookup::Available(value) => assert_eq!(value, 21),
        _ => panic!("available executor resource must remain available after mapping"),
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn expansion_resource_lookup_values_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::ResourceLookup",
            "tex_expand::ResourceResult",
            "tex_expand::ResourceNeed",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} must use shared state resource values instead of {forbidden}",
                path.display()
            );
        }
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
        &["fields `limit` and `burned` of struct `CommandFuel` are private"],
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
    let tex_lex_dir = manifest_dir.join("../tex-lex");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-exec", &manifest_dir),
        CompileFailDependency::path("tex-lex", &tex_lex_dir),
        CompileFailDependency::path("tex-state", &tex_state_dir),
    ];
    assert_compile_fail(
        "engine-checkpoint-forgery-forbidden",
        &manifest_dir.join("tests/ui/engine_checkpoint_forgery_forbidden.rs"),
        &dependencies,
        &["cannot construct `EngineCheckpoint`", "private fields"],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_checkpoints_do_not_forward_legacy_input_summaries() {
    let crate_root = test_support::repository_root().join("crates/tex-exec");
    let checkpoint =
        fs::read_to_string(crate_root.join("src/checkpoint.rs")).expect("read checkpoint boundary");
    let public_surface =
        fs::read_to_string(crate_root.join("src/lib.rs")).expect("read public surface");

    assert!(checkpoint.contains("enum CheckpointContinuation"));
    assert!(checkpoint.contains("Canonical(Box<CommandSummary>)"));
    assert!(checkpoint.contains("LegacyInput(InputSummary)"));
    assert!(
        !checkpoint.contains("input: InputSummary,"),
        "aggregate checkpoints must not always carry a legacy input continuation"
    );
    assert!(
        !checkpoint.contains("input: InputSummary::default()"),
        "canonical checkpoints must not encode absent legacy input with a sentinel"
    );
    assert!(
        !checkpoint.contains("pub fn restore_checkpoint<E"),
        "the dead generic InputStack reconstruction API must not return"
    );
    assert!(!checkpoint.contains("pub enum EngineRestoreError"));
    assert!(!public_surface.contains("EngineRestoreError"));
    for forbidden in [
        "InputStack::from_summary",
        "MemoryInput::from_offset",
        "WorldInput::from_content_at_offset",
        "LayoutCursor::new",
        "restore_editor_checkpoint",
    ] {
        assert!(
            !checkpoint.contains(forbidden),
            "checkpoint schema must not reconstruct retired editor input through {forbidden}"
        );
    }
}

#[test]
fn scoped_execution_transaction_cannot_escape_public_api() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let dependencies = [CompileFailDependency::path("tex-exec", &manifest_dir)];
    assert_compile_fail(
        "execution-transaction-private",
        &manifest_dir.join("tests/ui/execution_transaction_private.rs"),
        &dependencies,
        &["E0603", "module `transaction` is private"],
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn mode_list_mutation_capabilities_do_not_expose_mutable_aggregate_references() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let mode = fs::read_to_string(manifest_dir.join("src/mode.rs"))
        .expect("read mode-list mutation boundary");

    for forbidden in [
        "fn current_list_mut(",
        "fn list_mut(",
        "fn reconstitution_target(",
        "fn align_state_mut(",
        "impl DerefMut for ModeListMutation",
        "impl AsMut<ModeList> for ModeListMutation",
        "impl BorrowMut<ModeList> for ModeListMutation",
        "fn apply<R>(self",
    ] {
        assert!(
            !mode.contains(forbidden),
            "mode-list mutation boundary must not expose `{forbidden}`"
        );
    }
    for forbidden_return in [
        "-> &mut ModeList",
        "-> Option<&mut ModeList>",
        "-> &mut Vec<Node>",
        "-> Option<&mut Vec<Node>>",
        "-> &mut Node",
        "-> Option<&mut Node>",
        "-> &mut AlignState",
        "-> Option<&mut AlignState>",
    ] {
        assert!(
            !mode.contains(forbidden_return),
            "mode-list API must not return `{forbidden_return}`"
        );
    }
    assert!(
        mode.contains("impl for<'a> FnOnce(&'a mut Node)")
            && mode.contains("impl for<'a> FnOnce(&'a mut Vec<Node>)")
            && mode.contains("impl for<'a> FnOnce(&'a mut AlignState)"),
        "pre-existing aggregate edits must remain behind higher-ranked write barriers"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn canonical_main_control_has_one_command_owned_delivery_and_aggregate_rollback_boundary() {
    let manifest_dir = test_support::repository_root().join("crates/tex-exec");
    let driver = fs::read_to_string(manifest_dir.join("src/canonical_main_control.rs"))
        .expect("read canonical main-control boundary");

    for forbidden in [
        "use tex_lex",
        ": InputStack",
        "&mut InputStack",
        "next_semantic_raw_token",
        "crate::executor",
        "Executor::",
    ] {
        assert!(
            !driver.contains(forbidden),
            "canonical main control must receive command-owned delivery, not {forbidden}"
        );
    }
    assert!(
        driver.contains("match command.meaning()"),
        "canonical dispatch must classify typed CurrentCommand meanings"
    );
    assert!(
        !driver.contains("command.token()"),
        "canonical main control must not classify a raw token from CurrentCommand"
    );
    assert_eq!(
        driver.matches("fn snapshot_step(").count(),
        1,
        "canonical main control must have one aggregate snapshot constructor"
    );
    assert_eq!(
        driver.matches("fn rollback_step(").count(),
        1,
        "canonical main control must have one aggregate rollback implementation"
    );
    assert_eq!(
        driver.matches("stores.rollback(").count(),
        1,
        "no family may introduce a separate Universe rollback path"
    );
    assert!(
        !driver.contains("cached_command") && !driver.contains("retained_command"),
        "canonical retries must start a fresh command-owned processor episode"
    );
    assert!(
        driver.contains("struct ObservationBuffer") && driver.contains("pending.flush_into"),
        "observation must be transaction-buffered output from the same command processor, not a cached delivery path"
    );
    // One main-control operation runs several command-processor episodes: the
    // delivery episode, the nested math-field/math-script/`\mathchoice`
    // episodes a host-applied step runs, and the deferred `\output` episode.
    // Each construction site used to decide for itself whether to install the
    // operation's observer, and the nested math episodes never did, so a
    // `^{...}` script field was scanned with zero observations
    // (umber2-johp.195). One constructor, taking the commit slot as a
    // parameter, is what makes that unrepresentable.
    assert_eq!(
        driver.matches("CommandProcessor::new(").count(),
        1,
        "canonical main control must construct every processor episode through one constructor"
    );
    assert_eq!(
        driver.matches(".with_observer(").count(),
        1,
        "whether an episode is observed must be decided in that one constructor"
    );
    assert_eq!(
        driver.matches(".with_fuel(fuel)").count(),
        1,
        "the one constructor must lend the shared run ledger to every processor episode"
    );
    assert_eq!(
        driver.matches("fn command_processor<").count(),
        1,
        "that constructor must be `command_processor`"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_alignment_scanner_phases_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                assert!(
                    !source.contains("tex_lex::AlignmentScannerPhase")
                        && !source.contains("use tex_lex::{AlignmentScannerPhase")
                        && !source.contains(", AlignmentScannerPhase"),
                    "{} must use tex-state's alignment scanner phase identity",
                    path.display()
                );
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn expansion_read_transactions_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                for forbidden in [
                    "tex_expand::ReadRecorder",
                    "tex_expand::ReadRecorderBatch",
                    "tex_expand::ReadSetRecorder",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "{} must use tex-state's transactional read observation owner, not {forbidden}",
                        path.display()
                    );
                }
                for import in source.lines().filter(|line| line.contains("tex_expand")) {
                    assert!(
                        !import.contains("ReadRecorder"),
                        "{} must not import state-owned read observation through tex-expand: {import}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_main_control_recovery_does_not_destructure_expand_errors() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    let mut pending = vec![source_root];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).expect("read tex-exec production source") {
            let entry = entry.expect("read tex-exec production source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "tests") {
                    continue;
                }
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path.file_name().is_none_or(|name| name != "tests.rs")
            {
                let source = fs::read_to_string(&path).expect("read production Rust source");
                for forbidden in [
                    "tex_expand::ExpandError::UndefinedControlSequence",
                    "tex_expand::ExpandError::Captured",
                    "tex_expand::ExpandError::MacroCall",
                    "tex_expand::ExpandError::ExtraConditionalControl",
                    "tex_expand::args::MacroCallError",
                ] {
                    assert!(
                        !source.contains(forbidden),
                        "{} must consume state-owned expansion recovery, not {forbidden}",
                        path.display()
                    );
                }
            }
        }
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn profiling_feature_forwards_only_to_the_axis_owner() {
    let manifest =
        fs::read_to_string(test_support::repository_root().join("crates/tex-exec/Cargo.toml"))
            .expect("read tex-exec manifest");
    assert!(
        manifest.contains("profiling = [\"tex-state/profiling\"]"),
        "tex-exec profiling must forward only to the tex-state axis owner"
    );
    assert!(!manifest.contains("tex-expand/profiling"));
    assert!(!manifest.contains("tex-lex/profiling"));
}

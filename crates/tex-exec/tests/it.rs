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
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side architecture test
fn production_mode_snapshots_stay_on_the_state_owner() {
    let source_root = test_support::repository_root().join("crates/tex-exec/src");
    for path in production_rust_sources(&source_root) {
        let source = fs::read_to_string(&path).expect("read production Rust source");
        for forbidden in [
            "tex_expand::EngineMode",
            "tex_expand::EngineStateSnapshot",
        ] {
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

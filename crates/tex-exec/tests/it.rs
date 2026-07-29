use std::fs;
use std::path::Path;

use test_support::{CompileFailDependency, assert_compile_fail};

#[test]
fn engine_checkpoint_cannot_be_forged_by_callers() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let tex_lex_dir = manifest_dir.join("../tex-lex");
    let tex_state_dir = manifest_dir.join("../tex-state");
    let dependencies = [
        CompileFailDependency::path("tex-exec", manifest_dir),
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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dependencies = [CompileFailDependency::path("tex-exec", manifest_dir)];
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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
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
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
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

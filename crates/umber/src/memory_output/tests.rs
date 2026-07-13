use tex_state::{PrintSink, StreamSlot, Universe, World};

use super::*;

fn output_world() -> Universe {
    let mut stores = Universe::with_world(World::memory());
    let slot = StreamSlot::new(1);
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "shared");
    stores.world_mut().write_text(PrintSink::Terminal, "-term");
    stores.world_mut().write_text(PrintSink::Log, "-log");
    stores.world_mut().open_out(slot, "job.aux");
    stores
        .world_mut()
        .write_text(PrintSink::Stream(slot), "auxiliary");
    stores.world_mut().close_out(slot);
    stores
}

#[test]
fn final_collection_commits_once_without_dropping_or_duplicating_bytes() {
    let mut stores = output_world();
    let output = collect_final_memory_output(&mut stores, &[], 1 << 20).expect("collect output");

    assert_eq!(output.terminal, b"shared-term");
    assert_eq!(output.log, b"shared-log");
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].path, std::path::Path::new("job.aux"));
    assert_eq!(output.files[0].bytes, b"auxiliary");
    assert!(stores.world().effect_records().is_empty());

    let repeated =
        collect_final_memory_output(&mut stores, &[], 1 << 20).expect("idempotent collection");
    assert_eq!(repeated, output);
}

#[test]
fn output_limit_counts_terminal_log_dvi_and_auxiliary_bytes() {
    let mut stores = output_world();
    let error = collect_final_memory_output(&mut stores, &[], 8).expect_err("limit must fail");

    assert!(matches!(
        error,
        MemoryOutputCollectionError::OutputLimitExceeded {
            limit: 8,
            required_at_least
        } if required_at_least > 8
    ));
}

#[test]
fn discarded_attempt_outputs_are_invisible_to_the_final_world() {
    let mut discarded_attempt = output_world();
    let discarded_end = discarded_attempt.world().effect_pos();
    discarded_attempt
        .commit_effects(discarded_end)
        .expect("simulate attempt-local shipout commit");

    let mut final_attempt = Universe::with_world(World::memory());
    final_attempt
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, "final");
    let output =
        collect_final_memory_output(&mut final_attempt, &[], 1 << 20).expect("collect final");

    assert_eq!(output.terminal, b"final");
    assert_eq!(output.log, b"final");
    assert!(output.files.is_empty());
    assert_eq!(
        discarded_attempt.world().memory_output("job.aux"),
        Some(&b"auxiliary"[..])
    );
}

#[test]
fn real_world_is_rejected_after_safe_empty_commit() {
    let mut stores = Universe::with_world(World::real());
    assert!(matches!(
        collect_final_memory_output(&mut stores, &[], 1024),
        Err(MemoryOutputCollectionError::NotMemoryBacked)
    ));
}

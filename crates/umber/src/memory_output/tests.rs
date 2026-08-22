use tex_state::{GenerationBrand, PrintSink, StreamSlot, Universe, World};

use super::*;

fn output_world() -> World {
    let mut world = World::memory();
    let slot = StreamSlot::new(1);
    world.write_text(PrintSink::TerminalAndLog, "shared");
    world.write_text(PrintSink::Terminal, "-term");
    world.write_text(PrintSink::Log, "-log");
    world.open_out(slot, "job.aux");
    world.write_text(PrintSink::Stream(slot), "auxiliary");
    world.close_out(slot);
    world
}

fn with_stores<R>(
    world: World,
    use_stores: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> R {
    crate::with_engine_world(world, use_stores).expect("fresh universe")
}

#[test]
fn final_collection_commits_once_without_dropping_or_duplicating_bytes() {
    with_stores(output_world(), |stores| {
        let output = collect_final_memory_output(stores, &[], 1 << 20).expect("collect output");

        assert_eq!(output.terminal, b"shared-term");
        assert_eq!(output.log, b"shared-log");
        assert_eq!(output.files.len(), 1);
        assert_eq!(output.files[0].path, std::path::Path::new("job.aux"));
        assert_eq!(output.files[0].bytes, b"auxiliary");
        assert!(stores.world().effect_records().is_empty());

        let repeated =
            collect_final_memory_output(stores, &[], 1 << 20).expect("idempotent collection");
        assert_eq!(repeated, output);
    });
}

#[test]
fn output_limit_counts_terminal_log_dvi_and_auxiliary_bytes() {
    with_stores(output_world(), |stores| {
        let error = collect_final_memory_output(stores, &[], 8).expect_err("limit must fail");

        assert!(matches!(
            error,
            MemoryOutputCollectionError::OutputLimitExceeded {
                limit: 8,
                required_at_least
            } if required_at_least > 8
        ));
    });
}

#[test]
fn discarded_attempt_outputs_are_invisible_to_the_final_world() {
    let discarded = output_world();
    let mut discarded_destination = World::memory();
    discarded_destination
        .publish_detached_effect_records(discarded.effect_records())
        .expect("simulate attempt-local shipout commit");
    assert_eq!(
        discarded_destination.memory_output("job.aux"),
        Some(&b"auxiliary"[..])
    );

    with_stores(World::memory(), |final_attempt| {
        final_attempt
            .world_mut()
            .write_text(PrintSink::TerminalAndLog, "final");
        let output =
            collect_final_memory_output(final_attempt, &[], 1 << 20).expect("collect final");

        assert_eq!(output.terminal, b"final");
        assert_eq!(output.log, b"final");
        assert!(output.files.is_empty());
    });
}

#[test]
fn real_world_is_rejected_after_safe_empty_commit() {
    with_stores(World::real(), |stores| {
        assert!(matches!(
            collect_final_memory_output(stores, &[], 1024),
            Err(MemoryOutputCollectionError::NotMemoryBacked)
        ));
    });
}

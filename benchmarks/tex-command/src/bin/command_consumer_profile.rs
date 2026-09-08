#[path = "../command_consumer_core.rs"]
mod command_consumer_core;

#[global_allocator]
static GLOBAL: tex_state::measurement::HotCoreAllocator = tex_state::measurement::HotCoreAllocator;

fn snapshot() -> command_consumer_core::StructuralSnapshot {
    let census = tex_state::measurement::hot_core_census();
    let writes = tex_state::definition_build_write_counters();
    command_consumer_core::StructuralSnapshot {
        macro_expansions: census.macro_expansions,
        definition_direct_stores: writes.direct_stores,
        definition_chunk_transitions: writes.chunk_transitions,
        definition_episode_admissions: writes.episode_admissions,
    }
}

fn main() {
    command_consumer_core::run(command_consumer_core::ProfileHooks {
        name: "profiling",
        instrumented: true,
        snapshot,
    });
}

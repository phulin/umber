#[path = "../command_core_matrix.rs"]
mod command_core_matrix;

#[global_allocator]
static GLOBAL: tex_state::measurement::HotCoreAllocator = tex_state::measurement::HotCoreAllocator;

fn snapshot() -> command_core_matrix::StructuralSnapshot {
    command_core_matrix::StructuralSnapshot {
        macro_expansions: tex_state::measurement::hot_core_census().macro_expansions,
    }
}

fn main() {
    command_core_matrix::run(command_core_matrix::ProfileHooks {
        name: "profiling",
        snapshot,
    });
}

#[path = "../../src/command_core_matrix.rs"]
mod command_core_matrix;

fn main() {
    command_core_matrix::run(command_core_matrix::ProfileHooks::release());
}

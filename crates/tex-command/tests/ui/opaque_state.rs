use tex_command::{CommandState, CommandSummary};

fn inspect_state<G>(state: &CommandState<G>) {
    let _ = &state.input;
}

fn inspect_summary<G>(summary: &CommandSummary<G>) {
    let _ = &summary.generation;
}

fn main() {}

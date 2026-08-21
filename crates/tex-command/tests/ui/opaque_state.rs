use tex_command::{CommandState, CommandStateSnapshot, CommandSummary};

fn inspect_state<G>(state: &CommandState<G>) {
    let _ = &state.input;
}

fn inspect_snapshot<G, Owner>(snapshot: &CommandStateSnapshot<G, Owner>) {
    let _ = &snapshot.generation;
}

fn inspect_summary<G, Owner>(summary: &CommandSummary<G, Owner>) {
    let _ = &summary.generation;
}

fn main() {}

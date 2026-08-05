use tex_command::{CommandRuntime, CommandState};

fn main() {
    let state = CommandState::default();
    let _ = &state.input;

    let snapshot = state.snapshot();
    let _ = &snapshot.state;

    let summary = state.publish_summary().unwrap();
    let _ = &summary.input;

    let _runtime = CommandRuntime::default();
}

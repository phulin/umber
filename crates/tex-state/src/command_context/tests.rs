use super::CommandLineSource;
use crate::env::AssignmentScope;
use crate::env::banks::IntParam;
use crate::interner::InternerBudget;

fn with_test_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut crate::Universe<crate::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    crate::with_universe(budget, use_universe).expect("fresh universe")
}

fn output<G>(universe: &mut crate::Universe<G>) -> (String, String) {
    let world = universe.world_mut();
    let end = world.effect_pos();
    world.commit_effects(end).expect("commit printed prompt");
    (
        String::from_utf8(world.memory_terminal_output().expect("terminal").to_vec())
            .expect("text terminal"),
        String::from_utf8(world.memory_log_output().expect("log").to_vec()).expect("text log"),
    )
}

#[test]
fn paused_line_prints_an_operation_break_before_character_data() {
    for (newline_char, expected_terminal) in [(-1, "\nA=>"), (65, "\n\n=>")] {
        with_test_universe(|universe| {
            universe
                .assign_int_param(
                    IntParam::NEWLINE_CHAR,
                    newline_char,
                    AssignmentScope::Global,
                )
                .expect("set newlinechar");
            universe
                .world_mut()
                .push_memory_terminal_line("B")
                .expect("terminal response");
            let line = universe
                .command_context()
                .expect("command context")
                .input_ln(CommandLineSource::PausedFileLine { line: "A" });
            assert_eq!(line.as_deref(), Some("B"));
            let (terminal, log) = output(universe);
            assert_eq!(terminal, expected_terminal);
            assert_eq!(log, format!("{expected_terminal}B\n"));
        });
    }
}

#[test]
fn read_target_uses_live_control_sequence_spelling_after_a_print_break() {
    with_test_universe(|universe| {
        universe
            .assign_int_param(IntParam::ESCAPE_CHAR, 33, AssignmentScope::Global)
            .expect("set escapechar");
        universe
            .world_mut()
            .push_memory_terminal_line("")
            .expect("empty terminal response");
        let target = universe
            .command_context()
            .expect("command context")
            .intern_control_sequence("ordinary");
        let line = universe
            .command_context()
            .expect("command context")
            .input_ln(CommandLineSource::ReadTarget { target });
        assert_eq!(line.as_deref(), Some(""));
        let (terminal, log) = output(universe);
        assert_eq!(terminal, "\n!ordinary=");
        assert_eq!(log, "\n!ordinary=\n");
    });
}

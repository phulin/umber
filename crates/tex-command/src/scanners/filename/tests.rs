use tex_state::token::{Catcode, Token};

use crate::{CommandHostCapabilities, CommandState};

#[test]
fn filename_scan_stops_at_space_and_keeps_area_name_and_extension() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let tokens = "dir/job.tex "
            .chars()
            .map(|ch| Token::Char {
                ch,
                cat: if ch == ' ' {
                    Catcode::Space
                } else if ch.is_ascii_alphabetic() {
                    Catcode::Letter
                } else {
                    Catcode::Other
                },
            })
            .collect::<Vec<_>>();
        crate::test_harness::push(&mut command, tokens);
        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = crate::CommandFuelLedger::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let mut context = universe.command_context().expect("command context");
        let mut processor = crate::test_harness::processor(
            &mut command,
            &mut context,
            &mut capabilities,
            &mut fuel,
            &mut diagnostic_effects,
        );
        let mut scalar = crate::ScalarScanFrame::default();
        assert_eq!(
            processor.scan_file_name_into(&mut scalar),
            crate::ScalarScanStatus::Complete
        );
        let scanned = scalar.take_file_name();

        assert_eq!(scanned.packed(), "dir/job.tex");
        assert_eq!(scanned.components.area, "dir/");
        assert_eq!(scanned.components.name, "job");
        assert_eq!(scanned.components.extension, ".tex");
    });
}

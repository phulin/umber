use tex_command::{CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState};
use tex_state::Universe;
use tex_state::input::TracedTokenList;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, Token};

use super::{EngineBoundary, EngineCheckpoint};
use crate::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, ExecutionBudgetCounters, ModeNest,
};

#[test]
fn retained_checkpoint_restores_command_and_mode_token_roots() {
    let mut universe = Universe::new();
    let command_root = universe.intern_token_list_ref(&[Token::Char {
        ch: 'c',
        cat: Catcode::Other,
    }]);
    let u_template = universe.intern_token_list_ref(&[Token::Char {
        ch: 'u',
        cat: Catcode::Other,
    }]);
    let v_template = universe.intern_token_list_ref(&[Token::Char {
        ch: 'v',
        cat: Catcode::Other,
    }]);
    let mut command = CommandState::default();
    command.push_everypar(
        &universe.command_context(),
        TracedTokenList::synthetic(command_root),
    );
    {
        let mut context = universe.command_context();
        drop(command.publish_named_token_list_pushes(&mut context));
    }
    let mut modes = ModeNest::new();
    modes
        .current_list_mutation()
        .set_align_state(AlignState::new(
            AlignmentKind::HAlign,
            AlignmentPackSpec::Natural,
            vec![AlignColumn {
                u_template,
                v_template,
            }],
            Vec::new(),
            tex_state::glue::testing_zero_glue_ref(),
            None,
        ));
    let checkpoint = EngineCheckpoint::capture_checkpoint(
        EngineBoundary::JobStart,
        &command,
        &mut modes,
        &mut universe,
        ExecutionBudgetCounters::default(),
        true,
    )
    .expect("checkpoint captures");

    command = CommandState::default();
    modes = ModeNest::new();
    checkpoint
        .restore_state(&mut command, &mut modes, &mut universe)
        .expect("retained checkpoint restores");

    let mut capabilities = CommandHostCapabilities::default();
    let mut processor = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );
    assert_eq!(
        processor
            .get_next()
            .expect("restored command delivery")
            .expect("restored command token")
            .meaning(),
        Meaning::CharToken {
            ch: 'c',
            cat: Catcode::Other,
        }
    );
    drop(processor);

    let column = &modes
        .current_list()
        .align_state()
        .expect("restored alignment")
        .columns()[0];
    assert_eq!(
        &*universe.tokens(column.u_template.id()),
        &[Token::Char {
            ch: 'u',
            cat: Catcode::Other,
        }]
    );
    assert_eq!(
        &*universe.tokens(column.v_template.id()),
        &[Token::Char {
            ch: 'v',
            cat: Catcode::Other,
        }]
    );
}

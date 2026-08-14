use tex_command::ObservationValue;
use tex_state::Universe;
use tex_state::env::banks::TokParam;
use tex_state::token::{Catcode, Token};

use super::committer::AssignmentCommitter;

fn token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

#[test]
fn global_token_writes_keep_displaced_values_live_through_assignment_trace() {
    let mut stores = Universe::new();
    let displaced_register = stores.intern_token_list_ref(&[token('a')]);
    stores.set_toks_global(0, displaced_register.id());
    drop(displaced_register);
    let replacement_register = stores.intern_token_list_ref(&[token('b')]);

    AssignmentCommitter::new(&mut stores).toks(
        0,
        replacement_register.id(),
        ObservationValue::Integer(0),
        true,
    );
    assert_eq!(stores.tokens(stores.toks(0)).as_ref(), &[token('b')]);

    let parameter = TokParam::EVERY_PAR;
    let displaced_parameter = stores.intern_token_list_ref(&[token('c')]);
    stores.set_tok_param_option_global(parameter, Some(displaced_parameter.id()));
    drop(displaced_parameter);
    let replacement_parameter = stores.intern_token_list_ref(&[token('d')]);

    AssignmentCommitter::new(&mut stores).token_parameter(
        parameter.raw(),
        Some(replacement_parameter.id()),
        ObservationValue::Integer(0),
        "everypar".into(),
        true,
    );
    assert_eq!(
        stores
            .tokens(stores.tok_param_option(parameter).expect("everypar is set"))
            .as_ref(),
        &[token('d')]
    );
}

#[test]
fn local_token_write_undo_is_the_assignment_trace_liveness_negative_control() {
    let mut stores = Universe::new();
    let displaced = stores.intern_token_list_ref(&[token('a')]);
    stores.set_toks_global(0, displaced.id());
    drop(displaced);
    stores.enter_group();
    let replacement = stores.intern_token_list_ref(&[token('b')]);

    AssignmentCommitter::new(&mut stores).toks(
        0,
        replacement.id(),
        ObservationValue::Integer(0),
        false,
    );

    assert_eq!(stores.tokens(stores.toks(0)).as_ref(), &[token('b')]);
    let _ = stores.leave_group();
    assert_eq!(stores.tokens(stores.toks(0)).as_ref(), &[token('a')]);
}

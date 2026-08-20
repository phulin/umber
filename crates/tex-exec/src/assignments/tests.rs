use tex_command::ObservationValue;
use tex_state::Universe;
use tex_state::env::banks::TokParam;
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};

use super::committer::AssignmentCommitter;

fn token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Other,
    }
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    }
}

#[test]
fn global_token_writes_keep_displaced_values_live_through_assignment_trace() {
    let mut stores = Universe::new();
    let displaced_register = stores.intern_token_list_ref(&[token('a')]);
    stores.set_toks_global(0, displaced_register.id());
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

#[test]
fn global_glue_writes_keep_displaced_values_live_through_assignment_trace() {
    let mut stores = Universe::new();
    stores.set_int_param_global(IntParam::TRACING_ASSIGNS, 1);

    let displaced_skip = stores.intern_glue(glue(1));
    stores.set_skip_global(0, displaced_skip);
    AssignmentCommitter::new(&mut stores).skip(0, glue(2), true, false, false, false);
    assert_eq!(stores.glue(stores.skip(0)), glue(2));

    let displaced_muskip = stores.intern_glue(glue(3));
    stores.set_muskip_global(0, displaced_muskip);
    AssignmentCommitter::new(&mut stores).skip(0, glue(4), true, true, false, false);
    assert_eq!(stores.glue(stores.muskip(0)), glue(4));

    let parameter = GlueParam::BASELINE_SKIP;
    let displaced_parameter = stores.intern_glue(glue(5));
    stores.set_glue_param_global(parameter, displaced_parameter);
    AssignmentCommitter::new(&mut stores).glue_parameter(
        parameter.raw(),
        glue(6),
        "baselineskip".into(),
        true,
    );
    assert_eq!(stores.glue(stores.glue_param(parameter)), glue(6));
}

#[test]
fn local_glue_write_undo_is_the_assignment_trace_liveness_negative_control() {
    let mut stores = Universe::new();
    stores.set_int_param_global(IntParam::TRACING_ASSIGNS, 1);
    let displaced = stores.intern_glue(glue(1));
    stores.set_skip_global(0, displaced);
    stores.enter_group();

    AssignmentCommitter::new(&mut stores).skip(0, glue(2), false, false, false, false);

    assert_eq!(stores.glue(stores.skip(0)), glue(2));
    let _ = stores.leave_group();
    assert_eq!(stores.glue(stores.skip(0)), glue(1));
}

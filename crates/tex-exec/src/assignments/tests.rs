use tex_command::ObservationValue;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::TokParam;
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token, TokenWord};
use tex_state::{AssignmentScope, GroupKind};

use super::committer::AssignmentCommitter;

fn token(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Other,
    })
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
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        let displaced_register = stores
            .allocate_token_list(&[token('a')])
            .expect("token list");
        stores
            .assign_token_register(0, Some(displaced_register), AssignmentScope::Global)
            .expect("register");
        let replacement_register = stores
            .allocate_token_list(&[token('b')])
            .expect("token list");

        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).toks(
            0,
            replacement_register,
            ObservationValue::Integer(0),
            true,
        );
        assert_eq!(
            stores.token_list(stores.token_register(0).expect("register").expect("set")),
            &[token('b')]
        );

        let parameter = TokParam::EVERY_PAR;
        let displaced_parameter = stores
            .allocate_token_list(&[token('c')])
            .expect("token list");
        stores
            .assign_token_parameter(
                parameter,
                Some(displaced_parameter),
                AssignmentScope::Global,
            )
            .expect("parameter");
        let replacement_parameter = stores
            .allocate_token_list(&[token('d')])
            .expect("token list");

        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).token_parameter(
            parameter.raw(),
            Some(replacement_parameter),
            ObservationValue::Integer(0),
            "everypar".into(),
            true,
        );
        assert_eq!(
            stores.token_list(
                stores
                    .token_parameter(parameter)
                    .expect("parameter")
                    .expect("everypar is set")
            ),
            &[token('d')]
        );
    });
}

#[test]
fn local_token_write_undo_is_the_assignment_trace_liveness_negative_control() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        let displaced = stores
            .allocate_token_list(&[token('a')])
            .expect("token list");
        stores
            .assign_token_register(0, Some(displaced), AssignmentScope::Global)
            .expect("register");
        stores.begin_group(GroupKind::Simple, 0).expect("group");
        let replacement = stores
            .allocate_token_list(&[token('b')])
            .expect("token list");

        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).toks(
            0,
            replacement,
            ObservationValue::Integer(0),
            false,
        );

        assert_eq!(
            stores.token_list(stores.token_register(0).expect("register").expect("set")),
            &[token('b')]
        );
        stores.end_group(GroupKind::Simple).expect("group");
        assert_eq!(
            stores.token_list(stores.token_register(0).expect("register").expect("set")),
            &[token('a')]
        );
    });
}

#[test]
fn global_glue_writes_keep_displaced_values_live_through_assignment_trace() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        stores
            .assign_int_param(IntParam::TRACING_ASSIGNS, 1, AssignmentScope::Global)
            .expect("parameter");

        let displaced_skip = stores.allocate_glue(glue(1)).expect("glue");
        stores
            .assign_glue_register(0, Some(displaced_skip), AssignmentScope::Global)
            .expect("register");
        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).skip(
            0,
            glue(2),
            true,
            false,
            false,
            false,
        );
        assert_eq!(
            stores.glue(stores.glue_register(0).expect("register").expect("set")),
            glue(2)
        );

        let displaced_muskip = stores.allocate_glue(glue(3)).expect("glue");
        stores
            .assign_mu_glue_register(0, Some(displaced_muskip), AssignmentScope::Global)
            .expect("register");
        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).skip(
            0,
            glue(4),
            true,
            true,
            false,
            false,
        );
        assert_eq!(stores.glue(stores.muskip(0).expect("set")), glue(4));

        let parameter = GlueParam::BASELINE_SKIP;
        let displaced_parameter = stores.allocate_glue(glue(5)).expect("glue");
        stores
            .assign_glue_parameter(
                parameter,
                Some(displaced_parameter),
                AssignmentScope::Global,
            )
            .expect("parameter");
        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).glue_parameter(
            parameter.raw(),
            glue(6),
            "baselineskip".into(),
            true,
        );
        assert_eq!(
            stores.glue(stores.glue_param(parameter).expect("set")),
            glue(6)
        );
    });
}

#[test]
fn local_glue_write_undo_is_the_assignment_trace_liveness_negative_control() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut diagnostic_effects = DiagnosticEffects::new();
        stores
            .assign_int_param(IntParam::TRACING_ASSIGNS, 1, AssignmentScope::Global)
            .expect("parameter");
        let displaced = stores.allocate_glue(glue(1)).expect("glue");
        stores
            .assign_glue_register(0, Some(displaced), AssignmentScope::Global)
            .expect("register");
        stores.begin_group(GroupKind::Simple, 0).expect("group");

        AssignmentCommitter::new(&mut stores, &mut diagnostic_effects).skip(
            0,
            glue(2),
            false,
            false,
            false,
            false,
        );

        assert_eq!(
            stores.glue(stores.glue_register(0).expect("register").expect("set")),
            glue(2)
        );
        stores.end_group(GroupKind::Simple).expect("group");
        assert_eq!(
            stores.glue(stores.glue_register(0).expect("register").expect("set")),
            glue(1)
        );
    });
}

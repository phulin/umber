use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandRestoreError, CommandState,
};
use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{CheckpointRestoreError, EngineBoundary, EngineCheckpoint};
use crate::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, ExecutionBudgetCounters, ModeNest,
};

#[test]
fn retained_checkpoint_restores_command_and_mode_token_roots() {
    crate::test_harness::with_universe(|universe| {
        let command_root = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'c',
                cat: Catcode::Other,
            })])
            .expect("command root");
        let u_template = tex_state::node::NodeTokenList::new([TokenWord::pack(Token::Char {
            ch: 'u',
            cat: Catcode::Other,
        })]);
        let v_template = tex_state::node::NodeTokenList::new([TokenWord::pack(Token::Char {
            ch: 'v',
            cat: Catcode::Other,
        })]);
        let mut command = CommandState::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        command.push_everypar(
            &universe.command_context().expect("command context"),
            command_root,
        );
        command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
        );
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
                tex_state::glue::GlueSpec::ZERO,
                None,
            ));
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
            true,
        )
        .expect("checkpoint captures");

        let later_command_root = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            })])
            .expect("later command root");
        command.push_everypar(
            &universe.command_context().expect("command context"),
            later_command_root,
        );
        let _ = command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
        );
        modes
            .current_list_mutation()
            .take_align_state()
            .expect("alignment root is mutated after capture");
        checkpoint
            .restore_state(&mut command, &mut modes, universe)
            .expect("retained checkpoint restores into its owning timeline");

        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            universe.command_context().expect("command context"),
            CommandHostContext::new(&mut capabilities),
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("restored command delivery")
                .expect("restored command token")
                .meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'c',
                cat: Catcode::Other,
            })
        );
        drop(processor);

        let column = &modes
            .current_list()
            .align_state()
            .expect("restored alignment")
            .columns()[0];
        assert_eq!(
            column.u_template.words(),
            &[TokenWord::pack(Token::Char {
                ch: 'u',
                cat: Catcode::Other,
            })]
        );
        assert_eq!(
            column.v_template.words(),
            &[TokenWord::pack(Token::Char {
                ch: 'v',
                cat: Catcode::Other,
            })]
        );
    });
}

#[test]
fn retained_checkpoint_rejects_a_fresh_command_timeline_before_mutation() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut CommandState::default(),
            &mut ModeNest::new(),
            universe,
            ExecutionBudgetCounters::default(),
            true,
        )
        .expect("checkpoint captures");
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        let mode_hash_before = modes.summary().semantic_fingerprint(universe);

        assert!(matches!(
            checkpoint.restore_state(&mut command, &mut modes, universe),
            Err(CheckpointRestoreError::Command(
                CommandRestoreError::ForeignGeneration
            ))
        ));
        assert_eq!(
            modes.summary().semantic_fingerprint(universe),
            mode_hash_before,
            "foreign-timeline validation must precede mode mutation"
        );
        assert_eq!(
            universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("count"),
            20,
            "foreign-timeline validation must precede runtime mutation"
        );
    });
}

#[test]
fn command_validation_failure_leaves_runtime_and_mode_unchanged() {
    crate::test_harness::with_universe(|universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut CommandState::new(CommandProfile::TEX82),
            &mut ModeNest::new(),
            universe,
            ExecutionBudgetCounters::default(),
            true,
        )
        .expect("checkpoint captures");
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");
        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let mut modes = ModeNest::new();

        assert!(matches!(
            checkpoint.restore_state(&mut command, &mut modes, universe),
            Err(CheckpointRestoreError::Command(_))
        ));
        assert_eq!(command.profile(), CommandProfile::PDFTEX14029);
        assert_eq!(
            universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("count"),
            20,
            "command validation must precede runtime mutation"
        );
    });
}

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

#[test]
fn unsave_observes_classic_sparse_and_aftergroup_at_their_tex_positions() {
    with_test_universe(|universe| {
        use crate::token::{Catcode, OriginId, Token, TracedTokenWord};
        use crate::{GroupKind, GroupRestorationCell, GroupUnsaveEvent};
        let word = |ch| {
            TracedTokenWord::pack(
                Token::Char {
                    ch,
                    cat: Catcode::Other,
                },
                OriginId::UNKNOWN,
            )
        };
        let mut state = universe.command_context().expect("admitted state");
        state.begin_group(GroupKind::Simple, 1).expect("group");
        state
            .assign_count(0, 10, AssignmentScope::Local)
            .expect("classic write");
        state
            .save_aftergroup(word('A'))
            .expect("aftergroup before sparse marker");
        state
            .assign_count(256, 20, AssignmentScope::Local)
            .expect("sparse write");
        state
            .save_aftergroup(word('B'))
            .expect("aftergroup after sparse marker");
        state
            .assign_count(1, 30, AssignmentScope::Local)
            .expect("classic write above marker");
        let mut chronology = Vec::new();
        state
            .end_group_observed(GroupKind::Simple, |state, event| {
                assert_eq!(
                    state.current_group_values(),
                    (0, GroupKind::Simple.etex_code()),
                    "TeX decrements cur_level before popping but keeps cur_group until the boundary"
                );
                match event {
                    GroupUnsaveEvent::Restoration(entry) => {
                        let GroupRestorationCell::Count(index) = entry.cell() else {
                            panic!("count restoration")
                        };
                        assert_eq!(
                            state.count(index),
                            Ok(0),
                            "observer sees the restored value"
                        );
                        chronology.push(format!("count{index}"));
                    }
                    GroupUnsaveEvent::AfterGroup(token) => {
                        chronology.push(format!("{:?}", token.semantic_token()))
                    }
                }
            })
            .expect("unsave");
        assert_eq!(
            chronology,
            [
                "count1".to_owned(),
                format!("{:?}", word('B').semantic_token()),
                "count256".to_owned(),
                format!("{:?}", word('A').semantic_token()),
                "count0".to_owned()
            ]
        );
        assert_eq!(state.execution_group_depth(), 0);
    });
}

#[test]
fn empty_inner_group_preserves_outer_saved_cells_and_aftergroup() {
    use crate::GroupKind;
    with_test_universe(|universe| {
        let mut context = universe.command_context().expect("context");
        context.begin_group(GroupKind::Simple, 10).expect("outer");
        context
            .assign_count(0, 17, AssignmentScope::Local)
            .expect("outer value");
        let after = crate::token::TracedTokenWord::pack(
            crate::token::Token::Char {
                ch: 'A',
                cat: crate::token::Catcode::Letter,
            },
            crate::token::OriginId::UNKNOWN,
        );
        context.save_aftergroup(after).expect("outer aftergroup");
        context
            .begin_group(GroupKind::SemiSimple, 20)
            .expect("empty inner");
        assert!(context.end_group(GroupKind::Simple).is_err());
        assert_eq!(context.execution_group_depth(), 2);
        let inner = context
            .end_group_observed(GroupKind::SemiSimple, |_, _| {
                panic!("an empty group has no restoration or aftergroup events");
            })
            .expect("empty inner closes");
        assert_eq!(inner.entered_line(), 20);
        assert_eq!(context.execution_group_depth(), 1);
        assert_eq!(context.count(0).expect("outer value survives"), 17);
        let mut chronology = Vec::new();
        context
            .end_group_observed(GroupKind::Simple, |_, event| match event {
                crate::GroupUnsaveEvent::AfterGroup(word) => {
                    assert_eq!(word, after);
                    chronology.push("aftergroup");
                }
                crate::GroupUnsaveEvent::Restoration(_) => chronology.push("restore"),
            })
            .expect("outer closes");
        assert_eq!(chronology, ["aftergroup", "restore"]);
        assert_eq!(context.count(0).expect("original value restored"), 0);
        assert_eq!(context.execution_group_depth(), 0);
    });
}

#[test]
fn empty_close_declines_mismatch_saved_values_and_aftergroup_without_mutation() {
    use crate::{GroupKind, GroupUnsaveEvent};
    with_test_universe(|universe| {
        let mut context = universe.command_context().expect("context");
        assert!(
            !context
                .resident_groups()
                .1
                .end_empty_group(GroupKind::Simple)
        );
        context.begin_group(GroupKind::Simple, 7).expect("outer");
        context
            .assign_count(0, 17, AssignmentScope::Local)
            .expect("saved value");
        context
            .begin_group(GroupKind::SemiSimple, 8)
            .expect("empty inner");
        assert!(
            !context
                .resident_groups()
                .1
                .end_empty_group(GroupKind::Simple)
        );
        assert_eq!(context.execution_group_depth(), 2);
        assert!(
            context
                .resident_groups()
                .1
                .end_empty_group(GroupKind::SemiSimple)
        );
        assert_eq!(
            context.current_group_values(),
            (1, GroupKind::Simple.etex_code())
        );
        assert!(
            !context
                .resident_groups()
                .1
                .end_empty_group(GroupKind::Simple)
        );
        assert_eq!(context.count(0).expect("live value"), 17);
        context.end_group(GroupKind::Simple).expect("restore outer");
        assert_eq!(context.count(0).expect("restored value"), 0);

        let after = crate::token::TracedTokenWord::pack(
            crate::token::Token::Char {
                ch: 'A',
                cat: crate::token::Catcode::Letter,
            },
            crate::token::OriginId::UNKNOWN,
        );
        context
            .begin_group(GroupKind::Simple, 9)
            .expect("aftergroup scope");
        context.save_aftergroup(after).expect("save word");
        assert!(
            !context
                .resident_groups()
                .1
                .end_empty_group(GroupKind::Simple)
        );
        let mut delivered = Vec::new();
        context
            .end_group_observed(GroupKind::Simple, |_, event| match event {
                GroupUnsaveEvent::AfterGroup(word) => delivered.push(word),
                _ => panic!("only an aftergroup word was saved"),
            })
            .expect("actual unsave");
        assert_eq!(delivered, [after]);
        assert_eq!(context.current_group_values(), (0, 0));
    });
}

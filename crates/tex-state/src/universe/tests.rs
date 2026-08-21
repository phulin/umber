use super::{UniverseError, with_universe};
use crate::env::AssignmentScope;
use crate::interner::InternerBudget;
use crate::meaning::{Meaning, MeaningWord, ResolvedMeaning};
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use crate::node_arena::NodeArenaError;
use crate::token::Token;
use crate::{GroupKind, ParagraphShapeLine, PenaltyArrayKind};
use tex_arith::{GlueSetRatio, Scaled};

fn budget() -> InternerBudget {
    InternerBudget::new(32, 32, 1024).expect("budget")
}

#[test]
fn command_episode_admits_session_and_generation_once() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("alpha").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("assign");

        let context = universe.command_context().expect("admit episode");
        assert_eq!(context.resolve_symbol(symbol), Ok("alpha"));
        assert_eq!(
            context.meaning(symbol.symbol()),
            ResolvedMeaning::Static(Meaning::Relax)
        );
    })
    .expect("universe allocation");
}

#[test]
fn rollback_never_recycles_an_interned_symbol() {
    with_universe(budget(), |universe| {
        let first = universe.intern("first").expect("intern first");
        let cursor = universe.journal_cursor().expect("cursor");
        let second = universe.intern("second").expect("intern second");
        universe.restore_state(cursor).expect("state rollback");

        assert_eq!(universe.resolve_symbol(first), Ok("first"));
        assert_eq!(universe.resolve_symbol(second), Ok("second"));
        assert_eq!(universe.intern("second"), Ok(second));
    })
    .expect("universe allocation");
}

#[test]
fn whole_session_retirement_rejects_future_admission() {
    with_universe(budget(), |universe| {
        universe.intern("retained").expect("intern");
        let retired = universe.retire().expect("retire");
        assert_eq!(retired.interner_usage().control_sequence_names(), 1);
        assert!(universe.is_retired());
        assert_eq!(
            universe.command_context().err(),
            Some(UniverseError::Retired)
        );
        assert_eq!(universe.intern("late"), Err(UniverseError::Retired));
    })
    .expect("universe allocation");
}

#[test]
fn foreign_session_symbols_are_rejected_before_dense_access() {
    let mut foreign = None;
    with_universe(budget(), |universe| {
        foreign = Some(universe.intern("foreign").expect("intern"));
    })
    .expect("first universe");

    with_universe(budget(), |universe| {
        let local = universe.intern("local").expect("intern local");
        let context = universe.command_context().expect("context");
        assert_eq!(context.resolve_symbol(local), Ok("local"));
        assert!(
            context
                .resolve_symbol(foreign.expect("foreign id"))
                .is_err()
        );
    })
    .expect("second universe");
}

#[test]
fn retained_state_checkpoint_restores_dense_roots_before_arena_suffixes() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = universe.state_checkpoint().expect("checkpoint");
        let rejected = universe.publish_page_nodes(&[Node::Penalty(99)]);
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");

        universe
            .restore_state_checkpoint(&checkpoint)
            .expect("restore checkpoint");

        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .count(0)
                .expect("test fixture is valid"),
            10
        );
        assert_eq!(
            universe
                .page_node_list(rejected)
                .expect_err("invalid test fixture is rejected"),
            NodeArenaError::InvalidList
        );
        assert_eq!(
            universe.retire(),
            Err(UniverseError::State(crate::StateError::GenerationInUse))
        );
        drop(checkpoint);
        universe.retire().expect("last coarse owner released");
    })
    .expect("universe allocation");
}

#[test]
fn malformed_aggregate_restore_does_not_touch_dense_state() {
    with_universe(budget(), |universe| {
        let before_page = universe.page_node_cursor();
        let _ = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let malformed = universe.state_checkpoint().expect("future page cursor");
        universe
            .assign_count(0, 41, AssignmentScope::Global)
            .expect("candidate count");
        universe
            .truncate_page_nodes(before_page)
            .expect("discard page suffix before restore");

        assert_eq!(
            universe.restore_state_checkpoint(&malformed),
            Err(UniverseError::NodeArena(NodeArenaError::CursorBeyondEnd))
        );
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .count(0)
                .expect("test fixture is valid"),
            41,
            "page-cursor rejection must precede dense-state mutation"
        );
    })
    .expect("universe allocation");
}

#[test]
fn admitted_paragraph_shape_is_detached_and_group_restorable() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("context");
        let baseline = [ParagraphShapeLine {
            indent: Scaled::from_raw(10),
            width: Scaled::from_raw(100),
        }];
        context
            .assign_paragraph_shape(&baseline, AssignmentScope::Global)
            .expect("baseline shape");
        context.begin_group(GroupKind::Simple, 1).expect("group");
        let local = [
            ParagraphShapeLine {
                indent: Scaled::from_raw(20),
                width: Scaled::from_raw(200),
            },
            ParagraphShapeLine {
                indent: Scaled::from_raw(30),
                width: Scaled::from_raw(300),
            },
        ];
        context
            .assign_paragraph_shape(&local, AssignmentScope::Local)
            .expect("local shape");

        assert_eq!(context.paragraph_shape(), local);
        assert_eq!(context.paragraph_shape_len(), 2);
        assert_eq!(
            context.paragraph_shape_dimension(3, false),
            Scaled::from_raw(30),
            "lines after the explicit shape repeat its final entry"
        );
        assert_eq!(
            context.paragraph_shape_dimension(3, true),
            Scaled::from_raw(300)
        );
        assert_eq!(
            context.paragraph_shape_dimension(0, true),
            Scaled::from_raw(0)
        );

        context.end_group(GroupKind::Simple).expect("end group");
        assert_eq!(context.paragraph_shape(), baseline);
    })
    .expect("universe allocation");
}

#[test]
fn admitted_penalty_arrays_preserve_etex_projection_and_scope() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("context");
        context.begin_group(GroupKind::Simple, 1).expect("group");
        context
            .assign_penalty_array(
                PenaltyArrayKind::Club,
                &[10, 20, 30],
                AssignmentScope::Local,
            )
            .expect("local penalty array");

        assert_eq!(context.penalty_array(PenaltyArrayKind::Club), [10, 20, 30]);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, -1), 0);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 0), 3);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 2), 20);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 8), 30);

        context.end_group(GroupKind::Simple).expect("end group");
        assert!(context.penalty_array(PenaltyArrayKind::Club).is_empty());
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 0), 0);
    })
    .expect("universe allocation");
}

#[test]
fn admitted_assignment_rendering_never_reopens_the_universe() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("alpha").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("meaning");
        let mut context = universe.command_context().expect("context");
        assert_eq!(
            context.bounded_meaning_text(Token::Cs(symbol.symbol()), 32),
            "\\relax"
        );
        assert_eq!(context.box_assignment_trace_text(None), "void");

        let children = context.publish_page_nodes(Vec::new());
        let root = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: crate::glue::Order::Normal,
            children,
        }))]);
        assert_eq!(
            context.box_assignment_trace_text(Some(root)),
            "\\hbox(0.0+0.0)x0.0"
        );
    })
    .expect("universe allocation");
}

#[test]
fn dropped_shipout_restores_aggregate_roots_before_page_suffix_truncation() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(0, 7, AssignmentScope::Global)
            .expect("baseline count");
        let speculative_root = {
            let mut transaction = universe.begin_shipout();
            transaction
                .assign_count(0, 99, AssignmentScope::Global)
                .expect("speculative count");
            transaction
                .world_mut()
                .write_text(crate::PrintSink::Terminal, "speculative");
            let mut context = transaction.command_context().expect("context");
            let children = context.publish_page_nodes(vec![Node::Penalty(17)]);
            context.append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children,
            })));
            children
        };

        assert_eq!(universe.count(0).expect("count"), 7);
        assert!(universe.page_node_list(speculative_root).is_err());
        assert!(universe.world().effect_records().is_empty());
        assert!(
            universe
                .command_context()
                .expect("context")
                .page_contributions()
                .is_empty()
        );
    })
    .expect("universe allocation");
}

#[test]
fn pure_memo_capability_is_borrowed_and_does_not_keep_runtime_alive() {
    with_universe(budget(), |universe| {
        let runtime = std::sync::Arc::new(std::sync::Mutex::new(crate::PureMemoRuntime::default()));
        universe.attach_pure_memo_capability(&runtime);
        assert!(
            universe
                .with_pure_memo(|_| 41)
                .is_some_and(|value| value == 41)
        );
        drop(runtime);
        assert_eq!(universe.with_pure_memo(|_| 0), None);
    })
    .expect("universe allocation");
}

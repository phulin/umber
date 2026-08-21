use proptest::prelude::*;

use crate::env::AssignmentScope;
use crate::interner::InternerBudget;
use crate::node::Node;

fn budget() -> InternerBudget {
    InternerBudget::new(32, 32, 1024).expect("budget")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn typed_checkpoint_replay_restores_the_exact_semantic_prefix(
        prefix in prop::collection::vec((0_u16..16, any::<i32>()), 0..64),
        suffix in prop::collection::vec((0_u16..16, any::<i32>()), 0..64),
    ) {
        crate::with_universe(budget(), |universe| {
            let mut expected = [0_i32; 16];
            for &(index, value) in &prefix {
                universe
                    .assign_count(index, value, AssignmentScope::Global)
                    .expect("prefix assignment");
                expected[usize::from(index)] = value;
            }

            let checkpoint = universe.state_checkpoint().expect("typed checkpoint");
            let candidate_page = universe.publish_page_nodes(&[Node::Penalty(17)]);
            for &(index, value) in &suffix {
                universe
                    .assign_count(index, value, AssignmentScope::Global)
                    .expect("suffix assignment");
            }

            universe
                .restore_state_checkpoint(&checkpoint)
                .expect("restore typed checkpoint");
            let context = universe.command_context().expect("admit restored state");
            for (index, expected) in expected.into_iter().enumerate() {
                prop_assert_eq!(context.count(index as u16), Ok(expected));
            }
            drop(context);
            prop_assert!(universe.page_node_list(candidate_page).is_err());
            Ok(())
        })
        .expect("fresh universe")?;
    }
}

//! Actual warmed construction/retirement allocation, independent of layout.

use tex_state::glue::Order;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use tex_state::page_node_arena::{BuiltBoxOrigin, PageListId};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::{AssignmentScope, BoxDimension, CommandContext};

fn replace_box<G>(context: &mut CommandContext<'_, G>, with_body: bool) {
    let build = context.begin_page_node_region();
    let children = if with_body {
        context.publish_page_nodes([Node::Penalty(17)])
    } else {
        PageListId::empty()
    };
    let root = context.publish_page_nodes([Node::HList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(43),
        height: Scaled::from_raw(47),
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children,
    }))]);
    context
        .assign_built_page_box(
            2,
            Some(root),
            build,
            AssignmentScope::Global,
            BuiltBoxOrigin::SetBoxConstruction,
        )
        .expect("replace box through its actual durable owner");
}

#[test]
fn warmed_small_box_publication_and_retirement_allocate_no_heap() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut context = universe.command_context().expect("admitted context");
        for with_body in [false, true] {
            for _ in 0..16 {
                replace_box(&mut context, with_body);
            }
            let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                for _ in 0..128 {
                    replace_box(&mut context, with_body);
                }
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            assert_eq!(after.calls - before.calls, 0, "body={with_body}");
            assert_eq!(
                after.requested_bytes - before.requested_bytes,
                0,
                "body={with_body}"
            );
            assert_eq!(
                context.box_dimension(2, BoxDimension::Width),
                Some(Scaled::from_raw(43))
            );
        }
    });
}

use super::*;

use tex_state::env::AssignmentScope;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::interner::InternerBudget;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};

fn budget() -> InternerBudget {
    InternerBudget::new(32, 32, 1024).expect("test interner budget")
}

fn empty_vbox() -> Node {
    Node::VList(BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(0),
        height: Scaled::MAX_DIMEN,
        depth: Scaled::from_raw(0),
        shift: Scaled::from_raw(0),
        box_lr: BoxLr::Normal,
        glue_set: GlueSetRatio::ZERO,
        glue_sign: Sign::Normal,
        glue_order: Order::Normal,
        children: tex_state::node_arena::PageListId::empty(),
    }))
}

fn pending_text<G>(stores: &Universe<G>) -> String {
    stores
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            tex_state::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn completed_page_release_drops_exact_closure_and_scratch() {
    tex_state::with_universe(budget(), |stores| {
        let older = stores.publish_page_nodes(&[Node::Penalty(1)]);
        let child = stores.publish_page_nodes(&[Node::Penalty(2)]);
        let page = Node::VList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: child,
        }));
        let root = stores.publish_page_nodes(&[page]);
        let scratch = stores.page_node_cursor();
        let speculative = stores.publish_page_nodes(&[Node::Penalty(3)]);

        release_published_page(stores, scratch, root);

        assert!(stores.page_node_list(older).is_ok());
        assert!(stores.page_node_list(child).is_err());
        assert!(stores.page_node_list(root).is_err());
        assert!(stores.page_node_list(speculative).is_err());
    })
    .expect("fresh universe");
}

#[test]
fn huge_page_recovery_displays_deleted_box_only_when_not_already_traced() {
    crate::test_harness::with_tex82_universe(|untraced| {
        let root = untraced.publish_page_nodes(&[empty_vbox()]);
        report_huge_page_deleted_box(untraced, root, 0);
        let text = pending_text(untraced);
        assert!(
            text.contains("The following box has been deleted:\n\\vbox(16383.99998+0.0)x0.0"),
            "{text:?}"
        );
    });

    crate::test_harness::with_tex82_universe(|traced| {
        traced
            .assign_int_param(IntParam::TRACING_OUTPUT, 1, AssignmentScope::Global)
            .expect("assign tracingoutput");
        let root = traced.publish_page_nodes(&[empty_vbox()]);
        let tracing_output = traced.int_param(IntParam::TRACING_OUTPUT);
        report_huge_page_deleted_box(traced, root, tracing_output);
        assert_eq!(pending_text(traced), "");
    });
}

#[test]
fn huge_page_deleted_box_display_uses_live_escape_character() {
    tex_state::with_universe(budget(), |stores| {
        stores
            .assign_int_param(
                IntParam::ESCAPE_CHAR,
                i32::from(b'|'),
                AssignmentScope::Global,
            )
            .expect("assign escapechar");
        let root = stores.publish_page_nodes(&[empty_vbox()]);

        report_huge_page_deleted_box(stores, root, 0);

        let text = pending_text(stores);
        assert!(
            text.contains("The following box has been deleted:\n|vbox(16383.99998+0.0)x0.0"),
            "{text:?}"
        );
    })
    .expect("fresh universe");
}

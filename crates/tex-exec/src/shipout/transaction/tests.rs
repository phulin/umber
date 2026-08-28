use super::*;

use tex_state::env::AssignmentScope;
use tex_state::env::banks::IntParam;
use tex_state::glue::Order;
use tex_state::interner::InternerBudget;
use tex_state::node::{BoxLr, BoxNode, BoxNodeFields, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::OriginId;
use tex_state::world::ArtifactSourceRecipe;

struct EmptySourceResolver;

impl crate::output_provenance::ArtifactSourceResolver for EmptySourceResolver {
    fn detach_artifact_source(&self, _origin: OriginId) -> Option<ArtifactSourceRecipe> {
        None
    }
}

#[derive(Default)]
struct IgnoredGeometry;

impl crate::shipout::ShipoutGeometrySink for IgnoredGeometry {
    fn committed_shipout_geometry(&mut self, _geometry: crate::shipout::ShipoutGeometry) {}
}

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
fn completed_page_release_drops_exact_nested_region() {
    tex_state::with_universe(budget(), |stores| {
        let older = crate::test_harness::publish_page_nodes(stores, [Node::Penalty(1)]);
        let region = stores.begin_page_node_region();
        let child = crate::test_harness::publish_page_nodes(stores, [Node::Penalty(2)]);
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
        let root = crate::test_harness::publish_page_nodes(stores, [page]);
        let speculative = crate::test_harness::publish_page_nodes(stores, [Node::Penalty(3)]);

        release_published_page(stores, Some(region));

        assert!(stores.page_node_list(older).is_ok());
        assert!(stores.page_node_list(child).is_err());
        assert!(stores.page_node_list(root).is_err());
        assert!(stores.page_node_list(speculative).is_err());
    })
    .expect("fresh universe");
}

fn assert_aborted_shipout_diagnostic_is_unpublished<G>(stores: &mut Universe<G>) {
    stores
        .world_mut()
        .write_text(tex_state::PrintSink::Terminal, "open terminal line");
    let effect_prefix = stores.world().effect_records().to_vec();
    let partial_lines = stores.world().printable_lines_are_open();
    let history = stores.world().error_channel().history();

    let mut write = |_: &mut Universe<G>,
                     _: &mut DiagnosticEffects,
                     _: tex_state::PrintSink,
                     _: tex_state::ShipoutTokenSource<G>|
     -> Result<crate::shipout::ExpandedWrite, ExecError> {
        unreachable!("the negative control never begins traversal")
    };
    let mut replay = |_: &mut Universe<G>,
                      _: &mut DiagnosticEffects,
                      _: crate::shipout::ReplayTextKind,
                      _: tex_state::ShipoutTokenSource<G>|
     -> Result<crate::shipout::ExpandedReplayText, ExecError> {
        unreachable!("the negative control never begins traversal")
    };
    let mut geometry = IgnoredGeometry;
    let mut transaction: crate::shipout::ShipoutTransaction<'_, G> =
        crate::shipout::ShipoutTransaction::new(
            &mut write,
            &mut replay,
            &EmptySourceResolver,
            tex_state::ProvenanceDemand::DIAGNOSTICS,
            0,
            &mut geometry,
        );
    {
        let command = stores.command_context().expect("diagnostic admission");
        let mut diagnostic = command.begin_diagnostic(&mut transaction.diagnostic_effects);
        diagnostic.print_nl("staged shipout diagnostic");
        diagnostic.end(false);
    }
    assert_eq!(transaction.diagnostic_effects.len(), 1);

    // A failed staging result drops the transaction without extracting its
    // operation-local collector for outer publication.
    drop(transaction);

    assert_eq!(stores.world().effect_records(), effect_prefix);
    assert_eq!(stores.world().printable_lines_are_open(), partial_lines);
    assert_eq!(stores.world().error_channel().history(), history);
}

#[test]
fn aborted_shipout_transaction_publishes_no_diagnostic_program() {
    crate::test_harness::with_nonstop_plain_universe(|stores| {
        assert_aborted_shipout_diagnostic_is_unpublished(stores);
    });
}

#[test]
fn huge_page_recovery_displays_deleted_box_only_when_not_already_traced() {
    crate::test_harness::with_nonstop_tex82_universe(|untraced| {
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        let root = direct::ShipoutRoot::Page(empty_vbox());
        report_huge_page_deleted_box(untraced, &mut diagnostic_effects, &root, 0);
        untraced
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        let text = pending_text(untraced);
        assert!(
            text.contains("The following box has been deleted:\n\\vbox(16383.99998+0.0)x0.0"),
            "{text:?}"
        );
    });

    crate::test_harness::with_nonstop_tex82_universe(|traced| {
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        traced
            .assign_int_param(IntParam::TRACING_OUTPUT, 1, AssignmentScope::Global)
            .expect("assign tracingoutput");
        let root = direct::ShipoutRoot::Page(empty_vbox());
        let tracing_output = traced.int_param(IntParam::TRACING_OUTPUT);
        report_huge_page_deleted_box(traced, &mut diagnostic_effects, &root, tracing_output);
        traced
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);
        assert_eq!(pending_text(traced), "");
    });
}

#[test]
fn huge_page_deleted_box_display_uses_live_escape_character() {
    tex_state::with_universe(budget(), |stores| {
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        stores
            .assign_int_param(
                IntParam::ESCAPE_CHAR,
                i32::from(b'|'),
                AssignmentScope::Global,
            )
            .expect("assign escapechar");
        let root = direct::ShipoutRoot::Page(empty_vbox());

        report_huge_page_deleted_box(stores, &mut diagnostic_effects, &root, 0);
        stores
            .world_mut()
            .publish_diagnostic_effects(diagnostic_effects);

        let text = pending_text(stores);
        assert!(
            text.contains("The following box has been deleted:\n|vbox(16383.99998+0.0)x0.0"),
            "{text:?}"
        );
    })
    .expect("fresh universe");
}

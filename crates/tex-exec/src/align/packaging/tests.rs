use super::*;
use tex_state::glue::{GlueSpec, Order};
use tex_state::node::{GlueKind, UnsetKind};
use tex_state::scaled::Scaled;

fn sp(raw: i32) -> Scaled {
    Scaled::from_raw(raw * Scaled::UNITY)
}

#[test]
fn package_unset_cell_records_natural_extent_and_glue_orders() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut geometry = crate::geometry::IgnorePackGeometry;
        let diagnostic_context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
        let fil = GlueSpec {
            width: sp(3),
            stretch: sp(7),
            stretch_order: Order::Fil,
            shrink: sp(4),
            shrink_order: Order::Fill,
        };
        let fill = GlueSpec {
            width: sp(2),
            stretch: sp(9),
            stretch_order: Order::Fill,
            shrink: sp(6),
            shrink_order: Order::Fil,
        };
        let children = stores.publish_page_nodes(vec![
            Node::Rule {
                width: Some(sp(5)),
                height: Some(sp(2)),
                depth: Some(sp(1)),
            },
            Node::Glue {
                spec: fil,
                kind: GlueKind::Normal,
                leader: None,
            },
            Node::Glue {
                spec: fill,
                kind: GlueKind::Normal,
                leader: None,
            },
        ]);
        let children_ref = children.clone();

        for (alignment, kind) in [
            (AlignmentKind::HAlign, UnsetKind::HBox),
            (AlignmentKind::VAlign, UnsetKind::VBox),
        ] {
            let expected = tex_typeset::measure_unset(
                &crate::typeset_context::TypesetContext::new(&stores),
                &children_ref,
                kind,
            );
            let Node::Unset(cell) = make_unset_node(
                &mut stores,
                &mut tex_state::diagnostic::DiagnosticEffects::new(),
                &mut geometry,
                &diagnostic_context,
                children.clone(),
                kind,
                3,
                UnsetPackContext::Row,
            )
            .expect("a three-column span is far inside TeX82 \u{a7}110's max_quarterword") else {
                panic!("alignment cell must remain unset until fin_align");
            };

            assert_eq!(cell.kind, cell_unset_kind(alignment));
            assert_eq!(cell.span_count, 2);
            assert_eq!(cell.width, expected.width);
            assert_eq!(cell.height, expected.height);
            assert_eq!(cell.depth, expected.depth);
            assert_eq!(cell.stretch, expected.stretch);
            assert_eq!(cell.stretch_order, expected.stretch_order);
            assert_eq!(cell.shrink, expected.shrink);
            assert_eq!(cell.shrink_order, expected.shrink_order);
            assert_eq!(cell.stretch_order, Order::Fill);
            assert_eq!(cell.stretch, sp(9));
            assert_eq!(cell.shrink_order, Order::Fill);
            assert_eq!(cell.shrink, sp(4));
        }
    });
}

#[test]
fn span_record_256_limit_and_merge_fields() {
    // TeX82 §§797--798 store the zero-based span count in a quarterword. The
    // largest legal cell therefore spans 256 columns; one more succumbs with
    // the canonical confusion, without losing the packed metric fields.
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state is admitted");
        let mut geometry = crate::geometry::IgnorePackGeometry;
        let diagnostic_context = crate::pack_report::ExecutionDiagnosticContext::source_free("");
        let children = stores.publish_page_nodes(vec![Node::Rule {
            width: Some(sp(9)),
            height: Some(sp(2)),
            depth: Some(sp(1)),
        }]);
        let Node::Unset(limit) = make_unset_node(
            &mut stores,
            &mut tex_state::diagnostic::DiagnosticEffects::new(),
            &mut geometry,
            &diagnostic_context,
            children.clone(),
            UnsetKind::HBox,
            256,
            UnsetPackContext::Cell,
        )
        .expect("§798 permits max_quarterword span steps") else {
            panic!("legal span must remain unset");
        };
        assert_eq!(limit.span_count, 255);
        assert_eq!(
            (limit.width, limit.height, limit.depth),
            (sp(9), sp(2), sp(1))
        );

        let error = make_unset_node(
            &mut stores,
            &mut tex_state::diagnostic::DiagnosticEffects::new(),
            &mut geometry,
            &diagnostic_context,
            children,
            UnsetKind::HBox,
            257,
            UnsetPackContext::Cell,
        )
        .expect_err("the 256th delimiter exceeds §798's quarterword field");
        assert!(matches!(
            error,
            ExecError::Fatal(fatal) if fatal == FatalError::confusion("256 spans")
        ));
    });
}

use super::*;
use crate::input::SourceId;
use crate::interner::InternerBudget;
use crate::provenance::{RelatedLocationRole, SourceOrigin, SyntheticOrigin};
use crate::universe::with_universe;

fn budget() -> InternerBudget {
    InternerBudget::new(128, 256, 4096).expect("test fixture is valid")
}

#[test]
fn source_presentation_is_built_only_after_explicit_cold_demand() {
    with_universe(budget(), |universe| {
        universe
            .world_mut()
            .set_memory_file("utf8.tex", "α\r\nbéta".as_bytes().to_vec())
            .expect("test fixture is valid");
        let content = universe
            .world_mut()
            .read_file("utf8.tex")
            .expect("test fixture is valid");
        let source =
            SourceOrigin::new(SourceId::new(7), 5, 99, 99).with_input_record(content.record());
        let coordinate = universe
            .allocate_provenance(OriginRecord::Source(source))
            .expect("test fixture is valid");

        let resolver = ProvenanceResolver::new(universe, ColdProvenanceDemand::Diagnostic);
        assert_eq!(resolver.demand(), ColdProvenanceDemand::Diagnostic);
        let location = resolver
            .resolve_coordinate(coordinate)
            .expect("test fixture is valid");
        assert_eq!(location.path, "utf8.tex");
        assert_eq!((location.line, location.column), (2, 2));
        assert_eq!(location.excerpt, "béta");
    })
    .expect("test fixture is valid");
}

#[test]
fn detached_presentation_survives_without_live_coordinates() {
    let detached = with_universe(budget(), |universe| {
        universe
            .world_mut()
            .set_memory_file("main.tex", b"first\nsecond".to_vec())
            .expect("test fixture is valid");
        let content = universe
            .world_mut()
            .read_file("main.tex")
            .expect("test fixture is valid");
        let primary = universe
            .allocate_provenance(OriginRecord::Source(
                SourceOrigin::new(SourceId::new(1), 6, 0, 0).with_input_record(content.record()),
            ))
            .expect("test fixture is valid");
        let related_id = universe
            .allocate_provenance(OriginRecord::Synthetic(SyntheticOrigin::new(
                SyntheticOriginKind::Engine,
            )))
            .expect("test fixture is valid");
        let related = [DiagnosticProvenanceCoordinate {
            role: Some(RelatedLocationRole::RecoveryFrontier),
            coordinate: related_id,
        }];
        let request = DiagnosticProvenanceRequest {
            primary: Some(primary),
            related: &related,
            expansion: &[],
        };
        ProvenanceResolver::new(universe, ColdProvenanceDemand::Diagnostic)
            .detach_diagnostic(&request)
    })
    .expect("test fixture is valid");

    assert_eq!(
        detached
            .primary
            .as_ref()
            .expect("test fixture is valid")
            .excerpt,
        "second"
    );
    assert_eq!(detached.related[0].summary, "engine origin");
    let rendered = render_detached_diagnostic("boom", &detached);
    assert!(rendered.contains("main.tex:2:1"));
    assert!(rendered.contains("recovery begins here: engine origin"));
}

#[test]
fn generated_source_recipes_are_handle_free_owned_values() {
    with_universe(budget(), |universe| {
        let recipe = DetachedGeneratedSourceSpan {
            logical_path: "editor/root.tex".into(),
            bytes: b"left\tright".to_vec(),
            start: 5,
            end: 6,
        };
        let location = ProvenanceResolver::new(universe, ColdProvenanceDemand::RenderedSource)
            .resolve_generated(&recipe)
            .expect("test fixture is valid");
        assert_eq!(location.path, "editor/root.tex");
        assert_eq!(location.column, 9);
    })
    .expect("test fixture is valid");
}

#[test]
fn expansion_rows_are_bounded_by_the_explicit_resolver_budget() {
    with_universe(budget(), |universe| {
        let one = universe
            .allocate_provenance(OriginRecord::Synthetic(SyntheticOrigin::new(
                SyntheticOriginKind::Primitive,
            )))
            .expect("test fixture is valid");
        let two = universe
            .allocate_provenance(OriginRecord::Synthetic(SyntheticOrigin::new(
                SyntheticOriginKind::Format,
            )))
            .expect("test fixture is valid");
        let expansion = [one, two];
        let request = DiagnosticProvenanceRequest {
            primary: None,
            related: &[],
            expansion: &expansion,
        };
        let detached =
            ProvenanceResolver::with_trace_depth(universe, ColdProvenanceDemand::Diagnostic, 1)
                .detach_diagnostic(&request);
        assert_eq!(detached.expansion, ["primitive origin"]);
    })
    .expect("test fixture is valid");
}

#[test]
fn invalid_generated_ranges_degrade_to_unknown_without_live_lookup() {
    with_universe(budget(), |universe| {
        let recipe = DetachedGeneratedSourceSpan {
            logical_path: "memory".into(),
            bytes: b"x".to_vec(),
            start: 2,
            end: 3,
        };
        assert!(
            ProvenanceResolver::new(universe, ColdProvenanceDemand::RenderedSource)
                .resolve_generated(&recipe)
                .is_none()
        );
    })
    .expect("test fixture is valid");
}

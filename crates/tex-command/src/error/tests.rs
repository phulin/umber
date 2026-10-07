use super::*;

#[test]
fn source_annotation_retains_failure_owner_and_first_origin() {
    crate::test_harness::with_universe(|universe| {
        let mut context = universe.command_context().expect("context");
        let source = tex_state::input::SourceId::new(0);
        context
            .register_source(
                source,
                tex_state::source_map::SourceDescriptor::generated(std::sync::Arc::from(
                    &b"ab"[..],
                )),
            )
            .expect("source registration");
        let first = context.source_token_origin(source, 0, 1);
        let second = context.source_token_origin(source, 1, 2);
        assert_ne!(first, OriginId::UNKNOWN);
        assert_ne!(first, second);
        let failure = CommandFailure::new(CommandErrorKind::MacroPrefixMismatch);
        let owner = std::ptr::from_ref(failure.as_ref());
        let failure = failure
            .at_origin_unless_resource(first)
            .at_origin_unless_resource(second);
        assert_eq!(std::ptr::from_ref(failure.as_ref()), owner);
        assert_eq!(failure.origin, Some(first));
        assert_eq!(failure.kind, CommandErrorKind::MacroPrefixMismatch);

        let missing = CommandFailure::missing_input("a.tex".into(), "a".into());
        let owner = std::ptr::from_ref(missing.as_ref());
        let missing = missing.at_origin_unless_resource(first);
        assert_eq!(std::ptr::from_ref(missing.as_ref()), owner);
        assert_eq!(missing.origin, None);
        assert!(missing.is_resource_suspension());
    });
}

#[test]
fn dimension_diagnostic_conversion_and_units_are_canonical_values() {
    assert_eq!(
        DimensionDiagnostic::from(DimensionError::TooLarge),
        DimensionDiagnostic::TooLarge
    );
    assert_eq!(
        DimensionDiagnostic::IllegalUnit {
            inserted: InsertedUnit::Mu,
        }
        .to_string(),
        "Illegal unit of measure (mu inserted)"
    );
    assert_eq!(
        DimensionDiagnostic::from(PrepareMagDiagnostic::IncompatibleMagnification {
            attempted: 1200,
            retained: 1000,
        }),
        DimensionDiagnostic::IncompatibleMagnification {
            attempted: 1200,
            retained: 1000,
        }
    );
}

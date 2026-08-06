use super::*;

const TEX_ETEX: PrimitiveProfiles = PrimitiveProfiles::TEX82.union(PrimitiveProfiles::ETEX26);
const PRIMARY: &[PrimitiveSpelling] = &[PrimitiveSpelling {
    name: "sample",
    profiles: TEX_ETEX,
    kind: SpellingKind::Canonical,
    installation: InstallationPolicy::BOTH,
}];
const WEB: &[WebIdentity] = &[WebIdentity {
    profiles: TEX_ETEX,
    command: "sample_cmd",
    operand: Some(7),
    collision_group: None,
}];

const fn descriptor(operand: PrimitiveOperand) -> PrimitiveDescriptor {
    PrimitiveDescriptor {
        operand,
        spellings: PRIMARY,
        profiles: TEX_ETEX,
        expansion: ExpansionClass::Unexpandable,
        web: WEB,
        prefix: PrefixAdmissibility::Forbidden,
        parameter: None,
        documentation: DocumentationFamily::Tex82,
    }
}

#[test]
fn schema_names_every_current_meaning_domain_without_enum_order() {
    let domains = [
        PrimitiveOperandDomain::Expandable,
        PrimitiveOperandDomain::Unexpandable,
        PrimitiveOperandDomain::InternalInteger,
        PrimitiveOperandDomain::IntegerParameter,
        PrimitiveOperandDomain::DimensionParameter,
        PrimitiveOperandDomain::GlueParameter,
        PrimitiveOperandDomain::MathGlueParameter,
        PrimitiveOperandDomain::TokenParameter,
        PrimitiveOperandDomain::PageDimension,
        PrimitiveOperandDomain::PageInteger,
        PrimitiveOperandDomain::FontSelector,
        PrimitiveOperandDomain::Relax,
        PrimitiveOperandDomain::FrozenMacro,
        PrimitiveOperandDomain::InaccessibleCommand,
    ];
    let unique = domains
        .into_iter()
        .map(|domain| PrimitiveOperand::new(domain, 0))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), domains.len());
}

#[test]
fn valid_alias_parameter_and_frozen_exception_catalogue_passes() {
    const PARAMETER_SPELLINGS: &[PrimitiveSpelling] = &[
        PrimitiveSpelling {
            name: "counting",
            profiles: PrimitiveProfiles::TEX82,
            kind: SpellingKind::Canonical,
            installation: InstallationPolicy::BOTH,
        },
        PrimitiveSpelling {
            name: "countingalias",
            profiles: PrimitiveProfiles::TEX82,
            kind: SpellingKind::Alias,
            installation: InstallationPolicy::BOTH,
        },
    ];
    const DESCRIPTORS: &[PrimitiveDescriptor] = &[
        PrimitiveDescriptor {
            operand: PrimitiveOperand::new(PrimitiveOperandDomain::IntegerParameter, 3),
            spellings: PARAMETER_SPELLINGS,
            profiles: PrimitiveProfiles::TEX82,
            expansion: ExpansionClass::InternalQuantity,
            web: &[],
            prefix: PrefixAdmissibility::Admissible,
            parameter: Some((
                ParameterCell {
                    class: ParameterBankClass::Integer,
                    index: 3,
                },
                ParameterDefault::Integer(10_000),
            )),
            documentation: DocumentationFamily::Tex82,
        },
        PrimitiveDescriptor {
            operand: PrimitiveOperand::new(PrimitiveOperandDomain::FrozenMacro, 0),
            spellings: &[],
            profiles: PrimitiveProfiles::TEX82,
            expansion: ExpansionClass::FrozenMacro,
            web: &[],
            prefix: PrefixAdmissibility::Forbidden,
            parameter: None,
            documentation: DocumentationFamily::FrozenPrivate,
        },
    ];
    assert_eq!(PrimitiveCatalogue::new(DESCRIPTORS).validate(), []);
}

#[test]
fn validation_reports_every_duplicate_namespace() {
    const CELL: ParameterCell = ParameterCell {
        class: ParameterBankClass::Integer,
        index: 9,
    };
    const FIRST: PrimitiveDescriptor = PrimitiveDescriptor {
        parameter: Some((CELL, ParameterDefault::Integer(0))),
        ..descriptor(PrimitiveOperand::new(
            PrimitiveOperandDomain::IntegerParameter,
            9,
        ))
    };
    const SECOND: PrimitiveDescriptor = PrimitiveDescriptor {
        parameter: Some((CELL, ParameterDefault::Integer(0))),
        ..descriptor(PrimitiveOperand::new(
            PrimitiveOperandDomain::IntegerParameter,
            9,
        ))
    };
    const DESCRIPTORS: &[PrimitiveDescriptor] = &[FIRST, SECOND];
    let errors = PrimitiveCatalogue::new(DESCRIPTORS).validate();
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, CatalogueValidationError::DuplicateOperand { .. }))
    );
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, CatalogueValidationError::DuplicateSpelling { .. }))
    );
    assert!(
        errors
            .iter()
            .any(|error| matches!(error, CatalogueValidationError::DuplicateWebIdentity { .. }))
    );
    assert!(errors.iter().any(|error| matches!(
        error,
        CatalogueValidationError::DuplicateParameterCell { .. }
    )));
}

#[test]
fn explicitly_grouped_web_identity_collisions_are_not_duplicates() {
    const SHARED_WEB: &[WebIdentity] = &[WebIdentity {
        profiles: PrimitiveProfiles::TEX82,
        command: "start_par",
        operand: Some(0),
        collision_group: Some("start_without_indent"),
    }];
    const FIRST_SPELLING: &[PrimitiveSpelling] = &[PrimitiveSpelling {
        name: "noindent",
        profiles: PrimitiveProfiles::TEX82,
        kind: SpellingKind::Canonical,
        installation: InstallationPolicy::BOTH,
    }];
    const SECOND_SPELLING: &[PrimitiveSpelling] = &[PrimitiveSpelling {
        name: "quitvmode",
        profiles: PrimitiveProfiles::TEX82,
        kind: SpellingKind::Canonical,
        installation: InstallationPolicy::BOTH,
    }];
    const DESCRIPTORS: &[PrimitiveDescriptor] = &[
        PrimitiveDescriptor {
            spellings: FIRST_SPELLING,
            profiles: PrimitiveProfiles::TEX82,
            web: SHARED_WEB,
            ..descriptor(PrimitiveOperand::new(
                PrimitiveOperandDomain::Unexpandable,
                1,
            ))
        },
        PrimitiveDescriptor {
            spellings: SECOND_SPELLING,
            profiles: PrimitiveProfiles::TEX82,
            web: SHARED_WEB,
            ..descriptor(PrimitiveOperand::new(
                PrimitiveOperandDomain::Unexpandable,
                2,
            ))
        },
    ];
    assert_eq!(PrimitiveCatalogue::new(DESCRIPTORS).validate(), []);
}

#[test]
fn validation_reports_all_structural_failures_together() {
    const BAD_SPELLING: &[PrimitiveSpelling] = &[PrimitiveSpelling {
        name: "",
        profiles: PrimitiveProfiles::PDFTEX14029,
        kind: SpellingKind::Alias,
        installation: InstallationPolicy::BOTH,
    }];
    const BAD_WEB: &[WebIdentity] = &[WebIdentity {
        profiles: PrimitiveProfiles::PDFTEX14029,
        command: "bad",
        operand: None,
        collision_group: None,
    }];
    const BAD: &[PrimitiveDescriptor] = &[PrimitiveDescriptor {
        operand: PrimitiveOperand::new(PrimitiveOperandDomain::IntegerParameter, 4),
        spellings: BAD_SPELLING,
        profiles: PrimitiveProfiles::TEX82,
        expansion: ExpansionClass::InternalQuantity,
        web: BAD_WEB,
        prefix: PrefixAdmissibility::Admissible,
        parameter: None,
        documentation: DocumentationFamily::Tex82,
    }];
    let errors = PrimitiveCatalogue::new(BAD).validate();
    for expected in [
        "EmptySpelling",
        "SpellingOutsideProfiles",
        "WebIdentityOutsideProfiles",
        "MissingCanonicalSpelling",
        "MissingParameter",
    ] {
        assert!(
            errors
                .iter()
                .any(|error| format!("{error:?}").starts_with(expected)),
            "missing {expected}: {errors:?}"
        );
    }
}

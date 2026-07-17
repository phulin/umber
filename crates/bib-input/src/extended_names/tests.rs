use super::*;
use crate::{NameHashScope, classic_name_hash};
use bib_model::NameList;

#[test]
fn parses_pinned_extended_name_parts_and_attributes() {
    let parsed = parse_extended_name(
        "sortingnamekeytemplatename=test, family=Smith, given=Bill, useprefix=true, id=custom",
        ExtendedNameOptions::default(),
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let name = parsed.name.expect("valid extended name");
    assert_eq!(part_value(name.family()), Some("Smith"));
    assert_eq!(part_value(name.given()), Some("Bill"));
    assert_eq!(name.use_prefix(), Some(true));
    assert_eq!(name.sorting_name_key_template(), Some("test"));
    assert_eq!(name.hash_id(), Some("custom"));
    assert_eq!(
        classic_name_hash(&NameList::new([name.clone()], false), NameHashScope::Full),
        "8b9035807842a4e4dbe009f3f1478127"
    );
    assert_eq!(
        name.assignments()
            .map(NameAssignment::key)
            .collect::<Vec<_>>(),
        [
            "sortingnamekeytemplatename",
            "family",
            "given",
            "useprefix",
            "id"
        ]
    );
}

#[test]
fn matches_names_x_parts_initials_protection_and_hash_normalization() {
    for (source, family, given, prefix, suffix, family_i, given_i) in [
        (
            "given=John,family=Doe",
            "Doe",
            "John",
            None,
            None,
            vec!["D"],
            vec!["J"],
        ),
        (
            "family=Doe, suffix=Jr, given=John, given-i=J",
            "Doe",
            "John",
            None,
            Some("Jr"),
            vec!["D"],
            vec!["J"],
        ),
        (
            "prefix=von, family=Berlichingen zu Hornberg, given=Johann Gottfried",
            "Berlichingen zu~Hornberg",
            "Johann~Gottfried",
            Some("von"),
            None,
            vec!["B", "z", "H"],
            vec!["J", "G"],
        ),
        (
            "given={Jean Charles Gabriel}, prefix=de la, family={Vallée Poussin}",
            "Vallée Poussin",
            "Jean Charles Gabriel",
            Some("de~la"),
            None,
            vec!["V"],
            vec!["J"],
        ),
        (
            "given=J. C. G., prefix=de la, family=Vallée Poussin",
            "Vallée~Poussin",
            "J.~C.~G.",
            Some("de~la"),
            None,
            vec!["V", "P"],
            vec!["J", "C", "G"],
        ),
        (
            "given=Jean Charles Gabriel de la Vallée, given-i=JCGdV, family=Poussin",
            "Poussin",
            "Jean Charles Gabriel de la~Vallée",
            None,
            None,
            vec!["P"],
            vec!["J", "C", "G", "d", "V"],
        ),
        (
            "given=E. S., family=El-Mallah",
            "El-Mallah",
            "E.~S.",
            None,
            None,
            vec!["E-M"],
            vec!["E", "S"],
        ),
    ] {
        let parsed = parse_extended_name(source, ExtendedNameOptions::default());
        assert!(
            parsed.diagnostics.is_empty(),
            "{source}: {:?}",
            parsed.diagnostics
        );
        let name = parsed.name.expect("valid");
        assert_eq!(part_value(name.family()), Some(family), "{source}");
        assert_eq!(part_value(name.given()), Some(given), "{source}");
        assert_eq!(part_value(name.prefix()), prefix, "{source}");
        assert_eq!(part_value(name.suffix()), suffix, "{source}");
        assert_eq!(part_initials(name.family()), family_i, "{source}");
        assert_eq!(part_initials(name.given()), given_i, "{source}");
    }

    let composed = parse_extended_name(
        "family=Smith, given=Šomeone",
        ExtendedNameOptions::default(),
    )
    .name
    .expect("valid");
    let decomposed = parse_extended_name(
        "family=Smith, given=S\u{30c}omeone",
        ExtendedNameOptions::default(),
    )
    .name
    .expect("valid");
    assert_eq!(
        classic_name_hash(&NameList::new([composed], false), NameHashScope::Full),
        classic_name_hash(&NameList::new([decomposed], false), NameHashScope::Full)
    );
}

#[test]
fn supports_csv_quotes_explicit_initials_and_key_aliases() {
    let parsed = parse_extended_name(
        "\"last={Robert and Sons, Inc.}\", last-i={Ro}",
        ExtendedNameOptions {
            aliases: &[("last", "family"), ("last-i", "family-i")],
            ..ExtendedNameOptions::default()
        },
    );
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let name = parsed.name.expect("valid");
    assert_eq!(part_value(name.family()), Some("Robert and Sons, Inc."));
    assert_eq!(part_initials(name.family()), ["Ro"]);
}

#[test]
fn malformed_records_are_diagnostic_and_bounded() {
    let options = ExtendedNameOptions {
        limits: ExtendedNameLimits {
            max_record_bytes: 80,
            max_fields: 2,
            max_field_bytes: 24,
            max_nesting: 2,
            max_work: 80,
            max_diagnostics: 2,
        },
        ..ExtendedNameOptions::default()
    };
    let malformed = parse_extended_name("family={unclosed, nonsense, useprefix=maybe", options);
    assert!(malformed.name.is_none());
    assert!(malformed.diagnostics.len() <= 2);

    let overlong = parse_extended_name(&"x".repeat(81), options);
    assert_eq!(overlong.diagnostics.len(), 1);
    assert_eq!(
        overlong.diagnostics[0].kind,
        ExtendedNameDiagnosticKind::Limit
    );
}

fn part_value(part: Option<&NamePartValue>) -> Option<&str> {
    part.map(|part| part.value().as_str())
}

fn part_initials(part: Option<&NamePartValue>) -> Vec<&str> {
    part.map_or_else(Vec::new, |part| part.initials().collect())
}

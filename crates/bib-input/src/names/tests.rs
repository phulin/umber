use super::*;

struct Expected<'a> {
    source: &'a str,
    family: Option<&'a str>,
    given: Option<&'a str>,
    prefix: Option<&'a str>,
    suffix: Option<&'a str>,
    family_initials: &'a [&'a str],
    given_initials: &'a [&'a str],
}

#[test]
fn parses_pinned_classic_name_cases_exactly() {
    let cases = [
        Expected {
            source: "John Doe",
            family: Some("Doe"),
            given: Some("John"),
            prefix: None,
            suffix: None,
            family_initials: &["D"],
            given_initials: &["J"],
        },
        Expected {
            source: "Doe, Jr, John",
            family: Some("Doe"),
            given: Some("John"),
            prefix: None,
            suffix: Some("Jr"),
            family_initials: &["D"],
            given_initials: &["J"],
        },
        Expected {
            source: "von Berlichingen zu Hornberg, Johann Gottfried",
            family: Some("Berlichingen zu~Hornberg"),
            given: Some("Johann~Gottfried"),
            prefix: Some("von"),
            suffix: None,
            family_initials: &["B", "z", "H"],
            given_initials: &["J", "G"],
        },
        Expected {
            source: "{Robert and Sons, Inc.}",
            family: Some("Robert and Sons, Inc."),
            given: None,
            prefix: None,
            suffix: None,
            family_initials: &["R"],
            given_initials: &[],
        },
        Expected {
            source: "al-Ṣāliḥ, ʿAbdallāh",
            family: Some("al-Ṣāliḥ"),
            given: Some("ʿAbdallāh"),
            prefix: None,
            suffix: None,
            family_initials: &["Ṣ"],
            given_initials: &["A"],
        },
        Expected {
            source: "al- Hakim, Tawfik",
            family: Some("Hakim"),
            given: Some("Tawfik"),
            prefix: Some("al-"),
            suffix: None,
            family_initials: &["H"],
            given_initials: &["T"],
        },
        Expected {
            source: "Jean Charles Gabriel de la Vallée Poussin",
            family: Some("Vallée~Poussin"),
            given: Some("Jean Charles~Gabriel"),
            prefix: Some("de~la"),
            suffix: None,
            family_initials: &["V", "P"],
            given_initials: &["J", "C", "G"],
        },
        Expected {
            source: "{Jean Charles Gabriel} de la Vallée Poussin",
            family: Some("Vallée~Poussin"),
            given: Some("Jean Charles Gabriel"),
            prefix: Some("de~la"),
            suffix: None,
            family_initials: &["V", "P"],
            given_initials: &["J"],
        },
        Expected {
            source: "Jean Charles Gabriel {de la} Vallée Poussin",
            family: Some("Poussin"),
            given: Some("Jean Charles Gabriel {de la}~Vallée"),
            prefix: None,
            suffix: None,
            family_initials: &["P"],
            given_initials: &["J", "C", "G", "d", "V"],
        },
        Expected {
            source: "Jean Charles Gabriel de la {Vallée Poussin}",
            family: Some("Vallée Poussin"),
            given: Some("Jean Charles~Gabriel"),
            prefix: Some("de~la"),
            suffix: None,
            family_initials: &["V"],
            given_initials: &["J", "C", "G"],
        },
        Expected {
            source: "{Jean Charles Gabriel} de la {Vallée Poussin}",
            family: Some("Vallée Poussin"),
            given: Some("Jean Charles Gabriel"),
            prefix: Some("de~la"),
            suffix: None,
            family_initials: &["V"],
            given_initials: &["J"],
        },
        Expected {
            source: "Jean Charles Gabriel Poussin",
            family: Some("Poussin"),
            given: Some("Jean Charles~Gabriel"),
            prefix: None,
            suffix: None,
            family_initials: &["P"],
            given_initials: &["J", "C", "G"],
        },
        Expected {
            source: "Jean Charles {Poussin Lecoq}",
            family: Some("Poussin Lecoq"),
            given: Some("Jean~Charles"),
            prefix: None,
            suffix: None,
            family_initials: &["P"],
            given_initials: &["J", "C"],
        },
        Expected {
            source: "J. C. G. de la Vallée Poussin",
            family: Some("Vallée~Poussin"),
            given: Some("J.~C.~G."),
            prefix: Some("de~la"),
            suffix: None,
            family_initials: &["V", "P"],
            given_initials: &["J", "C", "G"],
        },
        Expected {
            source: "E. S. El-{M}allah",
            family: Some("El-{M}allah"),
            given: Some("E.~S."),
            prefix: None,
            suffix: None,
            family_initials: &["E-M"],
            given_initials: &["E", "S"],
        },
        Expected {
            source: "E. S. {K}ent-{B}oswell",
            family: Some("{K}ent-{B}oswell"),
            given: Some("E.~S."),
            prefix: None,
            suffix: None,
            family_initials: &["K-B"],
            given_initials: &["E", "S"],
        },
        Expected {
            source: "Other, A.~N.",
            family: Some("Other"),
            given: Some("A.~N."),
            prefix: None,
            suffix: None,
            family_initials: &["O"],
            given_initials: &["A", "N"],
        },
        Expected {
            source: "{{{British National Corpus}}}",
            family: Some("British National Corpus"),
            given: None,
            prefix: None,
            suffix: None,
            family_initials: &["B"],
            given_initials: &[],
        },
        Expected {
            source: "Vázques{ de }Parga, Luis",
            family: Some("Vázques{ de }Parga"),
            given: Some("Luis"),
            prefix: None,
            suffix: None,
            family_initials: &["V"],
            given_initials: &["L"],
        },
    ];

    for expected in cases {
        let name = parse_classic_name(expected.source, ClassicNameLimits::default())
            .unwrap_or_else(|error| panic!("{}: {error:?}", expected.source));
        assert_eq!(
            part(name.family()),
            expected.family,
            "{} family",
            expected.source
        );
        assert_eq!(
            part(name.given()),
            expected.given,
            "{} given",
            expected.source
        );
        assert_eq!(
            part(name.prefix()),
            expected.prefix,
            "{} prefix",
            expected.source
        );
        assert_eq!(
            part(name.suffix()),
            expected.suffix,
            "{} suffix",
            expected.source
        );
        assert_eq!(
            initials_of(name.family()),
            expected.family_initials,
            "{} family initials",
            expected.source
        );
        assert_eq!(
            initials_of(name.given()),
            expected.given_initials,
            "{} given initials",
            expected.source
        );
        assert_eq!(name.source(), Some(expected.source));
    }
}

#[test]
fn round_trips_classic_source_structure() {
    for (source, expected) in [
        ("John Doe", "Doe, John"),
        ("John van der Doe", "van der Doe, John"),
        ("Doe, Jr, John", "Doe, Jr, John"),
        ("von Doe, Jr, John", "von Doe, Jr, John"),
        ("John Alan Doe", "Doe, John Alan"),
        ("{Robert and Sons, Inc.}", "{Robert and Sons, Inc.}"),
        (
            "Jean Charles Gabriel de la {Vallée Poussin}",
            "de la {Vallée Poussin}, Jean Charles Gabriel",
        ),
        ("E. S. {K}ent-{B}oswell", "{K}ent-{B}oswell, E. S."),
    ] {
        let name = parse_classic_name(source, ClassicNameLimits::default()).expect("valid name");
        assert_eq!(name.to_bibtex(), expected, "{source}");
    }
}

#[test]
fn preserves_order_aliases_others_unicode_and_hashes() {
    let parsed = parse_classic_name_list(
        "Alfred Adler und Šomeone Smith und andere",
        ClassicNameOptions {
            separators: &["und"],
            others: &["andere"],
            limits: ClassicNameLimits::default(),
        },
    );
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.names.len(), 2);
    assert!(parsed.names.has_others());
    assert_eq!(
        parsed.names.iter().next().and_then(Name::source),
        Some("Alfred Adler")
    );
    assert_eq!(
        parsed.names.iter().nth(1).and_then(Name::source),
        Some("Šomeone Smith")
    );

    let adler = NameList::new(
        [parse_classic_name("Alfred Adler", ClassicNameLimits::default()).expect("valid")],
        false,
    );
    assert_eq!(
        classic_name_hash(&adler, NameHashScope::Full),
        "72287a68c1714cb1b9f4ab9e03a88b96"
    );
    let composed =
        parse_classic_name("Šomeone Smith", ClassicNameLimits::default()).expect("valid");
    let decomposed =
        parse_classic_name("S\u{30c}omeone Smith", ClassicNameLimits::default()).expect("valid");
    assert_eq!(
        classic_name_hash(&NameList::new([composed], false), NameHashScope::Full),
        classic_name_hash(&NameList::new([decomposed], false), NameHashScope::Full)
    );
}

#[test]
fn malformed_input_and_work_are_bounded() {
    let limits = ClassicNameLimits {
        max_names: 2,
        max_name_bytes: 16,
        max_nesting: 2,
        max_work: 64,
        max_diagnostics: 2,
    };
    let malformed = parse_classic_name_list(
        "Smith, Jr, Bill, Lee and and {unclosed",
        ClassicNameOptions {
            separators: &["and"],
            others: &["others"],
            limits,
        },
    );
    assert!(malformed.names.is_empty());
    assert_eq!(malformed.diagnostics.len(), 1);
    assert_eq!(
        malformed.diagnostics[0].kind,
        ClassicNameDiagnosticKind::UnbalancedBraces
    );

    let overlong = parse_classic_name_list(
        &"A".repeat(65),
        ClassicNameOptions {
            separators: &["and"],
            others: &["others"],
            limits,
        },
    );
    assert_eq!(overlong.diagnostics.len(), 1);
    assert_eq!(
        overlong.diagnostics[0].kind,
        ClassicNameDiagnosticKind::Limit
    );
}

fn part(value: Option<&NamePartValue>) -> Option<&str> {
    value.map(|part| part.value().as_str())
}

fn initials_of(value: Option<&NamePartValue>) -> Vec<&str> {
    value.map_or_else(Vec::new, |part| part.initials().collect())
}

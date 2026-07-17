use umber_vfs::{FileOrigin, LayerKind, LayeredFileStorage, VirtualFile, VirtualPath};

use super::*;

const OPTIONS_BCF: &[u8] =
    include_bytes!("../../../tests/corpus/bib/upstream-2.22/tdata/options.bcf");
const GENERAL_BCF: &[u8] =
    include_bytes!("../../../tests/corpus/bib/upstream-2.22/tdata/general.bcf");
const CONFIG: &[u8] =
    include_bytes!("../../../tests/corpus/bib/upstream-2.22/tdata/biber-test.conf");
const BIBLATEX_XML: &[u8] =
    include_bytes!("../../../tests/corpus/bib/upstream-2.22/tdata/biblatexml.bltxml");

#[test]
fn parses_control_options_templates_model_and_sections() {
    let control = parse_control_bytes(OPTIONS_BCF, XmlLimits::default()).expect("valid BCF 3.11");
    assert_eq!(control.version, CONTROL_VERSION);
    assert_eq!(control.biblatex_version, "3.21");
    assert_eq!(
        single(control.resolve_option(OptionComponent::Processor, "mincrossrefs", None)),
        "88"
    );
    assert_eq!(
        single(control.resolve_option(OptionComponent::Biblatex, "uniquename", None)),
        "init"
    );
    assert_eq!(
        single(control.resolve_option(OptionComponent::Biblatex, "useprefix", Some("book"))),
        "1"
    );
    let names = control.resolve_option(OptionComponent::Biblatex, "labelnamespec", Some("book"));
    assert_eq!(multiple(names), ["author", "editor"]);
    assert!(!control.templates.is_empty());
    assert!(!control.data_model.entry_types.is_empty());
    assert_eq!(
        control.sections.first().map(|section| section.number),
        Some(0)
    );
}

#[test]
fn parses_configuration_and_resolves_precedence() {
    let config = parse_config_bytes(CONFIG, XmlLimits::default()).expect("valid configuration");
    assert_eq!(
        config.value("sortlocale"),
        Some(&ConfigValue::Scalar("testlocale".into()))
    );
    assert!(
        matches!(config.value("sourcemap"), Some(ConfigValue::Tree(values)) if values.len() > 20)
    );

    let mut resolved = ResolvedConfiguration::default();
    resolved
        .push(
            ConfigurationLayer::CompiledDefaults,
            [("decodecharsset".into(), ConfigValue::Scalar("base".into()))],
        )
        .expect("defaults");
    resolved
        .push(
            ConfigurationLayer::UserConfiguration,
            config
                .values()
                .map(|(key, value)| (key.to_owned(), value.clone())),
        )
        .expect("user config");
    resolved
        .push(
            ConfigurationLayer::Command,
            [("mincrossrefs".into(), ConfigValue::Scalar("7".into()))],
        )
        .expect("command");
    resolved
        .push(
            ConfigurationLayer::ControlFile,
            [("sortcase".into(), ConfigValue::Scalar("0".into()))],
        )
        .expect("control");
    assert_eq!(
        resolved.resolve("mincrossrefs"),
        Some(&ConfigValue::Scalar("7".into()))
    );
    assert_eq!(
        resolved.resolve("sortcase"),
        Some(&ConfigValue::Scalar("0".into()))
    );
    assert_eq!(
        resolved.resolve("decodecharsset"),
        Some(&ConfigValue::Scalar("base".into()))
    );
}

#[test]
fn parses_typed_biblatexml_and_aliases() {
    let data =
        parse_biblatexml_bytes(BIBLATEX_XML, XmlLimits::default()).expect("valid BibLaTeXML");
    assert_eq!(data.canonical_id("bltx1a1"), Some("bltx1"));
    assert_eq!(data.canonical_id("bltx1a2"), Some("bltx1"));
    let entry = data.entry("bltx1").expect("entry");
    assert_eq!(entry.entry_type, "book");
    assert_eq!(
        entry.options.get("useprefix").map(String::as_str),
        Some("false")
    );
    assert!(
        matches!(entry.fields.get("author"), Some(XmlFieldValue::Names { values, attributes }) if values.len() == 3 && attributes.get("useprefix").map(String::as_str) == Some("true"))
    );
    assert!(
        matches!(entry.fields.get("pages"), Some(XmlFieldValue::Range(ranges)) if ranges.len() == 2)
    );
    assert_eq!(entry.annotations.len(), 9);
}

#[test]
fn exposes_structured_classic_names_from_bibtex_fields() {
    let source = parse_bibtex_bytes(
        br#"@book{x, author={Alfred Adler und Steven Secondauthor und andere}}"#,
        BibTexOptions::default(),
    );
    let author = source
        .entry("x")
        .and_then(|entry| entry.field("author"))
        .expect("author field");
    let parsed = author
        .classic_names(ClassicNameOptions {
            separators: &["und"],
            others: &["andere"],
            limits: ClassicNameLimits::default(),
        })
        .expect("name field");
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.names.len(), 2);
    assert!(parsed.names.has_others());
    assert_eq!(
        parsed
            .names
            .iter()
            .map(|name| name.source().expect("source"))
            .collect::<Vec<_>>(),
        ["Alfred Adler", "Steven Secondauthor"]
    );
}

#[test]
fn rejects_namespace_version_doctype_and_limits() {
    let wrong_namespace =
        br#"<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="wrong"/>"#;
    assert!(matches!(
        validate_control_bytes(wrong_namespace, XmlLimits::default()),
        Err(ControlError::Namespace { .. })
    ));
    let wrong_version = br#"<bcf:controlfile version="3.10" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex"/>"#;
    assert!(matches!(
        validate_control_bytes(wrong_version, XmlLimits::default()),
        Err(ControlError::Version { .. })
    ));
    assert!(matches!(
        validate_control_bytes(b"<!DOCTYPE x><x/>", XmlLimits::default()),
        Err(ControlError::Xml(XmlError::ForbiddenDoctype))
    ));
    assert!(matches!(
        validate_config_bytes(b"<config>&external;</config>", XmlLimits::default()),
        Err(ConfigError::Xml(XmlError::Malformed(_)))
    ));
    let limits = XmlLimits {
        max_depth: 1,
        ..XmlLimits::default()
    };
    assert!(matches!(
        validate_control_bytes(GENERAL_BCF, limits),
        Err(ControlError::Xml(XmlError::Limit {
            kind: "nesting",
            ..
        }))
    ));
}

#[test]
fn expands_vfs_includes_and_rejects_cycles() {
    let mut storage = LayeredFileStorage::new();
    insert(&mut storage, "main.xml", br#"<config xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="part.xml"/></config>"#);
    insert(&mut storage, "part.xml", b"<sortlocale>en_GB</sortlocale>");
    let config = parse_config(
        &storage.snapshot(),
        &VirtualPath::user("main.xml").expect("path"),
        XmlLimits::default(),
    )
    .expect("included config");
    assert_eq!(
        config.value("sortlocale"),
        Some(&ConfigValue::Scalar("en_GB".into()))
    );

    let mut cyclic = LayeredFileStorage::new();
    insert(&mut cyclic, "a.xml", br#"<config xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="b.xml"/></config>"#);
    insert(
        &mut cyclic,
        "b.xml",
        br#"<part xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="a.xml"/></part>"#,
    );
    assert!(matches!(
        parse_config(
            &cyclic.snapshot(),
            &VirtualPath::user("a.xml").expect("path"),
            XmlLimits::default()
        ),
        Err(ConfigError::Xml(XmlError::IncludeCycle(_)))
    ));
}

fn single(value: Option<&ControlOptionValue>) -> &str {
    match value {
        Some(ControlOptionValue::Single(value)) => &value.content,
        _ => panic!("single option expected"),
    }
}

fn multiple(value: Option<&ControlOptionValue>) -> Vec<&str> {
    match value {
        Some(ControlOptionValue::Multiple(values)) => {
            values.iter().map(|value| value.content.as_str()).collect()
        }
        _ => panic!("multiple option expected"),
    }
}

fn insert(storage: &mut LayeredFileStorage, path: &str, bytes: &[u8]) {
    let path = VirtualPath::user(path).expect("path");
    storage
        .insert(
            LayerKind::User,
            VirtualFile::new(path, bytes.to_vec(), FileOrigin::User),
        )
        .expect("insert");
}

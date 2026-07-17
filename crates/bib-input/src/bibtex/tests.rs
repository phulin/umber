use umber_vfs::{FileOrigin, LayerKind, LayeredFileStorage, VirtualFile};

use super::*;

#[test]
fn parses_macros_concatenation_nested_values_names_and_preamble() {
    let source = parse_bibtex_bytes(
        br#"@string{who = "Ada" # " " # {Lovelace}}
            @preamble{"prefix " # who}
            @article{Key, author = who # " and {The Team}", month = mar,
              title = {A {Nested} Title}, year = 1843}"#,
        BibTexOptions::default(),
    );
    assert!(
        source.diagnostics().is_empty(),
        "{:?}",
        source.diagnostics()
    );
    assert_eq!(source.preambles()[0].value(), "prefix Ada Lovelace");
    let entry = source.entry("key").expect("parsed entry");
    assert_eq!(entry.field("month").expect("month").value(), "3");
    assert_eq!(
        entry.field("title").expect("title").value(),
        "A {Nested} Title"
    );
    let names = entry
        .field("author")
        .expect("author")
        .raw_names()
        .expect("raw names");
    assert_eq!(
        names.iter().map(RawName::as_str).collect::<Vec<_>>(),
        ["Ada Lovelace", "{The Team}"]
    );
}

#[test]
fn diagnoses_collisions_and_recovers_at_the_next_entry() {
    let source = parse_bibtex_bytes(
        br#"@book{Same, title={one}} @book{same, title={two}}
            @broken{x title={lost}} @misc{after, note="ok"}"#,
        BibTexOptions::default(),
    );
    assert_eq!(source.entries().len(), 2);
    assert_eq!(source.entries()[1].key(), "after");
    assert!(
        source
            .diagnostics()
            .iter()
            .any(|d| d.kind == BibTexDiagnosticKind::CaseCollision)
    );
    assert!(
        source
            .diagnostics()
            .iter()
            .any(|d| d.kind == BibTexDiagnosticKind::Syntax)
    );
}

#[test]
fn enforces_nesting_and_work_limits() {
    let options = BibTexOptions {
        limits: BibTexLimits {
            max_nesting: 2,
            ..BibTexLimits::default()
        },
        ..BibTexOptions::default()
    };
    let source = parse_bibtex_bytes(b"@book{x,title={{{too deep}}}}", options);
    assert!(
        source
            .diagnostics()
            .iter()
            .any(|d| d.kind == BibTexDiagnosticKind::Limit)
    );

    let options = BibTexOptions {
        limits: BibTexLimits {
            max_work: 8,
            ..BibTexLimits::default()
        },
        ..BibTexOptions::default()
    };
    let source = parse_bibtex_bytes(b"lots of text without a record", options);
    assert!(
        source
            .diagnostics()
            .iter()
            .any(|d| d.kind == BibTexDiagnosticKind::Limit)
    );
}

#[test]
fn cache_uses_content_and_semantic_options_not_path() {
    let bytes = b"@misc{x,title={cached}}".to_vec();
    let mut storage = LayeredFileStorage::new();
    for path in ["one.bib", "two.bib"] {
        let path = VirtualPath::user(path).expect("valid path");
        storage
            .insert(
                LayerKind::User,
                VirtualFile::new(path, bytes.clone(), FileOrigin::User),
            )
            .expect("insert fixture");
    }
    let snapshot = storage.snapshot();
    let mut cache = BibTexCache::default();
    let first = cache
        .parse(
            &snapshot,
            &VirtualPath::user("one.bib").expect("valid path"),
            BibTexOptions::default(),
        )
        .expect("parse first");
    let second = cache
        .parse(
            &snapshot,
            &VirtualPath::user("two.bib").expect("valid path"),
            BibTexOptions::default(),
        )
        .expect("parse second");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.len(), 1);
}

#[test]
fn parses_owned_upstream_datasources() {
    let aliases = parse_bibtex_bytes(
        include_bytes!("../../../../tests/corpus/bib/upstream-2.22/tdata/bibtex-aliases.bib"),
        BibTexOptions::default(),
    );
    assert!(
        aliases.diagnostics().is_empty(),
        "{:?}",
        aliases.diagnostics()
    );
    assert_eq!(aliases.entries().len(), 8);
    assert_eq!(
        aliases
            .entry("alias4")
            .expect("alias4")
            .field("participant")
            .expect("participant")
            .value(),
        "Sam Smith"
    );

    let examples = parse_bibtex_bytes(
        include_bytes!("../../../../tests/corpus/bib/upstream-2.22/tdata/examples.bib"),
        BibTexOptions::default(),
    );
    assert!(examples.entries().len() > 100);
    assert_eq!(
        examples
            .entry("shore")
            .expect("shore")
            .field("month")
            .expect("month")
            .value(),
        "3"
    );
}

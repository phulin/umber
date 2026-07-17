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
fn raw_database_preserves_record_order_parts_duplicates_and_locations() {
    let raw = parse_raw_bibtex_bytes(
        br#"@comment{keep \LaTeX syntax}
            @string{who = "Ada" # family}
            @preamble{"prefix " # who}
            @book{Key, title = {A {Nested} \OE uvre}, title = "duplicate", author = who}
            @book{key, note = {case collision}}"#,
        BibTexOptions::default(),
    );
    assert_eq!(raw.records().len(), 5);
    let [
        RawBibRecord::Comment(comment),
        RawBibRecord::String(mac),
        RawBibRecord::Preamble(preamble),
        RawBibRecord::Entry(entry),
        RawBibRecord::Entry(collision),
    ] = raw.records()
    else {
        panic!("raw records retain every recognized record")
    };
    assert_eq!(comment.value().source(), "keep \\LaTeX syntax");
    assert_eq!(comment.value().control_sequences()[0].source(), "\\LaTeX");
    assert_eq!(mac.name().source(), "who");
    assert!(matches!(
        mac.value().parts(),
        [RawBibValuePart::Quoted(_), RawBibValuePart::Macro(_)]
    ));
    assert!(matches!(
        preamble.value().parts(),
        [RawBibValuePart::Quoted(_), RawBibValuePart::Macro(_)]
    ));
    assert_eq!(entry.key().source(), "Key");
    assert_eq!(entry.key().folded(), "key");
    assert_eq!(entry.fields().len(), 3);
    let RawBibValuePart::Braced(title) = &entry.fields()[0].value().parts()[0] else {
        panic!("title remains braced")
    };
    assert_eq!(title.source(), "A {Nested} \\OE uvre");
    assert_eq!(title.control_sequences()[0].source(), "\\OE");
    assert_eq!(collision.key().source(), "key");
    assert!(entry.location().byte_start() < collision.location().byte_start());
    assert_eq!(entry.location().line(), 4);
    assert!(
        raw.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind == BibTexDiagnosticKind::DuplicateField)
    );
    assert!(
        raw.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind == BibTexDiagnosticKind::CaseCollision)
    );

    let biber = BibTexSource::from_raw(&raw);
    assert_eq!(biber.entries().len(), 1);
    assert_eq!(
        biber.entries()[0].field("title").expect("title").value(),
        "A {Nested} \\OE uvre"
    );
    assert_eq!(raw.classic().records().len(), raw.records().len());
}

#[test]
fn raw_recovery_is_retained_without_discarding_the_next_record() {
    let raw = parse_raw_bibtex_bytes(
        b"@broken{x title={lost}\n@misc{after, note={ok}}",
        BibTexOptions::default(),
    );
    assert!(
        raw.records()
            .iter()
            .any(|record| matches!(record, RawBibRecord::Recovery(_)))
    );
    assert!(raw.records().iter().any(
        |record| matches!(record, RawBibRecord::Entry(entry) if entry.key().source() == "after")
    ));
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
fn ignores_percent_comment_records_and_recovers_at_line_records() {
    let source = parse_bibtex_bytes(
        b"% @book{fake, title={not data}}\n\
          @broken{x, title={unterminated\n\
          @misc{after, note={ok}}",
        BibTexOptions::default(),
    );
    assert!(source.entry("fake").is_none());
    assert_eq!(
        source
            .entry("after")
            .expect("recovered entry")
            .field("note")
            .expect("note")
            .value(),
        "ok"
    );
    assert!(
        source
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind == BibTexDiagnosticKind::Syntax)
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

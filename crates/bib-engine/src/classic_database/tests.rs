use bib_bst::{CompileLimits, compile};
use bib_input::{BibTexOptions, parse_raw_bibtex_bytes};
use umber_vfs::{FileContentId, VirtualPath};

use super::{
    ClassicDatabaseCache, ClassicDatabaseDiagnosticKind, ClassicDatabaseSource,
    prepare_classic_database,
};
use crate::{ClassicControl, ClassicDatabaseOptions};

fn control(citations: &[&str]) -> ClassicControl {
    // Test-only construction stays in this owning module, keeping the public
    // control value immutable and AUX-owned.
    ClassicControl::for_read_test(citations)
}

fn prepared(source: &[u8], citations: &[&str]) -> super::ClassicDatabase {
    let compiled = compile(
        b"ENTRY { title year } { } { } READ",
        CompileLimits::default(),
    );
    let style = compiled.program().expect("style");
    let path = VirtualPath::user("refs.bib").expect("path");
    let raw = parse_raw_bibtex_bytes(source, BibTexOptions::default());
    prepare_classic_database(
        &control(citations),
        style,
        &[ClassicDatabaseSource::new(
            &path,
            FileContentId::for_bytes(source),
            &raw,
        )],
        &ClassicDatabaseOptions::default(),
    )
}

#[test]
fn read_projects_declared_fields_and_preserves_citation_order() {
    let database = prepared(
        br#"@article{b, title = "B", year = jan, ignored = "no"}
            @book{a, title = "A"}"#,
        &["a", "b", "a"],
    );
    let compiled = compile(
        b"ENTRY { title year } { } { } READ",
        CompileLimits::default(),
    );
    let style = compiled.program().expect("style");
    let title = style.declarations().lookup("title").expect("title");
    let year = style.declarations().lookup("year").expect("year");
    let entries = database.entries().collect::<Vec<_>>();
    assert_eq!(
        entries.iter().map(|entry| entry.key()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(entries[0].field(title), Some("A"));
    assert_eq!(entries[0].field(year), None);
    assert_eq!(entries[1].field(year), Some("1"));
}

#[test]
fn wildcard_preamble_duplicates_and_crossref_inheritance_are_vm_visible() {
    let database = prepared(
        br#"@string{org = "Umber"}
            @preamble{"P" # org}
            @book{parent, title = org}
            @inproceedings{one, crossref = "parent", year = "1"}
            @inproceedings{two, crossref = "parent"}
            @book{PARENT, title = "ignored"}"#,
        &["*"],
    );
    let compiled = compile(
        b"ENTRY { title year } { } { } READ",
        CompileLimits::default(),
    );
    let style = compiled.program().expect("style");
    let title = style.declarations().lookup("title").expect("title");
    let entries = database.entries().collect::<Vec<_>>();
    assert_eq!(database.preamble(), "PUmber");
    assert_eq!(
        entries.iter().map(|entry| entry.key()).collect::<Vec<_>>(),
        ["parent", "one", "two"]
    );
    assert_eq!(entries[1].field(title), Some("Umber"));
    assert!(
        database
            .diagnostics()
            .any(|diagnostic| diagnostic.kind() == ClassicDatabaseDiagnosticKind::DuplicateEntry)
    );
}

#[test]
fn cache_key_changes_for_schema_and_read_options() {
    let source = b"@book{one, title = \"One\"}";
    let raw = parse_raw_bibtex_bytes(source, BibTexOptions::default());
    let path = VirtualPath::user("refs.bib").expect("path");
    let source = [ClassicDatabaseSource::new(
        &path,
        FileContentId::for_bytes(source),
        &raw,
    )];
    let control = control(&["one"]);
    let compiled_title = compile(b"ENTRY { title } { } { } READ", CompileLimits::default());
    let title = compiled_title.program().expect("style");
    let compiled_empty = compile(b"ENTRY { } { } { } READ", CompileLimits::default());
    let empty = compiled_empty.program().expect("style");
    let mut cache = ClassicDatabaseCache::default();
    let first = cache.prepare(&control, title, &source, &ClassicDatabaseOptions::default());
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &cache.prepare(&control, title, &source, &ClassicDatabaseOptions::default())
    ));
    cache.prepare(&control, empty, &source, &ClassicDatabaseOptions::default());
    cache.prepare(
        &control,
        title,
        &source,
        &ClassicDatabaseOptions::default().with_min_crossrefs(1),
    );
    let macro_one = compile(
        b"ENTRY { title } { } { } MACRO { greeting } { \"one\" } READ",
        CompileLimits::default(),
    );
    let macro_two = compile(
        b"ENTRY { title } { } { } MACRO { greeting } { \"two\" } READ",
        CompileLimits::default(),
    );
    cache.prepare(
        &control,
        macro_one.program().expect("style"),
        &source,
        &ClassicDatabaseOptions::default(),
    );
    cache.prepare(
        &control,
        macro_two.program().expect("style"),
        &source,
        &ClassicDatabaseOptions::default(),
    );
    assert_eq!(cache.len(), 5);
}

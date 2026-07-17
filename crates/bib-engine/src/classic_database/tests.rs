use bib_bst::{ClassicStringPool, CompileLimits, StringPoolLimits, compile};
use bib_input::{BibTexOptions, parse_raw_bibtex_bytes};
use umber_vfs::{FileContentId, VirtualPath};

use super::{
    ClassicDatabaseCache, ClassicDatabaseDiagnosticKind, ClassicDatabaseSource,
    prepare_classic_database,
};
use crate::{ClassicControl, ClassicDatabaseLimits, ClassicDatabaseOptions};

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
        br#"@article{b, title = "B", year = "1", ignored = "no"}
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
fn style_macros_are_visible_when_expanding_raw_entry_values() {
    let source = b"@misc{one, month = jan}";
    let raw = parse_raw_bibtex_bytes(source, BibTexOptions::default());
    let style = compile(
        b"ENTRY { month } { } { } MACRO { jan } { \"Jan.\" } READ",
        CompileLimits::default(),
    );
    let style = style.program().expect("style");
    let month = style.declarations().lookup("month").expect("month");
    let path = VirtualPath::user("refs.bib").expect("path");
    let database = prepare_classic_database(
        &control(&["one"]),
        style,
        &[ClassicDatabaseSource::new(
            &path,
            FileContentId::for_bytes(source),
            &raw,
        )],
        &ClassicDatabaseOptions::default(),
    );
    assert_eq!(
        database.entries().next().expect("entry").field(month),
        Some("Jan.")
    );
}

#[test]
fn read_collapses_literal_whitespace_without_losing_macro_boundaries() {
    let database = prepared(
        br#"@string{prefix = "ACM "}
            @book{one, title = prefix # {Symposium
                on Computing}}"#,
        &["one"],
    );
    let compiled = compile(
        b"ENTRY { title year } { } { } READ",
        CompileLimits::default(),
    );
    let title = compiled
        .program()
        .expect("style")
        .declarations()
        .lookup("title")
        .expect("title");
    assert_eq!(
        database.entries().next().expect("entry").field(title),
        Some("ACM Symposium on Computing")
    );
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

#[test]
fn cache_is_byte_weighted_and_revalidates_read_limits_per_job() {
    let source = b"@book{one, title = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}\n@book{two, title = \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}";
    let raw = parse_raw_bibtex_bytes(source, BibTexOptions::default());
    let path = VirtualPath::user("refs.bib").expect("path");
    let source = [ClassicDatabaseSource::new(
        &path,
        FileContentId::for_bytes(source),
        &raw,
    )];
    let all_control = control(&["*"]);
    let compiled = compile(b"ENTRY { title } { } { } READ", CompileLimits::default());
    let style = compiled.program().expect("style");
    let mut cache = ClassicDatabaseCache::new(32, 4_096);

    let permissive = cache.prepare(
        &all_control,
        style,
        &source,
        &ClassicDatabaseOptions::default(),
    );
    assert_eq!(permissive.entries().len(), 2);
    assert!(cache.retained_bytes() <= 4_096);

    let restricted = ClassicDatabaseOptions::default().with_limits(ClassicDatabaseLimits {
        entries: 1,
        ..ClassicDatabaseLimits::default()
    });
    let restrictive = cache.prepare(&all_control, style, &source, &restricted);
    assert!(!std::sync::Arc::ptr_eq(&permissive, &restrictive));
    assert_eq!(restrictive.entries().len(), 1);
    assert!(
        restrictive
            .diagnostics()
            .any(|diagnostic| diagnostic.kind() == ClassicDatabaseDiagnosticKind::Limit)
    );

    for suffix in 0..16 {
        let bytes = format!("@book{{entry{suffix}, title = \"{}\"}}", "x".repeat(64));
        let raw = parse_raw_bibtex_bytes(bytes.as_bytes(), BibTexOptions::default());
        let path = VirtualPath::user(&format!("refs-{suffix}.bib")).expect("path");
        let sources = [ClassicDatabaseSource::new(
            &path,
            FileContentId::for_bytes(bytes.as_bytes()),
            &raw,
        )];
        let control = control(&["*"]);
        cache.prepare(
            &control,
            style,
            &sources,
            &ClassicDatabaseOptions::default(),
        );
        assert!(cache.retained_bytes() <= 4_096, "job {suffix}");
    }
    assert!(cache.retained_bytes() > 0);
    assert!(
        cache.len() < 16,
        "byte budget must evict maximum-charge jobs"
    );
}

#[test]
fn raw_read_pool_trace_keeps_macros_preambles_and_selected_field_values() {
    let source =
        b"@string{abbr = \"Macro\"}\n@preamble{\"Prelude\"}\n@book{one, title = \"Title\"}";
    let database = prepared(source, &["one"]);
    let mut pool = ClassicStringPool::new(StringPoolLimits::unlimited());
    database.apply_pool_trace(&mut pool);
    assert_eq!(pool.usage().strings(), 4);
    assert_eq!(pool.usage().characters(), 21);
}

#[test]
fn whole_database_read_trace_owns_discovered_keys_not_the_aux_wildcard() {
    let database = prepared(b"@book{one, title = \"Title\"}", &["*"]);
    let mut pool = ClassicStringPool::new(StringPoolLimits::unlimited());
    database.apply_pool_trace(&mut pool);
    assert_eq!(pool.usage().strings(), 2);
    assert_eq!(pool.usage().characters(), 8);
}

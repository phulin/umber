use bib_bst::{CompileLimits, compile};
use bib_input::{BibTexOptions, parse_raw_bibtex_bytes};
use umber_vfs::{FileContentId, VirtualPath};

use super::{ClassicVmDiagnosticKind, ClassicVmLimits, execute_classic_style};
use crate::{
    ClassicControl, ClassicDatabaseOptions, ClassicDatabaseSource, prepare_classic_database,
};

fn database(
    style: &bib_bst::CompiledStyle,
    source: &[u8],
    citations: &[&str],
) -> crate::ClassicDatabase {
    let raw = parse_raw_bibtex_bytes(source, BibTexOptions::default());
    let path = VirtualPath::user("refs.bib").expect("path");
    prepare_classic_database(
        &ClassicControl::for_read_test(citations),
        style,
        &[ClassicDatabaseSource::new(
            &path,
            FileContentId::for_bytes(source),
            &raw,
        )],
        &ClassicDatabaseOptions::default(),
    )
}

fn run(source: &[u8], bib: &[u8]) -> super::ClassicVmResult {
    let compiled = compile(source, CompileLimits::default());
    let style = compiled
        .program()
        .unwrap_or_else(|| panic!("compiled BST: {:?}", compiled.diagnostics()));
    execute_classic_style(
        style,
        &database(style, bib, &["*"]),
        ClassicVmLimits::default(),
    )
}

#[test]
fn reads_mutates_sorts_and_emits_deterministically() {
    let result = run(
        br#"ENTRY { title } { count } { label }
INTEGERS { total }
FUNCTION {init} { #0 'total := }
FUNCTION {item} { total #1 + 'total := title 'label := label 'sort.key$ := cite$ write$ newline$ }
READ EXECUTE {init} ITERATE {item} SORT"#,
        br#"@book{z, title = "z"} @book{a, title = "a"}"#,
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(result.bbl(), Some("z\na\n"));
    assert_eq!(result.entry_order(), ["a", "z"]);
}

#[test]
fn nested_calls_and_control_flow_use_explicit_frames() {
    let result = run(
        br#"ENTRY {} {} {}
FUNCTION {leaf} { "x" write$ }
FUNCTION {branch} { #1 { leaf } { } if$ }
READ EXECUTE {branch}"#,
        b"@book{one}",
    );
    assert!(!result.is_fatal());
    assert_eq!(result.bbl(), Some("x"));
}

#[test]
fn wrong_types_and_underflow_are_fatal_and_transactional() {
    let result = run(
        b"ENTRY {} {} {} FUNCTION {bad} { pop$ } READ EXECUTE {bad}",
        b"@book{one}",
    );
    assert!(result.is_fatal());
    assert_eq!(result.bbl(), None);
    assert_eq!(
        result.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::Underflow
    );
}

#[test]
fn wrong_types_and_missing_entry_context_are_diagnostic() {
    let wrong_type = run(
        b"ENTRY {} {} {} FUNCTION {bad} { \"x\" #1 + } READ EXECUTE {bad}",
        b"@book{one}",
    );
    assert!(wrong_type.is_fatal());
    assert_eq!(
        wrong_type.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::WrongType
    );
    let no_entry = run(
        b"ENTRY {} {} {} FUNCTION {bad} { cite$ } READ EXECUTE {bad}",
        b"@book{one}",
    );
    assert!(no_entry.is_fatal());
    assert_eq!(
        no_entry.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::NoCurrentEntry
    );
}

#[test]
fn reverse_observes_the_current_database_order() {
    let result = run(
        b"ENTRY {} {} {} FUNCTION {item} { cite$ write$ } READ REVERSE {item}",
        b"@book{one, title = \"one\"} @book{two, title = \"two\"}",
    );
    assert!(!result.is_fatal());
    assert_eq!(result.bbl(), Some("twoone"));
}

#[test]
fn work_and_call_limits_terminate_loops_without_rust_recursion() {
    let compiled = compile(
        b"ENTRY {} {} {} FUNCTION {loop} { { #1 } { } while$ } READ EXECUTE {loop}",
        CompileLimits::default(),
    );
    let style = compiled.program().expect("style");
    let result = execute_classic_style(
        style,
        &database(style, b"@book{one}", &["*"]),
        ClassicVmLimits {
            work: 40,
            ..ClassicVmLimits::default()
        },
    );
    assert!(result.is_fatal());
    assert_eq!(
        result.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::Limit
    );
}

#[test]
fn compiler_rejects_direct_recursion_before_execution() {
    let compiled = compile(b"FUNCTION {loop} { loop }", CompileLimits::default());
    assert!(!compiled.is_success());
    assert!(
        compiled.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.kind(),
            bib_bst::DiagnosticKind::IllegalRecursion
        ))
    );
}

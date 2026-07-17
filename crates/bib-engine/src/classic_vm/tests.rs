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
fn quoted_assignments_and_control_continuations_preserve_operand_order() {
    let result = run(
        br#"ENTRY {} {} {}
INTEGERS { counter }
STRINGS { saved }
FUNCTION {increment} { counter #1 + 'counter := }
FUNCTION {condition} { counter #3 < }
FUNCTION {main} {
  "kept" 'saved :=
  #0 'counter :=
  'condition 'increment while$
  counter #3 =
    { saved write$ }
    { "wrong" write$ }
  if$
}
READ EXECUTE {main}"#,
        b"@book{one}",
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(result.bbl(), Some("kept"));
}

#[test]
fn quoted_mutable_symbols_are_deferred_control_operands() {
    let result = run(
        br#"ENTRY {} {} {}
INTEGERS { condition }
STRINGS { saved }
FUNCTION {main} {
  "kept" 'saved :=
  #0 'condition :=
  'condition 'skip$ while$
  #0 { "wrong" } 'saved if$ write$
}
READ EXECUTE {main}"#,
        b"@book{one}",
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(result.bbl(), Some("kept"));
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

#[test]
fn string_and_text_builtins_preserve_classic_tex_units() {
    let result = run(
        br#"ENTRY {} {} {}
FUNCTION {main} {
  "A" "B" * write$ "Hello" add.period$ write$
  "{\i}bc" text.length$ int.to.str$ write$
  "{\i}bc" #2 text.prefix$ write$
  "abcdef" #2 #3 substring$ write$
  "{\i} -- {\oe}" purify$ write$
  "MiXeD: A TITLE" "t" change.case$ write$
}
READ EXECUTE {main}"#,
        b"@book{one}",
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(
        result.bbl(),
        Some("ABHello.3{\\i}bbcdi    oeMixed: A title")
    );
}

#[test]
fn substring_negative_starts_count_back_from_the_right_edge() {
    assert_eq!(super::classic_substring("abcdef", -1, 4), "cdef");
    assert_eq!(super::classic_substring("abcdef", -2, 4), "bcde");
    assert_eq!(super::classic_substring("1984", -1, 4), "1984");
}

#[test]
fn conversion_names_entry_dispatch_and_width_are_available() {
    let result = run(
        br#"ENTRY {} {} {}
FUNCTION {book} { "typed" write$ }
FUNCTION {default.type} { "default" write$ }
FUNCTION {item} {
  "start" write$ call.type$ cite$ write$ type$ write$
  "John von Neumann and Jane Doe" num.names$ int.to.str$ write$
  "John von Neumann and Jane Doe" #1 "{vv }{ll}, {f}" format.name$ write$
  #65 int.to.chr$ chr.to.int$ int.to.str$ write$
  "A" width$ int.to.str$ write$
}
READ ITERATE {item}"#,
        b"@book{one, title = \"one\"}",
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(
        result.bbl(),
        Some("starttypedonebook2von Neumann, J65750"),
        "{:?}",
        result.diagnostics()
    );
}

#[test]
fn format_name_handles_no_comma_names_without_lowercase_particles() {
    let parsed = super::BibName::parse("Donald E. Knuth");
    assert_eq!(
        parsed.first.iter().map(String::as_str).collect::<Vec<_>>(),
        ["Donald", "E."]
    );
    assert!(parsed.von.is_empty());
    assert_eq!(
        parsed.last.iter().map(String::as_str).collect::<Vec<_>>(),
        ["Knuth"]
    );

    let result = run(
        br#"ENTRY {} {} {}
FUNCTION {item} {
  "Donald E. Knuth" #1 "{ff }{vv }{ll}{, jj}" format.name$ write$
}
READ ITERATE {item}"#,
        b"@book{one, title = \"one\"}",
    );
    assert!(!result.is_fatal(), "{:?}", result.diagnostics());
    assert_eq!(result.bbl(), Some("Donald E. Knuth"));

    assert_eq!(
        super::format_bib_name("Donald E. Knuth", "{ff~}{vv~}{ll}{, jj}"),
        "Donald~E. Knuth"
    );
    assert_eq!(
        super::format_bib_name("Donald E. Knuth", "{vv~}{ll}{, jj}{, f.}"),
        "Knuth, D.~E."
    );
}

#[test]
fn format_name_honors_nested_patterns_and_classic_initials() {
    assert_eq!(
        super::format_bib_name("L[eslie] A. Aamport", "{vv{ } }{ll{ }}{  f{ }}{  jj{ }}"),
        "Aamport  L A"
    );
    assert_eq!(
        super::format_bib_name("Masterly, {\\'{E}}douard", "{f.} {ll}"),
        "{\\'{E}}. Masterly"
    );
    assert_eq!(
        super::format_bib_name("Jean-Baptiste Missilany", "{f.} {ll}"),
        "J.-B. Missilany"
    );
}

#[test]
fn builtin_errors_and_output_limits_remain_bounded() {
    let wrong_type = run(
        b"ENTRY {} {} {} FUNCTION {bad} { #1 purify$ } READ EXECUTE {bad}",
        b"@book{one}",
    );
    assert!(wrong_type.is_fatal());
    assert_eq!(
        wrong_type.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::WrongType
    );

    let compiled = compile(
        b"ENTRY {} {} {} FUNCTION {out} { \"abcd\" write$ } READ EXECUTE {out}",
        CompileLimits::default(),
    );
    let style = compiled.program().expect("style");
    let result = execute_classic_style(
        style,
        &database(style, b"@book{one}", &["*"]),
        ClassicVmLimits {
            bbl_bytes: 3,
            ..ClassicVmLimits::default()
        },
    );
    assert!(result.is_fatal());
    assert_eq!(
        result.diagnostics()[0].kind(),
        ClassicVmDiagnosticKind::Limit
    );
}

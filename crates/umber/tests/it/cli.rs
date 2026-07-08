use std::{fs, process::Command};

use refexec::{DviComparison, RefTex, RunOpts};
use test_support::{assert_matches_fixture, normalize};
use tex_lex::{Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{Universe, World};

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/lexer");
    for entry in std::fs::read_dir(&corpus).expect("read lexer corpus") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("tex") {
            continue;
        }
        let stem = path.file_stem().expect("fixture stem");
        let expected = path.with_file_name(format!("{}.expected.tokens", stem.to_string_lossy()));

        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .arg("lex-dump")
            .arg(&path)
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        let expected = std::fs::read_to_string(&expected).expect("read expected tokens");
        assert_eq!(actual, expected, "fixture mismatch for {}", path.display());
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn expand_dump_prints_stable_token_format_for_corpus() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/expand");
    for entry in std::fs::read_dir(&corpus).expect("read expansion corpus") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("tex") {
            continue;
        }
        let stem = path.file_stem().expect("fixture stem").to_string_lossy();

        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .arg("expand-dump")
            .arg(&path)
            .output()
            .expect("run umber expand-dump");

        assert!(
            output.status.success(),
            "expand-dump failed for {}:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("expand-dump output is utf-8");
        assert_matches_fixture("expand", &stem, "tokens", &actual);
    }
}

#[test]
fn expand_dump_usage_errors_follow_lex_dump_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("expand-dump")
        .output()
        .expect("run umber expand-dump without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for expand-dump\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("expand-dump")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber expand-dump with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: expand-dump accepts exactly one input path\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_exec_corpus_matches_pdftex_diagnostics() {
    run_corpus_matches_pdftex_diagnostics("exec", false);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_typeset_corpus_matches_pdftex_box_dumps() {
    run_corpus_matches_pdftex_diagnostics("typeset", true);
}

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_corpus_matches_pdftex_diagnostics(area: &str, show_fixtures: bool) {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus")
        .join(area);
    let ref_tex = RefTex::locate().expect("reference TeX should be available");

    for entry in std::fs::read_dir(&corpus).expect("read exec corpus") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("tex") {
            continue;
        }
        let stem = path.file_stem().expect("fixture stem").to_string_lossy();

        let ref_output = ref_tex
            .run(&path, &RunOpts::default())
            .expect("reference TeX should run exec fixture");
        let expected = if show_fixtures {
            normalize::box_dump(&ref_output.log)
        } else {
            normalize::exec_log(&ref_output.log)
        };
        assert_matches_fixture(area, &stem, "log", &expected);

        let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
        command.arg("run");
        if show_fixtures {
            command.arg("--show-fixtures");
        }
        let output = command.arg(&path).output().expect("run umber run");
        assert!(
            output.status.success(),
            "umber run failed for {}:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual_stdout = String::from_utf8(output.stdout).expect("umber run output is utf-8");
        let actual = if show_fixtures {
            normalize::box_dump(&actual_stdout)
        } else {
            normalize::exec_log(&actual_stdout)
        };
        assert_eq!(
            actual,
            expected,
            "{area} fixture mismatch for {}",
            path.display()
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side optional corpus and command execution.
fn run_hyphen_showhyphens_corpus_matches_pdftex() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let hyphen_tex = repo_root.join("third_party/hyphen/hyphen.tex");
    if !hyphen_tex.exists() {
        eprintln!(
            "skipping hyphen showhyphens parity: {}; run scripts/fetch-hyphen-corpus.sh",
            hyphen_tex.display()
        );
        return;
    }
    let ref_tex = match RefTex::locate() {
        Ok(ref_tex) => ref_tex,
        Err(error) => {
            eprintln!("skipping hyphen showhyphens parity: {error:#}");
            return;
        }
    };

    assert_eq!(HYPHEN_PARITY_WORDS.len(), 200);
    let temp_dir = tempfile::tempdir().expect("create hyphen parity temp dir");
    fs::copy(&hyphen_tex, temp_dir.path().join("hyphen.tex")).expect("copy hyphen.tex");

    let ref_input = temp_dir.path().join("pdftex-showhyphens.tex");
    fs::write(&ref_input, showhyphens_source(false)).expect("write pdftex hyphen input");
    let ref_output = ref_tex
        .run(&ref_input, &RunOpts::default())
        .expect("run pdftex hyphen corpus");
    assert!(
        ref_output.success,
        "pdftex hyphen corpus failed:\n{}",
        ref_output.log
    );
    let expected = normalize::showhyphens(&ref_output.log);

    let umber_input = temp_dir.path().join("umber-showhyphens.tex");
    fs::write(&umber_input, showhyphens_source(true)).expect("write umber hyphen input");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg(&umber_input)
        .output()
        .expect("run umber hyphen corpus");
    assert!(
        output.status.success(),
        "umber hyphen corpus failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_stdout = String::from_utf8(output.stdout).expect("umber run output is utf-8");
    let actual = normalize::showhyphens(&actual_stdout);

    assert_eq!(actual, expected, "hyphen.tex showhyphens corpus drifted");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_single_glyph() {
    assert_dvi_case_matches_pdftex("single_glyph");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_overfull_rule() {
    assert_dvi_case_matches_pdftex("overfull_rule");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_default_output_end() {
    assert_dvi_case_matches_pdftex("default_output_end");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_custom_output_headline() {
    assert_dvi_case_matches_pdftex("custom_output_headline");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_mark_output_headers() {
    assert_dvi_case_matches_pdftex("mark_output_headers");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_dvi_smoke_matches_pdftex_insert_split_footnote() {
    assert_dvi_case_matches_pdftex("insert_split_footnote");
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_case_matches_pdftex(case: &str) {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = repo_root
        .join("tests/corpus/dvi")
        .join(format!("{case}.tex"));
    let cmr10 = repo_root.join("crates/tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let temp_dir = tempfile::tempdir().expect("create DVI smoke temp dir");
    let case_path = temp_dir.path().join(format!("{case}.tex"));
    let tfm_path = temp_dir.path().join("cmr10.tfm");
    let actual_path = temp_dir.path().join("actual.dvi");
    fs::copy(&source, &case_path).expect("copy DVI smoke source");
    fs::copy(&cmr10, &tfm_path).expect("copy pinned cmr10.tfm");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(temp_dir.path())
        .arg("run")
        .arg(format!("{case}.tex"))
        .arg("--dvi")
        .arg("actual.dvi")
        .output()
        .expect("run umber DVI smoke");
    assert!(
        output.status.success(),
        "umber DVI smoke failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(&actual_path).expect("read umber DVI");
    let comparison = RefTex::locate()
        .expect("locate pdftex")
        .compare_dvi(
            &case_path,
            &actual,
            &RunOpts {
                extra_inputs: vec![tfm_path],
                ..RunOpts::default()
            },
        )
        .expect("compare reference DVI");
    assert_eq!(comparison, DviComparison::Equal);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_reports_deadcycles_overflow_primary_text() {
    let temp_dir = tempfile::tempdir().expect("create deadcycles temp dir");
    let source = temp_dir.path().join("deadcycles.tex");
    fs::write(
        &source,
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\n\
         \\topskip=0pt \\setbox0=\\hbox{}\n\
         \\copy0 \\penalty-10000\n\
         \\copy0 \\penalty-10000\n",
    )
    .expect("write deadcycles fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber deadcycles fixture");

    assert!(!output.status.success(), "deadcycles run should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Output loop---1 consecutive dead cycles"));
}

#[test]
fn run_usage_errors_follow_existing_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .output()
        .expect("run umber run without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber run with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let missing_show_fixtures = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--show-fixtures")
        .output()
        .expect("run umber run with show-fixtures but without path");
    assert!(!missing_show_fixtures.status.success());
    assert_eq!(
        String::from_utf8(missing_show_fixtures.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let missing_dvi_path = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("one.tex")
        .arg("--dvi")
        .output()
        .expect("run umber run with --dvi but without output path");
    assert!(!missing_dvi_path.status.success());
    assert_eq!(
        String::from_utf8(missing_dvi_path.stderr).expect("stderr is utf-8"),
        "umber: missing output path for --dvi\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture command execution and file checks.
fn run_show_fixtures_harvests_without_committing_stream_effects() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let normal_dir = temp_dir.path().join("normal");
    let fixture_dir = temp_dir.path().join("fixture");
    fs::create_dir_all(&normal_dir).expect("create normal dir");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let input = temp_dir.path().join("stream_effect.tex");
    fs::write(&input, "\\openout0=side-effect.txt\n\\closeout0\n\\end\n").expect("write input");

    let normal = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(&normal_dir)
        .arg("run")
        .arg(&input)
        .output()
        .expect("run ordinary umber run");
    assert!(
        normal.status.success(),
        "ordinary run failed:\n{}",
        String::from_utf8_lossy(&normal.stderr)
    );
    assert!(
        normal_dir.join("side-effect.txt").exists(),
        "ordinary run should commit \\openout effects"
    );

    let fixture = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(&fixture_dir)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&input)
        .output()
        .expect("run umber fixture harvest");
    assert!(
        fixture.status.success(),
        "fixture run failed:\n{}",
        String::from_utf8_lossy(&fixture.stderr)
    );
    assert!(
        !fixture_dir.join("side-effect.txt").exists(),
        "--show-fixtures must not commit pending stream effects"
    );
}

fn showhyphens_source(load_hyphen: bool) -> String {
    let mut source = String::new();
    if load_hyphen {
        source.push_str("\\input hyphen\n");
    }
    for word in HYPHEN_PARITY_WORDS {
        source.push_str("\\showhyphens{");
        source.push_str(word);
        source.push_str("}\n");
    }
    source.push_str("\\end\n");
    source
}

const HYPHEN_PARITY_WORDS: &[&str] = &[
    "hyphenation",
    "representative",
    "algorithm",
    "computer",
    "science",
    "mathematics",
    "language",
    "programming",
    "portable",
    "implementation",
    "comparison",
    "diagnostic",
    "normalization",
    "exception",
    "patterns",
    "boundary",
    "paragraph",
    "typesetting",
    "discretionary",
    "ligature",
    "kerning",
    "baseline",
    "dimension",
    "magnification",
    "assignment",
    "primitive",
    "expansion",
    "conditionals",
    "registers",
    "universe",
    "snapshot",
    "rollback",
    "terminal",
    "transcript",
    "ordinary",
    "letters",
    "lowercase",
    "uppercase",
    "character",
    "sequence",
    "interpreter",
    "execution",
    "analysis",
    "architecture",
    "reference",
    "validation",
    "fixture",
    "corpus",
    "future",
    "stability",
    "automatic",
    "manual",
    "associate",
    "associates",
    "declination",
    "obligatory",
    "philanthropic",
    "reciprocity",
    "recognizance",
    "reformation",
    "table",
    "index",
    "memory",
    "format",
    "plain",
    "engine",
    "workflow",
    "coordinate",
    "quality",
    "testing",
    "failure",
    "success",
    "visible",
    "invisible",
    "accurate",
    "behavior",
    "semantic",
    "persistent",
    "journal",
    "content",
    "storage",
    "scanner",
    "token",
    "balanced",
    "braces",
    "spaces",
    "control",
    "symbol",
    "mutable",
    "immutable",
    "history",
    "version",
    "document",
    "process",
    "builder",
    "horizontal",
    "vertical",
    "material",
    "natural",
    "stretch",
    "shrink",
    "penalty",
    "badness",
    "tolerance",
    "pretolerance",
    "package",
    "project",
    "repository",
    "portable",
    "modern",
    "performance",
    "optimization",
    "profile",
    "correctness",
    "parity",
    "coverage",
    "regression",
    "represent",
    "normalize",
    "compare",
    "output",
    "input",
    "source",
    "available",
    "optional",
    "distribution",
    "installation",
    "developer",
    "maintainer",
    "interface",
    "command",
    "script",
    "fetching",
    "located",
    "current",
    "relative",
    "absolute",
    "directory",
    "temporary",
    "execution",
    "captured",
    "message",
    "underfull",
    "overfull",
    "paragraphs",
    "minimum",
    "maximum",
    "language",
    "english",
    "american",
    "dictionary",
    "exceptional",
    "educational",
    "institution",
    "international",
    "representation",
    "responsibility",
    "characteristic",
    "configuration",
    "communication",
    "documentation",
    "implementation",
    "initialization",
    "interpretation",
    "localization",
    "organization",
    "presentation",
    "recommendation",
    "specification",
    "transformation",
    "verification",
    "application",
    "development",
    "foundation",
    "generation",
    "operation",
    "resolution",
    "translation",
    "variation",
    "evaluation",
    "iteration",
    "integration",
    "migration",
    "selection",
    "transaction",
    "allocation",
    "collection",
    "definition",
    "description",
    "extension",
    "function",
    "location",
    "notation",
    "position",
    "question",
    "relation",
    "solution",
    "buffer",
    "kernel",
    "driver",
];

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus files, not engine I/O.
fn lexer_dynamic_corpus_covers_mutable_input_state() {
    assert_matches_fixture(
        "lexer_dynamic",
        "catcode_mutation",
        "tokens",
        &lex_catcode_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "endlinechar_mutation",
        "tokens",
        &lex_endlinechar_mutation_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "ignored_character",
        "tokens",
        &lex_ignored_character_fixture(),
    );
    assert_matches_fixture(
        "lexer_dynamic",
        "invalid_character",
        "tokens",
        &lex_invalid_character_fixture(),
    );
}

fn lex_catcode_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("catcode_mutation");
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_catcode('@', Catcode::Letter);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_endlinechar_mutation_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("endlinechar_mutation");
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);
    let mut actual = String::new();

    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, b'?' as i32);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    push_next_token(&mut actual, &mut lexer, &mut stores);
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_ignored_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("ignored_character");
    stores.set_catcode('!', Catcode::Ignored);
    let mut actual = String::new();

    push_remaining_tokens(&mut actual, &mut lexer, &mut stores);

    actual
}

fn lex_invalid_character_fixture() -> String {
    let (mut lexer, mut stores) = lexer_fixture("invalid_character");
    stores.set_catcode('?', Catcode::Invalid);
    let mut actual = String::new();

    loop {
        match lexer.next_token(&mut stores) {
            Ok(Some(token)) => push_token(&mut actual, token, &stores),
            Ok(None) => break,
            Err(err) => {
                actual.push_str(&format!("error:{err}\n"));
                break;
            }
        }
    }

    actual
}

fn lexer_fixture(case: &str) -> (Lexer<WorldInput>, Universe) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/lexer_dynamic")
        .join(format!("{case}.tex"));
    let mut stores = Universe::with_world(World::real());
    let content = stores
        .world_mut()
        .read_file(&path)
        .expect("open dynamic lexer fixture");
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    (Lexer::new(WorldInput::from_content(content)), stores)
}

fn push_remaining_tokens(
    actual: &mut String,
    lexer: &mut Lexer<WorldInput>,
    stores: &mut Universe,
) {
    while let Some(token) = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
    {
        push_token(actual, token, stores);
    }
}

fn push_next_token(actual: &mut String, lexer: &mut Lexer<WorldInput>, stores: &mut Universe) {
    let token = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
        .expect("dynamic lexer fixture ended early");
    push_token(actual, token, stores);
}

fn push_token(actual: &mut String, token: Token, stores: &Universe) {
    let line = match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
    };
    actual.push_str(&line);
    actual.push('\n');
}

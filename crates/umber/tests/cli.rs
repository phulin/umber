use std::process::Command;

use test_support::assert_matches_fixture;
use tex_lex::{FileInput, Lexer};
use tex_state::env::banks::IntParam;
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

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

#[allow(clippy::disallowed_methods)] // host-side corpus fixture open.
fn lexer_fixture(case: &str) -> (Lexer<FileInput>, Stores) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/corpus/lexer_dynamic")
        .join(format!("{case}.tex"));
    let file = std::fs::File::open(&path).expect("open dynamic lexer fixture");
    let mut stores = Stores::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    (Lexer::new(FileInput::from_file(file)), stores)
}

fn push_remaining_tokens(actual: &mut String, lexer: &mut Lexer<FileInput>, stores: &mut Stores) {
    while let Some(token) = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
    {
        push_token(actual, token, stores);
    }
}

fn push_next_token(actual: &mut String, lexer: &mut Lexer<FileInput>, stores: &mut Stores) {
    let token = lexer
        .next_token(stores)
        .expect("dynamic lexer fixture should succeed")
        .expect("dynamic lexer fixture ended early");
    push_token(actual, token, stores);
}

fn push_token(actual: &mut String, token: Token, stores: &Stores) {
    let line = match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
    };
    actual.push_str(&line);
    actual.push('\n');
}

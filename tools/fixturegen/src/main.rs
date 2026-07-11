#![allow(clippy::disallowed_methods)] // host-side fixture regeneration tool.

mod fonts;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use refexec::{RefTex, RunOpts, RunOutput};
use tempfile::TempDir;
use test_support::{corpus_cases, corpus_root, fixture_path, normalize};
use tex_lex::{Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::{Catcode, Token};
use tex_state::{Universe, World};

const TEXT_AREAS: &[&str] = &[
    "hello",
    "lexer",
    "expand",
    "lexer_dynamic",
    "exec",
    "typeset",
    "tex_exec",
    "tex_exec_io",
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("--area") => {
            let area = args.next().context("missing area after --area")?;
            ensure_no_extra_args(args)?;
            regenerate_area(&area)
        }
        Some("--case") => {
            let first = args.next().context("missing case after --case")?;
            let (area, case) = if let Some((area, case)) = first.split_once('/') {
                (area.to_owned(), strip_case_suffixes(case))
            } else {
                let case = args
                    .next()
                    .context("--case requires AREA CASE or AREA/CASE")?;
                (first, strip_case_suffixes(&case))
            };
            ensure_no_extra_args(args)?;
            regenerate_case(&area, &case)
        }
        Some("--help") | Some("-h") => {
            print_usage();
            Ok(())
        }
        Some(arg) => bail!("unknown argument: {arg}"),
        None => {
            print_usage();
            bail!("missing mode")
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage: fixturegen --area AREA | --case AREA/CASE | --case AREA CASE\n\
         areas: hello lexer expand lexer_dynamic exec typeset tex_exec tex_exec_io fonts"
    );
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(extra) = args.next() {
        bail!("unexpected extra argument: {extra}");
    }
    Ok(())
}

fn regenerate_area(area: &str) -> Result<()> {
    match area {
        "hello" => regenerate_cases(area, regenerate_hello_case),
        "lexer" => regenerate_cases(area, |case| {
            regenerate_umber_dump_case(area, case, "lex-dump")
        }),
        "expand" => regenerate_cases(area, |case| {
            regenerate_umber_dump_case(area, case, "expand-dump")
        }),
        "lexer_dynamic" => regenerate_cases(area, regenerate_lexer_dynamic_case),
        "exec" => regenerate_cases(area, |case| {
            regenerate_reference_log_case(area, case, false)
        }),
        "typeset" => regenerate_cases(area, |case| regenerate_reference_log_case(area, case, true)),
        "tex_exec" => regenerate_cases(area, regenerate_tex_exec_case),
        "tex_exec_io" => regenerate_cases(area, regenerate_tex_exec_io_case),
        "fonts" => fonts::run(&repo_root()),
        _ => bail!("unknown fixturegen area: {area}"),
    }
}

fn regenerate_cases(area: &str, mut regenerate: impl FnMut(&str) -> Result<()>) -> Result<()> {
    let cases = corpus_cases(area);
    if cases.is_empty() {
        bail!("no .tex cases found for area {area}");
    }
    for case in cases {
        regenerate(case.name())?;
    }
    Ok(())
}

fn regenerate_case(area: &str, case: &str) -> Result<()> {
    if area == "fonts" {
        bail!("--case is not meaningful for the fonts live check");
    }
    if !TEXT_AREAS.contains(&area) {
        bail!("unknown fixturegen area: {area}");
    }
    match area {
        "hello" => regenerate_hello_case(case),
        "lexer" => regenerate_umber_dump_case(area, case, "lex-dump"),
        "expand" => regenerate_umber_dump_case(area, case, "expand-dump"),
        "lexer_dynamic" => regenerate_lexer_dynamic_case(case),
        "exec" => regenerate_reference_log_case(area, case, false),
        "typeset" => regenerate_reference_log_case(area, case, true),
        "tex_exec" => regenerate_tex_exec_case(case),
        "tex_exec_io" => regenerate_tex_exec_io_case(case),
        _ => unreachable!("known area already checked"),
    }
}

fn regenerate_hello_case(case: &str) -> Result<()> {
    let source = source_path("hello", case);
    let output = RefTex::locate()?.run(&source, &RunOpts::default())?;
    if !output.success {
        bail!("reference TeX failed for hello/{case}:\n{}", output.log);
    }
    if !output.stdout.contains("hello umber") {
        bail!("hello/{case} reference stdout did not contain hello message");
    }
    write_text_fixture("hello", case, "log", &normalize::tex_log(&output.log))
}

fn regenerate_umber_dump_case(area: &str, case: &str, command_name: &str) -> Result<()> {
    let output = Command::new(umber_bin())
        .arg(command_name)
        .arg(source_path(area, case))
        .output()
        .with_context(|| format!("failed to run umber {command_name}"))?;
    if !output.status.success() {
        bail!(
            "umber {command_name} failed for {area}/{case}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let actual = String::from_utf8(output.stdout).context("umber dump output was not utf-8")?;
    write_text_fixture(area, case, "tokens", &actual)
}

fn regenerate_lexer_dynamic_case(case: &str) -> Result<()> {
    let actual = match case {
        "catcode_mutation" => lex_catcode_mutation_fixture(),
        "endlinechar_mutation" => lex_endlinechar_mutation_fixture(),
        "ignored_character" => lex_ignored_character_fixture(),
        "invalid_character" => lex_invalid_character_fixture(),
        _ => bail!("unknown lexer_dynamic case: {case}"),
    };
    write_text_fixture("lexer_dynamic", case, "tokens", &actual)
}

fn regenerate_reference_log_case(area: &str, case: &str, box_dump: bool) -> Result<()> {
    let output = RefTex::locate()?.run(&source_path(area, case), &RunOpts::default())?;
    let actual = if box_dump {
        normalize::box_dump(&output.log)
    } else {
        normalize::exec_log(&output.log)
    };
    write_text_fixture(area, case, "log", &actual)
}

fn regenerate_tex_exec_case(case: &str) -> Result<()> {
    let output = RefTex::locate()?.run(&source_path("tex_exec", case), &RunOpts::default())?;
    write_text_fixture("tex_exec", case, "ref", &format_micro_reference(&output))
}

fn regenerate_tex_exec_io_case(case: &str) -> Result<()> {
    let spec = io_case_spec(case)?;
    let temp_dir = TempDir::new().context("failed to create reference I/O temp dir")?;
    let source_name = format!("{case}.tex");
    fs::copy(
        source_path("tex_exec_io", case),
        temp_dir.path().join(&source_name),
    )
    .with_context(|| format!("failed to copy tex_exec_io/{case}.tex"))?;

    let needs_dvi = matches!(spec.effects, Some(IoEffects::LeaderPayload)) || spec.specials;
    let output = RefTex::locate()?.run_in_dir(
        temp_dir.path(),
        Path::new(&source_name),
        &RunOpts {
            dvi: needs_dvi,
            ..RunOpts::default()
        },
    )?;
    if !output.success {
        bail!(
            "reference TeX failed for tex_exec_io/{case}:\n{}",
            output.log
        );
    }

    if let Some(output_name) = spec.output_name {
        let bytes = fs::read(temp_dir.path().join(output_name))
            .with_context(|| format!("failed to read reference output {output_name}"))?;
        let text = String::from_utf8(bytes).context("reference output was not utf-8")?;
        write_text_fixture("tex_exec_io", case, "out", &text)?;
    }
    if let Some(effects) = spec.effects {
        let text = match effects {
            IoEffects::LeaderPayload => {
                let leader_out = if temp_dir.path().join("leader.out").exists() {
                    "present"
                } else {
                    "absent"
                };
                format!(
                    "leader.out: {leader_out}\nleader-write-in-log: {}\n",
                    output.log.contains("leader-write")
                )
            }
            IoEffects::OutputPresence(paths) => format_output_presence(temp_dir.path(), paths)?,
        };
        write_text_fixture("tex_exec_io", case, "effects", &text)?;
    }
    if spec.specials {
        let dvi = output.dvi.context("reference TeX did not produce DVI")?;
        write_text_fixture(
            "tex_exec_io",
            case,
            "specials",
            &format_special_payloads(&dvi_special_payloads(&dvi)),
        )?;
    }

    Ok(())
}

fn format_micro_reference(output: &RunOutput) -> String {
    format!(
        "success: {}\nstdout:\n{}log:\n{}",
        output.success,
        normalize_micro_reference_text(&output.stdout),
        normalize_micro_reference_text(&output.log)
    )
}

fn normalize_micro_reference_text(text: &str) -> String {
    let mut lines = Vec::new();
    for line in normalize::exec_log(text).lines() {
        let line = line.split_once(" [").map_or(line, |(message, _)| message);
        if line.starts_with("Output written on ")
            || line.starts_with("pdftex/")
            || line.starts_with("lic/")
            || line.starts_with("</")
        {
            continue;
        }
        lines.push(line.to_owned());
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

#[derive(Clone, Copy)]
struct IoCaseSpec {
    output_name: Option<&'static str>,
    effects: Option<IoEffects>,
    specials: bool,
}

#[derive(Clone, Copy)]
enum IoEffects {
    LeaderPayload,
    OutputPresence(&'static [&'static str]),
}

fn io_case_spec(case: &str) -> Result<IoCaseSpec> {
    match case {
        "top_open_close" => Ok(IoCaseSpec {
            output_name: Some("top.out"),
            effects: None,
            specials: false,
        }),
        "ordinary_open_close" => Ok(IoCaseSpec {
            output_name: Some("ordinary.out"),
            effects: None,
            specials: false,
        }),
        "open_close_without_write" => Ok(IoCaseSpec {
            output_name: None,
            effects: Some(IoEffects::OutputPresence(&[
                "immediate.out",
                "shipped.out",
                "boxed.out",
                "top.out",
            ])),
            specials: false,
        }),
        "special_payload" => Ok(IoCaseSpec {
            output_name: None,
            effects: None,
            specials: true,
        }),
        "leader_payload_effects" => Ok(IoCaseSpec {
            output_name: None,
            effects: Some(IoEffects::LeaderPayload),
            specials: true,
        }),
        _ => bail!("unknown tex_exec_io case: {case}"),
    }
}

fn format_output_presence(run_dir: &Path, paths: &[&str]) -> Result<String> {
    let mut output = String::new();
    for path in paths {
        let state = match fs::metadata(run_dir.join(path)) {
            Ok(metadata) => format!("present:{} bytes", metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_owned(),
            Err(error) => bail!("failed to stat reference output {path}: {error}"),
        };
        output.push_str(path);
        output.push_str(": ");
        output.push_str(&state);
        output.push('\n');
    }
    Ok(output)
}

fn write_text_fixture(area: &str, case: &str, kind: &str, actual: &str) -> Result<()> {
    let path = fixture_path(area, case, kind);
    let unchanged = fs::read_to_string(&path).ok().as_deref() == Some(actual);
    if unchanged {
        eprintln!("fixture unchanged: {}", display_repo_path(&path));
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create fixture directory {}", parent.display()))?;
    }
    fs::write(&path, actual).with_context(|| format!("failed to write {}", path.display()))?;
    eprintln!("fixture updated: {}", display_repo_path(&path));
    Ok(())
}

fn source_path(area: &str, case: &str) -> PathBuf {
    corpus_root().join(area).join(format!("{case}.tex"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn umber_bin() -> PathBuf {
    env::var_os("UMBER_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target/debug/umber"))
}

fn display_repo_path(path: &Path) -> String {
    if let Ok(rest) = path.strip_prefix(corpus_root()) {
        return format!("tests/corpus/{}", rest.display());
    }
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn strip_case_suffixes(case: &str) -> String {
    let mut name = case.strip_suffix(".tex").unwrap_or(case);
    for suffix in [
        ".expected.dvi",
        ".expected.log",
        ".expected.tokens",
        ".expected.ref",
        ".expected.out",
        ".expected.effects",
        ".expected.specials",
    ] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    name.to_owned()
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
    let path = source_path("lexer_dynamic", case);
    let mut stores = Universe::with_world(World::real());
    let content = stores
        .world_mut()
        .read_file(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
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
        token if token.is_frozen_end_template() => "frozen:endtemplate".to_owned(),
        token if token.is_frozen_endv() => "frozen:endv".to_owned(),
        Token::Frozen(_) => unreachable!("invalid frozen token payload"),
    };
    actual.push_str(&line);
    actual.push('\n');
}

fn format_special_payloads(payloads: &[Vec<u8>]) -> String {
    let mut output = String::new();
    for payload in payloads {
        output.push_str(&String::from_utf8_lossy(payload));
        output.push('\n');
    }
    output
}

fn dvi_special_payloads(dvi: &[u8]) -> Vec<Vec<u8>> {
    const XXX1: u8 = 239;
    const XXX4: u8 = 242;

    let mut payloads = Vec::new();
    let mut index = 0usize;
    while index < dvi.len() {
        match dvi[index] {
            XXX1 if index + 2 <= dvi.len() => {
                let len = dvi[index + 1] as usize;
                let start = index + 2;
                let end = start + len;
                if end <= dvi.len() {
                    payloads.push(dvi[start..end].to_vec());
                    index = end;
                    continue;
                }
            }
            XXX4 if index + 5 <= dvi.len() => {
                let len = u32::from_be_bytes([
                    dvi[index + 1],
                    dvi[index + 2],
                    dvi[index + 3],
                    dvi[index + 4],
                ]) as usize;
                let start = index + 5;
                let end = start + len;
                if end <= dvi.len() {
                    payloads.push(dvi[start..end].to_vec());
                    index = end;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    payloads
}

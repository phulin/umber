#![allow(clippy::disallowed_methods)] // host-side fixture regeneration tool.

mod classic_bibtex;
mod cohort_transaction;
mod corpus_sync;
mod fixture_transaction;
mod fonts;
mod pdf;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use fixturegen::reference::{RefTex, RunOpts, generate_reference_fixture};
use tempfile::TempDir;
use test_support::{corpus_cases, corpus_root, fixture_path, normalize};
use tex_command::{
    CatcodeQueries, CharacterCode, CommandDialect, CommandProfile, CommandState,
    SourceRegistration, SourceToken, SourceTokenizationStep,
};
use tex_state::env::banks::IntParam;
use tex_state::token::Catcode;
use tex_state::{Universe, World};

const TEXT_AREAS: &[&str] = &[
    "lexer",
    "expand",
    "lexer_dynamic",
    "exec",
    "etex_exec",
    "typeset",
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
        Some("--classic-bibtex-differential") => classic_bibtex::run(&repo_root(), args.collect()),
        Some("--check-pdf-raster") => {
            ensure_no_extra_args(args)?;
            pdf::check_raster_attestations()
        }
        Some("--area") => {
            let area = args.next().context("missing area after --area")?;
            ensure_no_extra_args(args)?;
            regenerate_area(&area)
        }
        Some("--cohort-transaction") => cohort_transaction::run_cli(args.collect()),
        Some("--sync-corpus") => sync_corpus(args.collect()),
        Some("--reference-dvi") => publish_reference_dvi(args.collect()),
        Some("--seal-classic-bibtex-case") => {
            let root = args.next().context("missing classic case directory")?;
            let case = args.next().context("missing classic case ID")?;
            ensure_no_extra_args(args)?;
            fixture_transaction::seal_classic_case(Path::new(&root), &case)
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
        "usage: fixturegen --area AREA | --case AREA/CASE | --case AREA CASE | --cohort-transaction (--plan|--apply) PLAN.json | --sync-corpus [--manifest PATH] [--dest PATH] [--offline] | --reference-dvi DOCUMENT OUTPUT | --check-pdf-raster\n\
         areas: lexer expand lexer_dynamic exec etex_exec typeset pdf fonts"
    );
}

fn publish_reference_dvi(args: Vec<String>) -> Result<()> {
    let [document, output] = args.as_slice() else {
        bail!("--reference-dvi requires DOCUMENT OUTPUT");
    };
    let repository = repo_root();
    let bytes = generate_reference_fixture(
        &repository,
        &repository.join("tests/corpus-manifest.txt"),
        &repository.join("third_party/corpus"),
        document,
    )?;
    let output = PathBuf::from(output);
    let tree = output
        .parent()
        .context("reference DVI output has no parent")?;
    let file = output
        .file_name()
        .map(PathBuf::from)
        .context("reference DVI output has no file name")?;
    let changed = fixture_transaction::publish_file_in_tree(
        &repository.join("tests/corpus"),
        tree,
        &file,
        bytes,
    )?;
    println!(
        "fixture {}: {}",
        if changed { "updated" } else { "unchanged" },
        output.display()
    );
    Ok(())
}

fn sync_corpus(args: Vec<String>) -> Result<()> {
    let mut options = corpus_sync::SyncOptions::default();
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--manifest" => {
                options.manifest_path = args
                    .next()
                    .map(PathBuf::from)
                    .context("missing path after --manifest")?;
            }
            "--dest" => {
                options.destination = args
                    .next()
                    .map(PathBuf::from)
                    .context("missing path after --dest")?;
            }
            "--offline" => options.offline = true,
            _ => bail!("unknown --sync-corpus option: {argument}"),
        }
    }
    for status in corpus_sync::run(&options)? {
        println!("{status}");
    }
    Ok(())
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(extra) = args.next() {
        bail!("unexpected extra argument: {extra}");
    }
    Ok(())
}

fn regenerate_area(area: &str) -> Result<()> {
    match area {
        "lexer" => regenerate_cases(area, |case| {
            regenerate_umber_dump_case(area, case, "lex-dump")
        }),
        "expand" => regenerate_cases(area, |case| {
            regenerate_umber_dump_case(area, case, "expand-dump")
        }),
        "lexer_dynamic" => regenerate_cases(area, regenerate_lexer_dynamic_case),
        "exec" => regenerate_cases(area, |case| {
            regenerate_reference_terminal_case(area, case, false)
        }),
        "etex_exec" => regenerate_cases(area, regenerate_etex_reference_log_case),
        "typeset" => regenerate_cases(area, |case| {
            regenerate_reference_terminal_case(area, case, true)
        }),
        "pdf" => pdf::regenerate_area(),
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
    if area == "pdf" {
        return pdf::regenerate_case(case);
    }
    if !TEXT_AREAS.contains(&area) {
        bail!("unknown fixturegen area: {area}");
    }
    match area {
        "lexer" => regenerate_umber_dump_case(area, case, "lex-dump"),
        "expand" => regenerate_umber_dump_case(area, case, "expand-dump"),
        "lexer_dynamic" => regenerate_lexer_dynamic_case(case),
        "exec" => regenerate_reference_terminal_case(area, case, false),
        "etex_exec" => regenerate_etex_reference_log_case(case),
        "typeset" => regenerate_reference_terminal_case(area, case, true),
        _ => unreachable!("known area already checked"),
    }
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

fn regenerate_reference_terminal_case(area: &str, case: &str, box_dump: bool) -> Result<()> {
    let source = initex_source(area, case)?;
    let output = RefTex::locate()?.run(
        &source.path,
        &RunOpts {
            ini: true,
            ..RunOpts::default()
        },
    )?;
    let actual = if box_dump {
        normalize::box_dump(&output.stdout)
    } else {
        normalize::exec_log(&output.stdout)
    };
    write_text_fixture(area, case, "terminal", &actual)
}

fn regenerate_etex_reference_log_case(case: &str) -> Result<()> {
    let area = "etex_exec";
    let source = initex_source(area, case)?;
    let support = corpus_root()
        .join(area)
        .join(case)
        .join(format!("{case}.txt"));
    let mut opts = RunOpts {
        ini: true,
        etex: true,
        ..RunOpts::default()
    };
    if support.exists() {
        opts.extra_inputs.push(support);
    }
    let output = RefTex::locate()?.run(&source.path, &opts)?;
    if !output.success {
        bail!("reference e-TeX failed for {area}/{case}:\n{}", output.log);
    }
    write_text_fixture(area, case, "log", &normalize::exec_log(&output.log))
}

/// A reference INITEX run needs the seven printable catcode assignments that
/// `umber run` installs without loading Plain; tex.web §232 itself leaves
/// these characters as `other_char`.
struct InitexSource {
    _directory: TempDir,
    path: PathBuf,
}

fn initex_source(area: &str, case: &str) -> Result<InitexSource> {
    const PLAIN_CATCODES: &[u8] =
        br"\catcode123=1 \catcode125=2 \catcode36=3 \catcode38=4 \catcode35=6 \catcode94=7 \catcode95=8 ";
    const CORPUS_FONT_PREFIX: &[u8] = b"../../crates/tex-fonts/tests/fixtures/cm/";

    let original = source_path(area, case);
    let file_name = original
        .file_name()
        .context("INITEX corpus source has no file name")?;
    let directory = TempDir::new().context("create INITEX corpus source directory")?;
    let path = directory.path().join(file_name);
    let source = fs::read(&original)
        .with_context(|| format!("failed to read INITEX corpus source {}", original.display()))?;
    let font_root = repo_root().join("crates/tex-fonts/tests/fixtures/cm");
    let mut absolute_font_prefix = font_root
        .to_str()
        .context("repository font fixture path is not UTF-8")?
        .as_bytes()
        .to_vec();
    absolute_font_prefix.push(b'/');
    let source = replace_bytes(&source, CORPUS_FONT_PREFIX, &absolute_font_prefix);
    let mut staged = Vec::with_capacity(PLAIN_CATCODES.len() + source.len());
    staged.extend_from_slice(PLAIN_CATCODES);
    staged.extend_from_slice(&source);
    fs::write(&path, staged).context("write INITEX corpus source")?;

    Ok(InitexSource {
        _directory: directory,
        path,
    })
}

fn replace_bytes(input: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut remaining = input;
    while let Some(index) = remaining
        .windows(needle.len())
        .position(|window| window == needle)
    {
        output.extend_from_slice(&remaining[..index]);
        output.extend_from_slice(replacement);
        remaining = &remaining[index + needle.len()..];
    }
    output.extend_from_slice(remaining);
    output
}

fn write_text_fixture(area: &str, case: &str, kind: &str, actual: &str) -> Result<()> {
    let path = fixture_path(area, case, kind);
    let unchanged = fs::read_to_string(&path).ok().as_deref() == Some(actual);
    if unchanged {
        eprintln!("fixture unchanged: {}", display_repo_path(&path));
        return Ok(());
    }
    if test_support::is_directory_case_area(area) {
        atomically_replace_case_output(area, case, &path, actual.as_bytes())?;
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create fixture directory {}", parent.display())
            })?;
        }
        fs::write(&path, actual).with_context(|| format!("failed to write {}", path.display()))?;
    }
    eprintln!("fixture updated: {}", display_repo_path(&path));
    Ok(())
}

fn atomically_replace_case_output(
    area: &str,
    case: &str,
    output: &Path,
    bytes: &[u8],
) -> Result<()> {
    let repository = repo_root();
    let current = corpus_root().join(area).join(case);
    let staging = TempDir::new_in(&repository).context("create repository-local candidate")?;
    let candidate = staging.path().join(case);
    test_support::closed_case::FixtureCase::discover_tracked(
        Path::new("tests/corpus").join(area).join(case),
        test_support::case_source_name(area, case),
        area,
    )?
    .stage_into(&candidate)?;
    let output_name = output
        .file_name()
        .context("fixture output has no file name")?;
    fs::write(candidate.join(output_name), bytes)?;
    let inventory = test_support::closed_case::StagedCase::validate(&candidate)?
        .inventory()
        .clone();
    fixture_transaction::publish_case_inventory(&corpus_root(), &current, inventory)
}

fn source_path(area: &str, case: &str) -> PathBuf {
    if test_support::is_directory_case_area(area) {
        corpus_root()
            .join(area)
            .join(case)
            .join(test_support::case_source_name(area, case))
    } else {
        corpus_root().join(area).join(format!("{case}.tex"))
    }
}

fn repo_root() -> PathBuf {
    test_support::repository_root()
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
        ".expected.terminal",
        ".expected.tokens",
        ".expected.ref",
        ".expected.out",
        ".expected.effects",
        ".expected.specials",
        ".expected.ref.pdf",
        ".expected.umber.pdf",
        ".expected.structure",
        ".expected.pgm",
        ".expected.render",
    ] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn historical_tex_exec_observations_have_no_fixturegen_mutator() {
        let area = super::regenerate_area("tex_exec").expect_err("area must be validation-only");
        assert!(area.to_string().contains("unknown fixturegen area"));

        let case = super::regenerate_case("tex_exec", "after")
            .expect_err("case must be validation-only");
        assert!(case.to_string().contains("unknown fixturegen area"));
    }
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
    let (mut state, mut stores) = lexer_fixture("invalid_character");
    stores.set_catcode('?', Catcode::Invalid);
    let mut actual = String::new();

    loop {
        match next_source_step(&mut state, &stores) {
            SourceTokenizationStep::Token(token) => push_token(&mut actual, token),
            SourceTokenizationStep::InvalidCharacter(invalid) => {
                let code = invalid
                    .code()
                    .to_byte()
                    .expect("exact-byte invalid character");
                actual.push_str(&format!(
                    "error:input contains invalid TeX character U+{code:04X}\n"
                ));
                break;
            }
            SourceTokenizationStep::End => break,
        }
    }

    actual
}

fn lexer_fixture(case: &str) -> (CommandState, Universe) {
    let path = source_path("lexer_dynamic", case);
    let mut stores = Universe::with_world(World::real());
    let content = stores
        .world_mut()
        .read_file(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut state = CommandState::new(CommandProfile::exact(CommandDialect::Tex82));
    let source = state
        .register_source(SourceRegistration::world(content))
        .expect("dynamic lexer fixture source should register");
    state
        .open_registered_source(source)
        .expect("dynamic lexer fixture source should open");
    (state, stores)
}

fn push_remaining_tokens(actual: &mut String, state: &mut CommandState, stores: &mut Universe) {
    loop {
        match next_source_step(state, stores) {
            SourceTokenizationStep::Token(token) => push_token(actual, token),
            SourceTokenizationStep::InvalidCharacter(invalid) => {
                panic!("dynamic lexer fixture contains invalid character: {invalid:?}")
            }
            SourceTokenizationStep::End => break,
        }
    }
}

fn push_next_token(actual: &mut String, state: &mut CommandState, stores: &mut Universe) {
    match next_source_step(state, stores) {
        SourceTokenizationStep::Token(token) => push_token(actual, token),
        SourceTokenizationStep::InvalidCharacter(invalid) => {
            panic!("dynamic lexer fixture contains invalid character: {invalid:?}")
        }
        SourceTokenizationStep::End => panic!("dynamic lexer fixture ended early"),
    }
}

fn next_source_step(state: &mut CommandState, stores: &Universe) -> SourceTokenizationStep {
    state.next_exact_source_step(
        stores.int_param(IntParam::END_LINE_CHAR),
        &mut CatcodeQueries(|code: CharacterCode| {
            stores.catcode(char::from(code.to_byte().expect("exact-byte source code")))
        }),
    )
}

fn push_token(actual: &mut String, token: SourceToken) {
    let line = match token {
        SourceToken::Character { code, catcode, .. } => format!(
            "char:{}:{}",
            code.to_byte().expect("exact-byte character token"),
            catcode as u8
        ),
        SourceToken::ControlSequence { name, .. } => format!(
            "cs:{}",
            name.iter()
                .copied()
                .map(|code| char::from(code.to_byte().expect("exact-byte control sequence")))
                .collect::<String>()
        ),
    };
    actual.push_str(&line);
    actual.push('\n');
}

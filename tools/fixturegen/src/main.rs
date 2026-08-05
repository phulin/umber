#![allow(clippy::disallowed_methods)] // host-side fixture regeneration tool.

mod classic_bibtex;
mod cohort_transaction;
mod corpus_sync;
mod fonts;
mod layout_migration;
mod pdf;
mod pdf_layout_migration;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use refexec::{RefTex, RunOpts, RunOutput};
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
    "hello",
    "lexer",
    "expand",
    "lexer_dynamic",
    "exec",
    "etex_exec",
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
        Some("--migrate-layout") => {
            let mode = match args.next().as_deref() {
                Some("--plan") => layout_migration::Mode::Plan,
                Some("--apply") => layout_migration::Mode::Apply,
                _ => bail!("--migrate-layout requires --plan or --apply"),
            };
            ensure_no_extra_args(args)?;
            let report =
                layout_migration::run(&corpus_root(), layout_migration::ALL_FAMILIES, mode)?;
            print!("{report}");
            Ok(())
        }
        Some("--migrate-pdf-layout") => pdf_layout_migration::run_cli(args.collect()),
        Some("--cohort-transaction") => cohort_transaction::run_cli(args.collect()),
        Some("--sync-corpus") => sync_corpus(args.collect()),
        Some("--reference-dvi") => publish_reference_dvi(args.collect()),
        Some("--seal-classic-bibtex-case") => {
            let root = args.next().context("missing classic case directory")?;
            let case = args.next().context("missing classic case ID")?;
            ensure_no_extra_args(args)?;
            layout_migration::seal_classic_case(Path::new(&root), &case)
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
        "usage: fixturegen --area AREA | --case AREA/CASE | --case AREA CASE | --migrate-layout (--plan|--apply) | --migrate-pdf-layout (--plan|--apply) | --cohort-transaction (--plan|--apply) PLAN.json | --sync-corpus [--manifest PATH] [--dest PATH] [--offline] | --reference-dvi DOCUMENT OUTPUT | --check-pdf-raster\n\
         areas: hello lexer expand lexer_dynamic exec etex_exec typeset tex_exec tex_exec_io pdf fonts"
    );
}

fn publish_reference_dvi(args: Vec<String>) -> Result<()> {
    let [document, output] = args.as_slice() else {
        bail!("--reference-dvi requires DOCUMENT OUTPUT");
    };
    let repository = repo_root();
    let bytes = parity_harness::generate_reference_fixture(
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
    let changed = layout_migration::publish_file_in_tree(
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
        "etex_exec" => regenerate_cases(area, regenerate_etex_reference_log_case),
        "typeset" => regenerate_cases(area, |case| regenerate_reference_log_case(area, case, true)),
        "tex_exec" => regenerate_cases(area, regenerate_tex_exec_case),
        "tex_exec_io" => regenerate_cases(area, regenerate_tex_exec_io_case),
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
        "hello" => regenerate_hello_case(case),
        "lexer" => regenerate_umber_dump_case(area, case, "lex-dump"),
        "expand" => regenerate_umber_dump_case(area, case, "expand-dump"),
        "lexer_dynamic" => regenerate_lexer_dynamic_case(case),
        "exec" => regenerate_reference_log_case(area, case, false),
        "etex_exec" => regenerate_etex_reference_log_case(case),
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
    let source = initex_source(area, case)?;
    let output = RefTex::locate()?.run(
        &source.path,
        &RunOpts {
            ini: true,
            ..RunOpts::default()
        },
    )?;
    let actual = if box_dump {
        normalize::box_dump(&output.log)
    } else {
        normalize::exec_log(&output.log)
    };
    write_text_fixture(area, case, "log", &actual)
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

fn regenerate_tex_exec_case(case: &str) -> Result<()> {
    let mut opts = RunOpts::default();
    let generated_images = if case == "pdf_ximage_enquiries" {
        Some(ximage_enquiry_inputs()?)
    } else {
        None
    };
    if let Some(inputs) = &generated_images {
        opts.extra_inputs.extend(inputs.paths.iter().cloned());
    }
    if matches!(
        case,
        "pdf_output_policy"
            | "pdf_image_config"
            | "pdf_metadata_config"
            | "pdf_font_config"
            | "pdf_microtype_effects"
            | "pdf_form_state"
            | "pdf_form_diagnostics"
            | "pdf_form_traversal_diagnostics"
            | "pdf_compatibility_controls"
            | "pdf_move_chars_warning"
            | "pdf_ignored_dimen_effects"
            | "pdf_navigation_dest_scan"
            | "pdf_navigation_dest_lifecycle"
            | "pdf_navigation_outline_scan"
            | "pdf_navigation_outline_tree"
            | "pdf_navigation_thread_scan"
            | "pdf_navigation_thread_lifecycle"
            | "pdf_navigation_thread_graph"
            | "pdf_ximage_enquiries"
    ) {
        opts.ini = true;
    }
    if case == "pdf_compatibility_controls" {
        opts.etex = true;
    }
    let output = RefTex::locate()?.run(&source_path("tex_exec", case), &opts)?;
    write_text_fixture("tex_exec", case, "ref", &format_micro_reference(&output))
}

struct GeneratedXImageInputs {
    _directory: TempDir,
    paths: Vec<PathBuf>,
}

fn ximage_enquiry_inputs() -> Result<GeneratedXImageInputs> {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64,
        0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    const JPEG: &[u8] = &[
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
    ];

    let directory = TempDir::new().context("create ximage enquiry inputs")?;
    let png = directory.path().join("depth8.png");
    let jpeg = directory.path().join("depth12.jpg");
    let pdf_path = directory.path().join("three-pages.pdf");
    fs::write(&png, PNG).context("write generated PNG enquiry input")?;
    fs::write(&jpeg, JPEG).context("write generated JPEG enquiry input")?;

    let catalog = pdf_writer::Ref::new(1);
    let pages = pdf_writer::Ref::new(2);
    let page_ids = [
        pdf_writer::Ref::new(3),
        pdf_writer::Ref::new(4),
        pdf_writer::Ref::new(5),
    ];
    let mut pdf = pdf_writer::Pdf::new();
    pdf.catalog(catalog).pages(pages);
    pdf.pages(pages).kids(page_ids).count(3);
    for page in page_ids {
        pdf.page(page)
            .parent(pages)
            .media_box(pdf_writer::Rect::new(0.0, 0.0, 10.0, 20.0))
            .resources();
    }
    fs::write(&pdf_path, pdf.finish()).context("write typed three-page PDF enquiry input")?;

    Ok(GeneratedXImageInputs {
        _directory: directory,
        paths: vec![png, jpeg, pdf_path],
    })
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
    let current = corpus_root().join(area).join(case);
    let mut inventory = std::collections::BTreeMap::new();
    for entry in fs::read_dir(&current).with_context(|| format!("read {}", current.display()))? {
        let entry = entry.context("read case entry")?;
        let kind = entry.file_type().context("read case entry type")?;
        if !kind.is_file() || kind.is_symlink() {
            bail!("case contains non-regular entry {}", entry.path().display());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("case contains a non-UTF-8 file name"))?;
        inventory.insert(name, fs::read(entry.path())?);
    }
    let output_name = output
        .file_name()
        .context("fixture output has no file name")?;
    inventory.insert(output_name.to_string_lossy().into_owned(), bytes.to_vec());
    layout_migration::publish_case_inventory(&corpus_root(), &current, inventory)
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
            name.iter().copied()
                .map(|code| char::from(code.to_byte().expect("exact-byte control sequence")))
                .collect::<String>()
        ),
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

#![allow(clippy::disallowed_methods)] // Host-side parity runner and triage writer.

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, anyhow, bail};
use corpus_manifest::{Document, Manifest, parse_manifest_file};
use refexec::{RefTex, RunOpts, normalized_dvi_for_comparison};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tex_out::dvi::disasm::DviFile;

const TRACE_PREFIX: &str =
    "\\tracingoutput=1 \\tracingonline=0 \\showboxbreadth=-1 \\showboxdepth=-1\n";
const JOB_NAME: &str = "parity-job.tex";
const CORPUS_TFMS: &[&str] = &[
    "cmbsy10", "cmbx10", "cmbx5", "cmbx6", "cmbx7", "cmbx8", "cmbx9", "cmcsc10", "cmdunh10",
    "cmex10", "cmmi10", "cmmi5", "cmmi6", "cmmi7", "cmmi8", "cmmi9", "cmmib10", "cmr10", "cmr5",
    "cmr6", "cmr7", "cmr8", "cmr9", "cmsl10", "cmsl8", "cmsl9", "cmsltt10", "cmss10", "cmssbx10",
    "cmssi10", "cmssq8", "cmssqi8", "cmsy10", "cmsy5", "cmsy6", "cmsy7", "cmsy8", "cmsy9",
    "cmti10", "cmti7", "cmti8", "cmti9", "cmtt10", "cmtt8", "cmtt9", "cmu10", "manfnt",
];

fn main() -> ExitCode {
    match run_cli() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<bool> {
    let options = Options::parse(env::args_os().skip(1))?;
    if options.self_test {
        run_self_test(&options.triage_dir)?;
        return Ok(true);
    }
    run_e2e(&options)
}

#[derive(Clone, Debug)]
struct Options {
    manifest_path: PathBuf,
    corpus_dir: PathBuf,
    triage_dir: PathBuf,
    umber_bin: PathBuf,
    doc_filter: Option<String>,
    keep_triage: bool,
    self_test: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = OsString>) -> Result<Self> {
        let mut options = Self {
            manifest_path: PathBuf::from("tests/corpus-manifest.txt"),
            corpus_dir: PathBuf::from("third_party/corpus"),
            triage_dir: PathBuf::from("target/parity-triage"),
            umber_bin: PathBuf::from("target/debug/umber"),
            doc_filter: None,
            keep_triage: false,
            self_test: false,
        };

        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--manifest") => {
                    options.manifest_path = next_path(&mut args, "--manifest")?;
                }
                Some("--corpus-dir") => {
                    options.corpus_dir = next_path(&mut args, "--corpus-dir")?;
                }
                Some("--triage-dir") => {
                    options.triage_dir = next_path(&mut args, "--triage-dir")?;
                }
                Some("--umber-bin") => {
                    options.umber_bin = next_path(&mut args, "--umber-bin")?;
                }
                Some("--doc") => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("missing value after --doc"))?;
                    options.doc_filter = Some(value.to_string_lossy().into_owned());
                }
                Some("--keep-triage") => options.keep_triage = true,
                Some("--self-test") => options.self_test = true,
                Some("--help") | Some("-h") => {
                    print_usage();
                    std::process::exit(0);
                }
                Some(flag) if flag.starts_with('-') => bail!("unknown option: {flag}"),
                _ => bail!("unexpected positional argument: {}", arg.to_string_lossy()),
            }
        }
        Ok(options)
    }
}

fn next_path(args: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<PathBuf> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing path after {flag}"))
}

#[derive(Debug)]
struct EngineDvi {
    bytes: Vec<u8>,
    normalized: Vec<u8>,
}

#[derive(Debug)]
struct UmberRun {
    success: bool,
    stdout: String,
    stderr: String,
    dvi: Option<Vec<u8>>,
}

#[derive(Debug)]
struct TraceRun {
    log: String,
    dvi: Option<Vec<u8>>,
    success: bool,
}

#[derive(Debug)]
struct TriageInput<'a> {
    doc: &'a Document,
    source_path: &'a Path,
    reference: Option<&'a EngineDvi>,
    umber: Option<&'a EngineDvi>,
    diff: Option<DviByteDiff>,
    reference_drift: Option<ReferenceDrift>,
    umber_failure: Option<&'a UmberRun>,
    trace: Option<TraceBundle>,
}

#[derive(Clone, Debug)]
struct DviByteDiff {
    offset: usize,
    reference_context: String,
    umber_context: String,
}

#[derive(Clone, Debug)]
struct ReferenceDrift {
    expected: String,
    actual: String,
}

#[derive(Debug)]
struct TraceBundle {
    reference: TraceRun,
    umber: TraceRun,
    reference_stable: Option<bool>,
    umber_stable: Option<bool>,
}

fn run_e2e(options: &Options) -> Result<bool> {
    let manifest = read_manifest(&options.manifest_path)?;
    if !options.keep_triage && options.triage_dir.exists() {
        fs::remove_dir_all(&options.triage_dir).with_context(|| {
            format!(
                "failed to remove old triage dir {}",
                options.triage_dir.display()
            )
        })?;
    }

    let ref_tex = RefTex::locate()?;
    let mut ok = true;
    for doc in manifest.doc.iter().filter(|doc| {
        options
            .doc_filter
            .as_ref()
            .is_none_or(|filter| filter == &doc.name)
    }) {
        let source_path = options.corpus_dir.join(&doc.name);
        let format_source_path = options.corpus_dir.join(&doc.format_source);
        println!("e2e {}", doc.name);
        let reference = run_reference_dvi(&ref_tex, &source_path, &format_source_path)
            .with_context(|| format!("reference TeX failed for {}", doc.name))?;
        let reference_hash = sha256_hex(&reference.normalized);
        if reference_hash != doc.expected_ref_dvi_sha256 {
            ok = false;
            let drift = ReferenceDrift {
                expected: doc.expected_ref_dvi_sha256.clone(),
                actual: reference_hash,
            };
            write_triage_bundle(
                &options.triage_dir,
                &TriageInput {
                    doc,
                    source_path: &source_path,
                    reference: Some(&reference),
                    umber: None,
                    diff: None,
                    reference_drift: Some(drift),
                    umber_failure: None,
                    trace: None,
                },
            )?;
            continue;
        }

        let umber_run = run_umber_dvi(&options.umber_bin, &source_path, &format_source_path)
            .with_context(|| format!("umber run failed to start for {}", doc.name))?;
        if !umber_run.success {
            ok = false;
            write_triage_bundle(
                &options.triage_dir,
                &TriageInput {
                    doc,
                    source_path: &source_path,
                    reference: Some(&reference),
                    umber: None,
                    diff: None,
                    reference_drift: None,
                    umber_failure: Some(&umber_run),
                    trace: None,
                },
            )?;
            continue;
        }
        let Some(umber_bytes) = umber_run.dvi.as_deref() else {
            ok = false;
            write_triage_bundle(
                &options.triage_dir,
                &TriageInput {
                    doc,
                    source_path: &source_path,
                    reference: Some(&reference),
                    umber: None,
                    diff: None,
                    reference_drift: None,
                    umber_failure: Some(&umber_run),
                    trace: None,
                },
            )?;
            continue;
        };
        let umber = EngineDvi {
            bytes: umber_bytes.to_vec(),
            normalized: normalized_dvi_for_comparison(umber_bytes)?,
        };

        if reference.normalized != umber.normalized {
            ok = false;
            let diff = first_diff(&reference.normalized, &umber.normalized);
            let trace = run_trace_bundle(
                &ref_tex,
                &options.umber_bin,
                &source_path,
                &format_source_path,
                &reference,
                &umber,
            )
            .with_context(|| format!("failed to capture tracing output for {}", doc.name))?;
            write_triage_bundle(
                &options.triage_dir,
                &TriageInput {
                    doc,
                    source_path: &source_path,
                    reference: Some(&reference),
                    umber: Some(&umber),
                    diff: Some(diff),
                    reference_drift: None,
                    umber_failure: None,
                    trace: Some(trace),
                },
            )?;
        }
    }

    if ok {
        println!("e2e parity passed");
    } else {
        eprintln!("e2e parity failed; see {}", options.triage_dir.display());
    }
    Ok(ok)
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let manifest =
        parse_manifest_file(path).with_context(|| format!("failed to parse {}", path.display()))?;
    if manifest.doc.is_empty() {
        bail!(
            "manifest {} does not contain any doc entries",
            path.display()
        );
    }
    Ok(manifest)
}

fn run_reference_dvi(
    ref_tex: &RefTex,
    source_path: &Path,
    format_source_path: &Path,
) -> Result<EngineDvi> {
    let temp = staged_source_dir(source_path, format_source_path, false)?;
    let output = ref_tex.run_in_dir(
        temp.path(),
        Path::new(JOB_NAME),
        &RunOpts {
            dvi: true,
            ini: true,
            extra_inputs: Vec::new(),
        },
    )?;
    let bytes = output
        .dvi
        .ok_or_else(|| anyhow!("reference TeX did not produce DVI\n{}", output.log))?;
    let normalized = normalized_dvi_for_comparison(&bytes)?;
    Ok(EngineDvi { bytes, normalized })
}

fn run_umber_dvi(
    umber_bin: &Path,
    source_path: &Path,
    format_source_path: &Path,
) -> Result<UmberRun> {
    let temp = staged_source_dir(source_path, format_source_path, false)?;
    let dvi_path = temp.path().join("umber.dvi");
    let umber_bin = runnable_umber_bin(umber_bin)?;
    let output = Command::new(&umber_bin)
        .env("SOURCE_DATE_EPOCH", "1783604160")
        .current_dir(temp.path())
        .arg("run")
        .arg(JOB_NAME)
        .arg("--dvi")
        .arg(&dvi_path)
        .output()
        .with_context(|| format!("failed to execute {}", umber_bin.display()))?;
    let dvi = if dvi_path.exists() {
        Some(
            fs::read(&dvi_path)
                .with_context(|| format!("failed to read {}", dvi_path.display()))?,
        )
    } else {
        None
    };
    Ok(UmberRun {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dvi,
    })
}

fn run_trace_bundle(
    ref_tex: &RefTex,
    umber_bin: &Path,
    source_path: &Path,
    format_source_path: &Path,
    reference: &EngineDvi,
    umber: &EngineDvi,
) -> Result<TraceBundle> {
    let reference_trace = run_reference_trace(ref_tex, source_path, format_source_path)?;
    let umber_trace = run_umber_trace(umber_bin, source_path, format_source_path)?;
    let reference_stable = trace_stability(reference_trace.dvi.as_deref(), &reference.normalized)?;
    let umber_stable = trace_stability(umber_trace.dvi.as_deref(), &umber.normalized)?;
    Ok(TraceBundle {
        reference: reference_trace,
        umber: umber_trace,
        reference_stable,
        umber_stable,
    })
}

fn trace_stability(traced: Option<&[u8]>, normal: &[u8]) -> Result<Option<bool>> {
    let Some(traced) = traced else {
        return Ok(None);
    };
    Ok(Some(normalized_dvi_for_comparison(traced)? == normal))
}

fn run_reference_trace(
    ref_tex: &RefTex,
    source_path: &Path,
    format_source_path: &Path,
) -> Result<TraceRun> {
    let temp = staged_source_dir(source_path, format_source_path, true)?;
    let output = ref_tex.run_in_dir(
        temp.path(),
        Path::new(JOB_NAME),
        &RunOpts {
            dvi: true,
            ini: true,
            extra_inputs: Vec::new(),
        },
    )?;
    Ok(TraceRun {
        log: output.log,
        dvi: output.dvi,
        success: output.success,
    })
}

fn run_umber_trace(
    umber_bin: &Path,
    source_path: &Path,
    format_source_path: &Path,
) -> Result<TraceRun> {
    let temp = staged_source_dir(source_path, format_source_path, true)?;
    let dvi_path = temp.path().join("umber-trace.dvi");
    let umber_bin = runnable_umber_bin(umber_bin)?;
    let output = Command::new(&umber_bin)
        .env("SOURCE_DATE_EPOCH", "1783604160")
        .current_dir(temp.path())
        .arg("run")
        .arg(JOB_NAME)
        .arg("--show-fixtures")
        .arg("--dvi")
        .arg(&dvi_path)
        .output()
        .with_context(|| format!("failed to execute {}", umber_bin.display()))?;
    let dvi = if dvi_path.exists() {
        Some(
            fs::read(&dvi_path)
                .with_context(|| format!("failed to read {}", dvi_path.display()))?,
        )
    } else {
        None
    };
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        log.push_str("\n[stderr]\n");
        log.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(TraceRun {
        log,
        dvi,
        success: output.status.success(),
    })
}

fn runnable_umber_bin(umber_bin: &Path) -> Result<PathBuf> {
    fs::canonicalize(umber_bin)
        .with_context(|| format!("failed to resolve umber binary {}", umber_bin.display()))
}

fn copy_source(source_path: &Path, dest: &Path) -> Result<()> {
    let file_name = source_path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", source_path.display()))?;
    fs::copy(source_path, dest.join(file_name))
        .with_context(|| format!("failed to copy {}", source_path.display()))?;
    Ok(())
}

fn copy_corpus_tfms(dest: &Path) -> Result<()> {
    for name in CORPUS_TFMS {
        let target = dest.join(format!("{name}.tfm"));
        let source = locate_tfm(name)?
            .ok_or_else(|| anyhow!("could not locate required plain TeX font metric {name}.tfm"))?;
        fs::copy(&source, &target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn locate_tfm(name: &str) -> Result<Option<PathBuf>> {
    let local = PathBuf::from(format!("crates/tex-fonts/tests/fixtures/cm/{name}.tfm"));
    if local.exists() {
        return Ok(Some(local));
    }

    let output = Command::new("kpsewhich")
        .arg(format!("{name}.tfm"))
        .output();
    let Ok(output) = output else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path.is_empty() {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(path)))
    }
}

fn staged_source_dir(
    source_path: &Path,
    format_source_path: &Path,
    tracing: bool,
) -> Result<tempfile::TempDir> {
    let temp = tempfile::tempdir().context("failed to create parity job temp dir")?;
    copy_source(source_path, temp.path())?;
    copy_source(format_source_path, temp.path())?;
    let hyphen = Path::new("third_party/hyphen/hyphen.tex");
    if !hyphen.is_file() {
        bail!(
            "missing {}; run scripts/fetch-hyphen-corpus.sh before e2e parity",
            hyphen.display()
        );
    }
    copy_source(hyphen, temp.path())?;
    copy_corpus_tfms(temp.path())?;
    let file_name = source_path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", source_path.display()))?;
    let format_name = format_source_path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", format_source_path.display()))?;
    let mut wrapper = format!("\\input {}\n", format_name.to_string_lossy());
    if tracing {
        wrapper.push_str(TRACE_PREFIX);
    }
    writeln!(wrapper, "\\input {}", file_name.to_string_lossy())?;
    fs::write(temp.path().join(JOB_NAME), wrapper).context("failed to write parity job wrapper")?;
    Ok(temp)
}

fn write_triage_bundle(root: &Path, input: &TriageInput<'_>) -> Result<PathBuf> {
    let bundle = root.join(safe_bundle_name(&input.doc.name));
    if bundle.exists() {
        fs::remove_dir_all(&bundle)
            .with_context(|| format!("failed to replace {}", bundle.display()))?;
    }
    fs::create_dir_all(&bundle)
        .with_context(|| format!("failed to create {}", bundle.display()))?;

    if let Some(reference) = input.reference {
        fs::write(bundle.join("reference.dvi"), &reference.bytes)?;
    }
    if let Some(umber) = input.umber {
        fs::write(bundle.join("umber.dvi"), &umber.bytes)?;
    }
    if let Some(diff) = &input.diff {
        fs::write(bundle.join("byte-diff.txt"), byte_diff_text(diff))?;
        if let (Some(reference), Some(umber)) = (input.reference, input.umber) {
            write_page_disassembly(&bundle, reference, umber, diff.offset)?;
        }
    }
    if let Some(failure) = input.umber_failure {
        fs::write(bundle.join("umber.stdout.txt"), &failure.stdout)?;
        fs::write(bundle.join("umber.stderr.txt"), &failure.stderr)?;
    }
    if let Some(trace) = &input.trace {
        fs::write(bundle.join("reference-tracing.log"), &trace.reference.log)?;
        fs::write(bundle.join("umber-tracing.log"), &trace.umber.log)?;
        fs::write(
            bundle.join("tracing-verification.txt"),
            trace_verification(trace),
        )?;
    }
    fs::write(bundle.join("summary.txt"), summary_text(input)?)?;
    Ok(bundle)
}

fn write_page_disassembly(
    bundle: &Path,
    reference: &EngineDvi,
    umber: &EngineDvi,
    diff_offset: usize,
) -> Result<()> {
    let reference_file = DviFile::parse(&reference.normalized)?;
    let umber_file = DviFile::parse(&umber.normalized)?;
    let page_index = reference_file
        .page_for_offset(diff_offset)
        .map_or(0, |page| page.index);
    let reference_page = reference_file
        .disassemble_page(page_index)
        .with_context(|| format!("failed to disassemble reference page {}", page_index + 1))?;
    let umber_page = umber_file
        .disassemble_page(page_index)
        .or_else(|_| umber_file.disassemble_page(0))
        .context("failed to disassemble umber page")?;
    fs::write(bundle.join("reference-page.dvitype"), &reference_page)?;
    fs::write(bundle.join("umber-page.dvitype"), &umber_page)?;
    let diff = TextDiff::from_lines(&reference_page, &umber_page)
        .unified_diff()
        .header("reference-page.dvitype", "umber-page.dvitype")
        .to_string();
    fs::write(bundle.join("page-disassembly.diff"), diff)?;
    Ok(())
}

fn summary_text(input: &TriageInput<'_>) -> Result<String> {
    let mut out = String::new();
    writeln!(out, "document: {}", input.doc.name)?;
    writeln!(out, "source: {}", input.source_path.display())?;
    if let Some(drift) = &input.reference_drift {
        writeln!(out, "status: reference drift")?;
        writeln!(out, "expected_ref_dvi_sha256: {}", drift.expected)?;
        writeln!(out, "actual_ref_dvi_sha256: {}", drift.actual)?;
        return Ok(out);
    }
    if let Some(failure) = input.umber_failure {
        writeln!(out, "status: umber failed")?;
        writeln!(out, "umber_success: {}", failure.success)?;
        writeln!(out, "umber_dvi_written: {}", failure.dvi.is_some())?;
        return Ok(out);
    }
    let Some(diff) = &input.diff else {
        writeln!(out, "status: unknown")?;
        return Ok(out);
    };
    writeln!(out, "status: dvi mismatch")?;
    writeln!(out, "first_divergent_byte_offset: {}", diff.offset)?;
    if let (Some(reference), Some(umber)) = (input.reference, input.umber) {
        let (page, reference_opcode, umber_opcode) =
            divergent_page_and_opcodes(reference, umber, diff.offset)?;
        writeln!(out, "divergent_page: {page}")?;
        writeln!(out, "reference_opcode: {reference_opcode}")?;
        writeln!(out, "umber_opcode: {umber_opcode}")?;
    }
    if let Some(trace) = &input.trace {
        writeln!(out, "reference_trace_success: {}", trace.reference.success)?;
        writeln!(out, "umber_trace_success: {}", trace.umber.success)?;
        writeln!(
            out,
            "reference_tracing_preserves_dvi: {}",
            display_optional_bool(trace.reference_stable)
        )?;
        writeln!(
            out,
            "umber_tracing_preserves_dvi: {}",
            display_optional_bool(trace.umber_stable)
        )?;
    }
    Ok(out)
}

fn divergent_page_and_opcodes(
    reference: &EngineDvi,
    umber: &EngineDvi,
    offset: usize,
) -> Result<(usize, String, String)> {
    let reference_file = DviFile::parse(&reference.normalized)?;
    let umber_file = DviFile::parse(&umber.normalized)?;
    let page_index = reference_file
        .page_for_offset(offset)
        .map_or(0, |page| page.index);
    let reference_opcode = reference_file
        .command_at_or_before(page_index, offset)?
        .map_or_else(|| "unknown".to_string(), |command| command.name.to_string());
    let umber_opcode = umber_file
        .command_at_or_before(page_index, offset)
        .ok()
        .flatten()
        .map_or_else(|| "unknown".to_string(), |command| command.name.to_string());
    Ok((page_index + 1, reference_opcode, umber_opcode))
}

fn trace_verification(trace: &TraceBundle) -> String {
    format!(
        "reference_success: {}\number_success: {}\nreference_tracing_preserves_dvi: {}\number_tracing_preserves_dvi: {}\n",
        trace.reference.success,
        trace.umber.success,
        display_optional_bool(trace.reference_stable),
        display_optional_bool(trace.umber_stable)
    )
}

fn display_optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "no-dvi",
    }
}

fn first_diff(reference: &[u8], umber: &[u8]) -> DviByteDiff {
    let common = reference.len().min(umber.len());
    let offset = reference
        .iter()
        .zip(umber)
        .position(|(left, right)| left != right)
        .unwrap_or(common);
    DviByteDiff {
        offset,
        reference_context: hex_context(reference, offset),
        umber_context: hex_context(umber, offset),
    }
}

fn byte_diff_text(diff: &DviByteDiff) -> String {
    format!(
        "first divergent byte offset: {}\nreference: {}\number:     {}\n",
        diff.offset, diff.reference_context, diff.umber_context
    )
}

fn hex_context(bytes: &[u8], offset: usize) -> String {
    const WINDOW: usize = 12;
    let start = offset.saturating_sub(WINDOW);
    let end = bytes.len().min(offset.saturating_add(WINDOW + 1));
    let mut out = format!("{start:08x}:");
    for (index, byte) in bytes.iter().enumerate().take(end).skip(start) {
        if index == offset {
            let _ = write!(out, " [{byte:02x}]");
        } else {
            let _ = write!(out, " {byte:02x}");
        }
    }
    if offset >= bytes.len() {
        out.push_str(" [EOF]");
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn safe_bundle_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn run_self_test(triage_dir: &Path) -> Result<PathBuf> {
    let root = triage_dir.join("self-test");
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(&root)?;
    let reference = synthetic_two_page_dvi();
    let mut umber = reference.clone();
    let right4_opcode = synthetic_second_page_body_offset(&reference);
    umber[right4_opcode] = 160;
    let reference = EngineDvi {
        normalized: reference.clone(),
        bytes: reference,
    };
    let umber = EngineDvi {
        normalized: umber.clone(),
        bytes: umber,
    };
    let diff = first_diff(&reference.normalized, &umber.normalized);
    let doc = Document {
        name: "self-test.tex".to_string(),
        url: "https://example.invalid/self-test.tex".to_string(),
        sha256: sha256_hex(b"self-test"),
        license: "MIT".to_string(),
        redistributable: true,
        format_source: "plain.tex".to_string(),
        expected_ref_dvi_sha256: sha256_hex(&reference.normalized),
        notes: "synthetic self-test".to_string(),
    };
    write_triage_bundle(
        &root,
        &TriageInput {
            doc: &doc,
            source_path: Path::new("self-test.tex"),
            reference: Some(&reference),
            umber: Some(&umber),
            diff: Some(diff),
            reference_drift: None,
            umber_failure: None,
            trace: None,
        },
    )?;
    let summary = fs::read_to_string(root.join("self-test.tex").join("summary.txt"))?;
    if !(summary.contains("divergent_page: 2")
        && summary.contains("reference_opcode: right4")
        && summary.contains("umber_opcode: down4"))
    {
        bail!("self-test summary did not pinpoint page/opcode:\n{summary}");
    }
    let bundle = root.join("self-test.tex");
    println!("self-test bundle: {}", bundle.display());
    Ok(bundle)
}

fn synthetic_second_page_body_offset(bytes: &[u8]) -> usize {
    let file = DviFile::parse(bytes).expect("synthetic DVI parses");
    file.pages[1].bop_offset + 45
}

fn synthetic_two_page_dvi() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[247, 2]);
    bytes.extend_from_slice(&25_400_000i32.to_be_bytes());
    bytes.extend_from_slice(&473_628_672i32.to_be_bytes());
    bytes.extend_from_slice(&1000i32.to_be_bytes());
    bytes.push(4);
    bytes.extend_from_slice(b"test");
    let first_bop = bytes.len();
    synthetic_page(&mut bytes, 1, -1, &[]);
    let second_bop = bytes.len();
    synthetic_page(
        &mut bytes,
        2,
        i32::try_from(first_bop).expect("small synthetic offset"),
        &[146, 0, 0, 0, 42, 65],
    );
    let post = bytes.len();
    bytes.push(248);
    bytes.extend_from_slice(
        &i32::try_from(second_bop)
            .expect("small synthetic offset")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&25_400_000i32.to_be_bytes());
    bytes.extend_from_slice(&473_628_672i32.to_be_bytes());
    bytes.extend_from_slice(&1000i32.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&2u16.to_be_bytes());
    bytes.push(249);
    bytes.extend_from_slice(
        &u32::try_from(post)
            .expect("small synthetic offset")
            .to_be_bytes(),
    );
    bytes.push(2);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(223);
    }
    bytes.extend_from_slice(&[223, 223, 223, 223]);
    bytes
}

fn synthetic_page(bytes: &mut Vec<u8>, count0: i32, previous: i32, body: &[u8]) {
    bytes.push(139);
    bytes.extend_from_slice(&count0.to_be_bytes());
    for _ in 1..10 {
        bytes.extend_from_slice(&0i32.to_be_bytes());
    }
    bytes.extend_from_slice(&previous.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes.push(140);
}

fn print_usage() {
    eprintln!(
        "usage: parity-harness [--manifest path] [--corpus-dir dir] [--triage-dir dir] [--umber-bin path] [--doc name] [--keep-triage] [--self-test]"
    );
}

#[cfg(test)]
mod tests {
    use super::run_self_test;

    #[test]
    fn self_test_bundle_pinpoints_page_and_opcode() {
        let temp = tempfile::tempdir().expect("create temp dir");

        let bundle = run_self_test(temp.path()).expect("run self-test");

        let summary =
            std::fs::read_to_string(bundle.join("summary.txt")).expect("read self-test summary");
        assert!(summary.contains("divergent_page: 2"), "{summary}");
        assert!(summary.contains("reference_opcode: right4"), "{summary}");
        assert!(summary.contains("umber_opcode: down4"), "{summary}");
    }
}

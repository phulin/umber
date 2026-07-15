use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use test_support::{corpus_cases, corpus_root, pdf::normalize_structure};

use super::umber_bin;

const PDFTEX_VERSION: &str = "pdfTeX 3.141592653-2.6-1.40.27 (TeX Live 2025)";
const RENDERER_VERSION: &str = "pdftoppm version 25.08.0";
const RENDERER_ARGS: &[&str] = &["-r", "72", "-gray", "-singlefile"];

pub(super) fn regenerate_area() -> Result<()> {
    let cases = corpus_cases("pdf");
    if cases.is_empty() {
        bail!("no .tex cases found for area pdf");
    }
    for case in cases {
        regenerate_case(case.name())?;
    }
    Ok(())
}

pub(super) fn regenerate_case(case: &str) -> Result<()> {
    let source = corpus_root().join("pdf").join(format!("{case}.tex"));
    if !source.is_file() {
        bail!("missing PDF fixture source {}", source.display());
    }
    let pdftex = locate_tool("UMBER_REF_PDFTEX", "pdftex")?;
    require_version(&pdftex, "--version", PDFTEX_VERSION)?;
    let renderer = locate_tool("UMBER_PDF_RENDERER", "pdftoppm")?;
    require_version(&renderer, "-v", RENDERER_VERSION)?;

    let temp = TempDir::new().context("failed to create PDF fixture temp directory")?;
    let source_name = format!("{case}.tex");
    fs::copy(&source, temp.path().join(&source_name))
        .context("failed to stage PDF fixture source")?;

    let reference_pdf = temp.path().join(format!("{case}.pdf"));
    let reference = Command::new(&pdftex)
        .current_dir(temp.path())
        .args(["--ini", "-interaction=nonstopmode"])
        .arg(&source_name)
        .output()
        .context("failed to run pinned pdfTeX")?;
    if !reference.status.success() || !reference_pdf.is_file() {
        bail!(
            "pinned pdfTeX failed for pdf/{case}:\n{}",
            String::from_utf8_lossy(&reference.stdout)
        );
    }

    let umber_pdf = temp.path().join(format!("{case}.umber.pdf"));
    let actual = Command::new(umber_bin())
        .args(["run", "--pdftex", "--pdf"])
        .arg(&umber_pdf)
        .arg(temp.path().join(&source_name))
        .output()
        .context("failed to run Umber PDF fixture")?;
    if !actual.status.success() || !umber_pdf.is_file() {
        bail!(
            "Umber failed for pdf/{case}:\n{}",
            String::from_utf8_lossy(&actual.stderr)
        );
    }

    let reference_bytes = fs::read(&reference_pdf).context("failed to read reference PDF")?;
    let umber_bytes = fs::read(&umber_pdf).context("failed to read Umber PDF")?;
    let reference_structure = normalize_structure(&reference_bytes)?;
    let umber_structure = normalize_structure(&umber_bytes)?;
    if reference_structure != umber_structure {
        bail!(
            "normalized PDF structure mismatch for pdf/{case}:\nreference:\n{reference_structure}\nUmber:\n{umber_structure}"
        );
    }

    let reference_pgm = render(&renderer, &reference_pdf, temp.path().join("reference"))?;
    let umber_pgm = render(&renderer, &umber_pdf, temp.path().join("umber"))?;
    if reference_pgm != umber_pgm {
        bail!("rendered PDF pixels differ for pdf/{case}");
    }

    write_fixture(case, "ref.pdf", &reference_bytes)?;
    write_fixture(case, "umber.pdf", &umber_bytes)?;
    write_fixture(case, "structure", reference_structure.as_bytes())?;
    write_fixture(case, "pgm", &reference_pgm)?;
    let attestation = format!(
        "pdf-render-v1\nrenderer {RENDERER_VERSION}\narguments {}\ncomparison exact-gray-pixels\nreference-pdf-sha256 {}\number-pdf-sha256 {}\npgm-sha256 {}\n",
        RENDERER_ARGS.join(" "),
        digest(&reference_bytes),
        digest(&umber_bytes),
        digest(&reference_pgm),
    );
    write_fixture(case, "render", attestation.as_bytes())
}

fn locate_tool(variable: &str, fallback: &str) -> Result<PathBuf> {
    if let Some(path) = env::var_os(variable) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "{variable} does not name an executable file: {}",
            path.display()
        );
    }
    let path = env::var_os("PATH").context("PATH is not set")?;
    for directory in env::split_paths(&path) {
        let candidate = directory.join(fallback);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("could not locate {fallback}; set {variable}=/absolute/path/to/{fallback}")
}

fn require_version(tool: &Path, argument: &str, expected: &str) -> Result<()> {
    let output = Command::new(tool)
        .arg(argument)
        .output()
        .with_context(|| format!("failed to query {} version", tool.display()))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() || !combined.lines().any(|line| line.contains(expected)) {
        bail!(
            "{} must report {expected:?}; got {:?}",
            tool.display(),
            combined.lines().next().unwrap_or("")
        );
    }
    Ok(())
}

fn render(renderer: &Path, pdf: &Path, prefix: PathBuf) -> Result<Vec<u8>> {
    let status = Command::new(renderer)
        .args(RENDERER_ARGS)
        .arg(pdf)
        .arg(&prefix)
        .status()
        .context("failed to run pinned PDF renderer")?;
    if !status.success() {
        bail!("pinned PDF renderer failed for {}", pdf.display());
    }
    fs::read(prefix.with_extension("pgm")).context("renderer did not write PGM output")
}

fn write_fixture(case: &str, kind: &str, bytes: &[u8]) -> Result<()> {
    let path = corpus_root()
        .join("pdf")
        .join(format!("{case}.expected.{kind}"));
    if fs::read(&path).ok().as_deref() == Some(bytes) {
        eprintln!("fixture unchanged: {}", path.display());
        return Ok(());
    }
    fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;
    eprintln!("fixture updated: {}", path.display());
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        write!(output, "{byte:02x}").expect("writing into String cannot fail");
    }
    output
}

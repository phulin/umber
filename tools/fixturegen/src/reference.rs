// Deterministic reference TeX and TFtoPL execution owned by fixture publication.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use corpus_manifest::parse_manifest_file;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use test_support::dvi::normalized_dvi_for_comparison;

pub const REFERENCE_JOB_NAME: &str = "parity-job.tex";
const DEFAULT_SOURCE_DATE_EPOCH: &str = "1783604160";

/// TFM font metrics loaded by `plain.tex`'s preload block.
pub const PLAIN_PRELOAD_FONTS: &[&str] = &[
    "cmbsy10", "cmbx10", "cmbx5", "cmbx6", "cmbx7", "cmbx8", "cmbx9", "cmcsc10", "cmdunh10",
    "cmex10", "cmmi10", "cmmi5", "cmmi6", "cmmi7", "cmmi8", "cmmi9", "cmmib10", "cmr10", "cmr5",
    "cmr6", "cmr7", "cmr8", "cmr9", "cmsl10", "cmsl8", "cmsl9", "cmsltt10", "cmss10", "cmssbx10",
    "cmssi10", "cmssq8", "cmssqi8", "cmsy10", "cmsy5", "cmsy6", "cmsy7", "cmsy8", "cmsy9",
    "cmti10", "cmti7", "cmti8", "cmti9", "cmtt10", "cmtt8", "cmtt9", "cmu10", "manfnt",
];

#[derive(Debug, Clone)]
pub struct RefTex {
    executable: PathBuf,
    engine: TexEngine,
}

#[derive(Debug, Clone)]
pub struct RefTftopl {
    executable: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    pub dvi: bool,
    pub ini: bool,
    /// Enables e-TeX's extended primitive table for INITEX observations.
    pub etex: bool,
    pub extra_inputs: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub log: String,
    pub dvi: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TexEngine {
    PdfTex,
    Tex,
}

impl RefTex {
    pub fn locate() -> Result<Self> {
        if let Some(path) = env::var_os("UMBER_REF_TEX").filter(|value| !value.is_empty()) {
            return Ok(Self::from_executable(path));
        }
        if let Some(path) = find_on_path("pdftex") {
            return Ok(Self {
                executable: path,
                engine: TexEngine::PdfTex,
            });
        }
        if let Some(path) = find_on_path("tex") {
            return Ok(Self {
                executable: path,
                engine: TexEngine::Tex,
            });
        }
        Err(anyhow!(
            "could not locate reference TeX: set UMBER_REF_TEX or make pdftex/tex available on PATH"
        ))
    }

    pub fn from_executable(executable: impl Into<PathBuf>) -> Self {
        let executable = executable.into();
        let engine = infer_engine(&executable);
        Self { executable, engine }
    }

    pub fn run(&self, tex_file: &Path, opts: &RunOpts) -> Result<RunOutput> {
        let temp_dir = TempDir::new().context("failed to create temporary TeX run directory")?;
        let job_name = file_name(tex_file)?;
        fs::copy(tex_file, temp_dir.path().join(job_name)).with_context(|| {
            format!(
                "failed to copy TeX input {} into temporary run directory",
                tex_file.display()
            )
        })?;
        for extra_input in &opts.extra_inputs {
            let extra_name = file_name(extra_input)?;
            fs::copy(extra_input, temp_dir.path().join(extra_name)).with_context(|| {
                format!(
                    "failed to copy extra input {} into temporary run directory",
                    extra_input.display()
                )
            })?;
        }
        self.run_in_dir(temp_dir.path(), Path::new(job_name), opts)
    }

    pub fn run_in_dir(&self, dir: &Path, tex_file: &Path, opts: &RunOpts) -> Result<RunOutput> {
        let job_name = file_name(tex_file)?;
        let stem = tex_file
            .file_stem()
            .ok_or_else(|| anyhow!("TeX input has no file stem: {}", tex_file.display()))?;
        let mut command = Command::new(&self.executable);
        command.current_dir(dir).arg(if opts.dvi {
            "-interaction=batchmode"
        } else {
            "-interaction=nonstopmode"
        });
        if opts.dvi && self.engine == TexEngine::PdfTex {
            command.arg("-output-format=dvi");
        }
        if opts.ini {
            command.arg("-ini");
        }
        if opts.etex && self.engine == TexEngine::PdfTex {
            command.arg("-etex");
        }
        command
            .env("SOURCE_DATE_EPOCH", source_date_epoch())
            .env("FORCE_SOURCE_DATE", force_source_date())
            .arg(job_name);
        let output = command.output().with_context(|| {
            format!(
                "failed to execute reference TeX {}",
                self.executable.display()
            )
        })?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let log_path = dir.join(stem).with_extension("log");
        let log = fs::read_to_string(&log_path)
            .with_context(|| format!("failed to read reference TeX log {}", log_path.display()))?;
        let dvi = if opts.dvi {
            let dvi_path = dir.join(stem).with_extension("dvi");
            Some(fs::read(&dvi_path).with_context(|| {
                format!("failed to read reference TeX DVI {}", dvi_path.display())
            })?)
        } else {
            None
        };
        Ok(RunOutput {
            success: output.status.success(),
            stdout,
            log,
            dvi,
        })
    }
}

impl RefTftopl {
    pub fn locate() -> Result<Self> {
        if let Some(path) = env::var_os("UMBER_REF_TFTOPL").filter(|value| !value.is_empty()) {
            return Ok(Self {
                executable: path.into(),
            });
        }
        if let Some(path) = find_on_path("tftopl") {
            return Ok(Self { executable: path });
        }
        Err(anyhow!(
            "could not locate reference tftopl: set UMBER_REF_TFTOPL or make tftopl available on PATH"
        ))
    }

    pub fn from_executable(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub fn to_pl(&self, tfm_file: &Path) -> Result<String> {
        let output = Command::new(&self.executable)
            .arg("-charcode-format=octal")
            .arg(tfm_file)
            .output()
            .with_context(|| {
                format!(
                    "failed to execute reference tftopl {}",
                    self.executable.display()
                )
            })?;
        if !output.status.success() {
            return Err(anyhow!(
                "reference tftopl failed for {} with status {}\nstdout:\n{}\nstderr:\n{}",
                tfm_file.display(),
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Stages and runs the manifest document recipe used by publication and live compatibility.
pub fn run_reference_document(
    repo_root: &Path,
    ref_tex: &RefTex,
    source_path: &Path,
    format_source_path: &Path,
    tracing: bool,
) -> Result<RunOutput> {
    let temp = stage_reference_document(repo_root, source_path, format_source_path, tracing)?;
    ref_tex.run_in_dir(
        temp.path(),
        Path::new(REFERENCE_JOB_NAME),
        &RunOpts {
            dvi: true,
            ini: true,
            etex: false,
            extra_inputs: Vec::new(),
        },
    )
}

/// Generates and verifies one manifest-bound DVI for atomic fixture publication.
pub fn generate_reference_fixture(
    repo_root: &Path,
    manifest_path: &Path,
    corpus_dir: &Path,
    document: &str,
) -> Result<Vec<u8>> {
    let manifest = parse_manifest_file(manifest_path)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let doc = manifest
        .entries
        .iter()
        .find(|doc| doc.is_document() && doc.name == document)
        .ok_or_else(|| anyhow!("document {document} is not declared in the manifest"))?;
    let output = run_reference_document(
        repo_root,
        &RefTex::locate()?,
        &corpus_dir.join(&doc.name),
        &corpus_dir.join(&doc.format_source),
        false,
    )?;
    let bytes = output
        .dvi
        .ok_or_else(|| anyhow!("reference TeX did not produce DVI\n{}", output.log))?;
    let hash = sha256_hex(&normalized_dvi_for_comparison(&bytes)?);
    if hash != doc.expected_ref_dvi_sha256 {
        bail!(
            "reference DVI hash drift for {}: expected {}, got {hash}",
            doc.name,
            doc.expected_ref_dvi_sha256
        );
    }
    Ok(bytes)
}

pub fn stage_reference_document(
    repo_root: &Path,
    source_path: &Path,
    format_source_path: &Path,
    tracing: bool,
) -> Result<TempDir> {
    let temp = tempfile::tempdir().context("failed to create parity job temp dir")?;
    copy_source(source_path, temp.path())?;
    copy_source(format_source_path, temp.path())?;
    let hyphen = repo_root.join("third_party/hyphen/hyphen.tex");
    if !hyphen.is_file() {
        bail!(
            "missing {}; run python3 scripts/provision.py worktree . before e2e parity",
            hyphen.display()
        );
    }
    copy_source(&hyphen, temp.path())?;
    copy_corpus_tfms(repo_root, temp.path())?;
    let source_name = file_name(source_path)?;
    let format_name = file_name(format_source_path)?;
    let mut wrapper = format!("\\input {}\n", format_name.to_string_lossy());
    if tracing {
        wrapper.push_str(
            "\\tracingoutput=1 \\tracingonline=0 \\showboxbreadth=-1 \\showboxdepth=-1\n",
        );
    }
    wrapper.push_str(&format!("\\input {}\n", source_name.to_string_lossy()));
    fs::write(temp.path().join(REFERENCE_JOB_NAME), wrapper)
        .context("failed to write parity job wrapper")?;
    Ok(temp)
}

fn copy_corpus_tfms(repo_root: &Path, dest: &Path) -> Result<()> {
    for name in PLAIN_PRELOAD_FONTS {
        let target = dest.join(format!("{name}.tfm"));
        let source = locate_tfm(repo_root, name)?
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

pub fn locate_tfm(repo_root: &Path, name: &str) -> Result<Option<PathBuf>> {
    let local = repo_root.join(format!("crates/tex-fonts/tests/fixtures/cm/{name}.tfm"));
    if local.exists() {
        return Ok(Some(local));
    }
    let cached = repo_root.join(format!("third_party/fonts/{name}.tfm"));
    Ok(cached.exists().then_some(cached))
}

fn copy_source(source_path: &Path, dest: &Path) -> Result<()> {
    let name = file_name(source_path)?;
    fs::copy(source_path, dest.join(name))
        .with_context(|| format!("failed to copy {}", source_path.display()))?;
    Ok(())
}

fn file_name(path: &Path) -> Result<&std::ffi::OsStr> {
    path.file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", path.display()))
}

fn source_date_epoch() -> std::ffi::OsString {
    env::var_os("SOURCE_DATE_EPOCH")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SOURCE_DATE_EPOCH.into())
}

fn force_source_date() -> std::ffi::OsString {
    env::var_os("FORCE_SOURCE_DATE")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "1".into())
}

fn infer_engine(path: &Path) -> TexEngine {
    path.file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .map_or(TexEngine::PdfTex, |name| {
            if name.ends_with("tex") && !name.ends_with("pdftex") {
                TexEngine::Tex
            } else {
                TexEngine::PdfTex
            }
        })
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| is_executable_file(candidate))
    })
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    use std::fmt::Write as _;
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn executable(dir: &Path, name: &str, body: &str) -> Result<PathBuf> {
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}"))?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions)?;
        Ok(path)
    }

    #[test]
    fn tex_kernel_preserves_flags_environment_staging_outputs_and_status() -> Result<()> {
        let root = tempfile::tempdir()?;
        let tex = root.path().join("case.tex");
        let extra = root.path().join("extra.dat");
        fs::write(&tex, "\\end\n")?;
        fs::write(&extra, "extra\n")?;
        let script = executable(
            root.path(),
            "fake-pdftex",
            r#"
last=
for arg in "$@"; do last="$arg"; done
stem=${last%.tex}
printf 'args:'
printf ' <%s>' "$@"
printf '\nepoch=%s force=%s extra=%s\n' "$SOURCE_DATE_EPOCH" "$FORCE_SOURCE_DATE" "$(test -f extra.dat && printf yes || printf no)"
printf 'REFERENCE LOG\n' > "$stem.log"
printf 'DVI-BYTES' > "$stem.dvi"
case "$0" in *fail*) exit 7;; esac
"#,
        )?;
        let opts = RunOpts {
            dvi: true,
            ini: true,
            etex: true,
            extra_inputs: vec![extra],
        };
        let output = RefTex::from_executable(&script).run(&tex, &opts)?;
        assert!(output.success);
        assert!(output.stdout.contains("<-interaction=batchmode>"));
        assert!(output.stdout.contains("<-output-format=dvi>"));
        assert!(output.stdout.contains("<-ini>"));
        assert!(output.stdout.contains("<-etex>"));
        assert!(output.stdout.contains("extra=yes"));
        let epoch =
            env::var("SOURCE_DATE_EPOCH").unwrap_or_else(|_| DEFAULT_SOURCE_DATE_EPOCH.into());
        let force = env::var("FORCE_SOURCE_DATE").unwrap_or_else(|_| "1".into());
        assert!(
            output
                .stdout
                .contains(&format!("epoch={epoch} force={force}"))
        );
        assert_eq!(output.log, "REFERENCE LOG\n");
        assert_eq!(output.dvi.as_deref(), Some(b"DVI-BYTES".as_slice()));

        let failed_script = root.path().join("fake-pdftex-fail");
        fs::copy(&script, &failed_script)?;
        let mut permissions = fs::metadata(&failed_script)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&failed_script, permissions)?;
        let failed = RefTex::from_executable(failed_script).run(&tex, &opts)?;
        assert!(!failed.success);
        assert_eq!(failed.log, output.log);
        assert_eq!(failed.dvi, output.dvi);
        Ok(())
    }

    #[test]
    fn tex_engine_inference_keeps_classic_tex_flags() -> Result<()> {
        let root = tempfile::tempdir()?;
        let tex = root.path().join("case.tex");
        fs::write(&tex, "\\end\n")?;
        let script = executable(
            root.path(),
            "tex",
            r#"
last=
for arg in "$@"; do last="$arg"; done
stem=${last%.tex}
printf '%s\n' "$@"
: > "$stem.log"
: > "$stem.dvi"
"#,
        )?;
        let output = RefTex::from_executable(script).run(
            &tex,
            &RunOpts {
                dvi: true,
                etex: true,
                ..RunOpts::default()
            },
        )?;
        assert!(!output.stdout.contains("-output-format=dvi"));
        assert!(!output.stdout.contains("-etex"));
        Ok(())
    }

    #[test]
    fn tftopl_kernel_preserves_flag_output_and_failure_classification() -> Result<()> {
        let root = tempfile::tempdir()?;
        let tfm = root.path().join("font.tfm");
        fs::write(&tfm, b"TFM")?;
        let success = executable(
            root.path(),
            "fake-tftopl",
            "printf 'flag=%s file=%s\\n' \"$1\" \"$2\"\n",
        )?;
        assert_eq!(
            RefTftopl::from_executable(success).to_pl(&tfm)?,
            format!("flag=-charcode-format=octal file={}\n", tfm.display())
        );
        let failure = executable(
            root.path(),
            "fake-tftopl-fail",
            "printf 'OUT'\nprintf 'ERR' >&2\nexit 9\n",
        )?;
        let error = RefTftopl::from_executable(failure)
            .to_pl(&tfm)
            .expect_err("nonzero TFtoPL status must fail")
            .to_string();
        assert!(error.contains("status exit status: 9"));
        assert!(error.contains("stdout:\nOUT"));
        assert!(error.contains("stderr:\nERR"));
        Ok(())
    }

    #[test]
    fn document_staging_is_closed_and_wrapper_is_exact() -> Result<()> {
        let root = tempfile::tempdir()?;
        let corpus = root.path().join("corpus");
        fs::create_dir_all(&corpus)?;
        fs::write(corpus.join("story.tex"), "story\n")?;
        fs::write(corpus.join("plain.tex"), "plain\n")?;
        fs::create_dir_all(root.path().join("third_party/hyphen"))?;
        fs::write(
            root.path().join("third_party/hyphen/hyphen.tex"),
            "hyphen\n",
        )?;
        let fonts = root.path().join("third_party/fonts");
        fs::create_dir_all(&fonts)?;
        for font in PLAIN_PRELOAD_FONTS {
            fs::write(fonts.join(format!("{font}.tfm")), font)?;
        }

        let staged = stage_reference_document(
            root.path(),
            &corpus.join("story.tex"),
            &corpus.join("plain.tex"),
            true,
        )?;
        assert_eq!(
            fs::read_to_string(staged.path().join("story.tex"))?,
            "story\n"
        );
        assert_eq!(
            fs::read_to_string(staged.path().join("plain.tex"))?,
            "plain\n"
        );
        assert_eq!(
            fs::read_to_string(staged.path().join("hyphen.tex"))?,
            "hyphen\n"
        );
        assert_eq!(
            fs::read_to_string(staged.path().join(REFERENCE_JOB_NAME))?,
            "\\input plain.tex\n\\tracingoutput=1 \\tracingonline=0 \\showboxbreadth=-1 \\showboxdepth=-1\n\\input story.tex\n"
        );
        for font in PLAIN_PRELOAD_FONTS {
            assert!(staged.path().join(format!("{font}.tfm")).is_file());
        }
        Ok(())
    }
}

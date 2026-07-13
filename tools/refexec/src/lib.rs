//! Reference TeX execution helper for parity tests.

#[allow(clippy::disallowed_methods)] // host tool, not engine code
mod imp {
    use std::env;
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::{Context, Result, anyhow};
    use tempfile::TempDir;

    const DEFAULT_SOURCE_DATE_EPOCH: &str = "1783604160";

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
        pub extra_inputs: Vec<PathBuf>,
    }

    #[derive(Debug, Clone)]
    pub struct RunOutput {
        pub success: bool,
        pub stdout: String,
        pub log: String,
        pub dvi: Option<Vec<u8>>,
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum DviComparison {
        Equal,
        Different(DviDiff),
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct DviDiff {
        pub offset: usize,
        pub expected_context: String,
        pub actual_context: String,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TexEngine {
        PdfTex,
        Tex,
    }

    impl RefTex {
        pub fn locate() -> Result<Self> {
            if let Some(path) = env::var_os("UMBER_REF_TEX").filter(|value| !value.is_empty()) {
                let path = PathBuf::from(path);
                let engine = infer_engine(&path);
                return Ok(Self {
                    executable: path,
                    engine,
                });
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

        pub fn run(&self, tex_file: &Path, opts: &RunOpts) -> Result<RunOutput> {
            let temp_dir =
                TempDir::new().context("failed to create temporary TeX run directory")?;
            let job_name = file_name(tex_file)?;
            let temp_tex_file = temp_dir.path().join(job_name);

            fs::copy(tex_file, &temp_tex_file).with_context(|| {
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
            command.env("SOURCE_DATE_EPOCH", source_date_epoch());
            command.env("FORCE_SOURCE_DATE", force_source_date());
            command.arg(job_name);

            let output = command.output().with_context(|| {
                format!(
                    "failed to execute reference TeX {}",
                    self.executable.display()
                )
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let log_path = dir.join(stem).with_extension("log");
            let log = fs::read_to_string(&log_path).with_context(|| {
                format!("failed to read reference TeX log {}", log_path.display())
            })?;
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

        pub fn compare_dvi(
            &self,
            tex_file: &Path,
            actual: &[u8],
            opts: &RunOpts,
        ) -> Result<DviComparison> {
            let mut opts = opts.clone();
            opts.dvi = true;
            let output = self.run(tex_file, &opts)?;
            if !output.success {
                return Err(anyhow!(
                    "reference TeX failed for {}\n{}",
                    tex_file.display(),
                    output.log
                ));
            }
            let expected = output
                .dvi
                .ok_or_else(|| anyhow!("reference TeX did not produce a DVI"))?;
            compare_dvi_bytes(&expected, actual)
        }
    }

    impl RefTftopl {
        pub fn locate() -> Result<Self> {
            if let Some(path) = env::var_os("UMBER_REF_TFTOPL").filter(|value| !value.is_empty()) {
                return Ok(Self {
                    executable: PathBuf::from(path),
                });
            }

            if let Some(path) = find_on_path("tftopl") {
                return Ok(Self { executable: path });
            }

            Err(anyhow!(
                "could not locate reference tftopl: set UMBER_REF_TFTOPL or make tftopl available on PATH"
            ))
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

    pub fn compare_dvi_bytes(expected: &[u8], actual: &[u8]) -> Result<DviComparison> {
        let expected = normalized_dvi_for_comparison(expected)?;
        let actual = normalized_dvi_for_comparison(actual)?;
        if expected == actual {
            return Ok(DviComparison::Equal);
        }
        Ok(DviComparison::Different(first_dvi_diff(&expected, &actual)))
    }

    pub fn normalized_dvi_for_comparison(bytes: &[u8]) -> Result<Vec<u8>> {
        let mut normalized = bytes.to_vec();
        normalize_dvi_preamble_comment(&mut normalized)?;
        Ok(normalized)
    }

    fn normalize_dvi_preamble_comment(bytes: &mut [u8]) -> Result<()> {
        const PRE: u8 = 247;
        const COMMENT_LEN_OFFSET: usize = 14;
        const COMMENT_OFFSET: usize = 15;
        const NORMALIZED_COMMENT: &[u8] = b"umber normalized dvi banner";

        if bytes.first() != Some(&PRE) || bytes.len() <= COMMENT_LEN_OFFSET {
            return Err(anyhow!("DVI is missing a valid preamble"));
        }
        let len = bytes[COMMENT_LEN_OFFSET] as usize;
        let end = COMMENT_OFFSET
            .checked_add(len)
            .ok_or_else(|| anyhow!("DVI preamble comment length overflowed"))?;
        if end > bytes.len() {
            return Err(anyhow!("DVI preamble comment is truncated"));
        }

        // The DVI preamble comment is the only sanctioned normalization in the
        // DVI parity harness. The reference banner contains engine/date text,
        // while Umber records its own job banner. We deliberately overwrite
        // exactly the existing k-length comment payload in both files; every
        // other byte, including the k length itself and all DVI pointers, must
        // already match.
        for (index, byte) in bytes[COMMENT_OFFSET..end].iter_mut().enumerate() {
            *byte = NORMALIZED_COMMENT.get(index).copied().unwrap_or(b' ');
        }
        Ok(())
    }

    fn first_dvi_diff(expected: &[u8], actual: &[u8]) -> DviDiff {
        let common = expected.len().min(actual.len());
        let offset = expected
            .iter()
            .zip(actual)
            .position(|(left, right)| left != right)
            .unwrap_or(common);
        DviDiff {
            offset,
            expected_context: hex_context(expected, offset),
            actual_context: hex_context(actual, offset),
        }
    }

    fn hex_context(bytes: &[u8], offset: usize) -> String {
        const WINDOW: usize = 12;
        let start = offset.saturating_sub(WINDOW);
        let end = bytes.len().min(offset.saturating_add(WINDOW + 1));
        let mut out = format!("{start:08x}:");
        for (index, byte) in bytes.iter().enumerate().take(end).skip(start) {
            if index == offset {
                out.push_str(" [");
                let _ = write!(out, "{byte:02x}");
                out.push(']');
            } else {
                let _ = write!(out, " {byte:02x}");
            }
        }
        if offset >= bytes.len() {
            out.push_str(" [EOF]");
        }
        out
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
        let path = env::var_os("PATH")?;
        env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| is_executable_file(candidate))
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
}

pub use imp::{
    DviComparison, DviDiff, RefTex, RefTftopl, RunOpts, RunOutput, compare_dvi_bytes,
    normalized_dvi_for_comparison,
};

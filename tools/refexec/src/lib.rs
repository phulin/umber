//! Reference TeX execution helper for parity tests.

#[allow(clippy::disallowed_methods)] // host tool, not engine code
mod imp {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use anyhow::{Context, Result, anyhow};
    use tempfile::TempDir;

    #[derive(Debug, Clone)]
    pub struct RefTex {
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

    impl RefTex {
        pub fn locate() -> Result<Self> {
            if let Some(path) = env::var_os("UMBER_REF_TEX").filter(|value| !value.is_empty()) {
                return Ok(Self {
                    executable: PathBuf::from(path),
                });
            }

            if let Some(path) = find_on_path("pdftex") {
                return Ok(Self { executable: path });
            }

            Err(anyhow!(
                "could not locate reference TeX: set UMBER_REF_TEX or make pdftex available on PATH"
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

            let mut command = Command::new(&self.executable);
            command
                .current_dir(temp_dir.path())
                .arg("-interaction=nonstopmode");

            if opts.dvi {
                command.arg("-output-format=dvi");
            }
            if opts.ini {
                command.arg("-ini");
            }
            command.arg(job_name);

            let output = command.output().with_context(|| {
                format!(
                    "failed to execute reference TeX {}",
                    self.executable.display()
                )
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stem = tex_file
                .file_stem()
                .ok_or_else(|| anyhow!("TeX input has no file stem: {}", tex_file.display()))?;
            let log_path = temp_dir.path().join(stem).with_extension("log");
            let log = fs::read_to_string(&log_path).with_context(|| {
                format!("failed to read reference TeX log {}", log_path.display())
            })?;
            let dvi = if opts.dvi {
                let dvi_path = temp_dir.path().join(stem).with_extension("dvi");
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

    fn file_name(path: &Path) -> Result<&std::ffi::OsStr> {
        path.file_name()
            .ok_or_else(|| anyhow!("path has no file name: {}", path.display()))
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

pub use imp::{RefTex, RunOpts, RunOutput};

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and support-file setup.
mod imp {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    use crate::{corpus_root, git_fixture::ClosedCase};

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CorpusCase {
        area: String,
        name: String,
        source_path: PathBuf,
    }

    impl CorpusCase {
        #[must_use]
        pub fn area(&self) -> &str {
            &self.area
        }

        #[must_use]
        pub fn name(&self) -> &str {
            &self.name
        }

        #[must_use]
        pub fn source_path(&self) -> &Path {
            &self.source_path
        }
    }

    pub fn corpus_area(area: &str) -> PathBuf {
        corpus_root().join(area)
    }

    pub fn corpus_cases(area: &str) -> Vec<CorpusCase> {
        corpus_cases_inner(area).unwrap_or_else(|error| panic!("{error:#}"))
    }

    fn corpus_cases_inner(area: &str) -> Result<Vec<CorpusCase>> {
        let area_path = corpus_area(area);
        let mut cases = Vec::new();
        for entry in fs::read_dir(&area_path)
            .with_context(|| format!("failed to read corpus area {}", area_path.display()))?
        {
            let path = entry
                .with_context(|| format!("failed to read corpus entry in {}", area_path.display()))?
                .path();
            let (name, source_path) = if is_directory_case_area(area) {
                if !path.is_dir() {
                    anyhow::bail!(
                        "directory-case corpus area contains non-directory entry: {}",
                        path.display()
                    );
                }
                let name = path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .with_context(|| format!("corpus case has invalid name: {}", path.display()))?
                    .to_owned();
                let closed =
                    ClosedCase::discover_tracked(Path::new("tests/corpus").join(area).join(&name))
                        .with_context(|| format!("{area}/{name} is not a closed fixture case"))?;
                let source = closed.path(&case_source_name(area, &name))?;
                (name, source)
            } else {
                if path.extension().and_then(OsStr::to_str) != Some("tex") {
                    continue;
                }
                let name = path
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .with_context(|| format!("corpus case has invalid stem: {}", path.display()))?
                    .to_owned();
                (name, path)
            };
            cases.push(CorpusCase {
                area: area.to_owned(),
                name,
                source_path,
            });
        }
        cases.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(cases)
    }

    pub fn copy_case_support_files(area: &str, case: &str, destination: &Path) -> Vec<PathBuf> {
        copy_case_support_files_inner(area, case, destination)
            .unwrap_or_else(|error| panic!("{error:#}"))
    }

    fn copy_case_support_files_inner(
        area: &str,
        case: &str,
        destination: &Path,
    ) -> Result<Vec<PathBuf>> {
        let area_path = if is_directory_case_area(area) {
            corpus_area(area).join(case)
        } else {
            corpus_area(area)
        };
        let mut copied = Vec::new();
        for entry in fs::read_dir(&area_path)
            .with_context(|| format!("failed to read corpus area {}", area_path.display()))?
        {
            let path = entry
                .with_context(|| format!("failed to read corpus entry in {}", area_path.display()))?
                .path();
            if !path.is_file() || !is_support_file(&path) {
                continue;
            }
            let name = path
                .file_name()
                .with_context(|| format!("support file has no name: {}", path.display()))?;
            let copied_path = destination.join(name);
            fs::copy(&path, &copied_path).with_context(|| {
                format!(
                    "failed to copy support file {} to {}",
                    path.display(),
                    copied_path.display()
                )
            })?;
            copied.push(copied_path);
        }
        copied.sort();
        Ok(copied)
    }

    fn is_support_file(path: &Path) -> bool {
        if path.extension().and_then(OsStr::to_str) == Some("tex") {
            return false;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            return false;
        };
        !name.contains(".expected.") && !name.starts_with("expected.")
    }

    pub fn is_directory_case_area(area: &str) -> bool {
        matches!(
            area,
            "canonical-dvi"
                | "hello"
                | "lexer"
                | "lexer_dynamic"
                | "stabilization"
                | "exec"
                | "etex_exec"
                | "typeset"
                | "math"
                | "align"
                | "tex_exec"
                | "tex_exec_io"
                | "expand"
                | "pdf"
        )
    }

    pub fn case_source_name(area: &str, case: &str) -> String {
        if matches!(
            area,
            "canonical-dvi" | "hello" | "lexer" | "lexer_dynamic" | "stabilization" | "pdf"
        ) {
            "source.tex".to_owned()
        } else {
            format!("{case}.tex")
        }
    }
}

pub use imp::{
    CorpusCase, case_source_name, copy_case_support_files, corpus_area, corpus_cases,
    is_directory_case_area,
};

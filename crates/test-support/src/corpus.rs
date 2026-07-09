#[allow(clippy::disallowed_methods)] // host-side corpus discovery and support-file setup.
mod imp {
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};

    use crate::corpus_root;

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
            if path.extension().and_then(OsStr::to_str) != Some("tex") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(OsStr::to_str)
                .with_context(|| format!("corpus case has invalid stem: {}", path.display()))?
                .to_owned();
            cases.push(CorpusCase {
                area: area.to_owned(),
                name,
                source_path: path,
            });
        }
        cases.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(cases)
    }

    pub fn copy_area_support_files(area: &str, destination: &Path) -> Vec<PathBuf> {
        copy_area_support_files_inner(area, destination).unwrap_or_else(|error| panic!("{error:#}"))
    }

    fn copy_area_support_files_inner(area: &str, destination: &Path) -> Result<Vec<PathBuf>> {
        let area_path = corpus_area(area);
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
        !name.contains(".expected.")
    }
}

pub use imp::{CorpusCase, copy_area_support_files, corpus_area, corpus_cases};

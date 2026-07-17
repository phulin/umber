use std::fmt::Write as _;

use anyhow::{Result, anyhow};

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

#[allow(clippy::disallowed_methods)] // host-side DVI fixture setup and comparison.
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use tempfile::TempDir;

    use super::{DviComparison, compare_dvi_bytes};
    use crate::{copy_area_support_files, corpus_root};

    const PINNED_CM_TFMS: &[&str] = &["cmr10.tfm", "cmmi10.tfm", "cmsy10.tfm", "cmex10.tfm"];

    pub struct DviCaseSetup {
        temp_dir: TempDir,
        source_path: PathBuf,
        actual_dvi_path: PathBuf,
        extra_inputs: Vec<PathBuf>,
    }

    impl DviCaseSetup {
        pub fn new(area: &str, case: &str) -> Self {
            Self::try_new(area, case).unwrap_or_else(|error| panic!("{error:#}"))
        }

        fn try_new(area: &str, case: &str) -> Result<Self> {
            let temp_dir = tempfile::tempdir().context("failed to create DVI fixture temp dir")?;
            let source = corpus_root().join(area).join(format!("{case}.tex"));
            let source_path = temp_dir.path().join(format!("{case}.tex"));
            fs::copy(&source, &source_path).with_context(|| {
                format!(
                    "failed to copy DVI source {} to {}",
                    source.display(),
                    source_path.display()
                )
            })?;

            let mut extra_inputs = copy_pinned_cm_tfms(temp_dir.path())?;
            extra_inputs.extend(copy_area_support_files(area, temp_dir.path()));
            extra_inputs.sort();

            Ok(Self {
                actual_dvi_path: temp_dir.path().join("actual.dvi"),
                temp_dir,
                source_path,
                extra_inputs,
            })
        }

        #[must_use]
        pub fn run_dir(&self) -> &Path {
            self.temp_dir.path()
        }

        #[must_use]
        pub fn source_path(&self) -> &Path {
            &self.source_path
        }

        #[must_use]
        pub fn source_file_name(&self) -> &str {
            self.source_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("DVI source file name should be utf-8")
        }

        #[must_use]
        pub fn actual_dvi_path(&self) -> &Path {
            &self.actual_dvi_path
        }

        #[must_use]
        pub fn actual_dvi_file_name(&self) -> &str {
            self.actual_dvi_path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("DVI output file name should be utf-8")
        }

        #[must_use]
        pub fn extra_inputs(&self) -> &[PathBuf] {
            &self.extra_inputs
        }
    }

    fn copy_pinned_cm_tfms(destination: &Path) -> Result<Vec<PathBuf>> {
        let cm_dir = corpus_root()
            .join("../../crates/tex-fonts/tests/fixtures/cm")
            .canonicalize()
            .context("failed to locate pinned CM TFM fixture directory")?;
        let mut copied = Vec::new();
        for tfm_name in PINNED_CM_TFMS {
            let source = cm_dir.join(tfm_name);
            let copied_path = destination.join(tfm_name);
            fs::copy(&source, &copied_path).with_context(|| {
                format!(
                    "failed to copy pinned TFM {} to {}",
                    source.display(),
                    copied_path.display()
                )
            })?;
            copied.push(copied_path);
        }
        Ok(copied)
    }

    pub fn assert_dvi_matches(expected: &[u8], actual: &[u8], label: &str) {
        let comparison = compare_dvi_bytes(expected, actual).expect("compare DVI bytes");
        assert_eq!(
            comparison,
            DviComparison::Equal,
            "DVI fixture mismatch for {label}"
        );
    }
}

pub use imp::{DviCaseSetup, assert_dvi_matches};

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{DviComparison, compare_dvi_bytes};

    #[test]
    fn comparison_normalizes_only_the_preamble_comment_payload() -> Result<()> {
        let mut left = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
        left.extend_from_slice(b"abc");
        left.extend_from_slice(&[139, 140]);
        let mut right = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
        right.extend_from_slice(b"xyz");
        right.extend_from_slice(&[139, 140]);

        assert_eq!(compare_dvi_bytes(&left, &right)?, DviComparison::Equal);

        right[18] = 141;
        let DviComparison::Different(diff) = compare_dvi_bytes(&left, &right)? else {
            panic!("body byte mismatch should be reported");
        };
        assert_eq!(diff.offset, 18);
        Ok(())
    }
}

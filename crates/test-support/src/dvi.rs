#[allow(clippy::disallowed_methods)] // host-side DVI fixture setup and comparison.
mod imp {
    use std::fs;
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use refexec::{DviComparison, RefTex, RunOpts, compare_dvi_bytes};
    use tempfile::TempDir;

    use crate::{
        copy_area_support_files, corpus_root, fixture_path, live_reference_enabled,
        read_binary_fixture, update_fixtures_enabled, write_binary_fixture,
    };

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

    pub fn expected_dvi_fixture(
        area: &str,
        case: &str,
        setup: &DviCaseSetup,
        ini: bool,
    ) -> Vec<u8> {
        if update_fixtures_enabled() || live_reference_enabled() {
            let expected = reference_dvi(setup, ini);
            if update_fixtures_enabled() {
                update_dvi_fixture_if_changed(area, case, &expected);
            }
            if live_reference_enabled() {
                let committed = read_binary_fixture(area, case, "dvi");
                assert_dvi_matches(
                    &expected,
                    &committed,
                    &format!("{area}/{case} committed fixture"),
                );
            }
            expected
        } else {
            read_binary_fixture(area, case, "dvi")
        }
    }

    pub fn reference_dvi(setup: &DviCaseSetup, ini: bool) -> Vec<u8> {
        let output = RefTex::locate()
            .expect("locate reference TeX")
            .run(
                setup.source_path(),
                &RunOpts {
                    dvi: true,
                    ini,
                    extra_inputs: setup.extra_inputs().to_vec(),
                },
            )
            .expect("run reference DVI fixture");
        assert!(
            output.success,
            "reference DVI fixture failed:\n{}",
            output.log
        );
        output.dvi.expect("reference TeX should produce DVI")
    }

    pub fn update_dvi_fixture_if_changed(area: &str, case: &str, expected: &[u8]) {
        let path = fixture_path(area, case, "dvi");
        let unchanged = fs::read(&path)
            .ok()
            .and_then(|current| compare_dvi_bytes(expected, &current).ok())
            == Some(DviComparison::Equal);
        if unchanged {
            return;
        }

        write_binary_fixture(area, case, "dvi", expected);
        panic!(
            "DVI fixture updated at {} -- rerun without UPDATE_FIXTURES",
            path.display()
        );
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

pub use imp::{
    DviCaseSetup, assert_dvi_matches, expected_dvi_fixture, reference_dvi,
    update_dvi_fixture_if_changed,
};

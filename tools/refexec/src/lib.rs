//! Compatibility facade for reference execution and DVI comparison.
//!
//! Fixture publication owns the process and staging kernel. This crate keeps
//! its established Rust and CLI surfaces while delegating that implementation.

use std::path::Path;

use anyhow::{Result, anyhow};
pub use fixturegen_reference::reference::{
    PLAIN_PRELOAD_FONTS, REFERENCE_JOB_NAME, RefTftopl, RunOpts, RunOutput,
    generate_reference_fixture, locate_tfm, stage_reference_document,
};
pub use test_support::dvi::{
    DviComparison, DviDiff, compare_dvi_bytes, normalized_dvi_for_comparison,
};

#[derive(Debug, Clone)]
pub struct RefTex(fixturegen_reference::reference::RefTex);

impl RefTex {
    pub fn locate() -> Result<Self> {
        fixturegen_reference::reference::RefTex::locate().map(Self)
    }

    pub fn from_executable(executable: impl Into<std::path::PathBuf>) -> Self {
        Self(fixturegen_reference::reference::RefTex::from_executable(
            executable,
        ))
    }

    pub fn run(&self, tex_file: &Path, opts: &RunOpts) -> Result<RunOutput> {
        self.0.run(tex_file, opts)
    }

    pub fn run_in_dir(&self, dir: &Path, tex_file: &Path, opts: &RunOpts) -> Result<RunOutput> {
        self.0.run_in_dir(dir, tex_file, opts)
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

    pub fn publication_inner(&self) -> &fixturegen_reference::reference::RefTex {
        &self.0
    }
}

pub fn run_reference_document(
    repo_root: &Path,
    ref_tex: &RefTex,
    source_path: &Path,
    format_source_path: &Path,
    tracing: bool,
) -> Result<RunOutput> {
    fixturegen_reference::reference::run_reference_document(
        repo_root,
        ref_tex.publication_inner(),
        source_path,
        format_source_path,
        tracing,
    )
}

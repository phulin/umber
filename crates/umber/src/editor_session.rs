use std::fmt;
use std::path::PathBuf;

use tex_state::ContentHash;
use umber_vfs::VirtualPath;

use crate::{
    CompileAttemptResult, CompileError, FixedPointLimits, MemoryOutputFile, MemoryRunOutput,
    NeedResources, ResourceResponse, SessionOptions, SourcePatch, TexFixedPointAttempt,
    TexFixedPointError, TexFixedPointOutput, TexFixedPointSession, VirtualCompileSession,
};

#[derive(Clone, Debug)]
pub struct EditorSessionOptions {
    pub tex: SessionOptions,
    pub stabilization: FixedPointLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorSessionStatus {
    Provisional {
        revision: tex_incr::RevisionId,
        stabilization_required: bool,
    },
    Stabilizing {
        revision: tex_incr::RevisionId,
        completed_passes: u32,
        stabilization_required: bool,
    },
    Stable {
        revision: tex_incr::RevisionId,
        passes: u32,
        stabilization_required: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorStabilizationAttempt {
    NeedResources(NeedResources),
    Complete(Box<TexFixedPointOutput>),
    Error(TexFixedPointError),
}

/// Persistent editor coordinator that keeps one-pass display latency separate
/// from explicit fixed-point stabilization.
pub struct EditorCompileSession {
    hot: Box<VirtualCompileSession>,
    limits: FixedPointLimits,
    display: Option<TexFixedPointOutput>,
    stable: Option<TexFixedPointOutput>,
    stabilizing: Option<TexFixedPointSession>,
}

impl EditorCompileSession {
    pub fn new(options: EditorSessionOptions) -> Result<Self, CompileError> {
        Ok(Self {
            hot: Box::new(VirtualCompileSession::new(options.tex)?),
            limits: options.stabilization,
            display: None,
            stable: None,
            stabilizing: None,
        })
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), CompileError> {
        self.hot.add_user_file(path, bytes)
    }

    pub fn apply_patch(&mut self, patch: SourcePatch) -> Result<(), CompileError> {
        self.stabilizing = None;
        self.hot.apply_patch(patch)
    }

    /// Cancels off-hot-path work without changing display or stable output.
    pub fn cancel_stabilization(&mut self) -> bool {
        self.stabilizing.take().is_some()
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), EditorResourceError> {
        if let Some(stabilizing) = &mut self.stabilizing {
            stabilizing
                .provide_resources(responses)
                .map_err(EditorResourceError::Stabilization)
        } else {
            self.hot
                .provide_resources(responses)
                .map_err(EditorResourceError::Advance)
        }
    }

    pub fn advance(&mut self) -> CompileAttemptResult {
        let result = self.hot.compile_attempt();
        if let CompileAttemptResult::Complete(output) = &result {
            match self.output_from_hot(output.clone()) {
                Ok(accepted) => {
                    if self.hot.stabilization_required() {
                        self.display = Some(accepted);
                    } else {
                        self.stable = Some(accepted.clone());
                        self.display = Some(accepted);
                    }
                }
                Err(error) => return CompileAttemptResult::Error(error),
            }
        }
        result
    }

    pub fn stabilize_attempt(&mut self) -> EditorStabilizationAttempt {
        let Some(display) = self.display.as_ref() else {
            return EditorStabilizationAttempt::Error(TexFixedPointError::InvalidPatch(
                "cannot stabilize before an editor pass completes".into(),
            ));
        };
        if !self.hot.stabilization_required() {
            return EditorStabilizationAttempt::Complete(Box::new(display.clone()));
        }
        if self.stabilizing.is_none() {
            match TexFixedPointSession::from_provisional(&self.hot, self.limits) {
                Ok(session) => self.stabilizing = Some(session),
                Err(error) => return EditorStabilizationAttempt::Error(error),
            }
        }
        let stabilizing = self.stabilizing.as_mut().expect("created above");
        match stabilizing.compile_attempt() {
            TexFixedPointAttempt::NeedResources(needs) => {
                EditorStabilizationAttempt::NeedResources(needs)
            }
            TexFixedPointAttempt::Error(error) => {
                self.stabilizing = None;
                EditorStabilizationAttempt::Error(error)
            }
            TexFixedPointAttempt::Complete(output) => {
                let mut completed = *output;
                // The fixed-point adapter counts its first cold pass as one;
                // include the already published provisional pass as well.
                completed.passes = completed.passes.saturating_add(1);
                let accepted_tex = stabilizing
                    .take_accepted_tex()
                    .expect("completed stabilization retains its accepted TeX session");
                self.hot = accepted_tex;
                self.hot.mark_stable();
                self.display = Some(completed.clone());
                self.stable = Some(completed.clone());
                self.stabilizing = None;
                EditorStabilizationAttempt::Complete(Box::new(completed))
            }
        }
    }

    #[must_use]
    pub fn status(&self) -> Option<EditorSessionStatus> {
        let display = self.display.as_ref()?;
        if let Some(stabilizing) = &self.stabilizing {
            return Some(EditorSessionStatus::Stabilizing {
                revision: display.revision,
                completed_passes: stabilizing.completed_passes().saturating_add(1),
                stabilization_required: true,
            });
        }
        if self.hot.stabilization_required() {
            Some(EditorSessionStatus::Provisional {
                revision: display.revision,
                stabilization_required: true,
            })
        } else {
            Some(EditorSessionStatus::Stable {
                revision: display.revision,
                passes: display.passes,
                stabilization_required: false,
            })
        }
    }

    #[must_use]
    pub const fn display_output(&self) -> Option<&TexFixedPointOutput> {
        self.display.as_ref()
    }

    #[must_use]
    pub const fn stable_output(&self) -> Option<&TexFixedPointOutput> {
        self.stable.as_ref()
    }

    fn output_from_hot(&self, tex: MemoryRunOutput) -> Result<TexFixedPointOutput, CompileError> {
        let revision = self.hot.revision().expect("completed pass has a revision");
        let content_hash = self.hot.content_hash().expect("completed pass has a hash");
        let generated_fingerprint = self.hot.accepted_generated_fingerprint()?;
        let generated_files = generated_files(&tex, &generated_fingerprint);
        Ok(TexFixedPointOutput {
            revision,
            content_hash,
            passes: 1,
            tex,
            generated_files,
            generated_fingerprint,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorResourceError {
    Advance(CompileError),
    Stabilization(TexFixedPointError),
}

impl fmt::Display for EditorResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Advance(error) => error.fmt(formatter),
            Self::Stabilization(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EditorResourceError {}

fn generated_files(
    output: &MemoryRunOutput,
    fingerprint: &[(VirtualPath, ContentHash)],
) -> Vec<MemoryOutputFile> {
    fingerprint
        .iter()
        .filter_map(|(path, _)| {
            output
                .files
                .iter()
                .find(|file| {
                    file.path
                        .to_str()
                        .and_then(|candidate| VirtualPath::user(candidate).ok())
                        .as_ref()
                        == Some(path)
                })
                .map(|file| MemoryOutputFile {
                    path: PathBuf::from(path.as_str()),
                    bytes: file.bytes.clone(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests;

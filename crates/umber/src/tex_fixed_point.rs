use std::fmt;

use tex_state::ContentHash;
use umber_vfs::VirtualPath;

use crate::{
    AcceptedInputObservationLedger, CompileError, FixedPointLimits, LatexProjectAttempt,
    LatexProjectError, LatexProjectSession, MemoryOutputFile, MemoryRunOutput, NeedResources,
    ResourceResponse, SessionOptions, SourcePatch,
};

#[derive(Clone, Debug)]
pub struct TexFixedPointOptions {
    pub tex: SessionOptions,
    pub limits: FixedPointLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TexFixedPointOutput {
    pub revision: tex_incr::RevisionId,
    pub content_hash: ContentHash,
    pub passes: u32,
    pub tex: MemoryRunOutput,
    pub generated_files: Vec<MemoryOutputFile>,
    pub generated_fingerprint: Vec<(VirtualPath, ContentHash)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TexFixedPointAttempt {
    NeedResources(NeedResources),
    Complete(Box<TexFixedPointOutput>),
    Error(TexFixedPointError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TexFixedPointError {
    Compile(CompileError),
    InvalidLimit { name: &'static str, value: u32 },
    PassLimit { limit: u32 },
    Oscillation { first_pass: u32, repeated_pass: u32 },
    Transaction(String),
    InvalidPatch(String),
    UnexpectedResource(String),
    ConflictingResource(String),
}

impl fmt::Display for TexFixedPointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::InvalidLimit { name, value } => {
                write!(formatter, "invalid fixed-point {name} limit {value}")
            }
            Self::PassLimit { limit } => {
                write!(formatter, "fixed-point pass limit {limit} reached")
            }
            Self::Oscillation {
                first_pass,
                repeated_pass,
            } => write!(
                formatter,
                "generated output oscillated between passes {first_pass} and {repeated_pass}"
            ),
            Self::Transaction(message) | Self::InvalidPatch(message) => {
                formatter.write_str(message)
            }
            Self::UnexpectedResource(name) => {
                write!(formatter, "resource response {name} was not requested")
            }
            Self::ConflictingResource(name) => {
                write!(
                    formatter,
                    "resource response {name} conflicts with retained content"
                )
            }
        }
    }
}

impl std::error::Error for TexFixedPointError {}

/// Host-neutral bounded TeX fixed-point session with private generated
/// generations and atomic acceptance.
pub struct TexFixedPointSession {
    inner: LatexProjectSession,
    accepted_output: Option<TexFixedPointOutput>,
}

impl TexFixedPointSession {
    pub fn new(options: TexFixedPointOptions) -> Result<Self, TexFixedPointError> {
        Ok(Self {
            inner: LatexProjectSession::new_tex_only(options.tex, options.limits)
                .map_err(tex_fixed_point_error)?,
            accepted_output: None,
        })
    }

    pub(crate) fn from_provisional(
        provisional: &crate::VirtualCompileSession,
        limits: FixedPointLimits,
    ) -> Result<Self, TexFixedPointError> {
        Ok(Self {
            inner: LatexProjectSession::new_tex_only_from_provisional(provisional, limits)
                .map_err(tex_fixed_point_error)?,
            accepted_output: None,
        })
    }

    pub fn add_user_file(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), TexFixedPointError> {
        self.inner
            .add_user_file(path, bytes)
            .map_err(tex_fixed_point_error)
    }

    pub fn apply_patch(&mut self, patch: SourcePatch) -> Result<(), TexFixedPointError> {
        self.inner.apply_patch(patch).map_err(tex_fixed_point_error)
    }

    pub fn provide_resources(
        &mut self,
        responses: Vec<ResourceResponse>,
    ) -> Result<(), TexFixedPointError> {
        self.inner
            .provide_resources(responses)
            .map_err(tex_fixed_point_error)
    }

    #[must_use]
    pub fn cancel_pending_patch(&mut self) -> bool {
        self.inner.cancel_pending_patch()
    }

    pub fn compile_attempt(&mut self) -> TexFixedPointAttempt {
        match self.inner.compile_attempt() {
            LatexProjectAttempt::NeedResources(needs) => TexFixedPointAttempt::NeedResources(needs),
            LatexProjectAttempt::Complete(output) => {
                let output = TexFixedPointOutput {
                    revision: output.revision,
                    content_hash: output.content_hash,
                    passes: output.passes,
                    tex: output.tex,
                    generated_files: output.generated_files,
                    generated_fingerprint: output.fingerprint.generated,
                };
                self.accepted_output = Some(output.clone());
                TexFixedPointAttempt::Complete(Box::new(output))
            }
            LatexProjectAttempt::Error(error) => {
                TexFixedPointAttempt::Error(tex_fixed_point_error(error))
            }
        }
    }

    #[must_use]
    pub const fn revision(&self) -> Option<tex_incr::RevisionId> {
        self.inner.revision()
    }

    #[must_use]
    pub fn content_hash(&self) -> Option<ContentHash> {
        self.inner.content_hash()
    }

    #[must_use]
    pub const fn accepted_output(&self) -> Option<&TexFixedPointOutput> {
        self.accepted_output.as_ref()
    }

    #[must_use]
    pub const fn accepted_input_observations(&self) -> Option<&AcceptedInputObservationLedger> {
        self.inner.accepted_input_observations()
    }

    #[must_use]
    pub(crate) fn completed_passes(&self) -> u32 {
        self.inner.completed_passes()
    }

    pub(crate) fn take_accepted_tex(&mut self) -> Option<Box<crate::VirtualCompileSession>> {
        self.inner.take_accepted_tex()
    }
}

fn tex_fixed_point_error(error: LatexProjectError) -> TexFixedPointError {
    match error {
        LatexProjectError::Compile(error) => TexFixedPointError::Compile(error),
        LatexProjectError::InvalidLimit { name, value } => {
            TexFixedPointError::InvalidLimit { name, value }
        }
        LatexProjectError::PassLimit { limit } => TexFixedPointError::PassLimit { limit },
        LatexProjectError::Oscillation {
            first_pass,
            repeated_pass,
        } => TexFixedPointError::Oscillation {
            first_pass,
            repeated_pass,
        },
        LatexProjectError::Transaction(message) => TexFixedPointError::Transaction(message),
        LatexProjectError::InvalidPatch(message) => TexFixedPointError::InvalidPatch(message),
        LatexProjectError::UnexpectedResource(name) => TexFixedPointError::UnexpectedResource(name),
        LatexProjectError::ConflictingResource(name) => {
            TexFixedPointError::ConflictingResource(name)
        }
        LatexProjectError::Bibliography(error) => TexFixedPointError::Transaction(format!(
            "disabled bibliography stage failed unexpectedly: {error:?}"
        )),
        LatexProjectError::BibliographyFatal { backend } => TexFixedPointError::Transaction(
            format!("disabled bibliography stage selected {backend:?} unexpectedly"),
        ),
    }
}

#[cfg(test)]
mod tests;

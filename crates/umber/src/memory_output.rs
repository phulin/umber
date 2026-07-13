use std::path::PathBuf;

use tex_state::{ContentHash, Universe, WorldError};

use crate::{DviBuildError, dvi_from_artifacts};

/// One committed auxiliary output returned by a memory-backed run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryOutputFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Exact observable outputs of one successful memory-backed run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRunOutput {
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    pub dvi: Vec<u8>,
    pub files: Vec<MemoryOutputFile>,
}

/// Commits the final effect prefix exactly once and collects all observable
/// outputs from a successful memory-backed run.
///
/// Callers must discard the complete `Universe` instead of invoking this for
/// an attempt that discovered missing files.
pub fn collect_final_memory_output(
    stores: &mut Universe,
    artifacts: &[ContentHash],
    output_byte_limit: usize,
) -> Result<MemoryRunOutput, MemoryOutputCollectionError> {
    let effect_pos = stores.world().effect_pos();
    stores.commit_effects(effect_pos)?;

    let terminal = stores
        .world()
        .memory_terminal_output()
        .ok_or(MemoryOutputCollectionError::NotMemoryBacked)?;
    let log = stores
        .world()
        .memory_log_output()
        .ok_or(MemoryOutputCollectionError::NotMemoryBacked)?;
    let outputs = stores
        .world()
        .memory_outputs()
        .ok_or(MemoryOutputCollectionError::NotMemoryBacked)?;

    let mut total = 0usize;
    account(&mut total, terminal.len(), output_byte_limit)?;
    account(&mut total, log.len(), output_byte_limit)?;

    let mut files = Vec::with_capacity(outputs.len());
    for output in outputs {
        account(&mut total, output.bytes().len(), output_byte_limit)?;
        files.push(MemoryOutputFile {
            path: output.path().to_owned(),
            bytes: output.bytes().to_vec(),
        });
    }

    // The downstream DVI writer requires at least one page. A successful TeX
    // job may legitimately ship none, which the browser API represents as an
    // empty binary output rather than converting into an engine failure.
    let dvi = if artifacts.is_empty() {
        Vec::new()
    } else {
        dvi_from_artifacts(stores, artifacts)?
    };
    account(&mut total, dvi.len(), output_byte_limit)?;

    Ok(MemoryRunOutput {
        terminal: terminal.to_vec(),
        log: log.to_vec(),
        dvi,
        files,
    })
}

fn account(
    total: &mut usize,
    bytes: usize,
    limit: usize,
) -> Result<(), MemoryOutputCollectionError> {
    let required =
        total
            .checked_add(bytes)
            .ok_or(MemoryOutputCollectionError::OutputLimitExceeded {
                limit,
                required_at_least: usize::MAX,
            })?;
    if required > limit {
        return Err(MemoryOutputCollectionError::OutputLimitExceeded {
            limit,
            required_at_least: required,
        });
    }
    *total = required;
    Ok(())
}

#[derive(Debug)]
pub enum MemoryOutputCollectionError {
    NotMemoryBacked,
    OutputLimitExceeded {
        limit: usize,
        required_at_least: usize,
    },
    World(WorldError),
    Dvi(DviBuildError),
}

impl std::fmt::Display for MemoryOutputCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMemoryBacked => write!(f, "final output collection requires a memory World"),
            Self::OutputLimitExceeded {
                limit,
                required_at_least,
            } => write!(
                f,
                "returned output requires at least {required_at_least} bytes, exceeding limit {limit}"
            ),
            Self::World(error) => error.fmt(f),
            Self::Dvi(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for MemoryOutputCollectionError {}

impl From<WorldError> for MemoryOutputCollectionError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}

impl From<DviBuildError> for MemoryOutputCollectionError {
    fn from(value: DviBuildError) -> Self {
        Self::Dvi(value)
    }
}

#[cfg(test)]
mod tests;

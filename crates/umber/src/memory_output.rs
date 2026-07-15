use std::path::PathBuf;

use tex_out::dvi::DviPagePlan;
use tex_state::{CommittedArtifact, ContentHash, Universe, World, WorldError};
use umber_vfs::{StageTransaction, TransactionError, VirtualPath};

use crate::{DviBuildError, dvi_from_artifacts, dvi_from_committed_artifacts, dvi_from_page_plans};

/// Fast-path variant for page bodies compiled before successful shipout.
pub fn collect_final_memory_output_from_plans(
    stores: &mut Universe,
    plans: &[DviPagePlan],
    output_byte_limit: usize,
) -> Result<MemoryRunOutput, MemoryOutputCollectionError> {
    collect_final_memory_output_with_dvi(stores, output_byte_limit, |_| {
        if plans.is_empty() {
            Ok(Vec::new())
        } else {
            dvi_from_page_plans(plans)
        }
    })
}

/// One committed auxiliary output returned by a memory-backed run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryOutputFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

/// Copies complete, committed auxiliary files from `World` into one private
/// VFS stage write set, preserving World's deterministic path order.
pub(crate) fn publish_auxiliary_outputs(
    world: &World,
    stage: &mut StageTransaction<'_, '_>,
) -> Result<Vec<MemoryOutputFile>, MemoryOutputCollectionError> {
    let outputs = world
        .memory_outputs()
        .ok_or(MemoryOutputCollectionError::NotMemoryBacked)?;
    let mut files = Vec::with_capacity(outputs.len());
    for output in outputs {
        let Some(path) = output.path().to_str() else {
            return Err(MemoryOutputCollectionError::InvalidAuxiliaryPath(
                output.path().to_owned(),
            ));
        };
        let virtual_path = VirtualPath::user(path).map_err(|_| {
            MemoryOutputCollectionError::InvalidAuxiliaryPath(output.path().to_owned())
        })?;
        let bytes = output.bytes().to_vec();
        stage.write(virtual_path, bytes.clone())?;
        files.push(MemoryOutputFile {
            path: output.path().to_owned(),
            bytes,
        });
    }
    Ok(files)
}

/// Fast-path variant for artifacts committed by the current in-process run.
pub fn collect_final_memory_output_from_commits(
    stores: &mut Universe,
    artifacts: &[CommittedArtifact],
    output_byte_limit: usize,
) -> Result<MemoryRunOutput, MemoryOutputCollectionError> {
    collect_final_memory_output_with_dvi(stores, output_byte_limit, |_| {
        if artifacts.is_empty() {
            Ok(Vec::new())
        } else {
            dvi_from_committed_artifacts(artifacts)
        }
    })
}

/// Exact observable outputs of one successful memory-backed run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRunOutput {
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    pub dvi: Vec<u8>,
    /// Standalone HTML requested by the host-neutral session.
    pub html: Option<Vec<u8>>,
    /// Content-addressed HTML assets. Embedded mode leaves this empty.
    pub html_assets: Vec<MemoryOutputFile>,
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
    collect_final_memory_output_with_dvi(stores, output_byte_limit, |stores| {
        if artifacts.is_empty() {
            Ok(Vec::new())
        } else {
            dvi_from_artifacts(stores, artifacts)
        }
    })
}

fn collect_final_memory_output_with_dvi<F>(
    stores: &mut Universe,
    output_byte_limit: usize,
    build_dvi: F,
) -> Result<MemoryRunOutput, MemoryOutputCollectionError>
where
    F: FnOnce(&Universe) -> Result<Vec<u8>, DviBuildError>,
{
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
    let dvi = build_dvi(stores)?;
    account(&mut total, dvi.len(), output_byte_limit)?;

    Ok(MemoryRunOutput {
        terminal: terminal.to_vec(),
        log: log.to_vec(),
        dvi,
        html: None,
        html_assets: Vec::new(),
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
    InvalidAuxiliaryPath(PathBuf),
    OutputLimitExceeded {
        limit: usize,
        required_at_least: usize,
    },
    World(WorldError),
    Dvi(DviBuildError),
    Transaction(TransactionError),
}

impl std::fmt::Display for MemoryOutputCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMemoryBacked => write!(f, "final output collection requires a memory World"),
            Self::InvalidAuxiliaryPath(path) => write!(
                f,
                "auxiliary output path {:?} is not a valid /job virtual path",
                path
            ),
            Self::OutputLimitExceeded {
                limit,
                required_at_least,
            } => write!(
                f,
                "returned output requires at least {required_at_least} bytes, exceeding limit {limit}"
            ),
            Self::World(error) => error.fmt(f),
            Self::Dvi(error) => error.fmt(f),
            Self::Transaction(error) => error.fmt(f),
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

impl From<TransactionError> for MemoryOutputCollectionError {
    fn from(value: TransactionError) -> Self {
        Self::Transaction(value)
    }
}

#[cfg(test)]
mod tests;

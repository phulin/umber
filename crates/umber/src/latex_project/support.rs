use umber_vfs::{FileOrigin, VirtualRoot};

use super::*;

pub(super) enum CandidateStop {
    Need(NeedResources),
    Failed(LatexProjectError),
}

impl From<LatexProjectError> for CandidateStop {
    fn from(value: LatexProjectError) -> Self {
        Self::Failed(value)
    }
}

pub(super) fn project_vfs_limits(options: &SessionOptions) -> umber_vfs::VfsLimits {
    umber_vfs::VfsLimits {
        user_files: options.limits.user_files,
        resolved_files: options.limits.resolved_files,
        stage_files: umber_vfs::VfsLimits::HARD_MAX.stage_files,
        generated_files: umber_vfs::VfsLimits::HARD_MAX.generated_files,
        one_file_bytes: options.limits.one_file_bytes,
        user_bytes: options.limits.user_source_bytes,
        resolved_bytes: options.limits.cached_file_bytes,
        stage_bytes: options.limits.output_bytes,
        generated_bytes: options.limits.output_bytes,
    }
}

pub(super) fn accepted_generated(
    files: &FileProvisioner,
) -> Result<BTreeMap<VirtualPath, Vec<u8>>, CandidateStop> {
    let snapshot = files.snapshot();
    let paths = snapshot
        .list_root(VirtualRoot::Job, files_limit())
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    let mut generated = BTreeMap::new();
    for path in paths {
        let file = snapshot
            .get(&path)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?
            .expect("listed path resolves");
        if matches!(file.origin(), FileOrigin::Generated { .. }) {
            generated.insert(path, file.bytes().to_vec());
        }
    }
    Ok(generated)
}

pub(super) fn add_candidate_inputs(
    session: &mut VirtualCompileSession,
    files: &FileProvisioner,
    main_path: &str,
    root: &[u8],
    generated: &BTreeMap<VirtualPath, Vec<u8>>,
) -> Result<(), CandidateStop> {
    let snapshot = files.snapshot();
    for path in snapshot
        .list_root(VirtualRoot::Job, files_limit())
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?
    {
        let file = snapshot
            .get(&path)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?
            .expect("listed path resolves");
        if matches!(file.origin(), FileOrigin::User) {
            session
                .add_user_file(path.as_str(), file.bytes().to_vec())
                .map_err(LatexProjectError::Compile)?;
        }
    }
    session
        .add_user_file(main_path, root.to_vec())
        .map_err(LatexProjectError::Compile)?;
    for (path, bytes) in generated {
        session
            .add_user_file(path.as_str(), bytes.clone())
            .map_err(LatexProjectError::Compile)?;
    }
    Ok(())
}

pub(super) fn merge_tex_files(
    generated: &mut BTreeMap<VirtualPath, Vec<u8>>,
    outputs: &[MemoryOutputFile],
) -> Result<(), CandidateStop> {
    for output in outputs {
        let path = output.path.to_str().ok_or_else(|| {
            LatexProjectError::Transaction("TeX generated a non-UTF-8 path".into())
        })?;
        let path = VirtualPath::user(path)
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
        generated.insert(path, output.bytes.clone());
    }
    Ok(())
}

pub(super) fn generated_signature(
    generated: &BTreeMap<VirtualPath, Vec<u8>>,
) -> Vec<(VirtualPath, ContentHash)> {
    generated
        .iter()
        .map(|(path, bytes)| (path.clone(), ContentHash::from_bytes(bytes)))
        .collect()
}

pub(super) fn candidate_snapshot(
    files: &FileProvisioner,
    main_path: &str,
    root: &[u8],
    generated: &BTreeMap<VirtualPath, Vec<u8>>,
) -> Result<umber_vfs::VfsSnapshot, CandidateStop> {
    let mut pending = files.clone();
    let main = VirtualPath::user(main_path)
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    pending
        .register_user(main, root.to_vec())
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    let mut build = pending.begin_build(BuildPlan::new(BuildId::new(1)));
    let mut stage = build
        .begin_stage(PROJECT_PRODUCER)
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    for (path, bytes) in generated {
        stage
            .write(path.clone(), bytes.clone())
            .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    }
    stage
        .finish()
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    build
        .accept()
        .map_err(|error| LatexProjectError::Transaction(error.to_string()))?;
    Ok(pending.snapshot())
}

pub(super) fn file_needs(batch: FileRequestBatch) -> NeedResources {
    NeedResources {
        required: batch
            .required
            .into_iter()
            .map(ResourceRequest::File)
            .collect(),
        prefetch_hints: batch
            .prefetch_hints
            .into_iter()
            .map(ResourceRequest::File)
            .collect(),
    }
}

fn files_limit() -> usize {
    umber_vfs::VfsLimits::HARD_MAX
        .user_files
        .saturating_add(umber_vfs::VfsLimits::HARD_MAX.generated_files)
}

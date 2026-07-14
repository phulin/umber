use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tex_exec::{ExecutionContext, FontResolver};
use tex_expand::InputResolver;
use tex_lex::WorldInput;
use tex_state::{FileContent, InputReadState};

use super::path::RequestedFile;
use super::{CompileError, FileKind, FileRequest, FileRequestKey, ResolvedFile, VirtualPath};

pub(super) struct VirtualRunResolvers<'a> {
    input: VirtualFileResolver<'a>,
    font: VirtualFileResolver<'a>,
}

struct VirtualFileResolver<'a> {
    user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
    resolved_files: &'a BTreeMap<FileRequestKey, ResolvedFile>,
    misses: Vec<(u64, FileRequest)>,
    seen: BTreeSet<FileRequestKey>,
    fatal: Option<CompileError>,
}

impl<'a> VirtualRunResolvers<'a> {
    pub(super) fn new(
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, ResolvedFile>,
    ) -> Self {
        Self {
            input: VirtualFileResolver::new(user_files, resolved_files),
            font: VirtualFileResolver::new(user_files, resolved_files),
        }
    }

    pub(super) fn context(&mut self, job_name: &'a str) -> ExecutionContext<'_, WorldInput> {
        ExecutionContext::with_resolvers(job_name, &mut self.input, &mut self.font)
    }

    pub(super) fn finish(self) -> (Vec<FileRequest>, Option<CompileError>) {
        let mut misses = self.input.misses;
        misses.extend(self.font.misses);
        misses.sort_by_key(|(request_index, _)| *request_index);
        (
            misses.into_iter().map(|(_, request)| request).collect(),
            self.input.fatal.or(self.font.fatal),
        )
    }
}

impl<'a> VirtualFileResolver<'a> {
    fn new(
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, ResolvedFile>,
    ) -> Self {
        Self {
            user_files,
            resolved_files,
            misses: Vec::new(),
            seen: BTreeSet::new(),
            fatal: None,
        }
    }

    fn open(
        &mut self,
        input: &mut dyn InputReadState,
        kind: FileKind,
        original_name: &str,
        request_index: u64,
    ) -> Result<FileContent, String> {
        let requested = match RequestedFile::parse(kind, original_name) {
            Ok(requested) => requested,
            Err(error) => {
                let failure = CompileError::InvalidRequestedPath {
                    name: original_name.to_owned(),
                    message: error.to_string(),
                };
                self.record_fatal(failure.clone());
                return Err(failure.to_string());
            }
        };

        match requested {
            RequestedFile::UserOnly(path) => {
                if !self.user_files.contains_key(&path) {
                    let failure = CompileError::UnavailableAbsoluteUserFile(path.to_string());
                    self.record_fatal(failure.clone());
                    return Err(failure.to_string());
                }
                self.read_seeded(input, path.as_path())
            }
            RequestedFile::Remote { user_path, key } => {
                if self.user_files.contains_key(&user_path) {
                    return self.read_seeded(input, user_path.as_path());
                }
                if let Some(resolved) = self.resolved_files.get(&key) {
                    return self.read_seeded(input, resolved.virtual_path.as_path());
                }
                if self.seen.insert(key.clone()) {
                    self.misses.push((
                        request_index,
                        FileRequest {
                            key,
                            original_name: original_name.to_owned(),
                        },
                    ));
                }
                Err(format!("{kind} file {original_name} is not cached"))
            }
        }
    }

    fn read_seeded(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
    ) -> Result<FileContent, String> {
        input.read_input_file(path).map_err(|error| {
            let failure = CompileError::World(format!(
                "seeded virtual file {} is unavailable: {error}",
                path.display()
            ));
            self.record_fatal(failure.clone());
            failure.to_string()
        })
    }

    fn record_fatal(&mut self, failure: CompileError) {
        if self.fatal.is_none() {
            self.fatal = Some(failure);
        }
    }
}

impl InputResolver<WorldInput> for VirtualFileResolver<'_> {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<WorldInput, String> {
        self.open(input, FileKind::TexInput, name, request_index)
            .map(WorldInput::from_content)
    }
}

impl FontResolver for VirtualFileResolver<'_> {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> Result<FileContent, String> {
        let Some(name) = path.to_str() else {
            let failure = CompileError::InvalidRequestedPath {
                name: path.display().to_string(),
                message: "font path is not UTF-8".to_owned(),
            };
            self.record_fatal(failure.clone());
            return Err(failure.to_string());
        };
        self.open(input, FileKind::Tfm, name, request_index)
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tex_expand::ExpansionHooks;
use tex_lex::WorldInput;
use tex_state::{FileContent, InputReadState};

use super::path::RequestedFile;
use super::{CompileError, FileKind, FileRequest, FileRequestKey, ResolvedFile, VirtualPath};

pub(super) struct VirtualRunHooks<'a> {
    user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
    resolved_files: &'a BTreeMap<FileRequestKey, ResolvedFile>,
    job_name: &'a str,
    misses: Vec<FileRequest>,
    seen: BTreeSet<FileRequestKey>,
    fatal: Option<CompileError>,
}

impl<'a> VirtualRunHooks<'a> {
    pub(super) fn new(
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, ResolvedFile>,
        job_name: &'a str,
    ) -> Self {
        Self {
            user_files,
            resolved_files,
            job_name,
            misses: Vec::new(),
            seen: BTreeSet::new(),
            fatal: None,
        }
    }

    pub(super) fn finish(self) -> (Vec<FileRequest>, Option<CompileError>) {
        (self.misses, self.fatal)
    }

    fn open<C: InputReadState>(
        &mut self,
        input: &mut C,
        kind: FileKind,
        original_name: &str,
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
                    self.misses.push(FileRequest {
                        key,
                        original_name: original_name.to_owned(),
                    });
                }
                Err(format!("{kind} file {original_name} is not cached"))
            }
        }
    }

    fn read_seeded<C: InputReadState>(
        &mut self,
        input: &mut C,
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

impl ExpansionHooks<WorldInput> for VirtualRunHooks<'_> {
    fn open_input<C: InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<WorldInput, String> {
        self.open(input, FileKind::TexInput, name)
            .map(WorldInput::from_content)
    }

    fn open_font<C: InputReadState>(
        &mut self,
        input: &mut C,
        path: &Path,
    ) -> Result<FileContent, String> {
        let Some(name) = path.to_str() else {
            let failure = CompileError::InvalidRequestedPath {
                name: path.display().to_string(),
                message: "font path is not UTF-8".to_owned(),
            };
            self.record_fatal(failure.clone());
            return Err(failure.to_string());
        };
        self.open(input, FileKind::Tfm, name)
    }

    fn job_name(&self) -> &str {
        self.job_name
    }
}

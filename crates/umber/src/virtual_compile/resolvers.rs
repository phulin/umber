use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tex_exec::FontResolver;
use tex_expand::InputResolver;
use tex_fonts::{
    AcceptedFontContainers, FontFeaturePolicy, FontPurposes, FontRequest, FontRequestKey,
    OpenTypeFont, VariationSelection,
};
use tex_lex::WorldInput;
use tex_state::{FileContent, InputReadState};

use super::path::RequestedFile;
use super::{CompileError, FileKind, FileRequest, FileRequestKey, VirtualPath};
use umber_vfs::VfsSnapshot;
pub(super) struct VirtualRunResolvers<'a> {
    input: VirtualFileResolver<'a>,
    font: VirtualFontResolver<'a>,
}

struct VirtualFileResolver<'a> {
    snapshot: &'a VfsSnapshot,
    resolved_paths: &'a BTreeMap<FileRequestKey, VirtualPath>,
    misses: Vec<(u64, FileRequest)>,
    seen: BTreeSet<FileRequestKey>,
    fatal: Option<CompileError>,
}

impl<'a> VirtualRunResolvers<'a> {
    pub(super) fn new(
        snapshot: &'a VfsSnapshot,
        resolved_paths: &'a BTreeMap<FileRequestKey, VirtualPath>,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        accepted_font_containers: AcceptedFontContainers,
        require_opentype: bool,
    ) -> Self {
        Self {
            input: VirtualFileResolver::new(snapshot, resolved_paths),
            font: VirtualFontResolver::new(
                snapshot,
                resolved_paths,
                resolved_fonts,
                accepted_font_containers,
                require_opentype,
            ),
        }
    }

    pub(super) fn resolvers(&mut self) -> (&mut dyn InputResolver, &mut dyn FontResolver) {
        (&mut self.input, &mut self.font)
    }

    pub(super) fn finish(self) -> (Vec<FileRequest>, Vec<FontRequest>, Option<CompileError>) {
        let mut misses = self.input.misses;
        misses.extend(self.font.files.misses);
        misses.sort_by_key(|(request_index, _)| *request_index);
        (
            misses.into_iter().map(|(_, request)| request).collect(),
            self.font.font_misses.into_values().collect(),
            self.input.fatal.or(self.font.files.fatal),
        )
    }
}

impl<'a> VirtualFileResolver<'a> {
    fn new(
        snapshot: &'a VfsSnapshot,
        resolved_paths: &'a BTreeMap<FileRequestKey, VirtualPath>,
    ) -> Self {
        Self {
            snapshot,
            resolved_paths,
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
                let Some(file) = self.snapshot_file(&path)? else {
                    let failure = CompileError::UnavailableAbsoluteUserFile(path.to_string());
                    self.record_fatal(failure.clone());
                    return Err(failure.to_string());
                };
                self.read_snapshot(input, file)
            }
            RequestedFile::Remote { user_path, key } => {
                if let Some(file) = self.snapshot_file(&user_path)? {
                    return self.read_snapshot(input, file);
                }
                if let Some(path) = self.resolved_paths.get(&key) {
                    let Some(file) = self.snapshot_file(path)? else {
                        let failure = CompileError::World(format!(
                            "resolved virtual file {path} is unavailable in its VFS snapshot"
                        ));
                        self.record_fatal(failure.clone());
                        return Err(failure.to_string());
                    };
                    return self.read_snapshot(input, file);
                }
                if self.seen.insert(key.clone()) {
                    self.misses
                        .push((request_index, FileRequest::new(key, original_name)));
                }
                Err(format!("{kind} file {original_name} is not cached"))
            }
        }
    }

    fn snapshot_file(
        &mut self,
        path: &VirtualPath,
    ) -> Result<Option<&'a umber_vfs::VirtualFile>, String> {
        self.snapshot.get(path).map_err(|error| {
            let failure = CompileError::World(error.to_string());
            self.record_fatal(failure.clone());
            failure.to_string()
        })
    }

    fn read_snapshot(
        &mut self,
        input: &mut dyn InputReadState,
        file: &umber_vfs::VirtualFile,
    ) -> Result<FileContent, String> {
        input
            .read_supplied_input_file(file.path().as_path(), file.shared_bytes())
            .map_err(|error| {
                let failure = CompileError::World(format!(
                    "VFS file {} could not be registered with World: {error}",
                    file.path()
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

impl InputResolver for VirtualFileResolver<'_> {
    fn open_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<Box<dyn tex_lex::InputSource>, String> {
        self.open(input, FileKind::TexInput, name, request_index)
            .map(WorldInput::from_content)
            .map(|source| Box::new(source) as Box<dyn tex_lex::InputSource>)
    }

    fn open_stream_input(
        &mut self,
        input: &mut dyn InputReadState,
        name: &str,
        request_index: u64,
    ) -> Result<Option<FileContent>, String> {
        self.open(input, FileKind::TexInput, name, request_index)
            .map(Some)
    }
}

struct VirtualFontResolver<'a> {
    files: VirtualFileResolver<'a>,
    resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
    accepted_font_containers: AcceptedFontContainers,
    require_opentype: bool,
    font_misses: BTreeMap<FontRequestKey, FontRequest>,
}

impl<'a> VirtualFontResolver<'a> {
    fn new(
        snapshot: &'a VfsSnapshot,
        resolved_paths: &'a BTreeMap<FileRequestKey, VirtualPath>,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        accepted_font_containers: AcceptedFontContainers,
        require_opentype: bool,
    ) -> Self {
        Self {
            files: VirtualFileResolver::new(snapshot, resolved_paths),
            resolved_fonts,
            accepted_font_containers,
            require_opentype,
            font_misses: BTreeMap::new(),
        }
    }
}

impl FontResolver for VirtualFontResolver<'_> {
    fn open_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> Result<tex_exec::FontSource, String> {
        let Some(name) = path.to_str() else {
            let failure = CompileError::InvalidRequestedPath {
                name: path.display().to_string(),
                message: "font path is not UTF-8".to_owned(),
            };
            self.files.record_fatal(failure.clone());
            return Err(failure.to_string());
        };
        let tfm = self.files.open(input, FileKind::Tfm, name, request_index);
        if !self.require_opentype {
            return tfm.map(|metrics| tex_exec::FontSource {
                metrics,
                opentype: None,
            });
        }
        let logical_name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name);
        let key = FontRequestKey::new(
            logical_name,
            0,
            VariationSelection::default(),
            FontFeaturePolicy::default(),
        )
        .map_err(|error| error.to_string())?;
        let Some(font) = self.resolved_fonts.get(&key) else {
            self.font_misses.entry(key.clone()).or_insert(FontRequest {
                key,
                accepted_containers: self.accepted_font_containers,
                purposes: FontPurposes::LAYOUT_AND_HTML,
            });
            return Err(format!("OpenType font {logical_name} is not cached"));
        };
        tfm.map(|metrics| tex_exec::FontSource {
            metrics,
            opentype: Some(tex_fonts::OpenTypeProgramSelection {
                program_identity: font.identity,
                object_identity: font.object_identity,
                container: font.container,
                face_index: key.face_index,
                variation: key.variation,
                features: key.feature_policy,
                direction: tex_fonts::WritingDirection::LeftToRight,
            }),
        })
    }
}

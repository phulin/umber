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
use super::{CachedFile, CompileError, FileKind, FileRequest, FileRequestKey, VirtualPath};

pub(super) struct VirtualRunResolvers<'a> {
    input: VirtualFileResolver<'a>,
    font: VirtualFontResolver<'a>,
}

struct VirtualFileResolver<'a> {
    user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
    resolved_files: &'a BTreeMap<FileRequestKey, CachedFile>,
    misses: Vec<(u64, FileRequest)>,
    seen: BTreeSet<FileRequestKey>,
    fatal: Option<CompileError>,
}

impl<'a> VirtualRunResolvers<'a> {
    pub(super) fn new(
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, CachedFile>,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        accepted_font_containers: AcceptedFontContainers,
        require_opentype: bool,
    ) -> Self {
        Self {
            input: VirtualFileResolver::new(user_files, resolved_files),
            font: VirtualFontResolver::new(
                user_files,
                resolved_files,
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
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, CachedFile>,
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
        user_files: &'a BTreeMap<VirtualPath, Vec<u8>>,
        resolved_files: &'a BTreeMap<FileRequestKey, CachedFile>,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        accepted_font_containers: AcceptedFontContainers,
        require_opentype: bool,
    ) -> Self {
        Self {
            files: VirtualFileResolver::new(user_files, resolved_files),
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

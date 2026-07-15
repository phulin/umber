use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use tex_exec::{FontResolver, PdfImagePageBox, PdfImageRequest, PdfImageResolver};
use tex_expand::InputResolver;
use tex_fonts::{
    AcceptedFontContainers, FontFeaturePolicy, FontPurposes, FontRequest, FontRequestKey,
    OpenTypeFont, VariationSelection,
};
use tex_lex::WorldInput;
use tex_state::scaled::Scaled;
use tex_state::{
    FileContent, InputReadState, PdfExternalImageMetadata, PdfExternalImageSource, PdfPageBox,
    PdfRasterColorSpace, PdfRasterFormat, PdfRasterImageMetadata,
};

use super::path::RequestedFile;
use super::{CompileError, FileKind, FileRequest, FileRequestKey, VirtualPath};
use umber_vfs::VfsSnapshot;
pub(super) struct VirtualRunResolvers<'a> {
    input: VirtualFileResolver<'a>,
    font: VirtualFontResolver<'a>,
    image: VirtualImageResolver<'a>,
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
            image: VirtualImageResolver {
                files: VirtualFileResolver::new(snapshot, resolved_paths),
            },
        }
    }

    pub(super) fn resolvers(
        &mut self,
    ) -> (
        &mut dyn InputResolver,
        &mut dyn FontResolver,
        &mut dyn PdfImageResolver,
    ) {
        (&mut self.input, &mut self.font, &mut self.image)
    }

    pub(super) fn finish(self) -> (Vec<FileRequest>, Vec<FontRequest>, Option<CompileError>) {
        let mut misses = self.input.misses;
        misses.extend(self.font.files.misses);
        misses.extend(self.image.files.misses);
        misses.sort_by_key(|(request_index, _)| *request_index);
        (
            misses.into_iter().map(|(_, request)| request).collect(),
            self.font.font_misses.into_values().collect(),
            self.input
                .fatal
                .or(self.font.files.fatal)
                .or(self.image.files.fatal),
        )
    }
}

struct VirtualImageResolver<'a> {
    files: VirtualFileResolver<'a>,
}

impl PdfImageResolver for VirtualImageResolver<'_> {
    fn open_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
        request_index: u64,
    ) -> Result<PdfExternalImageSource, String> {
        let content = self
            .files
            .open(input, FileKind::Image, &request.name, request_index)?;
        parse_image(&content, request)
    }
}

fn parse_image(
    content: &FileContent,
    request: &PdfImageRequest,
) -> Result<PdfExternalImageSource, String> {
    let bytes = content.bytes();
    if bytes.starts_with(b"%PDF-") {
        return parse_pdf_image(content, request);
    }
    let metadata = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() < 29 || &bytes[12..16] != b"IHDR" {
            return Err("invalid PNG header".to_owned());
        }
        let color_type = bytes[25];
        let (color_space, alpha) = match color_type {
            0 => (PdfRasterColorSpace::Gray, false),
            2 => (PdfRasterColorSpace::Rgb, false),
            3 => (PdfRasterColorSpace::Rgb, png_has_chunk(bytes, b"tRNS")),
            4 => (PdfRasterColorSpace::Gray, true),
            6 => (PdfRasterColorSpace::Rgb, true),
            _ => return Err(format!("unsupported PNG color type {color_type}")),
        };
        PdfRasterImageMetadata {
            format: PdfRasterFormat::Png,
            width: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            height: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
            bits_per_component: bytes[24],
            color_space,
            alpha,
            png_color_type: Some(color_type),
        }
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        let (width, height, bits, components) = jpeg_dimensions(bytes)?;
        PdfRasterImageMetadata {
            format: PdfRasterFormat::Jpeg,
            width,
            height,
            bits_per_component: bits,
            color_space: match components {
                1 => PdfRasterColorSpace::Gray,
                3 => PdfRasterColorSpace::Rgb,
                4 => PdfRasterColorSpace::Cmyk,
                _ => return Err(format!("unsupported JPEG component count {components}")),
            },
            alpha: false,
            png_color_type: None,
        }
    } else {
        return Err("image type is not PDF, PNG, or JPEG".to_owned());
    };
    if request.page != 1 {
        return Err("raster images have only page 1".to_owned());
    }
    Ok(PdfExternalImageSource {
        identity: content.hash(),
        metadata: PdfExternalImageMetadata::Raster(metadata),
        natural_width: pixels_to_scaled(metadata.width),
        natural_height: pixels_to_scaled(metadata.height),
        bytes: content.shared_bytes(),
    })
}

fn parse_pdf_image(
    content: &FileContent,
    request: &PdfImageRequest,
) -> Result<PdfExternalImageSource, String> {
    let document = lopdf::Document::load_mem(content.bytes()).map_err(|error| error.to_string())?;
    let page_id = document
        .get_pages()
        .get(&request.page)
        .copied()
        .ok_or_else(|| format!("page {} does not exist", request.page))?;
    let keys: &[&[u8]] = match request.page_box {
        PdfImagePageBox::Media => &[b"MediaBox"],
        PdfImagePageBox::Crop => &[b"CropBox", b"MediaBox"],
        PdfImagePageBox::Bleed => &[b"BleedBox", b"CropBox", b"MediaBox"],
        PdfImagePageBox::Trim => &[b"TrimBox", b"CropBox", b"MediaBox"],
        PdfImagePageBox::Art => &[b"ArtBox", b"CropBox", b"MediaBox"],
    };
    let coordinates = keys
        .iter()
        .find_map(|key| inherited_box(&document, page_id, key).transpose())
        .transpose()?
        .ok_or_else(|| "selected PDF page box is missing".to_owned())?;
    let page_box = PdfPageBox {
        left: pdf_points_to_scaled(coordinates[0]),
        bottom: pdf_points_to_scaled(coordinates[1]),
        right: pdf_points_to_scaled(coordinates[2]),
        top: pdf_points_to_scaled(coordinates[3]),
    };
    Ok(PdfExternalImageSource {
        identity: content.hash(),
        metadata: PdfExternalImageMetadata::PdfPage { page_box },
        natural_width: page_box.right - page_box.left,
        natural_height: page_box.top - page_box.bottom,
        bytes: content.shared_bytes(),
    })
}

fn png_has_chunk(bytes: &[u8], wanted: &[u8; 4]) -> bool {
    let mut cursor = 8usize;
    while cursor + 12 <= bytes.len() {
        let length = u32::from_be_bytes([
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ]) as usize;
        if &bytes[cursor + 4..cursor + 8] == wanted {
            return true;
        }
        let Some(next) = cursor.checked_add(length + 12) else {
            return false;
        };
        if next > bytes.len() {
            return false;
        }
        cursor = next;
    }
    false
}

fn inherited_box(
    document: &lopdf::Document,
    mut id: lopdf::ObjectId,
    key: &[u8],
) -> Result<Option<[f64; 4]>, String> {
    loop {
        let dictionary = document
            .get_dictionary(id)
            .map_err(|error| error.to_string())?;
        if let Ok(value) = dictionary.get(key) {
            let (_, value) = document
                .dereference(value)
                .map_err(|error| error.to_string())?;
            let values = value.as_array().map_err(|error| error.to_string())?;
            if values.len() != 4 {
                return Err("PDF page box must contain four numbers".to_owned());
            }
            let mut result = [0.0; 4];
            for (slot, value) in result.iter_mut().zip(values) {
                *slot = f64::from(value.as_float().map_err(|error| error.to_string())?);
            }
            return Ok(Some(result));
        }
        let Ok(parent) = dictionary.get(b"Parent") else {
            return Ok(None);
        };
        id = parent.as_reference().map_err(|error| error.to_string())?;
    }
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32, u8, u8), String> {
    let mut cursor = 2;
    while cursor + 4 <= bytes.len() {
        if bytes[cursor] != 0xff {
            cursor += 1;
            continue;
        }
        let marker = bytes[cursor + 1];
        cursor += 2;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if cursor + 2 > bytes.len() {
            break;
        }
        let length = usize::from(u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]));
        if length < 2 || cursor + length > bytes.len() {
            return Err("invalid JPEG marker length".to_owned());
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 7 {
                return Err("invalid JPEG frame header".to_owned());
            }
            return Ok((
                u32::from(u16::from_be_bytes([bytes[cursor + 5], bytes[cursor + 6]])),
                u32::from(u16::from_be_bytes([bytes[cursor + 3], bytes[cursor + 4]])),
                bytes[cursor + 2],
                bytes[cursor + 7],
            ));
        }
        cursor += length;
    }
    Err("JPEG has no supported frame header".to_owned())
}

fn pixels_to_scaled(pixels: u32) -> Scaled {
    pdf_points_to_scaled(f64::from(pixels))
}

fn pdf_points_to_scaled(points: f64) -> Scaled {
    Scaled::from_raw((points * 72.27 / 72.0 * 65_536.0).round() as i32)
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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use tex_exec::{
    PdfImageRequest, ResourceFulfillment, ResourceHost, ResourceNeed, ResourceOutcome,
    ResourceWorld,
};
use tex_fonts::{
    AcceptedFontContainers, FontFeaturePolicy, FontLayoutPolicy, FontMappingFallbackPolicy,
    FontPurposes, FontRequest, FontRequestKey, LegacyEncodingMap, OpenTypeFont, VariationSelection,
};
use tex_state::scaled::Scaled;
use tex_state::{
    FileContent, InputDependencyAccess, InputDependencyOutcome, InputOrigin, InputReadState,
    PdfExternalImageMetadata, PdfExternalImageSource, PdfPageBox, PdfRasterColorSpace,
    PdfRasterFormat, PdfRasterImageMetadata,
};

use super::path::RequestedFile;
use super::{
    CompileError, FileKind, FileRequest, FileRequestKey, FontResponseFingerprint, VirtualPath,
};
use umber_vfs::{FileOrigin, ResourceLedger, VfsSnapshot};
pub(super) struct VirtualRunResolvers<'a> {
    input: VirtualFileResolver<'a>,
    font: VirtualFontResolver<'a>,
    image: VirtualImageResolver<'a>,
    request_index: u64,
}

enum HostLookup<T> {
    Available(T),
    Unavailable,
    NeedResource,
}

impl<T> HostLookup<T> {
    fn map<U>(self, operation: impl FnOnce(T) -> U) -> HostLookup<U> {
        match self {
            Self::Available(value) => HostLookup::Available(operation(value)),
            Self::Unavailable => HostLookup::Unavailable,
            Self::NeedResource => HostLookup::NeedResource,
        }
    }
}

type HostResult<T> = Result<HostLookup<T>, String>;

pub(super) struct FontResolutionPolicy<'a> {
    pub accepted_containers: AcceptedFontContainers,
    pub layout: FontLayoutPolicy,
    pub fallback: FontMappingFallbackPolicy,
    pub font_responses: &'a BTreeMap<FontRequestKey, FontResponseFingerprint>,
}

struct VirtualFileResolver<'a> {
    snapshot: &'a VfsSnapshot,
    ledger: &'a ResourceLedger,
    misses: Vec<(u64, FileRequest)>,
    probes: Vec<(u64, FileRequest)>,
    seen: BTreeSet<FileRequestKey>,
    fatal: Option<CompileError>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FileOpenIntent {
    Required,
    Probe,
}

impl<'a> VirtualRunResolvers<'a> {
    pub(super) fn new(
        snapshot: &'a VfsSnapshot,
        ledger: &'a ResourceLedger,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        unavailable_fonts: &'a BTreeSet<FontRequestKey>,
        policy: FontResolutionPolicy<'a>,
    ) -> Self {
        Self {
            input: VirtualFileResolver::from_ledger(snapshot, ledger),
            font: VirtualFontResolver::new(
                snapshot,
                ledger,
                resolved_fonts,
                unavailable_fonts,
                policy,
            ),
            image: VirtualImageResolver {
                files: VirtualFileResolver::from_ledger(snapshot, ledger),
                cache: HashMap::new(),
            },
            request_index: 0,
        }
    }

    pub(super) fn finish(
        self,
    ) -> (
        Vec<FileRequest>,
        Vec<FileRequest>,
        Vec<FontRequest>,
        Option<CompileError>,
    ) {
        let mut misses = self.input.misses;
        misses.extend(self.font.files.misses);
        misses.extend(self.image.files.misses);
        misses.sort_by_key(|(request_index, _)| *request_index);
        let mut probes = self.input.probes;
        probes.extend(self.font.files.probes);
        probes.extend(self.image.files.probes);
        probes.sort_by_key(|(request_index, _)| *request_index);
        let probe_frontier = probes.first().map(|(request_index, _)| *request_index);
        if let Some(probe_frontier) = probe_frontier {
            probes.retain(|(request_index, _)| *request_index == probe_frontier);
            misses.retain(|(request_index, _)| *request_index < probe_frontier);
        }
        let font_misses = self
            .font
            .font_misses
            .into_values()
            .filter_map(|(request_index, request)| {
                (probe_frontier.is_none_or(|frontier| request_index < frontier)).then_some(request)
            })
            .collect();
        (
            misses.into_iter().map(|(_, request)| request).collect(),
            probes.into_iter().map(|(_, request)| request).collect(),
            font_misses,
            self.input
                .fatal
                .or(self.font.files.fatal)
                .or(self.image.files.fatal),
        )
    }
}

impl ResourceHost for VirtualRunResolvers<'_> {
    fn fulfill(&mut self, world: &mut ResourceWorld<'_>, need: &ResourceNeed) -> ResourceOutcome {
        let request_index = self.request_index;
        self.request_index = self.request_index.saturating_add(1);
        let result = match need {
            ResourceNeed::Input {
                name,
                original_name,
            } => world
                .with_input_read_state(|input| {
                    self.input
                        .open(input, FileKind::TexInput, original_name, request_index)
                })
                .map(|lookup| {
                    lookup.map(|content| {
                        let role = virtual_source_role(self.input.snapshot, &content);
                        ResourceFulfillment::world_input_with_role(name, content, role)
                    })
                }),
            ResourceNeed::InputProbe { request } => world
                .with_input_read_state(|input| {
                    self.input
                        .probe(input, FileKind::TexInput, &request.name, request_index)
                })
                .map(|lookup| {
                    lookup.map(|content| {
                        ResourceFulfillment::world_input_probe(request.clone(), content)
                    })
                }),
            ResourceNeed::Font { request } => world
                .with_input_read_state(|input| {
                    self.font
                        .open_command_font(input, Path::new(&request.name), request_index)
                })
                .map(|lookup| {
                    lookup.map(|source| ResourceFulfillment::Font {
                        request: request.clone(),
                        resource: Box::new(source),
                    })
                }),
            ResourceNeed::PdfImage { request } => world
                .with_input_read_state(|input| {
                    self.image.open_command_image(
                        input,
                        &output_image_request(request),
                        request_index,
                    )
                })
                .map(|lookup| {
                    lookup.map(|resource| ResourceFulfillment::PdfImage {
                        request: request.clone(),
                        resource: Box::new(resource),
                    })
                }),
        };
        match result {
            Ok(HostLookup::Available(fulfillment)) => ResourceOutcome::Fulfilled(fulfillment),
            Ok(HostLookup::Unavailable) => ResourceOutcome::Unavailable,
            Ok(HostLookup::NeedResource) => ResourceOutcome::Declined,
            Err(error) => {
                match need {
                    ResourceNeed::Input { .. } => {
                        self.input.record_fatal(CompileError::World(error))
                    }
                    ResourceNeed::InputProbe { .. } => {
                        self.input.record_fatal(CompileError::World(error))
                    }
                    ResourceNeed::Font { .. } => {
                        self.font.files.record_fatal(CompileError::Font(error))
                    }
                    ResourceNeed::PdfImage { .. } => {
                        self.image.files.record_fatal(CompileError::Output(error))
                    }
                }
                ResourceOutcome::Declined
            }
        }
    }
}

fn virtual_source_role(snapshot: &VfsSnapshot, content: &FileContent) -> tex_command::SourceRole {
    let Some(path) = content.path().to_str() else {
        return tex_command::SourceRole::GeneratedInput;
    };
    let virtual_path = if path.starts_with("/texlive/") {
        umber_vfs::VirtualPath::distribution(path).ok()
    } else {
        umber_vfs::VirtualPath::user(path).ok()
    };
    let origin = virtual_path
        .as_ref()
        .and_then(|path| snapshot.get(path).ok().flatten())
        .map(umber_vfs::VirtualFile::origin);
    match origin {
        Some(FileOrigin::Resolved(_)) => tex_command::SourceRole::DistributionPackageClass,
        Some(FileOrigin::Generated) | None => tex_command::SourceRole::GeneratedInput,
        Some(FileOrigin::User) if is_package_class_path(content.path()) => {
            tex_command::SourceRole::ProjectPackageClass
        }
        Some(FileOrigin::User) => tex_command::SourceRole::UserDocumentInclude,
    }
}

fn is_package_class_path(path: &Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| matches!(extension, "sty" | "cls"))
}

fn output_image_request(request: &tex_command::PdfImageRequest) -> PdfImageRequest {
    PdfImageRequest {
        name: request.name.clone(),
        page: match &request.page {
            tex_command::PdfImagePageSelection::Number(page) => {
                tex_exec::PdfImagePageSelection::Number(u32::try_from(*page).unwrap_or_default())
            }
            tex_command::PdfImagePageSelection::Named(name) => {
                tex_exec::PdfImagePageSelection::Named(name.clone())
            }
        },
        color_space_object: request.color_space_object,
        page_box: match request.page_box {
            tex_command::PdfImagePageBox::Crop => tex_exec::PdfImagePageBox::Crop,
            tex_command::PdfImagePageBox::Media => tex_exec::PdfImagePageBox::Media,
            tex_command::PdfImagePageBox::Bleed => tex_exec::PdfImagePageBox::Bleed,
            tex_command::PdfImagePageBox::Trim => tex_exec::PdfImagePageBox::Trim,
            tex_command::PdfImagePageBox::Art => tex_exec::PdfImagePageBox::Art,
        },
        resolution: request.resolution,
    }
}

struct VirtualImageResolver<'a> {
    files: VirtualFileResolver<'a>,
    cache: HashMap<PdfImageRequest, PdfExternalImageSource>,
}

impl VirtualImageResolver<'_> {
    fn open_command_image(
        &mut self,
        input: &mut dyn InputReadState,
        request: &PdfImageRequest,
        request_index: u64,
    ) -> HostResult<tex_command::PdfImageResource> {
        if let Some(source) = self.cache.get(request) {
            return Ok(HostLookup::Available(
                tex_command::PdfImageResource::Available(source.clone()),
            ));
        }
        match self
            .files
            .open(input, FileKind::Image, &request.name, request_index)?
        {
            HostLookup::Available(content) => {
                let source = match parse_image(&content, request) {
                    Ok(source) => source,
                    Err(error) => {
                        return Ok(HostLookup::Available(
                            tex_command::PdfImageResource::Invalid(error),
                        ));
                    }
                };
                if content.origin() == InputOrigin::External {
                    self.cache.insert(request.clone(), source.clone());
                }
                Ok(HostLookup::Available(
                    tex_command::PdfImageResource::Available(source),
                ))
            }
            HostLookup::Unavailable => Ok(HostLookup::Unavailable),
            HostLookup::NeedResource => Ok(HostLookup::NeedResource),
        }
    }
}

pub(crate) fn parse_image(
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
    if request.page != tex_exec::PdfImagePageSelection::Number(1) {
        return Err("raster images have only page 1".to_owned());
    }
    let (x_resolution, y_resolution) = match metadata.format {
        PdfRasterFormat::Png => png_resolution(bytes).unwrap_or((0, 0)),
        PdfRasterFormat::Jpeg => (0, 0),
    };
    let (x_resolution, y_resolution) =
        effective_raster_resolution(x_resolution, y_resolution, request.resolution);
    Ok(PdfExternalImageSource {
        identity: content.hash(),
        metadata: PdfExternalImageMetadata::Raster(metadata),
        natural_width: pixels_to_scaled(metadata.width, x_resolution),
        natural_height: pixels_to_scaled(metadata.height, y_resolution),
        bytes: content.shared_bytes(),
    })
}

fn parse_pdf_image(
    content: &FileContent,
    request: &PdfImageRequest,
) -> Result<PdfExternalImageSource, String> {
    let inspected = crate::pdf_import::inspect_pdf_page(
        content.shared_bytes(),
        &request.page,
        request.page_box,
    )?;
    let coordinates = inspected.page_box;
    let page_box = PdfPageBox {
        left: pdf_points_to_scaled(coordinates[0]),
        bottom: pdf_points_to_scaled(coordinates[1]),
        right: pdf_points_to_scaled(coordinates[2]),
        top: pdf_points_to_scaled(coordinates[3]),
    };
    let rotation = inspected.rotation;
    let box_width = page_box.right - page_box.left;
    let box_height = page_box.top - page_box.bottom;
    let (natural_width, natural_height) = if rotation.swaps_axes() {
        (box_height, box_width)
    } else {
        (box_width, box_height)
    };
    Ok(PdfExternalImageSource {
        identity: content.hash(),
        metadata: PdfExternalImageMetadata::PdfPage {
            page_box,
            rotation,
            page: inspected.page_number,
            total_pages: inspected.total_pages,
            has_page_group: inspected.has_page_group,
            pdf_version: inspected.pdf_version,
        },
        natural_width,
        natural_height,
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

/// Returns libpng's metre-based `pHYs` density as rounded dots per inch.
///
/// pdfTeX's `read_png_info` uses `round(0.0254 * pixels_per_meter)`, and
/// pdftex.web §1553 uses the live `\pdfimageresolution` only when either
/// intrinsic axis is absent.
fn png_resolution(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut cursor = 8usize;
    while cursor.checked_add(12)? <= bytes.len() {
        let length = usize::try_from(u32::from_be_bytes(
            bytes.get(cursor..cursor + 4)?.try_into().ok()?,
        ))
        .ok()?;
        let data_start = cursor.checked_add(8)?;
        let data_end = data_start.checked_add(length)?;
        let next = data_end.checked_add(4)?;
        if next > bytes.len() {
            return None;
        }
        if &bytes[cursor + 4..cursor + 8] == b"pHYs" && length == 9 {
            let data = &bytes[data_start..data_end];
            if data[8] != 1 {
                return None;
            }
            let x = u32::from_be_bytes(data[0..4].try_into().ok()?);
            let y = u32::from_be_bytes(data[4..8].try_into().ok()?);
            let to_dpi = |pixels_per_metre: u32| {
                u32::try_from((u64::from(pixels_per_metre) * 254 + 5_000) / 10_000).ok()
            };
            return Some((to_dpi(x)?, to_dpi(y)?));
        }
        cursor = next;
    }
    None
}

fn effective_raster_resolution(x: u32, y: u32, fallback: u32) -> (u32, u32) {
    if fallback > 0 && (x == 0 || y == 0) {
        (fallback, fallback)
    } else {
        (x, y)
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

fn pixels_to_scaled(pixels: u32, resolution: u32) -> Scaled {
    let resolution = if resolution == 0 { 72 } else { resolution };
    pdf_points_to_scaled(f64::from(pixels) * 72.0 / f64::from(resolution))
}

fn pdf_points_to_scaled(points: f64) -> Scaled {
    Scaled::from_raw((points * 72.27 / 72.0 * 65_536.0).round() as i32)
}

impl<'a> VirtualFileResolver<'a> {
    fn from_ledger(snapshot: &'a VfsSnapshot, ledger: &'a ResourceLedger) -> Self {
        Self {
            snapshot,
            ledger,
            misses: Vec::new(),
            probes: Vec::new(),
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
    ) -> HostResult<FileContent> {
        self.open_classified(
            input,
            kind,
            original_name,
            request_index,
            FileOpenIntent::Required,
        )
    }

    fn probe(
        &mut self,
        input: &mut dyn InputReadState,
        kind: FileKind,
        original_name: &str,
        request_index: u64,
    ) -> HostResult<FileContent> {
        self.open_classified(
            input,
            kind,
            original_name,
            request_index,
            FileOpenIntent::Probe,
        )
    }

    fn open_classified(
        &mut self,
        input: &mut dyn InputReadState,
        kind: FileKind,
        original_name: &str,
        request_index: u64,
        intent: FileOpenIntent,
    ) -> HostResult<FileContent> {
        let requested = match RequestedFile::parse(kind, original_name) {
            Ok(requested) => requested,
            Err(error) => {
                if intent == FileOpenIntent::Probe {
                    return Ok(HostLookup::Unavailable);
                }
                let failure = CompileError::InvalidRequestedPath {
                    name: original_name.to_owned(),
                    message: error.to_string(),
                };
                self.record_fatal(failure.clone());
                return Err(failure.to_string());
            }
        };
        let pending_path = match &requested {
            RequestedFile::UserOnly(path) => path.as_path(),
            RequestedFile::Remote { key, .. } => Path::new(key.name()),
        };
        if let Some(content) = self.read_pending_output(input, pending_path)? {
            return Ok(HostLookup::Available(content));
        }

        match requested {
            RequestedFile::UserOnly(path) => {
                let Some(file) = self.snapshot_file(&path)? else {
                    self.record_missing(input, &path, intent)?;
                    if intent == FileOpenIntent::Probe {
                        return Ok(HostLookup::Unavailable);
                    }
                    let failure = CompileError::UnavailableAbsoluteUserFile(path.to_string());
                    self.record_fatal(failure.clone());
                    return Err(failure.to_string());
                };
                self.read_snapshot(input, file, intent)
                    .map(HostLookup::Available)
            }
            RequestedFile::Remote { user_path, key } => {
                let missing_user_path = if let Some(user_path) = user_path {
                    if let Some(file) = self.snapshot_file(&user_path)? {
                        return self
                            .read_snapshot(input, file, intent)
                            .map(HostLookup::Available);
                    }
                    Some(user_path)
                } else {
                    None
                };
                if let Some(path) = self.ledger.resolved_path(&key) {
                    if let Some(path) = &missing_user_path {
                        self.record_missing(input, path, intent)?;
                    }
                    let Some(file) = self.snapshot_file(path)? else {
                        let failure = CompileError::World(format!(
                            "resolved virtual file {path} is unavailable in its VFS snapshot"
                        ));
                        self.record_fatal(failure.clone());
                        return Err(failure.to_string());
                    };
                    return self
                        .read_snapshot(input, file, intent)
                        .map(HostLookup::Available);
                }
                if self.ledger.is_unavailable(&key) {
                    if let Some(path) = &missing_user_path {
                        self.record_missing(input, path, intent)?;
                    }
                    return Ok(HostLookup::Unavailable);
                }
                let request = FileRequest::new(key.clone(), original_name);
                if self.seen.insert(key.clone()) {
                    if intent == FileOpenIntent::Probe {
                        self.probes.push((request_index, request));
                    } else {
                        self.misses.push((request_index, request));
                    }
                } else if intent == FileOpenIntent::Required
                    && let Some(position) = self
                        .probes
                        .iter()
                        .position(|(_, existing)| existing.key() == &key)
                {
                    let (probe_index, _) = self.probes.swap_remove(position);
                    self.misses.push((probe_index, request));
                }
                Ok(HostLookup::NeedResource)
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

    fn read_pending_output(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
    ) -> Result<Option<FileContent>, String> {
        input.read_pending_output_file(path).map_err(|error| {
            let failure = CompileError::World(error.to_string());
            self.record_fatal(failure.clone());
            failure.to_string()
        })
    }

    fn read_snapshot(
        &mut self,
        input: &mut dyn InputReadState,
        file: &umber_vfs::VirtualFile,
        intent: FileOpenIntent,
    ) -> Result<FileContent, String> {
        let content = input
            .read_supplied_input_file(file.path().as_path(), file.shared_bytes())
            .map_err(|error| {
                let failure = CompileError::World(format!(
                    "VFS file {} could not be registered with World: {error}",
                    file.path()
                ));
                self.record_fatal(failure.clone());
                failure.to_string()
            })?;
        input
            .record_input_dependency(
                file.path().as_path(),
                InputDependencyOutcome::Present(content.hash()),
                dependency_access(intent),
            )
            .map_err(|error| {
                let failure = CompileError::World(error.to_string());
                self.record_fatal(failure.clone());
                failure.to_string()
            })?;
        Ok(content)
    }

    fn record_missing(
        &mut self,
        input: &mut dyn InputReadState,
        path: &VirtualPath,
        intent: FileOpenIntent,
    ) -> Result<(), String> {
        input
            .record_input_dependency(
                path.as_path(),
                InputDependencyOutcome::Missing,
                dependency_access(intent),
            )
            .map_err(|error| {
                let failure = CompileError::World(error.to_string());
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

const fn dependency_access(intent: FileOpenIntent) -> InputDependencyAccess {
    match intent {
        FileOpenIntent::Required => InputDependencyAccess::RequiredRead,
        FileOpenIntent::Probe => InputDependencyAccess::AuthoritativeProbe,
    }
}

struct VirtualFontResolver<'a> {
    files: VirtualFileResolver<'a>,
    resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
    unavailable_fonts: &'a BTreeSet<FontRequestKey>,
    accepted_font_containers: AcceptedFontContainers,
    layout_policy: FontLayoutPolicy,
    fallback: FontMappingFallbackPolicy,
    font_responses: &'a BTreeMap<FontRequestKey, FontResponseFingerprint>,
    font_misses: BTreeMap<FontRequestKey, (u64, FontRequest)>,
}

impl<'a> VirtualFontResolver<'a> {
    fn new(
        snapshot: &'a VfsSnapshot,
        ledger: &'a ResourceLedger,
        resolved_fonts: &'a BTreeMap<FontRequestKey, OpenTypeFont>,
        unavailable_fonts: &'a BTreeSet<FontRequestKey>,
        policy: FontResolutionPolicy<'a>,
    ) -> Self {
        Self {
            files: VirtualFileResolver::from_ledger(snapshot, ledger),
            resolved_fonts,
            unavailable_fonts,
            accepted_font_containers: policy.accepted_containers,
            layout_policy: policy.layout,
            fallback: policy.fallback,
            font_responses: policy.font_responses,
            font_misses: BTreeMap::new(),
        }
    }
}

impl VirtualFontResolver<'_> {
    fn open_command_font(
        &mut self,
        input: &mut dyn InputReadState,
        path: &Path,
        request_index: u64,
    ) -> HostResult<tex_command::FontResource> {
        let Some(name) = path.to_str() else {
            let failure = CompileError::InvalidRequestedPath {
                name: path.display().to_string(),
                message: "font path is not UTF-8".to_owned(),
            };
            self.files.record_fatal(failure.clone());
            return Err(failure.to_string());
        };
        let opentype_only = name.strip_prefix("opentype:");
        let tfm = if opentype_only.is_some() {
            None
        } else {
            Some(self.files.open(input, FileKind::Tfm, name, request_index))
        };
        if self.layout_policy == FontLayoutPolicy::ClassicTfmExact && opentype_only.is_none() {
            return tfm
                .expect("classic selection has a TFM request")
                .map(|lookup| {
                    lookup.map(|metrics| tex_command::FontResource::Tfm {
                        metrics,
                        opentype: None,
                    })
                });
        }
        let tfm_content = match tfm {
            Some(Ok(HostLookup::Available(metrics))) => Some(metrics),
            Some(other) => return other.map(|lookup| lookup.map(|_| unreachable!())),
            None => None,
        };
        let logical_name = opentype_only.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(name)
        });
        let key = FontRequestKey::new(
            logical_name,
            0,
            VariationSelection::default(),
            FontFeaturePolicy::default(),
        )
        .map_err(|error| error.to_string())?;
        let Some(font) = self.resolved_fonts.get(&key) else {
            if self.unavailable_fonts.contains(&key) {
                return if let Some(metrics) = tfm_content {
                    match self.fallback {
                        FontMappingFallbackPolicy::ClassicTfmExact => Ok(HostLookup::Available(
                            tex_command::FontResource::ClassicTfmFallback { metrics },
                        )),
                        FontMappingFallbackPolicy::Error => Err(format!(
                            "no OpenType mapping resource for {logical_name} with TFM identity {}",
                            metrics.hash().hex()
                        )),
                    }
                } else {
                    Ok(HostLookup::Unavailable)
                };
            }
            self.font_misses.entry(key.clone()).or_insert_with(|| {
                (
                    request_index,
                    FontRequest {
                        key,
                        accepted_containers: self.accepted_font_containers,
                        purposes: FontPurposes::LAYOUT_AND_HTML,
                    },
                )
            });
            return Ok(HostLookup::NeedResource);
        };
        let mapped_bundle = self
            .font_responses
            .get(&key)
            .and_then(|response| response.legacy_mapping.as_ref());
        if let Some(metrics) = &tfm_content {
            let Some(bundle) = mapped_bundle else {
                return Err(format!(
                    "OpenType response for {logical_name} omitted its typed legacy mapping"
                ));
            };
            if bundle.tfm_ahash64 != tex_fonts::font_content_hash(metrics.bytes()) {
                return Err(format!(
                    "OpenType response for {logical_name} has the wrong TFM mapping identity"
                ));
            }
        }
        if let Some(bundle) = mapped_bundle {
            for (code, text) in bundle.encoding.iter().enumerate() {
                if let Some(text) = text
                    && text.chars().any(|scalar| font.cmap.glyph(scalar).is_none())
                {
                    return Err(format!(
                        "mapped font {logical_name} has no cmap glyph for encoding code {code:02x}"
                    ));
                }
            }
        }
        let selection = font.clone();
        match tfm_content {
            Some(metrics) => {
                let encoding_map = LegacyEncodingMap::new(
                    mapped_bundle
                        .expect("mapped TFM selection has a bundle")
                        .encoding
                        .clone(),
                )
                .map_err(str::to_owned)?;
                Ok(HostLookup::Available(
                    tex_command::FontResource::MappedTfm {
                        metrics,
                        opentype: selection,
                        encoding_map,
                    },
                ))
            }
            None => Ok(HostLookup::Available(tex_command::FontResource::OpenType(
                selection,
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FileOpenIntent, HostLookup, VirtualFileResolver, effective_raster_resolution,
        parse_pdf_image, pixels_to_scaled, png_resolution,
    };
    use crate::{
        CompileAttemptResult, EngineMode, FileKind, ResolvedFile, ResourceRequest,
        ResourceResponse, SessionOptions, VirtualCompileSession,
    };
    use test_support::pdf_fixture::{
        Dictionary as FixtureDictionary, ValidPdfFixture, array, name, reference,
    };
    use tex_exec::{PdfImagePageBox, PdfImageRequest};
    use tex_state::{PdfExternalImageMetadata, World};
    use umber_vfs::{ProjectWorkspace, VfsLimits};

    #[test]
    fn zero_image_resolution_uses_pdftexs_seventy_two_dpi_fallback() {
        assert_eq!(pixels_to_scaled(10, 0), pixels_to_scaled(10, 72));
        assert_eq!(pixels_to_scaled(10, 144), pixels_to_scaled(5, 72));
    }

    #[test]
    fn png_phys_density_precedes_the_live_fallback_resolution() {
        // pdftex.web §1553 and writepng.c's `read_png_info`: a metre-based
        // pHYs chunk supplies both natural-size axes, while a missing axis
        // makes the live \pdfimageresolution replace both.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&9_u32.to_be_bytes());
        png.extend_from_slice(b"pHYs");
        png.extend_from_slice(&3_937_u32.to_be_bytes());
        png.extend_from_slice(&7_874_u32.to_be_bytes());
        png.push(1);
        png.extend_from_slice(&[0; 4]);
        assert_eq!(png_resolution(&png), Some((100, 200)));
        assert_eq!(effective_raster_resolution(100, 200, 72), (100, 200));
        assert_eq!(effective_raster_resolution(100, 0, 144), (144, 144));
    }

    #[test]
    fn required_lookup_promotes_an_earlier_probe_without_changing_its_order() {
        let workspace = ProjectWorkspace::new(VfsLimits::default()).expect("empty workspace");
        let snapshot = workspace.snapshot();
        let mut resolver = VirtualFileResolver::from_ledger(&snapshot, workspace.resource_ledger());
        crate::with_engine_world(World::memory(), |stores| {
            assert!(matches!(
                resolver
                    .open_classified(
                        &mut stores.input_open_context(),
                        FileKind::TexInput,
                        "shared.cfg",
                        7,
                        FileOpenIntent::Probe,
                    )
                    .expect("probe lookup"),
                HostLookup::NeedResource
            ));
            assert!(matches!(
                resolver
                    .open_classified(
                        &mut stores.input_open_context(),
                        FileKind::TexInput,
                        "shared.cfg",
                        9,
                        FileOpenIntent::Required,
                    )
                    .expect("required lookup"),
                HostLookup::NeedResource
            ));
        })
        .expect("fresh universe");
        assert!(resolver.probes.is_empty());
        assert_eq!(resolver.misses.len(), 1);
        assert_eq!(resolver.misses[0].0, 7);
    }

    #[test]
    fn unresolved_probe_hides_later_speculative_fallback_miss() {
        let mut session = VirtualCompileSession::new(SessionOptions {
            engine: EngineMode::PdfTex,
            ..SessionOptions::default()
        })
        .expect("session");
        session
            .add_user_file(
                "main.tex",
                b"\\def\\empty{}\\edef\\found{\\pdffilesize{hyphen.cfg}}\\ifx\\found\\empty\\input hyphen.ltx\\else\\dump\\fi".to_vec(),
            )
            .expect("main source");

        let attempt = session.compile_attempt();
        let CompileAttemptResult::NeedResources(batch) = attempt else {
            panic!("file-size cache miss should expose its probe frontier: {attempt:?}");
        };
        assert!(
            batch.required.is_empty(),
            "fallback input after an unresolved probe is speculative"
        );
        let [ResourceRequest::File(probe)] = batch.probes.as_slice() else {
            panic!("expected exactly one file probe");
        };
        assert_eq!(probe.key().kind(), FileKind::TexInput);
        assert_eq!(probe.key().name(), "hyphen.cfg");
        session
            .provide_resources(vec![ResourceResponse::File(ResolvedFile {
                request: probe.key().clone(),
                virtual_path: "/texlive/hyphen.cfg".into(),
                bytes: b"cfg".to_vec().into(),
                expected_digest: None,
            })])
            .expect("positive probe response");

        assert!(matches!(
            session.compile_attempt(),
            CompileAttemptResult::Complete(_)
        ));
        assert!(
            session
                .into_accepted_finalization()
                .expect("accepted format finalization")
                .format_dump
                .is_some()
        );
    }

    #[test]
    fn unresolved_probe_hides_later_branch_dependent_probe() {
        let mut session = VirtualCompileSession::new(SessionOptions::default()).expect("session");
        session
            .add_user_file(
                "main.tex",
                b"\\openin0=first.cfg \\ifeof0 \\openin1=second.cfg \\fi \\dump".to_vec(),
            )
            .expect("main source");

        let CompileAttemptResult::NeedResources(batch) = session.compile_attempt() else {
            panic!("openin cache miss should expose its probe frontier");
        };
        assert!(batch.required.is_empty());
        let [ResourceRequest::File(probe)] = batch.probes.as_slice() else {
            panic!("only the earliest unresolved probe may escape the attempt");
        };
        assert_eq!(probe.key().name(), "first.cfg");
    }

    #[test]
    fn inherited_quarter_turn_swaps_pdf_page_natural_dimensions() {
        let mut document = ValidPdfFixture::new("1.5").expect("create rotated PDF");
        document
            .add_dictionary(
                1,
                FixtureDictionary::new()
                    .entry("Type", name("Catalog"))
                    .entry("Pages", reference(2)),
            )
            .expect("catalog");
        document
            .add_dictionary(
                2,
                FixtureDictionary::new()
                    .entry("Type", name("Pages"))
                    .entry("Kids", array([reference(3)]))
                    .entry("Count", b"1")
                    .entry("Rotate", b"90"),
            )
            .expect("page tree");
        document
            .add_dictionary(
                3,
                FixtureDictionary::new()
                    .entry("Type", name("Page"))
                    .entry("Parent", reference(2))
                    .entry("MediaBox", b"[0 0 10 20]")
                    .entry("Resources", b"<<>>")
                    .entry("Contents", reference(4)),
            )
            .expect("page");
        document
            .add_stream(4, FixtureDictionary::new(), b"")
            .expect("contents");
        document
            .set_trailer_entry("Root", reference(1))
            .expect("root");
        let bytes = document.finish().expect("serialize rotated PDF");

        let mut world = World::default();
        world
            .set_memory_file("rotated.pdf", bytes)
            .expect("seed rotated PDF");
        let content = world.read_file("rotated.pdf").expect("read rotated PDF");
        let acquired_bytes = content.shared_bytes();
        let source = parse_pdf_image(
            &content,
            &PdfImageRequest {
                name: "rotated.pdf".to_owned(),
                page: tex_exec::PdfImagePageSelection::Number(1),
                color_space_object: 0,
                page_box: PdfImagePageBox::Media,
                resolution: 0,
            },
        )
        .expect("parse rotated PDF");
        assert!(tex_state::SharedBytes::ptr_eq(
            &source.bytes,
            &acquired_bytes
        ));
        let PdfExternalImageMetadata::PdfPage {
            page_box, rotation, ..
        } = source.metadata
        else {
            panic!("expected PDF-page metadata");
        };
        assert_eq!(rotation, tex_state::PdfPageRotation::Clockwise90);
        assert_eq!(source.natural_width, page_box.top - page_box.bottom);
        assert_eq!(source.natural_height, page_box.right - page_box.left);
    }
}

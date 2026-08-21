use std::collections::{BTreeMap, BTreeSet};

use tex_fonts::{TfmFont, VfProgram};
use tex_state::{DetachedPdfFontOperation, PdfFontMapOperation};
use umber_vfs::{FileContentId, ProjectWorkspace};

use super::{FileKind, FileRequest, FileRequestKey, ResolvedPkFont, ResourceRequest};

#[derive(Clone, Debug)]
pub struct CachedVirtualFont {
    pub content_id: FileContentId,
    pub program: VfProgram,
}

#[derive(Clone, Debug)]
pub struct CachedLocalTfm {
    pub content_id: FileContentId,
    /// Exact bytes retained so detached finalization can instantiate this TFM
    /// at the size declared by each containing virtual font.
    pub bytes: Vec<u8>,
    pub font: TfmFont,
}

/// Immutable resources discovered after a PDF-mode engine candidate reaches
/// completion. Packet lowering consumes this cache only after acceptance.
#[derive(Clone, Debug, Default)]
pub struct PdfVirtualFontResources {
    pub virtual_fonts: BTreeMap<String, CachedVirtualFont>,
    pub local_tfms: BTreeMap<String, CachedLocalTfm>,
    pub(crate) font_maps: BTreeMap<Vec<u8>, tex_fonts::PdfFontMap>,
    pub(crate) encodings: BTreeMap<Vec<u8>, tex_fonts::PdfEncoding>,
    pub(crate) type1_programs: BTreeMap<Vec<u8>, tex_fonts::PdfType1Program>,
    pub(crate) truetype_programs: BTreeMap<Vec<u8>, tex_fonts::PdfTrueTypeProgram>,
    pub(crate) pk_fonts: BTreeMap<tex_fonts::PdfPkFontRequest, tex_fonts::PdfPkFont>,
}

pub(super) struct Discovery {
    pub required: Vec<ResourceRequest>,
    pub probes: Vec<ResourceRequest>,
    pub observed_files: Vec<FileRequest>,
    pub observed_pk_fonts: Vec<tex_fonts::PdfPkFontRequest>,
}

pub(super) fn discover(
    discovery: tex_incr::CompletionResourceDiscovery<'_>,
    files: &ProjectWorkspace,
    cache: &mut PdfVirtualFontResources,
    pk_fonts: &BTreeMap<tex_fonts::PdfPkFontRequest, ResolvedPkFont>,
    unavailable_pk_fonts: &BTreeSet<tex_fonts::PdfPkFontRequest>,
) -> Result<Discovery, String> {
    let mut required = BTreeMap::<FileRequestKey, FileRequest>::new();
    let mut probes = BTreeMap::<FileRequestKey, FileRequest>::new();
    let mut observed_files = BTreeMap::<FileRequestKey, FileRequest>::new();
    let mut observed_pk_fonts = BTreeSet::new();
    let Some(pdf) = discovery.pdf() else {
        return Ok(Discovery {
            required: Vec::new(),
            probes: Vec::new(),
            observed_files: Vec::new(),
            observed_pk_fonts: Vec::new(),
        });
    };
    let mut fonts = pdf
        .fonts()
        .iter()
        .filter(|resource| resource.recipe.opentype.is_none())
        .map(|resource| resource.recipe.name.clone())
        .collect::<BTreeSet<_>>();
    if fonts.is_empty() {
        return Ok(Discovery {
            required: Vec::new(),
            probes: Vec::new(),
            observed_files: Vec::new(),
            observed_pk_fonts: Vec::new(),
        });
    }
    let mut real_fonts = BTreeSet::new();
    let mut visited = BTreeSet::new();

    while let Some(name) = fonts.pop_first() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let vf_request = request(FileKind::VirtualFont, &name, "vf")?;
        observed_files.insert(vf_request.key().clone(), vf_request.clone());
        if files.is_unavailable(vf_request.key()) {
            real_fonts.insert(name);
            continue;
        }
        let Some(file) = files.get(vf_request.key()) else {
            probes.insert(vf_request.key().clone(), vf_request);
            continue;
        };
        if !cache.virtual_fonts.contains_key(&name) {
            let program = VfProgram::parse(file.bytes())
                .map_err(|error| format!("virtual font {name}: {error}"))?;
            cache.virtual_fonts.insert(
                name.clone(),
                CachedVirtualFont {
                    content_id: file.content_id(),
                    program,
                },
            );
        }
        let program = &cache
            .virtual_fonts
            .get(&name)
            .expect("newly cached VF is present")
            .program;
        for local in program.local_fonts() {
            let logical = String::from_utf8(local.logical_name())
                .map_err(|_| format!("virtual font {name} has a non-UTF-8 local font name"))?;
            let tfm_request = request(FileKind::Tfm, &logical, "tfm")?;
            observed_files.insert(tfm_request.key().clone(), tfm_request.clone());
            if files.is_unavailable(tfm_request.key()) {
                return Err(format!(
                    "virtual font {name} requires unavailable TFM {logical}"
                ));
            }
            if let Some(file) = files.get(tfm_request.key()) {
                if !cache.local_tfms.contains_key(&logical) {
                    let font = TfmFont::parse(file.bytes())
                        .map_err(|error| format!("local TFM {logical}: {error}"))?;
                    cache.local_tfms.insert(
                        logical.clone(),
                        CachedLocalTfm {
                            content_id: file.content_id(),
                            bytes: file.bytes().to_vec(),
                            font,
                        },
                    );
                }
                fonts.insert(logical);
            } else {
                required.insert(tfm_request.key().clone(), tfm_request);
            }
        }
    }

    if !required.is_empty() || !probes.is_empty() {
        return Ok(Discovery {
            required: required.into_values().map(ResourceRequest::File).collect(),
            probes: probes.into_values().map(ResourceRequest::File).collect(),
            observed_files: observed_files.into_values().collect(),
            observed_pk_fonts: observed_pk_fonts.into_iter().collect(),
        });
    }

    for name in font_map_file_requests(pdf) {
        let name = utf8_name("PDF font map", &name)?;
        let map_request = request(FileKind::PdfFontMap, name, "map")?;
        observed_files.insert(map_request.key().clone(), map_request.clone());
        if files.is_unavailable(map_request.key()) {
            return Err(format!("required PDF font map {name} is unavailable"));
        }
        if let Some(file) = files.get(map_request.key()) {
            let map = tex_fonts::PdfFontMap::parse(file.bytes())
                .map_err(|error| format!("PDF font map {name}: {error}"))?;
            cache.font_maps.insert(name.as_bytes().to_vec(), map);
        } else {
            required.insert(map_request.key().clone(), map_request);
        }
    }
    let resolved_map_lines = resolved_font_map_lines(pdf, cache);
    for entry in resolved_map_lines
        .iter()
        .filter(|entry| real_fonts.contains(utf8_name("mapped TFM", &entry.tex_name).unwrap_or("")))
    {
        for encoding in &entry.encoding_files {
            let name = utf8_name("PDF encoding", encoding)?;
            let request = request(FileKind::PdfEncoding, name, "")?;
            observed_files.insert(request.key().clone(), request.clone());
            if let Some(file) = files.get(request.key()) {
                cache.encodings.insert(
                    name.as_bytes().to_vec(),
                    tex_fonts::PdfEncoding::parse(file.bytes())
                        .map_err(|error| format!("PDF encoding {name}: {error}"))?,
                );
            } else if files.is_unavailable(request.key()) {
                return Err(format!("required PDF encoding {name} is unavailable"));
            } else {
                required.insert(request.key().clone(), request);
            }
        }
        if let Some(program) = &entry.font_file {
            let name = utf8_name("PDF font program", program)?;
            let is_truetype = crate::pdf_output::is_pdf_sfnt_program(name.as_bytes());
            let request = request(FileKind::PdfFontProgram, name, "")?;
            observed_files.insert(request.key().clone(), request.clone());
            if let Some(file) = files.get(request.key()) {
                if is_truetype {
                    cache.truetype_programs.insert(
                        name.as_bytes().to_vec(),
                        tex_fonts::PdfTrueTypeProgram::parse(file.bytes())
                            .map_err(|error| format!("PDF font program {name}: {error}"))?,
                    );
                } else {
                    cache.type1_programs.insert(
                        name.as_bytes().to_vec(),
                        tex_fonts::PdfType1Program::from_pfb(file.bytes())
                            .map_err(|error| format!("PDF font program {name}: {error}"))?,
                    );
                }
            } else if files.is_unavailable(request.key()) {
                return Err(format!("required PDF font program {name} is unavailable"));
            } else {
                required.insert(request.key().clone(), request);
            }
        }
    }

    let mapped_names = resolved_map_lines
        .iter()
        .map(|entry| entry.tex_name.clone())
        .collect::<BTreeSet<_>>();
    let virtual_names = cache
        .virtual_fonts
        .keys()
        .map(|name| name.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let base_dpi = pdf.output_parameters().map_or(
        crate::pdf_output::DEFAULT_PDF_PK_RESOLUTION,
        |parameters| {
            if parameters.pk_resolution == 0 {
                crate::pdf_output::DEFAULT_PDF_PK_RESOLUTION
            } else {
                parameters.pk_resolution
            }
        },
    );
    let pk_requests = pdf
        .fonts()
        .iter()
        .filter(|resource| resource.recipe.opentype.is_none())
        .filter_map(|resource| {
            let font = &resource.recipe;
            (!mapped_names.contains(font.name.as_bytes())
                && !virtual_names.contains(font.name.as_bytes()))
            .then(|| detached_pk_request(font, base_dpi))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut pk_required = Vec::new();
    for request in pk_requests {
        observed_pk_fonts.insert(request.clone());
        if cache.pk_fonts.contains_key(&request) {
            continue;
        }
        if unavailable_pk_fonts.contains(&request) {
            return Err(format!(
                "required PK font {} is unavailable",
                String::from_utf8_lossy(&request.logical_name())
            ));
        }
        if let Some(resolved) = pk_fonts.get(&request) {
            cache.pk_fonts.insert(
                request.clone(),
                tex_fonts::PdfPkFont::parse(&resolved.bytes)
                    .map_err(|error| format!("PK font: {error}"))?,
            );
        } else {
            pk_required.push(ResourceRequest::PkFont(request));
        }
    }
    Ok(Discovery {
        required: required
            .into_values()
            .map(ResourceRequest::File)
            .chain(pk_required)
            .collect(),
        probes: probes.into_values().map(ResourceRequest::File).collect(),
        observed_files: observed_files.into_values().collect(),
        observed_pk_fonts: observed_pk_fonts.into_iter().collect(),
    })
}

fn font_map_file_requests(pdf: &tex_state::DetachedPdfCompletion) -> Vec<Vec<u8>> {
    let maps = pdf
        .font_operations()
        .iter()
        .filter_map(|operation| match operation {
            DetachedPdfFontOperation::Map(map) => Some(map),
            _ => None,
        })
        .collect::<Vec<_>>();
    let loads_default = maps.first().is_none_or(|operation| {
        map_directive(operation) != tex_fonts::PdfFontMapDirective::Default
    });
    let mut requests = BTreeSet::new();
    if loads_default {
        requests.insert(b"pdftex.map".to_vec());
    }
    for operation in maps {
        if let PdfFontMapOperation::File(file) = operation {
            requests.insert(file.logical_name.clone());
        }
    }
    requests.into_iter().collect()
}

pub(crate) fn resolved_font_map_lines(
    pdf: &tex_state::DetachedPdfCompletion,
    cache: &PdfVirtualFontResources,
) -> Vec<tex_fonts::PdfFontMapEntry> {
    let maps = pdf
        .font_operations()
        .iter()
        .filter_map(|operation| match operation {
            DetachedPdfFontOperation::Map(map) => Some(map),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut entries = BTreeMap::new();
    if maps
        .first()
        .is_none_or(|operation| map_directive(operation) != tex_fonts::PdfFontMapDirective::Default)
    {
        apply_map_file(
            pdf,
            cache,
            b"pdftex.map",
            tex_fonts::PdfFontMapDirective::Default,
            &mut entries,
        );
    }
    for operation in maps {
        match operation {
            PdfFontMapOperation::BlockDefault => {}
            PdfFontMapOperation::Line(entry) => apply_map_entry(entry.clone(), &mut entries),
            PdfFontMapOperation::File(file) => {
                apply_map_file(pdf, cache, &file.logical_name, file.directive, &mut entries)
            }
        }
    }
    entries.into_values().collect()
}

fn apply_map_file(
    pdf: &tex_state::DetachedPdfCompletion,
    cache: &PdfVirtualFontResources,
    logical_name: &[u8],
    directive: tex_fonts::PdfFontMapDirective,
    entries: &mut BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
) {
    let map = pdf
        .font_operations()
        .iter()
        .rev()
        .find_map(|operation| match operation {
            DetachedPdfFontOperation::MapFileContent {
                logical_name: candidate,
                map,
            } if candidate == logical_name => Some(map),
            _ => None,
        })
        .or_else(|| cache.font_maps.get(logical_name));
    let Some(map) = map else { return };
    for entry in map.entries() {
        let mut entry = entry.clone();
        entry.directive = directive;
        apply_map_entry(entry, entries);
    }
}

fn apply_map_entry(
    entry: tex_fonts::PdfFontMapEntry,
    entries: &mut BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
) {
    match entry.directive {
        tex_fonts::PdfFontMapDirective::Default | tex_fonts::PdfFontMapDirective::Add => {
            entries.entry(entry.tex_name.clone()).or_insert(entry);
        }
        tex_fonts::PdfFontMapDirective::Replace => {
            entries.insert(entry.tex_name.clone(), entry);
        }
        tex_fonts::PdfFontMapDirective::Remove => {
            entries.remove(&entry.tex_name);
        }
    }
}

const fn map_directive(operation: &PdfFontMapOperation) -> tex_fonts::PdfFontMapDirective {
    match operation {
        PdfFontMapOperation::BlockDefault => tex_fonts::PdfFontMapDirective::Default,
        PdfFontMapOperation::File(file) => file.directive,
        PdfFontMapOperation::Line(line) => line.directive,
    }
}

pub(crate) fn detached_pk_request(
    font: &tex_state::FontArtifactRecipe,
    base_dpi: i32,
) -> Result<tex_fonts::PdfPkFontRequest, String> {
    let design_size = i64::from(font.design_size.raw());
    if design_size <= 0 {
        return Err(format!("font {} has invalid PK design size", font.name));
    }
    let dpi = i64::from(base_dpi.clamp(72, 8_000))
        .checked_mul(i64::from(font.at_size.raw()))
        .and_then(|value| value.checked_add(design_size / 2))
        .map(|value| value / design_size)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("font {} PK resolution overflows", font.name))?;
    Ok(tex_fonts::PdfPkFontRequest::new(
        font.name.as_bytes().to_vec(),
        dpi,
        Vec::new(),
    ))
}

fn request(kind: FileKind, name: &str, extension: &str) -> Result<FileRequest, String> {
    let normalized = if extension.is_empty()
        || name
            .rsplit('/')
            .next()
            .is_some_and(|part| part.contains('.'))
    {
        name.to_owned()
    } else {
        format!("{name}.{extension}")
    };
    let key = FileRequestKey::new(kind, &normalized).map_err(|error| error.to_string())?;
    Ok(FileRequest::new(key, normalized))
}

fn utf8_name<'a>(resource: &str, name: &'a [u8]) -> Result<&'a str, String> {
    std::str::from_utf8(name).map_err(|_| format!("{resource} name is not valid UTF-8"))
}

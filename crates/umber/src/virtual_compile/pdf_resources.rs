use std::collections::{BTreeMap, BTreeSet};

use tex_fonts::{TfmFont, VfProgram};
use tex_state::Universe;
use umber_vfs::{FileContentId, FileProvisioner};

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
}

pub(super) struct Discovery {
    pub required: Vec<ResourceRequest>,
    pub probes: Vec<ResourceRequest>,
}

pub(super) fn discover(
    stores: &mut Universe,
    files: &FileProvisioner,
    cache: &mut PdfVirtualFontResources,
    pk_fonts: &BTreeMap<tex_fonts::PdfPkFontRequest, ResolvedPkFont>,
    unavailable_pk_fonts: &BTreeSet<tex_fonts::PdfPkFontRequest>,
) -> Result<Discovery, String> {
    let mut required = BTreeMap::<FileRequestKey, FileRequest>::new();
    let mut probes = BTreeMap::<FileRequestKey, FileRequest>::new();
    let mut fonts = stores
        .pdf_font_resources()
        .filter_map(|resource| {
            let font = stores.font(resource.font());
            stores
                .font_uses_tfm_metrics(resource.font())
                .then(|| font.name().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if fonts.is_empty() {
        return Ok(Discovery {
            required: Vec::new(),
            probes: Vec::new(),
        });
    }
    let mut real_fonts = BTreeSet::new();
    let mut visited = BTreeSet::new();

    while let Some(name) = fonts.pop_first() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let vf_request = request(FileKind::VirtualFont, &name, "vf")?;
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
        });
    }

    let explicitly_requests_default = stores.pdf_font_maps().any(|operation| {
        matches!(
            operation,
            tex_state::PdfFontMapOperation::File(file)
                if file.logical_name == b"pdftex.map"
        )
    });
    let mut implicit_default = false;
    for name in stores.pdf_font_map_file_requests() {
        let name = utf8_name("PDF font map", &name)?;
        if name == "pdftex.map" && !explicitly_requests_default {
            implicit_default = true;
            continue;
        }
        let map_request = request(FileKind::PdfFontMap, name, "map")?;
        if files.is_unavailable(map_request.key()) {
            return Err(format!("required PDF font map {name} is unavailable"));
        }
        if let Some(file) = files.get(map_request.key()) {
            if !stores.has_pdf_font_map_file(name.as_bytes()) {
                stores
                    .provide_pdf_font_map_file(name.as_bytes().to_vec(), file.bytes())
                    .map_err(|error| format!("PDF font map {name}: {error}"))?;
            }
        } else {
            required.insert(map_request.key().clone(), map_request);
        }
    }
    let covered_names = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .map(|entry| entry.tex_name)
        .chain(stores.authoritative_pdf_font_map_names())
        .filter_map(|name| String::from_utf8(name).ok())
        .collect::<BTreeSet<_>>();
    if implicit_default && !real_fonts.is_subset(&covered_names) {
        let name = "pdftex.map";
        let map_request = request(FileKind::PdfFontMap, name, "map")?;
        if files.is_unavailable(map_request.key()) {
            return Err(format!("required PDF font map {name} is unavailable"));
        }
        if let Some(file) = files.get(map_request.key()) {
            if !stores.has_pdf_font_map_file(name.as_bytes()) {
                stores
                    .provide_pdf_font_map_file(name.as_bytes().to_vec(), file.bytes())
                    .map_err(|error| format!("PDF font map {name}: {error}"))?;
            }
        } else {
            required.insert(map_request.key().clone(), map_request);
        }
    }

    for entry in stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .filter(|entry| real_fonts.contains(utf8_name("mapped TFM", &entry.tex_name).unwrap_or("")))
    {
        for encoding in entry.encoding_files {
            let name = utf8_name("PDF encoding", &encoding)?;
            if stores.pdf_encoding(name.as_bytes()).is_none() {
                acquire_parsed(
                    stores,
                    files,
                    &mut required,
                    FileKind::PdfEncoding,
                    name,
                    |stores, bytes| {
                        stores
                            .provide_pdf_encoding(name.as_bytes().to_vec(), bytes)
                            .map_err(|error| error.to_string())
                    },
                )?;
            }
        }
        if let Some(program) = entry.font_file {
            let name = utf8_name("PDF font program", &program)?;
            let is_truetype = crate::pdf_output::is_pdf_sfnt_program(name.as_bytes());
            let present = if is_truetype {
                stores.pdf_truetype_program(name.as_bytes()).is_some()
            } else {
                stores.pdf_type1_program(name.as_bytes()).is_some()
            };
            if !present {
                acquire_parsed(
                    stores,
                    files,
                    &mut required,
                    FileKind::PdfFontProgram,
                    name,
                    |stores, bytes| {
                        if is_truetype {
                            stores
                                .provide_pdf_truetype_program(name.as_bytes().to_vec(), bytes)
                                .map_err(|error| error.to_string())
                        } else {
                            stores
                                .provide_pdf_type1_program(name.as_bytes().to_vec(), bytes)
                                .map_err(|error| error.to_string())
                        }
                    },
                )?;
            }
        }
    }

    let mapped_names = stores
        .resolved_pdf_font_map_lines()
        .into_iter()
        .map(|entry| entry.tex_name)
        .collect::<BTreeSet<_>>();
    let virtual_names = cache
        .virtual_fonts
        .keys()
        .map(|name| name.as_bytes().to_vec())
        .collect::<BTreeSet<_>>();
    let pk_requests = stores
        .pdf_font_resources()
        .filter_map(|resource| {
            let font = stores.font(resource.font());
            (!mapped_names.contains(font.name().as_bytes())
                && !virtual_names.contains(font.name().as_bytes()))
            .then(|| {
                crate::pdf_output::pk_font_request(
                    stores,
                    resource.font(),
                    crate::pdf_output::DEFAULT_PDF_PK_RESOLUTION,
                )
            })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut pk_required = Vec::new();
    for request in pk_requests {
        if stores.pdf_pk_font(&request).is_some() {
            continue;
        }
        if unavailable_pk_fonts.contains(&request) {
            return Err(format!(
                "required PK font {} is unavailable",
                String::from_utf8_lossy(&request.logical_name())
            ));
        }
        if let Some(resolved) = pk_fonts.get(&request) {
            stores
                .provide_pdf_pk_font(request, &resolved.bytes)
                .map_err(|error| format!("PK font: {error}"))?;
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
    })
}

fn acquire_parsed(
    stores: &mut Universe,
    files: &FileProvisioner,
    required: &mut BTreeMap<FileRequestKey, FileRequest>,
    kind: FileKind,
    name: &str,
    parse: impl FnOnce(&mut Universe, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let request = request(kind, name, "")?;
    if files.is_unavailable(request.key()) {
        return Err(format!("required {} {name} is unavailable", kind));
    }
    if let Some(file) = files.get(request.key()) {
        parse(stores, file.bytes())?;
    } else {
        required.insert(request.key().clone(), request);
    }
    Ok(())
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

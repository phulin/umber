use bib_engine::{BibJob, BibOptionsBuilder, BibliographyMode, OutputFormat, OutputRequest};
use js_sys::{Array, Reflect, Uint8Array};
use umber::{
    BibliographyProjectOptions, EngineMode, FeatureSetting, FileContentId, FileKind, FileRequest,
    FileRequestKey, FontContainer, FontFeaturePolicy, FontObjectIdentity, FontProgramIdentity,
    FontRequestKey, LatexProjectLimits, LatexProjectOptions, LatexProjectOptionsV2, OpenTypeTag,
    ResolvedFile, ResolvedFont, ResourceDomain, ResourceRequest, ResourceResponse, SessionLimits,
    SessionOptions, SessionWebFont, SourcePatch, VariationCoordinate, VariationSelection,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::js_error;

pub(crate) fn parse_options(value: &JsValue) -> Result<SessionOptions, JsValue> {
    require_object(value, "session options")?;
    let mut options = SessionOptions {
        main_path: required_string(value, "mainPath")?,
        ..SessionOptions::default()
    };
    options.job_name = optional_string(value, "jobName")?;
    options.format = optional_bytes(value, "format")?;
    let hints = field(value, "formatPrefetchHints")?;
    if !absent(&hints) {
        if !Array::is_array(&hints) {
            return Err(js_error("formatPrefetchHints must be an array"));
        }
        options.format_prefetch_hints = Some(
            Array::from(&hints)
                .iter()
                .map(|hint| {
                    require_object(&hint, "format prefetch hint")?;
                    if required_string(&hint, "type")? != "file" {
                        return Err(js_error("format prefetch hints must be file requests"));
                    }
                    let key = parse_request_key(&hint)?;
                    let original_name = optional_string(&hint, "originalName")?
                        .unwrap_or_else(|| key.name().to_owned());
                    Ok(ResourceRequest::File(FileRequest::new(key, original_name)))
                })
                .collect::<Result<Vec<_>, JsValue>>()?
                .into_boxed_slice(),
        );
    }
    if let Some(engine) = optional_string(value, "engine")? {
        options.engine = match engine.as_str() {
            "tex82" => EngineMode::Tex82,
            "etex" => EngineMode::ETex,
            "pdftex" => EngineMode::PdfTex,
            "latex" => EngineMode::Latex,
            "pdflatex" => EngineMode::PdfLatex,
            _ => {
                return Err(js_error(
                    "engine must be 'tex82', 'etex', 'pdftex', 'latex', or 'pdflatex'",
                ));
            }
        };
    }
    options.html = !field(value, "html")?.is_undefined() && !field(value, "html")?.is_null();
    if let Some(clock) = optional_object(value, "clock")? {
        options.clock.year = integer::<i32>(&clock, "year")?;
        options.clock.month = integer::<i32>(&clock, "month")?;
        options.clock.day = integer::<i32>(&clock, "day")?;
        options.clock.time = integer::<i32>(&clock, "minutes")?;
    }
    if let Some(limits) = optional_object(value, "limits")? {
        options.limits = parse_limits(&limits)?;
    }
    Ok(options)
}

pub(crate) enum ProjectOptions {
    Legacy(LatexProjectOptions),
    V2(LatexProjectOptionsV2),
}

pub(crate) fn parse_project_options(value: &JsValue) -> Result<ProjectOptions, JsValue> {
    let tex = parse_options(value)?;
    let bibliography = optional_object(value, "bibliography")?
        .ok_or_else(|| js_error("project options require bibliography"))?;
    let mode = optional_string(&bibliography, "mode")?;
    let control_path = optional_string(&bibliography, "controlPath")?
        .map(|path| parse_virtual_path(&path))
        .transpose()?;
    let mut builder = BibOptionsBuilder::new();
    let outputs = if matches!(mode.as_deref(), Some("classic") | Some("auto")) {
        Vec::new()
    } else {
        parse_array(&bibliography, "outputs")?
    };
    for output in outputs {
        let path = parse_virtual_path(&required_string(&output, "path")?)?;
        let format = match required_string(&output, "format")?.as_str() {
            "bbl" => OutputFormat::Bbl,
            "bibtex" => OutputFormat::Bibtex,
            "biblatex-xml" => OutputFormat::BibLatexXml,
            "bbl-xml" => OutputFormat::BblXml,
            "dot" => OutputFormat::Dot,
            _ => return Err(js_error("bibliography output format is not recognized")),
        };
        builder
            .output(OutputRequest::new(path, format))
            .map_err(crate::boundary_error)?;
    }
    if let Some(path) = optional_string(&bibliography, "configurationPath")? {
        builder.configuration(parse_virtual_path(&path)?);
    }
    let schemas = field(&bibliography, "schemaPaths")?;
    if !absent(&schemas) {
        if !Array::is_array(&schemas) {
            return Err(js_error("bibliography schemaPaths must be an array"));
        }
        for path in Array::from(&schemas).iter() {
            let path = path
                .as_string()
                .ok_or_else(|| js_error("bibliography schema paths must be strings"))?;
            builder
                .schema(parse_virtual_path(&path)?)
                .map_err(crate::boundary_error)?;
        }
    }
    let mut limits = LatexProjectLimits::default();
    if let Some(value) = optional_object(value, "projectLimits")? {
        if has_value(&value, "attempts")? {
            limits.attempts = integer::<u32>(&value, "attempts")?;
        }
        if has_value(&value, "passes")? {
            limits.passes = integer::<u32>(&value, "passes")?;
        }
    }
    let biblatex = builder.freeze();
    match mode.as_deref() {
        None => Ok(ProjectOptions::Legacy(LatexProjectOptions {
            tex,
            bibliography: BibJob::new(
                control_path
                    .ok_or_else(|| js_error("project bibliography requires controlPath"))?,
                biblatex,
            ),
            bib_session: bib_engine::BibSessionOptions::default(),
            limits,
        })),
        Some("biblatex") => Ok(ProjectOptions::V2(LatexProjectOptionsV2 {
            tex,
            bibliography: BibliographyProjectOptions {
                mode: BibliographyMode::Biblatex {
                    control_path: control_path
                        .ok_or_else(|| js_error("biblatex bibliography requires controlPath"))?,
                },
                biblatex,
                bib_session: bib_engine::BibSessionOptions::default(),
                classic: bib_engine::ClassicBibOptions::default(),
                detector: bib_engine::BibliographyDetectorOptions::default(),
            },
            limits,
        })),
        Some("classic") => Ok(ProjectOptions::V2(LatexProjectOptionsV2 {
            tex,
            bibliography: BibliographyProjectOptions::classic(parse_virtual_path(
                &required_string(&bibliography, "auxPath")?,
            )?),
            limits,
        })),
        Some("auto") => Ok(ProjectOptions::V2(LatexProjectOptionsV2 {
            tex,
            bibliography: BibliographyProjectOptions::auto(parse_virtual_path(&required_string(
                &bibliography,
                "jobPath",
            )?)?),
            limits,
        })),
        Some(_) => Err(js_error(
            "bibliography mode must be 'biblatex', 'classic', or 'auto'",
        )),
    }
}

fn parse_virtual_path(value: &str) -> Result<bib_engine::VirtualPath, JsValue> {
    bib_engine::VirtualPath::user(value).map_err(crate::boundary_error)
}

pub(crate) fn parse_html_font(value: &JsValue) -> Result<SessionWebFont, JsValue> {
    require_object(value, "HTML font")?;
    let woff2 = required_bytes(value, "woff2")?;
    let digest = parse_digest(&required_string(value, "sha256")?)?;
    let encoding_value = field(value, "encoding")?;
    if !Array::is_array(&encoding_value) {
        return Err(js_error("HTML font encoding must be an array"));
    }
    let array = Array::from(&encoding_value);
    if array.length() != 256 {
        return Err(js_error("HTML font encoding must contain 256 entries"));
    }
    let mut encoding = Vec::with_capacity(256);
    for value in array.iter() {
        if value.is_null() || value.is_undefined() {
            encoding.push(None);
        } else {
            encoding.push(Some(value.as_string().ok_or_else(|| {
                js_error("HTML font encoding entries must be strings or null")
            })?));
        }
    }
    Ok(SessionWebFont {
        name: required_string(value, "name")?,
        tfm_content_hash_hex: required_string(value, "tfmContentHash")?,
        woff2,
        sha256: digest,
        encoding,
        provenance: required_string(value, "provenance")?,
        embeddable: required_bool(value, "embeddable")?,
    })
}

pub(crate) fn parse_source_patch(value: &JsValue) -> Result<SourcePatch, JsValue> {
    require_object(value, "source patch")?;
    let start = integer::<usize>(value, "start")?;
    let end = integer::<usize>(value, "end")?;
    if start > end {
        return Err(js_error("source patch start must not exceed end"));
    }
    Ok(SourcePatch {
        next_revision: umber::RevisionId::new(u64::from(integer::<u32>(value, "nextRevision")?)),
        base_revision: umber::RevisionId::new(u64::from(integer::<u32>(value, "baseRevision")?)),
        expected_hash: parse_content_hash(&required_string(value, "expectedHash")?)?,
        range: start..end,
        replacement: required_string(value, "replacement")?,
    })
}

fn parse_content_hash(value: &str) -> Result<tex_state::ContentHash, JsValue> {
    if value.len() != 64 {
        return Err(js_error(
            "expectedHash must contain 64 lowercase hex digits",
        ));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let high = digit(pair[0])
            .ok_or_else(|| js_error("expectedHash must contain 64 lowercase hex digits"))?;
        let low = digit(pair[1])
            .ok_or_else(|| js_error("expectedHash must contain 64 lowercase hex digits"))?;
        bytes[index] = (high << 4) | low;
    }
    Ok(tex_state::ContentHash::new(bytes))
}

pub(crate) fn parse_request_key(value: &JsValue) -> Result<FileRequestKey, JsValue> {
    require_object(value, "file request key")?;
    let kind_name = required_string(value, "kind")?;
    let kind = FileKind::from_wire_name(&kind_name)
        .ok_or_else(|| js_error("file request kind is not recognized"))?;
    let domain = optional_string(value, "domain")?
        .map(|domain| {
            ResourceDomain::from_wire_name(&domain)
                .ok_or_else(|| js_error("file request domain is not recognized"))
        })
        .transpose()?
        .unwrap_or_else(|| kind.domain());
    FileRequestKey::for_domain(domain, kind, &required_string(value, "name")?)
        .map_err(crate::boundary_error)
}

pub(crate) fn parse_resource_responses(value: &JsValue) -> Result<Vec<ResourceResponse>, JsValue> {
    if !Array::is_array(value) {
        return Err(js_error("resource responses must be an array"));
    }
    Array::from(value)
        .iter()
        .map(|response| {
            require_object(&response, "resource response")?;
            match required_string(&response, "type")?.as_str() {
                "file" => Ok(ResourceResponse::File(ResolvedFile {
                    request: parse_request_key(&response)?,
                    virtual_path: required_string(&response, "virtualPath")?,
                    bytes: required_bytes(&response, "bytes")?,
                    expected_digest: optional_string(&response, "expectedContentId")?
                        .map(|digest| parse_digest(&digest).map(FileContentId::from_identity_bytes))
                        .transpose()?,
                })),
                "file-unavailable" => Ok(ResourceResponse::FileUnavailable(parse_request_key(
                    &response,
                )?)),
                "font" => Ok(ResourceResponse::Font(parse_resolved_font(&response)?)),
                "font-unavailable" => Ok(ResourceResponse::FontUnavailable(
                    parse_font_request_key(&response)?,
                )),
                _ => Err(js_error(
                    "resource response type must be 'file', 'file-unavailable', 'font', or 'font-unavailable'",
                )),
            }
        })
        .collect()
}

fn parse_resolved_font(value: &JsValue) -> Result<ResolvedFont, JsValue> {
    let request = parse_font_request_key(value)?;
    let container = match required_string(value, "container")?.as_str() {
        "woff2" => FontContainer::Woff2,
        _ => return Err(js_error("WASM font container must be 'woff2'")),
    };
    Ok(ResolvedFont {
        request,
        container,
        bytes: required_bytes(value, "bytes")?,
        declared_object_sha256: optional_string(value, "objectSha256")?
            .map(|digest| parse_digest(&digest).map(FontObjectIdentity::from_bytes))
            .transpose()?,
        declared_program_identity: optional_string(value, "programIdentity")?
            .map(|digest| parse_digest(&digest).map(FontProgramIdentity::from_bytes))
            .transpose()?,
        provenance: optional_string(value, "provenance")?,
    })
}

fn parse_font_request_key(value: &JsValue) -> Result<FontRequestKey, JsValue> {
    let variation = parse_array(value, "variations")?
        .into_iter()
        .map(|coordinate| {
            Ok(VariationCoordinate {
                tag: parse_tag(&required_string(&coordinate, "tag")?)?,
                value: signed_integer::<i32>(&coordinate, "value")?,
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let features = parse_array(value, "features")?
        .into_iter()
        .map(|feature| {
            Ok(FeatureSetting {
                tag: parse_tag(&required_string(&feature, "tag")?)?,
                enabled: required_bool(&feature, "enabled")?,
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    FontRequestKey::new(
        required_string(value, "logicalName")?,
        integer::<u32>(value, "faceIndex")?,
        VariationSelection::new(variation).map_err(crate::boundary_error)?,
        FontFeaturePolicy::new(features).map_err(crate::boundary_error)?,
    )
    .map_err(crate::boundary_error)
}

fn parse_array(object: &JsValue, name: &str) -> Result<Vec<JsValue>, JsValue> {
    let value = field(object, name)?;
    if !Array::is_array(&value) {
        return Err(js_error(&format!("{name} must be an array")));
    }
    Ok(Array::from(&value).iter().collect())
}

fn parse_tag(value: &str) -> Result<OpenTypeTag, JsValue> {
    let bytes: [u8; 4] = value
        .as_bytes()
        .try_into()
        .map_err(|_| js_error("OpenType tags must contain exactly four ASCII bytes"))?;
    if !bytes.iter().all(u8::is_ascii) {
        return Err(js_error("OpenType tags must be ASCII"));
    }
    Ok(OpenTypeTag::new(bytes))
}

fn parse_limits(value: &JsValue) -> Result<SessionLimits, JsValue> {
    let mut limits = SessionLimits::default();
    if has_value(value, "attempts")? {
        limits.attempts = integer::<u32>(value, "attempts")?;
    }
    if has_value(value, "userFiles")? {
        limits.user_files = integer::<usize>(value, "userFiles")?;
    }
    if has_value(value, "resolvedFiles")? {
        limits.resolved_files = integer::<usize>(value, "resolvedFiles")?;
    }
    if has_value(value, "oneFileBytes")? {
        limits.one_file_bytes = integer::<usize>(value, "oneFileBytes")?;
    }
    if has_value(value, "cachedFileBytes")? {
        limits.cached_file_bytes = integer::<usize>(value, "cachedFileBytes")?;
    }
    if has_value(value, "userSourceBytes")? {
        limits.user_source_bytes = integer::<usize>(value, "userSourceBytes")?;
    }
    if has_value(value, "outputBytes")? {
        limits.output_bytes = integer::<usize>(value, "outputBytes")?;
    }
    Ok(limits)
}

fn required_string(object: &JsValue, name: &str) -> Result<String, JsValue> {
    field(object, name)?
        .as_string()
        .ok_or_else(|| js_error(&format!("{name} must be a string")))
}

fn optional_string(object: &JsValue, name: &str) -> Result<Option<String>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    value
        .as_string()
        .map(Some)
        .ok_or_else(|| js_error(&format!("{name} must be a string")))
}

fn optional_bytes(object: &JsValue, name: &str) -> Result<Option<Vec<u8>>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    if !value.is_instance_of::<Uint8Array>() {
        return Err(js_error(&format!("{name} must be a Uint8Array")));
    }
    Ok(Some(Uint8Array::new(&value).to_vec()))
}

fn required_bytes(object: &JsValue, name: &str) -> Result<Vec<u8>, JsValue> {
    let value = field(object, name)?;
    if !value.is_instance_of::<Uint8Array>() {
        return Err(js_error(&format!("{name} must be a Uint8Array")));
    }
    Ok(Uint8Array::new(&value).to_vec())
}

fn required_bool(object: &JsValue, name: &str) -> Result<bool, JsValue> {
    field(object, name)?
        .as_bool()
        .ok_or_else(|| js_error(&format!("{name} must be a boolean")))
}

fn parse_digest(value: &str) -> Result<[u8; 32], JsValue> {
    if value.len() != 64 {
        return Err(js_error("sha256 must contain 64 lowercase hex digits"));
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(js_error("sha256 must use lowercase hex")),
        };
        digest[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(digest)
}

fn optional_object(object: &JsValue, name: &str) -> Result<Option<JsValue>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    require_object(&value, name)?;
    Ok(Some(value))
}

fn integer<T>(object: &JsValue, name: &str) -> Result<T, JsValue>
where
    T: TryFrom<u64>,
{
    let number = field(object, name)?
        .as_f64()
        .filter(|number| number.is_finite() && number.fract() == 0.0 && *number >= 0.0)
        .ok_or_else(|| js_error(&format!("{name} must be a non-negative integer")))?;
    if number > u64::MAX as f64 {
        return Err(js_error(&format!("{name} is out of range")));
    }
    T::try_from(number as u64).map_err(|_| js_error(&format!("{name} is out of range")))
}

fn signed_integer<T>(object: &JsValue, name: &str) -> Result<T, JsValue>
where
    T: TryFrom<i64>,
{
    let number = field(object, name)?
        .as_f64()
        .filter(|number| number.is_finite() && number.fract() == 0.0)
        .ok_or_else(|| js_error(&format!("{name} must be an integer")))?;
    if number < i64::MIN as f64 || number > i64::MAX as f64 {
        return Err(js_error(&format!("{name} is out of range")));
    }
    T::try_from(number as i64).map_err(|_| js_error(&format!("{name} is out of range")))
}

fn has_value(object: &JsValue, name: &str) -> Result<bool, JsValue> {
    Ok(!absent(&field(object, name)?))
}

fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &JsValue::from_str(name))
}

fn require_object(value: &JsValue, name: &str) -> Result<(), JsValue> {
    if !value.is_object() || value.is_null() {
        return Err(js_error(&format!("{name} must be an object")));
    }
    Ok(())
}

fn absent(value: &JsValue) -> bool {
    value.is_undefined() || value.is_null()
}

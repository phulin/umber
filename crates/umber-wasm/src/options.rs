use bib_engine::{BibOptionsBuilder, BibliographyMode, OutputFormat, OutputRequest};
use js_sys::{Array, Date, Reflect, Uint8Array};
use umber::{
    BibliographyProjectOptions, EngineMode, FeatureSetting, FileContentId, FileKind, FileRequest,
    FileRequestKey, FixedPointLimits, FontContainer, FontFeaturePolicy, FontLanguage,
    FontObjectIdentity, FontProgramIdentity, FontRequestKey, LatexProjectLimits,
    LatexProjectOptions, LegacyFontMapping, OpenTypeTag, OutputCapability, OutputCapabilitySet,
    PdfPkFontRequest, ResolvedFile, ResolvedFont, ResolvedPkFont, ResourceDomain, ResourceRequest,
    ResourceResponse, SessionLimits, SessionOptions, SourcePatch, VariationCoordinate,
    VariationSelection, WritingDirection,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::js_error;

pub(crate) fn parse_options(value: &JsValue) -> Result<SessionOptions, JsValue> {
    require_object(value, "session options")?;
    let mut options = SessionOptions {
        main_path: required_string(value, "mainPath")?,
        clock: browser_job_clock(),
        ..SessionOptions::default()
    };
    options.job_name = optional_string(value, "jobName")?;
    options.format = optional_bytes(value, "format")?;
    let hints = field(value, "formatPrefetchHints")?;
    if !absent(&hints) {
        if !Array::is_array(&hints) {
            return Err(js_error("formatPrefetchHints must be an array"));
        }
        options.initial_prefetch_hints = Some(
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
    let requested_outputs = field(value, "outputs")?;
    if !absent(&requested_outputs) {
        if !Array::is_array(&requested_outputs) {
            return Err(js_error("outputs must be a nonempty array"));
        }
        let mut outputs = None;
        for output in Array::from(&requested_outputs).iter() {
            let capability = match output.as_string().as_deref() {
                Some("dvi") => OutputCapability::Dvi,
                Some("pdf") => OutputCapability::Pdf,
                Some("html") => OutputCapability::Html,
                _ => return Err(js_error("outputs entries must be 'dvi', 'pdf', or 'html'")),
            };
            outputs = Some(outputs.map_or_else(
                || OutputCapabilitySet::new(capability),
                |set: OutputCapabilitySet| set.with(capability),
            ));
        }
        options.outputs = outputs.ok_or_else(|| js_error("outputs must be a nonempty array"))?;
    } else if has_value(value, "dvi")? || has_value(value, "html")? {
        return Err(js_error(
            "session options dvi/html were removed; use the nonempty outputs array",
        ));
    } else {
        return Err(js_error(
            "session options require a nonempty outputs array; outputs are never inferred from engine",
        ));
    }
    options.font_layout_policy = match optional_string(value, "fontLayoutPolicy")?.as_deref() {
        None => umber::FontLayoutPolicy::OpenTypePreferred,
        Some("classic-tfm-exact") => umber::FontLayoutPolicy::ClassicTfmExact,
        Some("opentype-preferred") => umber::FontLayoutPolicy::OpenTypePreferred,
        Some(_) => {
            return Err(js_error(
                "fontLayoutPolicy must be 'opentype-preferred' or 'classic-tfm-exact'",
            ));
        }
    };
    options.font_mapping_fallback = match optional_string(value, "fontMappingFallback")?.as_deref()
    {
        None | Some("classic-tfm-exact") => umber::FontMappingFallbackPolicy::ClassicTfmExact,
        Some("error") => umber::FontMappingFallbackPolicy::Error,
        Some(_) => {
            return Err(js_error(
                "fontMappingFallback must be 'error' or 'classic-tfm-exact'",
            ));
        }
    };
    if let Some(clock) = optional_object(value, "clock")? {
        options.clock.year = integer::<i32>(&clock, "year")?;
        options.clock.month = integer::<i32>(&clock, "month")?;
        options.clock.day = integer::<i32>(&clock, "day")?;
        options.clock.time = integer::<i32>(&clock, "minutes")?;
        options.clock.second = 0;
    }
    if let Some(limits) = optional_object(value, "limits")? {
        options.limits = parse_limits(&limits)?;
    }
    Ok(options)
}

fn browser_job_clock() -> tex_state::JobClock {
    let now = Date::new_0();
    tex_state::JobClock {
        time: (now.get_hours() * 60 + now.get_minutes()) as i32,
        second: now.get_seconds() as i32,
        day: now.get_date() as i32,
        month: (now.get_month() + 1) as i32,
        year: now.get_full_year() as i32,
    }
}

pub(crate) fn parse_project_options(value: &JsValue) -> Result<LatexProjectOptions, JsValue> {
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
        None => Ok(LatexProjectOptions {
            tex,
            bibliography: BibliographyProjectOptions::biblatex(
                control_path
                    .ok_or_else(|| js_error("project bibliography requires controlPath"))?,
                biblatex,
            ),
            limits,
        }),
        Some("biblatex") => Ok(LatexProjectOptions {
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
        }),
        Some("classic") => Ok(LatexProjectOptions {
            tex,
            bibliography: BibliographyProjectOptions::classic(parse_virtual_path(
                &required_string(&bibliography, "auxPath")?,
            )?),
            limits,
        }),
        Some("auto") => Ok(LatexProjectOptions {
            tex,
            bibliography: BibliographyProjectOptions::auto(parse_virtual_path(&required_string(
                &bibliography,
                "jobPath",
            )?)?),
            limits,
        }),
        Some(_) => Err(js_error(
            "bibliography mode must be 'biblatex', 'classic', or 'auto'",
        )),
    }
}

pub(crate) fn parse_editor_options(
    value: &JsValue,
) -> Result<umber::EditorSessionOptions, JsValue> {
    let tex = parse_options(value)?;
    let mut stabilization = FixedPointLimits::default();
    if let Some(limits) = optional_object(value, "stabilizationLimits")? {
        if has_value(&limits, "attempts")? {
            stabilization.attempts = integer::<u32>(&limits, "attempts")?;
        }
        if has_value(&limits, "passes")? {
            stabilization.passes = integer::<u32>(&limits, "passes")?;
        }
    }
    Ok(umber::EditorSessionOptions { tex, stabilization })
}

fn parse_virtual_path(value: &str) -> Result<bib_engine::VirtualPath, JsValue> {
    bib_engine::VirtualPath::user(value).map_err(crate::boundary_error)
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
                "pk-font" => Ok(ResourceResponse::PkFont(ResolvedPkFont {
                    request: parse_pk_font_request(&response)?,
                    virtual_path: required_string(&response, "virtualPath")?,
                    bytes: required_bytes(&response, "bytes")?,
                    expected_sha256: optional_string(&response, "expectedSha256")?
                        .map(|digest| parse_digest(&digest))
                        .transpose()?,
                })),
                "pk-font-unavailable" => Ok(ResourceResponse::PkFontUnavailable(
                    parse_pk_font_request(&response)?,
                )),
                _ => Err(js_error("resource response type is not recognized")),
            }
        })
        .collect()
}

fn parse_pk_font_request(value: &JsValue) -> Result<PdfPkFontRequest, JsValue> {
    Ok(PdfPkFontRequest::new(
        required_bytes(value, "texName")?,
        integer::<u32>(value, "dpi")?,
        required_bytes(value, "mode")?,
    ))
}

fn parse_resolved_font(value: &JsValue) -> Result<ResolvedFont, JsValue> {
    let request = parse_font_request_key(value)?;
    let container = match required_string(value, "container")?.as_str() {
        "woff2" => FontContainer::Woff2,
        _ => return Err(js_error("WASM font container must be 'woff2'")),
    };
    let mapping_value = field(value, "legacyMapping")?;
    let legacy_mapping = if absent(&mapping_value) {
        None
    } else {
        require_object(&mapping_value, "legacy font mapping")?;
        let encoding_value = field(&mapping_value, "encoding")?;
        if !Array::is_array(&encoding_value) || Array::from(&encoding_value).length() != 256 {
            return Err(js_error(
                "legacy font mapping encoding must contain 256 entries",
            ));
        }
        let encoding = Array::from(&encoding_value)
            .iter()
            .map(|entry| {
                if entry.is_null() || entry.is_undefined() {
                    Ok(None)
                } else {
                    entry.as_string().map(Some).ok_or_else(|| {
                        js_error("legacy font mapping entries must be strings or null")
                    })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Some(LegacyFontMapping {
            tfm_sha256: parse_digest(&required_string(&mapping_value, "tfmSha256")?)?,
            encoding,
            embeddable: required_bool(&mapping_value, "embeddable")?,
        })
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
        legacy_mapping,
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
                value: if has_value(&feature, "value")? {
                    integer::<u32>(&feature, "value")?
                } else {
                    u32::from(required_bool(&feature, "enabled")?)
                },
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let variation = if has_value(value, "variationInstance")? {
        let instance = field(value, "variationInstance")?;
        if instance.as_string().as_deref() == Some("default") {
            if !variation.is_empty() {
                return Err(js_error(
                    "default variation instance cannot include coordinates",
                ));
            }
            VariationSelection::default()
        } else if instance.as_string().as_deref() == Some("coordinates") {
            VariationSelection::new(variation).map_err(crate::boundary_error)?
        } else {
            require_object(&instance, "variationInstance")?;
            if !variation.is_empty() {
                return Err(js_error(
                    "named variation instance cannot include coordinates",
                ));
            }
            VariationSelection::named(integer::<u16>(&instance, "namedNameId")?)
        }
    } else {
        VariationSelection::new(variation).map_err(crate::boundary_error)?
    };
    let direction = match optional_string(value, "direction")?.as_deref() {
        None | Some("ltr") => WritingDirection::LeftToRight,
        Some("rtl") => WritingDirection::RightToLeft,
        Some(_) => return Err(js_error("direction must be ltr or rtl")),
    };
    let script = optional_string(value, "script")?
        .map(|script| parse_tag(&script))
        .transpose()?;
    let language = optional_string(value, "language")?
        .map(FontLanguage::new)
        .transpose()
        .map_err(crate::boundary_error)?;
    FontRequestKey::new(
        required_string(value, "logicalName")?,
        integer::<u32>(value, "faceIndex")?,
        variation,
        FontFeaturePolicy::new(features).map_err(crate::boundary_error)?,
    )
    .and_then(|key| key.with_shaping_context(direction, script, language))
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

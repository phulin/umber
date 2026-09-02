use bib_engine::{BibOptionsBuilder, BibliographyMode, OutputFormat, OutputRequest};
use js_sys::{Date, Reflect};
use serde::de::DeserializeOwned;
use umber::{
    BibliographyProjectOptions, EngineMode, FeatureSetting, FileContentId, FileKind, FileRequest,
    FileRequestKey, FixedPointLimits, FontContainer, FontFeaturePolicy, FontLanguage,
    FontObjectIdentity, FontProgramIdentity, FontRequestKey, LatexProjectLimits,
    LatexProjectOptions, LegacyFontMapping, OpenTypeTag, OutputCapability, OutputCapabilitySet,
    PdfPkFontRequest, ResolvedFile, ResolvedFont, ResolvedPkFont, ResourceDomain, ResourceRequest,
    ResourceResponse, SessionLimits, SessionOptions, SourcePatch, VariationCoordinate,
    VariationSelection, WritingDirection,
};
use wasm_bindgen::JsValue;

use crate::{js_error, wire};

pub(crate) fn parse_options(value: &JsValue) -> Result<SessionOptions, JsValue> {
    reject_removed_output_options(value)?;
    session_options(from_js(value.clone())?)
}

pub(crate) fn parse_project_options(value: &JsValue) -> Result<LatexProjectOptions, JsValue> {
    reject_removed_output_options(value)?;
    let dto: wire::ProjectSessionOptionsDto = from_js(value.clone())?;
    let tex = session_options(dto.session)?;
    let bibliography = dto.bibliography;
    let control_path = bibliography
        .control_path
        .as_deref()
        .map(parse_virtual_path)
        .transpose()?;
    let mut builder = BibOptionsBuilder::new();
    let outputs = if matches!(
        bibliography.mode,
        Some(wire::BibliographyModeDto::Classic | wire::BibliographyModeDto::Auto)
    ) {
        Vec::new()
    } else {
        bibliography.outputs
    };
    for output in outputs {
        let format = match output.format {
            wire::BibliographyOutputFormatDto::Bbl => OutputFormat::Bbl,
            wire::BibliographyOutputFormatDto::Bibtex => OutputFormat::Bibtex,
            wire::BibliographyOutputFormatDto::BiblatexXml => OutputFormat::BibLatexXml,
            wire::BibliographyOutputFormatDto::BblXml => OutputFormat::BblXml,
            wire::BibliographyOutputFormatDto::Dot => OutputFormat::Dot,
        };
        builder
            .output(OutputRequest::new(
                parse_virtual_path(&output.path)?,
                format,
            ))
            .map_err(crate::boundary_error)?;
    }
    if let Some(path) = bibliography.configuration_path {
        builder.configuration(parse_virtual_path(&path)?);
    }
    for path in bibliography.schema_paths.unwrap_or_default() {
        builder
            .schema(parse_virtual_path(&path)?)
            .map_err(crate::boundary_error)?;
    }
    let limits = fixed_point_limits(dto.project_limits, LatexProjectLimits::default());
    let biblatex = builder.freeze();
    let bibliography = match bibliography.mode {
        None => BibliographyProjectOptions::biblatex(
            control_path.ok_or_else(|| js_error("project bibliography requires controlPath"))?,
            biblatex,
        ),
        Some(wire::BibliographyModeDto::Biblatex) => BibliographyProjectOptions {
            mode: BibliographyMode::Biblatex {
                control_path: control_path
                    .ok_or_else(|| js_error("biblatex bibliography requires controlPath"))?,
            },
            biblatex,
            bib_session: bib_engine::BibSessionOptions::default(),
            classic: bib_engine::ClassicBibOptions::default(),
            detector: bib_engine::BibliographyDetectorOptions::default(),
        },
        Some(wire::BibliographyModeDto::Classic) => {
            BibliographyProjectOptions::classic(parse_virtual_path(
                bibliography
                    .aux_path
                    .as_deref()
                    .ok_or_else(|| js_error("classic bibliography requires auxPath"))?,
            )?)
        }
        Some(wire::BibliographyModeDto::Auto) => {
            BibliographyProjectOptions::auto(parse_virtual_path(
                bibliography
                    .job_path
                    .as_deref()
                    .ok_or_else(|| js_error("auto bibliography requires jobPath"))?,
            )?)
        }
    };
    Ok(LatexProjectOptions {
        tex,
        bibliography,
        limits,
    })
}

pub(crate) fn parse_editor_options(
    value: &JsValue,
) -> Result<umber::EditorSessionOptions, JsValue> {
    reject_removed_output_options(value)?;
    let dto: wire::EditorSessionOptionsDto = from_js(value.clone())?;
    Ok(umber::EditorSessionOptions {
        tex: session_options(dto.session)?,
        stabilization: fixed_point_limits(dto.stabilization_limits, FixedPointLimits::default()),
    })
}

fn session_options(dto: wire::SessionOptionsDto) -> Result<SessionOptions, JsValue> {
    let mut options = SessionOptions {
        main_path: dto.main_path,
        job_name: dto.job_name,
        format: dto.format,
        clock: browser_job_clock(),
        ..SessionOptions::default()
    };
    if let Some(hints) = dto.format_prefetch_hints {
        options.initial_prefetch_hints = Some(
            hints
                .into_iter()
                .map(|hint| match hint {
                    wire::ResourceRequestDto::File { key, original_name } => {
                        Ok(ResourceRequest::File(FileRequest::new(
                            file_request_key(key)?,
                            original_name,
                        )))
                    }
                    _ => Err(js_error("format prefetch hints must be file requests")),
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_boxed_slice(),
        );
    }
    if let Some(engine) = dto.engine {
        options.engine = match engine {
            wire::EngineModeDto::Tex82 => EngineMode::Tex82,
            wire::EngineModeDto::Etex => EngineMode::ETex,
            wire::EngineModeDto::Pdftex => EngineMode::PdfTex,
            wire::EngineModeDto::Latex => EngineMode::Latex,
            wire::EngineModeDto::Pdflatex => EngineMode::PdfLatex,
        };
    }
    let mut outputs = dto.outputs.into_iter().map(|output| match output {
        wire::OutputCapabilityDto::Dvi => OutputCapability::Dvi,
        wire::OutputCapabilityDto::Pdf => OutputCapability::Pdf,
        wire::OutputCapabilityDto::Html => OutputCapability::Html,
    });
    let first = outputs.next().ok_or_else(|| {
        js_error("session options require a nonempty outputs array; outputs are never inferred from engine")
    })?;
    options.outputs = outputs.fold(OutputCapabilitySet::new(first), OutputCapabilitySet::with);
    if let Some(clock) = dto.clock {
        options.clock = tex_state::JobClock {
            year: clock.year,
            month: i32::from(clock.month),
            day: i32::from(clock.day),
            time: i32::from(clock.minutes),
            second: 0,
        };
    }
    if let Some(limits) = dto.limits {
        apply_limits(&mut options.limits, limits)?;
    }
    options.font_layout_policy = match dto.font_layout_policy {
        None | Some(wire::FontLayoutPolicyDto::OpentypePreferred) => {
            umber::FontLayoutPolicy::OpenTypePreferred
        }
        Some(wire::FontLayoutPolicyDto::ClassicTfmExact) => {
            umber::FontLayoutPolicy::ClassicTfmExact
        }
    };
    options.font_mapping_fallback = match dto.font_mapping_fallback {
        None | Some(wire::FontMappingFallbackDto::ClassicTfmExact) => {
            umber::FontMappingFallbackPolicy::ClassicTfmExact
        }
        Some(wire::FontMappingFallbackDto::Error) => umber::FontMappingFallbackPolicy::Error,
    };
    Ok(options)
}

fn reject_removed_output_options(value: &JsValue) -> Result<(), JsValue> {
    for name in ["dvi", "html"] {
        let field = Reflect::get(value, &JsValue::from_str(name))?;
        if !field.is_undefined() && !field.is_null() {
            return Err(js_error(
                "session options dvi/html were removed; use the nonempty outputs array",
            ));
        }
    }
    Ok(())
}

fn fixed_point_limits(
    overrides: Option<wire::FixedPointLimitOverridesDto>,
    mut limits: FixedPointLimits,
) -> FixedPointLimits {
    if let Some(overrides) = overrides {
        if let Some(attempts) = overrides.attempts {
            limits.attempts = attempts;
        }
        if let Some(passes) = overrides.passes {
            limits.passes = passes;
        }
    }
    limits
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

fn parse_virtual_path(value: &str) -> Result<bib_engine::VirtualPath, JsValue> {
    bib_engine::VirtualPath::user(value).map_err(crate::boundary_error)
}

pub(crate) fn parse_source_patch(value: &JsValue) -> Result<SourcePatch, JsValue> {
    let dto: wire::SourcePatchDto = from_js(value.clone())?;
    if dto.start > dto.end {
        return Err(js_error("source patch start must not exceed end"));
    }
    Ok(SourcePatch {
        next_revision: umber::RevisionId::new(u64::from(dto.next_revision)),
        base_revision: umber::RevisionId::new(u64::from(dto.base_revision)),
        expected_hash: parse_content_hash(&dto.expected_hash)?,
        range: dto.start as usize..dto.end as usize,
        replacement: dto.replacement,
    })
}

fn parse_content_hash(value: &str) -> Result<tex_state::ContentHash, JsValue> {
    parse_digest(value).map(tex_state::ContentHash::new)
}

pub(crate) fn parse_resource_responses(value: &JsValue) -> Result<Vec<ResourceResponse>, JsValue> {
    let responses: Vec<wire::ResourceResponseDto> = from_js(value.clone())?;
    responses.into_iter().map(resource_response).collect()
}

fn resource_response(response: wire::ResourceResponseDto) -> Result<ResourceResponse, JsValue> {
    match response {
        wire::ResourceResponseDto::File {
            key,
            virtual_path,
            bytes,
            expected_content_id,
        } => Ok(ResourceResponse::File(ResolvedFile {
            request: file_request_key(key)?,
            virtual_path,
            bytes: bytes.into(),
            expected_digest: expected_content_id
                .map(|digest| parse_digest(&digest).map(FileContentId::from_identity_bytes))
                .transpose()?,
        })),
        wire::ResourceResponseDto::FileUnavailable { key } => {
            Ok(ResourceResponse::FileUnavailable(file_request_key(key)?))
        }
        wire::ResourceResponseDto::Font {
            key,
            container,
            bytes,
            object_ahash64,
            program_identity,
            provenance,
            legacy_mapping,
        } => Ok(ResourceResponse::Font(ResolvedFont {
            request: font_request_key(key)?,
            container: match container {
                wire::FontContainerDto::Woff2 => FontContainer::Woff2,
            },
            bytes,
            declared_object_ahash64: object_ahash64
                .map(|digest| parse_ahash64(&digest).map(FontObjectIdentity::from_bytes))
                .transpose()?,
            declared_program_identity: program_identity
                .map(|digest| parse_ahash64(&digest).map(FontProgramIdentity::from_bytes))
                .transpose()?,
            provenance,
            legacy_mapping: legacy_mapping
                .map(|mapping| {
                    if mapping.encoding.len() != 256 {
                        return Err(js_error(
                            "legacy font mapping encoding must contain 256 entries",
                        ));
                    }
                    Ok(LegacyFontMapping {
                        tfm_ahash64: parse_ahash64(&mapping.tfm_ahash64)?,
                        encoding: mapping.encoding,
                        embeddable: mapping.embeddable,
                    })
                })
                .transpose()?,
        })),
        wire::ResourceResponseDto::FontUnavailable { key } => {
            Ok(ResourceResponse::FontUnavailable(font_request_key(key)?))
        }
        wire::ResourceResponseDto::PkFont {
            key,
            virtual_path,
            bytes,
            expected_ahash64,
        } => Ok(ResourceResponse::PkFont(ResolvedPkFont {
            request: pk_font_request(key),
            virtual_path,
            bytes,
            expected_ahash64: expected_ahash64
                .map(|digest| parse_ahash64(&digest))
                .transpose()?,
        })),
        wire::ResourceResponseDto::PkFontUnavailable { key } => {
            Ok(ResourceResponse::PkFontUnavailable(pk_font_request(key)))
        }
    }
}

fn file_request_key(key: wire::FileRequestKeyDto) -> Result<FileRequestKey, JsValue> {
    let domain = match key.domain {
        wire::ResourceDomainDto::Tex => ResourceDomain::Tex,
        wire::ResourceDomainDto::Bibliography => ResourceDomain::Bibliography,
        wire::ResourceDomainDto::Generic => ResourceDomain::Generic,
    };
    let kind = match key.kind {
        wire::FileKindDto::Tex => FileKind::TexInput,
        wire::FileKindDto::Tfm => FileKind::Tfm,
        wire::FileKindDto::Format => FileKind::FormatImage,
        wire::FileKindDto::BibControl => FileKind::BibControl,
        wire::FileKindDto::BibData => FileKind::BibData,
        wire::FileKindDto::BibConfiguration => FileKind::BibConfiguration,
        wire::FileKindDto::XmlSchema => FileKind::XmlSchema,
        wire::FileKindDto::Asset => FileKind::GenericAsset,
        wire::FileKindDto::Image => FileKind::Image,
        wire::FileKindDto::BibAux => FileKind::BibAux,
        wire::FileKindDto::ClassicBibData => FileKind::ClassicBibData,
        wire::FileKindDto::BibStyle => FileKind::BibStyle,
        wire::FileKindDto::Vf => FileKind::VirtualFont,
        wire::FileKindDto::FontMap => FileKind::PdfFontMap,
        wire::FileKindDto::FontEncoding => FileKind::PdfEncoding,
        wire::FileKindDto::FontProgram => FileKind::PdfFontProgram,
    };
    FileRequestKey::for_domain(domain, kind, &key.name).map_err(crate::boundary_error)
}

fn font_request_key(key: wire::FontRequestKeyDto) -> Result<FontRequestKey, JsValue> {
    let coordinates = key
        .variations
        .into_iter()
        .map(|coordinate| {
            Ok(VariationCoordinate {
                tag: parse_tag(&coordinate.tag)?,
                value: exact_signed_integer(coordinate.value, "variation value")?,
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let variation = match key.variation_instance {
        wire::VariationInstanceDto::Name(wire::VariationInstanceNameDto::Default) => {
            if !coordinates.is_empty() {
                return Err(js_error(
                    "default variation instance cannot include coordinates",
                ));
            }
            VariationSelection::default()
        }
        wire::VariationInstanceDto::Name(wire::VariationInstanceNameDto::Coordinates) => {
            VariationSelection::new(coordinates).map_err(crate::boundary_error)?
        }
        wire::VariationInstanceDto::Named { named_name_id } => {
            if !coordinates.is_empty() {
                return Err(js_error(
                    "named variation instance cannot include coordinates",
                ));
            }
            VariationSelection::named(named_name_id)
        }
    };
    let features = key
        .features
        .into_iter()
        .map(|feature| {
            Ok(FeatureSetting {
                tag: parse_tag(&feature.tag)?,
                value: exact_unsigned_integer(feature.value, "feature value")?,
            })
        })
        .collect::<Result<Vec<_>, JsValue>>()?;
    let direction = match key.direction {
        wire::WritingDirectionDto::Ltr => WritingDirection::LeftToRight,
        wire::WritingDirectionDto::Rtl => WritingDirection::RightToLeft,
    };
    let script = key.script.as_deref().map(parse_tag).transpose()?;
    let language = key
        .language
        .map(FontLanguage::new)
        .transpose()
        .map_err(crate::boundary_error)?;
    FontRequestKey::new(
        key.logical_name,
        key.face_index,
        variation,
        FontFeaturePolicy::new(features).map_err(crate::boundary_error)?,
    )
    .and_then(|key| key.with_shaping_context(direction, script, language))
    .map_err(crate::boundary_error)
}

fn pk_font_request(key: wire::PkFontRequestKeyDto) -> PdfPkFontRequest {
    PdfPkFontRequest::new(key.tex_name, key.dpi, key.mode)
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

fn apply_limits(
    limits: &mut SessionLimits,
    overrides: wire::SessionLimitOverridesDto,
) -> Result<(), JsValue> {
    macro_rules! apply {
        ($field:ident, $target:ty) => {
            if let Some(value) = overrides.$field {
                limits.$field = <$target>::try_from(value.get())
                    .map_err(|_| js_error(concat!(stringify!($field), " is out of range")))?;
            }
        };
    }
    apply!(attempts, u32);
    apply!(user_files, usize);
    apply!(resolved_files, usize);
    apply!(one_file_bytes, usize);
    apply!(cached_file_bytes, usize);
    apply!(user_source_bytes, usize);
    apply!(output_bytes, usize);
    apply!(engine_fuel, u64);
    apply!(engine_steps, u64);
    apply!(input_frames, u64);
    apply!(journal_bytes, u64);
    apply!(effects, u64);
    Ok(())
}

fn exact_signed_integer(value: f64, name: &str) -> Result<i32, JsValue> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f64::from(i32::MIN)
        || value > f64::from(i32::MAX)
    {
        return Err(js_error(&format!("{name} must be an integer")));
    }
    Ok(value as i32)
}

fn exact_unsigned_integer(value: f64, name: &str) -> Result<u32, JsValue> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(js_error(&format!("{name} must be a non-negative integer")));
    }
    Ok(value as u32)
}

fn parse_digest(value: &str) -> Result<[u8; 32], JsValue> {
    if value.len() != 64 {
        return Err(js_error(
            "content identity must contain 64 lowercase hex digits",
        ));
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(js_error("content identity must use lowercase hex")),
        };
        digest[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(digest)
}

fn parse_ahash64(value: &str) -> Result<[u8; 8], JsValue> {
    if value.len() != 16 {
        return Err(js_error(
            "aHash64 digest must contain 16 hexadecimal characters",
        ));
    }
    let mut digest = [0_u8; 8];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(js_error("aHash64 digest must use lowercase hex")),
        };
        digest[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(digest)
}

fn from_js<T: DeserializeOwned>(value: JsValue) -> Result<T, JsValue> {
    serde_wasm_bindgen::from_value(value).map_err(|error| js_error(&error.to_string()))
}

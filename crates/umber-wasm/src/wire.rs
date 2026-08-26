//! Versioned, engine-independent values crossing the WebAssembly boundary.
//!
//! These types describe JavaScript data, not engine state.  Conversion to and
//! from engine-owned types belongs in the binding adapters.  Serde's default
//! unknown-field policy is intentional: schema 1 readers accept additive
//! fields while required fields and discriminants remain strict.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The compatibility version of every DTO in this module.
pub const SCHEMA_VERSION: u32 = 1;

/// Largest integer represented exactly by a JavaScript `number`.
pub const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SafeInteger(u64);

impl SafeInteger {
    pub fn new(value: u64) -> Result<Self, UnsafeInteger> {
        if value <= MAX_SAFE_INTEGER {
            Ok(Self(value))
        } else {
            Err(UnsafeInteger(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for SafeInteger {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SafeIntegerVisitor;

        impl serde::de::Visitor<'_> for SafeIntegerVisitor {
            type Value = SafeInteger;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JavaScript-safe unsigned integer")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                SafeInteger::new(value).map_err(E::custom)
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                let value = u64::try_from(value)
                    .map_err(|_| E::invalid_value(serde::de::Unexpected::Signed(value), &self))?;
                self.visit_u64(value)
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if !value.is_finite() || value.fract() != 0.0 || value < 0.0 {
                    return Err(E::invalid_value(serde::de::Unexpected::Float(value), &self));
                }
                if value > MAX_SAFE_INTEGER as f64 {
                    return Err(E::custom(format_args!(
                        "{value:.0} exceeds JavaScript's safe integer range"
                    )));
                }
                Ok(SafeInteger(value as u64))
            }
        }

        deserializer.deserialize_any(SafeIntegerVisitor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsafeInteger(u64);

impl std::fmt::Display for UnsafeInteger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} exceeds JavaScript's safe integer range",
            self.0
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "ResourceDomain")]
pub enum ResourceDomainDto {
    Tex,
    Bibliography,
    Generic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "FileKind")]
pub enum FileKindDto {
    Tex,
    Tfm,
    Format,
    BibControl,
    BibData,
    BibConfiguration,
    XmlSchema,
    Asset,
    Image,
    BibAux,
    ClassicBibData,
    BibStyle,
    Vf,
    FontMap,
    FontEncoding,
    FontProgram,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FileRequestKey")]
pub struct FileRequestKeyDto {
    pub domain: ResourceDomainDto,
    pub kind: FileKindDto,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FontCoordinate")]
pub struct FontCoordinateDto {
    pub tag: String,
    pub value: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged, rename_all_fields = "camelCase")]
#[ts(rename = "VariationInstance")]
pub enum VariationInstanceDto {
    Name(VariationInstanceNameDto),
    Named { named_name_id: u16 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "VariationInstanceName")]
pub enum VariationInstanceNameDto {
    Default,
    Coordinates,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FontRequestKey", optional_fields)]
pub struct FontRequestKeyDto {
    pub logical_name: String,
    pub face_index: u32,
    pub variation_instance: VariationInstanceDto,
    pub variations: Vec<FontCoordinateDto>,
    pub features: Vec<FontCoordinateDto>,
    pub direction: WritingDirectionDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "WritingDirection")]
pub enum WritingDirectionDto {
    Ltr,
    Rtl,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "FontContainer")]
pub enum FontContainerDto {
    Woff2,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PkFontRequestKey")]
pub struct PkFontRequestKeyDto {
    #[serde(with = "serde_bytes")]
    #[ts(type = "Uint8Array")]
    pub tex_name: Vec<u8>,
    pub dpi: u32,
    #[serde(with = "serde_bytes")]
    #[ts(type = "Uint8Array")]
    pub mode: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "ResourceRequest")]
pub enum ResourceRequestDto {
    File {
        #[serde(flatten)]
        key: FileRequestKeyDto,
        original_name: String,
    },
    Font {
        #[serde(flatten)]
        key: FontRequestKeyDto,
        accepted_containers: Vec<FontContainerDto>,
    },
    PkFont {
        #[serde(flatten)]
        key: PkFontRequestKeyDto,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "LegacyFontMapping")]
pub struct LegacyFontMappingDto {
    pub tfm_ahash64: String,
    pub encoding: Vec<Option<String>>,
    pub embeddable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "ResourceResponse")]
pub enum ResourceResponseDto {
    File {
        #[serde(flatten)]
        key: FileRequestKeyDto,
        virtual_path: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "Uint8Array")]
        bytes: Vec<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        expected_content_id: Option<String>,
    },
    FileUnavailable {
        #[serde(flatten)]
        key: FileRequestKeyDto,
    },
    Font {
        #[serde(flatten)]
        key: FontRequestKeyDto,
        container: FontContainerDto,
        #[serde(with = "serde_bytes")]
        #[ts(type = "Uint8Array")]
        bytes: Vec<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        object_ahash64: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        program_identity: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        provenance: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        legacy_mapping: Option<LegacyFontMappingDto>,
    },
    FontUnavailable {
        #[serde(flatten)]
        key: FontRequestKeyDto,
    },
    PkFont {
        #[serde(flatten)]
        key: PkFontRequestKeyDto,
        virtual_path: String,
        #[serde(with = "serde_bytes")]
        #[ts(type = "Uint8Array")]
        bytes: Vec<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        expected_ahash64: Option<String>,
    },
    PkFontUnavailable {
        #[serde(flatten)]
        key: PkFontRequestKeyDto,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SessionLimits")]
pub struct SessionLimitsDto {
    #[ts(type = "number")]
    pub attempts: SafeInteger,
    #[ts(type = "number")]
    pub user_files: SafeInteger,
    #[ts(type = "number")]
    pub resolved_files: SafeInteger,
    #[ts(type = "number")]
    pub one_file_bytes: SafeInteger,
    #[ts(type = "number")]
    pub cached_file_bytes: SafeInteger,
    #[ts(type = "number")]
    pub user_source_bytes: SafeInteger,
    #[ts(type = "number")]
    pub output_bytes: SafeInteger,
    #[ts(type = "number")]
    pub engine_fuel: SafeInteger,
    #[ts(type = "number")]
    pub engine_steps: SafeInteger,
    #[ts(type = "number")]
    pub input_frames: SafeInteger,
    #[ts(type = "number")]
    pub journal_bytes: SafeInteger,
    #[ts(type = "number")]
    pub effects: SafeInteger,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SessionLimitOverrides", optional_fields)]
pub struct SessionLimitOverridesDto {
    #[ts(optional, type = "number")]
    pub attempts: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub user_files: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub resolved_files: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub one_file_bytes: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub cached_file_bytes: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub user_source_bytes: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub output_bytes: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub engine_fuel: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub engine_steps: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub input_frames: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub journal_bytes: Option<SafeInteger>,
    #[ts(optional, type = "number")]
    pub effects: Option<SafeInteger>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "EngineMode")]
pub enum EngineModeDto {
    Tex82,
    Etex,
    Pdftex,
    Latex,
    Pdflatex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "OutputCapability")]
pub enum OutputCapabilityDto {
    Dvi,
    Pdf,
    Html,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "JobClock")]
pub struct JobClockDto {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub minutes: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "FontLayoutPolicy")]
pub enum FontLayoutPolicyDto {
    OpentypePreferred,
    ClassicTfmExact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "FontMappingFallback")]
pub enum FontMappingFallbackDto {
    Error,
    ClassicTfmExact,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SessionOptions", optional_fields)]
pub struct SessionOptionsDto {
    pub main_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_bytes"
    )]
    #[ts(type = "Uint8Array | undefined")]
    pub format: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_prefetch_hints: Option<Vec<ResourceRequestDto>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineModeDto>,
    pub outputs: Vec<OutputCapabilityDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<JobClockDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "Partial<SessionLimits> | undefined")]
    pub limits: Option<SessionLimitOverridesDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_layout_policy: Option<FontLayoutPolicyDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_mapping_fallback: Option<FontMappingFallbackDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "BibliographyOutputFormat")]
pub enum BibliographyOutputFormatDto {
    Bbl,
    Bibtex,
    BiblatexXml,
    BblXml,
    Dot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "BibliographyOutputRequest")]
pub struct BibliographyOutputRequestDto {
    pub path: String,
    pub format: BibliographyOutputFormatDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "BibliographyMode")]
pub enum BibliographyModeDto {
    Biblatex,
    Classic,
    Auto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "BibliographyOptions", optional_fields)]
pub struct BibliographyOptionsDto {
    pub mode: Option<BibliographyModeDto>,
    pub control_path: Option<String>,
    #[serde(default)]
    pub outputs: Vec<BibliographyOutputRequestDto>,
    pub configuration_path: Option<String>,
    pub schema_paths: Option<Vec<String>>,
    pub aux_path: Option<String>,
    pub job_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FixedPointLimitOverrides", optional_fields)]
pub struct FixedPointLimitOverridesDto {
    pub attempts: Option<u32>,
    pub passes: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ProjectSessionOptions")]
pub struct ProjectSessionOptionsDto {
    #[serde(flatten)]
    #[ts(flatten)]
    pub session: SessionOptionsDto,
    pub bibliography: BibliographyOptionsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub project_limits: Option<FixedPointLimitOverridesDto>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EditorSessionOptions")]
pub struct EditorSessionOptionsDto {
    #[serde(flatten)]
    #[ts(flatten)]
    pub session: SessionOptionsDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stabilization_limits: Option<FixedPointLimitOverridesDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SourcePatch")]
pub struct SourcePatchDto {
    pub next_revision: u32,
    pub base_revision: u32,
    pub expected_hash: String,
    pub start: u32,
    pub end: u32,
    pub replacement: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "DiagnosticCode")]
pub enum DiagnosticCodeDto {
    Compile,
    Limit,
    AttemptLimit,
    NoProgress,
    ConflictingResource,
    UnexpectedResource,
    InvalidResource,
    InvalidOptions,
    InvalidPatch,
    Transaction,
    PassLimit,
    Oscillation,
    Bibliography,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CompileSourceLocation")]
pub struct CompileSourceLocationDto {
    pub file: String,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "BibliographyDiagnostic")]
pub struct BibliographyDiagnosticDto {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Diagnostic", optional_fields)]
pub struct DiagnosticDto {
    pub code: DiagnosticCodeDto,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CompileSourceLocationDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bibliography_diagnostics: Option<Vec<BibliographyDiagnosticDto>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CompileOutputFile")]
pub struct CompileOutputFileDto {
    pub path: String,
    #[serde(with = "serde_bytes")]
    #[ts(type = "Uint8Array")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CompileOutput", optional_fields)]
pub struct CompileOutputDto {
    pub outputs: Vec<OutputCapabilityDto>,
    pub terminal: String,
    #[serde(with = "serde_bytes")]
    #[ts(type = "Uint8Array")]
    pub log: Vec<u8>,
    #[serde(with = "serde_bytes")]
    #[ts(type = "Uint8Array")]
    pub dvi: Vec<u8>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_bytes"
    )]
    #[ts(type = "Uint8Array | undefined")]
    pub html: Option<Vec<u8>>,
    pub html_assets: Vec<CompileOutputFileDto>,
    pub files: Vec<CompileOutputFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_input_observations: Option<AcceptedInputObservationLedgerDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "BibliographyBackend")]
pub enum BibliographyBackendDto {
    Biblatex,
    Classic,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "BibliographyResult")]
pub struct BibliographyResultDto {
    pub backend: BibliographyBackendDto,
    pub files: Vec<CompileOutputFileDto>,
    pub diagnostics: Vec<BibliographyDiagnosticDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ProjectCompileOutput", optional_fields)]
pub struct ProjectCompileOutputDto {
    pub revision: u32,
    pub content_hash: String,
    pub passes: u32,
    pub tex: CompileOutputDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bibliography: Option<BibliographyResultDto>,
    pub generated_files: Vec<CompileOutputFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_input_observations: Option<AcceptedInputObservationLedgerDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EditorCompileOutput", optional_fields)]
pub struct EditorCompileOutputDto {
    pub revision: u32,
    pub content_hash: String,
    pub passes: u32,
    pub tex: CompileOutputDto,
    pub generated_files: Vec<CompileOutputFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_input_observations: Option<AcceptedInputObservationLedgerDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "ObservationNamespace")]
pub enum ObservationNamespaceDto {
    Authored,
    Generated,
    Distribution,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "ObservationOutcome")]
pub enum ObservationOutcomeDto {
    Present { content_hash: String },
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "ObservationAccess")]
pub enum ObservationAccessDto {
    RequiredRead,
    AuthoritativeProbe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "ObservationPhase")]
pub enum ObservationPhaseDto {
    Tex,
    BibliographyDetection,
    Bibliography,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "ObservationOwner")]
pub enum ObservationOwnerDto {
    TexEngine,
    BibliographyDetector,
    Biblatex,
    ClassicBibtex,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AcceptedInputObservation", optional_fields)]
pub struct AcceptedInputObservationDto {
    pub path: String,
    pub namespace: ObservationNamespaceDto,
    pub outcome: ObservationOutcomeDto,
    pub access: ObservationAccessDto,
    pub resource_kind: FileKindDto,
    pub phase: ObservationPhaseDto,
    pub revision: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_pass: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requesting_source: Option<String>,
    pub owner: ObservationOwnerDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AcceptedInputObservationLedger")]
pub struct AcceptedInputObservationLedgerDto {
    pub schema_version: u32,
    pub revision: u32,
    pub observations: Vec<AcceptedInputObservationDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "SameHistoryStop")]
pub enum SameHistoryStopDto {
    Matched,
    ScheduleDiverged,
    HashesDiverged,
    NoComparableBoundary,
    NotAttempted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "ReuseMetrics")]
pub struct ReuseMetricsDto {
    pub pages_reused: u32,
    pub pages_retyped: u32,
    #[ts(type = "number")]
    pub reexecuted_bytes: SafeInteger,
    #[ts(type = "number")]
    pub reexecuted_tokens: SafeInteger,
    #[ts(type = "number")]
    pub reexecuted_commands: SafeInteger,
    #[ts(type = "number")]
    pub reexecuted_paragraphs: SafeInteger,
    pub same_history_attempts: u32,
    pub same_history_hash_mismatches: u32,
    pub same_history_stop: SameHistoryStopDto,
    #[ts(type = "number")]
    pub restart_fork_microseconds: SafeInteger,
    #[ts(type = "number")]
    pub reexecution_microseconds: SafeInteger,
    #[ts(type = "number")]
    pub splice_microseconds: SafeInteger,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RetentionMetrics")]
pub struct RetentionMetricsDto {
    #[ts(type = "number")]
    pub checkpoint_root_bytes: SafeInteger,
    #[ts(type = "number")]
    pub diagnostic_bytes: SafeInteger,
    #[ts(type = "number")]
    pub output_bytes: SafeInteger,
    #[ts(type = "number")]
    pub resource_bytes: SafeInteger,
    #[ts(type = "number")]
    pub protected_overage_bytes: SafeInteger,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "RenderedSourceResult")]
pub enum RenderedSourceResultDto {
    Current {
        path: String,
        start: u32,
        end: u32,
        line: u32,
        column: u32,
    },
    Deleted {
        minted_revision: u32,
    },
    StaleRevision {
        accepted: u32,
    },
    OutputMismatch {
        accepted_output: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(rename = "CompileResultOutput")]
pub enum CompileResultOutputDto {
    Compile(CompileOutputDto),
    Project(ProjectCompileOutputDto),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "AttemptResult")]
pub enum AttemptResultDto {
    NeedResources {
        required: Vec<ResourceRequestDto>,
        probes: Vec<ResourceRequestDto>,
        prefetch_hints: Vec<ResourceRequestDto>,
    },
    Complete {
        output: Box<CompileResultOutputDto>,
    },
    Error {
        diagnostic: DiagnosticDto,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "EditorStatus")]
pub enum EditorStatusDto {
    Provisional {
        revision: u32,
        stabilization_required: bool,
    },
    Stabilizing {
        revision: u32,
        completed_passes: u32,
        stabilization_required: bool,
    },
    Stable {
        revision: u32,
        passes: u32,
        stabilization_required: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
#[ts(rename = "EditorAttemptResult")]
pub enum EditorAttemptResultDto {
    NeedResources {
        phase: EditorPhaseDto,
        required: Vec<ResourceRequestDto>,
        probes: Vec<ResourceRequestDto>,
        prefetch_hints: Vec<ResourceRequestDto>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        status: Option<EditorStatusDto>,
    },
    Provisional {
        revision: u32,
        stabilization_required: bool,
        output: EditorCompileOutputDto,
    },
    Stable {
        revision: u32,
        passes: u32,
        stabilization_required: bool,
        output: EditorCompileOutputDto,
    },
    Error {
        phase: EditorPhaseDto,
        diagnostic: DiagnosticDto,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "EditorPhase")]
pub enum EditorPhaseDto {
    Advance,
    Stabilization,
}

/// Stable authored-facade and worker error codes. Engine diagnostic codes use
/// [`DiagnosticCodeDto`] and bibliography diagnostics retain their own codes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "HostErrorCode")]
pub enum HostErrorCodeDto {
    Compile,
    Resolve,
    Resource,
    InvalidBinding,
    InvalidResolver,
    InvalidOptions,
    InvalidResource,
    InvalidResourceResponses,
    RemovedOption,
    Limit,
    AttemptLimit,
    NoProgress,
    OperationPending,
    Disposed,
    WorkerUnavailable,
    WorkerProtocol,
    Worker,
    Timeout,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogShard")]
pub struct CatalogShardDto {
    pub index: u32,
    pub object: String,
    pub ahash64: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogPreparedBatch")]
pub struct CatalogPreparedBatchDto {
    pub root: String,
    pub shards: Vec<CatalogShardDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "CatalogJobRequirement")]
pub enum CatalogJobRequirementDto {
    Required,
    Hint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(rename = "CatalogJobKind")]
pub enum CatalogJobKindDto {
    File,
    Font,
    LegacyFontMapping,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogJobEntry", optional_fields)]
pub struct CatalogJobEntryDto {
    pub object: String,
    pub ahash64: String,
    #[ts(type = "number")]
    pub bytes: SafeInteger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<FontContainerDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unicode_map: Option<Vec<Option<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FontMappingFallbackDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogJob")]
pub struct CatalogJobDto {
    pub manifest_key: String,
    pub requirement: CatalogJobRequirementDto,
    pub kind: CatalogJobKindDto,
    pub request_index: Option<u32>,
    pub entry: CatalogJobEntryDto,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogBatchPlan")]
pub struct CatalogBatchPlanDto {
    pub jobs: Vec<CatalogJobDto>,
    pub misses: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "FormatInputClosure")]
pub struct FormatInputClosureDto {
    pub schema: u32,
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "NamedFormat", optional_fields)]
pub struct NamedFormatDto {
    pub name: String,
    pub object: String,
    pub ahash64: String,
    #[ts(type = "number")]
    pub bytes: SafeInteger,
    pub engine: String,
    pub engine_version: String,
    pub format_schema: u32,
    pub source_distribution: String,
    pub source_manifest_ahash64: String,
    #[ts(type = "number")]
    pub source_date_epoch: SafeInteger,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_closure: Option<FormatInputClosureDto>,
}

/// Derives the complete TypeScript schema from the Rust DTO definitions.
///
/// Publication tooling may write this text into the low-level package
/// declaration. Keeping generation callable also lets tests detect a field or
/// representation change without parsing Rust source.
pub fn typescript_declarations() -> String {
    macro_rules! declarations {
        ($($ty:ty),+ $(,)?) => {{
            let mut output = String::from("// Generated from the schema-1 DTOs in wire.rs.\n");
            $(
                output.push_str("export ");
                output.push_str(&<$ty>::decl());
                output.push_str("\n");
            )+
            output
        }};
    }

    declarations!(
        ResourceDomainDto,
        FileKindDto,
        FileRequestKeyDto,
        FontCoordinateDto,
        VariationInstanceNameDto,
        VariationInstanceDto,
        WritingDirectionDto,
        FontContainerDto,
        FontRequestKeyDto,
        PkFontRequestKeyDto,
        ResourceRequestDto,
        LegacyFontMappingDto,
        ResourceResponseDto,
        SessionLimitsDto,
        SessionLimitOverridesDto,
        EngineModeDto,
        OutputCapabilityDto,
        JobClockDto,
        FontLayoutPolicyDto,
        FontMappingFallbackDto,
        SessionOptionsDto,
        BibliographyOutputFormatDto,
        BibliographyOutputRequestDto,
        BibliographyModeDto,
        BibliographyOptionsDto,
        FixedPointLimitOverridesDto,
        ProjectSessionOptionsDto,
        EditorSessionOptionsDto,
        SourcePatchDto,
        DiagnosticCodeDto,
        CompileSourceLocationDto,
        BibliographyDiagnosticDto,
        DiagnosticDto,
        CompileOutputFileDto,
        CompileOutputDto,
        BibliographyBackendDto,
        BibliographyResultDto,
        ProjectCompileOutputDto,
        EditorCompileOutputDto,
        ObservationNamespaceDto,
        ObservationOutcomeDto,
        ObservationAccessDto,
        ObservationPhaseDto,
        ObservationOwnerDto,
        AcceptedInputObservationDto,
        AcceptedInputObservationLedgerDto,
        SameHistoryStopDto,
        ReuseMetricsDto,
        RetentionMetricsDto,
        RenderedSourceResultDto,
        CompileResultOutputDto,
        AttemptResultDto,
        EditorStatusDto,
        EditorAttemptResultDto,
        EditorPhaseDto,
        HostErrorCodeDto,
        CatalogShardDto,
        CatalogPreparedBatchDto,
        CatalogJobRequirementDto,
        CatalogJobKindDto,
        CatalogJobEntryDto,
        CatalogJobDto,
        CatalogBatchPlanDto,
        FormatInputClosureDto,
        NamedFormatDto,
    )
}

/// Serializes a DTO as an ordinary JavaScript object while preserving byte
/// buffers as `Uint8Array` and omitting `None` properties.
pub fn to_js_value<T: Serialize + ?Sized>(
    value: &T,
) -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true))
        .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}

mod option_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(bytes) => serializer.serialize_some(&serde_bytes::Bytes::new(bytes)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<serde_bytes::ByteBuf>::deserialize(deserializer)
            .map(|bytes| bytes.map(serde_bytes::ByteBuf::into_vec))
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn diagnostic_codes_and_optional_omission_are_stable() {
        let diagnostic = DiagnosticDto {
            code: DiagnosticCodeDto::NoProgress,
            message: "no progress".to_owned(),
            location: None,
            bibliography_diagnostics: None,
        };
        assert_eq!(
            serde_json::to_value(diagnostic).expect("diagnostic"),
            serde_json::json!({ "code": "no-progress", "message": "no progress" })
        );

        let diagnostic_codes = [
            DiagnosticCodeDto::Compile,
            DiagnosticCodeDto::Limit,
            DiagnosticCodeDto::AttemptLimit,
            DiagnosticCodeDto::NoProgress,
            DiagnosticCodeDto::ConflictingResource,
            DiagnosticCodeDto::UnexpectedResource,
            DiagnosticCodeDto::InvalidResource,
            DiagnosticCodeDto::InvalidOptions,
            DiagnosticCodeDto::InvalidPatch,
            DiagnosticCodeDto::Transaction,
            DiagnosticCodeDto::PassLimit,
            DiagnosticCodeDto::Oscillation,
            DiagnosticCodeDto::Bibliography,
        ];
        assert_eq!(
            serde_json::to_value(diagnostic_codes).expect("diagnostic codes"),
            serde_json::json!([
                "compile",
                "limit",
                "attempt-limit",
                "no-progress",
                "conflicting-resource",
                "unexpected-resource",
                "invalid-resource",
                "invalid-options",
                "invalid-patch",
                "transaction",
                "pass-limit",
                "oscillation",
                "bibliography"
            ])
        );

        let host_codes = [
            HostErrorCodeDto::Compile,
            HostErrorCodeDto::Resolve,
            HostErrorCodeDto::Resource,
            HostErrorCodeDto::InvalidBinding,
            HostErrorCodeDto::InvalidResolver,
            HostErrorCodeDto::InvalidOptions,
            HostErrorCodeDto::InvalidResource,
            HostErrorCodeDto::InvalidResourceResponses,
            HostErrorCodeDto::RemovedOption,
            HostErrorCodeDto::Limit,
            HostErrorCodeDto::AttemptLimit,
            HostErrorCodeDto::NoProgress,
            HostErrorCodeDto::OperationPending,
            HostErrorCodeDto::Disposed,
            HostErrorCodeDto::WorkerUnavailable,
            HostErrorCodeDto::WorkerProtocol,
            HostErrorCodeDto::Worker,
            HostErrorCodeDto::Timeout,
        ];
        assert_eq!(
            serde_json::to_value(host_codes).expect("host codes"),
            serde_json::json!([
                "compile",
                "resolve",
                "resource",
                "invalid-binding",
                "invalid-resolver",
                "invalid-options",
                "invalid-resource",
                "invalid-resource-responses",
                "removed-option",
                "limit",
                "attempt-limit",
                "no-progress",
                "operation-pending",
                "disposed",
                "worker-unavailable",
                "worker-protocol",
                "worker",
                "timeout"
            ])
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn derived_typescript_names_binary_fields_explicitly() {
        let output = typescript_declarations();
        assert_eq!(output, include_str!("wire_schema.d.ts"));
        assert!(output.contains("log: Uint8Array"));
        assert!(output.contains("dvi: Uint8Array"));
        assert!(output.contains("html?: Uint8Array | undefined"), "{output}");
        assert!(
            output.contains("acceptedInputObservations?: AcceptedInputObservationLedger"),
            "{output}"
        );
        assert!(!output.contains("number[]"));

        assert!(output.contains("bytes: Uint8Array"));
        assert!(output.contains("attempts: number"));
        assert!(!output.contains("number[]"));
        assert!(!output.contains("base64"));
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn wasm_shape_uses_typed_arrays_omission_and_safe_numbers() {
        use js_sys::{Reflect, Uint8Array};
        use wasm_bindgen::{JsCast as _, JsValue};

        let output = CompileOutputDto {
            outputs: vec![OutputCapabilityDto::Dvi],
            terminal: "ok".to_owned(),
            log: vec![0, 255],
            dvi: vec![247, 2],
            html: None,
            html_assets: Vec::new(),
            files: Vec::new(),
            accepted_input_observations: None,
        };
        let value = to_js_value(&output).expect("serialize wire output");
        let field = |name| Reflect::get(&value, &JsValue::from_str(name)).expect("wire field");
        assert!(field("log").is_instance_of::<Uint8Array>());
        assert_eq!(Uint8Array::new(&field("log")).to_vec(), [0, 255]);
        assert!(!Reflect::has(&value, &JsValue::from_str("html")).expect("html presence"));
        assert!(
            !Reflect::has(&value, &JsValue::from_str("acceptedInputObservations"))
                .expect("ledger presence")
        );

        let limits = js_sys::Object::new();
        for name in [
            "userFiles",
            "resolvedFiles",
            "oneFileBytes",
            "cachedFileBytes",
            "userSourceBytes",
            "outputBytes",
            "engineFuel",
            "engineSteps",
            "inputFrames",
            "journalBytes",
            "effects",
        ] {
            Reflect::set(&limits, &JsValue::from_str(name), &JsValue::from_f64(1.0))
                .expect("limit field");
        }
        Reflect::set(
            &limits,
            &JsValue::from_str("attempts"),
            &JsValue::from_f64(MAX_SAFE_INTEGER as f64),
        )
        .expect("maximum safe limit");
        let maximum: SessionLimitsDto =
            serde_wasm_bindgen::from_value(limits.clone().unchecked_into())
                .expect("maximum safe integer");
        assert_eq!(maximum.attempts.get(), MAX_SAFE_INTEGER);
        Reflect::set(
            &limits,
            &JsValue::from_str("attempts"),
            &JsValue::from_f64((MAX_SAFE_INTEGER + 1) as f64),
        )
        .expect("unsafe limit");
        assert!(
            serde_wasm_bindgen::from_value::<SessionLimitsDto>(limits.unchecked_into()).is_err()
        );
    }
}

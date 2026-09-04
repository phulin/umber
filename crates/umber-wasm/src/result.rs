use js_sys::{Object, Reflect};
use umber::{
    CompileAttemptResult, CompileDiagnostic, CompileError, EditorSessionStatus,
    EditorStabilizationAttempt, LatexProjectAttempt, LatexProjectError, LatexProjectOutput,
    MemoryRunOutput, TexFixedPointError, TexFixedPointOutput,
};
use wasm_bindgen::{JsCast, JsValue};

use crate::{JsAttemptResult, JsEditorAttemptResult, JsEditorStatus, wire};

mod metrics;
mod render;

pub(crate) use metrics::{
    accepted_input_observations, rendered_source_result, retention_metrics, reuse_metrics,
};
pub(crate) use render::render_update;

pub(crate) fn attempt_result(result: CompileAttemptResult) -> Result<JsAttemptResult, JsValue> {
    typed(attempt_dto(result)?)
}

pub(crate) fn project_attempt_result(
    result: LatexProjectAttempt,
) -> Result<JsAttemptResult, JsValue> {
    let dto = match result {
        LatexProjectAttempt::NeedResources(resources) => need_resources(resources),
        LatexProjectAttempt::Complete(output) => wire::AttemptResultDto::Complete {
            output: Box::new(wire::CompileResultOutputDto::Project(project_output(
                *output,
            )?)),
        },
        LatexProjectAttempt::Error(error) => wire::AttemptResultDto::Error {
            diagnostic: project_diagnostic(error)?,
        },
    };
    typed(dto)
}

pub(crate) fn editor_advance_result(
    result: CompileAttemptResult,
    status: Option<EditorSessionStatus>,
    output: Option<&TexFixedPointOutput>,
) -> Result<JsEditorAttemptResult, JsValue> {
    let dto = match result {
        CompileAttemptResult::NeedResources(resources) => {
            editor_need_resources(wire::EditorPhaseDto::Advance, resources, None)?
        }
        CompileAttemptResult::Complete(_) => {
            let output = output.expect("completed editor advance retains display output");
            match status.expect("completed editor advance has status") {
                EditorSessionStatus::Provisional {
                    revision,
                    stabilization_required,
                } => wire::EditorAttemptResultDto::Provisional {
                    revision: revision_u32(revision)?,
                    stabilization_required,
                    output: tex_fixed_point_output(output.clone())?,
                },
                EditorSessionStatus::Stable {
                    revision,
                    passes,
                    stabilization_required,
                } => wire::EditorAttemptResultDto::Stable {
                    revision: revision_u32(revision)?,
                    passes,
                    stabilization_required,
                    output: tex_fixed_point_output(output.clone())?,
                },
                EditorSessionStatus::Stabilizing { .. } => {
                    unreachable!("advance cannot complete while stabilization is active")
                }
            }
        }
        CompileAttemptResult::Error(error) => wire::EditorAttemptResultDto::Error {
            phase: wire::EditorPhaseDto::Advance,
            diagnostic: diagnostic(error)?,
        },
    };
    typed(dto)
}

pub(crate) fn editor_stabilization_result(
    result: EditorStabilizationAttempt,
    status: Option<EditorSessionStatus>,
) -> Result<JsEditorAttemptResult, JsValue> {
    let dto = match result {
        EditorStabilizationAttempt::NeedResources(resources) => editor_need_resources(
            wire::EditorPhaseDto::Stabilization,
            resources,
            status.map(editor_status_dto).transpose()?,
        )?,
        EditorStabilizationAttempt::Complete(output) => wire::EditorAttemptResultDto::Stable {
            revision: revision_u32(output.revision)?,
            passes: output.passes,
            stabilization_required: false,
            output: tex_fixed_point_output(*output)?,
        },
        EditorStabilizationAttempt::Error(error) => wire::EditorAttemptResultDto::Error {
            phase: wire::EditorPhaseDto::Stabilization,
            diagnostic: tex_fixed_point_diagnostic(error)?,
        },
    };
    typed(dto)
}

pub(crate) fn editor_status(
    status: Option<EditorSessionStatus>,
) -> Result<Option<JsEditorStatus>, JsValue> {
    status
        .map(editor_status_dto)
        .transpose()?
        .map(|dto| Ok(wire::to_js_value(&dto)?.unchecked_into()))
        .transpose()
}

fn attempt_dto(result: CompileAttemptResult) -> Result<wire::AttemptResultDto, JsValue> {
    Ok(match result {
        CompileAttemptResult::NeedResources(resources) => need_resources(resources),
        CompileAttemptResult::Complete(output) => wire::AttemptResultDto::Complete {
            output: Box::new(wire::CompileResultOutputDto::Compile(compile_output(
                output,
            )?)),
        },
        CompileAttemptResult::Error(error) => wire::AttemptResultDto::Error {
            diagnostic: diagnostic(error)?,
        },
    })
}

fn need_resources(resources: umber::NeedResources) -> wire::AttemptResultDto {
    wire::AttemptResultDto::NeedResources {
        required: resources
            .required
            .into_iter()
            .map(resource_request)
            .collect(),
        probes: resources.probes.into_iter().map(resource_request).collect(),
        prefetch_hints: resources
            .prefetch_hints
            .into_iter()
            .map(resource_request)
            .collect(),
    }
}

fn editor_need_resources(
    phase: wire::EditorPhaseDto,
    resources: umber::NeedResources,
    status: Option<wire::EditorStatusDto>,
) -> Result<wire::EditorAttemptResultDto, JsValue> {
    Ok(wire::EditorAttemptResultDto::NeedResources {
        phase,
        required: resources
            .required
            .into_iter()
            .map(resource_request)
            .collect(),
        probes: resources.probes.into_iter().map(resource_request).collect(),
        prefetch_hints: resources
            .prefetch_hints
            .into_iter()
            .map(resource_request)
            .collect(),
        status,
    })
}

fn editor_status_dto(status: EditorSessionStatus) -> Result<wire::EditorStatusDto, JsValue> {
    Ok(match status {
        EditorSessionStatus::Provisional {
            revision,
            stabilization_required,
        } => wire::EditorStatusDto::Provisional {
            revision: revision_u32(revision)?,
            stabilization_required,
        },
        EditorSessionStatus::Stabilizing {
            revision,
            completed_passes,
            stabilization_required,
        } => wire::EditorStatusDto::Stabilizing {
            revision: revision_u32(revision)?,
            completed_passes,
            stabilization_required,
        },
        EditorSessionStatus::Stable {
            revision,
            passes,
            stabilization_required,
        } => wire::EditorStatusDto::Stable {
            revision: revision_u32(revision)?,
            passes,
            stabilization_required,
        },
    })
}

fn resource_request(request: umber::ResourceRequest) -> wire::ResourceRequestDto {
    match request {
        umber::ResourceRequest::File(request) => wire::ResourceRequestDto::File {
            key: file_request_key(request.key()),
            original_name: request.original_name().to_owned(),
        },
        umber::ResourceRequest::Font(request) => wire::ResourceRequestDto::Font {
            key: font_request_key(&request.key),
            accepted_containers: request
                .accepted_containers
                .contains(umber::FontContainer::Woff2)
                .then_some(wire::FontContainerDto::Woff2)
                .into_iter()
                .collect(),
        },
        umber::ResourceRequest::PkFont(request) => wire::ResourceRequestDto::PkFont {
            key: wire::PkFontRequestKeyDto {
                tex_name: request.tex_name().to_vec(),
                dpi: request.dpi(),
                mode: request.mode().to_vec(),
            },
        },
    }
}

fn file_request_key(key: &umber::FileRequestKey) -> wire::FileRequestKeyDto {
    wire::FileRequestKeyDto {
        domain: match key.domain() {
            umber::ResourceDomain::Tex => wire::ResourceDomainDto::Tex,
            umber::ResourceDomain::Bibliography => wire::ResourceDomainDto::Bibliography,
            umber::ResourceDomain::Generic => wire::ResourceDomainDto::Generic,
        },
        kind: file_kind(key.kind()),
        name: key.name().to_owned(),
    }
}

fn file_kind(kind: umber::FileKind) -> wire::FileKindDto {
    match kind {
        umber::FileKind::TexInput => wire::FileKindDto::Tex,
        umber::FileKind::Tfm => wire::FileKindDto::Tfm,
        umber::FileKind::FormatImage => wire::FileKindDto::Format,
        umber::FileKind::BibControl => wire::FileKindDto::BibControl,
        umber::FileKind::BibData => wire::FileKindDto::BibData,
        umber::FileKind::BibConfiguration => wire::FileKindDto::BibConfiguration,
        umber::FileKind::XmlSchema => wire::FileKindDto::XmlSchema,
        umber::FileKind::GenericAsset => wire::FileKindDto::Asset,
        umber::FileKind::Image => wire::FileKindDto::Image,
        umber::FileKind::BibAux => wire::FileKindDto::BibAux,
        umber::FileKind::ClassicBibData => wire::FileKindDto::ClassicBibData,
        umber::FileKind::BibStyle => wire::FileKindDto::BibStyle,
        umber::FileKind::VirtualFont => wire::FileKindDto::Vf,
        umber::FileKind::PdfFontMap => wire::FileKindDto::FontMap,
        umber::FileKind::PdfEncoding => wire::FileKindDto::FontEncoding,
        umber::FileKind::PdfFontProgram => wire::FileKindDto::FontProgram,
    }
}

fn font_request_key(key: &umber::FontRequestKey) -> wire::FontRequestKeyDto {
    let variation_instance = match key.variation.instance() {
        umber::VariationInstance::Default => {
            wire::VariationInstanceDto::Name(wire::VariationInstanceNameDto::Default)
        }
        umber::VariationInstance::Coordinates => {
            wire::VariationInstanceDto::Name(wire::VariationInstanceNameDto::Coordinates)
        }
        umber::VariationInstance::Named(named_name_id) => {
            wire::VariationInstanceDto::Named { named_name_id }
        }
    };
    wire::FontRequestKeyDto {
        logical_name: key.logical_name().to_owned(),
        face_index: key.face_index,
        variation_instance,
        variations: key
            .variation
            .coordinates()
            .iter()
            .map(|coordinate| wire::FontCoordinateDto {
                tag: coordinate.tag.to_string(),
                value: f64::from(coordinate.value),
            })
            .collect(),
        features: key
            .feature_policy
            .settings()
            .iter()
            .map(|feature| wire::FontCoordinateDto {
                tag: feature.tag.to_string(),
                value: f64::from(feature.value),
            })
            .collect(),
        direction: match key.direction {
            umber::WritingDirection::LeftToRight => wire::WritingDirectionDto::Ltr,
            umber::WritingDirection::RightToLeft => wire::WritingDirectionDto::Rtl,
        },
        script: key.script.map(|script| script.to_string()),
        language: key
            .language
            .as_ref()
            .map(|language| language.as_str().to_owned()),
    }
}

fn compile_output(output: MemoryRunOutput) -> Result<wire::CompileOutputDto, JsValue> {
    Ok(wire::CompileOutputDto {
        outputs: output
            .outputs
            .iter()
            .map(|capability| match capability {
                umber::OutputCapability::Dvi => wire::OutputCapabilityDto::Dvi,
                umber::OutputCapability::Pdf => wire::OutputCapabilityDto::Pdf,
                umber::OutputCapability::Html => wire::OutputCapabilityDto::Html,
            })
            .collect(),
        terminal: String::from_utf8_lossy(&output.terminal).into_owned(),
        log: output.log,
        dvi: output.dvi,
        html: output.html,
        html_assets: output
            .html_assets
            .into_iter()
            .map(|file| output_file(file.path.to_string_lossy().into_owned(), file.bytes))
            .collect(),
        files: output
            .files
            .into_iter()
            .map(|file| output_file(file.path.to_string_lossy().into_owned(), file.bytes))
            .collect(),
        accepted_input_observations: None,
    })
}

fn project_output(output: LatexProjectOutput) -> Result<wire::ProjectCompileOutputDto, JsValue> {
    Ok(wire::ProjectCompileOutputDto {
        revision: revision_u32(output.revision)?,
        content_hash: output.content_hash.hex(),
        passes: output.passes,
        tex: compile_output(output.tex)?,
        bibliography: output.bibliography.as_ref().map(bibliography_result),
        generated_files: output
            .generated_files
            .into_iter()
            .map(|file| output_file(file.path.to_string_lossy().into_owned(), file.bytes))
            .collect(),
        accepted_input_observations: None,
    })
}

pub(crate) fn tex_fixed_point_output(
    output: TexFixedPointOutput,
) -> Result<wire::EditorCompileOutputDto, JsValue> {
    Ok(wire::EditorCompileOutputDto {
        revision: revision_u32(output.revision)?,
        content_hash: output.content_hash.hex(),
        passes: output.passes,
        tex: compile_output(output.tex)?,
        generated_files: output
            .generated_files
            .into_iter()
            .map(|file| output_file(file.path.to_string_lossy().into_owned(), file.bytes))
            .collect(),
        accepted_input_observations: None,
    })
}

fn output_file(path: String, bytes: Vec<u8>) -> wire::CompileOutputFileDto {
    wire::CompileOutputFileDto { path, bytes }
}

fn bibliography_result(result: &bib_engine::BibliographyResult) -> wire::BibliographyResultDto {
    wire::BibliographyResultDto {
        backend: match result.backend() {
            bib_engine::BibliographyBackend::Biblatex => wire::BibliographyBackendDto::Biblatex,
            bib_engine::BibliographyBackend::Classic => wire::BibliographyBackendDto::Classic,
        },
        files: result
            .files()
            .map(|file| output_file(file.path().as_str().to_owned(), file.bytes().to_vec()))
            .collect(),
        diagnostics: result
            .diagnostics()
            .map(|diagnostic| wire::BibliographyDiagnosticDto {
                code: match diagnostic.code() {
                    bib_engine::BibliographyDiagnosticCode::Biblatex(code) => code.as_str(),
                    bib_engine::BibliographyDiagnosticCode::Classic(code) => code.as_str(),
                }
                .to_owned(),
                message: diagnostic.message().to_owned(),
            })
            .collect(),
    }
}

fn project_diagnostic(error: LatexProjectError) -> Result<wire::DiagnosticDto, JsValue> {
    let code = project_error_code_dto(&error);
    let message = error.to_string();
    let location = match &error {
        LatexProjectError::Compile(CompileError::Diagnostic(diagnostic)) => diagnostic
            .location
            .as_ref()
            .map(source_location)
            .transpose()?,
        _ => None,
    };
    let bibliography_diagnostics = match &error {
        LatexProjectError::Bibliography(bib_engine::BibliographyFailure::Biblatex(failure)) => {
            Some(
                failure
                    .diagnostics()
                    .map(|diagnostic| wire::BibliographyDiagnosticDto {
                        code: diagnostic.code().as_str().to_owned(),
                        message: diagnostic.message().to_owned(),
                    })
                    .collect(),
            )
        }
        _ => None,
    };
    Ok(wire::DiagnosticDto {
        code,
        message,
        location,
        bibliography_diagnostics,
    })
}

pub(crate) fn project_error_code(error: &LatexProjectError) -> &'static str {
    diagnostic_code_name(project_error_code_dto(error))
}

fn project_error_code_dto(error: &LatexProjectError) -> wire::DiagnosticCodeDto {
    match error {
        LatexProjectError::Compile(error) => compile_error_code_dto(error),
        LatexProjectError::Bibliography(failure) => match failure {
            bib_engine::BibliographyFailure::Biblatex(failure) => match failure.kind() {
                bib_engine::BibFailureKind::NoProgress => wire::DiagnosticCodeDto::NoProgress,
                bib_engine::BibFailureKind::Limit => wire::DiagnosticCodeDto::Limit,
                bib_engine::BibFailureKind::ResourceConflict => {
                    wire::DiagnosticCodeDto::ConflictingResource
                }
                _ => wire::DiagnosticCodeDto::Bibliography,
            },
            bib_engine::BibliographyFailure::Classic(bib_engine::ClassicBibFailure::NoProgress) => {
                wire::DiagnosticCodeDto::NoProgress
            }
            bib_engine::BibliographyFailure::Classic(bib_engine::ClassicBibFailure::Limit) => {
                wire::DiagnosticCodeDto::Limit
            }
            bib_engine::BibliographyFailure::Classic(
                bib_engine::ClassicBibFailure::ResourceConflict,
            ) => wire::DiagnosticCodeDto::ConflictingResource,
            _ => wire::DiagnosticCodeDto::Bibliography,
        },
        LatexProjectError::BibliographyFatal { .. } => wire::DiagnosticCodeDto::Bibliography,
        LatexProjectError::InvalidLimit { .. } => wire::DiagnosticCodeDto::InvalidOptions,
        LatexProjectError::PassLimit { .. } => wire::DiagnosticCodeDto::PassLimit,
        LatexProjectError::Oscillation { .. } => wire::DiagnosticCodeDto::Oscillation,
        LatexProjectError::UnexpectedResource(_) => wire::DiagnosticCodeDto::UnexpectedResource,
        LatexProjectError::ConflictingResource(_) => wire::DiagnosticCodeDto::ConflictingResource,
        LatexProjectError::Transaction(_) => wire::DiagnosticCodeDto::Transaction,
        LatexProjectError::InvalidPatch(_) => wire::DiagnosticCodeDto::InvalidPatch,
    }
}

fn tex_fixed_point_diagnostic(error: TexFixedPointError) -> Result<wire::DiagnosticDto, JsValue> {
    if let TexFixedPointError::Compile(error) = error {
        return diagnostic(error);
    }
    Ok(wire::DiagnosticDto {
        code: tex_fixed_point_error_code_dto(&error),
        message: error.to_string(),
        location: None,
        bibliography_diagnostics: None,
    })
}

pub(crate) fn tex_fixed_point_error_code(error: &TexFixedPointError) -> &'static str {
    diagnostic_code_name(tex_fixed_point_error_code_dto(error))
}

fn tex_fixed_point_error_code_dto(error: &TexFixedPointError) -> wire::DiagnosticCodeDto {
    match error {
        TexFixedPointError::Compile(error) => compile_error_code_dto(error),
        TexFixedPointError::InvalidLimit { .. } => wire::DiagnosticCodeDto::InvalidOptions,
        TexFixedPointError::PassLimit { .. } => wire::DiagnosticCodeDto::PassLimit,
        TexFixedPointError::Oscillation { .. } => wire::DiagnosticCodeDto::Oscillation,
        TexFixedPointError::Transaction(_) => wire::DiagnosticCodeDto::Transaction,
        TexFixedPointError::InvalidPatch(_) => wire::DiagnosticCodeDto::InvalidPatch,
        TexFixedPointError::UnexpectedResource(_) => wire::DiagnosticCodeDto::UnexpectedResource,
        TexFixedPointError::ConflictingResource(_) => wire::DiagnosticCodeDto::ConflictingResource,
    }
}

fn diagnostic(error: CompileError) -> Result<wire::DiagnosticDto, JsValue> {
    let code = compile_error_code_dto(&error);
    let diagnostic = match error {
        CompileError::Diagnostic(diagnostic) => diagnostic,
        error => CompileDiagnostic {
            message: error.to_string(),
            location: None,
            context: None,
            first_recoverable: None,
        },
    };
    Ok(wire::DiagnosticDto {
        code,
        message: diagnostic.message,
        location: diagnostic
            .location
            .as_ref()
            .map(source_location)
            .transpose()?,
        bibliography_diagnostics: None,
    })
}

fn source_location(
    location: &umber::CompileSourceLocation,
) -> Result<wire::CompileSourceLocationDto, JsValue> {
    Ok(wire::CompileSourceLocationDto {
        file: location.file.clone(),
        byte_start: u32::try_from(location.byte_start)
            .map_err(|_| crate::js_error("diagnostic byteStart is out of range"))?,
        byte_end: u32::try_from(location.byte_end)
            .map_err(|_| crate::js_error("diagnostic byteEnd is out of range"))?,
        line: location.line,
        column: location.column,
    })
}

pub(crate) fn compile_error_code(error: &CompileError) -> &'static str {
    diagnostic_code_name(compile_error_code_dto(error))
}

fn compile_error_code_dto(error: &CompileError) -> wire::DiagnosticCodeDto {
    match error {
        CompileError::InvalidCommandFuelLimit(_)
        | CompileError::HardLimitExceeded { .. }
        | CompileError::LimitExceeded { .. } => wire::DiagnosticCodeDto::Limit,
        CompileError::AttemptLimit { .. } => wire::DiagnosticCodeDto::AttemptLimit,
        CompileError::NoProgress => wire::DiagnosticCodeDto::NoProgress,
        CompileError::ConflictingResolvedBinding(_)
        | CompileError::ConflictingHtmlFontBinding { .. }
        | CompileError::DistributionPathCollision(_) => {
            wire::DiagnosticCodeDto::ConflictingResource
        }
        CompileError::UnexpectedResourceResponse(_) => wire::DiagnosticCodeDto::UnexpectedResource,
        CompileError::InvalidVirtualPath { .. }
        | CompileError::FileProvision(_)
        | CompileError::Font(_) => wire::DiagnosticCodeDto::InvalidResource,
        _ => wire::DiagnosticCodeDto::Compile,
    }
}

const fn diagnostic_code_name(code: wire::DiagnosticCodeDto) -> &'static str {
    match code {
        wire::DiagnosticCodeDto::Compile => "compile",
        wire::DiagnosticCodeDto::Limit => "limit",
        wire::DiagnosticCodeDto::AttemptLimit => "attempt-limit",
        wire::DiagnosticCodeDto::NoProgress => "no-progress",
        wire::DiagnosticCodeDto::ConflictingResource => "conflicting-resource",
        wire::DiagnosticCodeDto::UnexpectedResource => "unexpected-resource",
        wire::DiagnosticCodeDto::InvalidResource => "invalid-resource",
        wire::DiagnosticCodeDto::InvalidOptions => "invalid-options",
        wire::DiagnosticCodeDto::InvalidPatch => "invalid-patch",
        wire::DiagnosticCodeDto::Transaction => "transaction",
        wire::DiagnosticCodeDto::PassLimit => "pass-limit",
        wire::DiagnosticCodeDto::Oscillation => "oscillation",
        wire::DiagnosticCodeDto::Bibliography => "bibliography",
    }
}

fn revision_u32(revision: umber::RevisionId) -> Result<u32, JsValue> {
    u32::try_from(revision.raw()).map_err(|_| crate::js_error("revision is out of range"))
}

fn typed<T: serde::Serialize, U: JsCast>(dto: T) -> Result<U, JsValue> {
    Ok(wire::to_js_value(&dto)?.unchecked_into())
}

pub(super) fn set(object: &Object, name: &str, value: &JsValue) -> Result<(), JsValue> {
    Reflect::set(object, &JsValue::from_str(name), value).map(|_| ())
}

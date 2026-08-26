//! Binary-safe WebAssembly representation adapter for Umber.

mod catalog_boundary;
mod options;
mod result;
pub mod wire;

use js_sys::Uint8Array;
use options::{
    parse_editor_options, parse_options, parse_project_options, parse_resource_responses,
};
use result::attempt_result;
use umber::{EditorCompileSession, LatexProjectSession, VirtualCompileSession};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_TYPES: &str = include_str!("wire_schema.d.ts");

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SessionOptions")]
    pub type JsSessionOptions;

    #[wasm_bindgen(typescript_type = "ProjectSessionOptions")]
    pub type JsProjectSessionOptions;

    #[wasm_bindgen(typescript_type = "EditorSessionOptions")]
    pub type JsEditorSessionOptions;

    #[wasm_bindgen(typescript_type = "SourcePatch")]
    pub type JsSourcePatch;

    #[wasm_bindgen(typescript_type = "AttemptResult")]
    pub type JsAttemptResult;

    #[wasm_bindgen(typescript_type = "EditorAttemptResult")]
    pub type JsEditorAttemptResult;

    #[wasm_bindgen(typescript_type = "ResourceResponse[]")]
    pub type JsResourceResponses;

    #[wasm_bindgen(typescript_type = "RenderedSourceResult")]
    pub type JsRenderedSourceResult;

    #[wasm_bindgen(typescript_type = "AcceptedInputObservationLedger")]
    pub type JsAcceptedInputObservationLedger;

    #[wasm_bindgen(typescript_type = "ReuseMetrics")]
    pub type JsReuseMetrics;

    #[wasm_bindgen(typescript_type = "RetentionMetrics")]
    pub type JsRetentionMetrics;

    #[wasm_bindgen(typescript_type = "EditorStatus")]
    pub type JsEditorStatus;

    #[wasm_bindgen(typescript_type = "CatalogPreparedBatch")]
    pub type JsCatalogPreparedBatch;

    #[wasm_bindgen(typescript_type = "string[]")]
    pub type JsCatalogKeys;

    #[wasm_bindgen(typescript_type = "CatalogBatchPlan")]
    pub type JsCatalogBatchPlan;

    #[wasm_bindgen(typescript_type = "NamedFormat")]
    pub type JsNamedFormat;
}

#[wasm_bindgen]
pub struct CompilerSession {
    session: Option<VirtualCompileSession<'static>>,
}

#[wasm_bindgen]
pub struct ProjectSession {
    session: Option<LatexProjectSession<'static>>,
}

#[wasm_bindgen]
pub struct EditorSession {
    session: Option<EditorCompileSession<'static>>,
}

#[wasm_bindgen(js_name = packageVersion)]
pub fn package_version() -> String {
    umber::PACKAGE_VERSION.to_owned()
}

#[wasm_bindgen(js_name = formatSchemaVersion)]
pub fn format_schema_version() -> u32 {
    tex_state::FORMAT_SCHEMA_VERSION
}

#[wasm_bindgen(js_name = acceptedInputObservationSchemaVersion)]
pub fn accepted_input_observation_schema_version() -> u32 {
    umber::ACCEPTED_INPUT_OBSERVATION_SCHEMA_VERSION
}

/// Returns the compatibility version of the host-neutral WebAssembly DTOs.
#[wasm_bindgen(js_name = wireSchemaVersion)]
pub fn wire_schema_version() -> u32 {
    wire::SCHEMA_VERSION
}

/// Returns Umber's exact content identity for bytes supplied across the JS boundary.
#[wasm_bindgen(js_name = contentHash)]
pub fn content_hash(bytes: &Uint8Array) -> String {
    tex_state::ContentHash::from_bytes(&bytes.to_vec()).hex()
}

#[wasm_bindgen]
impl CompilerSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsSessionOptions) -> Result<CompilerSession, JsValue> {
        let options = parse_options(options.as_ref())?;
        let session = VirtualCompileSession::new_standalone(options).map_err(boundary_error)?;
        Ok(Self {
            session: Some(session),
        })
    }

    #[wasm_bindgen(js_name = addUserFile)]
    pub fn add_user_file(&mut self, path: &str, bytes: &Uint8Array) -> Result<(), JsValue> {
        self.session_mut()?
            .add_user_file(path, bytes.to_vec())
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = provideResources)]
    pub fn provide_resources(&mut self, responses: &JsResourceResponses) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .provide_resources(responses)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsAttemptResult, JsValue> {
        self.advance()
    }

    /// Advances synchronously until completion, error, or a typed resource batch.
    pub fn advance(&mut self) -> Result<JsAttemptResult, JsValue> {
        let result = self.session_mut()?.compile_attempt();
        attempt_result(result)
    }

    #[wasm_bindgen(js_name = applyPatch)]
    pub fn apply_patch(&mut self, patch: &JsSourcePatch) -> Result<(), JsValue> {
        let patch = options::parse_source_patch(patch.as_ref())?;
        self.session_mut()?
            .apply_patch(patch)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = clearDistributionCache)]
    pub fn clear_distribution_cache(&mut self) -> Result<(), JsValue> {
        self.session_mut()?
            .clear_distribution_cache()
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = cancelPendingPatch)]
    pub fn cancel_pending_patch(&mut self) -> Result<bool, JsValue> {
        Ok(self.session_mut()?.cancel_pending_patch())
    }

    #[wasm_bindgen(js_name = renderUpdate)]
    pub fn render_update(&self) -> Result<JsValue, JsValue> {
        self.session_ref()?
            .render_update()
            .map(result::render_update)
            .transpose()
            .map(|value| value.unwrap_or(JsValue::NULL))
    }

    #[wasm_bindgen(js_name = acknowledgeRenderUpdate)]
    pub fn acknowledge_render_update(
        &mut self,
        revision: u32,
        digest: &str,
    ) -> Result<(), JsValue> {
        let digest = umber::RenderDigest::parse_hex(digest)
            .ok_or_else(|| js_error("render digest must be 64 lowercase hexadecimal digits"))?;
        self.session_mut()?
            .acknowledge_render_update(u64::from(revision), digest)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = renderResync)]
    pub fn render_resync(&self) -> Result<JsValue, JsValue> {
        self.session_ref()?
            .render_resync()
            .as_ref()
            .map(result::render_update)
            .transpose()
            .map(|value| value.unwrap_or(JsValue::NULL))
    }

    pub fn dispose(&mut self) {
        self.session = None;
    }

    #[wasm_bindgen(getter)]
    pub fn disposed(&self) -> bool {
        self.session.is_none()
    }

    #[wasm_bindgen(getter)]
    pub fn attempts(&self) -> Result<u32, JsValue> {
        Ok(self.session_ref()?.attempts())
    }

    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> Result<Option<u32>, JsValue> {
        self.session_ref()?
            .revision()
            .map(|revision| {
                u32::try_from(revision.raw())
                    .map_err(|_| js_error("accepted revision exceeds the WASM revision range"))
            })
            .transpose()
    }

    #[wasm_bindgen(getter, js_name = contentHash)]
    pub fn accepted_content_hash(&self) -> Result<Option<String>, JsValue> {
        Ok(self.session_ref()?.content_hash().map(|hash| hash.hex()))
    }

    /// Resolves a rendered HTML event and optional text-unit index lazily.
    #[wasm_bindgen(js_name = renderedSourceLocation)]
    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: String,
        revision: u32,
    ) -> Result<Option<JsRenderedSourceResult>, JsValue> {
        let output_id = umber::RenderedOutputId::parse_hex(&output_id)
            .ok_or_else(|| js_error("rendered output identity must be 32 hexadecimal digits"))?;
        match self
            .session_ref()?
            .rendered_source_location(
                page,
                event,
                unit,
                output_id,
                umber::RevisionId::new(u64::from(revision)),
            )
            .map_err(boundary_error)?
        {
            Some(result) => result::rendered_source_result(result).map(Some),
            None => Ok(None),
        }
    }

    #[wasm_bindgen(getter, js_name = reuseMetrics)]
    pub fn reuse_metrics(&self) -> Result<Option<JsReuseMetrics>, JsValue> {
        result::reuse_metrics(self.session_ref()?.reuse_metrics())
    }

    #[wasm_bindgen(getter, js_name = retentionMetrics)]
    pub fn retention_metrics(&self) -> Result<Option<JsRetentionMetrics>, JsValue> {
        result::retention_metrics(self.session_ref()?.retention_metrics())
    }

    #[wasm_bindgen(getter, js_name = acceptedInputObservations)]
    pub fn accepted_input_observations(
        &self,
    ) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
        result::accepted_input_observations(
            self.session_ref()?.accepted_input_observations().as_ref(),
        )
    }

    #[wasm_bindgen(getter, js_name = resolvedFileCount)]
    pub fn resolved_file_count(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.resolved_file_count())
    }

    #[wasm_bindgen(getter, js_name = cachedFileBytes)]
    pub fn cached_file_bytes(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.cached_file_bytes())
    }
}

#[wasm_bindgen]
impl EditorSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsEditorSessionOptions) -> Result<EditorSession, JsValue> {
        let options = parse_editor_options(options.as_ref())?;
        let session =
            EditorCompileSession::new_standalone(options).map_err(compile_boundary_error)?;
        Ok(Self {
            session: Some(session),
        })
    }

    #[wasm_bindgen(js_name = addUserFile)]
    pub fn add_user_file(&mut self, path: &str, bytes: &Uint8Array) -> Result<(), JsValue> {
        self.session_mut()?
            .add_user_file(path, bytes.to_vec())
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = provideResources)]
    pub fn provide_resources(&mut self, responses: &JsResourceResponses) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .provide_resources(responses)
            .map_err(editor_resource_boundary_error)
    }

    /// Runs exactly one latency-critical editor pass.
    pub fn advance(&mut self) -> Result<JsEditorAttemptResult, JsValue> {
        let session = self.session_mut()?;
        let attempt = session.advance();
        result::editor_advance_result(attempt, session.status(), session.display_output())
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsEditorAttemptResult, JsValue> {
        self.advance()
    }

    #[wasm_bindgen(js_name = stabilizeAttempt)]
    pub fn stabilize_attempt(&mut self) -> Result<JsEditorAttemptResult, JsValue> {
        let session = self.session_mut()?;
        let attempt = session.stabilize_attempt();
        result::editor_stabilization_result(attempt, session.status())
    }

    #[wasm_bindgen(js_name = applyPatch)]
    pub fn apply_patch(&mut self, patch: &JsSourcePatch) -> Result<(), JsValue> {
        let patch = options::parse_source_patch(patch.as_ref())?;
        self.session_mut()?
            .apply_patch(patch)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = cancelPendingPatch)]
    pub fn cancel_pending_patch(&mut self) -> Result<bool, JsValue> {
        Ok(self.session_mut()?.cancel_pending_patch())
    }

    #[wasm_bindgen(js_name = cancelStabilization)]
    pub fn cancel_stabilization(&mut self) -> Result<bool, JsValue> {
        Ok(self.session_mut()?.cancel_stabilization())
    }

    #[wasm_bindgen(js_name = renderUpdate)]
    pub fn render_update(&self) -> Result<JsValue, JsValue> {
        self.session_ref()?
            .render_update()
            .map(result::render_update)
            .transpose()
            .map(|value| value.unwrap_or(JsValue::NULL))
    }

    #[wasm_bindgen(js_name = acknowledgeRenderUpdate)]
    pub fn acknowledge_render_update(
        &mut self,
        revision: u32,
        digest: &str,
    ) -> Result<(), JsValue> {
        let digest = umber::RenderDigest::parse_hex(digest)
            .ok_or_else(|| js_error("render digest must be 64 lowercase hexadecimal digits"))?;
        self.session_mut()?
            .acknowledge_render_update(u64::from(revision), digest)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = renderResync)]
    pub fn render_resync(&self) -> Result<JsValue, JsValue> {
        self.session_ref()?
            .render_resync()
            .as_ref()
            .map(result::render_update)
            .transpose()
            .map(|value| value.unwrap_or(JsValue::NULL))
    }

    #[wasm_bindgen(getter)]
    pub fn status(&self) -> Result<Option<JsEditorStatus>, JsValue> {
        result::editor_status(self.session_ref()?.status())
    }

    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> Result<Option<u32>, JsValue> {
        self.session_ref()?
            .revision()
            .map(|revision| {
                u32::try_from(revision.raw())
                    .map_err(|_| js_error("accepted revision exceeds the WASM revision range"))
            })
            .transpose()
    }

    #[wasm_bindgen(getter, js_name = contentHash)]
    pub fn accepted_content_hash(&self) -> Result<Option<String>, JsValue> {
        Ok(self.session_ref()?.content_hash().map(|hash| hash.hex()))
    }

    /// Resolves a rendered HTML event against the current editor display.
    #[wasm_bindgen(js_name = renderedSourceLocation)]
    pub fn rendered_source_location(
        &self,
        page: u32,
        event: u32,
        unit: Option<u32>,
        output_id: String,
        revision: u32,
    ) -> Result<Option<JsRenderedSourceResult>, JsValue> {
        let output_id = umber::RenderedOutputId::parse_hex(&output_id)
            .ok_or_else(|| js_error("rendered output identity must be 32 hexadecimal digits"))?;
        match self
            .session_ref()?
            .rendered_source_location(
                page,
                event,
                unit,
                output_id,
                umber::RevisionId::new(u64::from(revision)),
            )
            .map_err(compile_boundary_error)?
        {
            Some(result) => result::rendered_source_result(result).map(Some),
            None => Ok(None),
        }
    }

    #[wasm_bindgen(getter, js_name = reuseMetrics)]
    pub fn reuse_metrics(&self) -> Result<Option<JsReuseMetrics>, JsValue> {
        result::reuse_metrics(self.session_ref()?.reuse_metrics())
    }

    #[wasm_bindgen(getter, js_name = retentionMetrics)]
    pub fn retention_metrics(&self) -> Result<Option<JsRetentionMetrics>, JsValue> {
        result::retention_metrics(self.session_ref()?.retention_metrics())
    }

    #[wasm_bindgen(getter, js_name = acceptedInputObservations)]
    pub fn accepted_input_observations(
        &self,
    ) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
        result::accepted_input_observations(
            self.session_ref()?.accepted_input_observations().as_ref(),
        )
    }

    #[wasm_bindgen(getter, js_name = resolvedFileCount)]
    pub fn resolved_file_count(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.resolved_file_count())
    }

    #[wasm_bindgen(getter, js_name = cachedFileBytes)]
    pub fn cached_file_bytes(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.cached_file_bytes())
    }

    pub fn dispose(&mut self) {
        self.session = None;
    }

    #[wasm_bindgen(getter)]
    pub fn disposed(&self) -> bool {
        self.session.is_none()
    }
}

#[wasm_bindgen]
impl ProjectSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsProjectSessionOptions) -> Result<ProjectSession, JsValue> {
        let options = parse_project_options(options.as_ref())?;
        let session =
            LatexProjectSession::new_standalone(options).map_err(project_boundary_error)?;
        Ok(Self {
            session: Some(session),
        })
    }

    #[wasm_bindgen(js_name = addUserFile)]
    pub fn add_user_file(&mut self, path: &str, bytes: &Uint8Array) -> Result<(), JsValue> {
        self.session_mut()?
            .add_user_file(path, bytes.to_vec())
            .map_err(project_boundary_error)
    }

    #[wasm_bindgen(js_name = provideResources)]
    pub fn provide_resources(&mut self, responses: &JsResourceResponses) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .provide_resources(responses)
            .map_err(project_boundary_error)
    }

    pub fn advance(&mut self) -> Result<JsAttemptResult, JsValue> {
        result::project_attempt_result(self.session_mut()?.compile_attempt())
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsAttemptResult, JsValue> {
        self.advance()
    }

    #[wasm_bindgen(js_name = applyPatch)]
    pub fn apply_patch(&mut self, patch: &JsSourcePatch) -> Result<(), JsValue> {
        let patch = options::parse_source_patch(patch.as_ref())?;
        self.session_mut()?
            .apply_patch(patch)
            .map_err(project_boundary_error)
    }

    #[wasm_bindgen(js_name = cancelPendingPatch)]
    pub fn cancel_pending_patch(&mut self) -> Result<bool, JsValue> {
        Ok(self.session_mut()?.cancel_pending_patch())
    }

    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> Result<Option<u32>, JsValue> {
        let revision = self.session_ref()?.revision();
        revision
            .map(|revision| {
                u32::try_from(revision.raw())
                    .map_err(|_| js_error("accepted revision exceeds the WASM revision range"))
            })
            .transpose()
    }

    #[wasm_bindgen(getter, js_name = contentHash)]
    pub fn accepted_content_hash(&self) -> Result<Option<String>, JsValue> {
        Ok(self.session_ref()?.content_hash().map(|hash| hash.hex()))
    }

    #[wasm_bindgen(getter, js_name = acceptedInputObservations)]
    pub fn accepted_input_observations(
        &self,
    ) -> Result<Option<JsAcceptedInputObservationLedger>, JsValue> {
        result::accepted_input_observations(self.session_ref()?.accepted_input_observations())
    }

    pub fn dispose(&mut self) {
        self.session = None;
    }

    #[wasm_bindgen(getter)]
    pub fn disposed(&self) -> bool {
        self.session.is_none()
    }
}

impl EditorSession {
    fn session_ref(&self) -> Result<&EditorCompileSession<'static>, JsValue> {
        self.session
            .as_ref()
            .ok_or_else(|| js_error("EditorSession has been disposed"))
    }

    fn session_mut(&mut self) -> Result<&mut EditorCompileSession<'static>, JsValue> {
        self.session
            .as_mut()
            .ok_or_else(|| js_error("EditorSession has been disposed"))
    }
}

impl ProjectSession {
    fn session_ref(&self) -> Result<&LatexProjectSession<'static>, JsValue> {
        self.session
            .as_ref()
            .ok_or_else(|| js_error("ProjectSession has been disposed"))
    }

    fn session_mut(&mut self) -> Result<&mut LatexProjectSession<'static>, JsValue> {
        self.session
            .as_mut()
            .ok_or_else(|| js_error("ProjectSession has been disposed"))
    }
}

impl CompilerSession {
    fn session_ref(&self) -> Result<&VirtualCompileSession<'static>, JsValue> {
        self.session
            .as_ref()
            .ok_or_else(|| js_error("CompilerSession has been disposed"))
    }

    fn session_mut(&mut self) -> Result<&mut VirtualCompileSession<'static>, JsValue> {
        self.session
            .as_mut()
            .ok_or_else(|| js_error("CompilerSession has been disposed"))
    }
}

fn boundary_error(error: impl std::fmt::Display) -> JsValue {
    js_error(&error.to_string())
}

fn compile_boundary_error(error: umber::CompileError) -> JsValue {
    let value = js_sys::Error::new(&error.to_string());
    tag_js_error(value.into(), result::compile_error_code(&error))
}

fn project_boundary_error(error: umber::LatexProjectError) -> JsValue {
    let value = js_sys::Error::new(&error.to_string());
    tag_js_error(value.into(), result::project_error_code(&error))
}

fn editor_resource_boundary_error(error: umber::EditorResourceError) -> JsValue {
    let code = match &error {
        umber::EditorResourceError::Advance(error) => result::compile_error_code(error),
        umber::EditorResourceError::Stabilization(error) => {
            result::tex_fixed_point_error_code(error)
        }
    };
    let value = js_sys::Error::new(&error.to_string());
    tag_js_error(value.into(), code)
}

fn tag_js_error(value: JsValue, code: &str) -> JsValue {
    js_sys::Reflect::set(&value, &JsValue::from_str("code"), &JsValue::from_str(code))
        .expect("Error objects accept a code property");
    value
}

fn js_error(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}

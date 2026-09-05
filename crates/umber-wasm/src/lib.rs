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
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::{from_value, to_value};
use umber::{
    EditorCompileSession, FileRequest, LatexProjectSession, ResourceResponse, VirtualCompileSession,
};
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

fn prefetch_files(responses: &[ResourceResponse]) -> Vec<FileRequest> {
    responses
        .iter()
        .filter_map(|response| match response {
            ResourceResponse::File(file) => Some(FileRequest::new(
                file.request.clone(),
                file.virtual_path.clone(),
            )),
            ResourceResponse::FileUnavailable(_)
            | ResourceResponse::Font(_)
            | ResourceResponse::FontUnavailable(_)
            | ResourceResponse::PkFont(_)
            | ResourceResponse::PkFontUnavailable(_) => None,
        })
        .collect()
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

#[derive(Serialize)]
struct JsLiteralHint {
    kind: String,
    #[serde(rename = "originalSpelling")]
    original_spelling: String,
    name: String,
    #[serde(rename = "byteOffset")]
    byte_offset: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsLiteralHintLimits {
    max_hints: usize,
    max_name_bytes: usize,
}

#[derive(Deserialize)]
struct JsPrefetchCandidate {
    key: String,
    #[serde(default)]
    object: String,
    #[serde(default)]
    ahash64: String,
    bytes: u64,
    #[serde(default)]
    class: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    required: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchBudget {
    max_files: usize,
    max_bytes: u64,
    max_runtime_bytes: u64,
    max_font_bytes: u64,
    max_image_bytes: u64,
    max_document_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchSelection {
    required_keys: Vec<String>,
    hint_keys: Vec<String>,
    hint_file_keys: Vec<JsPrefetchFileKey>,
    demand_bytes: u64,
    prefetch_bytes: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchFileKey {
    domain: String,
    kind: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchRequest {
    key: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    original_spelling: String,
    search_context: String,
    #[serde(default)]
    class: Option<String>,
    required: bool,
    depth: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchRequestOutput {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    original_spelling: String,
    search_context: String,
    class: String,
    required: bool,
    depth: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsPrefetchEscalation {
    tier: u8,
    discarded_work_delta: u64,
}

fn prefetch_class(class: &str) -> umber_distribution::PrefetchClass {
    match class {
        "small-runtime" => umber_distribution::PrefetchClass::SmallRuntime,
        "font" => umber_distribution::PrefetchClass::Font,
        "image" => umber_distribution::PrefetchClass::Image,
        "document" => umber_distribution::PrefetchClass::Document,
        _ => umber_distribution::PrefetchClass::Other,
    }
}

fn prefetch_class_name(class: umber_distribution::PrefetchClass) -> &'static str {
    match class {
        umber_distribution::PrefetchClass::SmallRuntime => "small-runtime",
        umber_distribution::PrefetchClass::Font => "font",
        umber_distribution::PrefetchClass::Image => "image",
        umber_distribution::PrefetchClass::Document => "document",
        umber_distribution::PrefetchClass::Other => "other",
    }
}

fn prefetch_request(request: JsPrefetchRequest) -> umber_distribution::PrefetchRequest {
    let file_key = request
        .domain
        .as_deref()
        .zip(request.kind.as_deref())
        .zip(request.name.as_deref())
        .and_then(|((domain, kind), name)| {
            umber_distribution::PrefetchFileKey::new(domain, kind, name)
        });
    let class = request.class.as_deref().map_or_else(
        || umber_distribution::PrefetchClass::for_key(&request.key),
        prefetch_class,
    );
    let output = file_key
        .map_or_else(
            || {
                umber_distribution::PrefetchRequest::new(
                    request.key.clone(),
                    request.original_spelling.clone(),
                    request.search_context.clone(),
                    request.required,
                )
            },
            |file_key| {
                umber_distribution::PrefetchRequest::for_file_key(
                    file_key,
                    request.key.clone(),
                    request.original_spelling.clone(),
                    request.search_context.clone(),
                    request.required,
                )
            },
        )
        .with_class(class)
        .with_depth(request.depth);
    output
}

fn prefetch_request_value(request: umber_distribution::PrefetchRequest) -> JsPrefetchRequestOutput {
    let depth = request.depth();
    JsPrefetchRequestOutput {
        key: request.key,
        domain: request.file_key.as_ref().map(|key| key.domain.clone()),
        kind: request.file_key.as_ref().map(|key| key.kind.clone()),
        name: request
            .file_key
            .as_ref()
            .map(|key| key.normalized_name.clone()),
        original_spelling: request.original_spelling,
        search_context: request.search_context,
        class: prefetch_class_name(request.class).to_owned(),
        required: request.required,
        depth,
    }
}

fn prefetch_candidate(candidate: JsPrefetchCandidate) -> umber_distribution::PrefetchCandidate {
    let class = candidate.class.as_deref().map_or_else(
        || umber_distribution::PrefetchClass::for_key(&candidate.key),
        prefetch_class,
    );
    let file_key = candidate
        .domain
        .as_deref()
        .zip(candidate.kind.as_deref())
        .zip(candidate.name.as_deref())
        .and_then(|((domain, kind), name)| {
            umber_distribution::PrefetchFileKey::new(domain, kind, name)
        });
    umber_distribution::PrefetchCandidate {
        key: candidate.key,
        object: umber_distribution::ObjectEntry {
            object: candidate.object,
            ahash64: candidate.ahash64,
            bytes: candidate.bytes,
        },
        class,
        required: candidate.required,
        file_key,
    }
}

fn prefetch_budget(budget: JsPrefetchBudget) -> umber_distribution::PrefetchBudget {
    umber_distribution::PrefetchBudget {
        max_files: budget.max_files,
        max_bytes: budget.max_bytes,
        max_runtime_bytes: budget.max_runtime_bytes,
        max_font_bytes: budget.max_font_bytes,
        max_image_bytes: budget.max_image_bytes,
        max_document_bytes: budget.max_document_bytes,
        ..umber_distribution::PrefetchBudget::default()
    }
}

fn prefetch_selection_value(
    selection: umber_distribution::PrefetchSelection,
) -> JsPrefetchSelection {
    let hint_file_keys = selection
        .hints
        .iter()
        .filter_map(|item| item.file_key.as_ref())
        .map(|key| JsPrefetchFileKey {
            domain: key.domain.clone(),
            kind: key.kind.clone(),
            name: key.normalized_name.clone(),
        })
        .collect();
    JsPrefetchSelection {
        required_keys: selection
            .required
            .into_iter()
            .map(|item| item.key)
            .collect(),
        hint_keys: selection.hints.into_iter().map(|item| item.key).collect(),
        hint_file_keys,
        demand_bytes: selection.demand_bytes,
        prefetch_bytes: selection.prefetch_bytes,
    }
}

/// Stateful WASM adapter for the host-neutral Rust prefetch policy.
///
/// JavaScript owns transport, provider precedence, and VFS admission. This
/// object owns queue identity, lexical closure, budgets, and replay escalation
/// for each resolver run so browser scheduling cannot drift from native.
#[wasm_bindgen(js_name = PrefetchPolicySession)]
pub struct PrefetchPolicySession {
    policy: umber_distribution::PrefetchPolicy,
    budget_configured: bool,
}

#[wasm_bindgen]
impl PrefetchPolicySession {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            policy: umber_distribution::PrefetchPolicy::new(
                umber_distribution::PrefetchBudget::default(),
            ),
            budget_configured: false,
        }
    }

    #[wasm_bindgen(js_name = enqueue)]
    pub fn enqueue(&mut self, requests: JsValue) -> Result<(), JsValue> {
        let requests = from_value::<Vec<JsPrefetchRequest>>(requests)
            .map_err(|error| js_error(&format!("invalid prefetch requests: {error}")))?;
        for request in requests {
            self.policy.enqueue(prefetch_request(request));
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = enqueueEscalation)]
    pub fn enqueue_escalation(&mut self, requests: JsValue, priority: u64) -> Result<(), JsValue> {
        let requests = from_value::<Vec<JsPrefetchRequest>>(requests)
            .map_err(|error| js_error(&format!("invalid prefetch escalation requests: {error}")))?;
        for request in requests {
            self.policy
                .enqueue_with_priority(prefetch_request(request), priority);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = enqueueLiteralHints)]
    pub fn enqueue_literal_hints(&mut self, source: &str) -> usize {
        self.policy.enqueue_literal_hints(source)
    }

    #[wasm_bindgen(js_name = select)]
    pub fn select(
        &mut self,
        required: JsValue,
        candidates: JsValue,
        budget: JsValue,
    ) -> Result<JsValue, JsValue> {
        let required = from_value::<Vec<JsPrefetchCandidate>>(required)
            .map_err(|error| js_error(&format!("invalid required prefetch candidates: {error}")))?
            .into_iter()
            .map(prefetch_candidate);
        let candidates = from_value::<Vec<JsPrefetchCandidate>>(candidates)
            .map_err(|error| js_error(&format!("invalid prefetch candidates: {error}")))?
            .into_iter()
            .map(prefetch_candidate);
        let budget = from_value::<JsPrefetchBudget>(budget)
            .map_err(|error| js_error(&format!("invalid prefetch budget: {error}")))?;
        if !self.budget_configured {
            self.policy.configure_budget(prefetch_budget(budget));
            self.budget_configured = true;
        }
        let selection = self.policy.select_prefetch_group(required, candidates);
        to_value(&prefetch_selection_value(selection))
            .map_err(|error| js_error(&format!("failed to encode prefetch selection: {error}")))
    }

    #[wasm_bindgen(js_name = drain)]
    pub fn drain(&mut self, limit: usize) -> Result<JsValue, JsValue> {
        let requests = self
            .policy
            .drain(limit)
            .into_iter()
            .map(prefetch_request_value)
            .collect::<Vec<_>>();
        to_value(&requests)
            .map_err(|error| js_error(&format!("failed to encode prefetch requests: {error}")))
    }

    #[wasm_bindgen(js_name = dependencyClosure)]
    pub fn dependency_closure(&self, key: &str, tier: u8) -> Result<JsValue, JsValue> {
        let requests = self
            .policy
            .dependency_closure(key, tier)
            .into_iter()
            .map(prefetch_request_value)
            .collect::<Vec<_>>();
        to_value(&requests)
            .map_err(|error| js_error(&format!("failed to encode dependency closure: {error}")))
    }

    #[wasm_bindgen(js_name = dependencyClosureRequest)]
    pub fn dependency_closure_request(
        &self,
        request: JsValue,
        tier: u8,
    ) -> Result<JsValue, JsValue> {
        let request = from_value::<JsPrefetchRequest>(request)
            .map_err(|error| js_error(&format!("invalid dependency request: {error}")))?;
        let request = prefetch_request(request);
        let file_key = request
            .file_key
            .as_ref()
            .ok_or_else(|| js_error("dependency request is missing semantic file key"))?;
        let requests = self
            .policy
            .dependency_closure_for_file_key(file_key, tier)
            .into_iter()
            .map(prefetch_request_value)
            .collect::<Vec<_>>();
        to_value(&requests)
            .map_err(|error| js_error(&format!("failed to encode dependency closure: {error}")))
    }

    #[wasm_bindgen(js_name = admit)]
    pub fn admit(
        &mut self,
        key: &str,
        virtual_path: &str,
        bytes: &Uint8Array,
        dependencies: Option<JsValue>,
    ) -> Result<(), JsValue> {
        self.admit_impl(key, virtual_path, bytes, None, dependencies)
    }

    /// Admission variant that preserves the semantic resource class when a
    /// file kind aliases a distribution key. This keeps image bytes out of
    /// the runtime-text scanner even when the image spelling has a `.sty`
    /// suffix.
    #[wasm_bindgen(js_name = admitWithClass)]
    pub fn admit_with_class(
        &mut self,
        key: &str,
        virtual_path: &str,
        bytes: &Uint8Array,
        class: Option<String>,
        dependencies: Option<JsValue>,
    ) -> Result<(), JsValue> {
        self.admit_impl(key, virtual_path, bytes, class.as_deref(), dependencies)
    }

    #[wasm_bindgen(js_name = admitRequest)]
    pub fn admit_request(
        &mut self,
        request: JsValue,
        virtual_path: &str,
        bytes: &Uint8Array,
        class: Option<String>,
        dependencies: Option<JsValue>,
    ) -> Result<(), JsValue> {
        let request = from_value::<JsPrefetchRequest>(request)
            .map_err(|error| js_error(&format!("invalid admitted prefetch request: {error}")))?;
        let request = prefetch_request(request);
        let dependencies = dependencies
            .filter(|value| !value.is_undefined() && !value.is_null())
            .map(|value| {
                from_value::<Vec<JsPrefetchRequest>>(value).map_err(|error| {
                    js_error(&format!("invalid admitted prefetch dependencies: {error}"))
                })
            })
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(prefetch_request);
        let class = class.as_deref().map_or(request.class, prefetch_class);
        let _ = virtual_path;
        self.policy
            .admitted_request_with_class(&request, class, &bytes.to_vec(), dependencies);
        Ok(())
    }

    fn admit_impl(
        &mut self,
        key: &str,
        virtual_path: &str,
        bytes: &Uint8Array,
        class: Option<&str>,
        dependencies: Option<JsValue>,
    ) -> Result<(), JsValue> {
        let dependencies = dependencies
            .filter(|value| !value.is_undefined() && !value.is_null())
            .map(|value| {
                from_value::<Vec<JsPrefetchRequest>>(value).map_err(|error| {
                    js_error(&format!("invalid admitted prefetch dependencies: {error}"))
                })
            })
            .transpose()?;
        let dependencies = dependencies
            .unwrap_or_default()
            .into_iter()
            .map(prefetch_request);
        if let Some(class) = class {
            self.policy.admitted_with_class(
                key,
                prefetch_class(class),
                &bytes.to_vec(),
                dependencies,
            );
        } else {
            self.policy
                .admitted_with_metadata(key, virtual_path, &bytes.to_vec(), dependencies);
        }
        Ok(())
    }

    #[wasm_bindgen(js_name = noteReplay)]
    pub fn note_replay(
        &mut self,
        region: &str,
        request_key: &str,
        discarded_work: u64,
    ) -> Result<JsValue, JsValue> {
        let Some(region) = umber_distribution::PrefetchRegionKey::new(region.to_owned()) else {
            return Ok(JsValue::NULL);
        };
        let escalation = self.policy.note_replay(region, request_key, discarded_work);
        escalation.map_or(Ok(JsValue::NULL), |escalation| {
            to_value(&JsPrefetchEscalation {
                tier: escalation.tier,
                discarded_work_delta: escalation.discarded_work_delta,
            })
            .map_err(|error| js_error(&format!("failed to encode prefetch escalation: {error}")))
        })
    }

    #[wasm_bindgen(js_name = noteReplayRequest)]
    pub fn note_replay_request(
        &mut self,
        region: &str,
        request: JsValue,
        discarded_work: u64,
    ) -> Result<JsValue, JsValue> {
        let request = from_value::<JsPrefetchRequest>(request)
            .map_err(|error| js_error(&format!("invalid replay request: {error}")))?;
        let request = prefetch_request(request);
        let Some(file_key) = request.file_key.as_ref() else {
            return Ok(JsValue::NULL);
        };
        let Some(region) = umber_distribution::PrefetchRegionKey::new(region.to_owned()) else {
            return Ok(JsValue::NULL);
        };
        let escalation = self
            .policy
            .note_replay_for_file_key(region, file_key, discarded_work);
        escalation.map_or(Ok(JsValue::NULL), |escalation| {
            to_value(&JsPrefetchEscalation {
                tier: escalation.tier,
                discarded_work_delta: escalation.discarded_work_delta,
            })
            .map_err(|error| js_error(&format!("failed to encode prefetch escalation: {error}")))
        })
    }
}

impl Default for PrefetchPolicySession {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsReplayContext {
    region: String,
    discarded_work: u64,
}

fn replay_context_value(context: Option<(String, u64)>) -> Result<JsValue, JsValue> {
    context.map_or(Ok(JsValue::NULL), |(region, discarded_work)| {
        to_value(&JsReplayContext {
            region,
            discarded_work,
        })
        .map_err(|error| js_error(&format!("failed to encode replay context: {error}")))
    })
}

#[wasm_bindgen(js_name = prefetchPolicyVersion)]
pub fn prefetch_policy_version() -> String {
    umber::prefetch::PREFETCH_POLICY_VERSION.to_owned()
}

/// Shared lexical policy DTO. Hosts retain transport and provider ordering,
/// while extraction stays identical to native policy scheduling.
#[wasm_bindgen(js_name = prefetchLiteralHints)]
pub fn prefetch_literal_hints(source: &str, limits: Option<JsValue>) -> Result<JsValue, JsValue> {
    let limits = match limits.filter(|value| !value.is_undefined() && !value.is_null()) {
        Some(value) => {
            let limits = from_value::<JsLiteralHintLimits>(value)
                .map_err(|error| js_error(&format!("invalid prefetch hint limits: {error}")))?;
            umber_distribution::LiteralHintLimits {
                max_hints: limits.max_hints,
                max_name_bytes: limits.max_name_bytes,
            }
        }
        None => umber_distribution::LiteralHintLimits::default(),
    };
    let hints = umber_distribution::extract_literal_hints(source, limits)
        .into_iter()
        .map(|hint| JsLiteralHint {
            kind: hint.kind.command().to_owned(),
            original_spelling: hint.original_spelling,
            name: hint.name,
            byte_offset: hint.byte_offset,
        })
        .collect::<Vec<_>>();
    to_value(&hints).map_err(|error| js_error(&format!("failed to encode prefetch hints: {error}")))
}

#[wasm_bindgen(js_name = prefetchSelect)]
pub fn prefetch_select(
    required: JsValue,
    candidates: JsValue,
    budget: JsValue,
) -> Result<JsValue, JsValue> {
    let required = from_value::<Vec<JsPrefetchCandidate>>(required)
        .map_err(|error| js_error(&format!("invalid required prefetch candidates: {error}")))?;
    let candidates = from_value::<Vec<JsPrefetchCandidate>>(candidates)
        .map_err(|error| js_error(&format!("invalid prefetch candidates: {error}")))?;
    let budget = from_value::<JsPrefetchBudget>(budget)
        .map_err(|error| js_error(&format!("invalid prefetch budget: {error}")))?;
    let selection = umber_distribution::select_prefetch_group(
        required.into_iter().map(prefetch_candidate),
        candidates.into_iter().map(prefetch_candidate),
        prefetch_budget(budget),
    );
    to_value(&prefetch_selection_value(selection))
        .map_err(|error| js_error(&format!("failed to encode prefetch selection: {error}")))
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

    #[wasm_bindgen(js_name = authorizePrefetchResources)]
    pub fn authorize_prefetch_resources(
        &mut self,
        responses: &JsResourceResponses,
    ) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .authorize_prefetch_files(prefetch_files(&responses));
        Ok(())
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsAttemptResult, JsValue> {
        self.advance()
    }

    #[wasm_bindgen(js_name = resourceReplayContext)]
    pub fn resource_replay_context(&self) -> Result<JsValue, JsValue> {
        replay_context_value(self.session_ref()?.resource_replay_context())
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

    #[wasm_bindgen(js_name = authorizePrefetchResources)]
    pub fn authorize_prefetch_resources(
        &mut self,
        responses: &JsResourceResponses,
    ) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .authorize_prefetch_files(prefetch_files(&responses));
        Ok(())
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

    #[wasm_bindgen(js_name = resourceReplayContext)]
    pub fn resource_replay_context(&self) -> Result<JsValue, JsValue> {
        replay_context_value(self.session_ref()?.resource_replay_context())
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

    #[wasm_bindgen(js_name = authorizePrefetchResources)]
    pub fn authorize_prefetch_resources(
        &mut self,
        responses: &JsResourceResponses,
    ) -> Result<(), JsValue> {
        let responses = parse_resource_responses(responses.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .authorize_prefetch_files(prefetch_files(&responses));
        Ok(())
    }

    pub fn advance(&mut self) -> Result<JsAttemptResult, JsValue> {
        result::project_attempt_result(self.session_mut()?.compile_attempt())
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsAttemptResult, JsValue> {
        self.advance()
    }

    #[wasm_bindgen(js_name = resourceReplayContext)]
    pub fn resource_replay_context(&self) -> Result<JsValue, JsValue> {
        replay_context_value(self.session_ref()?.resource_replay_context())
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

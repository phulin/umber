//! Binary-safe WebAssembly representation adapter for Umber.

mod options;
mod result;

use js_sys::{Array, Uint8Array};
use options::{parse_options, parse_project_options, parse_request_key, parse_resource_responses};
use result::attempt_result;
use umber::{LatexProjectSession, VirtualCompileSession};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_TYPES: &str = r#"
export type ResourceDomain = "tex" | "bibliography" | "generic";
export type FileKind = "tex" | "tfm" | "format" | "bib-control" | "bib-data" | "bib-configuration" | "xml-schema" | "asset" | "image" | "bib-aux" | "classic-bib-data" | "bib-style";

export interface FileRequestKey {
  domain: ResourceDomain;
  kind: FileKind;
  name: string;
}

export interface FileRequest extends FileRequestKey {
  type: "file";
  originalName: string;
}

export interface FontRequestKey {
  logicalName: string;
  faceIndex: number;
  variations: Array<{ tag: string; value: number }>;
  features: Array<{ tag: string; enabled: boolean }>;
}

export interface FontRequest extends FontRequestKey {
  type: "font";
  acceptedContainers: Array<"woff2">;
}

export type ResourceRequest = FileRequest | FontRequest;
export type ResourceResponse =
  | (FileRequestKey & { type: "file"; virtualPath: string; bytes: Uint8Array; expectedContentId?: string })
  | (FileRequestKey & { type: "file-unavailable" })
  | (FontRequestKey & {
      type: "font";
      container: "woff2";
      bytes: Uint8Array;
      objectSha256?: string;
      programIdentity?: string;
      provenance?: string;
    })
  | (FontRequestKey & { type: "font-unavailable" });

export interface SessionLimits {
  attempts: number;
  userFiles: number;
  resolvedFiles: number;
  oneFileBytes: number;
  cachedFileBytes: number;
  userSourceBytes: number;
  outputBytes: number;
}

export interface SessionOptions {
  mainPath: string;
  jobName?: string;
  format?: Uint8Array;
  /** Authenticated format-closure requests, consumed as one-shot cache hints. */
  formatPrefetchHints?: FileRequest[];
  engine?: "tex82" | "etex" | "pdftex" | "latex" | "pdflatex";
  clock?: { year: number; month: number; day: number; minutes: number };
  limits?: Partial<SessionLimits>;
  html?: { fonts: HtmlFontInput[] };
}

export type BibliographyOutputFormat = "bbl" | "bibtex" | "biblatex-xml" | "bbl-xml" | "dot";

export interface ProjectSessionOptions extends SessionOptions {
  bibliography: {
    /** Omit mode for the original biblatex-only option shape. */
    mode?: "biblatex" | "classic" | "auto";
    controlPath: string;
    outputs: Array<{ path: string; format: BibliographyOutputFormat }>;
    configurationPath?: string;
    schemaPaths?: string[];
    auxPath?: string;
    jobPath?: string;
  };
  projectLimits?: { attempts?: number; passes?: number };
}

export interface SourcePatch {
  nextRevision: number;
  baseRevision: number;
  expectedHash: string;
  start: number;
  end: number;
  replacement: string;
}

export interface ReuseMetrics {
  pagesReused: number;
  pagesRetyped: number;
  reexecutedBytes: number;
  reexecutedTokens: number;
  reexecutedCommands: number;
  reexecutedParagraphs: number;
  sameHistoryAttempts: number;
  sameHistoryHashMismatches: number;
  sameHistoryStop: "matched" | "schedule-diverged" | "hashes-diverged" | "no-comparable-boundary" | "not-attempted";
  restartForkMicroseconds: number;
  reexecutionMicroseconds: number;
  spliceMicroseconds: number;
}

export interface RetentionMetrics {
  checkpointRootBytes: number;
  diagnosticBytes: number;
  outputBytes: number;
  resourceBytes: number;
  protectedOverageBytes: number;
}

export interface HtmlFontInput {
  name: string;
  tfmContentHash: string;
  woff2: Uint8Array;
  sha256: string;
  encoding: Array<string | null>;
  provenance: string;
  embeddable: boolean;
}

export interface CompileOutputFile {
  path: string;
  bytes: Uint8Array;
}

export interface CompileOutput {
  terminal: string;
  log: Uint8Array;
  dvi: Uint8Array;
  html?: Uint8Array;
  htmlAssets: CompileOutputFile[];
  files: CompileOutputFile[];
}

export interface Diagnostic {
  code: string;
  message: string;
  file?: string;
  line?: number;
  column?: number;
}

export interface BibliographyDiagnostic {
  code: string;
  message: string;
}

export interface BibliographyResult {
  backend: "biblatex" | "classic";
  files: CompileOutputFile[];
  diagnostics: BibliographyDiagnostic[];
}

export interface ProjectCompileOutput {
  revision: number;
  contentHash: string;
  passes: number;
  tex: CompileOutput;
  bibliography?: BibliographyResult;
  generatedFiles: CompileOutputFile[];
}

export type RenderedSourceResult =
  | { kind: "current"; path: string; start: number; end: number; line: number; column: number }
  | { kind: "deleted"; mintedRevision: number }
  | { kind: "stale-revision"; accepted: number }
  | { kind: "output-mismatch"; acceptedOutput: string };

export type AttemptResult =
  | { kind: "need-resources"; required: ResourceRequest[]; probes: ResourceRequest[]; prefetchHints: ResourceRequest[] }
  | { kind: "complete"; output: CompileOutput | ProjectCompileOutput }
  | { kind: "error"; diagnostic: Diagnostic & { bibliographyDiagnostics?: Array<{ code: string; message: string }> } };
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SessionOptions")]
    pub type JsSessionOptions;

    #[wasm_bindgen(typescript_type = "ProjectSessionOptions")]
    pub type JsProjectSessionOptions;

    #[wasm_bindgen(typescript_type = "FileRequestKey")]
    pub type JsFileRequestKey;

    #[wasm_bindgen(typescript_type = "HtmlFontInput")]
    pub type JsHtmlFontInput;

    #[wasm_bindgen(typescript_type = "SourcePatch")]
    pub type JsSourcePatch;

    #[wasm_bindgen(typescript_type = "AttemptResult")]
    pub type JsAttemptResult;

    #[wasm_bindgen(typescript_type = "ResourceResponse")]
    pub type JsResourceResponse;

    #[wasm_bindgen(typescript_type = "RenderedSourceResult")]
    pub type JsRenderedSourceResult;
}

#[wasm_bindgen]
pub struct CompilerSession {
    session: Option<VirtualCompileSession>,
}

#[wasm_bindgen]
pub struct ProjectSession {
    session: Option<LatexProjectSession>,
}

#[wasm_bindgen(js_name = packageVersion)]
pub fn package_version() -> String {
    umber::PACKAGE_VERSION.to_owned()
}

#[wasm_bindgen(js_name = formatSchemaVersion)]
pub fn format_schema_version() -> u32 {
    tex_state::Universe::FORMAT_SCHEMA_VERSION
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
        let session = VirtualCompileSession::new(options).map_err(boundary_error)?;
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

    #[wasm_bindgen(js_name = addHtmlFont)]
    pub fn add_html_font(&mut self, font: &JsHtmlFontInput) -> Result<(), JsValue> {
        let font = options::parse_html_font(font.as_ref())?;
        self.session_mut()?
            .add_html_font(font)
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = provideResolvedFile)]
    pub fn provide_resolved_file(
        &mut self,
        request: &JsFileRequestKey,
        #[allow(non_snake_case)] virtualPath: &str,
        bytes: &Uint8Array,
    ) -> Result<(), JsValue> {
        let request = parse_request_key(request.as_ref())
            .map_err(|error| tag_js_error(error, "invalid-resource"))?;
        self.session_mut()?
            .provide_resolved_file(request, virtualPath, bytes.to_vec())
            .map_err(compile_boundary_error)
    }

    #[wasm_bindgen(js_name = provideResources)]
    pub fn provide_resources(&mut self, responses: &Array) -> Result<(), JsValue> {
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
    pub fn reuse_metrics(&self) -> Result<JsValue, JsValue> {
        result::reuse_metrics(self.session_ref()?.reuse_metrics())
    }

    #[wasm_bindgen(getter, js_name = retentionMetrics)]
    pub fn retention_metrics(&self) -> Result<JsValue, JsValue> {
        result::retention_metrics(self.session_ref()?.retention_metrics())
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
impl ProjectSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options: &JsProjectSessionOptions) -> Result<ProjectSession, JsValue> {
        let options = parse_project_options(options.as_ref())?;
        let session = LatexProjectSession::new(options).map_err(project_boundary_error)?;
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
    pub fn provide_resources(&mut self, responses: &Array) -> Result<(), JsValue> {
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

    pub fn dispose(&mut self) {
        self.session = None;
    }

    #[wasm_bindgen(getter)]
    pub fn disposed(&self) -> bool {
        self.session.is_none()
    }
}

impl ProjectSession {
    fn session_ref(&self) -> Result<&LatexProjectSession, JsValue> {
        self.session
            .as_ref()
            .ok_or_else(|| js_error("ProjectSession has been disposed"))
    }

    fn session_mut(&mut self) -> Result<&mut LatexProjectSession, JsValue> {
        self.session
            .as_mut()
            .ok_or_else(|| js_error("ProjectSession has been disposed"))
    }
}

impl CompilerSession {
    fn session_ref(&self) -> Result<&VirtualCompileSession, JsValue> {
        self.session
            .as_ref()
            .ok_or_else(|| js_error("CompilerSession has been disposed"))
    }

    fn session_mut(&mut self) -> Result<&mut VirtualCompileSession, JsValue> {
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

fn tag_js_error(value: JsValue, code: &str) -> JsValue {
    js_sys::Reflect::set(&value, &JsValue::from_str("code"), &JsValue::from_str(code))
        .expect("Error objects accept a code property");
    value
}

fn js_error(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}

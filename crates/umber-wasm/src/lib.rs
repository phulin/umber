//! Binary-safe WebAssembly representation adapter for Umber.

mod options;
mod result;

use js_sys::Uint8Array;
use options::{parse_options, parse_request_key};
use result::attempt_result;
use umber::VirtualCompileSession;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_TYPES: &str = r#"
export type FileKind = "tex" | "tfm";

export interface FileRequestKey {
  kind: FileKind;
  name: string;
}

export interface FileRequest extends FileRequestKey {
  originalName: string;
}

export interface SessionLimits {
  attempts: number;
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
  clock?: { year: number; month: number; day: number; minutes: number };
  limits?: Partial<SessionLimits>;
}

export interface CompileOutputFile {
  path: string;
  bytes: Uint8Array;
}

export interface CompileOutput {
  terminal: string;
  log: Uint8Array;
  dvi: Uint8Array;
  files: CompileOutputFile[];
}

export interface Diagnostic {
  message: string;
  file?: string;
  line?: number;
  column?: number;
}

export type AttemptResult =
  | { kind: "need-files"; files: FileRequest[] }
  | { kind: "complete"; output: CompileOutput }
  | { kind: "error"; diagnostic: Diagnostic };
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SessionOptions")]
    pub type JsSessionOptions;

    #[wasm_bindgen(typescript_type = "FileRequestKey")]
    pub type JsFileRequestKey;

    #[wasm_bindgen(typescript_type = "AttemptResult")]
    pub type JsAttemptResult;
}

#[wasm_bindgen]
pub struct CompilerSession {
    session: Option<VirtualCompileSession>,
}

#[wasm_bindgen(js_name = packageVersion)]
pub fn package_version() -> String {
    umber::PACKAGE_VERSION.to_owned()
}

#[wasm_bindgen(js_name = formatSchemaVersion)]
pub fn format_schema_version() -> u32 {
    tex_state::Universe::FORMAT_SCHEMA_VERSION
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
            .map_err(boundary_error)
    }

    #[wasm_bindgen(js_name = provideResolvedFile)]
    pub fn provide_resolved_file(
        &mut self,
        request: &JsFileRequestKey,
        #[allow(non_snake_case)] virtualPath: &str,
        bytes: &Uint8Array,
    ) -> Result<(), JsValue> {
        let request = parse_request_key(request.as_ref())?;
        self.session_mut()?
            .provide_resolved_file(request, virtualPath, bytes.to_vec())
            .map_err(boundary_error)
    }

    #[wasm_bindgen(js_name = compileAttempt)]
    pub fn compile_attempt(&mut self) -> Result<JsAttemptResult, JsValue> {
        let result = self.session_mut()?.compile_attempt();
        attempt_result(result)
    }

    #[wasm_bindgen(js_name = clearDistributionCache)]
    pub fn clear_distribution_cache(&mut self) -> Result<(), JsValue> {
        self.session_mut()?.clear_distribution_cache();
        Ok(())
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

    #[wasm_bindgen(getter, js_name = resolvedFileCount)]
    pub fn resolved_file_count(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.resolved_file_count())
    }

    #[wasm_bindgen(getter, js_name = cachedFileBytes)]
    pub fn cached_file_bytes(&self) -> Result<usize, JsValue> {
        Ok(self.session_ref()?.cached_file_bytes())
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

fn js_error(message: &str) -> JsValue {
    js_sys::Error::new(message).into()
}

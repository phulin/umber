//! Native host policy for driving one CLI compile through the resource loop.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tex_fonts::{AcceptedFontContainers, FontContainer, FontObjectIdentity, ResolvedFont};
use tex_state::World;
use umber_distribution::{
    FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey, Manifest,
    ManifestMiss, ManifestRequest, select,
};
use umber_fetch::{FetchClient, FetchClientConfig, FetchRequest, ObjectCache, fetch_manifest};

use crate::{
    CompileAttemptResult, EngineMode, FileContentId, FileKind, FileRequest, MemoryRunOutput,
    ResolvedFile, ResourceRequest, ResourceResponse, SessionOptions, TexFontSearchPath,
    TexInputSearchPath, VirtualCompileSession,
};

pub const DEFAULT_DISTRIBUTION_URL: &str = "https://static.umber.dev/texlive/latest/manifest.json";
// Phase 6 rotates this placeholder to the digest of the first published snapshot.
pub const DEFAULT_DISTRIBUTION_SHA256: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Debug)]
pub struct NativeRunOptions {
    pub input: PathBuf,
    pub format: Option<PathBuf>,
    pub engine: EngineMode,
    pub html: bool,
    pub distribution: Option<String>,
    pub distribution_sha256: Option<String>,
    pub offline: bool,
}

#[derive(Debug)]
pub enum NativeRunError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Cache(String),
    ManifestFetch(String),
    ManifestDigestMismatch {
        expected: String,
        actual: String,
    },
    ManifestParse(String),
    DistributionPinRequired(String),
    DistributionUnavailable(Vec<String>),
    Selection(String),
    Fetch(String),
    Compile(String),
    Format(String),
}

impl fmt::Display for NativeRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "failed to read {}: {source}", path.display()),
            Self::Cache(message) => write!(f, "distribution cache error: {message}"),
            Self::ManifestFetch(message) => write!(f, "distribution manifest error: {message}"),
            Self::ManifestDigestMismatch { expected, actual } => write!(
                f,
                "distribution manifest digest mismatch: expected {expected}, received {actual}"
            ),
            Self::ManifestParse(message) => write!(f, "invalid distribution manifest: {message}"),
            Self::DistributionPinRequired(source) => write!(
                f,
                "distribution {source} requires --distribution-sha256 (or UMBER_DISTRIBUTION_SHA256)"
            ),
            Self::DistributionUnavailable(keys) => write!(
                f,
                "distribution unavailable for required request(s): {}",
                keys.join(", ")
            ),
            Self::Selection(message) => write!(f, "distribution selection error: {message}"),
            Self::Fetch(message) => f.write_str(message),
            Self::Compile(message) => f.write_str(message),
            Self::Format(message) => write!(f, "format resource error: {message}"),
        }
    }
}

impl Error for NativeRunError {}

pub fn run(options: &NativeRunOptions) -> Result<MemoryRunOutput, NativeRunError> {
    let main = read(&options.input)?;
    let cache = ObjectCache::from_environment()
        .map_err(|error| NativeRunError::Cache(error.to_string()))?;
    let mut distribution = DistributionResolver::new(
        &cache,
        options.distribution.clone(),
        options.distribution_sha256.clone(),
        options.offline,
    );
    let format = match &options.format {
        Some(path) if path.exists() => Some(read(path)?),
        Some(path) => Some(distribution.resolve_format(path, options.engine)?),
        None => None,
    };
    let clock = World::real().job_clock();
    let name = options
        .input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("main.tex");
    let job_name = options
        .input
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("texput")
        .to_owned();
    let mut session = VirtualCompileSession::new(SessionOptions {
        main_path: format!("/job/{name}"),
        job_name: Some(job_name),
        format,
        engine: options.engine,
        clock,
        html: options.html,
        accepted_font_containers: if options.html {
            AcceptedFontContainers::WASM
        } else {
            AcceptedFontContainers::NATIVE_WITH_COLLECTIONS
        },
        ..SessionOptions::default()
    })
    .map_err(|error| NativeRunError::Compile(error.to_string()))?;
    session
        .add_user_file(name, main)
        .map_err(|error| NativeRunError::Compile(error.to_string()))?;
    let local = LocalResolver::from_environment(&options.input);
    loop {
        match session.compile_attempt() {
            CompileAttemptResult::Complete(output) => return Ok(output),
            CompileAttemptResult::Error(error) => {
                return Err(NativeRunError::Compile(error.to_string()));
            }
            CompileAttemptResult::NeedResources(batch) => {
                let responses = distribution.resolve_batch(&local, &batch.required)?;
                session
                    .provide_resources(responses)
                    .map_err(|error| NativeRunError::Compile(error.to_string()))?;
            }
        }
    }
}

struct LocalResolver {
    input: TexInputSearchPath,
    font: TexFontSearchPath,
}

impl LocalResolver {
    fn from_environment(main: &Path) -> Self {
        let areas = |name| {
            env::var_os(name)
                .map(|value| {
                    env::split_paths(&value)
                        .filter(|path| !path.as_os_str().is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let base = main.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        Self {
            input: TexInputSearchPath::new(&base, areas("TEXINPUTS")),
            font: TexFontSearchPath::new(base, areas("TEXFONTS")),
        }
    }

    fn resolve(&self, request: &FileRequest) -> Option<ResolvedFile> {
        let mut world = World::real();
        let content = match request.key().kind() {
            FileKind::TexInput | FileKind::Image => self
                .input
                .read_from_world(&mut world, request.original_name()),
            FileKind::Tfm => self
                .font
                .read_from_world(&mut world, Path::new(request.original_name())),
            _ => return None,
        }
        .ok()?;
        let bytes = content.bytes().to_vec();
        let digest = FileContentId::for_bytes(&bytes);
        Some(ResolvedFile {
            request: request.key().clone(),
            virtual_path: format!("/texlive/local/{digest}"),
            expected_digest: Some(digest),
            bytes,
        })
    }
}

#[derive(Clone)]
struct LoadedDistribution {
    manifest: Manifest,
    local_root: Option<PathBuf>,
}

struct DistributionResolver<'a> {
    cache: &'a ObjectCache,
    source: Option<String>,
    expected: Option<String>,
    offline: bool,
    loaded: Option<LoadedDistribution>,
}

impl<'a> DistributionResolver<'a> {
    fn new(
        cache: &'a ObjectCache,
        source: Option<String>,
        expected: Option<String>,
        offline: bool,
    ) -> Self {
        Self {
            cache,
            source,
            expected,
            offline,
            loaded: None,
        }
    }

    fn resolve_batch(
        &mut self,
        local: &LocalResolver,
        requests: &[ResourceRequest],
    ) -> Result<Vec<ResourceResponse>, NativeRunError> {
        let mut responses = Vec::new();
        let mut unresolved = Vec::new();
        for request in requests {
            match request {
                ResourceRequest::File(request) => {
                    if let Some(file) = local.resolve(request) {
                        responses.push(ResourceResponse::File(file));
                    } else {
                        unresolved.push(request.clone());
                    }
                }
                ResourceRequest::Font(request) => {
                    unresolved.push(FileRequest::new(
                        crate::FileRequestKey::new(
                            FileKind::GenericAsset,
                            request.key.logical_name(),
                        )
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                        request.key.logical_name(),
                    ));
                }
            }
        }
        if unresolved.is_empty() {
            return Ok(responses);
        }
        let loaded = self.load()?.clone();
        let mut manifest_requests = Vec::new();
        let mut original_files = BTreeMap::new();
        for request in &unresolved {
            let kind = match request.key().kind() {
                FileKind::TexInput => DistributionFileKind::Tex,
                FileKind::Tfm => DistributionFileKind::Tfm,
                FileKind::GenericAsset => continue,
                _ => {
                    responses.push(ResourceResponse::FileUnavailable(request.key().clone()));
                    continue;
                }
            };
            let key = DistributionFileRequestKey::new(kind, request.key().name())
                .map_err(|error| NativeRunError::Selection(error.to_string()))?;
            original_files.insert(key.manifest_key().to_string(), request.key().clone());
            manifest_requests.push(ManifestRequest::File(key));
        }
        for request in requests {
            if let ResourceRequest::Font(request) = request {
                manifest_requests.push(ManifestRequest::Font(
                    umber_distribution::FontRequestKey::new(request.key.logical_name())
                        .map_err(|error| NativeRunError::Selection(error.to_string()))?,
                ));
            }
        }
        let selection = select(&loaded.manifest, &manifest_requests);
        let mut jobs = selection.jobs;
        let fetch_requests = jobs
            .iter()
            .map(|job| FetchRequest {
                request_key: job.manifest_key.to_string(),
                object: job.object.clone(),
                max_bytes: crate::SessionLimits::default().one_file_bytes as u64,
            })
            .collect::<Vec<_>>();
        let fetched = if self.offline {
            let mut found = Vec::new();
            let mut missing = Vec::new();
            for request in &fetch_requests {
                match self
                    .cache
                    .load_object(&request.object.sha256, request.object.bytes)
                {
                    Ok(Some(bytes)) => found.push((request.request_key.clone(), bytes, true)),
                    Ok(None) => missing.push(request.request_key.clone()),
                    Err(error) => return Err(NativeRunError::Cache(error.to_string())),
                }
            }
            if !missing.is_empty() {
                return Err(NativeRunError::DistributionUnavailable(missing));
            }
            found
        } else if let Some(root) = &loaded.local_root {
            let mut found = Vec::new();
            for request in &fetch_requests {
                let bytes = read(&root.join(&request.object.object))?;
                self.cache
                    .store_object(&request.object.sha256, request.object.bytes, &bytes)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?;
                found.push((request.request_key.clone(), bytes, false));
            }
            found
        } else {
            let client = FetchClient::new(FetchClientConfig::default())
                .map_err(|error| NativeRunError::Fetch(error.to_string()))?;
            client
                .fetch_batch(
                    self.cache,
                    &loaded.manifest.objects_base_url,
                    &fetch_requests,
                )
                .map_err(|error| {
                    NativeRunError::Fetch(
                        error
                            .diagnostics
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                })?
                .into_iter()
                .map(|object| (object.request_key, object.bytes, object.cache_hit))
                .collect()
        };
        if fetched.iter().any(|(_, _, cache_hit)| !cache_hit) {
            eprintln!("umber: acquired {} distribution resource(s)", fetched.len());
        }
        let mut bytes = fetched
            .into_iter()
            .map(|(key, bytes, _)| (key, bytes))
            .collect::<BTreeMap<_, _>>();
        for job in jobs
            .drain(..)
            .filter(|job| job.requirement == umber_distribution::JobRequirement::Required)
        {
            let data = bytes
                .remove(job.manifest_key.as_str())
                .expect("fetched required object");
            match job.request {
                ManifestRequest::File(_) => {
                    let key = original_files
                        .remove(job.manifest_key.as_str())
                        .expect("original file request");
                    let path = loaded
                        .manifest
                        .files
                        .get(job.manifest_key.as_str())
                        .expect("selected file")
                        .virtual_path
                        .clone();
                    responses.push(ResourceResponse::File(ResolvedFile {
                        request: key,
                        expected_digest: Some(FileContentId::for_bytes(&data)),
                        virtual_path: path,
                        bytes: data,
                    }));
                }
                ManifestRequest::Font(key) => {
                    let entry = loaded
                        .manifest
                        .fonts
                        .get(job.manifest_key.as_str())
                        .expect("selected font");
                    let container = match entry.container.as_str() {
                        "woff2" => FontContainer::Woff2,
                        "otf" => FontContainer::OpenType,
                        "ttf" => FontContainer::TrueType,
                        "ttc" => FontContainer::Collection,
                        other => {
                            return Err(NativeRunError::Selection(format!(
                                "unsupported font container {other}"
                            )));
                        }
                    };
                    let request = requests
                        .iter()
                        .find_map(|request| match request {
                            ResourceRequest::Font(request)
                                if request.key.logical_name() == key.logical_name() =>
                            {
                                Some(request.key.clone())
                            }
                            _ => None,
                        })
                        .expect("original font request");
                    responses.push(ResourceResponse::Font(ResolvedFont {
                        request,
                        container,
                        declared_object_sha256: Some(FontObjectIdentity::for_bytes(&data)),
                        declared_program_identity: None,
                        provenance: entry.provenance.clone(),
                        bytes: data,
                    }));
                }
            }
        }
        for miss in selection.misses {
            match miss {
                ManifestMiss::File(key) => {
                    if let Some(original) = original_files.remove(key.manifest_key().as_str()) {
                        responses.push(ResourceResponse::FileUnavailable(original));
                    }
                }
                ManifestMiss::Font(key) => {
                    if let Some(request) = requests.iter().find_map(|request| match request {
                        ResourceRequest::Font(request)
                            if request.key.logical_name() == key.logical_name() =>
                        {
                            Some(request.key.clone())
                        }
                        _ => None,
                    }) {
                        responses.push(ResourceResponse::FontUnavailable(request));
                    }
                }
            }
        }
        Ok(responses)
    }

    fn resolve_format(
        &mut self,
        path: &Path,
        engine: EngineMode,
    ) -> Result<Vec<u8>, NativeRunError> {
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| NativeRunError::Format("format name is not valid UTF-8".into()))?;
        let loaded = self.load()?.clone();
        let entry = loaded
            .manifest
            .formats
            .get(name)
            .ok_or_else(|| NativeRunError::Format(format!("manifest has no format named {name}")))?
            .clone();
        if entry.engine_version != crate::PACKAGE_VERSION {
            return Err(NativeRunError::Format(format!(
                "format {name} requires Umber {}, this runtime is {}",
                entry.engine_version,
                crate::PACKAGE_VERSION
            )));
        }
        if entry.engine != engine.name() && entry.engine != "umber" {
            return Err(NativeRunError::Format(format!(
                "format {name} targets {}, not {}",
                entry.engine,
                engine.name()
            )));
        }
        if let Some(bytes) = self
            .cache
            .load_object(&entry.sha256, entry.bytes)
            .map_err(|error| NativeRunError::Cache(error.to_string()))?
        {
            return Ok(bytes);
        }
        if self.offline {
            return Err(NativeRunError::DistributionUnavailable(vec![format!(
                "format:{name}"
            )]));
        }
        let object = umber_distribution::ObjectEntry {
            object: entry.object,
            sha256: entry.sha256,
            bytes: entry.bytes,
        };
        if let Some(root) = &loaded.local_root {
            let bytes = read(&root.join(&object.object))?;
            self.cache
                .store_object(&object.sha256, object.bytes, &bytes)
                .map_err(|error| NativeRunError::Cache(error.to_string()))?;
            eprintln!("umber: acquired 1 distribution resource(s)");
            return Ok(bytes);
        }
        let request = FetchRequest {
            request_key: format!("format:{name}"),
            object,
            max_bytes: crate::SessionLimits::default().one_file_bytes as u64,
        };
        let object = FetchClient::new(FetchClientConfig::default())
            .map_err(|error| NativeRunError::Fetch(error.to_string()))?
            .fetch_batch(self.cache, &loaded.manifest.objects_base_url, &[request])
            .map_err(|error| {
                NativeRunError::Fetch(
                    error
                        .diagnostics
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?
            .pop()
            .expect("one format result");
        if !object.cache_hit {
            eprintln!("umber: acquired 1 distribution resource(s)");
        }
        Ok(object.bytes)
    }

    fn load(&mut self) -> Result<&LoadedDistribution, NativeRunError> {
        if self.loaded.is_none() {
            let source = self
                .source
                .clone()
                .unwrap_or_else(|| DEFAULT_DISTRIBUTION_URL.to_owned());
            let explicit = self.source.is_some();
            let path = PathBuf::from(&source);
            let local_path = if path.is_dir() {
                path.join("manifest.json")
            } else {
                path.clone()
            };
            let is_local = local_path.exists() || (!source.contains("://") && explicit);
            let expected = self
                .expected
                .clone()
                .or_else(|| (!explicit).then(|| DEFAULT_DISTRIBUTION_SHA256.to_owned()));
            let (manifest_bytes, local_root) = if is_local {
                let bytes = read(&local_path)?;
                if let Some(expected) = &expected {
                    verify_manifest_digest(&bytes, expected)?;
                }
                (bytes, local_path.parent().map(Path::to_owned))
            } else {
                let expected = expected
                    .ok_or_else(|| NativeRunError::DistributionPinRequired(source.clone()))?;
                let bytes = if let Some(bytes) = self
                    .cache
                    .load_manifest(&expected)
                    .map_err(|error| NativeRunError::Cache(error.to_string()))?
                {
                    bytes
                } else {
                    if self.offline {
                        return Err(NativeRunError::DistributionUnavailable(vec![
                            "manifest".into(),
                        ]));
                    }
                    let bytes = fetch_manifest(&source, &expected, Duration::from_secs(30))
                        .map_err(|error| NativeRunError::ManifestFetch(error.to_string()))?;
                    self.cache
                        .store_manifest(&expected, &bytes)
                        .map_err(|error| NativeRunError::Cache(error.to_string()))?;
                    bytes
                };
                (bytes, None)
            };
            let text = std::str::from_utf8(&manifest_bytes)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            let manifest = Manifest::parse(text)
                .map_err(|error| NativeRunError::ManifestParse(error.to_string()))?;
            self.loaded = Some(LoadedDistribution {
                manifest,
                local_root,
            });
        }
        Ok(self.loaded.as_ref().expect("distribution loaded"))
    }
}

fn verify_manifest_digest(bytes: &[u8], expected: &str) -> Result<(), NativeRunError> {
    let actual = hex_digest(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(NativeRunError::ManifestDigestMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(
    clippy::disallowed_methods,
    reason = "this module is the native CLI host I/O boundary"
)]
fn read(path: &Path) -> Result<Vec<u8>, NativeRunError> {
    fs::read(path).map_err(|source| NativeRunError::Io {
        path: path.to_owned(),
        source,
    })
}

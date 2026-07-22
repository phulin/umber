use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use bib_graph::{DataModel, GraphContext, GraphInput, GraphOptions, GraphProcessor, SectionSpec};
use bib_input::{
    BibTexOptions, BibTexSource, ConfigError, ControlError, ControlFile, XmlError, XmlLimits,
    parse_bibtex_bytes, parse_config_bytes, parse_config_with_paths, parse_control_bytes,
    parse_control_with_paths,
};
use bib_model::{
    BibConfigurationBuilder, BibDiagnostic, BibDiagnosticCode, BibSeverity, DataListId,
    DiagnosticBuilder, EntryId, ProcessedBibliographyBuilder, ProcessedSectionBuilder, SectionId,
};
use bib_output::{OutputContext, OutputRouter};
use bib_sort::{DataListBuilder, SortComponent, SortField, SortTemplate};
use bib_unicode::{CompatibilityVersion, UnicodeData};
use umber_vfs::{
    FileContentId, FileKind, FileOrigin, FileRequest, FileRequestBatch, FileRequestKey,
    SnapshotError, VfsSnapshot, VirtualFile, VirtualPath, VirtualRoot,
};

use crate::{BibAttempt, BibFailure, BibFailureKind, BibJob, BibResultBuilder};

mod convert;
use convert::{add_label_sources, convert_entry};

const DEFAULT_LIST: &str = "nty/global//global/global/global";

/// Bounds and deterministic cache policy retained by one bibliography session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BibSessionOptions {
    cache_entries: usize,
    xml_limits: XmlLimits,
    bibtex_options: BibTexOptions,
    maximum_datasources: usize,
    maximum_snapshot_files: usize,
    maximum_generated_files: usize,
    maximum_generated_bytes: usize,
}

impl BibSessionOptions {
    #[must_use]
    pub const fn with_cache_entries(mut self, entries: usize) -> Self {
        self.cache_entries = entries;
        self
    }
    #[must_use]
    pub const fn with_cache_capacity(self, entries: usize) -> Self {
        self.with_cache_entries(entries)
    }
    #[must_use]
    pub const fn without_caches(mut self) -> Self {
        self.cache_entries = 0;
        self
    }
    #[must_use]
    pub const fn with_xml_limits(mut self, limits: XmlLimits) -> Self {
        self.xml_limits = limits;
        self
    }
    #[must_use]
    pub const fn with_bibtex_options(mut self, options: BibTexOptions) -> Self {
        self.bibtex_options = options;
        self
    }
    #[must_use]
    pub const fn cache_entries(self) -> usize {
        self.cache_entries
    }
    #[must_use]
    pub const fn with_maximum_datasources(mut self, maximum: usize) -> Self {
        self.maximum_datasources = maximum;
        self
    }
    #[must_use]
    pub const fn with_maximum_snapshot_files(mut self, maximum: usize) -> Self {
        self.maximum_snapshot_files = maximum;
        self
    }
    #[must_use]
    pub const fn with_maximum_generated_files(mut self, maximum: usize) -> Self {
        self.maximum_generated_files = maximum;
        self
    }
    #[must_use]
    pub const fn with_maximum_generated_bytes(mut self, maximum: usize) -> Self {
        self.maximum_generated_bytes = maximum;
        self
    }
}

impl Default for BibSessionOptions {
    fn default() -> Self {
        Self {
            cache_entries: 32,
            xml_limits: XmlLimits::default(),
            bibtex_options: BibTexOptions::default(),
            maximum_datasources: 1_024,
            maximum_snapshot_files: 16_384,
            maximum_generated_files: 128,
            maximum_generated_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibInitFailure {
    InvalidLimit(&'static str),
}

impl fmt::Display for BibInitFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit(name) => {
                write!(formatter, "bibliography session limit `{name}` is zero")
            }
        }
    }
}

impl std::error::Error for BibInitFailure {}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ParsedKey {
    compatibility: &'static str,
    content: FileContentId,
}

#[derive(Clone, Debug)]
struct ParsedControl {
    value: Arc<ControlFile>,
    inputs: Arc<[VirtualPath]>,
}

/// Resumable, host-neutral bibliography processing over immutable VFS snapshots.
#[derive(Debug)]
pub struct BibSession {
    options: BibSessionOptions,
    unicode: UnicodeData,
    controls: BTreeMap<ParsedKey, Arc<ParsedControl>>,
    control_order: VecDeque<ParsedKey>,
    datasources: BTreeMap<ParsedKey, Arc<BibTexSource>>,
    datasource_order: VecDeque<ParsedKey>,
    previous_need: Option<(BibJob, FileRequestBatch)>,
    accepted_inputs: Vec<crate::BibliographyInput>,
}

impl BibSession {
    pub fn new(options: BibSessionOptions) -> Result<Self, BibInitFailure> {
        for (name, value) in [
            ("maximum_datasources", options.maximum_datasources),
            ("maximum_snapshot_files", options.maximum_snapshot_files),
            ("maximum_generated_files", options.maximum_generated_files),
            ("maximum_generated_bytes", options.maximum_generated_bytes),
        ] {
            if value == 0 {
                return Err(BibInitFailure::InvalidLimit(name));
            }
        }
        Ok(Self {
            options,
            unicode: UnicodeData::pinned(),
            controls: BTreeMap::new(),
            control_order: VecDeque::new(),
            datasources: BTreeMap::new(),
            datasource_order: VecDeque::new(),
            previous_need: None,
            accepted_inputs: Vec::new(),
        })
    }

    #[must_use]
    pub fn cache_len(&self) -> usize {
        self.controls.len() + self.datasources.len()
    }

    pub fn clear_caches(&mut self) {
        self.controls.clear();
        self.control_order.clear();
        self.datasources.clear();
        self.datasource_order.clear();
    }

    pub fn process(&mut self, job: &BibJob, snapshot: &VfsSnapshot) -> BibAttempt {
        let mut inputs = BTreeMap::new();
        match self.process_inner(job, snapshot, &mut inputs) {
            Ok(result) => {
                self.previous_need = None;
                self.accepted_inputs = inputs
                    .into_iter()
                    .map(|(path, kind)| crate::BibliographyInput::new(path, kind))
                    .collect();
                BibAttempt::Complete(result)
            }
            Err(ProcessFailure::Need(batch)) => {
                if self
                    .previous_need
                    .as_ref()
                    .is_some_and(|(previous_job, previous)| {
                        previous_job == job && previous == &batch
                    })
                {
                    self.previous_need = None;
                    return BibAttempt::Failed(failure(
                        BibFailureKind::NoProgress,
                        "RESOURCE_NO_PROGRESS",
                        "bibliography retry supplied none of the required resources",
                    ));
                }
                self.previous_need = Some((job.clone(), batch.clone()));
                BibAttempt::NeedResources(batch)
            }
            Err(ProcessFailure::Terminal(failure)) => {
                self.previous_need = None;
                BibAttempt::Failed(failure)
            }
        }
    }

    fn process_inner(
        &mut self,
        job: &BibJob,
        snapshot: &VfsSnapshot,
        inputs: &mut BTreeMap<VirtualPath, FileKind>,
    ) -> Result<crate::BibResult, ProcessFailure> {
        snapshot
            .list_root(VirtualRoot::Job, self.options.maximum_snapshot_files)
            .map_err(snapshot_failure)?;
        snapshot
            .list_root(
                VirtualRoot::Distribution,
                self.options.maximum_snapshot_files,
            )
            .map_err(snapshot_failure)?;

        let control_key = request_key(FileKind::BibControl, job.control_path().as_str());
        let Some(control_file) = locate(
            snapshot,
            Some(job.control_path()),
            &control_key,
            self.options.maximum_snapshot_files,
            inputs,
        )?
        else {
            return Err(ProcessFailure::Need(batch([request(
                control_key,
                job.control_path().as_str(),
            )])));
        };
        let control = self.control(snapshot, control_file, inputs)?;

        let mut required = Vec::new();
        if let Some(path) = job.options().configuration() {
            let key = request_key(FileKind::BibConfiguration, path.as_str());
            if let Some(file) = locate(
                snapshot,
                Some(path),
                &key,
                self.options.maximum_snapshot_files,
                inputs,
            )? {
                let parsed = if file
                    .bytes()
                    .windows(b"xi:include".len())
                    .any(|window| window == b"xi:include")
                {
                    parse_config_with_paths(snapshot, file.path(), self.options.xml_limits).map(
                        |(configuration, paths)| {
                            for path in paths {
                                inputs.insert(path, FileKind::BibConfiguration);
                            }
                            configuration
                        },
                    )
                } else {
                    parse_config_bytes(file.bytes(), self.options.xml_limits)
                };
                parsed.map_err(config_failure)?;
            } else {
                required.push(request(key, path.as_str()));
            }
        }
        for path in job.options().schemas() {
            let key = request_key(FileKind::XmlSchema, path.as_str());
            if locate(
                snapshot,
                Some(path),
                &key,
                self.options.maximum_snapshot_files,
                inputs,
            )?
            .is_none()
            {
                required.push(request(key, path.as_str()));
            }
        }
        let mut located = BTreeMap::new();
        let mut datasource_count = 0usize;
        for section in &control.sections {
            for name in &section.datasources {
                datasource_count = datasource_count.saturating_add(1);
                if datasource_count > self.options.maximum_datasources {
                    return Err(terminal(
                        BibFailureKind::Limit,
                        "DATASOURCE_LIMIT",
                        "bibliography datasource limit exceeded",
                    ));
                }
                let key = request_key(FileKind::BibData, name);
                if located.contains_key(&key) {
                    continue;
                }
                let local = local_datasource(control_file.path(), name);
                if let Some(file) = locate(
                    snapshot,
                    local.as_ref(),
                    &key,
                    self.options.maximum_snapshot_files,
                    inputs,
                )? {
                    located.insert(key, file);
                } else {
                    required.push(request(key, name));
                }
            }
        }
        if !required.is_empty() {
            return Err(ProcessFailure::Need(batch(required)));
        }

        let configuration =
            BibConfigurationBuilder::new(CompatibilityVersion::BIBER_2_22_BETA).freeze();
        let mut entries = Vec::new();
        let mut seen_entries = BTreeSet::new();
        let mut diagnostics = Vec::new();
        for file in located.values() {
            let source = self.datasource(file);
            for diagnostic in source.diagnostics() {
                diagnostics.push(diagnostic_from_parts(
                    "BIBTEX_INPUT",
                    if matches!(diagnostic.kind, bib_input::BibTexDiagnosticKind::Limit) {
                        BibSeverity::Error
                    } else {
                        BibSeverity::Warning
                    },
                    diagnostic.message.clone(),
                ));
            }
            for raw in source.entries() {
                if seen_entries.insert(raw.key().to_ascii_lowercase()) {
                    entries.push(convert_entry(raw, file.path())?);
                }
            }
        }

        let sections = control
            .sections
            .iter()
            .map(|section| {
                Ok(SectionSpec {
                    id: SectionId::new(section.number),
                    cited: section
                        .citekeys
                        .iter()
                        .filter(|key| key.as_str() != "*")
                        .map(|key| EntryId::new(key.clone()))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| invalid(error.to_string()))?,
                    include_all: section.citekeys.iter().any(|key| key == "*"),
                    min_crossrefs: None,
                })
            })
            .collect::<Result<Vec<_>, ProcessFailure>>()?;
        let graph = GraphProcessor::new(
            GraphContext::new(&configuration, &self.unicode),
            GraphOptions::default(),
        )
        .process(GraphInput {
            entries,
            aliases: Vec::new(),
            sections,
            maps: Vec::new(),
            data_model: DataModel::default(),
        })
        .map_err(|error| terminal(BibFailureKind::Semantic, "GRAPH", format!("{error:?}")))?;
        diagnostics.extend(graph.diagnostics);

        let mut document = ProcessedBibliographyBuilder::new(configuration);
        for section in graph.sections {
            let prepared = section
                .entries
                .into_iter()
                .map(add_label_sources)
                .collect::<Result<Vec<_>, _>>()?;
            let mut initial = ProcessedSectionBuilder::new(section.id);
            for entry in &prepared {
                initial.entry(entry.clone()).map_err(build_failure)?;
            }
            let initial = initial.freeze();
            let template = SortTemplate::new([SortComponent::ascending(SortField::CiteOrder)])
                .map_err(|error| terminal(BibFailureKind::Semantic, "SORT", error.to_string()))?;
            let list = DataListBuilder::new(
                &initial,
                DataListId::new(DEFAULT_LIST).expect("fixed list id is valid"),
                template,
            )
            .build()
            .map_err(|error| terminal(BibFailureKind::Semantic, "SORT", error.to_string()))?;
            let mut builder = ProcessedSectionBuilder::new(section.id);
            for entry in prepared {
                builder.entry(entry).map_err(build_failure)?;
            }
            builder.list(list).map_err(build_failure)?;
            document.section(builder.freeze()).map_err(build_failure)?;
        }
        let document = Arc::new(document.freeze());
        let router = OutputRouter::new(job.options().output_options().clone());
        let mut result = BibResultBuilder::new(Arc::clone(&document));
        let mut generated_bytes = 0usize;
        for request in job.options().outputs() {
            if result.files_len() == self.options.maximum_generated_files {
                return Err(terminal(
                    BibFailureKind::Limit,
                    "GENERATED_FILE_LIMIT",
                    "generated-file count limit exceeded",
                ));
            }
            let file = router
                .serialize(OutputContext::new(&document, &self.unicode), request)
                .map_err(|error| terminal(BibFailureKind::Output, "OUTPUT", error.to_string()))?;
            generated_bytes = generated_bytes
                .checked_add(file.bytes().len())
                .ok_or_else(|| {
                    terminal(
                        BibFailureKind::Limit,
                        "OUTPUT_LIMIT",
                        "generated-byte accounting overflow",
                    )
                })?;
            if generated_bytes > self.options.maximum_generated_bytes {
                return Err(terminal(
                    BibFailureKind::Limit,
                    "OUTPUT_LIMIT",
                    "generated-byte limit exceeded",
                ));
            }
            result.file(file).map_err(build_failure)?;
        }
        for diagnostic in diagnostics {
            result.diagnostic(diagnostic);
        }
        Ok(result.freeze())
    }

    pub fn accepted_inputs(&self) -> &[crate::BibliographyInput] {
        &self.accepted_inputs
    }

    fn control(
        &mut self,
        snapshot: &VfsSnapshot,
        file: &VirtualFile,
        inputs: &mut BTreeMap<VirtualPath, FileKind>,
    ) -> Result<Arc<ControlFile>, ProcessFailure> {
        let key = parsed_key(file.content_id());
        if let Some(parsed) = self.controls.get(&key) {
            for path in parsed.inputs.iter().cloned() {
                inputs.insert(path, FileKind::BibControl);
            }
            return Ok(Arc::clone(&parsed.value));
        }
        let (control, paths) = if file
            .bytes()
            .windows(b"xi:include".len())
            .any(|window| window == b"xi:include")
        {
            let (control, paths) =
                parse_control_with_paths(snapshot, file.path(), self.options.xml_limits)
                    .map_err(control_failure)?;
            (control, paths)
        } else {
            (
                parse_control_bytes(file.bytes(), self.options.xml_limits)
                    .map_err(control_failure)?,
                BTreeSet::from([file.path().clone()]),
            )
        };
        for path in paths.iter().cloned() {
            inputs.insert(path, FileKind::BibControl);
        }
        let control = Arc::new(control);
        let parsed = Arc::new(ParsedControl {
            value: Arc::clone(&control),
            inputs: paths.into_iter().collect(),
        });
        insert_bounded(
            &mut self.controls,
            &mut self.control_order,
            key,
            parsed,
            self.options.cache_entries,
        );
        Ok(control)
    }

    fn datasource(&mut self, file: &VirtualFile) -> Arc<BibTexSource> {
        let key = parsed_key(file.content_id());
        if let Some(source) = self.datasources.get(&key) {
            return Arc::clone(source);
        }
        let source = Arc::new(parse_bibtex_bytes(
            file.bytes(),
            self.options.bibtex_options,
        ));
        insert_bounded(
            &mut self.datasources,
            &mut self.datasource_order,
            key,
            Arc::clone(&source),
            self.options.cache_entries,
        );
        source
    }
}

impl Default for BibSession {
    fn default() -> Self {
        Self::new(BibSessionOptions::default()).expect("default session limits are valid")
    }
}

enum ProcessFailure {
    Need(FileRequestBatch),
    Terminal(BibFailure),
}

fn locate<'a>(
    snapshot: &'a VfsSnapshot,
    exact: Option<&VirtualPath>,
    request: &FileRequestKey,
    limit: usize,
    inputs: &mut BTreeMap<VirtualPath, FileKind>,
) -> Result<Option<&'a VirtualFile>, ProcessFailure> {
    if let Some(path) = exact
        && let Some(file) = snapshot.get(path).map_err(snapshot_failure)?
    {
        inputs.insert(file.path().clone(), request.kind());
        return Ok(Some(file));
    }
    for path in snapshot
        .list_root(VirtualRoot::Distribution, limit)
        .map_err(snapshot_failure)?
    {
        let file = snapshot
            .get(&path)
            .map_err(snapshot_failure)?
            .expect("enumerated path remains visible");
        if matches!(file.origin(), FileOrigin::Resolved(key) if key == request) {
            inputs.insert(file.path().clone(), request.kind());
            return Ok(Some(file));
        }
    }
    Ok(None)
}

fn request_key(kind: FileKind, original: &str) -> FileRequestKey {
    let name = if original.contains("://") {
        encoded_request_name("remote", original)
    } else {
        original
            .strip_prefix("/job/")
            .or_else(|| original.strip_prefix("/texlive/"))
            .unwrap_or(original)
            .to_owned()
    };
    FileRequestKey::new(kind, &name).unwrap_or_else(|_| {
        FileRequestKey::new(kind, &encoded_request_name("logical", original))
            .expect("hex-encoded request names are valid")
    })
}

fn encoded_request_name(prefix: &str, original: &str) -> String {
    let mut encoded = format!("{prefix}/");
    for byte in original.as_bytes() {
        use fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

fn request(key: FileRequestKey, original: &str) -> FileRequest {
    FileRequest::new(key, original)
}

fn batch(required: impl IntoIterator<Item = FileRequest>) -> FileRequestBatch {
    FileRequestBatch::new(required, [])
}

fn local_datasource(control: &VirtualPath, name: &str) -> Option<VirtualPath> {
    if name.contains("://") {
        return None;
    }
    if name.starts_with('/') {
        return VirtualPath::user(name).ok();
    }
    let directory = control.as_str().rsplit_once('/')?.0;
    VirtualPath::user(&format!("{directory}/{name}")).ok()
}

fn parsed_key(content: FileContentId) -> ParsedKey {
    ParsedKey {
        compatibility: CompatibilityVersion::BIBER_2_22_BETA.upstream_commit,
        content,
    }
}

fn insert_bounded<T>(
    values: &mut BTreeMap<ParsedKey, Arc<T>>,
    order: &mut VecDeque<ParsedKey>,
    key: ParsedKey,
    value: Arc<T>,
    capacity: usize,
) {
    if capacity == 0 {
        return;
    }
    while values.len() >= capacity {
        if let Some(evicted) = order.pop_front() {
            values.remove(&evicted);
        }
    }
    order.push_back(key.clone());
    values.insert(key, value);
}

fn control_failure(error: ControlError) -> ProcessFailure {
    match error {
        ControlError::Xml(XmlError::MissingResource(path)) => {
            let key = request_key(FileKind::XmlSchema, path.as_str());
            ProcessFailure::Need(batch([request(key, path.as_str())]))
        }
        ControlError::Xml(XmlError::Limit { .. }) => {
            terminal(BibFailureKind::Limit, "CONTROL_LIMIT", error.to_string())
        }
        ControlError::Version { .. } | ControlError::Namespace { .. } => terminal(
            BibFailureKind::IncompatibleVersion,
            "CONTROL_VERSION",
            error.to_string(),
        ),
        _ => terminal(
            BibFailureKind::MalformedInput,
            "CONTROL_INPUT",
            error.to_string(),
        ),
    }
}

fn config_failure(error: ConfigError) -> ProcessFailure {
    match error {
        ConfigError::Xml(XmlError::MissingResource(path)) => {
            let key = request_key(FileKind::BibConfiguration, path.as_str());
            ProcessFailure::Need(batch([request(key, path.as_str())]))
        }
        ConfigError::Xml(XmlError::Limit { .. }) => {
            terminal(BibFailureKind::Limit, "CONFIG_LIMIT", error.to_string())
        }
        _ => terminal(
            BibFailureKind::MalformedInput,
            "CONFIG_INPUT",
            error.to_string(),
        ),
    }
}

fn snapshot_failure(error: SnapshotError) -> ProcessFailure {
    let kind = if matches!(error, SnapshotError::Stale { .. }) {
        BibFailureKind::ResourceConflict
    } else {
        BibFailureKind::Limit
    };
    terminal(kind, "VFS_SNAPSHOT", error.to_string())
}

fn build_failure(error: impl fmt::Display) -> ProcessFailure {
    terminal(
        BibFailureKind::InternalInvariant,
        "MODEL_BUILD",
        error.to_string(),
    )
}

fn invalid(message: String) -> ProcessFailure {
    terminal(BibFailureKind::MalformedInput, "INVALID_INPUT", message)
}

fn terminal(kind: BibFailureKind, code: &str, message: impl Into<String>) -> ProcessFailure {
    ProcessFailure::Terminal(failure(kind, code, message))
}

fn failure(kind: BibFailureKind, code: &str, message: impl Into<String>) -> BibFailure {
    BibFailure::new(
        kind,
        vec![diagnostic_from_parts(code, BibSeverity::Error, message)],
    )
}

fn diagnostic_from_parts(
    code: &str,
    severity: BibSeverity,
    message: impl Into<String>,
) -> BibDiagnostic {
    DiagnosticBuilder::new(
        BibDiagnosticCode::new(code).expect("fixed diagnostic code is valid"),
        severity,
        message,
    )
    .expect("session diagnostics are nonempty")
    .freeze()
}

#[cfg(test)]
mod tests;

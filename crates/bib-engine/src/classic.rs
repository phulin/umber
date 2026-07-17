//! Resumable classic-BibTeX protocol selection and AUX control discovery.
//!
//! This module intentionally stops at the control/resource boundary. Style
//! compilation and `READ` execution are separate classic-backend phases; they
//! consume the typed closure produced here rather than reparsing host files.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use umber_vfs::{
    FileContentId, FileKind, FileOrigin, FileRequest, FileRequestBatch, FileRequestKey,
    SnapshotError, VfsSnapshot, VirtualFile, VirtualPath, VirtualRoot,
};

use crate::{
    BibJob, BibOptions, BibliographyAttempt, BibliographyDocument, BibliographyFailure,
    BibliographyHistory, BibliographyResult, BibliographyStats, ClassicBibFailure, ClassicBibJob,
    ClassicBibLimits, ClassicBibOptions, ClassicBibliography, ClassicBibliographyStats,
};

/// Explicit protocol selection after a TeX pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyMode {
    Biblatex { control_path: VirtualPath },
    Classic { aux_path: VirtualPath },
    Auto { job_path: VirtualPath },
}

/// Result of a resumable protocol-detection call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BibliographyDetection {
    Selected(crate::BibliographyJob),
    NoBibliography,
    NeedResources(FileRequestBatch),
    Failed(BibliographyFailure),
}

/// Bounds used while auto mode examines the AUX closure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BibliographyDetectorOptions {
    pub classic_limits: ClassicBibLimits,
}

/// Backend-neutral, deterministic bibliography-protocol detector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibliographyDetector {
    options: BibliographyDetectorOptions,
    control: ClassicControlSession,
    previous_need: Option<(BibliographyMode, FileRequestBatch)>,
}

impl BibliographyDetector {
    #[must_use]
    pub fn new(options: BibliographyDetectorOptions) -> Self {
        Self {
            options,
            control: ClassicControlSession::new(),
            previous_need: None,
        }
    }

    #[must_use]
    pub fn detect(
        &mut self,
        mode: &BibliographyMode,
        snapshot: &VfsSnapshot,
    ) -> BibliographyDetection {
        let result = match mode {
            BibliographyMode::Biblatex { control_path } => {
                BibliographyDetection::Selected(crate::BibliographyJob::Biblatex(BibJob::new(
                    control_path.clone(),
                    BibOptions::default(),
                )))
            }
            BibliographyMode::Classic { aux_path } => self.detect_classic(aux_path, snapshot),
            BibliographyMode::Auto { job_path } => self.detect_auto(job_path, snapshot),
        };
        match result {
            BibliographyDetection::NeedResources(batch) => {
                if self
                    .previous_need
                    .as_ref()
                    .is_some_and(|(previous, needs)| previous == mode && needs == &batch)
                {
                    self.previous_need = None;
                    BibliographyDetection::Failed(failure(
                        ClassicBibFailure::NoProgress,
                        "classic bibliography detection made no resource progress",
                    ))
                } else {
                    self.previous_need = Some((mode.clone(), batch.clone()));
                    BibliographyDetection::NeedResources(batch)
                }
            }
            other => {
                self.previous_need = None;
                other
            }
        }
    }

    fn detect_classic(
        &mut self,
        aux_path: &VirtualPath,
        snapshot: &VfsSnapshot,
    ) -> BibliographyDetection {
        let options = ClassicBibOptions::default().with_limits(self.options.classic_limits);
        match self.control.resolve(aux_path, &options, snapshot, true) {
            Ok(control) => match control.completeness() {
                ControlCompleteness::Complete => {
                    BibliographyDetection::Selected(crate::BibliographyJob::Classic(
                        ClassicBibJob::new(aux_path.clone(), ClassicBibOptions::default()),
                    ))
                }
                ControlCompleteness::Incomplete => BibliographyDetection::Failed(failure(
                    ClassicBibFailure::IncompleteControl,
                    "classic bibliography control requires both \\bibstyle and \\bibdata",
                )),
                ControlCompleteness::None => BibliographyDetection::NoBibliography,
            },
            Err(ControlFailure::Need(batch)) => BibliographyDetection::NeedResources(batch),
            Err(ControlFailure::Terminal(kind, message)) => {
                BibliographyDetection::Failed(failure(kind, message))
            }
        }
    }

    fn detect_auto(
        &mut self,
        job_path: &VirtualPath,
        snapshot: &VfsSnapshot,
    ) -> BibliographyDetection {
        let (bcf, aux) = companion_paths(job_path);
        let bcf_exists = match snapshot.get(&bcf) {
            Ok(file) => file.is_some(),
            Err(error) => {
                return BibliographyDetection::Failed(snapshot_failure(error).into_failure());
            }
        };
        let aux_exists = match snapshot.get(&aux) {
            Ok(file) => file.is_some(),
            Err(error) => {
                return BibliographyDetection::Failed(snapshot_failure(error).into_failure());
            }
        };
        if !aux_exists {
            return if bcf_exists {
                BibliographyDetection::Selected(crate::BibliographyJob::Biblatex(BibJob::new(
                    bcf,
                    BibOptions::default(),
                )))
            } else {
                BibliographyDetection::NoBibliography
            };
        }
        let options = ClassicBibOptions::default().with_limits(self.options.classic_limits);
        match self.control.resolve(&aux, &options, snapshot, false) {
            Ok(control) => match control.completeness() {
                ControlCompleteness::Complete if bcf_exists => {
                    BibliographyDetection::Failed(failure(
                        ClassicBibFailure::AmbiguousProtocol,
                        "both a BCF file and complete classic AUX control were generated",
                    ))
                }
                ControlCompleteness::Complete => {
                    BibliographyDetection::Selected(crate::BibliographyJob::Classic(
                        ClassicBibJob::new(aux, ClassicBibOptions::default()),
                    ))
                }
                ControlCompleteness::Incomplete => BibliographyDetection::Failed(failure(
                    ClassicBibFailure::IncompleteControl,
                    "classic bibliography control requires both \\bibstyle and \\bibdata",
                )),
                ControlCompleteness::None if bcf_exists => BibliographyDetection::Selected(
                    crate::BibliographyJob::Biblatex(BibJob::new(bcf, BibOptions::default())),
                ),
                ControlCompleteness::None => BibliographyDetection::NoBibliography,
            },
            Err(ControlFailure::Need(batch)) => BibliographyDetection::NeedResources(batch),
            Err(ControlFailure::Terminal(kind, message)) => {
                BibliographyDetection::Failed(failure(kind, message))
            }
        }
    }
}

impl Default for BibliographyDetector {
    fn default() -> Self {
        Self::new(BibliographyDetectorOptions::default())
    }
}

/// Ordered classic control data extracted from the complete AUX closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicControl {
    aux_files: Arc<[VirtualPath]>,
    databases: Arc<[String]>,
    style: Option<String>,
    citations: Arc<[String]>,
}

impl ClassicControl {
    pub fn aux_files(&self) -> impl ExactSizeIterator<Item = &VirtualPath> {
        self.aux_files.iter()
    }

    pub fn databases(&self) -> impl ExactSizeIterator<Item = &str> {
        self.databases.iter().map(String::as_str)
    }

    #[must_use]
    pub fn style(&self) -> Option<&str> {
        self.style.as_deref()
    }

    pub fn citations(&self) -> impl ExactSizeIterator<Item = &str> {
        self.citations.iter().map(String::as_str)
    }

    fn completeness(&self) -> ControlCompleteness {
        match (self.style.is_some(), self.databases.is_empty()) {
            (true, false) => ControlCompleteness::Complete,
            (false, true) => ControlCompleteness::None,
            _ => ControlCompleteness::Incomplete,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_read_test(citations: &[&str]) -> Self {
        Self {
            aux_files: Arc::new([]),
            databases: Arc::new([]),
            style: None,
            citations: citations
                .iter()
                .map(|citation| (*citation).to_owned())
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlCompleteness {
    None,
    Incomplete,
    Complete,
}

/// Shared classic session used by explicit execution and auto detection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ClassicControlSession {
    parsed: BTreeMap<FileContentId, Arc<ParsedAux>>,
    order: VecDeque<FileContentId>,
    previous_need: Option<(ClassicBibJob, FileRequestBatch)>,
}

impl ClassicControlSession {
    pub const fn new() -> Self {
        Self {
            parsed: BTreeMap::new(),
            order: VecDeque::new(),
            previous_need: None,
        }
    }

    pub fn process(&mut self, job: &ClassicBibJob, snapshot: &VfsSnapshot) -> BibliographyAttempt {
        match self.resolve(job.aux_path(), job.options(), snapshot, true) {
            Ok(control) => {
                self.previous_need = None;
                match control.completeness() {
                    ControlCompleteness::Complete => {
                        let required = resource_requests(&control);
                        let missing = match missing_resources(snapshot, required) {
                            Ok(missing) => missing,
                            Err(error) => {
                                self.previous_need = None;
                                return BibliographyAttempt::Failed(error.into_failure());
                            }
                        };
                        if let Some(missing) = missing {
                            return self.need(job, missing);
                        }
                        BibliographyAttempt::Finished(
                            BibliographyResult::new(
                                BibliographyHistory::Spotless,
                                BibliographyDocument::Classic(Arc::new(
                                    ClassicBibliography::from_control(&control),
                                )),
                                [],
                                [],
                                [],
                                BibliographyStats::Classic(ClassicBibliographyStats::default()),
                            )
                            .expect("classic control results have no generated artifacts"),
                        )
                    }
                    ControlCompleteness::None => BibliographyAttempt::Finished(
                        BibliographyResult::new(
                            BibliographyHistory::Spotless,
                            BibliographyDocument::Classic(Arc::new(ClassicBibliography::empty())),
                            [],
                            [],
                            [],
                            BibliographyStats::Classic(ClassicBibliographyStats::default()),
                        )
                        .expect("empty classic control result is valid"),
                    ),
                    ControlCompleteness::Incomplete => BibliographyAttempt::Failed(failure(
                        ClassicBibFailure::IncompleteControl,
                        "classic bibliography control requires both \\bibstyle and \\bibdata",
                    )),
                }
            }
            Err(ControlFailure::Need(batch)) => self.need(job, batch),
            Err(ControlFailure::Terminal(kind, message)) => {
                self.previous_need = None;
                BibliographyAttempt::Failed(failure(kind, message))
            }
        }
    }

    fn need(&mut self, job: &ClassicBibJob, batch: FileRequestBatch) -> BibliographyAttempt {
        if self
            .previous_need
            .as_ref()
            .is_some_and(|(previous, needs)| previous == job && needs == &batch)
        {
            self.previous_need = None;
            BibliographyAttempt::Failed(failure(
                ClassicBibFailure::NoProgress,
                "classic bibliography retry supplied none of the required resources",
            ))
        } else {
            self.previous_need = Some((job.clone(), batch.clone()));
            BibliographyAttempt::NeedResources(batch)
        }
    }

    fn resolve(
        &mut self,
        root: &VirtualPath,
        options: &ClassicBibOptions,
        snapshot: &VfsSnapshot,
        required_root: bool,
    ) -> Result<ClassicControl, ControlFailure> {
        validate_limits(options.limits())?;
        let root_key = request_key(FileKind::BibAux, root.as_str());
        let root_file = locate(snapshot, Some(root), &root_key)?;
        let Some(root_file) = root_file else {
            return if required_root {
                Err(ControlFailure::Need(FileRequestBatch::new(
                    [FileRequest::new(root_key, root.as_str())],
                    [],
                )))
            } else {
                Ok(empty_control())
            };
        };

        let mut queue = VecDeque::from([(root_file.path().clone(), root_file, 0usize)]);
        let mut seen = BTreeSet::new();
        let mut aux_files = Vec::new();
        let mut databases = Vec::new();
        let mut citations = Vec::new();
        let mut style = None;
        let mut bytes = 0usize;
        let mut missing = Vec::new();
        while let Some((path, file, depth)) = queue.pop_front() {
            if !seen.insert(path.clone()) {
                continue;
            }
            if seen.len() > options.limits().aux_files {
                return Err(limit("classic AUX file limit exceeded"));
            }
            if depth > options.limits().aux_depth {
                return Err(limit("classic AUX include depth limit exceeded"));
            }
            bytes = bytes.saturating_add(file.bytes().len());
            if bytes > options.limits().aux_bytes {
                return Err(limit("classic AUX byte limit exceeded"));
            }
            let parsed = self.parse(file, options)?;
            aux_files.push(path);
            citations.extend(parsed.citations.iter().cloned());
            databases.extend(parsed.databases.iter().cloned());
            if let Some(found) = &parsed.style
                && style.replace(found.clone()).is_some()
            {
                return Err(ControlFailure::Terminal(
                    ClassicBibFailure::MalformedInput,
                    "classic AUX closure contains multiple \\bibstyle commands".to_owned(),
                ));
            }
            for include in &parsed.includes {
                let key = request_key(FileKind::BibAux, include);
                let exact = VirtualPath::user(include).ok();
                match locate(snapshot, exact.as_ref(), &key)? {
                    Some(child) => queue.push_back((child.path().clone(), child, depth + 1)),
                    None => missing.push(FileRequest::new(key, include)),
                }
            }
        }
        if !missing.is_empty() {
            return Err(ControlFailure::Need(FileRequestBatch::new(missing, [])));
        }
        Ok(ClassicControl {
            aux_files: aux_files.into(),
            databases: databases.into(),
            style,
            citations: citations.into(),
        })
    }

    fn parse(
        &mut self,
        file: &VirtualFile,
        options: &ClassicBibOptions,
    ) -> Result<Arc<ParsedAux>, ControlFailure> {
        let key = file.content_id();
        if let Some(parsed) = self.parsed.get(&key) {
            return Ok(Arc::clone(parsed));
        }
        let parsed = Arc::new(parse_aux(file.bytes())?);
        if options.cache_entries() != 0 {
            while self.parsed.len() >= options.cache_entries() {
                if let Some(oldest) = self.order.pop_front() {
                    self.parsed.remove(&oldest);
                }
            }
            self.order.push_back(key);
            self.parsed.insert(key, Arc::clone(&parsed));
        }
        Ok(parsed)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ParsedAux {
    citations: Vec<String>,
    databases: Vec<String>,
    style: Option<String>,
    includes: Vec<String>,
}

fn parse_aux(bytes: &[u8]) -> Result<ParsedAux, ControlFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ControlFailure::Terminal(
            ClassicBibFailure::MalformedInput,
            "classic AUX is not valid UTF-8".to_owned(),
        )
    })?;
    let mut parsed = ParsedAux::default();
    for line in text.lines() {
        let line = line.trim_start();
        for (command, values) in [
            ("\\citation", &mut parsed.citations),
            ("\\bibdata", &mut parsed.databases),
            ("\\@input", &mut parsed.includes),
        ] {
            if let Some(value) = command_value(line, command)? {
                values.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                );
                continue;
            }
        }
        if let Some(value) = command_value(line, "\\bibstyle")?
            && parsed.style.replace(value).is_some()
        {
            return Err(ControlFailure::Terminal(
                ClassicBibFailure::MalformedInput,
                "classic AUX file contains multiple \\bibstyle commands".to_owned(),
            ));
        }
    }
    Ok(parsed)
}

fn command_value(line: &str, command: &str) -> Result<Option<String>, ControlFailure> {
    let Some(rest) = line.strip_prefix(command) else {
        return Ok(None);
    };
    let rest = rest.trim_start();
    if !rest.starts_with('{') {
        return Ok(None);
    }
    let Some(end) = rest.find('}') else {
        return Err(ControlFailure::Terminal(
            ClassicBibFailure::MalformedInput,
            format!("unterminated {command} command in classic AUX"),
        ));
    };
    Ok(Some(rest[1..end].to_owned()))
}

fn resource_requests(control: &ClassicControl) -> Vec<FileRequest> {
    let mut requests = control
        .databases()
        .map(|name| {
            FileRequest::new(
                request_key(FileKind::ClassicBibData, &default_extension(name, "bib")),
                name,
            )
        })
        .collect::<Vec<_>>();
    if let Some(style) = control.style() {
        requests.push(FileRequest::new(
            request_key(FileKind::BibStyle, &default_extension(style, "bst")),
            style,
        ));
    }
    requests
}

fn missing_resources(
    snapshot: &VfsSnapshot,
    requests: Vec<FileRequest>,
) -> Result<Option<FileRequestBatch>, ControlFailure> {
    let mut missing = Vec::new();
    for request in requests {
        if locate(snapshot, None, request.key())?.is_none() {
            missing.push(request);
        }
    }
    Ok((!missing.is_empty()).then(|| FileRequestBatch::new(missing, [])))
}

fn locate<'a>(
    snapshot: &'a VfsSnapshot,
    exact: Option<&VirtualPath>,
    request: &FileRequestKey,
) -> Result<Option<&'a VirtualFile>, ControlFailure> {
    if let Some(path) = exact
        && let Some(file) = snapshot.get(path).map_err(snapshot_failure)?
    {
        return Ok(Some(file));
    }
    for path in snapshot
        .list_root(VirtualRoot::Distribution, 16_384)
        .map_err(snapshot_failure)?
    {
        let file = snapshot
            .get(&path)
            .map_err(snapshot_failure)?
            .expect("visible file");
        if matches!(file.origin(), FileOrigin::Resolved(key) if key == request) {
            return Ok(Some(file));
        }
    }
    Ok(None)
}

fn companion_paths(path: &VirtualPath) -> (VirtualPath, VirtualPath) {
    let raw = path.as_str().strip_prefix("/job/").expect("user path");
    let stem = raw.rsplit_once('.').map_or(raw, |(stem, _)| stem);
    (
        VirtualPath::user(&format!("{stem}.bcf")).expect("companion path"),
        VirtualPath::user(&format!("{stem}.aux")).expect("companion path"),
    )
}

fn default_extension(name: &str, extension: &str) -> String {
    if name
        .rsplit('/')
        .next()
        .is_some_and(|part| part.contains('.'))
    {
        name.to_owned()
    } else {
        format!("{name}.{extension}")
    }
}

fn request_key(kind: FileKind, value: &str) -> FileRequestKey {
    let name = value.strip_prefix("/job/").unwrap_or(value);
    FileRequestKey::new(kind, name).unwrap_or_else(|_| {
        FileRequestKey::new(kind, &format!("opaque/{:x}", fxhash(value)))
            .expect("opaque resource name is valid")
    })
}

fn fxhash(value: &str) -> u64 {
    // This fallback is only for a syntactically invalid external name. It is
    // deterministic and does not participate in semantic file identities.
    value.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        hash.wrapping_mul(0x100_0000_01b3) ^ u64::from(byte)
    })
}

fn empty_control() -> ClassicControl {
    ClassicControl {
        aux_files: Arc::new([]),
        databases: Arc::new([]),
        style: None,
        citations: Arc::new([]),
    }
}

fn validate_limits(limits: ClassicBibLimits) -> Result<(), ControlFailure> {
    if limits.aux_bytes == 0 || limits.aux_files == 0 || limits.aux_depth == 0 {
        return Err(ControlFailure::Terminal(
            ClassicBibFailure::Limit,
            "classic AUX limits must be nonzero".to_owned(),
        ));
    }
    Ok(())
}

enum ControlFailure {
    Need(FileRequestBatch),
    Terminal(ClassicBibFailure, String),
}

impl ControlFailure {
    fn into_failure(self) -> BibliographyFailure {
        match self {
            Self::Need(_) => BibliographyFailure::Classic(ClassicBibFailure::InternalInvariant),
            Self::Terminal(kind, _) => BibliographyFailure::Classic(kind),
        }
    }
}

fn snapshot_failure(error: SnapshotError) -> ControlFailure {
    let kind = if matches!(error, SnapshotError::Stale { .. }) {
        ClassicBibFailure::ResourceConflict
    } else {
        ClassicBibFailure::Limit
    };
    ControlFailure::Terminal(kind, error.to_string())
}

fn limit(message: impl Into<String>) -> ControlFailure {
    ControlFailure::Terminal(ClassicBibFailure::Limit, message.into())
}

fn failure(kind: ClassicBibFailure, message: impl Into<String>) -> BibliographyFailure {
    let _ = message.into();
    BibliographyFailure::Classic(kind)
}

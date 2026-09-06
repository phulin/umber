//! Shared predictive resource policy used by native and browser adapters.

use std::collections::BTreeSet;

use umber_distribution::{
    FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey, LookupManifest,
    LookupOutcome, LookupRecord, LookupRole, NegativeScope, PrefetchBudget, PrefetchClass,
    PrefetchEscalation, PrefetchFileKey, PrefetchIdentity, PrefetchPolicy, PrefetchRegionKey,
    Readiness, ResolvedIdentity,
};
use umber_hash::{AHash64, HashDomain};

use crate::{FileKind, FileRequest, FileRequestKey, ResourceRequest};

#[cfg(test)]
mod tests;

pub const PREFETCH_POLICY_VERSION: &str = "literal-groups-v1";

/// Maximum number of planner decisions emitted by the opt-in resource-boundary
/// flight recorder.
pub const MAX_PREFETCH_DIAGNOSTIC_DECISIONS: usize = 64;

/// Metrics deliberately describe work avoided and bytes admitted, rather than
/// only reporting cache hits (a warm persistent cache still needs admission).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PrefetchMetrics {
    pub startup_candidates: u64,
    pub literal_hints: u64,
    pub group_candidates: u64,
    pub prefetch_bytes: u64,
    pub demand_bytes: u64,
    pub unused_prefetch_bytes: u64,
    pub ready_resources: u64,
    pub exists_not_ready_resources: u64,
    pub absent_resources: u64,
}

/// One cold planner-boundary observation. The native host formats these
/// directly; the shared planner only supplies a small disposition enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrefetchDiagnosticDisposition {
    /// A literal/prior hint was discovered and selected as its typed key.
    HintSelected,
    CandidateSelected,
    CatalogPresent,
    CatalogAbsent,
    ContentAdmitted,
    AlreadyResident,
    RuntimeTextScanned,
    RuntimeTextScanSkipped,
    BudgetSkipped,
}

pub type PrefetchDiagnosticSink = fn(u64, &PrefetchFileKey, PrefetchDiagnosticDisposition);

#[derive(Clone, Copy, Debug)]
struct PrefetchDiagnostics {
    sink: PrefetchDiagnosticSink,
    emitted: u64,
    dropped: u64,
}

/// Host-neutral predictor.  It never reads bytes or performs I/O; adapters
/// feed it accepted lookup outcomes and use its startup set to schedule host
/// acquisition before the first engine attempt.
#[derive(Clone, Debug)]
pub struct PrefetchPlanner {
    identity: PrefetchIdentity,
    budget: PrefetchBudget,
    accepted: LookupManifest,
    prior: Option<LookupManifest>,
    seen_startup: BTreeSet<String>,
    policy: PrefetchPolicy,
    metrics: PrefetchMetrics,
    diagnostics: Option<PrefetchDiagnostics>,
}

impl PrefetchPlanner {
    pub fn new(identity: PrefetchIdentity, budget: PrefetchBudget) -> Self {
        Self {
            accepted: LookupManifest::new(identity.clone()),
            prior: None,
            identity,
            budget,
            seen_startup: BTreeSet::new(),
            policy: PrefetchPolicy::new(budget),
            metrics: PrefetchMetrics::default(),
            diagnostics: None,
        }
    }

    pub fn with_prior(
        identity: PrefetchIdentity,
        budget: PrefetchBudget,
        prior: Option<LookupManifest>,
    ) -> Self {
        let mut planner = Self::new(identity.clone(), budget);
        if let Some(prior) = prior.filter(|manifest| manifest.identity() == &identity) {
            planner.metrics.startup_candidates = prior.resolved_records().count() as u64;
            planner.prior = Some(prior);
        }
        planner
    }

    #[must_use]
    pub const fn identity(&self) -> &PrefetchIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn budget(&self) -> PrefetchBudget {
        self.budget
    }

    /// Enables bounded planner counters without selecting an output sink.
    /// This is used by focused policy tests; native callers should use
    /// [`Self::enable_diagnostics_with_sink`].
    pub fn enable_diagnostics(&mut self) {
        self.enable_diagnostics_with_sink(|_, _, _| {});
    }

    /// Enables bounded planner observations. The sink is called only at cold
    /// resource/planner seams and is responsible for native formatting.
    pub fn enable_diagnostics_with_sink(&mut self, sink: PrefetchDiagnosticSink) {
        self.diagnostics = Some(PrefetchDiagnostics {
            sink,
            emitted: 0,
            dropped: 0,
        });
    }

    #[must_use]
    pub const fn diagnostics_enabled(&self) -> bool {
        self.diagnostics.is_some()
    }

    #[must_use]
    pub const fn diagnostic_counts(&self) -> Option<(u64, u64)> {
        match self.diagnostics {
            Some(diagnostics) => Some((diagnostics.emitted, diagnostics.dropped)),
            None => None,
        }
    }

    pub fn select_prefetch_group(
        &mut self,
        required: impl IntoIterator<Item = umber_distribution::PrefetchCandidate>,
        candidates: impl IntoIterator<Item = umber_distribution::PrefetchCandidate>,
    ) -> umber_distribution::PrefetchSelection {
        if self.diagnostics.is_none() {
            return self.policy.select_prefetch_group(required, candidates);
        }
        let required = required.into_iter().collect::<Vec<_>>();
        let candidates = candidates.into_iter().collect::<Vec<_>>();
        let selection = self
            .policy
            .select_prefetch_group(required.clone(), candidates.clone());
        let selected = selection
            .required
            .iter()
            .chain(&selection.hints)
            .map(candidate_identity)
            .collect::<BTreeSet<_>>();
        for candidate in required.iter().chain(&candidates) {
            if selected.contains(&candidate_identity(candidate)) {
                if !self.startup_seen_candidate(candidate) {
                    self.note_selected_candidate(candidate);
                }
            } else if !self.policy.candidate_is_known(candidate) {
                self.note_budget_skipped_candidate(candidate);
            }
        }
        selection
    }

    #[must_use]
    pub const fn metrics(&self) -> PrefetchMetrics {
        self.metrics
    }

    /// Starts the one optional reservation phase owned by the native compile
    /// session. The phase spans its initial batch and all admission-driven
    /// closure waves; only a new actual engine demand should call this.
    pub fn begin_phase(&mut self) {
        self.policy.begin_phase();
    }

    /// Combines prior accepted lookups and literal source hints.  Requests are
    /// deduplicated by typed key and remain in stable source/history order.
    pub fn startup_hints(&mut self, source: &str) -> Vec<ResourceRequest> {
        self.enqueue_startup_requests(source);
        let requests = self
            .policy
            .drain_prefetch_wave(self.budget.max_files)
            .into_iter()
            .filter_map(|request| resource_request(&request))
            .filter(|request| self.seen_startup.insert(request_identity(request)))
            .collect::<Vec<_>>();
        if self.diagnostics.is_some() {
            for request in &requests {
                self.note_hint_selected(request);
            }
        }
        requests
    }

    /// Starts scheduling for a new source/context without carrying replay
    /// counters, admitted-byte state, or queued aliases across runs. The
    /// caller drains the startup queue at its next preflight boundary.
    pub fn reset_for_context(&mut self, source: &str) {
        self.accepted = LookupManifest::new(self.identity.clone());
        self.seen_startup.clear();
        self.policy = PrefetchPolicy::new(self.budget);
        if let Some(diagnostics) = self.diagnostics.as_mut() {
            diagnostics.emitted = 0;
            diagnostics.dropped = 0;
        }
        self.metrics = PrefetchMetrics {
            startup_candidates: self
                .prior_manifest()
                .map_or(0, |manifest| manifest.resolved_records().count() as u64),
            ..PrefetchMetrics::default()
        };
        self.enqueue_startup_requests(source);
    }

    fn enqueue_startup_requests(&mut self, source: &str) {
        let prior_requests = self
            .prior_manifest()
            .into_iter()
            .flat_map(|prior| prior.resolved_records())
            .filter_map(distribution_record_request)
            .collect::<Vec<_>>();
        for request in prior_requests {
            if let Some(policy_request) = policy_request(&request, "accepted", false) {
                self.policy.enqueue_with_priority(policy_request, 3);
            }
        }
        let queued_before = self.policy.metrics().queued_requests;
        self.policy.enqueue_literal_hints_with_priority(source, 2);
        self.metrics.literal_hints = self.metrics.literal_hints.saturating_add(
            self.policy
                .metrics()
                .queued_requests
                .saturating_sub(queued_before),
        );
    }

    /// Feeds the policy only after the caller has successfully admitted the
    /// verified payload to the engine VFS. The returned values are bounded
    /// lexical follow-ups, never semantic resource answers.
    pub fn admit_file(&mut self, request: &FileRequest, bytes: &[u8]) {
        self.admit_file_with_metadata(request, "", bytes, std::iter::empty());
    }

    pub fn admit_file_with_metadata(
        &mut self,
        request: &FileRequest,
        _virtual_path: &str,
        bytes: &[u8],
        dependencies: impl IntoIterator<Item = FileRequest>,
    ) {
        let Ok(key) =
            DistributionFileRequestKey::from_manifest_key(&distribution_key(request.key()))
        else {
            return;
        };
        let dependencies = dependencies.into_iter().filter_map(|dependency| {
            policy_request(
                &ResourceRequest::File(dependency),
                "distribution-dependency",
                false,
            )
        });
        let class = if request.key().kind() == FileKind::Image {
            PrefetchClass::Image
        } else {
            PrefetchClass::for_key(key.manifest_key().as_str())
        };
        let diagnostic_key = self
            .diagnostics
            .is_some()
            .then(|| prefetch_file_key(request.key()))
            .flatten();
        let was_admitted = diagnostic_key
            .as_ref()
            .is_some_and(|key| self.policy.file_key_is_admitted(key));
        let runtime_scan_allowed = diagnostic_key.as_ref().is_some_and(|key| {
            class == PrefetchClass::SmallRuntime
                && self.policy.runtime_scan_is_allowed(key, bytes.len())
        });
        let Some(parent) =
            policy_request(&ResourceRequest::File(request.clone()), "admission", false)
        else {
            return;
        };
        self.policy
            .admitted_request_with_class(&parent, class, bytes, dependencies);
        if let Some(key) = diagnostic_key {
            if was_admitted {
                self.note_diagnostic(&key, PrefetchDiagnosticDisposition::AlreadyResident);
            } else {
                self.note_diagnostic(&key, PrefetchDiagnosticDisposition::ContentAdmitted);
                if class == PrefetchClass::SmallRuntime {
                    let disposition = if runtime_scan_allowed {
                        PrefetchDiagnosticDisposition::RuntimeTextScanned
                    } else {
                        PrefetchDiagnosticDisposition::RuntimeTextScanSkipped
                    };
                    self.note_diagnostic(&key, disposition);
                }
            }
        }
    }

    pub fn drain_followups(&mut self) -> Vec<ResourceRequest> {
        let requests = self
            .policy
            .drain_prefetch_wave(self.budget.max_files)
            .into_iter()
            .filter_map(|request| resource_request(&request))
            .collect::<Vec<_>>();
        if self.diagnostics.is_some() {
            for request in &requests {
                self.note_hint_selected(request);
            }
        }
        requests
    }

    /// Keeps an optional hint pending when the current phase cannot reserve
    /// it. It is released at the next phase boundary and is never reported as
    /// a semantic unavailable binding.
    pub fn defer_prefetch(&mut self, request: &ResourceRequest) {
        let Some(policy_request) = policy_request(request, "deferred", false) else {
            return;
        };
        self.policy.defer(policy_request);
    }

    /// Records whether a planner candidate was present in the authenticated
    /// catalogue.  This is a diagnostic observation only; a negative result
    /// never changes the ordinary resolver outcome.
    pub fn note_catalog_result(&mut self, request: &FileRequest, present: bool) {
        if self.diagnostics.is_none() {
            return;
        }
        let Some(key) = prefetch_file_key(request.key()) else {
            return;
        };
        self.note_diagnostic(
            &key,
            if present {
                PrefetchDiagnosticDisposition::CatalogPresent
            } else {
                PrefetchDiagnosticDisposition::CatalogAbsent
            },
        );
    }

    fn note_hint_selected(&mut self, request: &ResourceRequest) {
        let ResourceRequest::File(request) = request else {
            return;
        };
        let Some(key) = prefetch_file_key(request.key()) else {
            return;
        };
        self.note_diagnostic(&key, PrefetchDiagnosticDisposition::HintSelected);
    }

    fn note_selected_candidate(&mut self, candidate: &umber_distribution::PrefetchCandidate) {
        let Some(key) = candidate.file_key.as_ref() else {
            return;
        };
        self.note_diagnostic(key, PrefetchDiagnosticDisposition::CandidateSelected);
    }

    fn startup_seen_candidate(&self, candidate: &umber_distribution::PrefetchCandidate) -> bool {
        candidate.file_key.as_ref().is_some_and(|key| {
            self.seen_startup
                .contains(&format!("file:{}:{}", key.kind, key.normalized_name))
        })
    }

    fn note_budget_skipped_candidate(&mut self, candidate: &umber_distribution::PrefetchCandidate) {
        let Some(key) = candidate.file_key.as_ref() else {
            return;
        };
        self.note_diagnostic(key, PrefetchDiagnosticDisposition::BudgetSkipped);
    }

    fn note_diagnostic(
        &mut self,
        key: &PrefetchFileKey,
        disposition: PrefetchDiagnosticDisposition,
    ) {
        let Some(diagnostics) = self.diagnostics.as_mut() else {
            return;
        };
        if diagnostics.emitted >= MAX_PREFETCH_DIAGNOSTIC_DECISIONS as u64 {
            diagnostics.dropped = diagnostics.dropped.saturating_add(1);
            return;
        }
        diagnostics.emitted += 1;
        (diagnostics.sink)(diagnostics.emitted, key, disposition);
    }

    #[must_use]
    pub fn note_replay(
        &mut self,
        region: &str,
        request: &FileRequestKey,
        discarded_work: u64,
    ) -> Option<PrefetchEscalation> {
        let key = prefetch_file_key(request)?;
        let region = PrefetchRegionKey::new(region.to_owned())?;
        self.policy
            .note_replay_for_file_key(region, &key, discarded_work)
    }

    pub fn enqueue_escalation(&mut self, requests: impl IntoIterator<Item = FileRequest>) {
        self.enqueue_escalation_with_priority(requests, 4);
    }

    pub fn enqueue_escalation_with_priority(
        &mut self,
        requests: impl IntoIterator<Item = FileRequest>,
        priority: u64,
    ) {
        for request in requests {
            let resource = ResourceRequest::File(request);
            if let Some(policy_request) = policy_request(&resource, "replay-escalation", false) {
                self.policy.enqueue_with_priority(policy_request, priority);
            }
        }
    }

    pub fn escalation_dependencies(&self, request: &FileRequestKey, tier: u8) -> Vec<FileRequest> {
        let Some(file_key) = prefetch_file_key(request) else {
            return Vec::new();
        };
        self.policy
            .dependency_closure_for_file_key(&file_key, tier)
            .into_iter()
            .filter_map(|request| match resource_request(&request) {
                Some(ResourceRequest::File(request)) => Some(request),
                Some(ResourceRequest::Font(_) | ResourceRequest::PkFont(_)) | None => None,
            })
            .collect()
    }

    /// Records a successful lookup for publication after the surrounding run
    /// is accepted.  Errors are intentionally represented by no call.
    pub fn record_file(
        &mut self,
        request: &FileRequest,
        role: LookupRole,
        search_context: &str,
        virtual_path: Option<String>,
        object: String,
        bytes: &[u8],
    ) {
        let manifest_key = distribution_key(request.key());
        let digest = AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex();
        let Ok(identity) = ResolvedIdentity::new(
            manifest_key.clone(),
            virtual_path,
            object,
            digest,
            bytes.len() as u64,
        ) else {
            return;
        };
        let kind = request.key().kind().wire_name();
        if let Ok(record) = LookupRecord::new(
            request.original_name(),
            manifest_key,
            kind,
            search_context,
            role,
            LookupOutcome::Resolved(identity),
        ) {
            let _ = self.accepted.record(record);
        }
    }

    /// Records an authoritative negative with its namespace/version scope.
    pub fn record_absent(
        &mut self,
        request: &FileRequest,
        role: LookupRole,
        search_context: &str,
        scope: NegativeScope,
    ) {
        let manifest_key = distribution_key(request.key());
        if let Ok(record) = LookupRecord::new(
            request.original_name(),
            manifest_key,
            request.key().kind().wire_name(),
            search_context,
            role,
            LookupOutcome::Absent(scope),
        ) {
            let _ = self.accepted.record(record);
        }
    }

    pub fn note_readiness(&mut self, readiness: Readiness) {
        match readiness {
            Readiness::Ready => self.metrics.ready_resources += 1,
            Readiness::ExistsNotReady => self.metrics.exists_not_ready_resources += 1,
            Readiness::Absent => self.metrics.absent_resources += 1,
        }
    }

    pub fn note_demand_bytes(&mut self, bytes: u64) {
        self.metrics.demand_bytes = self.metrics.demand_bytes.saturating_add(bytes);
    }

    pub fn note_prefetch_bytes(&mut self, bytes: u64) {
        self.metrics.prefetch_bytes = self.metrics.prefetch_bytes.saturating_add(bytes);
    }

    pub fn note_unused_prefetch_bytes(&mut self, bytes: u64) {
        self.metrics.unused_prefetch_bytes =
            self.metrics.unused_prefetch_bytes.saturating_add(bytes);
    }

    /// Takes the accepted-run manifest.  Call only after the engine and its
    /// generated transaction have committed successfully.
    pub fn into_manifest(self) -> LookupManifest {
        self.accepted
    }

    fn prior_manifest(&self) -> Option<&LookupManifest> {
        // The planner stores only the current accepted manifest so it cannot
        // accidentally publish unobserved predictions.  Adapters may retain a
        // decoded prior manifest and feed its records through `startup_hints`
        // by constructing a planner with the optional prior below.
        self.prior.as_ref()
    }
}

fn request_identity(request: &ResourceRequest) -> String {
    match request {
        ResourceRequest::File(request) => format!(
            "file:{}:{}",
            request.key().kind().wire_name(),
            request.key().name()
        ),
        ResourceRequest::Font(request) => format!("font:{:?}", request.key),
        ResourceRequest::PkFont(request) => format!("pk-font:{:?}", request),
    }
}

fn candidate_identity(candidate: &umber_distribution::PrefetchCandidate) -> String {
    candidate.file_key.as_ref().map_or_else(
        || format!("transport:{}", candidate.key),
        PrefetchFileKey::identity,
    )
}

fn policy_request(
    request: &ResourceRequest,
    search_context: &str,
    required: bool,
) -> Option<umber_distribution::PrefetchRequest> {
    let ResourceRequest::File(request) = request else {
        return None;
    };
    let key = distribution_key(request.key());
    let mut policy_request = umber_distribution::PrefetchRequest::for_file_key(
        prefetch_file_key(request.key())?,
        key,
        request.original_name(),
        search_context,
        required,
    );
    if request.key().kind() == FileKind::Image {
        policy_request = policy_request.with_class(PrefetchClass::Image);
    }
    Some(policy_request)
}

fn resource_request(request: &umber_distribution::PrefetchRequest) -> Option<ResourceRequest> {
    let file_key = request.file_key.as_ref()?;
    let domain = crate::ResourceDomain::from_wire_name(&file_key.domain)?;
    let kind = FileKind::from_wire_name(&file_key.kind)?;
    let key = FileRequestKey::for_domain(domain, kind, &file_key.normalized_name).ok()?;
    Some(ResourceRequest::File(FileRequest::new(
        key,
        request.original_spelling.clone(),
    )))
}

fn prefetch_file_key(request: &FileRequestKey) -> Option<PrefetchFileKey> {
    PrefetchFileKey::new(
        request.domain().wire_name(),
        request.kind().wire_name(),
        request.name(),
    )
}

pub(crate) fn semantic_file_key(request: &FileRequestKey) -> Option<PrefetchFileKey> {
    prefetch_file_key(request)
}

fn distribution_key(request: &FileRequestKey) -> String {
    let kind = match request.kind() {
        FileKind::TexInput
        | FileKind::Image
        | FileKind::GenericAsset
        | FileKind::VirtualFont
        | FileKind::PdfFontMap
        | FileKind::PdfEncoding
        | FileKind::PdfFontProgram => DistributionFileKind::Tex,
        FileKind::Tfm => DistributionFileKind::Tfm,
        FileKind::BibAux => DistributionFileKind::BibAux,
        FileKind::ClassicBibData => DistributionFileKind::ClassicBib,
        FileKind::BibStyle => DistributionFileKind::BibStyle,
        _ => DistributionFileKind::Tex,
    };
    DistributionFileRequestKey::new(kind, request.name()).map_or_else(
        |_| format!("{}:{}", kind.manifest_name(), request.name()),
        |key| key.manifest_key().to_string(),
    )
}

fn distribution_record_request(record: &LookupRecord) -> Option<ResourceRequest> {
    let key = DistributionFileRequestKey::from_manifest_key(&record.request_key).ok()?;
    let kind = FileKind::from_wire_name(&record.kind).unwrap_or_else(|| match key.kind() {
        DistributionFileKind::Tex => FileKind::TexInput,
        DistributionFileKind::Tfm => FileKind::Tfm,
        DistributionFileKind::BibAux => FileKind::BibAux,
        DistributionFileKind::ClassicBib => FileKind::ClassicBibData,
        DistributionFileKind::BibStyle => FileKind::BibStyle,
    });
    let key = FileRequestKey::new(kind, key.normalized_name()).ok()?;
    Some(ResourceRequest::File(FileRequest::new(
        key,
        record.original_spelling.clone(),
    )))
}

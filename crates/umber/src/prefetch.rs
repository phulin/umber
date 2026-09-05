//! Shared predictive resource policy used by native and browser adapters.

use std::collections::BTreeSet;

use umber_distribution::{
    FileKind as DistributionFileKind, FileRequestKey as DistributionFileRequestKey, LookupManifest,
    LookupOutcome, LookupRecord, LookupRole, NegativeScope, PrefetchBudget, PrefetchIdentity,
    Readiness, ResolvedIdentity, extract_literal_hints,
};
use umber_hash::{AHash64, HashDomain};

use crate::{FileKind, FileRequest, FileRequestKey, ResourceRequest};

#[cfg(test)]
mod tests;

pub const PREFETCH_POLICY_VERSION: &str = "literal-groups-v1";

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
    metrics: PrefetchMetrics,
}

impl PrefetchPlanner {
    pub fn new(identity: PrefetchIdentity, budget: PrefetchBudget) -> Self {
        Self {
            accepted: LookupManifest::new(identity.clone()),
            prior: None,
            identity,
            budget,
            seen_startup: BTreeSet::new(),
            metrics: PrefetchMetrics::default(),
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

    #[must_use]
    pub const fn metrics(&self) -> PrefetchMetrics {
        self.metrics
    }

    /// Combines prior accepted lookups and literal source hints.  Requests are
    /// deduplicated by typed key and remain in stable source/history order.
    pub fn startup_hints(&mut self, source: &str) -> Vec<ResourceRequest> {
        let mut output = Vec::new();
        let prior_requests = self
            .prior_manifest()
            .into_iter()
            .flat_map(|prior| prior.resolved_records())
            .filter_map(distribution_record_request)
            .collect::<Vec<_>>();
        for request in prior_requests {
            if self.seen_startup.insert(request_identity(&request)) {
                output.push(request);
            }
        }
        let literal = extract_literal_hints(source, Default::default());
        self.metrics.literal_hints = self
            .metrics
            .literal_hints
            .saturating_add(literal.len() as u64);
        for hint in literal {
            let kind = match hint.kind {
                umber_distribution::LiteralHintKind::IncludeGraphics => FileKind::Image,
                _ => FileKind::TexInput,
            };
            let Ok(key) = FileRequestKey::new(kind, &hint.name) else {
                continue;
            };
            let request = ResourceRequest::File(FileRequest::new(key, hint.original_spelling));
            if self.seen_startup.insert(request_identity(&request)) {
                output.push(request);
            }
        }
        output.truncate(self.budget.max_files);
        output
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

//! Bounded literal source hints and package-group scheduling.
//!
//! This scanner is intentionally lexical.  It recognizes only a small set of
//! literal LaTeX control sequences and never expands a macro, executes TeX,
//! or changes the engine's search semantics.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{FileKind, ObjectEntry};

#[cfg(test)]
mod tests;

const MAX_DEFAULT_HINTS: usize = 256;
const MAX_DEFAULT_NAME_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LiteralHintKind {
    DocumentClass,
    Package,
    Input,
    IncludeGraphics,
}

impl LiteralHintKind {
    #[must_use]
    pub const fn command(self) -> &'static str {
        match self {
            Self::DocumentClass => "documentclass",
            Self::Package => "usepackage",
            Self::Input => "input",
            Self::IncludeGraphics => "includegraphics",
        }
    }

    #[must_use]
    pub const fn file_kind(self) -> FileKind {
        // TeX Live's canonical file catalogue keeps graphics and runtime
        // inputs in the tex namespace; the caller retains its richer engine
        // FileKind when admitting the resulting hint.
        FileKind::Tex
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LiteralHint {
    pub kind: LiteralHintKind,
    pub original_spelling: String,
    pub name: String,
    pub byte_offset: usize,
}

impl LiteralHint {
    /// Returns the bounded catalogue candidate for this semantic hint.
    ///
    /// Extraction deliberately retains the source spelling in [`Self::name`]
    /// so callers can preserve the lookup context.  TeX's known class,
    /// package, and input surfaces have a predictable default suffix when the
    /// final path component has no explicit extension; graphics keep their
    /// existing extension/search behavior.
    #[must_use]
    pub fn normalized_name(&self) -> String {
        normalize_literal_hint_name(self.kind, &self.name)
    }
}

/// Normalizes one literal hint into a bounded catalogue candidate without
/// changing the spelling used by the eventual resolver.
///
/// This is predictor-only behavior: it does not assert that the candidate
/// exists or alter provider precedence.  Only a missing extension on the
/// final path component receives a semantic default.  Dots in directory
/// names do not suppress the default, and graphics remain extensionless so
/// their existing image search policy continues to decide the winner.
#[must_use]
pub fn normalize_literal_hint_name(kind: LiteralHintKind, name: &str) -> String {
    let extension = match kind {
        LiteralHintKind::DocumentClass => Some("cls"),
        LiteralHintKind::Package => Some("sty"),
        LiteralHintKind::Input => Some("tex"),
        LiteralHintKind::IncludeGraphics => None,
    };
    let Some(extension) = extension else {
        return name.to_owned();
    };
    let component = name.rsplit('/').next().unwrap_or(name);
    let has_extension = !matches!(component, "." | "..")
        && component.rfind('.').is_some_and(|position| position > 0);
    if has_extension {
        return name.to_owned();
    }
    let mut normalized = String::with_capacity(name.len() + extension.len() + 1);
    normalized.push_str(name);
    normalized.push('.');
    normalized.push_str(extension);
    normalized
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiteralHintLimits {
    pub max_hints: usize,
    pub max_name_bytes: usize,
}

impl Default for LiteralHintLimits {
    fn default() -> Self {
        Self {
            max_hints: MAX_DEFAULT_HINTS,
            max_name_bytes: MAX_DEFAULT_NAME_BYTES,
        }
    }
}

/// Extracts obvious literal file names while ignoring comments and malformed
/// command arguments.  An escaped percent is ordinary source text; an
/// unescaped percent ends the current line.
#[must_use]
pub fn extract_literal_hints(source: &str, limits: LiteralHintLimits) -> Vec<LiteralHint> {
    if limits.max_hints == 0 || limits.max_name_bytes == 0 {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut index = 0;
    while index < source.len() && output.len() < limits.max_hints {
        let Some(relative) = source[index..].find('\\') else {
            break;
        };
        index += relative;
        let slash = index;
        if in_comment(source, slash) {
            index = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            continue;
        }
        index += 1;
        let Some(command_start) = source.get(index..).and_then(|tail| {
            tail.chars()
                .next()
                .filter(|character| character.is_ascii_alphabetic())
                .map(|_| index)
        }) else {
            continue;
        };
        while index < source.len()
            && source[index..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        {
            index += source[index..]
                .chars()
                .next()
                .expect("checked command character")
                .len_utf8();
        }
        let command = &source[command_start..index];
        let kind = match command {
            "documentclass" => LiteralHintKind::DocumentClass,
            "usepackage" | "RequirePackage" => LiteralHintKind::Package,
            "input" | "include" => LiteralHintKind::Input,
            "includegraphics" => LiteralHintKind::IncludeGraphics,
            _ => continue,
        };
        let mut cursor = skip_horizontal_space(source, index);
        if matches!(
            kind,
            LiteralHintKind::DocumentClass
                | LiteralHintKind::Package
                | LiteralHintKind::IncludeGraphics
        ) && source.as_bytes().get(cursor) == Some(&b'[')
        {
            let Some(end) = balanced_bracket(source, cursor, b'[', b']') else {
                continue;
            };
            cursor = skip_horizontal_space(source, end);
        }
        let Some((argument, end)) = literal_argument(source, cursor, limits.max_name_bytes) else {
            continue;
        };
        for name in argument.split(',') {
            let name = name.trim();
            if name.is_empty()
                || name.len() > limits.max_name_bytes
                || name.chars().any(char::is_control)
            {
                continue;
            }
            output.push(LiteralHint {
                kind,
                original_spelling: name.to_owned(),
                name: name.to_owned(),
                byte_offset: slash,
            });
            if output.len() == limits.max_hints {
                break;
            }
        }
        index = end;
    }
    output
}

fn skip_horizontal_space(source: &str, mut index: usize) -> usize {
    while index < source.len() {
        let byte = source.as_bytes()[index];
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn balanced_bracket(source: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0_u32;
    let mut index = start;
    while index < source.len() {
        let byte = source.as_bytes()[index];
        if byte == b'%' && !escaped(source, index) {
            index = source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset);
            continue;
        }
        if byte == open {
            depth = depth.checked_add(1)?;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}

fn literal_argument(source: &str, start: usize, max_name_bytes: usize) -> Option<(&str, usize)> {
    if source.as_bytes().get(start) == Some(&b'{') {
        let end = balanced_bracket(source, start, b'{', b'}')?;
        let argument = source.get(start + 1..end - 1)?;
        (argument.len() <= max_name_bytes).then_some((argument, end))
    } else {
        let tail = source.get(start..)?;
        let end = tail
            .find(|character: char| character.is_whitespace() || matches!(character, '%' | '\\'))
            .map_or(source.len(), |offset| start + offset);
        let argument = source.get(start..end)?;
        (!argument.is_empty() && argument.len() <= max_name_bytes).then_some((argument, end))
    }
}

fn escaped(source: &str, index: usize) -> bool {
    let mut backslashes = 0;
    for byte in source.as_bytes()[..index].iter().rev() {
        if *byte != b'\\' {
            break;
        }
        backslashes += 1;
    }
    backslashes % 2 == 1
}

fn in_comment(source: &str, index: usize) -> bool {
    let line_start = source[..index].rfind('\n').map_or(0, |offset| offset + 1);
    let mut percent = None;
    for position in line_start..index {
        if source.as_bytes()[position] == b'%' && !escaped(source, position) {
            percent = Some(position);
        }
    }
    percent.is_some()
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PrefetchClass {
    SmallRuntime,
    Font,
    Image,
    Document,
    Other,
}

/// Complete semantic identity carried by the prefetch policy.
///
/// Distribution catalogue keys are intentionally many-to-one for several
/// TeX resource kinds (for example VF and PDF font files share `tex:`). A
/// policy request therefore retains the canonical domain, semantic kind, and
/// normalized name separately from that transport key. The strings mirror
/// `umber-vfs::FileRequestKey` without making this dependency-free crate
/// depend on the VFS crate.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PrefetchFileKey {
    pub domain: String,
    pub kind: String,
    pub normalized_name: String,
}

impl PrefetchFileKey {
    pub fn new(
        domain: impl Into<String>,
        kind: impl Into<String>,
        normalized_name: impl Into<String>,
    ) -> Option<Self> {
        let key = Self {
            domain: domain.into(),
            kind: kind.into(),
            normalized_name: normalized_name.into(),
        };
        key.valid().then_some(key)
    }

    fn valid(&self) -> bool {
        !self.domain.is_empty()
            && !self.kind.is_empty()
            && !self.normalized_name.is_empty()
            && self.domain.len() <= 64
            && self.kind.len() <= 64
            && self.normalized_name.len() <= 4096
            && self
                .domain
                .bytes()
                .chain(self.kind.bytes())
                .chain(self.normalized_name.bytes())
                .all(|byte| !byte.is_ascii_control())
    }

    /// Deterministic identity used only for policy maps and payload
    /// accounting. It is not a catalogue or URL key.
    #[must_use]
    pub fn identity(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.domain, self.kind, self.normalized_name
        )
    }
}

impl PrefetchClass {
    #[must_use]
    pub fn for_key(key: &str) -> Self {
        if key.starts_with("tfm:") || key.starts_with("font:") {
            return Self::Font;
        }
        let name = key.split_once(':').map_or(key, |(_, name)| name);
        let extension = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "eps" | "svg") {
            return Self::Image;
        }
        if matches!(extension.as_str(), "tex" | "sty" | "cls" | "def" | "ltx") {
            return Self::SmallRuntime;
        }
        if matches!(extension.as_str(), "pdf" | "ps" | "dvi") {
            Self::Document
        } else {
            Self::Other
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefetchCandidate {
    pub key: String,
    pub object: ObjectEntry,
    pub class: PrefetchClass,
    pub required: bool,
    pub file_key: Option<PrefetchFileKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrefetchBudget {
    pub max_files: usize,
    pub max_bytes: u64,
    pub max_runtime_bytes: u64,
    pub max_font_bytes: u64,
    pub max_image_bytes: u64,
    pub max_document_bytes: u64,
    /// Maximum admitted runtime text inspected for literal follow-up hints.
    pub max_runtime_scan_bytes: u64,
    /// Maximum follow-up literals admitted from one bounded closure pass.
    pub max_followup_hints: usize,
    /// Number of admitted runtime-text tiers followed after a seed hint.
    pub max_followup_depth: usize,
}

impl Default for PrefetchBudget {
    fn default() -> Self {
        Self {
            max_files: 64,
            max_bytes: 16 * 1024 * 1024,
            max_runtime_bytes: 8 * 1024 * 1024,
            max_font_bytes: 2 * 1024 * 1024,
            max_image_bytes: 2 * 1024 * 1024,
            max_document_bytes: 512 * 1024,
            max_runtime_scan_bytes: 256 * 1024,
            max_followup_hints: 32,
            max_followup_depth: 1,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrefetchSelection {
    pub required: Vec<PrefetchCandidate>,
    pub hints: Vec<PrefetchCandidate>,
    pub demand_bytes: u64,
    pub prefetch_bytes: u64,
}

fn candidate_identity(candidate: &PrefetchCandidate) -> String {
    candidate.file_key.as_ref().map_or_else(
        || format!("transport:{}", candidate.key),
        PrefetchFileKey::identity,
    )
}

fn payload_identity(object: &ObjectEntry) -> (String, String, u64) {
    (object.object.clone(), object.ahash64.clone(), object.bytes)
}

/// Selects required records plus a bounded, deterministic small group.  The
/// required set is never dropped for speculative budget reasons; each hint
/// class has an independent ceiling so a large font/image cannot consume the
/// runtime budget.
#[must_use]
pub fn select_prefetch_group(
    required: impl IntoIterator<Item = PrefetchCandidate>,
    candidates: impl IntoIterator<Item = PrefetchCandidate>,
    budget: PrefetchBudget,
) -> PrefetchSelection {
    let mut output = PrefetchSelection::default();
    let mut seen = BTreeSet::new();
    let mut required_payloads = BTreeSet::new();
    for candidate in required {
        if !seen.insert(candidate_identity(&candidate)) {
            continue;
        }
        let payload = payload_identity(&candidate.object);
        if required_payloads.insert(payload) {
            output.demand_bytes = output.demand_bytes.saturating_add(candidate.object.bytes);
        }
        output.required.push(candidate);
    }
    let mut files = 0;
    let mut total = 0_u64;
    let mut by_class = [0_u64; 5];
    let mut payloads = required_payloads;
    for mut candidate in candidates {
        if candidate.required || !seen.insert(candidate_identity(&candidate)) {
            continue;
        }
        if files >= budget.max_files {
            break;
        }
        let class_index = candidate.class as usize;
        let class_limit = match candidate.class {
            PrefetchClass::SmallRuntime => budget.max_runtime_bytes,
            PrefetchClass::Font => budget.max_font_bytes,
            PrefetchClass::Image => budget.max_image_bytes,
            PrefetchClass::Document => budget.max_document_bytes,
            PrefetchClass::Other => budget.max_bytes,
        };
        let bytes = candidate.object.bytes;
        let payload = payload_identity(&candidate.object);
        let new_payload = payloads.insert(payload.clone());
        if new_payload
            && (bytes > class_limit.saturating_sub(by_class[class_index])
                || bytes > budget.max_bytes.saturating_sub(total))
        {
            payloads.remove(&payload);
            continue;
        }
        candidate.required = false;
        if new_payload {
            by_class[class_index] = by_class[class_index].saturating_add(bytes);
            total = total.saturating_add(bytes);
            output.prefetch_bytes = output.prefetch_bytes.saturating_add(bytes);
        }
        files += 1;
        output.hints.push(candidate);
    }
    output
}

/// A host-neutral file request used by the shared prefetch policy.
///
/// `key` is the immutable catalogue transport key. `file_key` is the complete
/// semantic identity and is the policy's equality/deduplication key whenever
/// supplied by a typed host. The original spelling and search context remain
/// attached so an adapter can issue the same lookup without reconstructing it
/// from a coarse budget class.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PrefetchRequest {
    pub key: String,
    pub file_key: Option<PrefetchFileKey>,
    pub original_spelling: String,
    pub search_context: String,
    pub class: PrefetchClass,
    pub required: bool,
    depth: usize,
}

impl PrefetchRequest {
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        original_spelling: impl Into<String>,
        search_context: impl Into<String>,
        required: bool,
    ) -> Self {
        let key = key.into();
        Self {
            // Legacy callers only have a catalogue transport key. Keep that
            // identity opaque; typed hosts must use `for_file_key` so no
            // semantic kind is guessed from a prefix or budget class.
            file_key: None,
            class: PrefetchClass::for_key(&key),
            key,
            original_spelling: original_spelling.into(),
            search_context: search_context.into(),
            required,
            depth: 0,
        }
    }

    /// Constructs a request at a typed host boundary. This is the production
    /// constructor; the transport key is retained only for catalogue lookup.
    #[must_use]
    pub fn for_file_key(
        file_key: PrefetchFileKey,
        transport_key: impl Into<String>,
        original_spelling: impl Into<String>,
        search_context: impl Into<String>,
        required: bool,
    ) -> Self {
        let key = transport_key.into();
        Self {
            class: PrefetchClass::for_key(&key),
            key,
            file_key: Some(file_key),
            original_spelling: original_spelling.into(),
            search_context: search_context.into(),
            required,
            depth: 0,
        }
    }

    #[must_use]
    pub fn identity(&self) -> String {
        self.file_key.as_ref().map_or_else(
            || format!("transport:{}", self.key),
            PrefetchFileKey::identity,
        )
    }

    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    #[must_use]
    pub fn with_class(mut self, class: PrefetchClass) -> Self {
        self.class = class;
        self
    }

    #[must_use]
    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }
}

fn literal_hint_request(
    hint: &LiteralHint,
    search_context: &str,
    required: bool,
    typed: bool,
    depth: usize,
) -> Option<PrefetchRequest> {
    let name = hint.normalized_name();
    let key = crate::FileRequestKey::new(FileKind::Tex, name).ok()?;
    let file_key = PrefetchFileKey::new(
        "tex",
        if hint.kind == LiteralHintKind::IncludeGraphics {
            "image"
        } else {
            "tex"
        },
        key.normalized_name(),
    )?;
    let transport_key = key.manifest_key().to_string();
    let class = match hint.kind {
        LiteralHintKind::IncludeGraphics => PrefetchClass::Image,
        _ => PrefetchClass::for_key(&transport_key),
    };
    let request = if typed {
        PrefetchRequest::for_file_key(
            file_key,
            transport_key,
            hint.original_spelling.clone(),
            search_context,
            required,
        )
    } else {
        PrefetchRequest::new(
            transport_key,
            hint.original_spelling.clone(),
            search_context,
            required,
        )
    };
    Some(request.with_class(class).with_depth(depth))
}

/// Stable region identity supplied by a retained checkpoint owner.
///
/// This is intentionally opaque to the policy.  Native and WASM callers may
/// serialize it, but neither caller may substitute a suspension counter or a
/// host retry number for the retained-anchor identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PrefetchRegionKey(String);

impl PrefetchRegionKey {
    #[must_use]
    pub fn new(anchor: impl Into<String>) -> Option<Self> {
        let anchor = anchor.into();
        (!anchor.is_empty() && anchor.len() <= 256).then_some(Self(anchor))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Escalation advice after a resource miss discarded meaningful work in one
/// retained replay region.  The policy never chooses a whole distribution;
/// adapters use the tier to admit only already-known package/dependency
/// companions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrefetchEscalation {
    pub tier: u8,
    pub discarded_work_delta: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PrefetchPolicyMetrics {
    pub queued_requests: u64,
    pub admitted_files: u64,
    pub scanned_runtime_bytes: u64,
    pub followup_hints: u64,
    pub replay_escalations: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ReplayMisses {
    region: u32,
    requests: u32,
    tier: u8,
}

#[derive(Clone, Debug, Default)]
struct PrefetchAccounting {
    semantic_keys: BTreeSet<String>,
    demanded_keys: BTreeSet<String>,
    payloads: BTreeSet<(String, String, u64)>,
    total_bytes: u64,
    class_bytes: [u64; 5],
}

/// Shared bounded queue and replay escalation policy.
///
/// The policy has no catalog, transport, VFS, or engine dependency.  It only
/// owns canonical de-duplication, lexical closure scanning, budget accounting
/// knobs, and replay-region counters.  A request is considered ready by a
/// caller only after that caller has successfully admitted its verified bytes
/// to the engine-readable VFS.
#[derive(Clone, Debug)]
pub struct PrefetchPolicy {
    budget: PrefetchBudget,
    queue: VecDeque<PrefetchRequest>,
    queued: BTreeMap<String, usize>,
    queue_priorities: BTreeMap<String, u64>,
    depths: BTreeMap<String, usize>,
    classes: BTreeMap<String, PrefetchClass>,
    dependencies: BTreeMap<String, Vec<PrefetchRequest>>,
    admitted: BTreeSet<String>,
    scanned: BTreeSet<String>,
    scanned_runtime_bytes: u64,
    speculative_scheduled: usize,
    replay: BTreeMap<(PrefetchRegionKey, String), ReplayMisses>,
    replay_regions: BTreeMap<PrefetchRegionKey, u32>,
    replay_region_requests: BTreeMap<PrefetchRegionKey, BTreeSet<String>>,
    /// Semantic keys that have left the queue and are now in-flight or have
    /// already been attempted. Optional rediscovery in this run cannot
    /// requeue them; a required demand may still enqueue the same key.
    attempted: BTreeSet<String>,
    accounting: PrefetchAccounting,
    last_discarded_work: u64,
    metrics: PrefetchPolicyMetrics,
}

impl PrefetchPolicy {
    #[must_use]
    pub fn new(budget: PrefetchBudget) -> Self {
        Self {
            budget,
            queue: VecDeque::new(),
            queued: BTreeMap::new(),
            queue_priorities: BTreeMap::new(),
            depths: BTreeMap::new(),
            classes: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            admitted: BTreeSet::new(),
            scanned: BTreeSet::new(),
            scanned_runtime_bytes: 0,
            speculative_scheduled: 0,
            replay: BTreeMap::new(),
            replay_regions: BTreeMap::new(),
            replay_region_requests: BTreeMap::new(),
            attempted: BTreeSet::new(),
            accounting: PrefetchAccounting::default(),
            last_discarded_work: 0,
            metrics: PrefetchPolicyMetrics::default(),
        }
    }

    #[must_use]
    pub const fn budget(&self) -> PrefetchBudget {
        self.budget
    }

    /// Configures the host-provided limits before the first stateful
    /// selection. Once a reservation or demand has been recorded, changing
    /// limits would reset neither accounting nor ownership, so reject it.
    pub fn configure_budget(&mut self, budget: PrefetchBudget) -> bool {
        if !self.accounting.semantic_keys.is_empty()
            || !self.accounting.demanded_keys.is_empty()
            || !self.accounting.payloads.is_empty()
        {
            return false;
        }
        self.budget = budget;
        true
    }

    #[must_use]
    pub const fn metrics(&self) -> PrefetchPolicyMetrics {
        self.metrics
    }

    /// Reports whether a semantic candidate has already been accounted for by
    /// this policy.  Hosts use this only for opt-in diagnostics; it does not
    /// participate in selection or admission.
    #[must_use]
    pub fn candidate_is_known(&self, candidate: &PrefetchCandidate) -> bool {
        let identity = candidate_identity(candidate);
        self.admitted.contains(&identity)
            || self.accounting.semantic_keys.contains(&identity)
            || self.accounting.demanded_keys.contains(&identity)
            || self.queued.contains_key(&identity)
            || self.attempted.contains(&identity)
    }

    /// Reports whether a typed file key's payload has crossed the planner's
    /// engine-admission boundary.  This is host telemetry only.
    #[must_use]
    pub fn file_key_is_admitted(&self, request_key: &PrefetchFileKey) -> bool {
        self.admitted.contains(&request_key.identity())
    }

    /// Reports whether this admission would enter the bounded runtime scan.
    /// Hosts use this only to label an opt-in diagnostic; the admission path
    /// itself remains unchanged.
    #[must_use]
    pub fn runtime_scan_is_allowed(
        &self,
        request_key: &PrefetchFileKey,
        byte_count: usize,
    ) -> bool {
        let identity = request_key.identity();
        let byte_count = byte_count as u64;
        !self.scanned.contains(&identity)
            && byte_count <= self.budget.max_runtime_scan_bytes
            && byte_count
                <= self
                    .budget
                    .max_runtime_scan_bytes
                    .saturating_sub(self.scanned_runtime_bytes)
    }

    /// Selects one host batch against the policy's cumulative per-run
    /// reservation ledger. Required payloads are always returned and never
    /// consume speculative byte or file ceilings. Optional reservations are
    /// charged before acquisition; a failed optional fetch keeps its charge
    /// for the rest of this run, which is conservative and prevents retry
    /// cycles from exceeding the declared budget.
    pub fn select_prefetch_group(
        &mut self,
        required: impl IntoIterator<Item = PrefetchCandidate>,
        candidates: impl IntoIterator<Item = PrefetchCandidate>,
    ) -> PrefetchSelection {
        let mut output = PrefetchSelection::default();
        let mut seen = BTreeSet::new();
        let mut required_payloads = BTreeSet::new();
        for candidate in required {
            let identity = candidate_identity(&candidate);
            if !seen.insert(identity.clone()) {
                continue;
            }
            if self.accounting.demanded_keys.insert(identity) {
                let payload = payload_identity(&candidate.object);
                if required_payloads.insert(payload.clone()) {
                    output.demand_bytes =
                        output.demand_bytes.saturating_add(candidate.object.bytes);
                }
                self.accounting.payloads.insert(payload);
            }
            output.required.push(candidate);
        }
        for mut candidate in candidates {
            if candidate.required {
                continue;
            }
            let identity = candidate_identity(&candidate);
            if !seen.insert(identity.clone())
                || self.accounting.demanded_keys.contains(&identity)
                || self.accounting.semantic_keys.contains(&identity)
            {
                continue;
            }
            if self.accounting.semantic_keys.len() >= self.budget.max_files {
                break;
            }
            let class_index = candidate.class as usize;
            let class_limit = match candidate.class {
                PrefetchClass::SmallRuntime => self.budget.max_runtime_bytes,
                PrefetchClass::Font => self.budget.max_font_bytes,
                PrefetchClass::Image => self.budget.max_image_bytes,
                PrefetchClass::Document => self.budget.max_document_bytes,
                PrefetchClass::Other => self.budget.max_bytes,
            };
            let bytes = candidate.object.bytes;
            let payload = payload_identity(&candidate.object);
            let new_payload = !self.accounting.payloads.contains(&payload);
            if new_payload
                && (bytes
                    > self
                        .budget
                        .max_bytes
                        .saturating_sub(self.accounting.total_bytes)
                    || bytes > class_limit.saturating_sub(self.accounting.class_bytes[class_index]))
            {
                continue;
            }
            candidate.required = false;
            self.accounting.semantic_keys.insert(identity);
            self.accounting.payloads.insert(payload.clone());
            if new_payload {
                self.accounting.total_bytes = self.accounting.total_bytes.saturating_add(bytes);
                self.accounting.class_bytes[class_index] =
                    self.accounting.class_bytes[class_index].saturating_add(bytes);
                output.prefetch_bytes = output.prefetch_bytes.saturating_add(bytes);
            }
            output.hints.push(candidate);
        }
        output
    }

    /// Enqueues one canonical request.  Required requests are never rejected
    /// because a speculative budget is full; hosts still enforce their normal
    /// demanded-resource limits separately.
    pub fn enqueue(&mut self, request: PrefetchRequest) -> bool {
        self.enqueue_with_priority(request, 0)
    }

    /// Enqueues one request ahead of lower-priority speculative work. Larger
    /// discarded-work deltas receive larger priorities; equal priorities keep
    /// FIFO order. Required requests still bypass the speculative cap.
    pub fn enqueue_with_priority(&mut self, request: PrefetchRequest, priority: u64) -> bool {
        let identity = request.identity();
        if request.key.is_empty()
            || self.admitted.contains(&identity)
            || (!request.required && self.attempted.contains(&identity))
        {
            return false;
        }
        if self.queued.contains_key(&identity) {
            let previous = self
                .queue_priorities
                .get(&identity)
                .copied()
                .unwrap_or_default();
            let Some(index) = self
                .queue
                .iter()
                .position(|queued| queued.identity() == identity)
            else {
                return false;
            };
            let queued = self.queue.remove(index).expect("queue index exists");
            let promoted = request.required && !queued.required;
            if !promoted && priority <= previous {
                self.queue.insert(index, queued);
                return false;
            }
            if promoted {
                // The speculative reservation represented this one queued
                // file. Once a demanded request takes it over, release that
                // reservation so another optional request may use the cap.
                self.speculative_scheduled = self.speculative_scheduled.saturating_sub(1);
                self.depths.insert(identity.clone(), request.depth());
                self.classes.insert(identity.clone(), request.class);
            }
            let queued = if promoted { request } else { queued };
            let priority = priority.max(previous);
            self.queue_priorities.insert(identity.clone(), priority);
            let insertion = self
                .queue
                .iter()
                .position(|queued| {
                    self.queue_priorities
                        .get(&queued.identity())
                        .copied()
                        .unwrap_or_default()
                        < priority
                })
                .unwrap_or(self.queue.len());
            self.queue.insert(insertion, queued);
            return true;
        }
        if !request.required && self.speculative_scheduled >= self.budget.max_files {
            return false;
        }
        let key = identity;
        let depth = request.depth();
        if !request.required {
            self.speculative_scheduled = self.speculative_scheduled.saturating_add(1);
        }
        self.depths.insert(key.clone(), depth);
        self.classes.insert(key.clone(), request.class);
        self.queued.insert(key, depth);
        self.queue_priorities.insert(request.identity(), priority);
        let insertion = self
            .queue
            .iter()
            .position(|queued| {
                self.queue_priorities
                    .get(&queued.identity())
                    .copied()
                    .unwrap_or_default()
                    < priority
            })
            .unwrap_or(self.queue.len());
        self.queue.insert(insertion, request);
        self.metrics.queued_requests = self.metrics.queued_requests.saturating_add(1);
        true
    }

    /// Enqueues the shared lexical seed set and returns the bounded first
    /// batch.  Prior-run and explicit format closure requests are enqueued by
    /// the host through [`Self::enqueue`] before calling this method.
    pub fn enqueue_literal_hints(&mut self, source: &str) -> usize {
        let mut count = 0;
        for hint in extract_literal_hints(source, LiteralHintLimits::default()) {
            let Some(request) = literal_hint_request(&hint, "literal", false, true, 0) else {
                continue;
            };
            if self.enqueue(request) {
                count += 1;
            }
        }
        count
    }

    /// Drains a finite queue in source/history order.  The caller may use a
    /// smaller host batch limit, but must eventually acknowledge an empty
    /// response so false-positive startup hints cannot create a retry loop.
    pub fn drain(&mut self, limit: usize) -> Vec<PrefetchRequest> {
        let mut output = Vec::new();
        let limit = limit.min(self.budget.max_files.max(1));
        while output.len() < limit {
            let Some(request) = self.queue.pop_front() else {
                break;
            };
            let identity = request.identity();
            self.queued.remove(&identity);
            self.queue_priorities.remove(&identity);
            self.attempted.insert(identity);
            output.push(request);
        }
        output
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Marks a verified payload only after the host has admitted it to its
    /// engine-readable VFS.  Small runtime text is then scanned lexically and
    /// its child literals are queued at the next bounded tier. Authenticated
    /// dependency metadata may be supplied by the host at this same seam;
    /// metadata is scheduling evidence and never engine readiness.
    pub fn admitted(&mut self, request_key: &str, bytes: &[u8]) {
        self.admitted_with_metadata(request_key, "", bytes, std::iter::empty());
    }

    pub fn admitted_with_metadata(
        &mut self,
        request_key: &str,
        _virtual_path: &str,
        bytes: &[u8],
        dependencies: impl IntoIterator<Item = PrefetchRequest>,
    ) {
        let request = PrefetchRequest::new(request_key, request_key, "admission", false);
        self.admitted_request_with_metadata(&request, bytes, dependencies);
    }

    pub fn admitted_request_with_metadata(
        &mut self,
        request: &PrefetchRequest,
        bytes: &[u8],
        dependencies: impl IntoIterator<Item = PrefetchRequest>,
    ) {
        self.admitted_request_with_class(request, request.class, bytes, dependencies);
    }

    /// Admission variant for adapters that retain a semantic file kind which
    /// aliases the distribution namespace (notably images and TeX inputs).
    /// The explicit class prevents an image named `figure.sty` from being
    /// interpreted as runtime text merely because its catalogue key ends in a
    /// runtime-looking suffix.
    pub fn admitted_with_class(
        &mut self,
        request_key: &str,
        class: PrefetchClass,
        bytes: &[u8],
        dependencies: impl IntoIterator<Item = PrefetchRequest>,
    ) {
        let request =
            PrefetchRequest::new(request_key, request_key, "admission", false).with_class(class);
        self.admitted_request_with_class(&request, class, bytes, dependencies);
    }

    pub fn admitted_request_with_class(
        &mut self,
        request: &PrefetchRequest,
        class: PrefetchClass,
        bytes: &[u8],
        dependencies: impl IntoIterator<Item = PrefetchRequest>,
    ) {
        let identity = request.identity();
        if !self.admitted.insert(identity.clone()) {
            return;
        }
        self.classes.insert(identity.clone(), class);
        if let Some(index) = self
            .queue
            .iter()
            .position(|queued| queued.identity() == identity)
        {
            self.queue.remove(index);
            self.queued.remove(&identity);
            self.queue_priorities.remove(&identity);
        }
        self.metrics.admitted_files = self.metrics.admitted_files.saturating_add(1);
        let parent_depth = self.depths.get(&identity).copied().unwrap_or_default();
        for dependency in dependencies {
            let dependency = dependency.with_depth(parent_depth.saturating_add(1));
            let known = self.dependencies.entry(identity.clone()).or_default();
            if !known
                .iter()
                .any(|known| known.identity() == dependency.identity())
            {
                known.push(dependency.clone());
            }
            self.enqueue(dependency);
        }
        let class = self.classes.get(&identity).copied().unwrap_or(class);
        if class != PrefetchClass::SmallRuntime || !self.scanned.insert(identity) {
            return;
        }
        let byte_count = bytes.len() as u64;
        if byte_count > self.budget.max_runtime_scan_bytes
            || byte_count
                > self
                    .budget
                    .max_runtime_scan_bytes
                    .saturating_sub(self.scanned_runtime_bytes)
        {
            return;
        }
        self.scanned_runtime_bytes = self.scanned_runtime_bytes.saturating_add(byte_count);
        self.metrics.scanned_runtime_bytes = self
            .metrics
            .scanned_runtime_bytes
            .saturating_add(byte_count);
        let text = String::from_utf8_lossy(bytes);
        let typed_parent = request.file_key.is_some();
        if parent_depth >= self.budget.max_followup_depth {
            return;
        }
        let remaining_hints = self
            .budget
            .max_followup_hints
            .saturating_sub(self.metrics.followup_hints as usize);
        if remaining_hints == 0 {
            return;
        }
        for hint in extract_literal_hints(
            &text,
            LiteralHintLimits {
                max_hints: remaining_hints,
                max_name_bytes: MAX_DEFAULT_NAME_BYTES,
            },
        ) {
            let Some(request) = literal_hint_request(
                &hint,
                "runtime",
                false,
                typed_parent,
                parent_depth.saturating_add(1),
            ) else {
                continue;
            };
            if self.enqueue(request) {
                self.metrics.followup_hints = self.metrics.followup_hints.saturating_add(1);
            }
        }
    }

    /// Returns the bounded, authenticated dependency closure for replay
    /// escalation.  Runtime-text depth is reset at this boundary because the
    /// requests are known package companions rather than fresh lexical child
    /// hints; the tier itself remains capped by the shared policy.
    #[must_use]
    pub fn dependency_closure(&self, request_key: &str, tier: u8) -> Vec<PrefetchRequest> {
        self.dependency_closure_for_identity(format!("transport:{request_key}"), tier)
    }

    #[must_use]
    pub fn dependency_closure_for_file_key(
        &self,
        request_key: &PrefetchFileKey,
        tier: u8,
    ) -> Vec<PrefetchRequest> {
        self.dependency_closure_for_identity(request_key.identity(), tier)
    }

    fn dependency_closure_for_identity(&self, identity: String, tier: u8) -> Vec<PrefetchRequest> {
        let mut output = Vec::new();
        let mut seen = BTreeSet::from([identity.clone()]);
        let mut frontier = vec![identity];
        for _ in 0..usize::from(tier.max(1)).min(3) {
            let mut next = Vec::new();
            for parent in frontier.drain(..) {
                for dependency in self.dependencies.get(&parent).into_iter().flatten() {
                    let identity = dependency.identity();
                    if seen.insert(identity.clone()) {
                        output.push(dependency.clone().with_depth(0));
                        next.push(identity);
                    }
                }
            }
            frontier = next;
        }
        output
    }

    /// Records a miss against the retained anchor.  A new region starts with
    /// a clean count; an unrelated request at the same region contributes to
    /// the region count as well as retaining its own diagnostic count.
    pub fn note_replay(
        &mut self,
        region: PrefetchRegionKey,
        request_key: &str,
        discarded_work: u64,
    ) -> Option<PrefetchEscalation> {
        self.note_replay_for_identity(region, format!("transport:{request_key}"), discarded_work)
    }

    pub fn note_replay_for_file_key(
        &mut self,
        region: PrefetchRegionKey,
        request_key: &PrefetchFileKey,
        discarded_work: u64,
    ) -> Option<PrefetchEscalation> {
        self.note_replay_for_identity(region, request_key.identity(), discarded_work)
    }

    fn note_replay_for_identity(
        &mut self,
        region: PrefetchRegionKey,
        request_key: String,
        discarded_work: u64,
    ) -> Option<PrefetchEscalation> {
        let delta = discarded_work.saturating_sub(self.last_discarded_work);
        if delta > 0 {
            self.last_discarded_work = discarded_work;
        }
        let region_count = if delta > 0 {
            let count = self
                .replay_regions
                .entry(region.clone())
                .and_modify(|count| *count = count.saturating_add(1))
                .or_insert(1);
            *count
        } else {
            *self.replay_regions.get(&region)?
        };
        let region_request_count = self
            .replay_region_requests
            .entry(region.clone())
            .or_default();
        region_request_count.insert(request_key.clone());
        let region_request_count = region_request_count.len();
        let miss = self.replay.entry((region, request_key)).or_default();
        miss.region = region_count;
        miss.requests = miss.requests.saturating_add(1);
        let tier = if miss.tier != 0 {
            miss.tier.saturating_add(1).min(3)
        } else if miss.requests >= 2 || region_count >= 2 || region_request_count >= 2 {
            1
        } else {
            0
        };
        if tier == 0 {
            return None;
        }
        miss.tier = tier;
        self.metrics.replay_escalations = self.metrics.replay_escalations.saturating_add(1);
        Some(PrefetchEscalation {
            tier: miss.tier,
            discarded_work_delta: delta,
        })
    }
}

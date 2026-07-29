//! Declarative, property-owned command semantic minifixtures.
//!
//! This module owns the corpus contract: manifest parsing and validation, the
//! bounded canonical run, and the projections. It lives in the library rather
//! than in a test binary so the regeneration path can drive exactly the same
//! code the gate does.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only corpus module discovers and reads its committed fixtures"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile,
    DiagnosticArgument, FatalError, FontResource, InputReason, InputTransition, ObservedToken,
    RecoveryKind, RegisteredSourceKind, SourceRegistration, canonical_names,
};
use tex_exec::{CanonicalMainControl, MainControlStep, Mode};
use tex_state::{
    ContentHash, EffectRecord, InputOpenState, InputReadState, PrintSink, Universe,
    macro_store::MacroMeaning,
    meaning::{Meaning, MeaningFlags},
    node::Node,
    token::Token,
};

pub mod channels;
#[cfg(test)]
mod tests;

pub use channels::{
    CapturedChannels, ChannelAuthority, ChannelContract, ChannelFailure, ChannelMismatch,
    STREAM_CHANNELS, StreamChannel, StreamDisposition, validate_xfail_disposition,
};

pub const SCHEMA: u32 = 1;
// A command-semantic minifixture must be truly minimal: short, self-contained,
// and exercising only the one engine behavior its case is about. The observed
// maximum across the committed corpus is 1,240 bytes
// (etex-diagnostics/etex-expressions.tex), so 2,048 is a real ceiling -- about
// 65% of headroom over the largest legitimate case today -- rather than the
// former 4,096, which nothing came close to.
pub const MAX_SOURCE_BYTES: u64 = 2 * 1024;
// The observed maximum is 31 lines (main-control/spacefactor-assignment.tex).
// 64 gives the same kind of real, but not knife-edge, headroom as
// `MAX_SOURCE_BYTES`.
pub const MAX_SOURCE_LINES: usize = 64;
// TeX82 permits 255 spans before the 256-span confusion boundary, so the
// bounded semantic runner must admit that one deliberately maximal case.
pub const MAX_STEPS: usize = 2_048;
pub const COUNT_SLOTS: usize = 256;
pub const BUG_PREFIX: &str = "umber2-";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainManifest {
    pub schema: u32,
    pub domain: String,
    #[serde(default)]
    pub property_domain: Option<String>,
    pub cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub property_id: String,
    #[serde(default)]
    pub profile: SessionProfile,
    #[serde(default)]
    pub font_inputs: BTreeMap<String, String>,
    pub source: String,
    pub provenance: Provenance,
    pub projection: Projection,
    pub expected: Vec<String>,
    pub expectation: Expectation,
    /// The disposition of every channel this case's run produces.
    ///
    /// Parsed as optional only so the regeneration path can read a manifest it
    /// is about to write this block into. [`validate_case`] requires it, so a
    /// case that omits it fails the gate instead of silently asserting
    /// nothing outside its projection.
    #[serde(default)]
    pub channels: Option<ChannelContract>,
    #[serde(default)]
    pub terminal_lines: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    /// tex.web's `-interaction` mode this case's job runs under.
    ///
    /// Defaults to `scrollmode`: `scripts/run-minifixture-oracle.sh` runs
    /// every case that way (see its "Interaction mode" comment for why), so a
    /// case's channels are comparable to that sweep's oracle capture only
    /// under the default. A case that needs a different mode -- e.g.
    /// `main-control/show-completion`, which exists to exercise the `?`
    /// prompt only `errorstopmode` issues -- declares one explicitly, and
    /// [`validate_case`] then requires [`Self::interaction_mode_note`] to say
    /// why, so the corpus records the deviation instead of leaving it to be
    /// discovered by a confused diff against the standard sweep.
    #[serde(default)]
    pub interaction_mode: CaseInteractionMode,
    /// Required exactly when [`Self::interaction_mode`] is not the default,
    /// explaining what that case's channels are being compared against
    /// instead of a standard `scrollmode` oracle capture.
    #[serde(default)]
    pub interaction_mode_note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub authority: String,
    pub manifest: String,
    pub sections: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    pub kind: ProjectionKind,
    #[serde(default)]
    pub count_registers: Vec<u16>,
    #[serde(default)]
    pub include_count_mutations: bool,
    #[serde(default)]
    pub kinds: Vec<ObservationKind>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub command_names: Vec<String>,
    #[serde(default)]
    pub alignment_transitions: Vec<String>,
    #[serde(default)]
    pub box_registers: Vec<u16>,
    pub node_depth: Option<u8>,
    #[serde(default)]
    pub include_mode_transitions: bool,
    #[serde(default)]
    pub include_artifact_hashes: bool,
    #[serde(default)]
    pub terminal_checks: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectionKind {
    Classification,
    ConditionSteps,
    SkippingConditionSteps,
    BranchSelections,
    PredicateOutcomes,
    Observations,
    ExecutionBoundaries,
    State,
    TerminalChecks,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionProfile {
    #[default]
    Initex,
    EtexInitex,
    EtexLoaded,
    Production,
}

/// tex.web's four `-interaction` modes, spelled exactly as pdfTeX's own flag
/// does (`-interaction=scrollmode`, ...) so a case's declared value can be
/// handed straight to the oracle runner's command line.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CaseInteractionMode {
    #[default]
    Scrollmode,
    Errorstopmode,
    Nonstopmode,
    Batchmode,
}

impl CaseInteractionMode {
    /// The engine-level mode this case's declared value selects.
    #[must_use]
    pub const fn engine_mode(self) -> tex_state::InteractionMode {
        match self {
            Self::Scrollmode => tex_state::InteractionMode::Scroll,
            Self::Errorstopmode => tex_state::InteractionMode::ErrorStop,
            Self::Nonstopmode => tex_state::InteractionMode::Nonstop,
            Self::Batchmode => tex_state::InteractionMode::Batch,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationKind {
    Command,
    Input,
    Alignment,
    Recovery,
    ScannerStatus,
    Macro,
    Scanner,
    TokenList,
    Mutation,
    Diagnostic,
    Effect,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Expectation {
    Pass,
    Xfail {
        bug: String,
        mismatch: MismatchFingerprint,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MismatchFingerprint {
    pub index: usize,
    pub kind: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Deserialize)]
pub struct PropertyShard {
    pub domain: String,
    pub properties: Vec<OwnedProperty>,
}

#[derive(Debug, Deserialize)]
pub struct OwnedProperty {
    pub id: String,
}

pub struct DeclaredCase {
    pub domain_dir: PathBuf,
    pub domain: String,
    pub case: Case,
}

#[derive(Default)]
pub struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

pub struct SemanticRun {
    pub observations: Vec<CommandObservation>,
    pub counts: [i32; COUNT_SLOTS],
    pub universe: Universe,
    pub mode_transitions: Vec<Mode>,
    pub artifacts: Vec<ContentHash>,
    /// The complete serialized `.dvi` file this run produced, or empty if it
    /// shipped no pages. This is the same bytes §642's `finish_job` reports
    /// the length of -- built with `tex_out::dvi::DviStreamWriter` over the
    /// run's own [`tex_exec::PreparedDviPage::into_plan`] output, exactly the
    /// recipe `umber::dvi_from_page_plans` uses.
    pub dvi: Vec<u8>,
    /// TeX82 §93 `succumb`'s terminal state, when the job ended through
    /// §81's `jump_out` instead of running to `\end`.
    ///
    /// A fatal error is deliberately *not* an `Err` here. `Err` means the
    /// runner could not produce a run at all, and such a run is unprojectable
    /// on purpose so an engine crash can never be mistaken for a fixture
    /// outcome. A fatal error is the opposite: the engine reached a defined
    /// TeX82 terminal state and the job is over, which is an observable
    /// semantic fact that a fixture must be able to pin.
    pub fatal: Option<FatalError>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ExpectationError {
    PassMismatch(MismatchFingerprint),
    Xpass,
    ChangedFailure {
        pinned: Box<MismatchFingerprint>,
        observed: Box<MismatchFingerprint>,
    },
}

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn property_owners(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let shards = root.join("tests/tex82-properties/shards");
    let mut paths = fs::read_dir(&shards)
        .map_err(|error| format!("{}: {error}", shards.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", shards.display()))?;
    paths.sort();
    let mut owners = BTreeMap::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let shard: PropertyShard = read_json(&path)?;
        for property in shard.properties {
            if let Some(first) = owners.insert(property.id.clone(), shard.domain.clone()) {
                return Err(format!(
                    "property {} is owned by both {first} and {}",
                    property.id, shard.domain
                ));
            }
        }
    }
    Ok(owners)
}

pub fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

pub fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub fn valid_bug_id(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix(BUG_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix.split('.').all(|component| {
            !component.is_empty()
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

pub fn validate_expectation(expectation: &Expectation) -> Result<(), String> {
    let Expectation::Xfail { bug, mismatch } = expectation else {
        return Ok(());
    };
    if !valid_bug_id(bug) {
        return Err(format!(
            "xfail bug must be a concrete {BUG_PREFIX}<slug>[.<slug>] id"
        ));
    }
    if !matches!(
        mismatch.kind.as_str(),
        "observation" | "missing" | "extra" | "execution"
    ) {
        return Err("xfail mismatch kind is not canonical".into());
    }
    if mismatch.expected.is_empty() || mismatch.actual.is_empty() {
        return Err("xfail mismatch must pin nonempty expected and actual values".into());
    }
    Ok(())
}

/// The committed expectation file for one stream channel of one case.
#[must_use]
pub fn channel_file(domain_dir: &Path, case_id: &str, channel: StreamChannel) -> PathBuf {
    domain_dir
        .join("expected")
        .join(format!("{case_id}.{}", channel.name()))
}

/// Requires that a case account for every channel its run can produce.
///
/// A missing block is a validation failure rather than a defaulted-empty
/// contract on purpose. Defaulting would let a case that ships a page or
/// writes a log read as covered, which is the exact failure this contract
/// exists to remove.
pub fn validate_channels(case: &Case, domain_dir: &Path) -> Result<(), String> {
    let Some(channels) = &case.channels else {
        // A case whose engine run does not complete has no channels to
        // record, and inventing a contract for it would be a fiction. The
        // exemption is granted only to a case already pinned as `xfail`, so
        // it expires with the bug: fixing the run makes the contract
        // mandatory, and a passing case can never reach this arm.
        if let Expectation::Xfail { bug, .. } = &case.expectation {
            return valid_bug_id(bug).then_some(()).ok_or_else(|| {
                format!(
                    "case {} omits its channel contract under malformed bug {bug:?}",
                    case.id
                )
            });
        }
        return Err(format!(
            "case {} declares no channel contract; every case must declare \
             events, status, terminal, log, dvi, and effects",
            case.id
        ));
    };
    if channels.status != "clean" && !channels.status.starts_with("fatal:") {
        return Err(format!(
            "case {} declares channel status {:?}, expected \"clean\" or \"fatal:<label>\"",
            case.id, channels.status
        ));
    }
    for channel in STREAM_CHANNELS {
        let declared = channels.stream(channel);
        let path = channel_file(domain_dir, &case.id, channel);
        let present = path.exists();
        match declared {
            StreamDisposition::Empty if present => {
                return Err(format!(
                    "case {} declares channel {} empty but commits {}",
                    case.id,
                    channel.name(),
                    path.display()
                ));
            }
            StreamDisposition::File { .. } | StreamDisposition::Xfail { .. } if !present => {
                return Err(format!(
                    "case {} declares channel {} committed but {} is absent",
                    case.id,
                    channel.name(),
                    path.display()
                ));
            }
            _ => {}
        }
        if let StreamDisposition::Xfail { bug, mismatch, .. } = declared {
            validate_xfail_disposition(channel, bug, mismatch)
                .map_err(|error| format!("case {}: {error}", case.id))?;
        }
    }
    Ok(())
}

/// Enforces the minifixture's size ceilings on an already-read source.
///
/// Kept separate from the file read in [`validate_case`] so both ceilings are
/// plain, unit-testable checks over already-known lengths rather than logic
/// entangled with the filesystem.
fn validate_source_dimensions(id: &str, byte_len: usize, line_count: usize) -> Result<(), String> {
    if byte_len == 0 || byte_len as u64 > MAX_SOURCE_BYTES {
        return Err(format!(
            "case {id} source must be 1..={MAX_SOURCE_BYTES} bytes"
        ));
    }
    if line_count > MAX_SOURCE_LINES {
        return Err(format!(
            "case {id} source must be at most {MAX_SOURCE_LINES} lines"
        ));
    }
    Ok(())
}

/// Rejects a minifixture source that would load a format or macro package.
///
/// A command-semantic minifixture is truly minimal: it loads no format and no
/// macro package, so it may not reference `plain.tex` or `\input plain`, and it
/// may not `\input` a file the case does not declare in its `inputs` map.
///
/// Two committed cases legitimately `\input` a companion file:
/// `input-expansion/input-start-file` (`\input nested`) and
/// `input-expansion/input-level-lifecycle` (`\input child.tex`). Both targets
/// are declared in those cases' `inputs` maps, so the undeclared-target check
/// below still passes them; nothing exempts them by name.
///
/// `\dump` is deliberately *not* forbidden. It writes a format rather than
/// loading one, so it does not make a fixture less minimal, and
/// `main-control/final-cleanup-end-or-dump` exists precisely to exercise tex.web
/// §1335's rejection of it. Forbidding it would have required an exception
/// carved to fit that one source, which is the shape of rule that stops meaning
/// anything. What keeps a fixture from assembling a format is the
/// undeclared-`\input` check below, and that applies to every case alike.
fn validate_no_format_loading(case: &Case, source: &str) -> Result<(), String> {
    if source.contains("plain.tex") {
        return Err(format!(
            "case {} source references plain.tex, which loads a format or package",
            case.id
        ));
    }
    if source.contains("\\input plain") {
        return Err(format!(
            "case {} source uses \\input plain, which loads a format",
            case.id
        ));
    }
    for target in input_targets(source) {
        if !case.inputs.contains_key(&target) {
            return Err(format!(
                "case {} uses \\input {target:?}, which is not declared in this case's inputs map",
                case.id
            ));
        }
    }
    Ok(())
}

/// Extracts every `\input` control word's TeX-normalized file-name argument.
///
/// This mirrors `\input`'s own file-name scanning closely enough for this
/// corpus's purposes: it skips the control word, skips the blanks a control
/// word always absorbs, then reads the run of non-blank,
/// non-control-sequence characters that follows as the file name, appending
/// `.tex` when the name has no extension of its own -- exactly the extension
/// `\input nested` picks up. A longer control word such as `\inputlineno` is
/// left alone.
fn input_targets(source: &str) -> Vec<String> {
    const KEYWORD: &str = "\\input";
    let bytes = source.as_bytes();
    let mut targets = Vec::new();
    let mut search_from = 0;
    while let Some(offset) = source[search_from..].find(KEYWORD) {
        let start = search_from + offset;
        let after_keyword = start + KEYWORD.len();
        if bytes
            .get(after_keyword)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            // A longer control word, e.g. `\inputlineno`; not `\input`.
            search_from = after_keyword;
            continue;
        }
        let mut cursor = after_keyword;
        while bytes.get(cursor) == Some(&b' ') {
            cursor += 1;
        }
        let name_start = cursor;
        while bytes.get(cursor).is_some_and(|byte| {
            !byte.is_ascii_whitespace() && *byte != b'\\' && *byte != b'%' && *byte != b'{'
        }) {
            cursor += 1;
        }
        let mut name = source[name_start..cursor].to_string();
        if !name.is_empty() {
            if !name.contains('.') {
                name.push_str(".tex");
            }
            targets.push(name);
        }
        search_from = cursor.max(after_keyword + 1);
    }
    targets
}

pub fn validate_case(
    case: &Case,
    property_domain: &str,
    domain_dir: &Path,
    root: &Path,
    owners: &BTreeMap<String, String>,
    policy: ChannelPolicy,
) -> Result<(), String> {
    if !valid_slug(&case.id) {
        return Err(format!("case id {:?} is not a lower-kebab slug", case.id));
    }
    if policy == ChannelPolicy::Required {
        validate_channels(case, domain_dir)?;
    }
    match owners.get(&case.property_id) {
        Some(owner) if owner == property_domain => {}
        Some(owner) => {
            return Err(format!(
                "case {} claims {} owned by domain {owner}",
                case.id, case.property_id
            ));
        }
        None => {
            return Err(format!(
                "case {} claims unowned property {}",
                case.id, case.property_id
            ));
        }
    }
    let source = Path::new(&case.source);
    if !is_safe_relative(source)
        || source.components().count() != 1
        || source.extension().and_then(|value| value.to_str()) != Some("tex")
    {
        return Err(format!(
            "case {} has unsafe source {:?}",
            case.id, case.source
        ));
    }
    let source_path = domain_dir.join(source);
    let source_bytes =
        fs::read(&source_path).map_err(|error| format!("{}: {error}", source_path.display()))?;
    let byte_len = source_bytes.len();
    let source_text = String::from_utf8(source_bytes)
        .map_err(|error| format!("case {} source is not valid UTF-8: {error}", case.id))?;
    validate_source_dimensions(&case.id, byte_len, source_text.lines().count())?;
    validate_no_format_loading(case, &source_text)?;
    if case.provenance.authority.is_empty() {
        return Err(format!("case {} has empty canonical authority", case.id));
    }
    let provenance_manifest = Path::new(&case.provenance.manifest);
    if !is_safe_relative(provenance_manifest) || !root.join(provenance_manifest).is_file() {
        return Err(format!(
            "case {} has missing or unsafe provenance manifest {:?}",
            case.id, case.provenance.manifest
        ));
    }
    if case.provenance.sections.is_empty()
        || case.provenance.sections.contains(&0)
        || !case
            .provenance
            .sections
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    {
        return Err(format!(
            "case {} sections must be nonzero, sorted, and unique",
            case.id
        ));
    }
    if case.expected.is_empty()
        || case
            .expected
            .iter()
            .any(|observation| observation.is_empty() || observation.len() > 256)
    {
        return Err(format!(
            "case {} needs short, nonempty expected observations",
            case.id
        ));
    }
    if case
        .projection
        .count_registers
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(format!(
            "case {} count registers must be sorted and unique",
            case.id
        ));
    }
    if case.projection.kind == ProjectionKind::Observations {
        if case.projection.kinds.is_empty() {
            return Err(format!(
                "case {} observations projection needs at least one kind",
                case.id
            ));
        }
        if case
            .projection
            .kinds
            .iter()
            .enumerate()
            .any(|(index, kind)| case.projection.kinds[..index].contains(kind))
        {
            return Err(format!("case {} observation kinds must be unique", case.id));
        }
        if case
            .projection
            .commands
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(format!(
                "case {} commands must be sorted and unique",
                case.id
            ));
        }
    } else if !case.projection.kinds.is_empty() || !case.projection.commands.is_empty() {
        return Err(format!(
            "case {} selects observations outside the observations projection",
            case.id
        ));
    }
    let has_execution_selector = !case.projection.command_names.is_empty()
        || !case.projection.box_registers.is_empty()
        || case.projection.include_mode_transitions
        || case.projection.include_artifact_hashes;
    if case.projection.kind == ProjectionKind::ExecutionBoundaries {
        if !has_execution_selector {
            return Err(format!(
                "case {} execution-boundaries projection has no boundary selector",
                case.id
            ));
        }
        if case
            .projection
            .command_names
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
            || case
                .projection
                .box_registers
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || case.projection.command_names.iter().any(String::is_empty)
            || case.projection.node_depth.is_some_and(|depth| depth > 8)
        {
            return Err(format!(
                "case {} execution-boundary selectors are invalid",
                case.id
            ));
        }
    } else if has_execution_selector || case.projection.node_depth.is_some() {
        return Err(format!(
            "case {} selects execution boundaries outside that projection",
            case.id
        ));
    }
    if case.projection.kind == ProjectionKind::TerminalChecks {
        if case.projection.terminal_checks.is_empty()
            || case
                .projection
                .terminal_checks
                .iter()
                .enumerate()
                .any(|(index, check)| case.projection.terminal_checks[..index].contains(check))
            || case.projection.terminal_checks.iter().any(|check| {
                check.is_empty()
                    || check.contains('\n')
                    || check.contains('\r')
                    || check.len() > 128
            })
        {
            return Err(format!(
                "case {} terminal checks must be short, nonempty, and unique",
                case.id
            ));
        }
    } else if !case.projection.terminal_checks.is_empty() {
        return Err(format!(
            "case {} selects terminal checks outside that projection",
            case.id
        ));
    }
    if case.terminal_lines.iter().any(|line| {
        line.contains("\n") || line.contains("\r") || line.len() > MAX_SOURCE_BYTES as usize
    }) {
        return Err(format!("case {} has an invalid terminal line", case.id));
    }
    if case.inputs.iter().any(|(name, bytes)| {
        !is_safe_relative(Path::new(name))
            || name.is_empty()
            || bytes.is_empty()
            || bytes.len() > MAX_SOURCE_BYTES as usize
    }) {
        return Err(format!("case {} has an invalid named input", case.id));
    }
    if case.font_inputs.iter().any(|(name, source)| {
        !is_safe_relative(Path::new(name))
            || name.is_empty()
            || !is_safe_relative(Path::new(source))
            || !root.join(source).is_file()
    }) {
        return Err(format!(
            "case {} has an invalid declarative font input",
            case.id
        ));
    }
    // A case that declares a non-default interaction mode is declaring that
    // its channels are not comparable to `scripts/run-minifixture-oracle.sh`'s
    // standard scrollmode sweep the way every other case's are, and that fact
    // must be visible in the corpus rather than left to be rediscovered from
    // a confusing diff. A required, nonempty note is the chosen mechanism
    // (over silent documentation or a separate validator) because it keeps
    // the explanation in the one place a reader of the manifest will already
    // be looking, and [`validate_case`] enforces it exactly like every other
    // committed invariant here rather than trusting it to stay written.
    match (
        case.interaction_mode == CaseInteractionMode::default(),
        case.interaction_mode_note.as_deref().unwrap_or(""),
    ) {
        (false, "") => {
            return Err(format!(
                "case {} declares interaction_mode {:?} but no interaction_mode_note \
                 explaining why its channels are not comparable to the standard scrollmode \
                 oracle sweep",
                case.id, case.interaction_mode
            ));
        }
        (true, note) if !note.is_empty() => {
            return Err(format!(
                "case {} has an interaction_mode_note but interaction_mode is the default \
                 scrollmode",
                case.id
            ));
        }
        _ => {}
    }
    validate_expectation(&case.expectation).map_err(|error| format!("case {}: {error}", case.id))
}
pub fn claim_case_identity(
    case_ids: &mut BTreeSet<String>,
    sources: &mut BTreeSet<String>,
    declared_sources: &mut BTreeSet<String>,
    domain: &str,
    id: &str,
    source: &str,
) -> Result<(), String> {
    let case_key = format!("{domain}:{id}");
    if !case_ids.insert(case_key.clone()) {
        return Err(format!("duplicate case {case_key}"));
    }
    let source_key = format!("{domain}:{source}");
    if !sources.insert(source_key.clone()) || !declared_sources.insert(source.into()) {
        return Err(format!("duplicate case source {source_key}"));
    }
    Ok(())
}

/// Loads and fully validates the committed corpus.
pub fn load_suite() -> Result<Vec<DeclaredCase>, String> {
    load_suite_with(ChannelPolicy::Required)
}

/// Whether loading requires each case to already declare its channel contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelPolicy {
    /// The gate's policy: a case without a channel contract is invalid.
    Required,
    /// The regeneration path's policy, used only while deriving the contract
    /// a manifest does not yet carry.
    Deriving,
}

/// Loads the committed corpus under an explicit channel policy.
pub fn load_suite_with(policy: ChannelPolicy) -> Result<Vec<DeclaredCase>, String> {
    let root = repository_root();
    let owners = property_owners(&root)?;
    let corpus = root.join("tests/corpus/command-semantic");
    let mut domain_dirs = fs::read_dir(&corpus)
        .map_err(|error| format!("{}: {error}", corpus.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", corpus.display()))?;
    domain_dirs.sort();

    let mut declared = Vec::new();
    let mut case_ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for domain_dir in domain_dirs {
        if !domain_dir.is_dir() {
            if domain_dir.file_name().and_then(|value| value.to_str())
                == Some("manifest.schema.json")
            {
                continue;
            }
            return Err(format!("unowned corpus entry {}", domain_dir.display()));
        }
        let directory_name = domain_dir
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("non-UTF-8 domain directory {}", domain_dir.display()))?;
        let manifest_path = domain_dir.join("manifest.json");
        let manifest: DomainManifest = read_json(&manifest_path)?;
        if manifest.schema != SCHEMA {
            return Err(format!(
                "{} has schema {}, expected {SCHEMA}",
                manifest_path.display(),
                manifest.schema
            ));
        }
        if manifest.domain != directory_name || !valid_slug(&manifest.domain) {
            return Err(format!(
                "{} domain {:?} does not own directory {directory_name}",
                manifest_path.display(),
                manifest.domain
            ));
        }
        let property_domain = manifest
            .property_domain
            .as_deref()
            .unwrap_or(&manifest.domain);
        if !valid_slug(property_domain) {
            return Err(format!(
                "{} property domain {:?} is not a lower-kebab slug",
                manifest_path.display(),
                property_domain
            ));
        }
        if manifest.cases.is_empty() {
            return Err(format!("{} declares no cases", manifest_path.display()));
        }
        let mut declared_sources = BTreeSet::new();
        for case in manifest.cases {
            validate_case(&case, property_domain, &domain_dir, &root, &owners, policy)?;
            claim_case_identity(
                &mut case_ids,
                &mut sources,
                &mut declared_sources,
                &manifest.domain,
                &case.id,
                &case.source,
            )?;
            declared.push(DeclaredCase {
                domain_dir: domain_dir.clone(),
                domain: manifest.domain.clone(),
                case,
            });
        }
        let mut fixture_sources = fs::read_dir(&domain_dir)
            .map_err(|error| format!("{}: {error}", domain_dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{}: {error}", domain_dir.display()))?;
        fixture_sources.sort();
        for fixture in fixture_sources {
            if fixture.extension().and_then(|value| value.to_str()) == Some("tex") {
                let name = fixture
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| format!("non-UTF-8 source {}", fixture.display()))?;
                if !declared_sources.contains(name) {
                    return Err(format!("unowned fixture source {}", fixture.display()));
                }
            }
        }
    }
    if declared.is_empty() {
        return Err("command semantic corpus declares no cases".into());
    }
    Ok(declared)
}

pub fn execute(source: &[u8], case: &Case) -> Result<SemanticRun, String> {
    let mut universe = Universe::new();
    for line in &case.terminal_lines {
        universe
            .world_mut()
            .push_memory_terminal_line(line.clone())
            .map_err(|error| format!("terminal line registration: {error}"))?;
    }
    let mut control = match case.profile {
        SessionProfile::Initex => CanonicalMainControl::tex82_initex(&mut universe),
        SessionProfile::EtexInitex => {
            let _tex82_registry = CanonicalMainControl::tex82_initex(&mut universe);
            tex_command::install_etex_expandable_primitives(&mut universe);
            tex_exec::install_etex_unexpandable_primitives(&mut universe);
            CanonicalMainControl::prepared_initex(CommandProfile::ETEX26)
        }
        SessionProfile::EtexLoaded => {
            let _tex82_registry = CanonicalMainControl::tex82_initex(&mut universe);
            tex_command::install_etex_expandable_primitives(&mut universe);
            tex_exec::install_etex_unexpandable_primitives(&mut universe);
            // Bounded format-loaded macro identity probe. TeX82 §§341/1221
            // expose the `def_ref` head after §1309's format memory compaction
            // removes the unreachable frozen `\endwrite` definition.
            let empty = universe.intern_token_list(&[]);
            let relax = universe.intern("relax");
            let replacement = universe.intern_token_list(&[Token::Cs(relax.symbol())]);
            let format_macro =
                universe.intern_macro(MacroMeaning::new(MeaningFlags::EMPTY, empty, replacement));
            let format_macro_symbol = universe.intern("formatmacro");
            universe.set_meaning_global(
                format_macro_symbol,
                Meaning::Macro {
                    flags: MeaningFlags::EMPTY,
                    definition: format_macro,
                },
            );
            // Exercise e-TeX change [50.1307], which resets optional e-TeX
            // state cells immediately before tex.web §1307 dumps `eqtb`.
            universe.set_int_param(tex_state::env::banks::IntParam::TEX_XET_STATE, 1);
            let format = universe
                .dump_format()
                .map_err(|error| format!("e-TeX format creation: {error}"))?;
            universe = Universe::from_format(tex_state::World::memory(), &format)
                .map_err(|error| format!("e-TeX format restore: {error}"))?;
            tex_command::register_tex82_expandable_primitives(&mut universe);
            tex_command::register_etex_expandable_primitives(&mut universe);
            CanonicalMainControl::with_profile(CommandProfile::ETEX26)
        }
        SessionProfile::Production => {
            let _initialized = CanonicalMainControl::tex82_initex(&mut universe);
            CanonicalMainControl::new()
        }
    };
    for (name, bytes) in &case.inputs {
        control.capabilities_mut().register_input(
            name,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(bytes.as_bytes()),
            ),
        );
    }
    for (name, source) in &case.font_inputs {
        let bytes = fs::read(repository_root().join(source))
            .map_err(|error| format!("font fixture read: {error}"))?;
        universe
            .world_mut()
            .set_memory_file(name, bytes)
            .map_err(|error| format!("font fixture registration: {error}"))?;
        let metrics =
            InputReadState::read_input_file(&mut universe.input_open_context(), Path::new(name))
                .map_err(|error| format!("font fixture parsing: {error}"))?;
        control.capabilities_mut().register_font(
            name,
            FontResource::Tfm {
                metrics,
                opentype: None,
            },
        );
    }

    // The oracle this corpus is measured against runs whatever
    // `-interaction` mode the case declares (`Case::interaction_mode`,
    // default `scrollmode`). `scripts/run-minifixture-oracle.sh`'s
    // "Interaction mode" comment works through why `scrollmode` is the
    // default: it is the one mode that both tolerates the `\read`/`\pausing`
    // cases that need `>nonstop_mode` *and* "omits error stops" (tex.web
    // §1749) the way batch/nonstop do, so an error this simulation didn't
    // anticipate still just prints and lets the run finish instead of
    // demanding an unanswerable `?` prompt. A case that needs a real `?`
    // prompt -- `main-control/show-completion` -- declares `errorstopmode`
    // instead, with `interaction_mode_note` recording why (see
    // [`validate_case`]). This is set on the constructed `Universe` -- after
    // the profile match above, because `EtexLoaded`/`Production` replace
    // `universe` with one restored from a dumped format, and a format dump
    // carries no interaction mode.
    universe.set_interaction_mode(case.interaction_mode.engine_mode());

    // `EtexLoaded` and `Production` construct `control` past INITEX
    // (`with_profile`/`new` rather than `tex82_initex`/`prepared_initex`), so
    // `CanonicalMainControl::begin_job`'s `format_ident` -- which only knows
    // how to print INITEX's `" (INITEX)"` -- refuses outright rather than
    // guess a `(preloaded format=...)` name for a format this harness never
    // actually names or dumps to a file (see `job::format_ident`'s doc
    // comment). `scripts/run-minifixture-oracle.sh` documents the same gap
    // from the other side: it cannot reproduce either profile at all, so
    // there is no oracle output these 5 cases could be framed against
    // anyway. The honest choice is to leave them exactly as unframed as they
    // were before this module ran every case as a job, rather than fabricate
    // a banner neither side can check.
    let job_framed = matches!(
        case.profile,
        SessionProfile::Initex | SessionProfile::EtexInitex
    );
    if job_framed {
        // §534/§536/§61: the start-up banner and the `**` line, which must
        // precede the root file's own `(` (see `crate::job`'s doc comment on
        // `begin_job`). `first_line` echoes what the oracle is invoked with
        // on its command line -- the bare source filename, e.g.
        // `show-box.tex`.
        control.begin_job(&mut universe, &case.source);
    }
    let root = SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(source));
    let root = if job_framed {
        // kpathsea resolves a same-directory file through `./`, so pdfTeX's
        // §537 `a_make_name_string` records (and prints) `./show-box.tex`
        // rather than the bare name `begin_job` was just given. Matching
        // that leading `./` is what makes Umber's own `(` line comparable to
        // the oracle's. An unframed run leaves the root unnamed, exactly as
        // before this module existed: an unnamed registration queues no
        // §537 `FileFramingEvent::Open` (see `tex_command::CommandState::
        // push_source_level`), so it prints no orphan `(` without the
        // banner that would normally precede it.
        root.with_name(format!("./{}", case.source))
    } else {
        root
    };
    control
        .register_root_source(root)
        .map_err(|error| format!("source registration: {error:?}"))?;
    let mut recorder = Recorder::default();
    let mut mode_transitions = vec![control.current_mode()];
    for _ in 0..MAX_STEPS {
        let step = control
            .step_with_observer(&mut universe, &mut recorder)
            .map_err(|error| {
                let rendered = format!("{error:?}");
                if rendered.starts_with("Command(InputInvariant(") {
                    "main-control step: Command(InputInvariant)".into()
                } else {
                    format!("main-control step: {rendered}")
                }
            })?;
        let mode = control.current_mode();
        if mode_transitions.last() != Some(&mode) {
            mode_transitions.push(mode);
        }
        match step {
            MainControlStep::Continue => {}
            MainControlStep::End | MainControlStep::EndOfInput => {
                let counts = std::array::from_fn(|slot| {
                    universe.count(
                        u16::try_from(slot).expect("count slot fits in TeX82 register index"),
                    )
                });
                let pages = control.take_prepared_dvi_pages();
                let artifacts: Vec<ContentHash> = pages.iter().map(|page| page.hash()).collect();
                // §1333's `close_files_and_terminate` is reached from both
                // outcomes here, not only `\end`/`\dump`'s `End`: §93's
                // `fatal_error` (raised for `EndOfInput` by
                // `crate::job::print_terminal_exhausted`, TeX82 §362's `*`
                // prompt finding no more terminal input) calls `succumb`,
                // which calls `error` and then `jump_out`s straight past
                // §1335's `final_cleanup` to §1333 -- skipping the paren
                // close and history note `final_cleanup` would have printed,
                // but not the DVI/transcript report itself. A prior
                // `\shipout` can leave `pages` nonempty even when the job
                // never saw `\end`, so this serializes them exactly as the
                // `End` path does.
                let dvi = if !pages.is_empty() {
                    let plans: Vec<_> = pages
                        .into_iter()
                        .map(tex_exec::PreparedDviPage::into_plan)
                        .collect();
                    let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
                    for plan in &plans {
                        writer
                            .write_page_plan(plan)
                            .map_err(|error| format!("dvi page serialization: {error:?}"))?;
                    }
                    writer
                        .finish()
                        .map_err(|error| format!("dvi finish: {error:?}"))?
                } else {
                    Vec::new()
                };
                // §1333's DVI/transcript report is itself framing (it closes
                // out the banner `begin_job` printed), so it is gated on
                // `job_framed` exactly like `begin_job` above -- an unframed
                // run gets neither end.
                if job_framed {
                    let job_name = control.capabilities_mut().job_name().to_owned();
                    let dvi_output = (!dvi.is_empty()).then(|| tex_exec::DviJobOutput {
                        file_name: format!("{job_name}.dvi"),
                        byte_len: dvi.len() as u64,
                    });
                    control.finish_job(&mut universe, dvi_output);
                }
                return Ok(SemanticRun {
                    observations: recorder.0,
                    counts,
                    universe,
                    mode_transitions,
                    artifacts,
                    dvi,
                    fatal: control.fatal_error(),
                });
            }
        }
    }
    Err(format!("exceeded {MAX_STEPS} main-control steps"))
}

pub fn push_counts(run: &SemanticRun, projection: &Projection, output: &mut Vec<String>) {
    output.extend(
        projection
            .count_registers
            .iter()
            .map(|register| format!("count:{register}={}", run.counts[usize::from(*register)])),
    );
}

pub fn mode_name(mode: Mode) -> String {
    match mode {
        Mode::Vertical => "vertical",
        Mode::Horizontal => "horizontal",
        Mode::DisplayMath => "display-math",
        Mode::InternalVertical => "internal-vertical",
        Mode::RestrictedHorizontal => "restricted-horizontal",
        Mode::Math => "math",
    }
    .into()
}

pub fn node_name(node: &Node) -> String {
    match node {
        Node::Char { .. } => "char",
        Node::Lig { .. } => "ligature",
        Node::Kern { .. } => "kern",
        Node::Glue { .. } => "glue",
        Node::Penalty(_) => "penalty",
        Node::Rule { .. } => "rule",
        Node::HList(_) => "hlist",
        Node::VList(_) => "vlist",
        Node::Unset(_) => "unset",
        Node::Disc { .. } => "discretionary",
        Node::Mark { .. } => "mark",
        Node::Ins { .. } => "insertion",
        Node::Whatsit(_) => "whatsit",
        Node::MathOn(_) => "math-on",
        Node::MathOff(_) => "math-off",
        Node::Direction(_) => "direction",
        Node::MathNoad(_) => "math-noad",
        Node::FractionNoad(_) => "fraction-noad",
        Node::MathStyle(_) => "math-style",
        Node::MathChoice(_) => "math-choice",
        Node::MathList(_) => "math-list",
        Node::Nonscript => "nonscript",
        Node::Adjust(_) => "adjust",
    }
    .into()
}

pub fn push_node_outline(
    universe: &Universe,
    list: tex_state::ids::NodeListId,
    prefix: &str,
    depth: u8,
    output: &mut Vec<String>,
) {
    for (index, node) in universe.nodes(list).iter().enumerate() {
        let node = node.to_owned();
        let path = format!("{prefix}/{index}");
        output.push(format!("{path}:{}", node_name(&node)));
        if depth == 0 {
            continue;
        }
        match &node {
            Node::HList(boxed) | Node::VList(boxed) => {
                push_node_outline(universe, boxed.children, &path, depth - 1, output);
            }
            _ => {}
        }
    }
}

pub fn execution_boundaries(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    let mut output = run
        .observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::Command(record) = observation else {
                return None;
            };
            (record.boundary == CommandDeliveryBoundary::Expanded
                && projection.command_names.contains(&record.command))
            .then(|| {
                record.command_operand.map_or_else(
                    || format!("command:{}", record.command),
                    |operand| format!("command:{}:{operand}", record.command),
                )
            })
        })
        .collect::<Vec<_>>();
    for register in &projection.box_registers {
        match run.universe.box_reg(*register) {
            Some(list) => push_node_outline(
                &run.universe,
                list,
                &format!("box:{register}"),
                projection.node_depth.unwrap_or(3),
                &mut output,
            ),
            None => output.push(format!("box:{register}:void")),
        }
    }
    if projection.include_mode_transitions {
        output.extend(
            run.mode_transitions
                .iter()
                .map(|mode| format!("mode:{}", mode_name(*mode))),
        );
    }
    if projection.include_artifact_hashes {
        output.extend(
            run.artifacts
                .iter()
                .map(|artifact| format!("artifact:{}", artifact.hex())),
        );
    }
    output
}

pub fn condition_observations(run: &SemanticRun) -> Vec<String> {
    run.observations
        .iter()
        .filter_map(|observation| {
            let CommandObservation::Condition(record) = observation else {
                return None;
            };
            Some(format!(
                "condition:{}:{}:{}:{}",
                record.transition,
                record.condition,
                record.limit,
                record.branch.as_deref().unwrap_or("-")
            ))
        })
        .collect()
}

pub fn predicate_outcomes(run: &SemanticRun) -> Vec<String> {
    let mut stack = Vec::new();
    let mut active = BTreeMap::<u64, (String, Vec<(&str, String)>)>::new();
    let mut outcomes = Vec::new();
    for observation in &run.observations {
        match observation {
            CommandObservation::Condition(record) if record.transition == "push" => {
                stack.push(record.identity);
                active.insert(record.identity, (record.condition.clone(), Vec::new()));
            }
            CommandObservation::Scanner(record) => {
                if let Some(identity) = stack.last()
                    && let Some((_, scalars)) = active.get_mut(identity)
                {
                    scalars.push((record.kind, record.value.clone()));
                }
            }
            CommandObservation::Condition(record) if record.transition == "branch" => {
                let Some(truth) = record
                    .branch
                    .as_deref()
                    .filter(|value| matches!(*value, "true" | "false"))
                else {
                    continue;
                };
                let (condition, scalars) = active
                    .get(&record.identity)
                    .expect("boolean branch belongs to a pushed condition");
                let scalars = if scalars.is_empty() {
                    "-".into()
                } else {
                    scalars
                        .iter()
                        .map(|(kind, value)| format!("{kind}={value}"))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                outcomes.push(format!("predicate:{condition}:{scalars}:{truth}"));
            }
            CommandObservation::Condition(record) if record.transition == "pop" => {
                if stack.pop() != Some(record.identity) {
                    outcomes.push("invalid:condition-stack-not-lifo".into());
                }
                active.remove(&record.identity);
            }
            _ => {}
        }
    }
    outcomes
}

pub fn observed_token_text(token: &ObservedToken) -> String {
    match token {
        ObservedToken::Character { character, catcode } => format!(
            "char:{}:{}",
            canonical_names::catcode_name(*catcode),
            u32::from(*character)
        ),
        ObservedToken::ControlSequence(name) => format!("cs:{name}"),
        ObservedToken::MacroMatch => "macro-match".into(),
        ObservedToken::MacroEndMatch => "macro-end-match".into(),
        ObservedToken::Parameter(slot) => format!("parameter:{slot}"),
        token => format!(
            "cs:{}",
            canonical_names::observed_token_control_sequence(token)
                .expect("every frozen observed token has canonical control-sequence text")
        ),
    }
}

pub fn observed_tokens_text(tokens: &[ObservedToken]) -> String {
    tokens
        .iter()
        .map(observed_token_text)
        .collect::<Vec<_>>()
        .join(",")
}

/// Returns the canonical schema transition for a command-owned alignment
/// observation.
///
/// Recovery records retain their exact reason internally so the schema
/// translator can populate `recovery`; tex-oracle schema v1 represents all of
/// those reasons with the single `recovery` transition. Semantic fixture
/// filters operate on that canonical transition, not the internal reason.
pub fn canonical_alignment_transition(transition: &str) -> &str {
    match transition {
        // Raw brace delivery retains its command-owned reason internally,
        // while tex-oracle schema v1 exposes TeX82 §309's `align_state`
        // mutation as the canonical `state_change` transition. This is the
        // same lowering performed by the full command-stream translator.
        "begin_group" | "end_group" => "state_change",
        "missing_parameter"
        | "extra_parameter"
        | "missing_left_brace"
        | "missing_right_brace"
        | "extra_tab"
        | "outer_validity" => "recovery",
        transition => transition,
    }
}

pub fn observation_projection(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    let mut output = Vec::new();
    for observation in &run.observations {
        let item = match observation {
            CommandObservation::Command(record)
                if projection.kinds.contains(&ObservationKind::Command)
                    && (projection.commands.is_empty()
                        || projection.commands.contains(&record.command)) =>
            {
                let boundary = match record.boundary {
                    CommandDeliveryBoundary::Raw => "raw",
                    CommandDeliveryBoundary::Expanded => "expanded",
                };
                Some(format!(
                    "command:{boundary}:{}:{}:{}",
                    observed_token_text(&record.spelling),
                    record.command,
                    record
                        .command_operand
                        .map_or_else(|| "-".into(), |operand| operand.to_string())
                ))
            }
            CommandObservation::Input(record)
                if projection.kinds.contains(&ObservationKind::Input) =>
            {
                let transition = match record.transition {
                    InputTransition::Push => "push",
                    InputTransition::Retire => "retire",
                    InputTransition::Stop => "stop",
                    InputTransition::Backup => "backup",
                    InputTransition::Recovery => "recovery",
                };
                let reason = match record.reason {
                    // A source level is named by tex.web §303's `name`
                    // classification, which §307's `token_type` -- the rest of
                    // this table -- cannot express.
                    InputReason::Source => record
                        .source_name
                        .map_or("source", canonical_names::source_name_class_name),
                    InputReason::Backup => "backup",
                    InputReason::Macro => "macro",
                    InputReason::Parameter => "parameter",
                    InputReason::AlignmentUTemplate => "u-template",
                    InputReason::AlignmentVTemplate => "v-template",
                    InputReason::Recovery => "recovery",
                    InputReason::OutputRoutine
                    | InputReason::EveryPar
                    | InputReason::EveryMath
                    | InputReason::EveryDisplay
                    | InputReason::EveryHBox
                    | InputReason::EveryVBox
                    | InputReason::EveryJob
                    | InputReason::EveryCr
                    | InputReason::EveryEof
                    | InputReason::Mark
                    | InputReason::UmberReplay(_) => "token-list",
                    InputReason::Write => "write",
                };
                Some(format!("input:{transition}:{reason}"))
            }
            CommandObservation::Alignment(record)
                if projection.kinds.contains(&ObservationKind::Alignment)
                    && (projection.alignment_transitions.is_empty()
                        || projection.alignment_transitions.iter().any(|transition| {
                            transition == canonical_alignment_transition(record.transition)
                        })) =>
            {
                Some(format!(
                    "alignment:{}:{}",
                    canonical_alignment_transition(record.transition),
                    record.align_state
                ))
            }
            CommandObservation::Recovery(record)
                if projection.kinds.contains(&ObservationKind::Recovery) =>
            {
                let kind = match record.kind {
                    RecoveryKind::Backup => "backup",
                    RecoveryKind::InsertedToken => "inserted-token",
                    RecoveryKind::InsertedControlSequence => "inserted-control-sequence",
                };
                Some(format!(
                    "recovery:{kind}:{}",
                    observed_tokens_text(&record.tokens)
                ))
            }
            CommandObservation::ScannerStatus(record)
                if projection.kinds.contains(&ObservationKind::ScannerStatus) =>
            {
                Some(format!("scanner-status:{}>{}", record.from, record.to))
            }
            CommandObservation::Macro(record)
                if projection.kinds.contains(&ObservationKind::Macro) =>
            {
                Some(format!(
                    "macro:{}:{}:{}:{}",
                    if record.activation {
                        "activate"
                    } else {
                        "argument"
                    },
                    record.control_sequence.as_deref().unwrap_or("-"),
                    record
                        .argument
                        .map_or_else(|| "-".into(), |slot| slot.to_string()),
                    observed_tokens_text(&record.tokens)
                ))
            }
            CommandObservation::Scanner(record)
                if projection.kinds.contains(&ObservationKind::Scanner) =>
            {
                Some(format!(
                    "scanner:{}:{}:{}",
                    record.kind,
                    record.value,
                    record
                        .tokens
                        .as_deref()
                        .map_or_else(|| "-".into(), observed_tokens_text)
                ))
            }
            CommandObservation::TokenList(record)
                if projection.kinds.contains(&ObservationKind::TokenList) =>
            {
                Some(format!(
                    "token-list:{}:{}:{}",
                    record.transition,
                    record.purpose,
                    observed_tokens_text(&record.tokens)
                ))
            }
            CommandObservation::Mutation(record)
                if projection.kinds.contains(&ObservationKind::Mutation) =>
            {
                Some(format!(
                    "mutation:{}:{}:{}:{}",
                    record.target,
                    record.key.as_deref().unwrap_or("-"),
                    record.value,
                    if record.global { "global" } else { "local" }
                ))
            }
            CommandObservation::Diagnostic(record)
                if projection.kinds.contains(&ObservationKind::Diagnostic) =>
            {
                let arguments = record
                    .arguments
                    .iter()
                    .map(|argument| match argument {
                        DiagnosticArgument::Token(token) => observed_token_text(token),
                        DiagnosticArgument::Name(name) => format!("name:{name}"),
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                Some(format!(
                    "diagnostic:{}:{}:{arguments}",
                    record.severity, record.diagnostic
                ))
            }
            CommandObservation::Effect(record)
                if projection.kinds.contains(&ObservationKind::Effect) =>
            {
                Some(format!("effect:{}:{}", record.kind, record.detail))
            }
            _ => None,
        };
        if let Some(item) = item {
            output.push(item);
        }
    }
    output
}

pub fn captured_terminal_text(run: &SemanticRun) -> String {
    let committed = run
        .universe
        .world()
        .memory_terminal_output()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default();
    let pending: String = run
        .universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|effect| match effect {
            EffectRecord::StreamWrite {
                sink: PrintSink::Terminal | PrintSink::TerminalAndLog | PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    committed + &pending
}

pub fn terminal_check_results(output: &str, checks: &[String]) -> Vec<String> {
    checks
        .iter()
        .map(|check| format!("terminal-check:{check}={}", output.contains(check)))
        .collect()
}

pub fn terminal_check_projection(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    terminal_check_results(&captured_terminal_text(run), &projection.terminal_checks)
}

/// Projects one completed run.
///
/// A fatal termination is a property of the *run*, not of any one projection
/// kind, so its marker is emitted for every kind, ahead of everything else. A
/// case whose engine now dies where it used to finish therefore gains a line
/// its `expected` array must declare; it can never quietly lose one.
pub fn project(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    let mut output: Vec<String> = run
        .fatal
        .map(|fatal| format!("execution:error:{}", fatal.label()))
        .into_iter()
        .collect();
    output.extend(match projection.kind {
        ProjectionKind::Classification => {
            let mut output = Vec::new();
            for command in ["if_test", "fi_or_else"] {
                let mut operands = Vec::new();
                for observation in &run.observations {
                    let CommandObservation::Command(record) = observation else {
                        continue;
                    };
                    if record.command == command
                        && let Some(operand) = record.command_operand
                        && !operands.contains(&operand)
                    {
                        operands.push(operand);
                    }
                }
                output.extend(
                    operands
                        .into_iter()
                        .map(|operand| format!("command:{command}:{operand}")),
                );
            }
            output.extend(run.observations.iter().filter_map(|observation| {
                let CommandObservation::Recovery(record) = observation else {
                    return None;
                };
                if record.kind != RecoveryKind::InsertedToken {
                    return None;
                }
                match record.tokens.as_slice() {
                    [ObservedToken::ControlSequence(name)] => {
                        Some(format!("recovery:inserted-token:cs:{name}"))
                    }
                    _ => None,
                }
            }));
            output
        }
        ProjectionKind::ConditionSteps | ProjectionKind::SkippingConditionSteps => {
            condition_observations(run)
        }
        ProjectionKind::BranchSelections => {
            run.observations
                .iter()
                .filter_map(|observation| {
                    let CommandObservation::Condition(record) = observation else {
                        return None;
                    };
                    record.branch.as_ref().map(|branch| {
                        format!("branch:{}:{}:{branch}", record.condition, record.limit)
                    })
                })
                .collect()
        }
        ProjectionKind::PredicateOutcomes => predicate_outcomes(run),
        ProjectionKind::Observations => observation_projection(run, projection),
        ProjectionKind::TerminalChecks => terminal_check_projection(run, projection),
        ProjectionKind::ExecutionBoundaries => execution_boundaries(run, projection),
        ProjectionKind::State => Vec::new(),
    });
    if projection.include_count_mutations {
        output.extend(run.observations.iter().filter_map(|observation| {
            let CommandObservation::Mutation(record) = observation else {
                return None;
            };
            record
                .value
                .starts_with("count:")
                .then(|| format!("mutation:{}", record.value))
        }));
    }
    if matches!(projection.kind, ProjectionKind::SkippingConditionSteps) {
        output.extend(run.observations.iter().filter_map(|observation| {
            let CommandObservation::ScannerStatus(record) = observation else {
                return None;
            };
            (record.from == "skipping" || record.to == "skipping")
                .then(|| format!("scanner-status:{}>{}", record.from, record.to))
        }));
    }
    push_counts(run, projection, &mut output);
    output
}

pub fn first_mismatch(expected: &[String], actual: &[String]) -> Option<MismatchFingerprint> {
    let common = expected.len().min(actual.len());
    for index in 0..common {
        if expected[index] != actual[index] {
            return Some(MismatchFingerprint {
                index,
                kind: "observation".into(),
                expected: expected[index].clone(),
                actual: actual[index].clone(),
            });
        }
    }
    if expected.len() > common {
        Some(MismatchFingerprint {
            index: common,
            kind: "missing".into(),
            expected: expected[common].clone(),
            actual: "<end>".into(),
        })
    } else if actual.len() > common {
        Some(MismatchFingerprint {
            index: common,
            kind: "extra".into(),
            expected: "<end>".into(),
            actual: actual[common].clone(),
        })
    } else {
        None
    }
}

pub fn observed_mismatch(
    expected: &[String],
    actual: &Result<Vec<String>, String>,
) -> Option<MismatchFingerprint> {
    match actual {
        Ok(actual) => first_mismatch(expected, actual),
        Err(error) => Some(MismatchFingerprint {
            index: 0,
            kind: "execution".into(),
            expected: expected.first().map_or("<end>", String::as_str).into(),
            actual: error.clone(),
        }),
    }
}

pub fn evaluate_expectation(
    expected: &[String],
    actual: &Result<Vec<String>, String>,
    expectation: &Expectation,
) -> Result<(), ExpectationError> {
    let mismatch = observed_mismatch(expected, actual);
    match (expectation, mismatch) {
        (Expectation::Pass, None) => Ok(()),
        (Expectation::Pass, Some(mismatch)) => Err(ExpectationError::PassMismatch(mismatch)),
        (Expectation::Xfail { .. }, None) => Err(ExpectationError::Xpass),
        (Expectation::Xfail { mismatch, .. }, Some(observed)) if *mismatch == observed => Ok(()),
        (Expectation::Xfail { mismatch, .. }, Some(observed)) => {
            Err(ExpectationError::ChangedFailure {
                pinned: Box::new(mismatch.clone()),
                observed: Box::new(observed),
            })
        }
    }
}

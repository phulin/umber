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

use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile,
    DiagnosticArgument, FatalError, FontResource, InputReason, InputTransition, ObservedToken,
    RecoveryKind, RegisteredSourceKind, SourceRegistration, canonical_names,
};
use tex_exec::{MainControl, MainControlStep, Mode};
use tex_state::{ContentHash, InputReadState, node::NodeKind};

pub mod channels;
pub mod classify;
#[cfg(test)]
mod tests;

pub use channels::{
    CapturedChannels, ChannelContract, ChannelFailure, ChannelMismatch, EffectArtifact,
    STREAM_CHANNELS, StreamChannel, StreamDisposition, first_line_difference,
    first_line_difference_in, normalize_channel, normalize_log_clock, portable_effect_channel,
    split_channel_lines, strip_diagnostic_reports, validate_xfail_diagnostics_disposition,
    validate_xfail_disposition,
};
pub use classify::{DivergenceClass, classify_divergence, reclassify_no_error_channel};

pub const SCHEMA: u32 = 2;
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseManifestV2 {
    pub schema: u32,
    pub property_id: String,
    #[serde(default)]
    pub profile: SessionProfile,
    #[serde(default)]
    pub capture: CapturePolicy,
    #[serde(default)]
    pub font_inputs: BTreeMap<String, String>,
    pub provenance: Provenance,
    pub projection: Projection,
    pub expected: Vec<String>,
    #[serde(default)]
    pub expectation: Expectation,
    #[serde(default)]
    pub channels: Option<ChannelContractV2>,
    #[serde(default)]
    pub terminal_lines: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    #[serde(default)]
    pub interaction_mode: CaseInteractionMode,
    #[serde(default)]
    pub interaction_mode_note: Option<String>,
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
    /// The canonical projection a real run must produce.
    ///
    /// Parsed as possibly-empty only so the regeneration path can read a
    /// manifest it is about to derive this into for a brand-new `pass`
    /// case (see [`ChannelPolicy`], which this shares with `channels`
    /// below): [`validate_case`] requires it nonempty for every other case,
    /// exactly as it required `channels` before that block could be
    /// derived. A `pass` case's `expected` *is* mechanically derivable --
    /// passing means the run's own projection matches it exactly -- but an
    /// `xfail` case's is not: it names the still-uncorrected divergence's
    /// position, which is definitionally not what the run currently
    /// produces, so it stays hand-authored and this field is never treated
    /// as empty-and-derivable for one.
    #[serde(default)]
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
    #[serde(default)]
    pub capture: CapturePolicy,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub authority: String,
    pub manifest: String,
    pub sections: Vec<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionProfile {
    #[default]
    Initex,
    EtexInitex,
    EtexLoaded,
    Production,
    RawTex82Loaded,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CapturePolicy {
    #[default]
    Profile,
    Exclude {
        reason: String,
    },
}

impl CapturePolicy {
    #[must_use]
    pub const fn selected(&self) -> bool {
        matches!(self, Self::Profile)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionRoute {
    Fresh,
    ProductionPdftex14029Loaded,
    RawEtex26Loaded,
    RawTex82Loaded,
}

impl SessionProfile {
    #[must_use]
    pub const fn execution_route(self) -> ExecutionRoute {
        match self {
            Self::EtexLoaded => ExecutionRoute::RawEtex26Loaded,
            Self::Production => ExecutionRoute::ProductionPdftex14029Loaded,
            Self::RawTex82Loaded => ExecutionRoute::RawTex82Loaded,
            Self::Initex | Self::EtexInitex => ExecutionRoute::Fresh,
        }
    }
}

/// tex.web's four `-interaction` modes, spelled exactly as pdfTeX's own flag
/// does (`-interaction=scrollmode`, ...) so a case's declared value can be
/// handed straight to the oracle runner's command line.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq)]
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
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Expectation {
    #[default]
    Pass,
    Xfail {
        bug: String,
        mismatch: MismatchFingerprint,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MismatchFingerprint {
    pub index: usize,
    pub kind: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelContractV2 {
    pub events: usize,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub terminal: Option<StreamDisposition>,
    #[serde(default)]
    pub log: Option<StreamDisposition>,
    #[serde(default)]
    pub dvi: Option<StreamDisposition>,
    #[serde(default)]
    pub effects: Option<StreamDisposition>,
}

impl CaseManifestV2 {
    fn resolve(self, fixture_dir: &Path, id: String) -> Case {
        let disposition = |channel| {
            if channel_file(fixture_dir, channel).is_file() {
                StreamDisposition::File
            } else {
                StreamDisposition::Empty
            }
        };
        let channels = self.channels.map(|channels| ChannelContract {
            events: channels.events,
            status: channels.status.unwrap_or_else(|| "clean".to_owned()),
            terminal: channels
                .terminal
                .unwrap_or_else(|| disposition(StreamChannel::Terminal)),
            log: channels
                .log
                .unwrap_or_else(|| disposition(StreamChannel::Log)),
            dvi: channels
                .dvi
                .unwrap_or_else(|| disposition(StreamChannel::Dvi)),
            effects: channels
                .effects
                .unwrap_or_else(|| disposition(StreamChannel::Effects)),
        });
        Case {
            source: format!("{id}.tex"),
            id,
            property_id: self.property_id,
            profile: self.profile,
            font_inputs: self.font_inputs,
            provenance: self.provenance,
            projection: self.projection,
            expected: self.expected,
            expectation: self.expectation,
            channels,
            terminal_lines: self.terminal_lines,
            inputs: self.inputs,
            interaction_mode: self.interaction_mode,
            interaction_mode_note: self.interaction_mode_note,
            capture: self.capture,
        }
    }
}

/// Structural JSON Schema generated from the V2 Rust manifest contract.
#[must_use]
pub fn manifest_schema() -> serde_json::Value {
    serde_json::to_value(schema_for!(CaseManifestV2))
        .expect("a generated JSON Schema always serializes")
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
    pub fixture_dir: PathBuf,
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
    pub box_outlines: BTreeMap<u16, Option<Vec<umber::DetachedNodeOutlineEntry>>>,
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
    /// Already materialized terminal and log prefixes, detached before the
    /// generation retires.
    pub terminal: Vec<u8>,
    pub log: Vec<u8>,
    /// Pending effect suffix needed to complete terminal/log routing for a
    /// root-EOF fragment.
    pub pending_effects: Vec<tex_state::EffectRecord>,
    /// Numbered stream outputs materialized by a complete job.
    pub effect_artifacts: Vec<EffectArtifact>,
    /// Complete-job bytes used only by the reference-derived stream-channel
    /// contract. The other fields are the authored-fragment property
    /// projection, which stops at root EOF without inventing `\end`.
    pub complete_job_channel_streams: Option<[Vec<u8>; 4]>,
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
    test_support::repository_root()
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
pub fn channel_file(fixture_dir: &Path, channel: StreamChannel) -> PathBuf {
    fixture_dir.join(format!("expected.{}", channel.name()))
}

/// Requires that a case account for every channel its run can produce.
///
/// A missing block is a validation failure rather than a defaulted-empty
/// contract on purpose. Defaulting would let a case that ships a page or
/// writes a log read as covered, which is the exact failure this contract
/// exists to remove.
pub fn validate_channels(case: &Case, fixture_dir: &Path) -> Result<(), String> {
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
        let path = channel_file(fixture_dir, channel);
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
            StreamDisposition::File
            | StreamDisposition::Xfail { .. }
            | StreamDisposition::XfailDiagnostics { .. }
                if !present =>
            {
                return Err(format!(
                    "case {} declares channel {} committed but {} is absent",
                    case.id,
                    channel.name(),
                    path.display()
                ));
            }
            _ => {}
        }
        if let StreamDisposition::Unsupported { reason } = declared {
            if channel != StreamChannel::Effects {
                return Err(format!(
                    "case {} declares channel {} unsupported; only effects can lack a portable oracle projection",
                    case.id,
                    channel.name()
                ));
            }
            if reason.trim().is_empty() {
                return Err(format!(
                    "case {} declares effects unsupported without a reason",
                    case.id
                ));
            }
            if present {
                return Err(format!(
                    "case {} declares effects unsupported but commits {}; unsupported evidence has no expected bytes",
                    case.id,
                    path.display()
                ));
            }
        }
        match declared {
            StreamDisposition::Xfail { bug, mismatch } => {
                validate_xfail_disposition(channel, bug, mismatch)
                    .map_err(|error| format!("case {}: {error}", case.id))?;
            }
            StreamDisposition::XfailDiagnostics { bug } => {
                validate_xfail_diagnostics_disposition(channel, bug)
                    .map_err(|error| format!("case {}: {error}", case.id))?;
            }
            StreamDisposition::Empty
            | StreamDisposition::File
            | StreamDisposition::Unsupported { .. } => {}
        }
    }
    Ok(())
}

/// Requires a fixture directory to be a closed, self-contained inventory.
///
/// This prevents a source, metadata file, or expected channel from drifting
/// back into a domain-level catalogue, and rejects symlinks so a fixture
/// cannot quietly depend on `target/` or another checkout.
fn validate_fixture_entries(case: &Case, fixture_dir: &Path) -> Result<(), String> {
    let mut allowed = BTreeSet::from(["manifest.json".to_owned(), case.source.clone()]);
    if let Some(channels) = &case.channels {
        for channel in STREAM_CHANNELS {
            if !matches!(
                channels.stream(channel),
                StreamDisposition::Empty | StreamDisposition::Unsupported { .. }
            ) {
                allowed.insert(format!("expected.{}", channel.name()));
            }
        }
    }
    let mut entries = fs::read_dir(fixture_dir)
        .map_err(|error| format!("{}: {error}", fixture_dir.display()))?
        .map(|entry| entry.map_err(|error| format!("{}: {error}", fixture_dir.display())))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut observed = BTreeSet::new();
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        if !file_type.is_file() {
            return Err(format!(
                "fixture entry {} must be a regular committed file",
                entry.path().display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("non-UTF-8 fixture entry {}", entry.path().display()))?;
        if !allowed.contains(&name) {
            return Err(format!("unowned fixture entry {}", entry.path().display()));
        }
        observed.insert(name);
    }
    if observed != allowed {
        let missing: Vec<_> = allowed.difference(&observed).cloned().collect();
        return Err(format!(
            "fixture {} is missing local file(s): {}",
            case.id,
            missing.join(", ")
        ));
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
    fixture_dir: &Path,
    root: &Path,
    owners: &BTreeMap<String, String>,
    policy: ChannelPolicy,
) -> Result<(), String> {
    if !valid_slug(&case.id) {
        return Err(format!("case id {:?} is not a lower-kebab slug", case.id));
    }
    if policy == ChannelPolicy::Required {
        validate_channels(case, fixture_dir)?;
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
    let source_path = fixture_dir.join(source);
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
    // An empty `expected` is permitted only for the one case the
    // regeneration path is actively deriving it for: a brand-new `pass`
    // case, under `ChannelPolicy::Deriving`, that has not been run yet. The
    // gate's own `ChannelPolicy::Required` never grants this, so a case
    // cannot reach the committed corpus with `expected` still empty, and an
    // `xfail` case -- whose `expected` can never be derived from a run, see
    // the field's own doc -- never grants it either.
    let expected_may_be_empty =
        policy == ChannelPolicy::Deriving && matches!(case.expectation, Expectation::Pass);
    if !(expected_may_be_empty && case.expected.is_empty())
        && (case.expected.is_empty()
            || case
                .expected
                .iter()
                .any(|observation| observation.is_empty() || observation.len() > 256))
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
    if matches!(&case.capture, CapturePolicy::Exclude { reason } if reason.trim().is_empty()) {
        return Err(format!(
            "case {} has an empty capture exclusion reason",
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
        let mut fixture_dirs = fs::read_dir(&domain_dir)
            .map_err(|error| format!("{}: {error}", domain_dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{}: {error}", domain_dir.display()))?;
        fixture_dirs.sort();
        let mut declared_sources = BTreeSet::new();
        for fixture_dir in fixture_dirs {
            if fixture_dir.is_file()
                && fixture_dir.file_name().and_then(|value| value.to_str()) == Some("README.md")
            {
                continue;
            }
            if !fixture_dir.is_dir() {
                return Err(format!("unowned domain entry {}", fixture_dir.display()));
            }
            let fixture_name = fixture_dir
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("non-UTF-8 fixture directory {}", fixture_dir.display()))?;
            let manifest_path = fixture_dir.join("manifest.json");
            let manifest: CaseManifestV2 = read_json(&manifest_path)?;
            if manifest.schema != SCHEMA {
                return Err(format!(
                    "{} has schema {}, expected {SCHEMA}",
                    manifest_path.display(),
                    manifest.schema
                ));
            }
            if !valid_slug(directory_name) {
                return Err(format!(
                    "{} has invalid domain directory {directory_name:?}",
                    manifest_path.display(),
                ));
            }
            let property_domain = owners
                .get(&manifest.property_id)
                .cloned()
                .unwrap_or_else(|| directory_name.to_owned());
            let case = manifest.resolve(&fixture_dir, fixture_name.to_owned());
            validate_case(
                &case,
                &property_domain,
                &fixture_dir,
                &root,
                &owners,
                policy,
            )?;
            validate_fixture_entries(&case, &fixture_dir)?;
            claim_case_identity(
                &mut case_ids,
                &mut sources,
                &mut declared_sources,
                directory_name,
                &case.id,
                &case.source,
            )?;
            declared.push(DeclaredCase {
                fixture_dir,
                domain: directory_name.to_owned(),
                case,
            });
        }
    }
    if declared.is_empty() {
        return Err("command semantic corpus declares no cases".into());
    }
    Ok(declared)
}

/// The terminal lines this case's engine run reads, byte-for-byte what the
/// oracle harness pipes into the reference engine.
///
/// `scripts/run-minifixture-oracle.sh` builds that stdin with
/// `printf '%s\n' "${terminal_lines[@]}"`, and `printf` runs its format once
/// even with no arguments, so a case that declares no lines still hands the
/// engine one empty line -- enough for §360's `*` prompt or §83's `? ` prompt
/// to succeed once before the next read hits end of file. Comparing channels
/// only means anything when both engines are given the same input, so this is
/// the same rule rather than "the declared lines".
fn terminal_stdin(case: &Case) -> Vec<String> {
    if case.terminal_lines.is_empty() {
        return vec![String::new()];
    }
    case.terminal_lines.clone()
}

pub fn execute(source: &[u8], case: &Case) -> Result<SemanticRun, String> {
    if case.profile.execution_route() != ExecutionRoute::Fresh {
        let provider = umber::PreparedFormatProvider::from_environment(format_worker_launcher())
            .map_err(|error| format!("persistent format provider: {error}"))?;
        return execute_with_provider(source, case, &provider);
    }
    execute_fresh(source, case)
}

/// Executes a case while injecting the persistent provider's store authority.
///
/// This is the hermetic test boundary: production callers use [`execute`]'s
/// platform cache, while tests can scope a real persistent store without
/// changing format preparation or loaded-job behavior.
pub fn execute_with_provider(
    source: &[u8],
    case: &Case,
    provider: &umber::PreparedFormatProvider,
) -> Result<SemanticRun, String> {
    let complete = execute_with_provider_completion(
        source,
        case,
        provider,
        tex_exec::RootCompletionPolicy::RequireTeXEnd,
    )?;
    let complete_job_channel_streams = CapturedChannels::capture(&complete).streams;
    let mut fragment = execute_with_provider_completion(
        source,
        case,
        provider,
        tex_exec::RootCompletionPolicy::StopAtRootEof,
    )?;
    validate_completion_projection_pair(&fragment, &complete)?;
    fragment.complete_job_channel_streams = Some(complete_job_channel_streams);
    Ok(fragment)
}

fn execute_with_provider_completion(
    source: &[u8],
    case: &Case,
    provider: &umber::PreparedFormatProvider,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    match case.profile.execution_route() {
        ExecutionRoute::ProductionPdftex14029Loaded => {
            execute_production_pdftex14029_loaded(source, case, provider, completion)
        }
        ExecutionRoute::RawEtex26Loaded => {
            execute_raw_etex26_loaded(source, case, provider, completion)
        }
        ExecutionRoute::RawTex82Loaded => {
            execute_raw_tex82_loaded(source, case, provider, completion)
        }
        ExecutionRoute::Fresh => execute_fresh_with_completion(source, case, completion),
    }
}

fn execute_fresh(source: &[u8], case: &Case) -> Result<SemanticRun, String> {
    let complete =
        execute_fresh_with_completion(source, case, tex_exec::RootCompletionPolicy::RequireTeXEnd)?;
    let complete_job_channel_streams = CapturedChannels::capture(&complete).streams;
    let mut fragment =
        execute_fresh_with_completion(source, case, tex_exec::RootCompletionPolicy::StopAtRootEof)?;
    validate_completion_projection_pair(&fragment, &complete)?;
    fragment.complete_job_channel_streams = Some(complete_job_channel_streams);
    Ok(fragment)
}

fn execute_fresh_with_completion(
    source: &[u8],
    case: &Case,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    umber::with_engine_universe(|universe| -> Result<SemanticRun, String> {
        for line in terminal_stdin(case) {
            universe
                .world_mut()
                .push_memory_terminal_line(line)
                .map_err(|error| format!("terminal line registration: {error}"))?;
        }
        let mut control = match case.profile {
            SessionProfile::Initex => MainControl::tex82_initex(universe),
            SessionProfile::EtexInitex => {
                let _tex82_registry = MainControl::tex82_initex(universe);
                tex_command::install_etex_expandable_primitives(universe);
                tex_exec::install_etex_unexpandable_primitives(universe);
                MainControl::prepared_initex(CommandProfile::ETEX26)
            }
            SessionProfile::EtexLoaded => unreachable!("loaded profile handled above"),
            SessionProfile::Production => unreachable!("loaded profile handled above"),
            SessionProfile::RawTex82Loaded => unreachable!("loaded profile handled above"),
        };
        for (name, bytes) in &case.inputs {
            // The same kpathsea `./` the root source carries, for the same
            // reason: a bare `\input child.tex` resolves beside the job, and
            // §537's `a_make_name_string` records -- and prints -- the resolved
            // `./child.tex`. A declared name that already names a directory is
            // taken as written, since kpathsea would not rewrite one.
            let resolved = if name.contains('/') {
                name.clone()
            } else {
                format!("./{name}")
            };
            control.capabilities_mut().register_input(
                name,
                SourceRegistration::new(
                    RegisteredSourceKind::Generated,
                    Arc::<[u8]>::from(bytes.as_bytes()),
                )
                .with_name(resolved),
            );
        }
        for (name, source) in &case.font_inputs {
            let bytes = fs::read(repository_root().join(source))
                .map_err(|error| format!("font fixture read: {error}"))?;
            universe
                .world_mut()
                .set_memory_file(name, bytes)
                .map_err(|error| format!("font fixture registration: {error}"))?;
            let metrics = InputReadState::read_input_file(
                &mut universe.input_open_context(),
                Path::new(name),
            )
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
        // [`validate_case`]). The fresh execution callback configures the
        // admitted generation here; loaded profiles use the prepared-format
        // provider's job-local interaction setting instead.
        universe.set_interaction_mode(case.interaction_mode.engine_mode());

        // Every profile is framed. `EtexLoaded`/`Production` used to be left
        // unframed because `begin_job` could only spell INITEX's `" (INITEX)"`
        // and no oracle could be reproduced to check a fabricated
        // `(preloaded format=...)` against. Both halves of that are now closed:
        // the oracle runner does a real `\dump`/`-fmt` roundtrip
        // (`umber2-alfh.1`), and `job::terminal_format_ident`/`log_format_ident`
        // spell the two sinks' different renderings from a declared
        // `PreloadedFormat` rather than guessing one (`umber2-alfh.15`).
        // §534/§536/§61: the start-up banner and the `**` line, which must
        // precede the root file's own `(` (see `crate::job`'s doc comment on
        // `begin_job`). `first_line` echoes what the oracle is invoked with on
        // its command line -- the bare source filename, e.g. `show-box.tex`.
        control.set_engine_binary(tex_exec::EngineBinaryIdentity::Pdftex14029);
        control.begin_job(universe, &case.source);
        control.set_root_completion_policy(completion);
        // kpathsea resolves a same-directory file through `./`, so pdfTeX's §537
        // `a_make_name_string` records (and prints) `./show-box.tex` rather than
        // the bare name `begin_job` was just given. Matching that leading `./` is
        // what makes Umber's own `(` line comparable to the oracle's.
        let root =
            SourceRegistration::new(RegisteredSourceKind::Generated, Arc::<[u8]>::from(source))
                .with_name(format!("./{}", case.source));
        control
            .register_root_source(root)
            .map_err(|error| format!("source registration: {error:?}"))?;
        let mut recorder = Recorder::default();
        let mut mode_transitions = vec![control.current_mode()];
        for _ in 0..MAX_STEPS {
            let step = control
                .step_with_observer(universe, &mut recorder)
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
                    let mut counts = [0; COUNT_SLOTS];
                    for (slot, value) in counts.iter_mut().enumerate() {
                        *value = universe
                            .count(u16::try_from(slot).expect("count register index"))
                            .map_err(|error| format!("count projection: {error:?}"))?;
                    }
                    let pages = control.take_prepared_dvi_pages();
                    let artifacts: Vec<ContentHash> =
                        pages.iter().map(|page| page.hash()).collect();
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
                    // §1333's DVI/transcript report closes out the banner
                    // `begin_job` printed, so every run gets both ends.
                    let job_name = control.capabilities_mut().job_name().to_owned();
                    let dvi_output = (!dvi.is_empty()).then(|| tex_exec::DviJobOutput {
                        file_name: format!("{job_name}.dvi"),
                        byte_len: dvi.len() as u64,
                    });
                    control.finish_job(universe, dvi_output, None);
                    let box_outlines = capture_box_outlines(universe, &case.projection)?;
                    let (terminal, log, pending_effects, effect_artifacts) =
                        capture_runtime_channels(universe);
                    return Ok(SemanticRun {
                        observations: recorder.0,
                        counts,
                        box_outlines,
                        mode_transitions,
                        artifacts,
                        dvi,
                        fatal: control.fatal_error(),
                        terminal,
                        log,
                        pending_effects,
                        effect_artifacts,
                        complete_job_channel_streams: None,
                    });
                }
            }
        }
        Err(format!("exceeded {MAX_STEPS} main-control steps"))
    })
    .map_err(|error| format!("fresh generation: {error:?}"))?
}

fn format_worker_launcher() -> umber::FormatWorkerLauncher {
    if std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_owned()))
        .is_some_and(|name| name == "command-semantic-channels")
    {
        umber::FormatWorkerLauncher::production()
    } else {
        umber::FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap")
    }
}

fn raw_tex82_recipe() -> umber::FormatRecipe {
    let mut recipe = umber::FormatRecipe::raw_tex82();
    recipe.format_name = "production".into();
    recipe.format_ident_name = "production".into();
    recipe
}

fn raw_etex26_recipe() -> umber::FormatRecipe {
    let mut recipe = umber::FormatRecipe::raw_etex26();
    recipe.format_name = "etex-loaded".into();
    recipe.format_ident_name = "etex-loaded".into();
    // This loaded-profile cohort intentionally observes both a macro that
    // survives §1309's format-memory compaction and e-TeX change [50.1307]'s
    // reset of optional state immediately before the dump.
    recipe.construction_source_name = "etex-loaded.ini".into();
    recipe.construction_source = b"\\catcode`\\{=1 \\catcode`\\}=2 \\def\\formatmacro{\\relax}\
\\catcode`\\{=12 \\catcode`\\}=12 \\TeXXeTstate=1 \\dump\n"
        .to_vec();
    recipe
}

/// Complete command-minifixture recipe for a loaded execution route.
#[must_use]
pub fn loaded_format_recipe(route: ExecutionRoute) -> Option<umber::FormatRecipe> {
    match route {
        ExecutionRoute::RawTex82Loaded => Some(raw_tex82_recipe()),
        ExecutionRoute::RawEtex26Loaded => Some(raw_etex26_recipe()),
        ExecutionRoute::ProductionPdftex14029Loaded => {
            Some(umber::FormatRecipe::production_pdftex14029())
        }
        ExecutionRoute::Fresh => None,
    }
}

fn execute_production_pdftex14029_loaded(
    source: &[u8],
    case: &Case,
    provider: &umber::PreparedFormatProvider,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    execute_loaded_format(
        provider,
        umber::FormatRecipe::production_pdftex14029(),
        source,
        case,
        "production pdfTeX 1.40.29",
        completion,
    )
}

fn execute_raw_etex26_loaded(
    source: &[u8],
    case: &Case,
    provider: &umber::PreparedFormatProvider,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    execute_loaded_format(
        provider,
        raw_etex26_recipe(),
        source,
        case,
        "raw e-TeX 2.6",
        completion,
    )
}

fn execute_raw_tex82_loaded(
    source: &[u8],
    case: &Case,
    provider: &umber::PreparedFormatProvider,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    execute_loaded_format(
        provider,
        raw_tex82_recipe(),
        source,
        case,
        "raw TeX82",
        completion,
    )
}

fn execute_loaded_format(
    provider: &umber::PreparedFormatProvider,
    recipe: umber::FormatRecipe,
    source: &[u8],
    case: &Case,
    format_label: &str,
    completion: tex_exec::RootCompletionPolicy,
) -> Result<SemanticRun, String> {
    let fixture = provider
        .prepare(&recipe)
        .map_err(|error| format!("prepare {format_label}: {error}"))?;
    let mut recorder = Recorder::default();
    let mut resources = Vec::with_capacity(case.inputs.len() + case.font_inputs.len());
    for (name, bytes) in &case.inputs {
        let resolved_name = if name.contains('/') {
            name.clone()
        } else {
            format!("./{name}")
        };
        resources.push(umber::LoadedFormatResource::Input {
            logical_name: name.clone(),
            resolved_name,
            source_kind: RegisteredSourceKind::Generated,
            bytes: bytes.as_bytes().to_vec(),
        });
    }
    for (name, fixture_source) in &case.font_inputs {
        let bytes = fs::read(repository_root().join(fixture_source))
            .map_err(|error| format!("font fixture read: {error}"))?;
        resources.push(umber::LoadedFormatResource::Tfm {
            logical_name: name.clone(),
            bytes,
        });
    }
    let job = umber::PreparedFormatJob {
        engine: recipe.engine,
        engine_binary: tex_exec::EngineBinaryIdentity::Pdftex14029,
        backend: umber::OutputCapability::Dvi,
        clock: tex_state::JobClock::default(),
        interaction: case.interaction_mode.engine_mode(),
        error_context_widths: tex_state::print::ErrorContextWidths::default(),
        provenance_demand: tex_state::ProvenanceDemand::DIAGNOSTICS,
        guards: recipe.guards,
        startup_line: case.source.clone(),
        source_name: case.source.clone(),
        source_kind: RegisteredSourceKind::Generated,
        source: source.to_vec(),
        resources,
        terminal_input: terminal_stdin(case),
        projection: umber::LoadedFormatProjectionDemand {
            count_registers: case.projection.count_registers.clone(),
            box_outlines: case
                .projection
                .box_registers
                .iter()
                .map(|&register| umber::LoadedBoxOutlineDemand {
                    register,
                    depth: case.projection.node_depth.unwrap_or(3),
                })
                .collect(),
            channels: completion == tex_exec::RootCompletionPolicy::RequireTeXEnd,
        },
        observer: &mut recorder,
    };
    let loaded = match completion {
        tex_exec::RootCompletionPolicy::RequireTeXEnd => provider.run(&fixture, job),
        tex_exec::RootCompletionPolicy::StopAtRootEof => provider.run_fragment(&fixture, job),
    }
    .map_err(|error| format!("loaded {format_label} run: {error}"))?;
    let umber::LoadedFormatRun { result, projection } = loaded;
    let mut counts = [0; COUNT_SLOTS];
    for (register, value) in projection.counts {
        counts[usize::from(register)] = value;
    }
    let box_outlines = projection
        .boxes
        .into_iter()
        .map(|outline| (outline.register, outline.nodes))
        .collect();
    let artifacts = result.artifacts.clone();
    let mut dvi = Vec::new();
    if !result.dvi_pages.is_empty() {
        let mut writer = tex_out::dvi::DviStreamWriter::new(Vec::new());
        for page in &result.dvi_pages {
            writer
                .write_page_plan(page)
                .map_err(|error| format!("loaded DVI page: {error:?}"))?;
        }
        dvi = writer
            .finish()
            .map_err(|error| format!("loaded DVI finish: {error:?}"))?;
    }
    let (terminal, log, pending_effects, effect_artifacts) = projection.channels.map_or_else(
        || {
            (
                result.terminal_text.as_bytes().to_vec(),
                Vec::new(),
                result.effects,
                Vec::new(),
            )
        },
        |channels| {
            (
                channels.terminal,
                channels.log,
                channels.pending_effects,
                channels
                    .outputs
                    .into_iter()
                    .map(|output| EffectArtifact {
                        path: output.path.to_string_lossy().into_owned(),
                        bytes: output.bytes,
                    })
                    .collect(),
            )
        },
    );
    Ok(SemanticRun {
        observations: recorder.0,
        counts,
        box_outlines,
        mode_transitions: result.mode_transitions,
        artifacts,
        dvi,
        fatal: result.fatal,
        terminal,
        log,
        pending_effects,
        effect_artifacts,
        complete_job_channel_streams: None,
    })
}

/// Detaches only the box registers selected by the fixture projection.
fn capture_box_outlines<G>(
    universe: &mut tex_state::Universe<G>,
    projection: &Projection,
) -> Result<BTreeMap<u16, Option<Vec<umber::DetachedNodeOutlineEntry>>>, String> {
    let mut outlines = BTreeMap::new();
    for &register in &projection.box_registers {
        let nodes = universe
            .copy_box_to_page(register)
            .map(|root| {
                let mut output = Vec::new();
                push_live_node_outline(
                    universe,
                    root,
                    &mut Vec::new(),
                    projection.node_depth.unwrap_or(3),
                    &mut output,
                )?;
                Ok::<_, String>(output)
            })
            .transpose()?;
        outlines.insert(register, nodes);
    }
    Ok(outlines)
}

fn push_live_node_outline<G>(
    universe: &tex_state::Universe<G>,
    root: tex_state::node_arena::PageListId,
    path: &mut Vec<usize>,
    depth: u8,
    output: &mut Vec<umber::DetachedNodeOutlineEntry>,
) -> Result<(), String> {
    let list = universe
        .page_node_list(root)
        .map_err(|error| format!("box outline root: {error:?}"))?;
    push_live_node_children(universe, list.nodes(), path, depth, output)
}

fn push_live_node_children<G>(
    universe: &tex_state::Universe<G>,
    nodes: tex_state::node_arena::NodeCursor<'_>,
    path: &mut Vec<usize>,
    depth: u8,
    output: &mut Vec<umber::DetachedNodeOutlineEntry>,
) -> Result<(), String> {
    for (index, node) in nodes.into_iter().enumerate() {
        path.push(index);
        output.push(umber::DetachedNodeOutlineEntry {
            path: path.clone(),
            kind: node.kind(),
        });
        if depth > 0
            && let tex_state::node::Node::HList(boxed) | tex_state::node::Node::VList(boxed) = node
        {
            let children = universe
                .page_node_list(boxed.children)
                .map_err(|error| format!("box outline child: {error:?}"))?;
            push_live_node_children(universe, children.nodes(), path, depth - 1, output)?;
        }
        path.pop();
    }
    Ok(())
}

fn capture_runtime_channels<G>(
    universe: &tex_state::Universe<G>,
) -> (
    Vec<u8>,
    Vec<u8>,
    Vec<tex_state::EffectRecord>,
    Vec<EffectArtifact>,
) {
    let world = universe.world();
    let terminal = world.memory_terminal_output().unwrap_or_default().to_vec();
    let log = world.memory_log_output().unwrap_or_default().to_vec();
    let pending_effects = world.effect_records().to_vec();
    let effect_artifacts = world
        .memory_outputs()
        .into_iter()
        .flatten()
        .map(|output| EffectArtifact {
            path: output.path().to_string_lossy().into_owned(),
            bytes: output.bytes().to_vec(),
        })
        .collect();
    (terminal, log, pending_effects, effect_artifacts)
}

fn validate_completion_projection_pair(
    fragment: &SemanticRun,
    complete: &SemanticRun,
) -> Result<(), String> {
    validate_completion_observations(&fragment.observations, &complete.observations)?;
    if !complete
        .mode_transitions
        .starts_with(&fragment.mode_transitions)
    {
        return Err("complete-job modes diverged before the fragment root-EOF boundary".into());
    }
    if !complete.artifacts.starts_with(&fragment.artifacts) {
        return Err("complete-job artifacts diverged before the fragment root-EOF boundary".into());
    }
    Ok(())
}

fn validate_completion_observations(
    fragment: &[CommandObservation],
    complete: &[CommandObservation],
) -> Result<(), String> {
    let first_difference = fragment
        .iter()
        .zip(complete)
        .position(|(fragment, complete)| fragment != complete);
    if let Some(index) = first_difference {
        let is_termination = |observation: &CommandObservation| {
            matches!(
                observation,
                CommandObservation::Effect(effect)
                    if effect.kind == tex_command::ObservationEffectKind::Terminate
            )
        };
        if !is_termination(&fragment[index]) {
            return Err(
                "complete-job observations diverged before the fragment root-EOF boundary".into(),
            );
        }
    }
    Ok(())
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

pub fn node_name(node: NodeKind) -> &'static str {
    match node {
        NodeKind::Char => "char",
        NodeKind::Lig => "ligature",
        NodeKind::Kern => "kern",
        NodeKind::MarginKern => "margin-kern",
        NodeKind::Glue => "glue",
        NodeKind::Penalty => "penalty",
        NodeKind::Rule => "rule",
        NodeKind::HList => "hlist",
        NodeKind::VList => "vlist",
        NodeKind::Unset => "unset",
        NodeKind::Disc => "discretionary",
        NodeKind::Mark => "mark",
        NodeKind::Ins => "insertion",
        NodeKind::Whatsit => "whatsit",
        NodeKind::MathOn => "math-on",
        NodeKind::MathOff => "math-off",
        NodeKind::Direction => "direction",
        NodeKind::MathNoad => "math-noad",
        NodeKind::FractionNoad => "fraction-noad",
        NodeKind::MathStyle => "math-style",
        NodeKind::MathChoice => "math-choice",
        NodeKind::MathList => "math-list",
        NodeKind::Nonscript => "nonscript",
        NodeKind::Adjust => "adjust",
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
        match run.box_outlines.get(register).and_then(Option::as_ref) {
            Some(entries) => output.extend(entries.iter().map(|entry| {
                let path = entry
                    .path
                    .iter()
                    .fold(format!("box:{register}"), |mut path, index| {
                        use std::fmt::Write as _;
                        write!(path, "/{index}").expect("writing to a String cannot fail");
                        path
                    });
                format!("{path}:{}", node_name(entry.kind))
            })),
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
                    scalars.push((record.kind, scanner_value_text(record)));
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
                    scanner_value_text(record),
                    match &record.value {
                        tex_command::ObservationValue::Tokens(tokens) => {
                            observed_tokens_text(tokens)
                        }
                        _ => "-".into(),
                    }
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
                    observation_value_text(&record.key),
                    observation_value_text(&record.value),
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
                Some(format!(
                    "effect:{}:{}:{}",
                    record.kind,
                    record.channel,
                    observation_value_text(&record.value)
                ))
            }
            _ => None,
        };
        if let Some(item) = item {
            output.push(item);
        }
    }
    output
}

fn scanner_value_text(record: &tex_command::ScannerRecord) -> String {
    use tex_command::ObservationValue;

    match &record.value {
        ObservationValue::None => "none".into(),
        ObservationValue::Integer(value) => value.to_string(),
        ObservationValue::Character(value) => value.to_string(),
        ObservationValue::Scaled(value) if record.kind == "internal" => format!("scaled:{value}"),
        ObservationValue::Scaled(value) => value.to_string(),
        ObservationValue::Glue {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } => format!(
            "{}width={width};stretch={stretch};stretch_order={stretch_order};shrink={shrink};shrink_order={shrink_order}",
            if record.kind == "internal" {
                "glue:"
            } else {
                ""
            }
        ),
        ObservationValue::Name(value) => value.clone(),
        ObservationValue::Bytes(value) => String::from_utf8_lossy(value).into_owned(),
        ObservationValue::Tokens(_) => "tokens".into(),
    }
}

fn observation_value_text(value: &tex_command::ObservationValue) -> String {
    use tex_command::ObservationValue;

    match value {
        ObservationValue::None => "none".into(),
        ObservationValue::Integer(value) => value.to_string(),
        ObservationValue::Character(value) => value.to_string(),
        ObservationValue::Scaled(value) => format!("scaled:{value}"),
        ObservationValue::Glue {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } => format!(
            "glue:width={width};stretch={stretch};stretch_order={stretch_order};shrink={shrink};shrink_order={shrink_order}"
        ),
        ObservationValue::Name(value) => value.clone(),
        ObservationValue::Bytes(value) => String::from_utf8_lossy(value).into_owned(),
        ObservationValue::Tokens(tokens) => observed_tokens_text(tokens),
    }
}

pub fn captured_terminal_text(run: &SemanticRun) -> String {
    channels::captured_printable_text(run).0
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
            let tex_command::ObservationValue::Name(key) = &record.key else {
                return None;
            };
            key.starts_with("count:")
                .then(|| format!("mutation:{key}={}", observation_value_text(&record.value)))
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

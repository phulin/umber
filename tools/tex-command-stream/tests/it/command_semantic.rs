//! Declarative, property-owned command semantic minifixtures.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only fixture test discovers and reads its committed corpus"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, DiagnosticArgument, InputReason,
    InputTransition, ObservedToken, RecoveryKind, RegisteredSourceKind, SourceRegistration,
    canonical_names,
};
use tex_exec::{CanonicalMainControl, MainControlStep};
use tex_state::Universe;

const SCHEMA: u32 = 1;
const MAX_SOURCE_BYTES: u64 = 4 * 1024;
const MAX_STEPS: usize = 512;
const COUNT_SLOTS: usize = 256;
const BUG_PREFIX: &str = "umber2-johp.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainManifest {
    schema: u32,
    domain: String,
    #[serde(default)]
    property_domain: Option<String>,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    property_id: String,
    source: String,
    provenance: Provenance,
    projection: Projection,
    expected: Vec<String>,
    expectation: Expectation,
    #[serde(default)]
    terminal_lines: Vec<String>,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    authority: String,
    manifest: String,
    sections: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    kind: ProjectionKind,
    #[serde(default)]
    count_registers: Vec<u16>,
    #[serde(default)]
    include_count_mutations: bool,
    #[serde(default)]
    kinds: Vec<ObservationKind>,
    #[serde(default)]
    commands: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ProjectionKind {
    Classification,
    ConditionSteps,
    SkippingConditionSteps,
    BranchSelections,
    PredicateOutcomes,
    Observations,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ObservationKind {
    Command,
    Input,
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
enum Expectation {
    Pass,
    Xfail {
        bug: String,
        mismatch: MismatchFingerprint,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct MismatchFingerprint {
    index: usize,
    kind: String,
    expected: String,
    actual: String,
}

#[derive(Debug, Deserialize)]
struct PropertyShard {
    domain: String,
    properties: Vec<OwnedProperty>,
}

#[derive(Debug, Deserialize)]
struct OwnedProperty {
    id: String,
}

struct DeclaredCase {
    domain_dir: PathBuf,
    domain: String,
    case: Case,
}

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

struct SemanticRun {
    observations: Vec<CommandObservation>,
    counts: [i32; COUNT_SLOTS],
}

#[derive(Debug, Eq, PartialEq)]
enum ExpectationError {
    PassMismatch(MismatchFingerprint),
    Xpass,
    ChangedFailure {
        pinned: Box<MismatchFingerprint>,
        observed: Box<MismatchFingerprint>,
    },
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .components()
        .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn property_owners(root: &Path) -> Result<BTreeMap<String, String>, String> {
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

fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_expectation(expectation: &Expectation) -> Result<(), String> {
    let Expectation::Xfail { bug, mismatch } = expectation else {
        return Ok(());
    };
    if !bug.starts_with(BUG_PREFIX)
        || bug.len() == BUG_PREFIX.len()
        || !bug[BUG_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "xfail bug must be a concrete {BUG_PREFIX}<number> id"
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

fn validate_case(
    case: &Case,
    property_domain: &str,
    domain_dir: &Path,
    root: &Path,
    owners: &BTreeMap<String, String>,
) -> Result<(), String> {
    if !valid_slug(&case.id) {
        return Err(format!("case id {:?} is not a lower-kebab slug", case.id));
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
    let metadata = fs::metadata(&source_path)
        .map_err(|error| format!("{}: {error}", source_path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "case {} source must be 1..={MAX_SOURCE_BYTES} bytes",
            case.id
        ));
    }
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
    validate_expectation(&case.expectation).map_err(|error| format!("case {}: {error}", case.id))
}
fn claim_case_identity(
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

fn load_suite() -> Result<Vec<DeclaredCase>, String> {
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
            validate_case(&case, property_domain, &domain_dir, &root, &owners)?;
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

fn execute(source: &[u8], case: &Case) -> Result<SemanticRun, String> {
    let mut universe = Universe::new();
    for line in &case.terminal_lines {
        universe
            .world_mut()
            .push_memory_terminal_line(line.clone())
            .map_err(|error| format!("terminal line registration: {error}"))?;
    }
    let mut control = CanonicalMainControl::tex82_initex(&mut universe);
    for (name, bytes) in &case.inputs {
        control.capabilities_mut().register_input(
            name,
            SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(bytes.as_bytes()),
            ),
        );
    }
    let source = control
        .command_mut()
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .map_err(|error| format!("source registration: {error:?}"))?;
    control
        .command_mut()
        .open_registered_source(source)
        .map_err(|error| format!("source opening: {error:?}"))?;
    let mut recorder = Recorder::default();
    for _ in 0..MAX_STEPS {
        match control
            .step_with_observer(&mut universe, &mut recorder)
            .map_err(|error| {
                let rendered = format!("{error:?}");
                if rendered.starts_with("Command(InputInvariant(") {
                    "main-control step: Command(InputInvariant)".into()
                } else {
                    format!("main-control step: {rendered}")
                }
            })? {
            MainControlStep::Continue => {}
            MainControlStep::End | MainControlStep::EndOfInput => {
                let counts = std::array::from_fn(|slot| {
                    universe.count(
                        u16::try_from(slot).expect("count slot fits in TeX82 register index"),
                    )
                });
                return Ok(SemanticRun {
                    observations: recorder.0,
                    counts,
                });
            }
        }
    }
    Err(format!("exceeded {MAX_STEPS} main-control steps"))
}

fn push_counts(run: &SemanticRun, projection: &Projection, output: &mut Vec<String>) {
    output.extend(
        projection
            .count_registers
            .iter()
            .map(|register| format!("count:{register}={}", run.counts[usize::from(*register)])),
    );
}

fn condition_observations(run: &SemanticRun) -> Vec<String> {
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

fn predicate_outcomes(run: &SemanticRun) -> Vec<String> {
    let mut stack = Vec::new();
    let mut active = BTreeMap::<u64, (&str, Vec<(&str, String)>)>::new();
    let mut outcomes = Vec::new();
    for observation in &run.observations {
        match observation {
            CommandObservation::Condition(record) if record.transition == "push" => {
                stack.push(record.identity);
                active.insert(record.identity, (record.condition, Vec::new()));
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

fn observed_token_text(token: &ObservedToken) -> String {
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

fn observed_tokens_text(tokens: &[ObservedToken]) -> String {
    tokens
        .iter()
        .map(observed_token_text)
        .collect::<Vec<_>>()
        .join(",")
}

fn observation_projection(run: &SemanticRun, projection: &Projection) -> Vec<String> {
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
                    InputReason::Source => "source",
                    InputReason::Backup => "backup",
                    InputReason::Macro => "macro",
                    InputReason::Parameter => "parameter",
                    InputReason::AlignmentUTemplate => "u-template",
                    InputReason::AlignmentVTemplate => "v-template",
                    InputReason::Recovery => "recovery",
                    InputReason::TokenList => "token-list",
                    InputReason::Write => "write",
                };
                Some(format!("input:{transition}:{reason}"))
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

fn project(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    let mut output = match projection.kind {
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
    };
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

fn first_mismatch(expected: &[String], actual: &[String]) -> Option<MismatchFingerprint> {
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

fn observed_mismatch(
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

fn evaluate_expectation(
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

#[test]
fn declared_command_semantic_cases_match() {
    let cases =
        load_suite().unwrap_or_else(|error| panic!("invalid command-semantic corpus: {error}"));
    let mut failures = Vec::new();
    for declared in &cases {
        let label = format!("{}/{}", declared.domain, declared.case.id);
        let actual = fs::read(declared.domain_dir.join(&declared.case.source))
            .map_err(|error| format!("source read: {error}"))
            .and_then(|source| execute(&source, &declared.case))
            .map(|run| project(&run, &declared.case.projection));
        if let Err(error) =
            evaluate_expectation(&declared.case.expected, &actual, &declared.case.expectation)
        {
            failures.push(format!("{label}: {error:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} declared cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn validator_rejects_duplicate_and_unowned_cases() {
    let mut case_ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut declared_sources = BTreeSet::new();
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-a.tex",
        )
        .is_ok()
    );
    assert!(
        claim_case_identity(
            &mut case_ids,
            &mut sources,
            &mut declared_sources,
            "conditionals",
            "case-a",
            "case-b.tex",
        )
        .expect_err("duplicate case identity must be rejected")
        .contains("duplicate case")
    );

    let unowned: Case = serde_json::from_slice(
        br#"{
            "id":"case-b",
            "property_id":"tex82.conditionals.not-owned",
            "source":"case-b.tex",
            "provenance":{"authority":"tex.web","manifest":"tests/tex82-oracle-manifest.txt","sections":[505]},
            "projection":{"kind":"predicate-outcomes"},
            "expected":["predicate:iftrue:-:true"],
            "expectation":{"kind":"pass"}
        }"#,
    )
    .expect("negative case has a valid manifest shape");
    assert!(
        validate_case(
            &unowned,
            "conditionals",
            Path::new("."),
            Path::new("."),
            &BTreeMap::new(),
        )
        .expect_err("unowned property must be rejected")
        .contains("unowned property")
    );
}
#[test]
fn xfail_manifest_validation_rejects_malformed_and_missing_bug_links() {
    let missing_bug = br#"{
        "kind":"xfail",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#;
    assert!(serde_json::from_slice::<Expectation>(missing_bug).is_err());

    let malformed: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"not-a-bead",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("shape is parseable before semantic validation");
    assert!(validate_expectation(&malformed).is_err());
}

#[test]
fn strict_xfail_accepts_only_the_pinned_failure_and_rejects_xpass() {
    let expectation = Expectation::Xfail {
        bug: "umber2-johp.246".into(),
        mismatch: MismatchFingerprint {
            index: 0,
            kind: "observation".into(),
            expected: "expected".into(),
            actual: "known-bug".into(),
        },
    };
    let expected = vec!["expected".into()];
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["expected".into()]), &expectation),
        Err(ExpectationError::Xpass)
    );
    assert!(matches!(
        evaluate_expectation(&expected, &Ok(vec!["new-failure".into()]), &expectation),
        Err(ExpectationError::ChangedFailure { .. })
    ));
    assert_eq!(
        evaluate_expectation(&expected, &Ok(vec!["known-bug".into()]), &expectation),
        Ok(())
    );
}

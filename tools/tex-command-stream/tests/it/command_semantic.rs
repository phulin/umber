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
    CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile,
    DiagnosticArgument, FatalError, FontResource, InputReason, InputTransition, ObservedToken,
    RecoveryKind, RegisteredSourceKind, SourceRegistration, canonical_names,
};
use tex_exec::{CanonicalMainControl, MainControlStep, Mode};
use tex_state::{
    ContentHash, EffectRecord, InputOpenState, InputReadState, PrintSink, Universe, node::Node,
};

const SCHEMA: u32 = 1;
const MAX_SOURCE_BYTES: u64 = 4 * 1024;
// TeX82 permits 255 spans before the 256-span confusion boundary, so the
// bounded semantic runner must admit that one deliberately maximal case.
const MAX_STEPS: usize = 2_048;
const COUNT_SLOTS: usize = 256;
const BUG_PREFIX: &str = "umber2-";

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
    #[serde(default)]
    profile: SessionProfile,
    #[serde(default)]
    font_inputs: BTreeMap<String, String>,
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
    #[serde(default)]
    command_names: Vec<String>,
    #[serde(default)]
    alignment_transitions: Vec<String>,
    #[serde(default)]
    box_registers: Vec<u16>,
    node_depth: Option<u8>,
    #[serde(default)]
    include_mode_transitions: bool,
    #[serde(default)]
    include_artifact_hashes: bool,
    #[serde(default)]
    terminal_checks: Vec<String>,
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
    ExecutionBoundaries,
    State,
    TerminalChecks,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum SessionProfile {
    #[default]
    Initex,
    EtexInitex,
    EtexLoaded,
    Production,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ObservationKind {
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
    universe: Universe,
    mode_transitions: Vec<Mode>,
    artifacts: Vec<ContentHash>,
    /// TeX82 §93 `succumb`'s terminal state, when the job ended through
    /// §81's `jump_out` instead of running to `\end`.
    ///
    /// A fatal error is deliberately *not* an `Err` here. `Err` means the
    /// runner could not produce a run at all, and such a run is unprojectable
    /// on purpose so an engine crash can never be mistaken for a fixture
    /// outcome. A fatal error is the opposite: the engine reached a defined
    /// TeX82 terminal state and the job is over, which is an observable
    /// semantic fact that a fixture must be able to pin.
    fatal: Option<FatalError>,
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

fn valid_bug_id(value: &str) -> bool {
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

fn validate_expectation(expectation: &Expectation) -> Result<(), String> {
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
                let artifacts = control
                    .take_prepared_dvi_pages()
                    .into_iter()
                    .map(|page| page.hash())
                    .collect();
                return Ok(SemanticRun {
                    observations: recorder.0,
                    counts,
                    universe,
                    mode_transitions,
                    artifacts,
                    fatal: control.fatal_error(),
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

fn mode_name(mode: Mode) -> String {
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

fn node_name(node: &Node) -> String {
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

fn push_node_outline(
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

fn execution_boundaries(run: &SemanticRun, projection: &Projection) -> Vec<String> {
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
                    | InputReason::Mark
                    | InputReason::UmberReplay(_) => "token-list",
                    InputReason::Write => "write",
                };
                Some(format!("input:{transition}:{reason}"))
            }
            CommandObservation::Alignment(record)
                if projection.kinds.contains(&ObservationKind::Alignment)
                    && (projection.alignment_transitions.is_empty()
                        || projection
                            .alignment_transitions
                            .iter()
                            .any(|transition| transition == record.transition)) =>
            {
                Some(format!(
                    "alignment:{}:{}",
                    record.transition, record.align_state
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

fn captured_terminal_text(run: &SemanticRun) -> String {
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

fn terminal_check_results(output: &str, checks: &[String]) -> Vec<String> {
    checks
        .iter()
        .map(|check| format!("terminal-check:{check}={}", output.contains(check)))
        .collect()
}

fn terminal_check_projection(run: &SemanticRun, projection: &Projection) -> Vec<String> {
    terminal_check_results(&captured_terminal_text(run), &projection.terminal_checks)
}

/// Projects one completed run.
///
/// A fatal termination is a property of the *run*, not of any one projection
/// kind, so its marker is emitted for every kind, ahead of everything else. A
/// case whose engine now dies where it used to finish therefore gains a line
/// its `expected` array must declare; it can never quietly lose one.
fn project(run: &SemanticRun, projection: &Projection) -> Vec<String> {
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

    let opaque: Expectation = serde_json::from_slice(
        br#"{
        "kind":"xfail",
        "bug":"umber2-o96f",
        "mismatch":{"index":0,"kind":"observation","expected":"a","actual":"b"}
    }"#,
    )
    .expect("opaque Beads id has the manifest shape");
    assert!(validate_expectation(&opaque).is_ok());
}

#[test]
fn state_projection_emits_only_requested_final_counts() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        counts,
        universe: Universe::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        fatal: None,
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(project(&run, &projection), ["count:2=7"]);
}

#[test]
fn fatal_termination_precedes_every_projection_kinds_own_output() {
    let mut counts = [0; COUNT_SLOTS];
    counts[2] = 7;
    let run = SemanticRun {
        observations: Vec::new(),
        counts,
        universe: Universe::new(),
        mode_transitions: Vec::new(),
        artifacts: Vec::new(),
        fatal: Some(FatalError::confusion("256 spans")),
    };
    let projection = Projection {
        kind: ProjectionKind::State,
        count_registers: vec![2],
        include_count_mutations: false,
        kinds: Vec::new(),
        commands: Vec::new(),
        command_names: Vec::new(),
        alignment_transitions: Vec::new(),
        box_registers: Vec::new(),
        node_depth: None,
        include_mode_transitions: false,
        include_artifact_hashes: false,
        terminal_checks: Vec::new(),
    };

    assert_eq!(
        project(&run, &projection),
        ["execution:error:confusion(256 spans)", "count:2=7"]
    );
}

#[test]
fn terminal_checks_report_presence_and_absence_in_declaration_order() {
    let checks = vec!["alpha beta".into(), "gamma".into()];

    assert_eq!(
        terminal_check_results("alpha beta", &checks),
        [
            "terminal-check:alpha beta=true",
            "terminal-check:gamma=false"
        ]
    );
}

#[test]
fn strict_xfail_accepts_only_the_pinned_failure_and_rejects_xpass() {
    let expectation = Expectation::Xfail {
        bug: "umber2-o96f".into(),
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

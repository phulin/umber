//! Offline comparison of command-owned observations with committed oracle streams.
//!
//! This host-only crate owns the deliberately lossy boundary translation from
//! `tex-command` observer records to the portable oracle schema. Production
//! command processing neither imports nor knows about canonical fixtures.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tex_command::{
    CommandProfile, FontResource, RegisteredSourceKind, SourceNameClass, SourceRegistration,
};
use tex_exec::{MainControl, MainControlStep};
use tex_oracle::{
    CommittedFixture, EngineDialect, Event, SchemaVersion, validate_tex82_command_trace_suite,
    validate_tex82_geometry_trace_fixture,
};
use tex_state::InputReadState;

pub mod compare;
pub mod documents;
pub mod group;
pub mod policy;
pub mod report;
pub mod semantic;

use tex_observe::LiveSessionTranslator;
pub use tex_observe::ObservedEvent;
pub use tex_oracle::OracleBundle;

type Recorder = LiveSessionTranslator;

pub use compare::{
    AlignmentTuning, Comparison, DEFAULT_ANCHOR_SCAN, DEFAULT_REALIGN_CONFIRMATION,
    DEFAULT_REALIGN_WINDOW, MismatchSides, Repair, ResyncAnchor, StreamMismatch, compare_streams,
    find_divergences,
};
pub use group::{RootSite, group};
pub use policy::{
    OrdinaryComparison, OrdinaryComparisonAccounting, OrdinaryComparisonPolicy,
    StrictTripAccounting, StrictTripChannel, StrictTripComparison, StrictTripComparisonPolicy,
    StrictTripDivergence, StrictTripError,
};
pub use report::{ComparisonReport, EXIT_NOT_RUN, FixtureState, FixtureSummary, RunOutcome};

const FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const MAX_DIAGNOSTIC_CHARS: usize = 960;
/// Characters of already-agreeing context shown before the first differing
/// character, and of divergent text shown after it, by
/// [`hidden_difference_excerpt`].
const DIFFERENCE_LEAD_CHARS: usize = 120;
const DIFFERENCE_TRAIL_CHARS: usize = 360;
const MAX_DELIVERIES_OVERHEAD: usize = 64;
const TERMINAL_FILENAME_TERMINATOR: u8 = b' ';
const CANONICAL_ROOT_PUSH_NAME: &str = "terminal";

/// Maximum number of source files in one fixture selected by an automated
/// differential-tracer test.
///
/// This and the other automated-fixture bounds classify fixtures by their
/// structural footprint, not by a document name. Full-document traces belong
/// to [`documents`] and the explicit [`run_repository`] diagnostic.
const AUTOMATED_MAX_SOURCES: usize = 64;
/// Maximum combined source bytes in one automated tracer fixture.
const AUTOMATED_MAX_SOURCE_BYTES: u64 = 64 * 1024;
/// Maximum ordered oracle events in one automated tracer fixture.
const AUTOMATED_MAX_EVENTS: usize = 50_000;

/// Default cap on ordered divergences reported *per fixture*
/// (`--max-divergences` overrides it). Chosen to comfortably batch a
/// fixture's independent defects into one worklist without an unbounded
/// report against a long fixture.
///
/// The cap is per fixture rather than per run so that one noisy fixture --
/// typically a single structural defect producing a long unbroken run of
/// consecutive-index mismatches -- cannot starve every fixture ordered after
/// it, which would hide whole documents from the worklist.
///
/// # Why the unit is ordered divergences and not root sites
///
/// Since the worklist began printing one entry per root site
/// (`crate::group`), the budget and the printed entry count are different
/// quantities, and re-basing the budget onto root sites has been considered
/// and rejected (`umber2-johp.207`). Three reasons, any one sufficient:
///
/// - A root-site budget bounds nothing. This cap exists to stop one long
///   fixture from producing an unbounded walk and an unbounded report, and
///   the case it was introduced for is a single structural defect recurring
///   without end. Root sites are grouped by content, so that defect is *one*
///   root site however many times it recurs: a budget of 20 root sites would
///   walk the whole fixture and print a recurrence list thousands of indices
///   long, which is exactly the outcome the cap prevents today.
/// - It would move the ambiguity rather than remove it. Budget and printed
///   entry count already agree exactly under `--ungrouped`, and would stop
///   agreeing there. No unit equals the printed entry count in both views, so
///   that equality is not an available invariant; each number naming its own
///   unit where it is printed is (`crate::report`).
/// - The comparator would acquire a dependency on the presentation layer.
///   Grouping is documented as changing only how the worklist prints, never
///   what is compared or in what order; a root-site budget would make the
///   grouping projection decide where the comparison stops, so `--ungrouped`
///   and the grouped view would compare different amounts of the stream.
///
/// A `--max-divergences N` run therefore reports at most `N` stream
/// mismatches per fixture, collapsing to at most `N` printed entries, and the
/// one contained replay failure a fixture may produce is reported outside
/// this budget (see `crate::report::FixtureState::Compared`).
pub const DEFAULT_MAX_DIVERGENCES: usize = 20;

/// Everything one offline comparison run is allowed to vary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunOptions {
    /// Cap on ordered divergences reported per fixture.
    pub max_divergences: usize,
    /// Bounds on the comparator's resynchronization search.
    pub alignment: AlignmentTuning,
    /// Collapse exact recurrences of one root site into a single reported
    /// entry (`--ungrouped` clears it). Presentation only: the comparison, the
    /// divergence order, and the divergence count are identical either way.
    pub grouped: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            max_divergences: DEFAULT_MAX_DIVERGENCES,
            alignment: AlignmentTuning::default(),
            grouped: true,
        }
    }
}

/// Runs every committed TeX82 command fixture with no live-engine access,
/// reporting up to `options.max_divergences` ordered divergences *per fixture*
/// instead of only the first.
///
/// This is the routine native-test gate. It validates the committed fixture
/// inventory before replaying it, so a missing or drifted microfixture is an
/// error rather than an empty successful comparison. Generated full-document
/// traces are deliberately outside this entry point; use [`run_repository`]
/// for the explicit, potentially slow diagnostic that includes them.
pub fn run_committed_repository(
    repository: impl AsRef<Path>,
    options: RunOptions,
) -> Result<ComparisonReport, RunnerError> {
    let repository = repository.as_ref();
    let suite = validate_tex82_command_trace_suite(repository)
        .map_err(|error| RunnerError::Suite(error.to_string()))?;
    let mut report = ComparisonReport {
        grouped: options.grouped,
        max_divergences: options.max_divergences,
        ..ComparisonReport::default()
    };
    for entry in suite.fixtures {
        let fixture_directory =
            repository
                .join(FIXTURE_ROOT)
                .join(entry.selector.strip_prefix("tex82/").ok_or_else(|| {
                    RunnerError::Suite(format!("unsafe selector {}", entry.selector))
                })?);
        let fixture = CommittedFixture::load(&fixture_directory)
            .map_err(|error| RunnerError::Fixture(entry.selector.clone(), error.to_string()))?;
        validate_automated_fixture(&fixture)?;
        collect_fixture_divergences(
            &fixture_directory,
            &fixture,
            &ReplayResources::default(),
            options,
            &mut report,
        )?;
    }
    collect_geometry_divergences(repository, options, &mut report)?;

    Ok(report)
}

fn validate_automated_fixture(fixture: &CommittedFixture) -> Result<(), RunnerError> {
    let sources = fixture.manifest.sources.len();
    let source_bytes = fixture
        .manifest
        .sources
        .values()
        .try_fold(0_u64, |total, source| total.checked_add(source.bytes))
        .ok_or_else(|| {
            RunnerError::Fixture(
                fixture.manifest.name.clone(),
                "source-byte total overflows u64".into(),
            )
        })?;
    validate_automated_footprint(
        &fixture.manifest.name,
        AutomatedFixtureFootprint {
            sources,
            source_bytes,
            events: fixture.stream.events.len(),
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AutomatedFixtureFootprint {
    sources: usize,
    source_bytes: u64,
    events: usize,
}

fn validate_automated_footprint(
    name: &str,
    footprint: AutomatedFixtureFootprint,
) -> Result<(), RunnerError> {
    if footprint.sources <= AUTOMATED_MAX_SOURCES
        && footprint.source_bytes <= AUTOMATED_MAX_SOURCE_BYTES
        && footprint.events <= AUTOMATED_MAX_EVENTS
    {
        return Ok(());
    }
    Err(RunnerError::Fixture(
        name.into(),
        format!(
            "automated differential tracing accepts only bounded microfixtures: \
             observed {} source(s), {} source byte(s), and {} event(s); limits are \
             {AUTOMATED_MAX_SOURCES} source(s), {AUTOMATED_MAX_SOURCE_BYTES} source byte(s), \
             and {AUTOMATED_MAX_EVENTS} event(s). Move full-document traces to \
             tests/corpus/command/tex82-documents and run the explicit \
             `cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- --repository .` diagnostic",
            footprint.sources, footprint.source_bytes, footprint.events
        ),
    ))
}

fn collect_geometry_divergences(
    repository: &Path,
    options: RunOptions,
    report: &mut ComparisonReport,
) -> Result<(), RunnerError> {
    let fixture = validate_tex82_geometry_trace_fixture(repository)
        .map_err(|error| RunnerError::Suite(error.to_string()))?;
    let replay = Startup::geometry(fixture.source, fixture.stream.events.len()).replay()?;
    let actual = replay
        .events
        .into_iter()
        .filter(|event| matches!(event.event, Event::Geometry(_)))
        .collect::<Vec<_>>();
    let actual_events = actual.len();
    let identity = format!("{} projection={}", fixture.selector, fixture.identity);
    let comparison = OrdinaryComparisonPolicy {
        max_divergences: options.max_divergences,
        alignment: options.alignment,
    }
    .compare(&identity, &fixture.stream.events, &actual);
    let first = report.advisories.len();
    let budgeted = comparison.accounting.ordered_divergences;
    let budget_reached = comparison.accounting.budget_reached;
    report.advisories.extend(comparison.divergences);
    if let Some(failure) = replay.failure {
        report.advisories.push(Divergence::Failure {
            fixture: identity.clone(),
            index: actual_events,
            failure,
        });
    }
    report.fixtures.push(FixtureSummary {
        name: fixture.selector,
        identity,
        advisory: true,
        state: FixtureState::Compared {
            divergences: report.advisories.len() - first,
            budgeted,
            first_index: report.advisories.get(first).map(Divergence::index),
            budget_reached,
        },
    });
    Ok(())
}

/// Runs every registered TeX82 trace with no live-engine access, reporting up
/// to `options.max_divergences` ordered divergences *per fixture* instead of
/// only the first.
///
/// Returns the [`ComparisonReport`] for any run that happened, diverging or
/// not; the `Err` arm is reserved for a run that could not be performed.
/// "Found no divergence" is therefore not a success *value* the caller can
/// confuse with "compared nothing": the report carries its own
/// [`ComparisonReport::outcome`], and a run that skipped a registered fixture
/// is [`RunOutcome::Partial`], never [`RunOutcome::Clean`].
///
/// Two registries are replayed, in this order: the committed, hermetic
/// fixtures under `tests/corpus/command/tex82`, then the generated-on-demand
/// full-document traces described by [`documents`]. Committed fixtures come
/// first because they are always present and their divergences are the
/// cheapest to act on; a document trace tree that has not been generated on
/// this checkout is reported as skipped, not as a failure. Every fixture is
/// replayed and gets its own divergence budget, so an earlier fixture's
/// defects never hide a later fixture's.
///
/// A fixture whose replay hits a Rust panic or a command-core `ExecError` is
/// contained (`catch_panic`/`ReplayFailure`) and reported as its own ordered
/// divergence entry, exactly like a stream mismatch: it does not abort the
/// run or hide any fixture ordered after it. Comparison also does not stop
/// at a fixture's first stream mismatch -- it resynchronizes the two streams
/// (`compare`) and keeps scanning that fixture's remaining events for
/// independent, later divergences until either the fixture's stream is
/// exhausted, the alignment is abandoned as structurally irreparable, or the
/// fixture's divergence budget is spent.
pub fn run_repository(
    repository: impl AsRef<Path>,
    options: RunOptions,
) -> Result<ComparisonReport, RunnerError> {
    let repository = repository.as_ref();
    let mut report = run_committed_repository(repository, options)?;

    let registry = documents::load_registry(repository)?;
    for name in &registry.skipped {
        eprintln!(
            "tex-command-stream: document trace {name} is not generated on this checkout; \
             run scripts/build-tex82-document-traces.sh to include it"
        );
        report.fixtures.push(FixtureSummary::not_generated(name));
    }
    for entry in registry.traces {
        let trace = entry.load()?;
        collect_fixture_divergences(
            &trace.directory,
            &trace.fixture,
            &trace.resources,
            options,
            &mut report,
        )?;
    }

    Ok(report)
}

fn collect_fixture_divergences(
    directory: &Path,
    fixture: &CommittedFixture,
    resources: &ReplayResources,
    options: RunOptions,
    report: &mut ComparisonReport,
) -> Result<(), RunnerError> {
    let replay = replay_fixture(directory, fixture, resources)?;
    let identity = format!(
        "{} manifest={}",
        fixture.manifest.name, fixture.stream.header.manifest
    );
    let comparison = OrdinaryComparisonPolicy {
        max_divergences: options.max_divergences,
        alignment: options.alignment,
    }
    .compare(
        &identity,
        &fixture.stream.events[replay.verified_prefix..],
        &replay.events,
    );
    let mut comparison = comparison;
    for divergence in &mut comparison.divergences {
        if let Divergence::Mismatch(mismatch) = divergence {
            mismatch.offset_indices(replay.verified_prefix);
        }
    }
    let first = report.divergences.len();
    // The budget counts these and only these; the contained failure below is
    // outside it, so the two are recorded separately rather than summed.
    let budgeted = comparison.accounting.ordered_divergences;
    let budget_reached = comparison.accounting.budget_reached;
    report.divergences.extend(comparison.divergences);
    // A contained failure is at most one entry per fixture and names a
    // concrete `ExecError` or panic site, so it is reported outside the
    // mismatch budget: the twentieth consecutive mismatch of an
    // already-reported structural defect must never crowd it out.
    if let Some(failure) = replay.failure {
        report.divergences.push(Divergence::Failure {
            fixture: identity.clone(),
            index: replay.verified_prefix.saturating_add(replay.events.len()),
            failure,
        });
    }
    report.fixtures.push(FixtureSummary {
        name: fixture.manifest.name.clone(),
        identity,
        advisory: false,
        state: FixtureState::Compared {
            divergences: report.divergences.len() - first,
            budgeted,
            first_index: report.divergences.get(first).map(Divergence::index),
            budget_reached,
        },
    });
    Ok(())
}

const USAGE: &str = "expected --repository <path>, --max-divergences <n>, \
                     --realign-window <n>, --realign-confirm <n>, --anchor-scan <n>, \
                     or --ungrouped";

/// Parses the intentionally narrow offline runner interface.
///
/// The three alignment tunables exist so a coordinator can widen the search
/// when a suspected repair is larger than the default window, or narrow it to
/// prove a reported realignment is not an artifact of an over-generous bound.
/// `--ungrouped` restores the one-entry-per-divergence worklist, so the
/// grouped report can always be checked against the list it summarizes.
pub fn run_cli() -> Result<ComparisonReport, RunnerError> {
    let mut arguments = env::args_os().skip(1);
    let mut repository = None;
    let mut options = RunOptions::default();
    while let Some(argument) = arguments.next() {
        if argument == "--repository" {
            repository = Some(
                arguments
                    .next()
                    .ok_or_else(|| {
                        RunnerError::Usage("--repository requires a directory argument".into())
                    })?
                    .into(),
            );
        } else if argument == "--max-divergences" {
            options.max_divergences = positive_argument(&mut arguments, "--max-divergences")?;
        } else if argument == "--realign-window" {
            options.alignment.window = positive_argument(&mut arguments, "--realign-window")?;
        } else if argument == "--realign-confirm" {
            options.alignment.confirmation =
                positive_argument(&mut arguments, "--realign-confirm")?;
        } else if argument == "--anchor-scan" {
            options.alignment.anchor_scan = positive_argument(&mut arguments, "--anchor-scan")?;
        } else if argument == "--ungrouped" {
            options.grouped = false;
        } else {
            return Err(RunnerError::Usage(format!(
                "unknown argument {}; {USAGE}",
                argument.to_string_lossy()
            )));
        }
    }
    let repository = resolve_cli_repository(repository)?;
    run_repository(repository, options)
}

fn resolve_cli_repository(repository: Option<PathBuf>) -> Result<PathBuf, RunnerError> {
    let requested = match repository {
        Some(repository) => repository,
        None => env::current_dir()
            .map_err(|error| RunnerError::Suite(format!("determine current directory: {error}")))?,
    };
    test_support::repository_root_at(&requested).map_err(|error| {
        RunnerError::Suite(format!(
            "resolve repository root {}: {error:#}",
            requested.display()
        ))
    })
}

fn positive_argument(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Result<usize, RunnerError> {
    let value = arguments
        .next()
        .ok_or_else(|| RunnerError::Usage(format!("{flag} requires an integer argument")))?;
    value
        .to_str()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            RunnerError::Usage(format!(
                "{flag} requires a positive integer, got {}",
                value.to_string_lossy()
            ))
        })
}

/// One ordered worklist entry: either a stream-content mismatch or a
/// contained replay failure (a command-core `ExecError` or a Rust panic)
/// that ended a fixture's replay early. Both are ordered and labeled the
/// same way so a coordinator can batch them without distinguishing "found a
/// wrong event" from "the engine gave up" -- both are equally a defect to
/// file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Divergence {
    Mismatch(Box<StreamMismatch>),
    Failure {
        fixture: String,
        /// The observed-event index the failure occurred after; the
        /// contained failure has no expected/actual event of its own.
        index: usize,
        failure: ReplayFailure,
    },
}

impl Divergence {
    /// The fixture identity this divergence belongs to.
    pub fn fixture(&self) -> &str {
        match self {
            Self::Mismatch(mismatch) => &mismatch.fixture,
            Self::Failure { fixture, .. } => fixture,
        }
    }

    /// The cheap structural label this divergence is batched by.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Mismatch(mismatch) => mismatch.kind,
            Self::Failure { failure, .. } => failure.kind(),
        }
    }

    /// The oracle event index this divergence is reported at. For a contained
    /// replay failure this is the observed index the failure occurred after.
    pub fn index(&self) -> usize {
        match self {
            Self::Mismatch(mismatch) => mismatch.index,
            Self::Failure { index, .. } => *index,
        }
    }

    /// Cascade mismatches this divergence stands in for; always zero for a
    /// contained replay failure, which stands in for nothing.
    pub fn suppressed_cascade(&self) -> usize {
        match self {
            Self::Mismatch(mismatch) => mismatch.suppressed_cascade,
            Self::Failure { .. } => 0,
        }
    }
}

impl fmt::Display for Divergence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mismatch(mismatch) => mismatch.fmt(formatter),
            Self::Failure {
                fixture,
                index,
                failure,
            } => write!(
                formatter,
                "fixture {fixture} {} after event {index} [{}]\n  {}",
                failure.label(),
                failure.kind(),
                failure.message()
            ),
        }
    }
}

/// Why a fixture's replay stopped before producing a complete observed
/// stream. Both variants are contained (`catch_panic`, ordinary `Result`
/// propagation) rather than aborting the whole run: see
/// [`run_repository`]'s documentation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayFailure {
    /// A command-core `ExecError`/`SessionError` the processor
    /// returned normally.
    Error(String),
    /// A Rust panic caught by [`catch_panic`], with its rendered message and
    /// source location (when the panic runtime provided one).
    Panic(String),
}

impl ReplayFailure {
    fn label(&self) -> &'static str {
        match self {
            Self::Error(_) => "replay failed",
            Self::Panic(_) => "engine panicked",
        }
    }

    /// Cheap structural label, the contained-failure counterpart of a
    /// mismatch's [`StreamMismatch`] kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Error(_) => "exec_error",
            Self::Panic(_) => "panic",
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Error(message) | Self::Panic(message) => message,
        }
    }
}

#[derive(Debug)]
pub enum RunnerError {
    Usage(String),
    Suite(String),
    Fixture(String, String),
    /// The generated document-trace registry is registered but inconsistent
    /// with its committed pin, or unreadable.
    Document(String),
    Replay(String),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error)
            | Self::Suite(error)
            | Self::Document(error)
            | Self::Replay(error) => formatter.write_str(error),
            Self::Fixture(fixture, error) => write!(formatter, "fixture {fixture}: {error}"),
        }
    }
}

impl Error for RunnerError {}

/// Runs `f`, containing a panic instead of letting it unwind past this call.
///
/// The default panic hook already prints a panic's message and source
/// location to stderr; this installs a capturing hook for the duration of
/// the call so that same rendered text becomes an ordered [`Divergence`]
/// entry too, then restores the previous hook. `RUST_BACKTRACE=1` still
/// produces a full backtrace on stderr exactly as it would for an uncaught
/// panic, since the default hook's formatting is preserved verbatim
/// (`PanicHookInfo`'s own `Display` impl), only additionally captured.
fn catch_panic<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> Result<T, String> {
    use std::panic;
    use std::sync::{Arc, Mutex};

    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&captured);
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let rendered = info.to_string();
        eprintln!("{rendered}");
        *sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(rendered);
    }));
    let result = panic::catch_unwind(f);
    panic::set_hook(previous_hook);
    result.map_err(|_| {
        captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| "engine panicked with no captured message".into())
    })
}

fn replay_fixture(
    directory: &Path,
    fixture: &CommittedFixture,
    resources: &ReplayResources,
) -> Result<ReplayOutput, RunnerError> {
    if fixture.manifest.oracle.engine.dialect != EngineDialect::Tex82
        || fixture.manifest.profile.invocation != "initex"
        || fixture.manifest.profile.characters != "eight_bit_exact"
    {
        return Err(RunnerError::Replay(format!(
            "{} is not a TeX82 INITEX eight-bit fixture",
            fixture.manifest.name
        )));
    }
    Startup::from_fixture(directory, fixture, resources)?.replay_against(&fixture.stream.events)
}

/// Replay inputs a fixture needs beyond its own manifest: the opaque font
/// metrics canonical `\font` resolution must find already registered. Which
/// declared source TeX's terminal filename scan selects is not a replay
/// input at all -- it is the fixture's own [`FixtureManifest::root_source`],
/// tex.web §537's `start_input` target, so [`Startup::from_fixture`]
/// reads it from the fixture being replayed instead of from here.
///
/// `MainControl::resolve_font_resource` never suspends -- an
/// unregistered font is an immediate `ExecError::MissingFont` -- so
/// every font a document can reach is registered up front, before the first
/// step, rather than through a lazy resource-host retry loop.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplayResources {
    fonts: BTreeMap<String, Vec<u8>>,
}

/// Observer output plus a contained, nonterminal replay failure, if one
/// occurred.
///
/// The complete fixture stream is still compared first, so a failure after
/// an already-produced earlier semantic divergence cannot mask that
/// deterministic earlier mismatch. If the observed prefix is exact, the
/// failure becomes its own ordered [`Divergence::Failure`] entry instead of
/// being mistaken for a clean EOF.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayOutput {
    /// Exact positional prefix compared and released during replay.
    verified_prefix: usize,
    events: Vec<ObservedEvent>,
    failure: Option<ReplayFailure>,
}

/// Typed host-harness state preceding the first TeX command transition.
///
/// TeX starts by scanning a terminal filename, then opens that selected root
/// source above the still-live terminal input.  This is intentionally not an
/// iteration over all fixture sources: child inputs are capabilities consumed
/// later by canonical `\\input` processing.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Startup {
    profile: CommandProfile,
    terminal_filename: Vec<u8>,
    root_name: String,
    root_bytes: Vec<u8>,
    input_capabilities: BTreeMap<String, Vec<u8>>,
    fonts: BTreeMap<String, Vec<u8>>,
    expected_events: usize,
    schema: SchemaVersion,
}

impl Startup {
    fn geometry(source: Vec<u8>, expected_events: usize) -> Self {
        Self {
            profile: CommandProfile::TEX82,
            terminal_filename: b"geometry.tex ".to_vec(),
            root_name: "geometry.tex".into(),
            root_bytes: source,
            input_capabilities: BTreeMap::new(),
            fonts: BTreeMap::new(),
            expected_events,
            schema: SchemaVersion::V2,
        }
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "this offline host tool reads fixture bytes after CommittedFixture validation"
    )]
    fn from_fixture(
        directory: &Path,
        fixture: &CommittedFixture,
        resources: &ReplayResources,
    ) -> Result<Self, RunnerError> {
        let root_source = fixture.manifest.root_source.as_str();
        let artifact = fixture.manifest.sources.get(root_source).ok_or_else(|| {
            RunnerError::Replay(format!(
                "{} does not declare its root source {root_source}",
                fixture.manifest.name
            ))
        })?;
        let bytes = std::fs::read(directory.join(&artifact.path)).map_err(|error| {
            RunnerError::Replay(format!(
                "{} source {root_source} cannot be read: {error}",
                fixture.manifest.name
            ))
        })?;
        if u64::try_from(bytes.len()).ok() != Some(artifact.bytes) {
            return Err(RunnerError::Replay(format!(
                "{} source {root_source} changed after fixture validation",
                fixture.manifest.name
            )));
        }

        let mut input_capabilities = BTreeMap::new();
        for (source_name, source_artifact) in &fixture.manifest.sources {
            if source_name == root_source {
                continue;
            }
            let input_name = canonical_input_name(source_name)?;
            let source_bytes =
                std::fs::read(directory.join(&source_artifact.path)).map_err(|error| {
                    RunnerError::Replay(format!(
                        "{} source {source_name} cannot be read: {error}",
                        fixture.manifest.name
                    ))
                })?;
            if u64::try_from(source_bytes.len()).ok() != Some(source_artifact.bytes) {
                return Err(RunnerError::Replay(format!(
                    "{} source {source_name} changed after fixture validation",
                    fixture.manifest.name
                )));
            }
            if input_capabilities
                .insert(input_name.clone(), source_bytes)
                .is_some()
            {
                return Err(RunnerError::Replay(format!(
                    "{} maps multiple registered sources to input capability {input_name}",
                    fixture.manifest.name
                )));
            }
        }

        let mut terminal_filename = root_source.as_bytes().to_vec();
        terminal_filename.push(TERMINAL_FILENAME_TERMINATOR);
        Ok(Self {
            profile: CommandProfile::TEX82,
            terminal_filename,
            root_name: root_source.into(),
            root_bytes: bytes,
            input_capabilities,
            fonts: resources.fonts.clone(),
            expected_events: fixture.stream.events.len(),
            schema: SchemaVersion::try_from(fixture.manifest.oracle.schema).map_err(|error| {
                RunnerError::Replay(format!("unsupported fixture schema: {error}"))
            })?,
        })
    }

    fn replay(self) -> Result<ReplayOutput, RunnerError> {
        self.replay_with_expected(None)
    }

    fn replay_against(
        self,
        expected: &[tex_oracle::NormalizedEvent],
    ) -> Result<ReplayOutput, RunnerError> {
        self.replay_with_expected(Some(expected))
    }

    fn replay_with_expected(
        self,
        expected: Option<&[tex_oracle::NormalizedEvent]>,
    ) -> Result<ReplayOutput, RunnerError> {
        // Replay must terminate even when a defect leaves the engine looping.
        // Registered input bytes alone bound the committed suite's synthetic
        // fixtures, but a real document expands far more commands than it has
        // source bytes, so the expected stream length -- which a correct
        // replay reproduces exactly -- bounds the useful work too.
        let limit = self
            .terminal_filename
            .len()
            .checked_add(self.root_bytes.len())
            .and_then(|count| {
                self.input_capabilities
                    .values()
                    .try_fold(count, |total, source| total.checked_add(source.len()))
            })
            .and_then(|count| count.checked_add(self.expected_events))
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(MAX_DELIVERIES_OVERHEAD))
            .ok_or_else(|| RunnerError::Replay("startup replay bound overflowed".into()))?;
        umber::with_engine_universe(|universe| -> Result<ReplayOutput, RunnerError> {
        // `scripts/build-tex82-document-traces.sh` captures every one of these
        // fixtures with `-interaction=nonstopmode`, so the replay has to run
        // the same job. tex.web §75 starts in `error_stop_mode`, and §82
        // enters §83's dialog on that alone -- against a memory terminal
        // holding nothing but the `**` line, the first recoverable error in a
        // fixture would then reach §71's `fatal_error` and end the replay
        // where the oracle simply scrolled on.
        universe.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        let mut control = MainControl::tex82_initex(universe);
        let command = control.command_mut();
        let terminal = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                self.terminal_filename.clone(),
            ))
            .map_err(|error| {
                RunnerError::Replay(format!("terminal filename cannot register: {error}"))
            })?;
        let root = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::World,
                self.root_bytes.clone(),
            ))
            .map_err(|error| {
                RunnerError::Replay(format!("root source cannot register: {error}"))
            })?;
        // The `**` filename line is read from the terminal, which tex.web
        // §331 opens with `name:=0`; §537's `start_input` is what then opens
        // the root file it names.
        command
            .open_registered_source_as(terminal, SourceNameClass::Terminal)
            .map_err(|error| {
                RunnerError::Replay(format!("terminal filename cannot open: {error}"))
            })?;

        for (name, bytes) in &self.input_capabilities {
            control.capabilities_mut().register_input(
                name,
                SourceRegistration::new(RegisteredSourceKind::World, bytes.clone()),
            );
        }
        // `resolve_font_resource` returns `MissingFont` immediately
        // rather than suspending, so the whole staged metric set is installed
        // before the first step instead of through a retry loop.
        for (name, bytes) in &self.fonts {
            let metrics = universe
                .input_open_context()
                .read_supplied_input_file(
                    Path::new(name),
                    Arc::<[u8]>::from(bytes.clone()).into(),
                )
                .map_err(|error| {
                    RunnerError::Replay(format!("font metrics {name} cannot be supplied: {error}"))
                })?;
            control.capabilities_mut().register_font(
                Path::new(name),
                FontResource::Tfm {
                    metrics,
                    opentype: None,
                },
            );
        }
        let mut recorder = Recorder::new("terminal", self.schema);
        let scanned = control
            .scan_startup_file_name(universe, &mut recorder)
            .map_err(|error| {
                RunnerError::Replay(format!("terminal filename scan failed: {error}"))
            })?;
        if scanned != self.root_name {
            return Err(RunnerError::Replay(format!(
                "terminal filename selected {scanned:?}, not canonical root {:?}",
                self.root_name
            )));
        }
        control
            .command_mut()
            .open_registered_source(root)
            .map_err(|error| RunnerError::Replay(format!("root source cannot open: {error}")))?;
        recorder.record_source_open(CANONICAL_ROOT_PUSH_NAME, &self.root_name, root);
        recorder.activate_source(
            self.root_name.clone(),
            root,
            Arc::<[u8]>::from(self.root_bytes.clone()).into(),
        );

        let mut verified_prefix = 0;
        let mut retained_events = Vec::new();
        retain_after_first_divergence(
            &mut recorder,
            expected,
            &mut verified_prefix,
            &mut retained_events,
        );

        let mut deliveries = 0;
        {
            loop {
                if deliveries == limit {
                    // Contained like any other replay failure: an engine that
                    // will not terminate is a worklist entry, not a reason to
                    // hide every divergence already observed.
                    return Ok(ReplayOutput {
                        verified_prefix,
                        events: finish_retained_events(recorder, retained_events),
                        failure: Some(ReplayFailure::Error(format!(
                            "root source {} exceeded replay bound {limit}",
                            self.root_name
                        ))),
                    });
                }
                let step = catch_panic(std::panic::AssertUnwindSafe(|| {
                    control.step_with_observer(universe, &mut recorder)
                }));
                retain_after_first_divergence(
                    &mut recorder,
                    expected,
                    &mut verified_prefix,
                    &mut retained_events,
                );
                match step {
                    Ok(Ok(MainControlStep::Continue)) => deliveries += 1,
                    Ok(Ok(MainControlStep::End | MainControlStep::EndOfInput)) => break,
                    Ok(Err(error)) => {
                        return Ok(ReplayOutput {
                            verified_prefix,
                            events: finish_retained_events(recorder, retained_events),
                            failure: Some(ReplayFailure::Error(format!(
                                "root source {} replay failed after {deliveries} deliveries: {error}",
                                self.root_name
                            ))),
                        });
                    }
                    Err(message) => {
                        return Ok(ReplayOutput {
                            verified_prefix,
                            events: finish_retained_events(recorder, retained_events),
                            failure: Some(ReplayFailure::Panic(format!(
                                "root source {} panicked after {deliveries} deliveries: {message}",
                                self.root_name
                            ))),
                        });
                    }
                }
            }
        }
        Ok(ReplayOutput {
            verified_prefix,
            events: finish_retained_events(recorder, retained_events),
            failure: None,
        })
        })
        .map_err(|error| RunnerError::Replay(format!("fresh generation: {error:?}")))?
    }
}

fn retain_after_first_divergence(
    recorder: &mut Recorder,
    expected: Option<&[tex_oracle::NormalizedEvent]>,
    verified_prefix: &mut usize,
    retained: &mut Vec<ObservedEvent>,
) {
    retain_events(recorder.take_events(), expected, verified_prefix, retained);
}

fn retain_events(
    events: impl IntoIterator<Item = ObservedEvent>,
    expected: Option<&[tex_oracle::NormalizedEvent]>,
    verified_prefix: &mut usize,
    retained: &mut Vec<ObservedEvent>,
) {
    for event in events {
        if retained.is_empty()
            && expected.is_some_and(|expected| {
                crate::compare::events_match(
                    expected.get(*verified_prefix).map(|event| &event.semantic),
                    Some(&event.event),
                )
            })
        {
            *verified_prefix += 1;
        } else {
            retained.push(event);
        }
    }
}

fn finish_retained_events(
    mut recorder: Recorder,
    mut retained: Vec<ObservedEvent>,
) -> Vec<ObservedEvent> {
    retained.append(&mut recorder.take_events());
    retained
}

/// The fixture's virtual `\\input` namespace is deliberately narrower than
/// host path resolution: each declared source contributes exactly its own
/// `.tex` file name. TeX82 §537 supplies that extension before asking the
/// host to open the file.
fn canonical_input_name(source_name: &str) -> Result<String, RunnerError> {
    if source_name.strip_suffix(".tex").is_none_or(str::is_empty) {
        return Err(RunnerError::Replay(format!(
            "registered fixture source {source_name:?} has no canonical .tex input name"
        )));
    }
    Ok(source_name.to_owned())
}

/// Renders the point where two same-kind events first differ, for the case
/// concise event rendering cannot show: a long payload -- a token list, a macro
/// body, a mutation value -- whose divergence sits past the truncation point,
/// so both printed sides are byte-identical and say nothing.
///
/// Deliberately text-level rather than schema-aware: it works for every event
/// kind and every payload field without enumerating them, and a new schema
/// variant needs no change here. Returns `None` when the renderings are equal
/// (the divergence is not in the payload text at all) or when they already
/// differ inside the concise prefix.
fn hidden_difference_excerpt(expected: &dyn fmt::Debug, actual: &dyn fmt::Debug) -> Option<String> {
    let expected = format!("{expected:?}");
    let actual = format!("{actual:?}");
    let common = expected
        .chars()
        .zip(actual.chars())
        .take_while(|(expected, actual)| expected == actual)
        .count();
    if common == expected.chars().count() && common == actual.chars().count() {
        return None;
    }
    if common < MAX_DIAGNOSTIC_CHARS {
        return None;
    }
    let start = common.saturating_sub(DIFFERENCE_LEAD_CHARS);
    let excerpt = |rendered: &str| {
        let text: String = rendered
            .chars()
            .skip(start)
            .take(DIFFERENCE_LEAD_CHARS + DIFFERENCE_TRAIL_CHARS)
            .collect();
        let tail = if rendered.chars().count() > start + text.chars().count() {
            "…"
        } else {
            ""
        };
        format!("…{text}{tail}")
    };
    Some(format!(
        "\n  first difference at character {common}, past the truncation above:\
         \n    expected: {}\
         \n    actual:   {}",
        excerpt(&expected),
        excerpt(&actual)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{
        CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandState,
    };
    use tex_oracle::{
        CanonicalCommand, CanonicalValue, CommandDelivery, CommandEvent, Event, InputEvent,
        InputReason, MacroEvent, NormalizedEvent, ScannerEvent, SourceLocation,
    };
    use tex_state::token::Token;

    fn committed_fixture() -> CommittedFixture {
        let repository = test_support::repository_root();
        CommittedFixture::load(repository.join(FIXTURE_ROOT).join("command-transitions-v1"))
            .expect("committed TeX82 fixture")
    }

    fn scanner(value: &str) -> Event {
        Event::Scanner(ScannerEvent {
            scanner: "integer".into(),
            result: CanonicalValue::Name(value.into()),
        })
    }
    fn observed(value: &str) -> ObservedEvent {
        ObservedEvent::new(scanner(value), "source=case.tex; input_level=1".into())
    }

    #[test]
    fn committed_command_microfixture_satisfies_automated_bounds() {
        let fixture = committed_fixture();
        validate_automated_fixture(&fixture).expect("focused fixture must remain test-sized");
    }

    #[test]
    fn committed_macro_fixture_keeps_every_trimmed_argument_observation_exact() {
        let repository = test_support::repository_root();
        let fixture_path = repository.join(FIXTURE_ROOT).join("expansion-macros-v1");
        let fixture = CommittedFixture::load(&fixture_path).expect("committed macro fixture");
        let startup = Startup::from_fixture(&fixture_path, &fixture, &ReplayResources::default())
            .expect("macro startup");
        let actual = startup.replay().expect("macro fixture replays");

        let expected_arguments = fixture
            .stream
            .events
            .iter()
            .filter_map(|event| match &event.semantic {
                Event::Macro(argument @ MacroEvent::Argument { .. }) => Some(argument.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let actual_arguments = actual
            .events
            .iter()
            .filter_map(|event| match &event.event {
                Event::Macro(argument @ MacroEvent::Argument { .. }) => Some(argument.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(actual_arguments, expected_arguments);
        let MacroEvent::Argument { tokens, .. } = &expected_arguments[0] else {
            unreachable!("filtered argument")
        };
        assert_eq!(tokens.first().map(|token| token.character), Some(123));
        assert_eq!(tokens.last().map(|token| token.character), Some(125));
        assert_eq!(
            tokens.iter().filter(|token| token.character == 125).count(),
            1,
            "the outer argument's stripped right brace must not reappear"
        );
    }

    #[test]
    fn explicit_repository_resolves_without_ambient_checkout_discovery() {
        let repository = test_support::repository_root();
        let resolved =
            resolve_cli_repository(Some(repository.clone())).expect("resolve explicit repository");
        assert_eq!(resolved, repository);
    }

    #[test]
    fn automated_selection_rejects_a_full_document_footprint() {
        let error = validate_automated_footprint(
            "tex82/accidental-document",
            AutomatedFixtureFootprint {
                sources: 1,
                source_bytes: 4096,
                events: AUTOMATED_MAX_EVENTS + 1,
            },
        )
        .expect_err("a document-scale trace must not enter Cargo tests");
        let message = error.to_string();
        assert!(
            message.contains("accepts only bounded microfixtures"),
            "{message}"
        );
        assert!(
            message.contains("tests/corpus/command/tex82-documents"),
            "{message}"
        );
        assert!(message.contains("--repository ."), "{message}");
        assert!(message.contains("50001 event(s)"), "{message}");
    }

    /// The transport owns no catcode table of its own: it renders whatever
    /// `canonical_names` spells, so a frozen sentinel arrives as a §289
    /// control-sequence token rather than a `Debug` rendering of Umber's enum.
    fn nested_startup() -> Startup {
        Startup {
            profile: CommandProfile::TEX82,
            terminal_filename: b"transitions.tex ".to_vec(),
            root_name: "transitions.tex".into(),
            root_bytes: b"a\\input child b".to_vec(),
            input_capabilities: BTreeMap::from([("child.tex".into(), b"c".to_vec())]),
            fonts: BTreeMap::new(),
            expected_events: 0,
            schema: SchemaVersion::V1,
        }
    }

    #[test]
    fn exact_streams_pass_quietly() {
        let expected = vec![NormalizedEvent {
            sequence: 0,
            semantic: scanner("one"),
        }];
        assert_eq!(
            compare_streams("tex82/exact", &expected, &[observed("one")]),
            Ok(())
        );
    }

    #[test]
    fn exact_prefix_is_released_and_first_mismatch_retains_the_complete_suffix() {
        let expected = vec![
            NormalizedEvent {
                sequence: 0,
                semantic: scanner("one"),
            },
            NormalizedEvent {
                sequence: 1,
                semantic: scanner("two"),
            },
            NormalizedEvent {
                sequence: 2,
                semantic: scanner("three"),
            },
        ];
        let independently_produced = [observed("one"), observed("wrong"), observed("three")];
        let mut verified = 0;
        let mut retained = Vec::new();
        retain_events(
            independently_produced,
            Some(&expected),
            &mut verified,
            &mut retained,
        );

        assert_eq!(verified, 1);
        assert_eq!(retained, [observed("wrong"), observed("three")]);
        let mut mismatch = find_divergences(
            "tex82/streaming",
            &expected[verified..],
            &retained,
            20,
            AlignmentTuning::default(),
        )
        .entries
        .remove(0);
        mismatch.offset_indices(verified);
        assert_eq!(mismatch.index(), 1);
    }

    #[test]
    fn injected_mismatch_reports_earliest_context() {
        let expected = vec![
            NormalizedEvent {
                sequence: 0,
                semantic: scanner("one"),
            },
            NormalizedEvent {
                sequence: 1,
                semantic: scanner("two"),
            },
        ];
        let mismatch = compare_streams(
            "tex82/injected",
            &expected,
            &[observed("zero"), observed("three")],
        )
        .expect_err("mismatch");
        assert_eq!(mismatch.index, 0);
        let report = mismatch.to_string();
        assert!(report.contains("tex82/injected diverged at event 0"));
        assert!(report.contains("source=case.tex"));
        assert!(report.contains("zero"));
        assert!(!report.contains("three"));
    }

    #[test]
    fn macro_call_comparison_projects_only_the_reference_operand() {
        let expected = Event::Command(CommandEvent {
            delivery: CommandDelivery::Raw,
            command: CanonicalCommand {
                command: "call".into(),
                operand: CanonicalValue::Integer(249_985),
                control_sequence: Some("identity".into()),
                location: None,
            },
        });
        let actual = Event::Command(CommandEvent {
            delivery: CommandDelivery::Raw,
            command: CanonicalCommand {
                command: "call".into(),
                operand: CanonicalValue::None,
                control_sequence: Some("identity".into()),
                location: None,
            },
        });
        let expected = vec![NormalizedEvent {
            sequence: 0,
            semantic: expected,
        }];
        let actual = vec![ObservedEvent::new(actual, String::new())];

        assert_eq!(compare_streams("tex82/macro", &expected, &actual), Ok(()));

        let retained_other_reference = vec![ObservedEvent::new(
            Event::Command(CommandEvent {
                delivery: CommandDelivery::Raw,
                command: CanonicalCommand {
                    command: "call".into(),
                    operand: CanonicalValue::Integer(17),
                    control_sequence: Some("identity".into()),
                    location: None,
                },
            }),
            String::new(),
        )];
        assert_eq!(
            compare_streams("tex82/macro", &expected, &retained_other_reference),
            Ok(()),
            "TeX's def_ref address is an allocator reference, not macro semantics"
        );

        let wrong_name = vec![ObservedEvent::new(
            Event::Command(CommandEvent {
                delivery: CommandDelivery::Raw,
                command: CanonicalCommand {
                    command: "call".into(),
                    operand: CanonicalValue::None,
                    control_sequence: Some("other".into()),
                    location: None,
                },
            }),
            String::new(),
        )];
        assert!(compare_streams("tex82/macro", &expected, &wrong_name).is_err());
    }

    #[test]
    fn sparse_register_comparison_projects_only_the_oracle_node_address() {
        let command = |operand| {
            Event::Command(CommandEvent {
                delivery: CommandDelivery::Raw,
                command: CanonicalCommand {
                    command: "register".into(),
                    operand,
                    control_sequence: Some("alias".into()),
                    location: None,
                },
            })
        };
        let expected = vec![NormalizedEvent {
            sequence: 0,
            semantic: command(CanonicalValue::Integer(1_926)),
        }];
        let semantic = vec![ObservedEvent::new(
            command(CanonicalValue::Name("skip:32767".into())),
            String::new(),
        )];
        assert_eq!(compare_streams("etex/sparse", &expected, &semantic), Ok(()));

        let absent = vec![ObservedEvent::new(
            command(CanonicalValue::None),
            String::new(),
        )];
        assert!(compare_streams("etex/sparse", &expected, &absent).is_err());
        let opaque = vec![ObservedEvent::new(
            command(CanonicalValue::Integer(17)),
            String::new(),
        )];
        assert!(compare_streams("etex/sparse", &expected, &opaque).is_err());
    }

    #[test]
    fn startup_matches_the_terminal_scan_before_root_delivery() {
        let fixture = committed_fixture();
        let repository = test_support::repository_root();
        let startup = Startup::from_fixture(
            &repository.join(FIXTURE_ROOT).join("command-transitions-v1"),
            &fixture,
            &ReplayResources::default(),
        )
        .expect("startup");

        assert_eq!(startup.profile, CommandProfile::TEX82);
        assert_eq!(startup.root_name, fixture.manifest.root_source);
        let actual = startup.replay().expect("startup replays");
        let actual_events = actual.events[..40]
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>();
        let expected_events = fixture.stream.events[..40]
            .iter()
            .map(|event| event.semantic.clone())
            .collect::<Vec<_>>();
        assert_eq!(&actual_events[..6], &expected_events[..6]);
        // The observer records the retired level but does not yet retain its
        // replay reason; all remaining terminal-scan and root-open events are
        // canonical and ordered.
        assert_eq!(&actual_events[7..], &expected_events[7..]);
        assert_eq!(
            actual.events[37].context,
            "source=transitions.tex; source_id=1"
        );
        assert_eq!(
            actual.events[38].event, fixture.stream.events[38].semantic,
            "the first command carrying a committed source location matches"
        );
    }

    #[test]
    fn registered_nested_input_retires_and_returns_to_the_caller_deterministically() {
        let first = nested_startup().replay().expect("nested source replays");
        let second = nested_startup()
            .replay()
            .expect("nested source replays again");
        assert_eq!(first, second, "registered input replay must be repeatable");
        assert_eq!(first.failure, None);

        let child_delivery = first
            .events
            .iter()
            .position(|event| event.context.contains("input_level=3"))
            .expect("the child receives its own input level");
        let child_retirement = first
            .events
            .iter()
            .enumerate()
            .skip(child_delivery)
            .find_map(|(index, event)| {
                matches!(
                    &event.event,
                    Event::Input(InputEvent {
                        transition: tex_oracle::InputTransition::Retire,
                        reason: InputReason::Source,
                        ..
                    })
                )
                .then_some(index)
            })
            .expect("the exhausted child source retires");
        assert_eq!(
            first.events[child_retirement].event,
            Event::Input(InputEvent {
                transition: tex_oracle::InputTransition::Retire,
                reason: InputReason::Source,
                // tex.web §537's `start_input` defaults the missing
                // extension to `.tex` before packing the name it opens and
                // prints; the retirement identity must match that name.
                name: "child.tex".into(),
            }),
            "the retirement event retains the child identity before the trace stack pops it"
        );
        assert!(
            first.events[child_retirement + 1..]
                .iter()
                .any(|event| event.context.contains("input_level=2")),
            "input resumes the still-live parent source after child EOF"
        );
        assert!(first.events.iter().any(|event| {
            event.context.starts_with("source=child.tex;")
                && matches!(
                    &event.event,
                    Event::Command(CommandEvent {
                        command: CanonicalCommand {
                            location: Some(SourceLocation { source, .. }),
                            ..
                        },
                        ..
                    }) if source == "child.tex"
                )
        }));
    }

    #[test]
    fn nested_input_reports_a_missing_registered_capability() {
        let mut startup = nested_startup();
        startup.input_capabilities.clear();

        let replay = startup
            .replay()
            .expect("missing input reports typed replay failure");
        assert!(
            !replay
                .events
                .iter()
                .any(|event| event.context.starts_with("source=child;")),
            "missing input must not be opened from the host"
        );
        assert!(
            matches!(&replay.failure, Some(ReplayFailure::Error(message))
            if message.contains("missing token while scanning \\input"))
        );
    }

    #[test]
    fn nested_input_snapshot_rollback_replays_the_same_virtual_source_stack() {
        fn suffix<G>(
            command: &mut CommandState<G>,
            universe: &mut tex_state::Universe<G>,
            capabilities: &mut CommandHostCapabilities,
            fuel: &mut tex_command::CommandFuelLedger,
            diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        ) -> Vec<(char, u64)> {
            let mut context = universe
                .command_context()
                .expect("admitted command context");
            let mut processor = CommandProcessor::new(
                command,
                &mut context,
                CommandHostContext::new(capabilities),
                fuel.fuel_mut(),
                None,
                diagnostic_effects,
            );
            let mut delivered = Vec::new();
            while let Some(current) = processor.get_x_token().expect("nested input replays") {
                if let Token::Char { ch, .. } = current.spelling().semantic_token() {
                    delivered.push((ch, current.delivery_stamp().input_level()));
                }
            }
            delivered
        }

        umber::with_engine_universe(|universe| {
            let mut command = CommandState::new(CommandProfile::TEX82);
            let root = command
                .register_source(SourceRegistration::new(
                    RegisteredSourceKind::World,
                    &b"x\\input child y"[..],
                ))
                .expect("root registers");
            command.open_registered_source(root).expect("root opens");
            tex_command::install_tex82_expandable_primitives(universe);
            let mut capabilities = CommandHostCapabilities::default();
            let mut fuel = tex_command::CommandFuelLedger::default();
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::default();
            capabilities.register_input(
                "child.tex",
                SourceRegistration::new(RegisteredSourceKind::World, &b"c"[..]),
            );

            {
                let mut context = universe
                    .command_context()
                    .expect("admitted command context");
                let mut processor = CommandProcessor::new(
                    &mut command,
                    &mut context,
                    CommandHostContext::new(&mut capabilities),
                    fuel.fuel_mut(),
                    None,
                    &mut diagnostic_effects,
                );
                assert!(matches!(
                    processor
                        .get_x_token()
                        .expect("root starts")
                        .expect("root character")
                        .spelling()
                        .semantic_token(),
                    Token::Char { ch: 'x', .. }
                ));
            }

            let snapshot = command.snapshot(universe).expect("snapshot captures");
            let first = suffix(
                &mut command,
                universe,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );
            command
                .rollback(&snapshot, universe)
                .expect("matching snapshot restores");
            let second = suffix(
                &mut command,
                universe,
                &mut capabilities,
                &mut fuel,
                &mut diagnostic_effects,
            );

            assert_eq!(first, second, "rollback preserves nested source identity");
            assert!(
                first.iter().any(|(_, level)| *level > 0),
                "the nested source receives a distinct input-level identity"
            );
            assert!(first.iter().any(|(_, level)| *level == 0));
        })
        .expect("fresh generation constructs");
    }

    #[test]
    fn startup_rejects_a_stale_root_even_if_its_bytes_are_available() {
        let startup = Startup {
            profile: CommandProfile::TEX82,
            terminal_filename: b"transitions.tex ".to_vec(),
            root_name: "alignment-delivery.tex".into(),
            root_bytes: b"\\relax".to_vec(),
            input_capabilities: BTreeMap::new(),
            fonts: BTreeMap::new(),
            expected_events: 0,
            schema: SchemaVersion::V1,
        };

        let error = startup.replay().expect_err("stale root must not open");
        assert!(
            error
                .to_string()
                .contains("not canonical root \"alignment-delivery.tex\"")
        );
    }
}

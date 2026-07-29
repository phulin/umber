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

use tex_command::canonical_names;
use tex_command::{
    AlignmentRecord, CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile,
    ConditionRecord, EffectRecord, FontResource, GeometryRecord, InputReason as CommandInputReason,
    InputRecord, InputTransition, MacroRecord, MutationRecord, ObservedToken,
    RecoveryKind as CommandRecoveryKind, RecoveryRecord, RegisteredSourceKind, ScannerStatusRecord,
    SourceNameClass, SourceRegistration, TokenListRecord,
};
use tex_exec::{CanonicalMainControl, MainControlStep};
use tex_oracle::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, CommittedFixture, ConditionEvent, ConditionTransition, DiagnosticEvent,
    DiagnosticSeverity, EffectEvent, EffectKind, EngineDialect, Event, GeometryEvent, InputEvent,
    InputReason, MacroEvent, MutationEvent, Normalizer, ObservationHeader, ObservationStream,
    OracleToken, RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus, ScannerStatusEvent,
    SchemaVersion, SourceLocation, StateTarget, Tex82ObserverProfile, TokenListEvent,
    TokenListTransition, validate_tex82_command_trace_suite, validate_tex82_geometry_trace_fixture,
};
use tex_state::{InputOpenState, InputReadState, SourceId, Universe};

pub mod compare;
pub mod documents;
pub mod group;
pub mod report;

pub use compare::{
    AlignmentTuning, Comparison, DEFAULT_ANCHOR_SCAN, DEFAULT_REALIGN_CONFIRMATION,
    DEFAULT_REALIGN_WINDOW, MismatchSides, Repair, ResyncAnchor, StreamMismatch, compare_streams,
    find_divergences,
};
pub use group::{RootSite, group};
pub use report::{ComparisonReport, EXIT_NOT_RUN, FixtureState, FixtureSummary, RunOutcome};

const FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const MAX_DIAGNOSTIC_CHARS: usize = 960;
/// Characters of already-agreeing context shown before the first differing
/// character, and of divergent text shown after it, by
/// [`hidden_difference_excerpt`].
const DIFFERENCE_LEAD_CHARS: usize = 120;
const DIFFERENCE_TRAIL_CHARS: usize = 360;
const MAX_DELIVERIES_OVERHEAD: usize = 64;
const CANONICAL_ROOT_SOURCE: &str = "transitions.tex";
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
            &ReplayResources::committed(),
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
             `cargo run-dev -q -p tex-command-stream -- --repository .` diagnostic",
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
    let replay =
        CanonicalStartup::geometry(fixture.source, fixture.stream.events.len()).replay()?;
    let actual = replay
        .events
        .into_iter()
        .filter(|event| matches!(event.event, Event::Geometry(_)))
        .collect::<Vec<_>>();
    let actual_events = actual.len();
    let identity = format!("{} projection={}", fixture.selector, fixture.identity);
    let comparison = find_divergences(
        &identity,
        &fixture.stream.events,
        &actual,
        options.max_divergences,
        options.alignment,
    );
    let first = report.divergences.len();
    let budgeted = comparison.entries.len();
    report.divergences.extend(
        comparison
            .entries
            .into_iter()
            .map(Box::new)
            .map(Divergence::Mismatch),
    );
    if let Some(failure) = replay.failure {
        report.divergences.push(Divergence::Failure {
            fixture: identity.clone(),
            index: actual_events,
            failure,
        });
    }
    report.fixtures.push(FixtureSummary {
        name: fixture.selector,
        identity,
        state: FixtureState::Compared {
            divergences: report.divergences.len() - first,
            budgeted,
            first_index: report.divergences.get(first).map(Divergence::index),
            budget_reached: comparison.budget_reached,
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
    for trace in &registry.traces {
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
    let comparison = find_divergences(
        &identity,
        &fixture.stream.events,
        &replay.events,
        options.max_divergences,
        options.alignment,
    );
    let first = report.divergences.len();
    // The budget counts these and only these; the contained failure below is
    // outside it, so the two are recorded separately rather than summed.
    let budgeted = comparison.entries.len();
    report.divergences.extend(
        comparison
            .entries
            .into_iter()
            .map(Box::new)
            .map(Divergence::Mismatch),
    );
    // A contained failure is at most one entry per fixture and names a
    // concrete `ExecError` or panic site, so it is reported outside the
    // mismatch budget: the twentieth consecutive mismatch of an
    // already-reported structural defect must never crowd it out.
    if let Some(failure) = replay.failure {
        report.divergences.push(Divergence::Failure {
            fixture: identity.clone(),
            index: replay.events.len(),
            failure,
        });
    }
    report.fixtures.push(FixtureSummary {
        name: fixture.manifest.name.clone(),
        identity,
        state: FixtureState::Compared {
            divergences: report.divergences.len() - first,
            budgeted,
            first_index: report.divergences.get(first).map(Divergence::index),
            budget_reached: comparison.budget_reached,
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
    let mut repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut options = RunOptions::default();
    while let Some(argument) = arguments.next() {
        if argument == "--repository" {
            repository = arguments
                .next()
                .ok_or_else(|| {
                    RunnerError::Usage("--repository requires a directory argument".into())
                })?
                .into();
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
    run_repository(repository, options)
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

/// One translated observer event plus source/provenance-only diagnostic context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedEvent {
    event: Event,
    context: String,
}

impl ObservedEvent {
    fn new(event: Event, context: String) -> Self {
        Self { event, context }
    }

    /// Portable semantic value, without host-only diagnostic context.
    #[must_use]
    pub fn semantic(&self) -> &Event {
        &self.event
    }

    /// Source and delivery context retained for divergence reporting.
    #[must_use]
    pub fn context(&self) -> &str {
        &self.context
    }
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
    /// A command-core `ExecError`/`CanonicalSessionError` the processor
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
    CanonicalStartup::from_fixture(directory, fixture, resources)?.replay()
}

/// Replay inputs a fixture needs that its committed `.tex` sources do not
/// carry: which declared source TeX's terminal filename scan selects, and the
/// opaque font metrics canonical `\font` resolution must find already
/// registered.
///
/// `CanonicalMainControl::resolve_font_resource` never suspends -- an
/// unregistered font is an immediate `ExecError::MissingCanonicalFont` -- so
/// every font a document can reach is registered up front, before the first
/// step, rather than through a lazy resource-host retry loop.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplayResources {
    root_source: String,
    fonts: BTreeMap<String, Arc<[u8]>>,
}

impl ReplayResources {
    /// The committed suite's convention: a fixed root-source name and
    /// font-independent sources.
    fn committed() -> Self {
        Self {
            root_source: CANONICAL_ROOT_SOURCE.into(),
            fonts: BTreeMap::new(),
        }
    }
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
struct CanonicalStartup {
    profile: CommandProfile,
    terminal_filename: Arc<[u8]>,
    root_name: String,
    root_bytes: Arc<[u8]>,
    input_capabilities: BTreeMap<String, Arc<[u8]>>,
    fonts: BTreeMap<String, Arc<[u8]>>,
    expected_events: usize,
    schema: SchemaVersion,
}

impl CanonicalStartup {
    fn geometry(source: Vec<u8>, expected_events: usize) -> Self {
        Self {
            profile: CommandProfile::TEX82,
            terminal_filename: Arc::from(&b"geometry.tex "[..]),
            root_name: "geometry.tex".into(),
            root_bytes: Arc::from(source),
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
        let root_source = resources.root_source.as_str();
        let artifact = fixture.manifest.sources.get(root_source).ok_or_else(|| {
            RunnerError::Replay(format!(
                "{} does not declare canonical root source {root_source}",
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
                .insert(input_name.clone(), Arc::from(source_bytes))
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
            terminal_filename: Arc::from(terminal_filename),
            root_name: root_source.into(),
            root_bytes: Arc::from(bytes),
            input_capabilities,
            fonts: resources.fonts.clone(),
            expected_events: fixture.stream.events.len(),
            schema: SchemaVersion::try_from(fixture.manifest.oracle.schema).map_err(|error| {
                RunnerError::Replay(format!("unsupported fixture schema: {error}"))
            })?,
        })
    }

    fn replay(self) -> Result<ReplayOutput, RunnerError> {
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
            .ok_or_else(|| {
                RunnerError::Replay("canonical startup replay bound overflowed".into())
            })?;
        let mut universe = Universe::new();
        let mut control = CanonicalMainControl::tex82_initex(&mut universe);
        let command = control.command_mut();
        // Source IDs are part of the command state's durable input identity:
        // terminal is always 0 and the selected root is always 1.
        let terminal = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::clone(&self.terminal_filename),
            ))
            .map_err(|error| {
                RunnerError::Replay(format!("terminal filename cannot register: {error}"))
            })?;
        let root = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::World,
                Arc::clone(&self.root_bytes),
            ))
            .map_err(|error| {
                RunnerError::Replay(format!("root source cannot register: {error}"))
            })?;
        if terminal != SourceId::new(0) || root != SourceId::new(1) {
            return Err(RunnerError::Replay(
                "canonical startup assigned non-deterministic source identities".into(),
            ));
        }
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
                SourceRegistration::new(RegisteredSourceKind::World, Arc::clone(bytes)),
            );
        }
        // `resolve_font_resource` returns `MissingCanonicalFont` immediately
        // rather than suspending, so the whole staged metric set is installed
        // before the first step instead of through a retry loop.
        for (name, bytes) in &self.fonts {
            let metrics = universe
                .input_open_context()
                .read_supplied_input_file(Path::new(name), Arc::clone(bytes))
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
        let mut recorder = Recorder::new("terminal", self.input_capabilities, self.schema);
        let scanned = control
            .scan_startup_file_name(&mut universe, &mut recorder)
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
        recorder.activate_source(self.root_name.clone(), root, Arc::clone(&self.root_bytes));

        let mut deliveries = 0;
        {
            loop {
                if deliveries == limit {
                    // Contained like any other replay failure: an engine that
                    // will not terminate is a worklist entry, not a reason to
                    // hide every divergence already observed.
                    return Ok(ReplayOutput {
                        events: recorder.events,
                        failure: Some(ReplayFailure::Error(format!(
                            "root source {} exceeded replay bound {limit}",
                            self.root_name
                        ))),
                    });
                }
                let step = catch_panic(std::panic::AssertUnwindSafe(|| {
                    control.step_with_observer(&mut universe, &mut recorder)
                }));
                match step {
                    Ok(Ok(MainControlStep::Continue)) => deliveries += 1,
                    Ok(Ok(MainControlStep::End | MainControlStep::EndOfInput)) => break,
                    Ok(Err(error)) => {
                        return Ok(ReplayOutput {
                            events: recorder.events,
                            failure: Some(ReplayFailure::Error(format!(
                                "root source {} replay failed after {deliveries} deliveries: {error}",
                                self.root_name
                            ))),
                        });
                    }
                    Err(message) => {
                        return Ok(ReplayOutput {
                            events: recorder.events,
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
            events: recorder.events,
            failure: None,
        })
    }
}

/// The fixture's virtual `\\input` namespace is deliberately narrower than
/// host path resolution: each declared `.tex` source contributes exactly its
/// extensionless logical input name.
fn canonical_input_name(source_name: &str) -> Result<String, RunnerError> {
    source_name
        .strip_suffix(".tex")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            RunnerError::Replay(format!(
                "registered fixture source {source_name:?} has no canonical .tex input name"
            ))
        })
}

/// Immutable source material needed to translate command provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSource {
    pub name: String,
    pub source: SourceId,
    pub bytes: Arc<[u8]>,
}

/// Terminal state supplied by the host after normal engine execution returns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveSessionOutcome {
    Completed,
    Failed { diagnostic: String, detail: String },
}

/// Canonical full diagnostic stream and its stable TRIP-profile projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveSessionStreams {
    pub diagnostic: Vec<u8>,
    pub stable: Vec<u8>,
}

/// Host-side translation of captured normal-session observations.
///
/// This owns only detached oracle transport state. It neither drives nor
/// mutates a `CanonicalEngineSession`; callers may hand it observations after
/// the engine has returned, including after an early failure.
pub struct LiveSessionTranslator {
    sources: Vec<ActiveSource>,
    registered_inputs: BTreeMap<String, Arc<[u8]>>,
    next_registered_source: u32,
    alignment_nesting: AlignmentNesting,
    events: Vec<ObservedEvent>,
    geometry: bool,
    preserve_macro_reference_operands: bool,
}

type Recorder = LiveSessionTranslator;

struct ActiveSource {
    name: String,
    source: Option<SourceId>,
    bytes: Arc<[u8]>,
    /// Physical line starts, calculated once when the source becomes active.
    ///
    /// Command observation needs a line and byte-column for every direct
    /// source delivery. Recounting newlines in the source prefix for each of
    /// those deliveries made provenance translation scale quadratically with
    /// the document length.
    line_starts: Arc<[usize]>,
}

impl LiveSessionTranslator {
    fn new(
        source: impl Into<String>,
        registered_inputs: BTreeMap<String, Arc<[u8]>>,
        schema: SchemaVersion,
    ) -> Self {
        Self {
            sources: vec![ActiveSource {
                name: source.into(),
                source: None,
                bytes: Arc::from(&b""[..]),
                line_starts: Arc::from([0]),
            }],
            registered_inputs,
            next_registered_source: 2,
            alignment_nesting: AlignmentNesting::default(),
            events: Vec::new(),
            geometry: schema == SchemaVersion::V2,
            preserve_macro_reference_operands: false,
        }
    }

    /// Creates a translator for an already-open root source.
    #[must_use]
    pub fn for_root(
        schema: SchemaVersion,
        terminal_name: impl Into<String>,
        root: LiveSource,
        registered_inputs: BTreeMap<String, Arc<[u8]>>,
    ) -> Self {
        let next_registered_source = root.source.raw().saturating_add(1);
        let mut translator = Self::new(terminal_name, registered_inputs, schema);
        translator.preserve_macro_reference_operands = true;
        translator.next_registered_source = next_registered_source;
        translator.activate_source(root.name, root.source, root.bytes);
        translator
    }

    /// Translates a captured committed observation sequence exactly once.
    pub fn translate_captured(
        &mut self,
        observations: impl IntoIterator<Item = CommandObservation>,
    ) {
        for observation in observations {
            self.committed(observation);
        }
    }

    /// Finalizes both the full diagnostic stream and the byte-identical stable
    /// TRIP projection under the caller-supplied pinned stream header.
    pub fn finish(
        mut self,
        header: ObservationHeader,
        outcome: LiveSessionOutcome,
    ) -> Result<LiveSessionStreams, String> {
        let schema = SchemaVersion::try_from(header.schema)?;
        if schema != SchemaVersion::V1 {
            return Err("live diagnostic translation currently requires schema v1".into());
        }
        if let LiveSessionOutcome::Failed { diagnostic, detail } = outcome {
            self.events.push(ObservedEvent::new(
                Event::Diagnostic(DiagnosticEvent {
                    severity: DiagnosticSeverity::Fatal,
                    diagnostic,
                    arguments: vec![CanonicalValue::Name(detail)],
                }),
                "source=host; terminal_outcome=failure".into(),
            ));
            self.ensure_terminated();
        }
        let diagnostic =
            encode_observed_stream(&header, self.events.iter().map(|event| &event.event))?;
        let stable_events = self.events.iter().filter_map(|event| match &event.event {
            Event::Effect(effect)
                if matches!(effect.kind, EffectKind::Shipout | EffectKind::Terminate) =>
            {
                Some(&event.event)
            }
            Event::Input(input)
                if input.transition == tex_oracle::InputTransition::Stop
                    && input.reason == InputReason::Source
                    && input.name == "terminal" =>
            {
                Some(&event.event)
            }
            _ => None,
        });
        let stable = encode_observed_stream(&header, stable_events)?;
        let decoded = ObservationStream::from_canonical_json_lines(&stable)
            .map_err(|error| error.to_string())?;
        Tex82ObserverProfile::Trip.validate(&decoded)?;
        Ok(LiveSessionStreams { diagnostic, stable })
    }

    fn ensure_terminated(&mut self) {
        let ends_in_stop = self.events.last().is_some_and(|event| {
            matches!(
                &event.event,
                Event::Input(input)
                    if input.transition == tex_oracle::InputTransition::Stop
                        && input.reason == InputReason::Source
                        && input.name == "terminal"
            )
        });
        let ends_in_termination = self.events.last().is_some_and(|event| {
            matches!(
                &event.event,
                Event::Effect(effect)
                    if effect.kind == EffectKind::Terminate && effect.channel == "engine"
            )
        });
        if !ends_in_stop && !ends_in_termination {
            self.events.push(ObservedEvent::new(
                Event::Input(InputEvent {
                    transition: tex_oracle::InputTransition::Stop,
                    reason: InputReason::Source,
                    name: "terminal".into(),
                }),
                "source=terminal; terminal_outcome=failure".into(),
            ));
        }
        if !ends_in_termination {
            self.events.push(ObservedEvent::new(
                Event::Effect(EffectEvent {
                    kind: EffectKind::Terminate,
                    channel: "engine".into(),
                    value: CanonicalValue::None,
                }),
                "source=terminal; terminal_outcome=failure".into(),
            ));
        }
    }

    /// Records the harness's completed source-open operation. This is an
    /// actual typed startup transition, not an expected-event reconstruction.
    fn record_source_open(&mut self, trace_name: &str, root_name: &str, source: SourceId) {
        self.events.push(ObservedEvent::new(
            Event::Input(InputEvent {
                transition: tex_oracle::InputTransition::Push,
                reason: InputReason::Source,
                name: trace_name.into(),
            }),
            format!("source={root_name}; source_id={}", source.raw()),
        ));
    }

    fn activate_source(&mut self, name: impl Into<String>, source: SourceId, bytes: Arc<[u8]>) {
        let line_starts = source_line_starts(&bytes);
        self.sources.push(ActiveSource {
            name: name.into(),
            source: Some(source),
            bytes,
            line_starts,
        });
    }

    fn activate_registered_input(&mut self, name: &str) {
        let Some(bytes) = self.registered_inputs.get(name).cloned() else {
            return;
        };
        let source = SourceId::new(self.next_registered_source);
        self.next_registered_source += 1;
        self.record_source_open(CANONICAL_ROOT_PUSH_NAME, name, source);
        self.activate_source(canonical_trace_source_name(name), source, bytes);
    }

    fn current_source(&self) -> &ActiveSource {
        self.sources
            .last()
            .expect("terminal source is always active during replay")
    }

    fn retire_current_source(&mut self) {
        if self.sources.len() > 1 {
            self.sources.pop();
        }
    }
}

fn encode_observed_stream<'a>(
    header: &ObservationHeader,
    events: impl IntoIterator<Item = &'a Event>,
) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(header).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    let mut normalizer = Normalizer::new();
    for event in events {
        bytes.extend_from_slice(
            &serde_json::to_vec(&normalizer.normalize(event.clone()))
                .map_err(|error| error.to_string())?,
        );
        bytes.push(b'\n');
    }
    ObservationStream::from_canonical_json_lines(&bytes).map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Recovers TeX82's `.tex`-defaulted display name for an `\input` target
/// scanned without an explicit extension.
///
/// tex.web §537's `start_input` applies `if cur_ext="" then cur_ext:=".tex"`
/// before packing the name it opens and later prints (`slow_print(name)`).
/// The fixture registry keys registered inputs by the bare stem actually
/// typed in source (`\input case-shift`), so the trace must independently
/// reapply that same default rather than special-casing individual fixture
/// file stems.
fn canonical_trace_source_name(name: &str) -> String {
    if name.contains('.') {
        name.into()
    } else {
        format!("{name}.tex")
    }
}

impl CommandObserver for Recorder {
    fn observes_geometry(&self) -> bool {
        self.geometry
    }

    fn committed(&mut self, observation: CommandObservation) {
        if matches!(observation, CommandObservation::Geometry(_)) && !self.geometry {
            return;
        }
        if let CommandObservation::Effect(EffectRecord {
            kind: "input",
            detail,
            ..
        }) = &observation
        {
            // The effect carries the command-core capability hand-off, while
            // the portable trace observes only the resulting source push.
            self.activate_registered_input(detail);
            return;
        }
        let (source_name, source_id, source_bytes, source_line_starts) = {
            let source = self.current_source();
            (
                source.name.clone(),
                source.source,
                Arc::clone(&source.bytes),
                Arc::clone(&source.line_starts),
            )
        };
        self.events.push(translate_observation(
            &source_name,
            source_id,
            Some(&source_bytes),
            Some(&source_line_starts),
            observation.clone(),
            &mut self.alignment_nesting,
            self.preserve_macro_reference_operands,
        ));
        if let CommandObservation::Input(InputRecord {
            transition: InputTransition::Retire,
            reason: CommandInputReason::Source,
            ..
        }) = observation
        {
            self.retire_current_source();
        }
    }
}

fn translate_observation(
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    source_line_starts: Option<&[usize]>,
    observation: CommandObservation,
    alignment_nesting: &mut AlignmentNesting,
    preserve_macro_reference_operands: bool,
) -> ObservedEvent {
    match observation {
        CommandObservation::Command(record) => {
            let provenance = record.provenance;
            let context = format!(
                "source={source}; input_level={}; position={}; delivery_sequence={}; has_origin={}",
                provenance.input_level,
                provenance.position,
                provenance.delivery_sequence,
                provenance.has_origin
            );
            let (mut operand, control_sequence) = command_token(&record.spelling);
            if let Some(command_operand) = record.command_operand
                && (preserve_macro_reference_operands
                    || !matches!(
                        record.command.as_str(),
                        "call" | "long_call" | "outer_call" | "long_outer_call"
                    ))
            {
                operand = CanonicalValue::Integer(command_operand);
            }
            ObservedEvent::new(
                Event::Command(CommandEvent {
                    delivery: match record.boundary {
                        CommandDeliveryBoundary::Raw => CommandDelivery::Raw,
                        CommandDeliveryBoundary::Expanded => CommandDelivery::Expanded,
                    },
                    command: CanonicalCommand {
                        // `canonical_command_identity` already named this
                        // delivery from its *effective* meaning, which is what
                        // §23's outer-validity recovery changes without
                        // changing the spelling. Re-deriving a name from the
                        // spelling here would be a second, divergent table.
                        command: record.command.clone(),
                        operand,
                        control_sequence,
                        location: command_location(
                            &record,
                            source,
                            source_id,
                            source_bytes,
                            source_line_starts,
                        ),
                    },
                }),
                context,
            )
        }
        CommandObservation::Input(record) => {
            let context = format!(
                "source={source}; level={}; position={}",
                record.level, record.position
            );
            ObservedEvent::new(translate_input(record, source), context)
        }
        CommandObservation::Recovery(record) => {
            ObservedEvent::new(translate_recovery(record), format!("source={source}"))
        }
        CommandObservation::ScannerStatus(record) => {
            ObservedEvent::new(translate_status(record), format!("source={source}"))
        }
        CommandObservation::Macro(record) => {
            let context = format!("source={source}; definition={}", record.definition);
            ObservedEvent::new(translate_macro(record), context)
        }
        CommandObservation::Condition(record) => {
            let context = format!("source={source}; condition={}", record.identity);
            ObservedEvent::new(translate_condition(record), context)
        }
        CommandObservation::Scanner(record) => {
            let result = if let Some(tokens) = record.tokens {
                CanonicalValue::Tokens(tokens.into_iter().map(oracle_token).collect())
            } else if record.kind == "internal" {
                if let Some(value) = record.value.strip_prefix("scaled:") {
                    value.parse::<i64>().map_or_else(
                        |_| CanonicalValue::Name(record.value),
                        CanonicalValue::Scaled,
                    )
                } else if let Some(value) = record.value.strip_prefix("glue:") {
                    parse_glue_scanner_value(value).unwrap_or(CanonicalValue::Name(record.value))
                } else {
                    record.value.parse::<i64>().map_or_else(
                        |_| CanonicalValue::Name(record.value),
                        CanonicalValue::Integer,
                    )
                }
            } else if matches!(
                record.kind,
                "integer" | "interaction_mode" | "expression_integer"
            ) {
                record.value.parse::<i64>().map_or_else(
                    |_| CanonicalValue::Name(record.value),
                    CanonicalValue::Integer,
                )
            } else if matches!(record.kind, "dimension" | "expression_dimension") {
                record.value.parse::<i64>().map_or_else(
                    |_| CanonicalValue::Name(record.value),
                    CanonicalValue::Scaled,
                )
            } else if matches!(
                record.kind,
                "glue" | "expression_glue" | "expression_muglue" | "mu_to_glue" | "glue_to_mu"
            ) {
                parse_glue_scanner_value(&record.value)
                    .unwrap_or(CanonicalValue::Name(record.value))
            } else {
                CanonicalValue::Name(record.value)
            };
            ObservedEvent::new(
                Event::Scanner(ScannerEvent {
                    scanner: record.kind.into(),
                    result,
                }),
                format!("source={source}"),
            )
        }
        CommandObservation::TokenList(record) => {
            ObservedEvent::new(translate_token_list(record), format!("source={source}"))
        }
        CommandObservation::Alignment(record) => {
            let nesting = alignment_nesting.observe(&record);
            let context = format!("source={source}; nesting={nesting:?}");
            ObservedEvent::new(translate_alignment(record, nesting), context)
        }
        CommandObservation::Mutation(record) => {
            ObservedEvent::new(translate_mutation(record), format!("source={source}"))
        }
        CommandObservation::Diagnostic(record) => ObservedEvent::new(
            Event::Diagnostic(DiagnosticEvent {
                severity: match record.severity {
                    "note" => DiagnosticSeverity::Note,
                    "warning" => DiagnosticSeverity::Warning,
                    "fatal" => DiagnosticSeverity::Fatal,
                    _ => DiagnosticSeverity::Error,
                },
                diagnostic: record.diagnostic.into(),
                arguments: record
                    .arguments
                    .iter()
                    .cloned()
                    .map(|argument| match argument {
                        tex_command::DiagnosticArgument::Token(token) => {
                            CanonicalValue::Token(oracle_token(token))
                        }
                        tex_command::DiagnosticArgument::Name(name) => CanonicalValue::Name(name),
                    })
                    .collect(),
            }),
            format!("source={source}"),
        ),
        CommandObservation::Effect(record) => {
            ObservedEvent::new(translate_effect(record), format!("source={source}"))
        }
        CommandObservation::Geometry(record) => ObservedEvent::new(
            Event::Geometry(match record {
                GeometryRecord::Hpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                } => GeometryEvent::Hpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                },
                GeometryRecord::Vpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                } => GeometryEvent::Vpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                },
                GeometryRecord::Shipout {
                    page_width_sp,
                    page_height_sp,
                    counts,
                } => GeometryEvent::Shipout {
                    page_width_sp,
                    page_height_sp,
                    counts,
                },
            }),
            format!("source={source}"),
        ),
    }
}

fn parse_glue_scanner_value(value: &str) -> Option<CanonicalValue> {
    let mut fields = value.split(';').map(|field| field.split_once('='));
    let width = fields.next()??.1.parse().ok()?;
    let stretch = fields.next()??.1.parse().ok()?;
    // The producer already spelled §135's order through `canonical_names`;
    // this must not re-case or otherwise reinterpret a canonical name.
    let stretch_order = fields.next()??.1.to_owned();
    let shrink = fields.next()??.1.parse().ok()?;
    let shrink_order = fields.next()??.1.to_owned();
    if fields.next().is_some() {
        return None;
    }
    Some(CanonicalValue::Glue {
        width,
        stretch,
        stretch_order,
        shrink,
        shrink_order,
    })
}

fn command_location(
    record: &tex_command::CommandDeliveryRecord,
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    source_line_starts: Option<&[usize]>,
) -> Option<SourceLocation> {
    let location = record.provenance.source_location?;
    if Some(location.source()) != source_id {
        return None;
    }
    let bytes = source_bytes?;
    let line_starts = source_line_starts?;
    let byte = usize::try_from(location.byte()).ok()?;
    bytes.get(..byte)?;
    let line_index = line_starts.partition_point(|start| *start <= byte);
    let line_start = *line_starts.get(line_index.checked_sub(1)?)?;
    Some(SourceLocation {
        source: source.into(),
        line: u32::try_from(line_index).ok()?,
        byte: u32::try_from(byte.checked_sub(line_start)?).ok()?,
    })
}

fn source_line_starts(bytes: &[u8]) -> Arc<[usize]> {
    let mut starts = Vec::with_capacity(bytes.iter().filter(|&&byte| byte == b'\n').count() + 1);
    starts.push(0);
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    starts.into()
}

/// The spelling and typed operand a delivered command carries.
///
/// A character command carries its character code; a control sequence carries
/// its spelling. Every other spelling is a §289 token-only or frozen sentinel,
/// which `canonical_names` names for both fields -- this must never fall back
/// to a `Debug` rendering of Umber's own enum (`umber2-johp.141`).
fn command_token(token: &ObservedToken) -> (CanonicalValue, Option<String>) {
    match token {
        ObservedToken::Character { character, .. } => (
            CanonicalValue::Integer(i64::from(u32::from(*character))),
            None,
        ),
        ObservedToken::ControlSequence(name) => (CanonicalValue::None, Some(name.clone())),
        token => (
            CanonicalValue::None,
            canonical_names::observed_token_control_sequence(token).map(str::to_owned),
        ),
    }
}

fn oracle_token(token: ObservedToken) -> OracleToken {
    OracleToken {
        character: canonical_names::observed_token_character(&token),
        catcode: canonical_names::observed_token_catcode(&token).into(),
        control_sequence: canonical_names::observed_token_control_sequence(&token)
            .map(str::to_owned),
        location: None,
    }
}

fn translate_input(record: InputRecord, active_source: &str) -> Event {
    let transition = match record.transition {
        InputTransition::Push => tex_oracle::InputTransition::Push,
        InputTransition::Retire => tex_oracle::InputTransition::Retire,
        InputTransition::Stop => tex_oracle::InputTransition::Stop,
        InputTransition::Backup | InputTransition::Recovery => tex_oracle::InputTransition::Push,
    };
    // The reference instrumentation derives the coarse `reason` from the same
    // tex.web §307 `token_type` it names the level with: `backed_up` reports
    // `backup`, `inserted` reports `recovery`, `macro` reports `macro`, the
    // two templates report `alignment_template`, and every remaining token
    // type -- `parameter`, `output_text`, the `every_*` hooks, `mark_text`,
    // and `write_text` -- reports `token_list`.
    let reason = match record.reason {
        CommandInputReason::Source => InputReason::Source,
        CommandInputReason::Backup => InputReason::Backup,
        CommandInputReason::Macro => InputReason::Macro,
        CommandInputReason::AlignmentUTemplate | CommandInputReason::AlignmentVTemplate => {
            InputReason::AlignmentTemplate
        }
        CommandInputReason::Recovery => InputReason::Recovery,
        CommandInputReason::Parameter
        | CommandInputReason::OutputRoutine
        | CommandInputReason::EveryPar
        | CommandInputReason::EveryMath
        | CommandInputReason::EveryDisplay
        | CommandInputReason::EveryHBox
        | CommandInputReason::EveryVBox
        | CommandInputReason::EveryJob
        | CommandInputReason::EveryCr
        | CommandInputReason::Mark
        | CommandInputReason::Write
        | CommandInputReason::UmberReplay(_) => InputReason::TokenList,
    };
    Event::Input(InputEvent {
        transition,
        reason,
        // TeX82's `end_file_reading` observer carries only the lifecycle
        // transition.  The harness attaches the source identity while the
        // source frame is still active, before it removes that frame from
        // its parallel trace stack.
        name: canonical_names::input_level_name(record.reason)
            .map_or_else(|| active_source.into(), Into::into),
    })
}

fn translate_recovery(record: RecoveryRecord) -> Event {
    Event::Recovery(RecoveryEvent {
        kind: match record.kind {
            CommandRecoveryKind::Backup => RecoveryKind::Backup,
            CommandRecoveryKind::InsertedToken => RecoveryKind::InsertedToken,
            CommandRecoveryKind::InsertedControlSequence => RecoveryKind::InsertedControlSequence,
        },
        tokens: record.tokens.into_iter().map(oracle_token).collect(),
    })
}
fn translate_status(record: ScannerStatusRecord) -> Event {
    Event::ScannerStatus(ScannerStatusEvent {
        from: scanner_status(record.from),
        to: scanner_status(record.to),
    })
}
/// Maps tex.web §305's `scanner_status` name onto the schema's enum.
///
/// The record already carries the canonical name, so this is an exact match on
/// the five non-normal values, never a prefix test against a `Debug` rendering
/// of Umber's own variants (`umber2-johp.141`).
fn scanner_status(status: &str) -> ScannerStatus {
    match status {
        "skipping" => ScannerStatus::Skipping,
        "defining" => ScannerStatus::Defining,
        "matching" => ScannerStatus::Matching,
        "aligning" => ScannerStatus::Aligning,
        "absorbing" => ScannerStatus::Absorbing,
        _ => ScannerStatus::Normal,
    }
}
fn translate_macro(record: MacroRecord) -> Event {
    if record.activation {
        Event::Macro(MacroEvent::Activation {
            control_sequence: record
                .control_sequence
                .unwrap_or_else(|| format!("definition-{}", record.definition)),
            argument_count: record.argument.unwrap_or(0).into(),
        })
    } else {
        Event::Macro(MacroEvent::Argument {
            parameter: record.argument.unwrap_or(0).into(),
            tokens: record.tokens.into_iter().map(oracle_token).collect(),
        })
    }
}
fn translate_condition(record: ConditionRecord) -> Event {
    let transition = match record.transition {
        "push" => ConditionTransition::Push,
        "limit" => ConditionTransition::LimitChange,
        "branch" => ConditionTransition::Branch,
        _ => ConditionTransition::Pop,
    };
    Event::Condition(ConditionEvent {
        transition,
        condition: record.condition.into(),
        limit: record.limit.into(),
        branch: record.branch,
    })
}
fn translate_token_list(record: TokenListRecord) -> Event {
    Event::TokenList(TokenListEvent {
        transition: if record.transition == "splice" {
            TokenListTransition::Splice
        } else {
            TokenListTransition::Complete
        },
        purpose: record.purpose.into(),
        tokens: record.tokens.into_iter().map(oracle_token).collect(),
    })
}
/// Projects TeX82's `align_ptr` stack onto the portable one-based nesting
/// field. Alignment identities are process-local replay handles: §37's
/// `fin_align` calls `pop_alignment`, so a later independent alignment can
/// have a larger identity while returning to nesting one.
#[derive(Debug, Default)]
struct AlignmentNesting {
    stack: Vec<u64>,
}

impl AlignmentNesting {
    fn observe(&mut self, record: &AlignmentRecord) -> Option<u32> {
        let identity = record.alignment?;
        match record.transition {
            "begin" => {
                self.stack.push(identity);
                Self::depth(self.stack.len())
            }
            "finish" => {
                let nesting = Self::depth(self.stack.len());
                if self.stack.last() == Some(&identity) {
                    self.stack.pop();
                }
                nesting
            }
            "suspend" | "resume" => {
                debug_assert_eq!(self.stack.last(), Some(&identity));
                Self::depth(self.stack.len())
            }
            _ => self
                .stack
                .iter()
                .rposition(|active| *active == identity)
                .and_then(|index| Self::depth(index + 1)),
        }
    }

    fn depth(depth: usize) -> Option<u32> {
        u32::try_from(depth).ok().filter(|depth| *depth != 0)
    }
}

fn translate_alignment(record: AlignmentRecord, nesting: Option<u32>) -> Event {
    Event::Alignment(AlignmentEvent {
        transition: match record.transition {
            "begin" => AlignmentTransition::Begin,
            "finish" => AlignmentTransition::Finish,
            "suspend" => AlignmentTransition::Suspend,
            "resume" => AlignmentTransition::Resume,
            "preamble_start" => AlignmentTransition::PreambleStart,
            "preamble_finish" => AlignmentTransition::PreambleFinish,
            "begin_group" | "end_group" => AlignmentTransition::StateChange,
            "state_change" => AlignmentTransition::StateChange,
            "template_push" | "u_template_push" | "v_template_push" | "omit_template_push" => {
                AlignmentTransition::TemplatePush
            }
            "template_retire"
            | "u_template_retire"
            | "v_template_retire"
            | "omit_template_retire" => AlignmentTransition::TemplateRetire,
            "delimiter" => AlignmentTransition::Delimiter,
            "backup_correction" => AlignmentTransition::BackupCorrection,
            _ => AlignmentTransition::Recovery,
        },
        align_state: i64::from(record.align_state),
        template: match record.transition {
            "u_template_push" | "u_template_retire" => Some("u".into()),
            "v_template_push" | "v_template_retire" => Some("v".into()),
            "omit_template_push" | "omit_template_retire" => Some("omit".into()),
            _ => None,
        },
        nesting,
        previous_align_state: record.previous_align_state.map(i64::from),
        delimiter: record.delimiter.map(str::to_owned),
        recovery: match record.transition {
            "missing_parameter" => Some("missing_parameter".into()),
            "extra_parameter" => Some("extra_parameter".into()),
            "missing_left_brace" => Some("missing_left_brace".into()),
            "missing_right_brace" => Some("missing_right_brace".into()),
            "extra_tab" => Some("extra_tab".into()),
            "outer_validity" => Some("outer_validity".into()),
            _ => None,
        },
    })
}
fn translate_mutation(record: MutationRecord) -> Event {
    // `meaning`, `register`, and `parameter` records all pair an explicit key
    // with a typed value, and it is the value's own encoding -- not the
    // target -- that says how to parse it. Selecting the target once and
    // sharing the value decoding keeps every combination available to every
    // keyed target, which is what lets `\dimen` *parameter* mutations reuse
    // the `scaled:` encoding `\dimen` registers already used
    // (umber2-johp.124); enumerating target/encoding pairs by hand had left
    // that one combination silently falling through to the untyped tail.
    let keyed_target = match record.target {
        "meaning" => Some(StateTarget::Meaning),
        "register" => Some(StateTarget::Register),
        "parameter" => Some(StateTarget::Parameter),
        _ => None,
    };
    if let Some(target) = keyed_target
        && let Some(key) = record.key.as_ref()
    {
        let value = if let Some(tokens) = record.tokens.as_ref() {
            Some(CanonicalValue::Tokens(
                tokens.iter().cloned().map(oracle_token).collect(),
            ))
        } else if let Some(value) = record.value.strip_prefix("scaled:") {
            value.parse::<i64>().ok().map(CanonicalValue::Scaled)
        } else if let Some(value) = record.value.strip_prefix("glue:") {
            parse_glue_scanner_value(value)
        } else {
            None
        };
        if let Some(value) = value {
            return Event::Mutation(MutationEvent {
                target,
                key: CanonicalValue::Name(key.clone()),
                value,
                scope: if record.global { "global" } else { "local" }.into(),
            });
        }
    }
    if record.target == "catcode"
        && let Some((character, value)) = record.value.split_once('=')
        && let (Ok(character), Some(value)) = (
            character.parse::<u32>(),
            canonical_catcode_assignment(value),
        )
    {
        return Event::Mutation(MutationEvent {
            target: StateTarget::Catcode,
            key: CanonicalValue::Character(character),
            value: CanonicalValue::Name(value.into()),
            scope: if record.global { "global" } else { "local" }.into(),
        });
    }
    if record.target == "meaning"
        && let Some(key) = record.key
    {
        let value = if let Some(value) = record.value.strip_prefix("character:") {
            value
                .parse::<u32>()
                .map(CanonicalValue::Character)
                .unwrap_or_else(|_| CanonicalValue::Name(record.value.clone()))
        } else if let Some(value) = record.value.strip_prefix("integer:") {
            value
                .parse::<i64>()
                .map(CanonicalValue::Integer)
                .unwrap_or_else(|_| CanonicalValue::Name(record.value.clone()))
        } else {
            CanonicalValue::Name(record.value)
        };
        return Event::Mutation(MutationEvent {
            target: StateTarget::Meaning,
            key: CanonicalValue::Name(key),
            value,
            scope: if record.global { "global" } else { "local" }.into(),
        });
    }
    let parsed = record.value.split_once('=').and_then(|(key, value)| {
        value
            .parse::<i64>()
            .ok()
            .map(|value| (key.to_owned(), value))
    });
    let (key, value) = match parsed {
        Some((key, value)) => (CanonicalValue::Name(key), CanonicalValue::Integer(value)),
        None => (
            CanonicalValue::Name(record.target.into()),
            CanonicalValue::Name(record.value),
        ),
    };
    Event::Mutation(MutationEvent {
        target: match record.target {
            "meaning" => StateTarget::Meaning,
            "catcode" => StateTarget::Catcode,
            "code_table" => StateTarget::CodeTable,
            "parameter" => StateTarget::Parameter,
            _ => StateTarget::Register,
        },
        key,
        value,
        scope: if record.global { "global" } else { "local" }.into(),
    })
}

/// Converts TeX's numeric `cat_code` table value into tex.web §207's category
/// code name, through the one shared table every observation vocabulary uses.
fn canonical_catcode_assignment(value: &str) -> Option<&'static str> {
    canonical_names::catcode_assignment_name(value.parse::<i64>().ok()?)
}
fn translate_effect(record: EffectRecord) -> Event {
    if record.kind == "message" {
        return Event::Effect(EffectEvent {
            kind: EffectKind::Message,
            channel: "terminal".into(),
            value: CanonicalValue::Bytes(record.detail.into_bytes()),
        });
    }
    let (channel, detail) = match record.detail.split_once('\0') {
        Some((channel, detail)) => (channel.to_owned(), detail.to_owned()),
        None => (record.kind.to_owned(), record.detail),
    };
    if record.kind == "shipout" {
        let page = detail
            .parse::<i64>()
            .expect("shipout effects carry their canonical DVI page number");
        return Event::Effect(EffectEvent {
            kind: EffectKind::Shipout,
            channel,
            value: CanonicalValue::Integer(page),
        });
    }
    if record.kind == "terminate" {
        return Event::Effect(EffectEvent {
            kind: EffectKind::Terminate,
            channel,
            value: CanonicalValue::None,
        });
    }
    if record.kind == "close" {
        // tex.web §1374's close branch closes `write_file[j]` without naming
        // it -- only the open branch assigns `cur_name`/`cur_area`/`cur_ext`,
        // and TeX keeps no name for an open stream (§1378 closes the
        // survivors the same way). The stream number in `channel` is the
        // whole committed identity, so the value is `None` rather than an
        // empty name.
        return Event::Effect(EffectEvent {
            kind: EffectKind::Close,
            channel,
            value: CanonicalValue::None,
        });
    }
    Event::Effect(EffectEvent {
        kind: match record.kind {
            "message" => EffectKind::Message,
            "write" => EffectKind::Write,
            "open" => EffectKind::Open,
            "close" => EffectKind::Close,
            "shipout" => EffectKind::Shipout,
            _ => EffectKind::Terminate,
        },
        channel,
        value: record
            .tokens
            .map_or(CanonicalValue::Name(detail), |tokens| {
                CanonicalValue::Tokens(tokens.into_iter().map(oracle_token).collect())
            }),
    })
}

fn bounded_debug(value: &impl fmt::Debug) -> String {
    let rendered = format!("{value:?}");
    if rendered.chars().count() <= MAX_DIAGNOSTIC_CHARS {
        rendered
    } else {
        let prefix = rendered
            .chars()
            .take(MAX_DIAGNOSTIC_CHARS)
            .collect::<String>();
        format!("{prefix}…")
    }
}

/// Renders the point where two same-kind events first differ, for the case
/// [`bounded_debug`] cannot show: a long payload -- a token list, a macro
/// body, a mutation value -- whose divergence sits past the truncation point,
/// so both printed sides are byte-identical and say nothing.
///
/// Deliberately text-level rather than schema-aware: it works for every event
/// kind and every payload field without enumerating them, and a new schema
/// variant needs no change here. Returns `None` when the renderings are equal
/// (the divergence is not in the payload text at all) or when they already
/// differ inside the part `bounded_debug` prints.
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
        CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime, CommandState,
    };
    use tex_oracle::{Event, NormalizedEvent, ScannerEvent};
    use tex_state::{
        meaning::{ExpandablePrimitive, Meaning},
        token::{Catcode, Token},
    };

    fn committed_fixture() -> CommittedFixture {
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
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

    #[test]
    fn source_line_index_preserves_line_and_column_boundaries() {
        let starts = source_line_starts(b"first\n\nlast");
        assert_eq!(&*starts, &[0, 6, 7]);
        assert_eq!(starts.partition_point(|start| *start < 1), 1);
        assert_eq!(starts.partition_point(|start| *start <= 5), 1);
        assert_eq!(starts.partition_point(|start| *start <= 6), 2);
        assert_eq!(starts.partition_point(|start| *start <= 7), 3);
        assert_eq!(starts.partition_point(|start| *start <= 10), 3);
    }

    /// The transport owns no catcode table of its own: it renders whatever
    /// `canonical_names` spells, so a frozen sentinel arrives as a §289
    /// control-sequence token rather than a `Debug` rendering of Umber's enum.
    #[test]
    fn token_transport_carries_only_canonical_names() {
        assert_eq!(
            oracle_token(ObservedToken::Character {
                character: '_',
                catcode: Catcode::Subscript,
            }),
            OracleToken {
                character: u32::from('_'),
                catcode: "sub_mark".into(),
                control_sequence: None,
                location: None,
            }
        );
        assert_eq!(
            oracle_token(ObservedToken::FrozenEndTemplate),
            OracleToken {
                character: 0,
                catcode: "escape".into(),
                control_sequence: Some("endtemplate".into()),
                location: None,
            }
        );
        assert_eq!(
            command_token(&ObservedToken::FrozenEndV),
            (CanonicalValue::None, Some("endtemplate".into()))
        );
    }

    #[test]
    fn recovery_transport_preserves_the_command_owned_kind() {
        let token = ObservedToken::ControlSequence("par".into());
        for (kind, expected) in [
            (CommandRecoveryKind::Backup, RecoveryKind::Backup),
            (
                CommandRecoveryKind::InsertedToken,
                RecoveryKind::InsertedToken,
            ),
            (
                CommandRecoveryKind::InsertedControlSequence,
                RecoveryKind::InsertedControlSequence,
            ),
        ] {
            assert!(matches!(
                translate_recovery(RecoveryRecord {
                    kind,
                    tokens: vec![token.clone()],
                }),
                Event::Recovery(RecoveryEvent { kind: actual, .. }) if actual == expected
            ));
        }
    }

    #[test]
    fn message_effects_use_terminal_bytes() {
        assert_eq!(
            translate_effect(EffectRecord {
                kind: "message",
                detail: "READY".into(),
                tokens: None,
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Message,
                channel: "terminal".into(),
                value: CanonicalValue::Bytes(b"READY".to_vec()),
            })
        );
    }

    #[test]
    fn shipout_effects_use_dvi_page_numbers() {
        assert_eq!(
            translate_effect(EffectRecord {
                kind: "shipout",
                detail: "dvi\x001".into(),
                tokens: None,
            }),
            Event::Effect(EffectEvent {
                kind: EffectKind::Shipout,
                channel: "dvi".into(),
                value: CanonicalValue::Integer(1),
            })
        );
    }

    #[test]
    fn condition_transitions_project_canonical_names_and_limits() {
        assert_eq!(
            translate_condition(ConditionRecord {
                transition: "push",
                identity: 17,
                condition: "iftrue",
                limit: "evaluating",
                branch: None,
            }),
            Event::Condition(ConditionEvent {
                transition: ConditionTransition::Push,
                condition: "iftrue".into(),
                limit: "evaluating".into(),
                branch: None,
            })
        );
        assert_eq!(
            translate_condition(ConditionRecord {
                transition: "branch",
                identity: 17,
                condition: "iftrue",
                limit: "evaluating",
                branch: Some("true".into()),
            }),
            Event::Condition(ConditionEvent {
                transition: ConditionTransition::Branch,
                condition: "iftrue".into(),
                limit: "evaluating".into(),
                branch: Some("true".into()),
            })
        );
    }

    #[test]
    fn brace_delivery_transitions_preserve_command_owned_align_state_changes() {
        for (transition, previous_align_state, align_state) in [
            ("begin_group", -1_000_000, -999_999),
            ("end_group", -999_999, -1_000_000),
        ] {
            assert_eq!(
                translate_alignment(
                    AlignmentRecord {
                        transition,
                        alignment: Some(1),
                        align_state,
                        delimiter: None,
                        previous_align_state: Some(previous_align_state),
                    },
                    Some(1),
                ),
                Event::Alignment(AlignmentEvent {
                    transition: AlignmentTransition::StateChange,
                    align_state: i64::from(align_state),
                    template: None,
                    nesting: Some(1),
                    previous_align_state: Some(i64::from(previous_align_state)),
                    delimiter: None,
                    recovery: None,
                })
            );
        }
    }

    #[test]
    fn alignment_nesting_returns_to_one_after_nested_finish() {
        let mut nesting = AlignmentNesting::default();
        let record = |transition, alignment| AlignmentRecord {
            transition,
            alignment: Some(alignment),
            align_state: 0,
            delimiter: None,
            previous_align_state: None,
        };

        assert_eq!(nesting.observe(&record("begin", 1)), Some(1));
        assert_eq!(nesting.observe(&record("suspend", 1)), Some(1));
        assert_eq!(nesting.observe(&record("begin", 2)), Some(2));
        assert_eq!(nesting.observe(&record("finish", 2)), Some(2));
        assert_eq!(nesting.observe(&record("resume", 1)), Some(1));
        assert_eq!(nesting.observe(&record("finish", 1)), Some(1));

        // Identity allocation is monotonic, but TeX82 §37's `pop_alignment`
        // removes the completed structural level before this new alignment.
        assert_eq!(nesting.observe(&record("begin", 3)), Some(1));
    }

    #[test]
    fn catcode_mutations_use_canonical_assignment_names_and_scope() {
        let event = translate_mutation(MutationRecord {
            target: "catcode",
            value: "123=1".into(),
            key: None,
            tokens: None,
            global: true,
        });
        assert_eq!(
            event,
            Event::Mutation(MutationEvent {
                target: StateTarget::Catcode,
                key: CanonicalValue::Character(123),
                value: CanonicalValue::Name("left_brace".into()),
                scope: "global".into(),
            })
        );
        assert_eq!(canonical_catcode_assignment("16"), None);
    }

    #[test]
    fn token_register_mutations_keep_the_frozen_list() {
        let event = translate_mutation(MutationRecord {
            target: "register",
            value: "tokens".into(),
            key: Some("toks:0".into()),
            tokens: Some(vec![ObservedToken::Character {
                character: 'X',
                catcode: Catcode::Letter,
            }]),
            global: false,
        });
        assert_eq!(
            event,
            Event::Mutation(MutationEvent {
                target: StateTarget::Register,
                key: CanonicalValue::Name("toks:0".into()),
                value: CanonicalValue::Tokens(vec![OracleToken {
                    character: u32::from('X'),
                    catcode: "letter".into(),
                    control_sequence: None,
                    location: None,
                }]),
                scope: "local".into(),
            })
        );
    }

    #[test]
    fn meaning_mutations_keep_the_assigned_control_sequence() {
        let event = translate_mutation(MutationRecord {
            target: "meaning",
            value: "begin_group".into(),
            key: Some("alignmentbegingroup".into()),
            tokens: None,
            global: false,
        });
        assert_eq!(
            event,
            Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name("alignmentbegingroup".into()),
                value: CanonicalValue::Name("begin_group".into()),
                scope: "local".into(),
            })
        );
    }

    #[test]
    fn toksdef_meanings_project_as_assign_toks() {
        let event = translate_mutation(MutationRecord {
            target: "meaning",
            value: "assign_toks".into(),
            key: Some("tokens".into()),
            tokens: None,
            global: false,
        });
        assert_eq!(
            event,
            Event::Mutation(MutationEvent {
                target: StateTarget::Meaning,
                key: CanonicalValue::Name("tokens".into()),
                value: CanonicalValue::Name("assign_toks".into()),
                scope: "local".into(),
            })
        );
    }

    #[test]
    fn glue_scanners_and_mutations_keep_structured_orders() {
        // The producer already spells tex.web §135's order names; the
        // transport carries them through verbatim rather than re-casing a
        // Rust `Debug` rendering (`umber2-johp.141`).
        let value =
            "width=131072;stretch=196608;stretch_order=fil;shrink=262144;shrink_order=normal";
        let expected = CanonicalValue::Glue {
            width: 131_072,
            stretch: 196_608,
            stretch_order: "fil".into(),
            shrink: 262_144,
            shrink_order: "normal".into(),
        };
        assert_eq!(parse_glue_scanner_value(value), Some(expected.clone()));
        assert_eq!(
            translate_mutation(MutationRecord {
                target: "register",
                value: format!("glue:{value}"),
                key: Some("skip:0".into()),
                tokens: None,
                global: false,
            }),
            Event::Mutation(MutationEvent {
                target: StateTarget::Register,
                key: CanonicalValue::Name("skip:0".into()),
                value: expected.clone(),
                scope: "local".into(),
            })
        );
        assert_eq!(
            translate_mutation(MutationRecord {
                target: "parameter",
                value: format!("glue:{value}"),
                key: Some("glue_parameter:11".into()),
                tokens: None,
                global: false,
            }),
            Event::Mutation(MutationEvent {
                target: StateTarget::Parameter,
                key: CanonicalValue::Name("glue_parameter:11".into()),
                value: expected,
                scope: "local".into(),
            })
        );
    }

    fn nested_startup() -> CanonicalStartup {
        CanonicalStartup {
            profile: CommandProfile::TEX82,
            terminal_filename: Arc::from(&b"transitions.tex "[..]),
            root_name: CANONICAL_ROOT_SOURCE.into(),
            root_bytes: Arc::from(&b"a\\input child b"[..]),
            input_capabilities: BTreeMap::from([("child".into(), Arc::from(&b"c"[..]))]),
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
    fn canonical_startup_matches_the_terminal_scan_before_root_delivery() {
        let fixture = committed_fixture();
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let startup = CanonicalStartup::from_fixture(
            &repository.join(FIXTURE_ROOT).join("command-transitions-v1"),
            &fixture,
            &ReplayResources::committed(),
        )
        .expect("canonical startup");

        assert_eq!(startup.profile, CommandProfile::TEX82);
        assert_eq!(startup.root_name, CANONICAL_ROOT_SOURCE);
        let actual = startup.replay().expect("canonical startup replays");
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
        fn suffix(
            command: &mut CommandState,
            universe: &mut Universe,
            capabilities: &mut CommandHostCapabilities,
        ) -> Vec<(char, u64)> {
            let mut runtime = CommandRuntime::default();
            let mut processor = CommandProcessor::new(
                command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(capabilities),
            );
            let mut delivered = Vec::new();
            while let Some(current) = processor.get_x_token().expect("nested input replays") {
                if let Token::Char { ch, .. } = current.spelling().semantic_token() {
                    delivered.push((ch, current.delivery_stamp().input_level()));
                }
            }
            delivered
        }

        let mut command = CommandState::new(CommandProfile::TEX82);
        let root = command
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::World,
                &b"x\\input child y"[..],
            ))
            .expect("root registers");
        command.open_registered_source(root).expect("root opens");
        let mut universe = Universe::new();
        let input = universe.intern("input").symbol();
        universe.set_meaning(
            input,
            Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
        );
        let mut capabilities = CommandHostCapabilities::default();
        capabilities.register_input(
            "child",
            SourceRegistration::new(RegisteredSourceKind::World, &b"c"[..]),
        );

        {
            let mut runtime = CommandRuntime::default();
            let mut processor = CommandProcessor::new(
                &mut command,
                &mut runtime,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
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

        let snapshot = command.snapshot();
        let first = suffix(&mut command, &mut universe, &mut capabilities);
        command
            .rollback(snapshot)
            .expect("matching snapshot restores");
        let second = suffix(&mut command, &mut universe, &mut capabilities);

        assert_eq!(first, second, "rollback preserves nested source identity");
        assert!(
            first.iter().any(|(_, level)| *level > 0),
            "the nested source receives a distinct input-level identity"
        );
        assert!(first.iter().any(|(_, level)| *level == 0));
    }

    #[test]
    fn canonical_startup_rejects_a_stale_root_even_if_its_bytes_are_available() {
        let startup = CanonicalStartup {
            profile: CommandProfile::TEX82,
            terminal_filename: Arc::from(&b"transitions.tex "[..]),
            root_name: "alignment-delivery.tex".into(),
            root_bytes: Arc::from(&b"\\relax"[..]),
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

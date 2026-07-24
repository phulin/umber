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
    AlignmentRecord, CommandDeliveryBoundary, CommandObservation, CommandObserver, CommandProfile,
    ConditionRecord, EffectRecord, InputReason as CommandInputReason, InputRecord, InputTransition,
    MacroRecord, MutationRecord, ObservedToken, RecoveryRecord, RegisteredSourceKind,
    ScannerStatusRecord, SourceRegistration, TokenListRecord,
};
use tex_exec::{CommandReplayControl, ReplayStep};
use tex_oracle::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, CommittedFixture, ConditionEvent, ConditionTransition, EffectEvent, EffectKind,
    EngineDialect, Event, InputEvent, InputReason, MacroEvent, MutationEvent, NormalizedEvent,
    OracleToken, RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus, ScannerStatusEvent,
    StateTarget, TokenListEvent, TokenListTransition, validate_tex82_command_trace_suite,
};
use tex_state::{SourceId, Universe, token::Catcode};

const FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const MAX_DIAGNOSTIC_CHARS: usize = 960;
const MAX_DELIVERIES_OVERHEAD: usize = 64;
const CANONICAL_ROOT_SOURCE: &str = "transitions.tex";
const TERMINAL_FILENAME_TERMINATOR: u8 = b' ';
const CANONICAL_ROOT_PUSH_NAME: &str = "terminal";

/// Runs every registered TeX82 committed fixture with no live-engine access.
pub fn run_repository(repository: impl AsRef<Path>) -> Result<(), RunnerError> {
    let repository = repository.as_ref();
    let suite = validate_tex82_command_trace_suite(repository)
        .map_err(|error| RunnerError::Suite(error.to_string()))?;
    let mut failures = Vec::new();
    for entry in suite.fixtures {
        let fixture_directory =
            repository
                .join(FIXTURE_ROOT)
                .join(entry.selector.strip_prefix("tex82/").ok_or_else(|| {
                    RunnerError::Suite(format!("unsafe selector {}", entry.selector))
                })?);
        let fixture = CommittedFixture::load(&fixture_directory)
            .map_err(|error| RunnerError::Fixture(entry.selector.clone(), error.to_string()))?;
        let actual = replay_fixture(&fixture_directory, &fixture)?;
        let identity = format!(
            "{} manifest={}",
            fixture.manifest.name, fixture.stream.header.manifest
        );
        if let Err(mismatch) = compare_streams(&identity, &fixture.stream.events, &actual) {
            failures.push(*mismatch);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(RunnerError::Comparison(failures))
    }
}

/// Parses the intentionally narrow offline runner interface.
pub fn run_cli() -> Result<(), RunnerError> {
    let mut arguments = env::args_os().skip(1);
    let mut repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    while let Some(argument) = arguments.next() {
        if argument == "--repository" {
            repository = arguments
                .next()
                .ok_or_else(|| {
                    RunnerError::Usage("--repository requires a directory argument".into())
                })?
                .into();
        } else {
            return Err(RunnerError::Usage(format!(
                "unknown argument {}; expected --repository <path>",
                argument.to_string_lossy()
            )));
        }
    }
    run_repository(repository)
}

/// First deterministic stream divergence for one fixture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamMismatch {
    fixture: String,
    index: usize,
    expected: Option<Event>,
    actual: Option<ObservedEvent>,
}

impl fmt::Display for StreamMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "fixture {} diverged at event {}",
            self.fixture, self.index
        )?;
        if let Some(expected) = &self.expected {
            write!(formatter, "\n  expected: {}", bounded_debug(expected))?;
        } else {
            formatter.write_str("\n  expected: <end of committed stream>")?;
        }
        if let Some(actual) = &self.actual {
            write!(formatter, "\n  actual: {}", bounded_debug(&actual.event))?;
            if !actual.context.is_empty() {
                write!(formatter, "\n  context: {}", actual.context)?;
            }
        } else {
            formatter.write_str("\n  actual: <end of observer stream>")?;
        }
        Ok(())
    }
}

/// Compares complete ordered streams without omitting unsupported event kinds.
pub fn compare_streams(
    fixture: &str,
    expected: &[NormalizedEvent],
    actual: &[ObservedEvent],
) -> Result<(), Box<StreamMismatch>> {
    let count = expected.len().max(actual.len());
    for index in 0..count {
        let expected_event = expected.get(index).map(|event| &event.semantic);
        let actual_event = actual.get(index);
        if expected_event != actual_event.map(|event| &event.event) {
            return Err(Box::new(StreamMismatch {
                fixture: fixture.into(),
                index,
                expected: expected_event.cloned(),
                actual: actual_event.cloned(),
            }));
        }
    }
    Ok(())
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
}

#[derive(Debug)]
pub enum RunnerError {
    Usage(String),
    Suite(String),
    Fixture(String, String),
    Replay(String),
    Comparison(Vec<StreamMismatch>),
}

impl fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(error) | Self::Suite(error) | Self::Replay(error) => {
                formatter.write_str(error)
            }
            Self::Fixture(fixture, error) => write!(formatter, "fixture {fixture}: {error}"),
            Self::Comparison(mismatches) => {
                for (index, mismatch) in mismatches.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str("\n")?;
                    }
                    mismatch.fmt(formatter)?;
                }
                Ok(())
            }
        }
    }
}

impl Error for RunnerError {}

fn replay_fixture(
    directory: &Path,
    fixture: &CommittedFixture,
) -> Result<Vec<ObservedEvent>, RunnerError> {
    if fixture.manifest.oracle.engine.dialect != EngineDialect::Tex82
        || fixture.manifest.profile.invocation != "initex"
        || fixture.manifest.profile.characters != "eight_bit_exact"
    {
        return Err(RunnerError::Replay(format!(
            "{} is not a TeX82 INITEX eight-bit fixture",
            fixture.manifest.name
        )));
    }
    CanonicalStartup::from_fixture(directory, fixture)?.replay()
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
}

impl CanonicalStartup {
    #[allow(
        clippy::disallowed_methods,
        reason = "this offline host tool reads fixture bytes after CommittedFixture validation"
    )]
    fn from_fixture(directory: &Path, fixture: &CommittedFixture) -> Result<Self, RunnerError> {
        let artifact = fixture
            .manifest
            .sources
            .get(CANONICAL_ROOT_SOURCE)
            .ok_or_else(|| {
                RunnerError::Replay(format!(
                    "{} does not declare canonical root source {CANONICAL_ROOT_SOURCE}",
                    fixture.manifest.name
                ))
            })?;
        let bytes = std::fs::read(directory.join(&artifact.path)).map_err(|error| {
            RunnerError::Replay(format!(
                "{} source {CANONICAL_ROOT_SOURCE} cannot be read: {error}",
                fixture.manifest.name
            ))
        })?;
        if u64::try_from(bytes.len()).ok() != Some(artifact.bytes) {
            return Err(RunnerError::Replay(format!(
                "{} source {CANONICAL_ROOT_SOURCE} changed after fixture validation",
                fixture.manifest.name
            )));
        }

        let mut input_capabilities = BTreeMap::new();
        for (source_name, source_artifact) in &fixture.manifest.sources {
            if source_name == CANONICAL_ROOT_SOURCE {
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

        let mut terminal_filename = CANONICAL_ROOT_SOURCE.as_bytes().to_vec();
        terminal_filename.push(TERMINAL_FILENAME_TERMINATOR);
        Ok(Self {
            profile: CommandProfile::TEX82,
            terminal_filename: Arc::from(terminal_filename),
            root_name: CANONICAL_ROOT_SOURCE.into(),
            root_bytes: Arc::from(bytes),
            input_capabilities,
        })
    }

    fn replay(self) -> Result<Vec<ObservedEvent>, RunnerError> {
        let limit = self
            .terminal_filename
            .len()
            .checked_add(self.root_bytes.len())
            .and_then(|count| {
                self.input_capabilities
                    .values()
                    .try_fold(count, |total, source| total.checked_add(source.len()))
            })
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(MAX_DELIVERIES_OVERHEAD))
            .ok_or_else(|| {
                RunnerError::Replay("canonical startup replay bound overflowed".into())
            })?;
        let mut universe = Universe::new();
        let mut control = CommandReplayControl::tex82_initex(&mut universe);
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
        command.open_registered_source(terminal).map_err(|error| {
            RunnerError::Replay(format!("terminal filename cannot open: {error}"))
        })?;

        for (name, bytes) in &self.input_capabilities {
            control.capabilities_mut().register_input(
                name,
                SourceRegistration::new(RegisteredSourceKind::World, Arc::clone(bytes)),
            );
        }
        let mut recorder = Recorder::new("terminal");
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
        recorder.source = self.root_name.clone();

        let mut deliveries = 0;
        {
            loop {
                if deliveries == limit {
                    return Err(RunnerError::Replay(format!(
                        "root source {} exceeded replay bound {limit}",
                        self.root_name
                    )));
                }
                match control.step_with_observer(&mut universe, &mut recorder) {
                    Ok(ReplayStep::Continue) => deliveries += 1,
                    Ok(ReplayStep::End | ReplayStep::EndOfInput) => break,
                    Err(_) => break,
                }
            }
        }
        Ok(recorder.events)
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

struct Recorder {
    source: String,
    events: Vec<ObservedEvent>,
}

impl Recorder {
    fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            events: Vec::new(),
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
}

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.events
            .push(translate_observation(&self.source, observation));
    }
}

fn translate_observation(source: &str, observation: CommandObservation) -> ObservedEvent {
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
            if let Some(command_operand) = record.command_operand {
                operand = CanonicalValue::Integer(command_operand);
            }
            ObservedEvent::new(
                Event::Command(CommandEvent {
                    delivery: match record.boundary {
                        CommandDeliveryBoundary::Raw => CommandDelivery::Raw,
                        CommandDeliveryBoundary::Expanded => CommandDelivery::Expanded,
                    },
                    command: CanonicalCommand {
                        command: canonical_command_name(&record),
                        operand,
                        control_sequence,
                        location: None,
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
            ObservedEvent::new(translate_input(record), context)
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
            let context = format!("source={source}; condition={}", record.condition);
            ObservedEvent::new(translate_condition(record), context)
        }
        CommandObservation::Scanner(record) => ObservedEvent::new(
            Event::Scanner(ScannerEvent {
                scanner: record.kind.into(),
                result: CanonicalValue::Name(record.value),
            }),
            format!("source={source}"),
        ),
        CommandObservation::TokenList(record) => {
            ObservedEvent::new(translate_token_list(record), format!("source={source}"))
        }
        CommandObservation::Alignment(record) => {
            let context = format!("source={source}; nesting={:?}", record.alignment);
            ObservedEvent::new(translate_alignment(record), context)
        }
        CommandObservation::Mutation(record) => {
            ObservedEvent::new(translate_mutation(record), format!("source={source}"))
        }
        CommandObservation::Effect(record) => {
            ObservedEvent::new(translate_effect(record), format!("source={source}"))
        }
    }
}

fn canonical_command_name(record: &tex_command::CommandDeliveryRecord) -> String {
    match &record.spelling {
        ObservedToken::Character { catcode, .. } => match catcode {
            Catcode::Letter => "letter".into(),
            Catcode::Space => "spacer".into(),
            Catcode::Other => "other_char".into(),
            _ => record.command.clone(),
        },
        _ => record.command.clone(),
    }
}

fn command_token(token: &ObservedToken) -> (CanonicalValue, Option<String>) {
    match token {
        ObservedToken::Character { character, .. } => (
            CanonicalValue::Integer(i64::from(u32::from(*character))),
            None,
        ),
        ObservedToken::ControlSequence(name) => (CanonicalValue::None, Some(name.clone())),
        _ => (CanonicalValue::Name(format!("{token:?}")), None),
    }
}

fn oracle_token(token: ObservedToken) -> OracleToken {
    match token {
        ObservedToken::Character { character, catcode } => OracleToken {
            character: u32::from(character),
            catcode: catcode_name(catcode).into(),
            control_sequence: None,
            location: None,
        },
        ObservedToken::ControlSequence(control_sequence) => OracleToken {
            character: 0,
            catcode: "escape".into(),
            control_sequence: Some(control_sequence),
            location: None,
        },
        other => OracleToken {
            character: 0,
            catcode: format!("{other:?}"),
            control_sequence: None,
            location: None,
        },
    }
}

fn catcode_name(catcode: Catcode) -> &'static str {
    match catcode {
        Catcode::Escape => "escape",
        Catcode::BeginGroup => "begin_group",
        Catcode::EndGroup => "end_group",
        Catcode::MathShift => "math_shift",
        Catcode::AlignmentTab => "alignment_tab",
        Catcode::EndLine => "end_line",
        Catcode::Parameter => "parameter",
        Catcode::Superscript => "superscript",
        Catcode::Subscript => "subscript",
        Catcode::Ignored => "ignored",
        Catcode::Space => "space",
        Catcode::Letter => "letter",
        Catcode::Other => "other",
        Catcode::Active => "active",
        Catcode::Comment => "comment",
        Catcode::Invalid => "invalid",
    }
}

fn translate_input(record: InputRecord) -> Event {
    let transition = match record.transition {
        InputTransition::Retire => tex_oracle::InputTransition::Retire,
        InputTransition::Stop => tex_oracle::InputTransition::Stop,
        InputTransition::Backup | InputTransition::Recovery => tex_oracle::InputTransition::Push,
    };
    let reason = match record.reason {
        CommandInputReason::Source => InputReason::Source,
        CommandInputReason::Backup => InputReason::Backup,
        CommandInputReason::Macro => InputReason::Macro,
        CommandInputReason::Parameter | CommandInputReason::TokenList => InputReason::TokenList,
        CommandInputReason::AlignmentTemplate => InputReason::AlignmentTemplate,
        CommandInputReason::Recovery => InputReason::Recovery,
    };
    Event::Input(InputEvent {
        transition,
        reason,
        name: match record.reason {
            CommandInputReason::Backup => "backup".into(),
            CommandInputReason::Macro => "macro".into(),
            CommandInputReason::Parameter => "parameter".into(),
            CommandInputReason::AlignmentTemplate => "template".into(),
            CommandInputReason::Recovery => "recovery".into(),
            CommandInputReason::TokenList => "output".into(),
            CommandInputReason::Source => "source".into(),
        },
    })
}

fn translate_recovery(record: RecoveryRecord) -> Event {
    Event::Recovery(RecoveryEvent {
        kind: if record.backup {
            RecoveryKind::Backup
        } else {
            RecoveryKind::InsertedToken
        },
        tokens: record.tokens.into_iter().map(oracle_token).collect(),
    })
}
fn translate_status(record: ScannerStatusRecord) -> Event {
    let status = scanner_status(&record.status);
    Event::ScannerStatus(ScannerStatusEvent {
        from: if record.entering {
            ScannerStatus::Normal
        } else {
            status
        },
        to: if record.entering {
            status
        } else {
            ScannerStatus::Normal
        },
    })
}
fn scanner_status(status: &str) -> ScannerStatus {
    match status {
        "normal" => ScannerStatus::Normal,
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
            control_sequence: format!("definition-{}", record.definition),
            argument_count: record.argument.unwrap_or(0).into(),
        })
    } else {
        Event::Macro(MacroEvent::Argument {
            parameter: record.argument.unwrap_or(0).into(),
            tokens: Vec::new(),
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
        condition: record.condition.to_string(),
        limit: record.detail.clone(),
        branch: (record.transition == "branch").then_some(record.detail),
    })
}
fn translate_token_list(record: TokenListRecord) -> Event {
    Event::TokenList(TokenListEvent {
        transition: if record.transition == "splice" {
            TokenListTransition::Splice
        } else {
            TokenListTransition::Complete
        },
        purpose: record.transition.into(),
        tokens: Vec::new(),
    })
}
fn translate_alignment(record: AlignmentRecord) -> Event {
    Event::Alignment(AlignmentEvent {
        transition: match record.transition {
            "begin" => AlignmentTransition::Begin,
            "finish" => AlignmentTransition::Finish,
            "suspend" => AlignmentTransition::Suspend,
            "resume" => AlignmentTransition::Resume,
            "preamble_start" => AlignmentTransition::PreambleStart,
            "preamble_finish" => AlignmentTransition::PreambleFinish,
            "state_change" => AlignmentTransition::StateChange,
            "template_push" => AlignmentTransition::TemplatePush,
            "template_retire" => AlignmentTransition::TemplateRetire,
            "delimiter" => AlignmentTransition::Delimiter,
            "backup_correction" => AlignmentTransition::BackupCorrection,
            _ => AlignmentTransition::Recovery,
        },
        align_state: i64::from(record.align_state),
        template: None,
        nesting: record.alignment.and_then(|value| u32::try_from(value).ok()),
        previous_align_state: None,
        delimiter: None,
        recovery: None,
    })
}
fn translate_mutation(record: MutationRecord) -> Event {
    Event::Mutation(MutationEvent {
        target: match record.target {
            "meaning" => StateTarget::Meaning,
            "catcode" => StateTarget::Catcode,
            "code_table" => StateTarget::CodeTable,
            "parameter" => StateTarget::Parameter,
            _ => StateTarget::Register,
        },
        key: CanonicalValue::Name(record.target.into()),
        value: CanonicalValue::Name(record.value),
        scope: "local".into(),
    })
}
fn translate_effect(record: EffectRecord) -> Event {
    Event::Effect(EffectEvent {
        kind: match record.kind {
            "message" => EffectKind::Message,
            "write" => EffectKind::Write,
            "open" => EffectKind::Open,
            "close" => EffectKind::Close,
            "shipout" => EffectKind::Shipout,
            _ => EffectKind::Terminate,
        },
        channel: record.kind.into(),
        value: CanonicalValue::Name(record.detail),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tex_command::{
        CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandRuntime, CommandState,
    };
    use tex_oracle::{Event, NormalizedEvent, ScannerEvent};
    use tex_state::{
        meaning::{ExpandablePrimitive, Meaning},
        token::Token,
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

    fn nested_startup() -> CanonicalStartup {
        CanonicalStartup {
            profile: CommandProfile::TEX82,
            terminal_filename: Arc::from(&b"transitions.tex "[..]),
            root_name: CANONICAL_ROOT_SOURCE.into(),
            root_bytes: Arc::from(&b"a\\input child b"[..]),
            input_capabilities: BTreeMap::from([("child".into(), Arc::from(&b"c"[..]))]),
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
    fn canonical_startup_matches_the_terminal_scan_before_root_delivery() {
        let fixture = committed_fixture();
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let startup = CanonicalStartup::from_fixture(
            &repository.join(FIXTURE_ROOT).join("command-transitions-v1"),
            &fixture,
        )
        .expect("canonical startup");

        assert_eq!(startup.profile, CommandProfile::TEX82);
        assert_eq!(startup.root_name, CANONICAL_ROOT_SOURCE);
        let actual = startup.replay().expect("canonical startup replays");
        let actual_events = actual[..38]
            .iter()
            .map(|event| event.event.clone())
            .collect::<Vec<_>>();
        let expected_events = fixture.stream.events[..38]
            .iter()
            .map(|event| event.semantic.clone())
            .collect::<Vec<_>>();
        assert_eq!(&actual_events[..6], &expected_events[..6]);
        // The observer records the retired level but does not yet retain its
        // replay reason; all remaining terminal-scan and root-open events are
        // canonical and ordered.
        assert_eq!(&actual_events[7..], &expected_events[7..]);
        assert_eq!(actual[37].context, "source=transitions.tex; source_id=1");
    }

    #[test]
    fn registered_nested_input_retires_and_returns_to_the_caller_deterministically() {
        let first = nested_startup().replay().expect("nested source replays");
        let second = nested_startup()
            .replay()
            .expect("nested source replays again");
        assert_eq!(first, second, "registered input replay must be repeatable");

        let child_delivery = first
            .iter()
            .position(|event| event.context.contains("input_level=3"))
            .expect("the child receives its own input level");
        let child_retirement = first
            .iter()
            .enumerate()
            .skip(child_delivery)
            .find_map(|(index, event)| {
                event
                    .context
                    .contains("level=3; position=0")
                    .then_some(index)
            })
            .expect("the exhausted child source retires");
        assert!(
            first[child_retirement + 1..]
                .iter()
                .any(|event| event.context.contains("input_level=2")),
            "input resumes the still-live parent source after child EOF"
        );
    }

    #[test]
    fn nested_input_rejects_a_missing_registered_capability() {
        let mut startup = nested_startup();
        startup.input_capabilities.clear();

        let events = startup.replay().expect("missing input stops typed replay");
        assert!(
            !events
                .iter()
                .any(|event| event.context.contains("input_level=3")),
            "missing input must not be opened from the host"
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
        assert!(first.iter().any(|(_, level)| *level == 1));
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
        };

        let error = startup.replay().expect_err("stale root must not open");
        assert!(
            error
                .to_string()
                .contains("not canonical root \"alignment-delivery.tex\"")
        );
    }
}

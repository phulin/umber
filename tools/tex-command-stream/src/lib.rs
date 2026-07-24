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
    SourceLocation, StateTarget, TokenListEvent, TokenListTransition,
    validate_tex82_command_trace_suite,
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
        let replay = replay_fixture(&fixture_directory, &fixture)?;
        let identity = format!(
            "{} manifest={}",
            fixture.manifest.name, fixture.stream.header.manifest
        );
        if let Err(mismatch) = compare_streams(&identity, &fixture.stream.events, &replay.events) {
            failures.push(*mismatch);
        } else if let Some(error) = replay.failure {
            return Err(RunnerError::Replay(format!(
                "fixture {} replay did not reach a terminal command state: {error}",
                fixture.manifest.name
            )));
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
        if !events_match(expected_event, actual_event.map(|event| &event.event)) {
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

/// Compares portable command observations with the committed TeX82 trace.
///
/// TeX82 records the `cur_chr` field for every raw command.  For a macro call
/// that field is the mutable `def_ref` token-list address, rather than a
/// semantic macro operand.  The command core intentionally replaces that
/// allocator address with immutable macro-definition ownership, so the trace
/// comparison projects this one reference-only field away.  Macro command
/// kind and control-sequence spelling remain exact, and an observed macro
/// call must likewise expose no operand.
fn events_match(expected: Option<&Event>, actual: Option<&Event>) -> bool {
    match (expected, actual) {
        (Some(expected), Some(actual)) if macro_call_operand_is_reference(expected, actual) => true,
        _ => expected == actual,
    }
}

fn macro_call_operand_is_reference(expected: &Event, actual: &Event) -> bool {
    let (
        Event::Command(CommandEvent {
            delivery: expected_delivery,
            command:
                CanonicalCommand {
                    command: expected_command,
                    operand: CanonicalValue::Integer(_),
                    control_sequence: expected_control_sequence,
                    location: expected_location,
                },
        }),
        Event::Command(CommandEvent {
            delivery: actual_delivery,
            command:
                CanonicalCommand {
                    command: actual_command,
                    operand: CanonicalValue::None,
                    control_sequence: actual_control_sequence,
                    location: actual_location,
                },
        }),
    ) = (expected, actual)
    else {
        return false;
    };

    is_macro_call_command(expected_command)
        && expected_delivery == actual_delivery
        && expected_command == actual_command
        && expected_control_sequence == actual_control_sequence
        && expected_location == actual_location
}

fn is_macro_call_command(command: &str) -> bool {
    matches!(
        command,
        "call" | "long_call" | "outer_call" | "long_outer_call"
    )
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
    CanonicalStartup::from_fixture(directory, fixture)?.replay()
}

/// Observer output plus a nonterminal executor failure, if one occurred.
///
/// The complete fixture stream is still compared first, so a command error
/// after an already-produced earlier semantic divergence cannot mask that
/// deterministic earliest mismatch. If the observed prefix is exact, the
/// failure becomes a runner error instead of being mistaken for EOF.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayOutput {
    events: Vec<ObservedEvent>,
    failure: Option<String>,
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

    fn replay(self) -> Result<ReplayOutput, RunnerError> {
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
        let mut recorder = Recorder::new("terminal", self.input_capabilities);
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
                    return Err(RunnerError::Replay(format!(
                        "root source {} exceeded replay bound {limit}",
                        self.root_name
                    )));
                }
                match control.step_with_observer(&mut universe, &mut recorder) {
                    Ok(ReplayStep::Continue) => deliveries += 1,
                    Ok(ReplayStep::End | ReplayStep::EndOfInput) => break,
                    Err(error) => {
                        return Ok(ReplayOutput {
                            events: recorder.events,
                            failure: Some(format!(
                                "root source {} replay failed after {deliveries} deliveries: {error}",
                                self.root_name
                            )),
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

struct Recorder {
    sources: Vec<ActiveSource>,
    registered_inputs: BTreeMap<String, Arc<[u8]>>,
    next_registered_source: u32,
    events: Vec<ObservedEvent>,
}

struct ActiveSource {
    name: String,
    source: Option<SourceId>,
    bytes: Arc<[u8]>,
}

impl Recorder {
    fn new(source: impl Into<String>, registered_inputs: BTreeMap<String, Arc<[u8]>>) -> Self {
        Self {
            sources: vec![ActiveSource {
                name: source.into(),
                source: None,
                bytes: Arc::from(&b""[..]),
            }],
            registered_inputs,
            next_registered_source: 2,
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

    fn activate_source(&mut self, name: impl Into<String>, source: SourceId, bytes: Arc<[u8]>) {
        self.sources.push(ActiveSource {
            name: name.into(),
            source: Some(source),
            bytes,
        });
    }

    fn activate_registered_input(&mut self, name: &str) {
        let Some(bytes) = self.registered_inputs.get(name) else {
            return;
        };
        let source = SourceId::new(self.next_registered_source);
        self.next_registered_source += 1;
        self.activate_source(name, source, Arc::clone(bytes));
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

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        let source = self.current_source();
        self.events.push(translate_observation(
            &source.name,
            source.source,
            Some(&source.bytes),
            observation.clone(),
        ));
        match observation {
            CommandObservation::Effect(EffectRecord {
                kind: "input",
                detail,
            }) => self.activate_registered_input(&detail),
            CommandObservation::Input(InputRecord {
                transition: InputTransition::Retire,
                reason: CommandInputReason::Source,
                ..
            }) => self.retire_current_source(),
            _ => {}
        }
    }
}

fn translate_observation(
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    observation: CommandObservation,
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
                        location: command_location(&record, source, source_id, source_bytes),
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
        CommandObservation::Scanner(record) => {
            let result = if let Some(tokens) = record.tokens {
                CanonicalValue::Tokens(tokens.into_iter().map(oracle_token).collect())
            } else if record.kind == "integer" {
                record.value.parse::<i64>().map_or_else(
                    |_| CanonicalValue::Name(record.value),
                    CanonicalValue::Integer,
                )
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

fn command_location(
    record: &tex_command::CommandDeliveryRecord,
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
) -> Option<SourceLocation> {
    let range = record.provenance.source_range?;
    if Some(range.source()) != source_id {
        return None;
    }
    let bytes = source_bytes?;
    let mut byte = usize::try_from(range.start()).ok()?;
    if matches!(record.spelling, ObservedToken::ControlSequence(_)) {
        byte = usize::try_from(range.end().checked_sub(1)?).ok()?;
    }
    let prefix = bytes.get(..byte)?;
    let line_start = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    Some(SourceLocation {
        source: source.into(),
        line: u32::try_from(prefix.iter().filter(|byte| **byte == b'\n').count() + 1).ok()?,
        byte: u32::try_from(byte.checked_sub(line_start)?).ok()?,
    })
}

fn canonical_command_name(record: &tex_command::CommandDeliveryRecord) -> String {
    match &record.spelling {
        ObservedToken::Character { catcode, .. } => match catcode {
            Catcode::BeginGroup => "left_brace".into(),
            Catcode::EndGroup => "right_brace".into(),
            Catcode::MathShift => "math_shift".into(),
            Catcode::AlignmentTab => "tab_mark".into(),
            Catcode::EndLine => "car_ret".into(),
            Catcode::Parameter => "mac_param".into(),
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
        ObservedToken::MacroMatch => OracleToken {
            character: u32::from('#'),
            catcode: "match".into(),
            control_sequence: None,
            location: None,
        },
        ObservedToken::MacroEndMatch => OracleToken {
            character: 0,
            catcode: "end_match".into(),
            control_sequence: None,
            location: None,
        },
        ObservedToken::Parameter(slot) => OracleToken {
            character: u32::from(slot),
            catcode: "out_parameter".into(),
            control_sequence: None,
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
        // The schema deliberately retains TeX's command names rather than
        // Rust/engine catcode variant names. Token-bearing events therefore
        // use the same spellings as raw command delivery.
        Catcode::BeginGroup => "left_brace",
        Catcode::EndGroup => "right_brace",
        Catcode::MathShift => "math_shift",
        Catcode::AlignmentTab => "tab_mark",
        Catcode::EndLine => "end_line",
        Catcode::Parameter => "parameter",
        Catcode::Superscript => "superscript",
        Catcode::Subscript => "subscript",
        Catcode::Ignored => "ignored",
        Catcode::Space => "spacer",
        Catcode::Letter => "letter",
        Catcode::Other => "other_char",
        Catcode::Active => "active",
        Catcode::Comment => "comment",
        Catcode::Invalid => "invalid",
    }
}

fn translate_input(record: InputRecord) -> Event {
    let transition = match record.transition {
        InputTransition::Push => tex_oracle::InputTransition::Push,
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
    if status.starts_with("Skipping") {
        ScannerStatus::Skipping
    } else if status.starts_with("Defining") {
        ScannerStatus::Defining
    } else if status.starts_with("Matching") {
        ScannerStatus::Matching
    } else if status.starts_with("Aligning") {
        ScannerStatus::Aligning
    } else if status.starts_with("Absorbing") {
        ScannerStatus::Absorbing
    } else {
        ScannerStatus::Normal
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
        purpose: record.purpose.into(),
        tokens: record.tokens.into_iter().map(oracle_token).collect(),
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
    if record.target == "meaning"
        && let (Some(key), Some(tokens)) = (record.key.as_ref(), record.tokens.as_ref())
    {
        return Event::Mutation(MutationEvent {
            target: StateTarget::Meaning,
            key: CanonicalValue::Name(key.clone()),
            value: CanonicalValue::Tokens(tokens.iter().cloned().map(oracle_token).collect()),
            scope: if record.global { "global" } else { "local" }.into(),
        });
    }
    if record.target == "register"
        && let (Some(key), Some(tokens)) = (record.key.as_ref(), record.tokens.as_ref())
    {
        return Event::Mutation(MutationEvent {
            target: StateTarget::Register,
            key: CanonicalValue::Name(key.clone()),
            value: CanonicalValue::Tokens(tokens.iter().cloned().map(oracle_token).collect()),
            scope: if record.global { "global" } else { "local" }.into(),
        });
    }
    if record.target == "parameter"
        && let (Some(key), Some(tokens)) = (record.key.as_ref(), record.tokens.as_ref())
    {
        return Event::Mutation(MutationEvent {
            target: StateTarget::Parameter,
            key: CanonicalValue::Name(key.clone()),
            value: CanonicalValue::Tokens(tokens.iter().cloned().map(oracle_token).collect()),
            scope: if record.global { "global" } else { "local" }.into(),
        });
    }
    if record.target == "catcode" {
        if let Some((character, value)) = record.value.split_once('=') {
            if let Ok(character) = character.parse::<u32>() {
                return Event::Mutation(MutationEvent {
                    target: StateTarget::Catcode,
                    key: CanonicalValue::Character(character),
                    value: CanonicalValue::Name(value.into()),
                    scope: "local".into(),
                });
            }
        }
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

    #[test]
    fn token_catcodes_use_canonical_tex_command_names() {
        assert_eq!(catcode_name(Catcode::BeginGroup), "left_brace");
        assert_eq!(catcode_name(Catcode::EndGroup), "right_brace");
        assert_eq!(catcode_name(Catcode::AlignmentTab), "tab_mark");
        assert_eq!(catcode_name(Catcode::Space), "spacer");
        assert_eq!(catcode_name(Catcode::Other), "other_char");
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
                event
                    .context
                    .contains("level=3; position=0")
                    .then_some(index)
            })
            .expect("the exhausted child source retires");
        assert!(
            first.events[child_retirement + 1..]
                .iter()
                .any(|event| event.context.contains("input_level=2")),
            "input resumes the still-live parent source after child EOF"
        );
        assert!(first.events.iter().any(|event| {
            event.context.starts_with("source=child;")
                && matches!(
                    &event.event,
                    Event::Command(CommandEvent {
                        command: CanonicalCommand {
                            location: Some(SourceLocation { source, .. }),
                            ..
                        },
                        ..
                    }) if source == "child"
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
            replay
                .failure
                .as_deref()
                .is_some_and(|failure| failure.contains("missing token while scanning \\input"))
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

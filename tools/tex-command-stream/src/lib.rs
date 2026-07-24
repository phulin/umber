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
    AlignmentRecord, CommandDeliveryBoundary, CommandHostCapabilities, CommandHostContext,
    CommandObservation, CommandObserver, CommandProcessor, CommandProfile, CommandRuntime,
    CommandState, ConditionRecord, EffectRecord, InputRecord, InputTransition, MacroRecord,
    MutationRecord, ObservedToken, RecoveryRecord, RegisteredSourceKind, ScannerStatusRecord,
    SourceRegistration, TokenListRecord,
};
use tex_oracle::{
    AlignmentEvent, AlignmentTransition, CanonicalCommand, CanonicalValue, CommandDelivery,
    CommandEvent, CommittedFixture, ConditionEvent, ConditionTransition, EffectEvent, EffectKind,
    EngineDialect, Event, InputEvent, InputReason, MacroEvent, MutationEvent, NormalizedEvent,
    OracleToken, RecoveryEvent, RecoveryKind, ScannerEvent, ScannerStatus, ScannerStatusEvent,
    StateTarget, TokenListEvent, TokenListTransition, validate_tex82_command_trace_suite,
};
use tex_state::{Universe, token::Catcode};

const FIXTURE_ROOT: &str = "tests/corpus/command/tex82";
const MAX_DIAGNOSTIC_CHARS: usize = 960;
const MAX_DELIVERIES_OVERHEAD: usize = 64;

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
    let mut sources = BTreeMap::new();
    for (name, artifact) in &fixture.manifest.sources {
        #[allow(
            clippy::disallowed_methods,
            reason = "this offline host tool reads fixture bytes after CommittedFixture validation"
        )]
        let bytes = std::fs::read(directory.join(&artifact.path)).map_err(|error| {
            RunnerError::Replay(format!(
                "{} source {name} cannot be read: {error}",
                fixture.manifest.name
            ))
        })?;
        if u64::try_from(bytes.len()).ok() != Some(artifact.bytes) {
            return Err(RunnerError::Replay(format!(
                "{} source {name} changed after fixture validation",
                fixture.manifest.name
            )));
        }
        sources.insert(name.clone(), Arc::<[u8]>::from(bytes));
    }
    let mut events = Vec::new();
    for (source, bytes) in sources {
        events.extend(replay_source(&source, bytes)?);
    }
    Ok(events)
}

fn replay_source(source: &str, bytes: Arc<[u8]>) -> Result<Vec<ObservedEvent>, RunnerError> {
    let limit = bytes
        .len()
        .checked_mul(2)
        .and_then(|count| count.checked_add(MAX_DELIVERIES_OVERHEAD))
        .ok_or_else(|| RunnerError::Replay(format!("source {source} replay bound overflowed")))?;
    let mut command = CommandState::new(CommandProfile::TEX82);
    let source_id = command
        .register_source(SourceRegistration::new(RegisteredSourceKind::World, bytes))
        .map_err(|error| {
            RunnerError::Replay(format!("source {source} cannot register: {error}"))
        })?;
    command
        .open_registered_source(source_id)
        .map_err(|error| RunnerError::Replay(format!("source {source} cannot open: {error}")))?;
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder {
        source: source.into(),
        events: Vec::new(),
    };
    let mut deliveries = 0;
    {
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);
        loop {
            if deliveries == limit {
                return Err(RunnerError::Replay(format!(
                    "source {source} exceeded replay bound {limit}"
                )));
            }
            match processor.get_next() {
                Ok(Some(_)) => deliveries += 1,
                Ok(None) => break,
                Err(error) => {
                    return Err(RunnerError::Replay(format!(
                        "source {source} failed after {deliveries} deliveries: {error}"
                    )));
                }
            }
        }
    }
    Ok(recorder.events)
}

struct Recorder {
    source: String,
    events: Vec<ObservedEvent>,
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
            let (operand, control_sequence) = command_token(&record.spelling);
            ObservedEvent::new(
                Event::Command(CommandEvent {
                    delivery: match record.boundary {
                        CommandDeliveryBoundary::Raw => CommandDelivery::Raw,
                        CommandDeliveryBoundary::Expanded => CommandDelivery::Expanded,
                    },
                    command: CanonicalCommand {
                        command: record.command,
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
    let reason = match record.transition {
        InputTransition::Backup => InputReason::Backup,
        InputTransition::Recovery => InputReason::Recovery,
        _ => InputReason::Source,
    };
    Event::Input(InputEvent {
        transition,
        reason,
        name: match record.transition {
            InputTransition::Backup => "backup".into(),
            InputTransition::Recovery => "recovery".into(),
            _ => "source".into(),
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
    use tex_oracle::{Event, NormalizedEvent, ScannerEvent};

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
}

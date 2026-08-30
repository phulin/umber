use super::*;
use tex_oracle::GeometryLocation;

pub(crate) fn translate_observation(
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    source_line_starts: Option<&[usize]>,
    observation: CommandObservation,
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
            if let Some(semantic_operand) = &record.semantic_operand {
                operand = CanonicalValue::Name(semantic_operand.clone());
            } else if let Some(command_operand) = record.command_operand
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
        CommandObservation::GeneratedSource(_) => {
            unreachable!("generated source context is consumed before semantic translation")
        }
        CommandObservation::Recovery(record) => {
            ObservedEvent::new(translate_recovery(record), format!("source={source}"))
        }
        CommandObservation::ScannerStatus(record) => {
            ObservedEvent::new(translate_status(record), format!("source={source}"))
        }
        CommandObservation::Macro(record) => {
            let name = match &record {
                MacroRecord::Activation {
                    control_sequence, ..
                }
                | MacroRecord::Argument {
                    control_sequence, ..
                } => control_sequence,
            };
            let context = format!("source={source}; macro={name}");
            ObservedEvent::new(translate_macro(record), context)
        }
        CommandObservation::Condition(record) => {
            let context = format!("source={source}; condition={}", record.identity);
            ObservedEvent::new(translate_condition(record), context)
        }
        CommandObservation::Scanner(record) => ObservedEvent::new(
            Event::Scanner(ScannerEvent {
                scanner: record.kind.into(),
                result: observation_value(record.value),
            }),
            format!("source={source}"),
        ),
        CommandObservation::TokenList(record) => {
            ObservedEvent::new(translate_token_list(record), format!("source={source}"))
        }
        CommandObservation::Alignment(record) => {
            let nesting = record.nesting;
            let context = format!("source={source}; nesting={nesting:?}");
            ObservedEvent::new(translate_alignment(record), context)
        }
        CommandObservation::Mutation(record) => {
            ObservedEvent::new(translate_mutation(record), format!("source={source}"))
        }
        CommandObservation::Diagnostic(record) => ObservedEvent::new(
            Event::Diagnostic(DiagnosticEvent {
                severity: diagnostic_severity(record.severity),
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
        CommandObservation::DiagnosticLifecycle(record) => ObservedEvent::new(
            Event::DiagnosticLifecycle(match record {
                DiagnosticLifecycleRecord::Report {
                    class,
                    severity,
                    diagnostic,
                    arguments,
                    location,
                } => DiagnosticLifecycleEvent::Report {
                    class: match class {
                        CommandDiagnosticClass::RecoverableError => {
                            DiagnosticClass::RecoverableError
                        }
                        CommandDiagnosticClass::Warning => DiagnosticClass::Warning,
                        CommandDiagnosticClass::Fatal => DiagnosticClass::Fatal,
                    },
                    severity: diagnostic_severity(severity),
                    diagnostic: diagnostic.into(),
                    arguments: arguments.into_iter().map(diagnostic_argument).collect(),
                    location: source_location(
                        location,
                        source,
                        source_id,
                        source_bytes,
                        source_line_starts,
                    ),
                },
                DiagnosticLifecycleRecord::Outcome { history, outcome } => {
                    DiagnosticLifecycleEvent::Outcome {
                        history: match history {
                            CommandDiagnosticHistory::Spotless => DiagnosticHistory::Spotless,
                            CommandDiagnosticHistory::WarningIssued => {
                                DiagnosticHistory::WarningIssued
                            }
                            CommandDiagnosticHistory::ErrorMessageIssued => {
                                DiagnosticHistory::ErrorMessageIssued
                            }
                            CommandDiagnosticHistory::FatalErrorStop => {
                                DiagnosticHistory::FatalErrorStop
                            }
                        },
                        outcome: match outcome {
                            CommandDiagnosticOutcome::Completed => DiagnosticOutcome::Completed,
                            CommandDiagnosticOutcome::Aborted => DiagnosticOutcome::Aborted,
                        },
                    }
                }
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
                    line,
                    source: _,
                } => GeometryEvent::Hpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                    location: Some(GeometryLocation {
                        source: source.into(),
                        line,
                    }),
                },
                GeometryRecord::Vpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                    line,
                    source: _,
                } => GeometryEvent::Vpack {
                    width_sp,
                    height_sp,
                    depth_sp,
                    location: Some(GeometryLocation {
                        source: source.into(),
                        line,
                    }),
                },
                GeometryRecord::Shipout {
                    page_width_sp,
                    page_height_sp,
                    counts,
                    line,
                    source: _,
                } => GeometryEvent::Shipout {
                    page_width_sp,
                    page_height_sp,
                    counts,
                    location: Some(GeometryLocation {
                        source: source.into(),
                        line,
                    }),
                },
            }),
            format!("source={source}"),
        ),
    }
}

fn diagnostic_severity(severity: &str) -> DiagnosticSeverity {
    match severity {
        "note" => DiagnosticSeverity::Note,
        "warning" => DiagnosticSeverity::Warning,
        "error" => DiagnosticSeverity::Error,
        "fatal" => DiagnosticSeverity::Fatal,
        invalid => panic!("command core published invalid diagnostic severity {invalid:?}"),
    }
}

fn diagnostic_argument(argument: tex_command::DiagnosticArgument) -> CanonicalValue {
    match argument {
        tex_command::DiagnosticArgument::Token(token) => CanonicalValue::Token(oracle_token(token)),
        tex_command::DiagnosticArgument::Name(name) => CanonicalValue::Name(name),
    }
}

pub(crate) fn observation_value(value: ObservationValue) -> CanonicalValue {
    match value {
        ObservationValue::None => CanonicalValue::None,
        ObservationValue::Integer(value) => CanonicalValue::Integer(value),
        ObservationValue::Character(value) => CanonicalValue::Character(value),
        ObservationValue::Scaled(value) => CanonicalValue::Scaled(value),
        ObservationValue::Glue {
            width,
            stretch,
            stretch_order,
            shrink,
            shrink_order,
        } => CanonicalValue::Glue {
            width,
            stretch,
            stretch_order: stretch_order.into(),
            shrink,
            shrink_order: shrink_order.into(),
        },
        ObservationValue::Name(value) => CanonicalValue::Name(value),
        ObservationValue::Bytes(value) => CanonicalValue::Bytes(value),
        ObservationValue::Tokens(tokens) => {
            CanonicalValue::Tokens(tokens.into_iter().map(oracle_token).collect())
        }
    }
}

pub(crate) fn command_location(
    record: &tex_command::CommandDeliveryRecord,
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    source_line_starts: Option<&[usize]>,
) -> Option<SourceLocation> {
    let location = record.provenance.source_location?;
    source_location(
        location,
        source,
        source_id,
        source_bytes,
        source_line_starts,
    )
}

pub(crate) fn source_location(
    location: tex_command::SourceLocation,
    source: &str,
    source_id: Option<SourceId>,
    source_bytes: Option<&[u8]>,
    source_line_starts: Option<&[usize]>,
) -> Option<SourceLocation> {
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

pub(crate) fn source_line_starts(bytes: &[u8]) -> Arc<[usize]> {
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
pub(crate) fn command_token(token: &ObservedToken) -> (CanonicalValue, Option<String>) {
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

pub(crate) fn oracle_token(token: ObservedToken) -> OracleToken {
    OracleToken {
        character: canonical_names::observed_token_character(&token),
        catcode: canonical_names::observed_token_catcode(&token).into(),
        control_sequence: canonical_names::observed_token_control_sequence(&token)
            .map(str::to_owned),
        location: None,
    }
}

pub(crate) fn translate_input(record: InputRecord, active_source: &str) -> Event {
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
        | CommandInputReason::EveryEof
        | CommandInputReason::Mark
        | CommandInputReason::Write
        | CommandInputReason::UmberReplay(_) => InputReason::TokenList,
    };
    let name = if record.reason == CommandInputReason::Source
        && record.transition == InputTransition::Stop
    {
        record
            .source_name
            .map(canonical_names::source_name_class_name)
            .unwrap_or("terminal")
            .into()
    } else if record.reason == CommandInputReason::Source {
        match record.source_name {
            // TeX82 §§328 and 483 give terminal and \read pseudo-files their
            // own exact `name` identity, but neither opens a registered
            // source in the translator's parallel source stack. Their
            // matching retirement must therefore name that level itself,
            // rather than name and pop the surrounding text file.
            Some(
                class @ (tex_command::SourceNameClass::Terminal
                | tex_command::SourceNameClass::ReadStream(_)),
            ) => canonical_names::source_name_class_name(class).into(),
            // A real file or e-TeX scantokens pseudo-file is activated by its
            // detached source record. Preserve that full source name instead
            // of the coarse numeric-name class.
            Some(
                tex_command::SourceNameClass::Scantokens(_) | tex_command::SourceNameClass::File,
            )
            | None => active_source.into(),
        }
    } else {
        canonical_names::input_level_name(record.reason)
            .map_or_else(|| active_source.into(), Into::into)
    };
    Event::Input(InputEvent {
        transition,
        reason,
        // TeX82's `end_file_reading` observer carries only the lifecycle
        // transition.  The harness attaches the source identity while the
        // source frame is still active, before it removes that frame from
        // its parallel trace stack. Conversely, `begin_file_reading` is
        // observed before that stack activates the child, so a source push
        // keeps the canonical name carried by the event itself.
        name,
    })
}

pub(crate) fn translate_recovery(record: RecoveryRecord) -> Event {
    Event::Recovery(RecoveryEvent {
        kind: match record.kind {
            CommandRecoveryKind::Backup => RecoveryKind::Backup,
            CommandRecoveryKind::InsertedToken => RecoveryKind::InsertedToken,
            CommandRecoveryKind::InsertedControlSequence => RecoveryKind::InsertedControlSequence,
        },
        tokens: record.tokens.into_iter().map(oracle_token).collect(),
    })
}
pub(crate) fn translate_status(record: ScannerStatusRecord) -> Event {
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
pub(crate) fn scanner_status(status: &str) -> ScannerStatus {
    match status {
        "skipping" => ScannerStatus::Skipping,
        "defining" => ScannerStatus::Defining,
        "matching" => ScannerStatus::Matching,
        "aligning" => ScannerStatus::Aligning,
        "absorbing" => ScannerStatus::Absorbing,
        _ => ScannerStatus::Normal,
    }
}
pub(crate) fn translate_macro(record: MacroRecord) -> Event {
    match record {
        MacroRecord::Activation {
            control_sequence,
            argument_count,
            ..
        } => Event::Macro(MacroEvent::Activation {
            control_sequence,
            argument_count: argument_count.into(),
        }),
        MacroRecord::Argument {
            parameter, tokens, ..
        } => Event::Macro(MacroEvent::Argument {
            parameter: parameter.into(),
            tokens: tokens.into_iter().map(oracle_token).collect(),
        }),
    }
}
pub(crate) fn translate_condition(record: ConditionRecord) -> Event {
    let transition = match record.transition {
        "push" => ConditionTransition::Push,
        "limit" => ConditionTransition::LimitChange,
        "branch" => ConditionTransition::Branch,
        _ => ConditionTransition::Pop,
    };
    Event::Condition(ConditionEvent {
        transition,
        condition: record.condition,
        limit: record.limit.into(),
        branch: record.branch,
    })
}
pub(crate) fn translate_token_list(record: TokenListRecord) -> Event {
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
pub(crate) fn translate_alignment(record: AlignmentRecord) -> Event {
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
        nesting: record.nesting,
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
pub(crate) fn translate_mutation(record: MutationRecord) -> Event {
    Event::Mutation(MutationEvent {
        target: match record.target {
            MutationTarget::Meaning => StateTarget::Meaning,
            MutationTarget::Catcode => StateTarget::Catcode,
            MutationTarget::CodeTable => StateTarget::CodeTable,
            MutationTarget::Parameter => StateTarget::Parameter,
            MutationTarget::Register => StateTarget::Register,
        },
        key: observation_value(record.key),
        value: observation_value(record.value),
        scope: if record.global { "global" } else { "local" }.into(),
    })
}

pub(crate) fn translate_effect(record: EffectRecord) -> Event {
    Event::Effect(EffectEvent {
        kind: match record.kind {
            ObservationEffectKind::Message => EffectKind::Message,
            ObservationEffectKind::Write => EffectKind::Write,
            ObservationEffectKind::Open => EffectKind::Open,
            ObservationEffectKind::Close => EffectKind::Close,
            ObservationEffectKind::Shipout => EffectKind::Shipout,
            ObservationEffectKind::Input
            | ObservationEffectKind::Terminate
            | ObservationEffectKind::ShowTokens
            | ObservationEffectKind::ShowIfs
            | ObservationEffectKind::ShowGroups => EffectKind::Terminate,
        },
        channel: record.channel,
        value: observation_value(record.value),
    })
}

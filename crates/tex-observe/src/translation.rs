use super::*;

pub(crate) fn translate_observation(
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
                "integer"
                    | "interaction_mode"
                    | "expression_integer"
                    | "current_group_level"
                    | "current_group_type"
                    | "current_condition_level"
                    | "current_condition_type"
                    | "current_condition_branch"
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

pub(crate) fn parse_glue_scanner_value(value: &str) -> Option<CanonicalValue> {
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

pub(crate) fn command_location(
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
/// Projects TeX82's `align_ptr` stack onto the portable one-based nesting
/// field. Alignment identities are process-local replay handles: §37's
/// `fin_align` calls `pop_alignment`, so a later independent alignment can
/// have a larger identity while returning to nesting one.
#[derive(Debug, Default)]
pub(crate) struct AlignmentNesting {
    stack: Vec<u64>,
}

impl AlignmentNesting {
    pub(crate) fn observe(&mut self, record: &AlignmentRecord) -> Option<u32> {
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

pub(crate) fn translate_alignment(record: AlignmentRecord, nesting: Option<u32>) -> Event {
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
pub(crate) fn translate_mutation(record: MutationRecord) -> Event {
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
pub(crate) fn canonical_catcode_assignment(value: &str) -> Option<&'static str> {
    canonical_names::catcode_assignment_name(value.parse::<i64>().ok()?)
}
pub(crate) fn translate_effect(record: EffectRecord) -> Event {
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

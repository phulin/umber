use core::cell::Cell;

use super::*;

fn profile() -> DetachedCommandProfile {
    DetachedCommandProfile {
        schema: COMMAND_CONTINUATION_SCHEMA_VERSION,
        fingerprint: 0x1234_5678,
        dialect: 2,
        character_mode: 1,
    }
}

fn valid_continuation() -> OwnedCommandContinuation {
    let mut builder = ContinuationRecipeBuilder::new(profile());
    let source = builder
        .push_source(SourceRecipe::Generated {
            logical_path: Some("job.tex".to_owned()),
            bytes: b"hello".to_vec(),
        })
        .expect("source index");
    let source_point = builder
        .push_origin(OriginRecipe::SourcePoint {
            source,
            byte: 0,
            line: 1,
            column: 1,
        })
        .expect("source origin index");
    let name = builder
        .push_name(NameRecipe {
            kind: DetachedNameKind::MultiLetter,
            spelling: "hello".to_owned(),
        })
        .expect("name index");
    let words = builder
        .push_token_list(TokenListRecipe {
            words: vec![DetachedWord {
                token: DetachedToken::ControlSequence(name),
                origin: source_point,
            }],
        })
        .expect("token list index");
    let parameter_text = builder
        .push_token_list(TokenListRecipe {
            words: vec![DetachedWord {
                token: DetachedToken::Parameter(1),
                origin: source_point,
            }],
        })
        .expect("parameter-text index");
    let parameter_origins = builder
        .push_origin_list(OriginListRecipe {
            origins: vec![source_point],
        })
        .expect("parameter-origin index");
    let replacement_origins = builder
        .push_origin_list(OriginListRecipe {
            origins: vec![source_point],
        })
        .expect("replacement-origin index");
    let definition = builder
        .push_macro(MacroRecipe {
            flags: 0,
            parameter_text,
            replacement_text: words,
            definition_origin: source_point,
            parameter_origins,
            replacement_origins,
        })
        .expect("macro index");
    let summary = CommandSummaryRecipe {
        input: vec![
            InputFrameRecipe::Source(SourceFrameRecipe {
                source,
                next_physical_byte: 5,
                next_line: 2,
                line: None,
                lexer_state: 0,
                end_after_line: false,
                name_class: 0,
                retirement: 0,
                every_eof: Some(words),
                group_depth: 0,
                condition_depth: 0,
            }),
            InputFrameRecipe::Tokens(TokenFrameRecipe {
                payload: InputPayloadRecipe::Arguments {
                    words,
                    ranges: [
                        Some(RecipeRange { start: 0, len: 1 }),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ],
                },
                replay: DetachedReplayKind::MacroArgument,
                index: 0,
            }),
        ],
        pending_sources: Vec::new(),
        activations: vec![ActivationRecipe {
            name,
            definition,
            arguments: words,
            ranges: [
                Some(RecipeRange { start: 0, len: 1 }),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
            invocation: source_point,
        }],
        conditions: Vec::new(),
        align_state: 1_000_000,
    };
    builder.finish(summary).expect("valid recipes")
}

#[derive(Debug, Default, Eq, PartialEq)]
struct LiveDestination {
    names: Vec<String>,
}

#[test]
fn complete_recipe_graph_materializes_with_one_atomic_publication() {
    let continuation = valid_continuation();
    let mut destination = CommandContinuationDestination::new(LiveDestination::default());

    continuation
        .materialize(
            &mut destination,
            CommandContinuationLimits::default(),
            |_live, validated| {
                Ok::<_, ()>(
                    validated
                        .schema()
                        .names
                        .iter()
                        .map(|name| name.spelling.clone())
                        .collect::<Vec<_>>(),
                )
            },
            |live, names| live.names = names,
        )
        .expect("materialization succeeds");

    assert_eq!(destination.live().names, ["hello"]);
}

#[test]
fn malformed_recipe_rejects_before_staging_or_publication() {
    let mut continuation = valid_continuation();
    let InputFrameRecipe::Source(frame) = &mut continuation.schema.summary.input[0] else {
        panic!("source frame");
    };
    frame.next_physical_byte = 6;
    let builds = Cell::new(0);
    let mut destination = CommandContinuationDestination::new(LiveDestination {
        names: vec!["existing".to_owned()],
    });

    let result = continuation.materialize(
        &mut destination,
        CommandContinuationLimits::default(),
        |_live, _validated| {
            builds.set(builds.get() + 1);
            Ok::<_, ()>(Vec::<String>::new())
        },
        |live, names| live.names = names,
    );

    assert_eq!(
        result,
        Err(MaterializationError::Continuation(
            CommandContinuationError::InvalidRecipe("source cursor exceeds its source bytes")
        ))
    );
    assert_eq!(builds.get(), 0);
    assert_eq!(destination.live().names, ["existing"]);
}

#[test]
fn failed_destination_rebuild_leaves_live_state_unchanged() {
    let continuation = valid_continuation();
    let mut destination = CommandContinuationDestination::new(LiveDestination {
        names: vec!["existing".to_owned()],
    });

    let result = continuation.materialize(
        &mut destination,
        CommandContinuationLimits::default(),
        |_live, _validated| Err::<Vec<String>, _>("allocation failed"),
        |live, names| live.names = names,
    );

    assert_eq!(
        result,
        Err(MaterializationError::Build("allocation failed"))
    );
    assert_eq!(destination.live().names, ["existing"]);
}

#[test]
fn staged_graph_is_stamped_for_exactly_one_destination() {
    let continuation = valid_continuation();
    let first = CommandContinuationDestination::new(LiveDestination::default());
    let mut second = CommandContinuationDestination::new(LiveDestination::default());
    let staged = first
        .stage(
            &continuation,
            CommandContinuationLimits::default(),
            |_live, _validated| Ok::<_, ()>(vec!["rebuilt".to_owned()]),
        )
        .expect("stage");

    assert_eq!(
        second.publish(staged, |live, names| live.names = names),
        Err(CommandContinuationError::ForeignDestination)
    );
    assert!(first.live().names.is_empty());
    assert!(second.live().names.is_empty());
}

#[test]
fn logical_origin_cycles_are_rejected() {
    let mut continuation = valid_continuation();
    continuation.schema.origins[0] = OriginRecipe::Derived {
        operation: DetachedOriginOperation::Synthesized,
        primary: OriginRecipeIndex::from_len(0).expect("zero index"),
        related: None,
    };

    assert_eq!(
        continuation.validate(CommandContinuationLimits::default()),
        Err(CommandContinuationError::InvalidRecipe(
            "origin recipes contain a cycle"
        ))
    );
}

#[test]
fn admission_limits_reject_before_destination_building() {
    let continuation = valid_continuation();
    let limits = CommandContinuationLimits {
        tokens: 0,
        ..CommandContinuationLimits::default()
    };
    let builds = Cell::new(0);
    let destination = CommandContinuationDestination::new(LiveDestination::default());

    let result = destination.stage(&continuation, limits, |_live, _validated| {
        builds.set(builds.get() + 1);
        Ok::<_, ()>(())
    });

    assert!(matches!(
        result,
        Err(MaterializationError::Continuation(
            CommandContinuationError::LimitExceeded("tokens")
        ))
    ));
    assert_eq!(builds.get(), 0);
}

#[test]
fn detached_schema_source_names_no_runtime_storage_types() {
    let source = include_str!("schema.rs");
    for forbidden in [
        "tex_state::",
        "Symbol",
        "DefinitionRef",
        "NodeId",
        "SourceId",
        "ArenaOffset",
        "Arc<",
        "GenerationOwner",
        "GenerationKey",
        "JournalCursor",
    ] {
        assert!(
            !source.contains(forbidden),
            "detached schema contains forbidden runtime type {forbidden}"
        );
    }
}

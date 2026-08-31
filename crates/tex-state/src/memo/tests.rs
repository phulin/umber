use super::*;
use crate::interner::InternerBudget;
use crate::scaled::Scaled;
use crate::universe::with_universe;

fn budget() -> InternerBudget {
    InternerBudget::new(256, 512, 8 * 1024).expect("test fixture is valid")
}

#[test]
fn token_lists_cross_generation_only_as_spellings_and_values() {
    let detached = with_universe(budget(), |source| {
        let control = source.intern("answer").expect("test fixture is valid");
        let words = [
            TokenWord::pack(Token::Cs(control.symbol())),
            TokenWord::pack(Token::Char {
                ch: 'λ',
                cat: Catcode::Letter,
            }),
            TokenWord::pack(Token::param(2)),
        ];
        let id = source
            .allocate_token_list(&words)
            .expect("test fixture is valid");
        source.detach_token_list(id).expect("test fixture is valid")
    })
    .expect("test fixture is valid");

    with_universe(budget(), |destination| {
        let id = destination
            .import_memo_token_list(&detached, MemoValueLimits::default())
            .expect("test fixture is valid");
        let context = destination
            .command_context()
            .expect("test fixture is valid");
        let tokens = context
            .token_list(id)
            .iter()
            .map(|word| word.semantic_token())
            .collect::<Vec<_>>();
        assert_eq!(
            tokens[1],
            Token::Char {
                ch: 'λ',
                cat: Catcode::Letter
            }
        );
        assert_eq!(tokens[2], Token::param(2));
        let Token::Cs(symbol) = tokens[0] else {
            panic!("control sequence")
        };
        assert_eq!(context.resolve(symbol), "answer");
    })
    .expect("test fixture is valid");
}

#[test]
fn macro_materialization_stages_then_publishes_one_definition() {
    let detached = with_universe(budget(), |source| {
        let name = source.intern("x").expect("test fixture is valid");
        let parameters = [TokenWord::pack(Token::param(1))];
        let replacement = [TokenWord::pack(Token::Cs(name.symbol()))];
        let id = source
            .allocate_definition(&parameters, &replacement)
            .expect("test fixture is valid");
        source
            .detach_macro_meaning(MeaningFlags::LONG | MeaningFlags::PROTECTED, id)
            .expect("test fixture is valid")
    })
    .expect("test fixture is valid");
    let staged = detached
        .stage_macro(MemoValueLimits::default())
        .expect("test fixture is valid");

    with_universe(budget(), |destination| {
        let meaning = destination
            .publish_memo_macro(staged)
            .expect("test fixture is valid")
            .resolve();
        let crate::meaning::ResolvedMeaning::Macro { flags, definition } = meaning else {
            panic!("macro meaning")
        };
        assert!(flags.contains(MeaningFlags::LONG));
        assert!(flags.contains(MeaningFlags::PROTECTED));
        let context = destination
            .command_context()
            .expect("test fixture is valid");
        let definition = context.definition(definition);
        assert_eq!(
            definition.parameter_text()[0].semantic_token(),
            Token::param(1)
        );
        let Token::Cs(symbol) = definition.replacement_text()[0].semantic_token() else {
            panic!("replacement control sequence")
        };
        assert_eq!(context.resolve(symbol), "x");
    })
    .expect("test fixture is valid");
}

#[test]
fn malformed_macro_program_rejects_without_publishing_or_panicking() {
    let staged = StagedMemoMacro {
        value: DetachedMacro {
            flags: 0,
            parameter_text: vec![DetachedToken::Param(2)],
            replacement_text: vec![],
        },
    };

    with_universe(budget(), |destination| {
        let core = destination.core.as_ref().expect("live state");
        let cursor = core.generation_cursor();
        let accounting = core.memory_accounting().words(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            destination.publish_memo_macro(staged)
        }));
        assert_eq!(
            result.expect("malformed memo macro must not panic"),
            Err(MemoValueError::Publication(
                PromotionError::InvalidDefinition
            ))
        );
        let core = destination.core.as_ref().expect("live state");
        assert_eq!(core.generation_cursor(), cursor);
        assert_eq!(core.memory_accounting().words(false), accounting);
    })
    .expect("test fixture is valid");
}

#[test]
fn memo_publication_uses_destination_definition_identity_policy() {
    let detached = with_universe(budget(), |source| {
        let parameter = [TokenWord::pack(Token::param(1))];
        let replacement = [TokenWord::pack(Token::param(1))];
        let definition = source
            .allocate_definition(&parameter, &replacement)
            .expect("source definition");
        source
            .detach_macro_meaning(MeaningFlags::EMPTY, definition)
            .expect("detached macro")
    })
    .expect("test fixture is valid");

    with_universe(budget(), |destination| {
        assert!(destination.enable_reachable_state_identity());
        let parameter = [TokenWord::pack(Token::param(1))];
        let replacement = [TokenWord::pack(Token::param(1))];
        let direct = destination
            .allocate_definition(&parameter, &replacement)
            .expect("direct definition");
        let imported = destination
            .import_memo_macro_meaning(&detached, MemoValueLimits::default())
            .expect("imported macro")
            .resolve();
        let crate::meaning::ResolvedMeaning::Macro {
            definition: imported,
            ..
        } = imported
        else {
            panic!("macro meaning")
        };
        let context = destination.command_context().expect("command context");
        assert_eq!(
            context.definition(direct).semantic_identity(),
            context.definition(imported).semantic_identity()
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn glue_round_trip_is_handle_free_and_semantic() {
    let value = GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(20),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(3),
        shrink_order: Order::Fil,
    };
    let detached = with_universe(budget(), |source| {
        let id = source.allocate_glue(value).expect("test fixture is valid");
        source.detach_glue(id).expect("test fixture is valid")
    })
    .expect("test fixture is valid");
    assert!(
        !detached
            .payload(MemoValueKind::Glue)
            .expect("test fixture is valid")
            .is_empty()
    );
    with_universe(budget(), |destination| {
        let id = destination
            .import_memo_glue(&detached)
            .expect("test fixture is valid");
        assert_eq!(
            destination
                .command_context()
                .expect("test fixture is valid")
                .glue(id),
            value
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn envelope_rejects_corruption_stale_schema_and_wrong_kind() {
    let value = DetachedMemoValue::from_artifact(&DetachedArtifact {
        artifact_schema: 7,
        payload: vec![1, 2, 3],
    })
    .expect("test fixture is valid");
    let bytes = value.to_bytes().expect("test fixture is valid");
    assert_eq!(
        DetachedMemoValue::from_bytes(&bytes, MemoValueLimits::default())
            .expect("test fixture is valid"),
        value
    );
    assert!(matches!(
        value.stage_glue(),
        Err(MemoValueError::Kind { .. })
    ));

    let mut wire: WireEnvelope = bincode::deserialize(&bytes).expect("test fixture is valid");
    wire.payload.push(9);
    let corrupt = bincode::serialize(&wire).expect("test fixture is valid");
    assert_eq!(
        DetachedMemoValue::from_bytes(&corrupt, MemoValueLimits::default()),
        Err(MemoValueError::Integrity)
    );
    wire.schema = MEMO_VALUE_SCHEMA_VERSION - 1;
    let stale = bincode::serialize(&wire).expect("test fixture is valid");
    assert_eq!(
        DetachedMemoValue::from_bytes(&stale, MemoValueLimits::default()),
        Err(MemoValueError::StaleSchema {
            found: MEMO_VALUE_SCHEMA_VERSION - 1
        })
    );
}

#[test]
fn malformed_values_fail_during_staging_before_publication() {
    let tokens = [DetachedToken::Param(0)];
    let malformed = DetachedMemoValue::encode(MemoValueKind::Tokens, &tokens[..])
        .expect("test fixture is valid");
    assert!(matches!(
        malformed.stage_token_list(MemoValueLimits::default()),
        Err(MemoValueError::Invalid("invalid parameter slot"))
    ));
}

#[test]
fn generic_detached_payload_families_remain_bounded() {
    let diagnostics = vec![DetachedDiagnostic {
        code: "E".into(),
        message: "message".into(),
        input_ordinal: Some(4),
    }];
    let value = DetachedMemoValue::from_diagnostics(&diagnostics).expect("test fixture is valid");
    assert_eq!(
        value
            .diagnostics(MemoValueLimits::default())
            .expect("test fixture is valid"),
        diagnostics
    );
    let limits = MemoValueLimits {
        max_tokens: 0,
        ..MemoValueLimits::default()
    };
    assert!(matches!(
        value.diagnostics(limits),
        Err(MemoValueError::Oversized { .. })
    ));
}

use super::*;
use crate::interner::InternerBudget;
use crate::scaled::Scaled;
use crate::universe::with_universe;

fn budget() -> InternerBudget {
    InternerBudget::new(256, 512, 8 * 1024).unwrap()
}

#[test]
fn token_lists_cross_generation_only_as_spellings_and_values() {
    let detached = with_universe(budget(), |source| {
        let control = source.intern("answer").unwrap();
        let words = [
            TokenWord::pack(Token::Cs(control.symbol())),
            TokenWord::pack(Token::Char {
                ch: 'λ',
                cat: Catcode::Letter,
            }),
            TokenWord::pack(Token::param(2)),
        ];
        let id = source.allocate_token_list(&words).unwrap();
        source.detach_token_list(id).unwrap()
    })
    .unwrap();

    with_universe(budget(), |destination| {
        let id = destination
            .import_memo_token_list(&detached, MemoValueLimits::default())
            .unwrap();
        let context = destination.command_context().unwrap();
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
        assert_eq!(context.resolve(symbol), Some("answer"));
    })
    .unwrap();
}

#[test]
fn macro_materialization_stages_then_publishes_one_definition() {
    let detached = with_universe(budget(), |source| {
        let name = source.intern("x").unwrap();
        let parameters = [TokenWord::pack(Token::param(1))];
        let replacement = [TokenWord::pack(Token::Cs(name.symbol()))];
        let id = source
            .allocate_definition(&parameters, &replacement)
            .unwrap();
        source
            .detach_macro_meaning(MeaningFlags::LONG | MeaningFlags::PROTECTED, id)
            .unwrap()
    })
    .unwrap();
    let staged = detached.stage_macro(MemoValueLimits::default()).unwrap();

    with_universe(budget(), |destination| {
        let meaning = destination.publish_memo_macro(staged).unwrap().resolve();
        let crate::meaning::ResolvedMeaning::Macro { flags, definition } = meaning else {
            panic!("macro meaning")
        };
        assert!(flags.contains(MeaningFlags::LONG));
        assert!(flags.contains(MeaningFlags::PROTECTED));
        let context = destination.command_context().unwrap();
        let definition = context.definition(definition);
        assert_eq!(
            definition.parameter_text()[0].semantic_token(),
            Token::param(1)
        );
        let Token::Cs(symbol) = definition.replacement_text()[0].semantic_token() else {
            panic!("replacement control sequence")
        };
        assert_eq!(context.resolve(symbol), Some("x"));
    })
    .unwrap();
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
        let id = source.allocate_glue(value).unwrap();
        source.detach_glue(id).unwrap()
    })
    .unwrap();
    assert!(!detached.payload(MemoValueKind::Glue).unwrap().is_empty());
    with_universe(budget(), |destination| {
        let id = destination.import_memo_glue(&detached).unwrap();
        assert_eq!(destination.command_context().unwrap().glue(id), value);
    })
    .unwrap();
}

#[test]
fn envelope_rejects_corruption_stale_schema_and_wrong_kind() {
    let value = DetachedMemoValue::from_artifact(&DetachedArtifact {
        artifact_schema: 7,
        payload: vec![1, 2, 3],
    })
    .unwrap();
    let bytes = value.to_bytes().unwrap();
    assert_eq!(
        DetachedMemoValue::from_bytes(&bytes, MemoValueLimits::default()).unwrap(),
        value
    );
    assert!(matches!(
        value.stage_glue(),
        Err(MemoValueError::Kind { .. })
    ));

    let mut wire: WireEnvelope = bincode::deserialize(&bytes).unwrap();
    wire.payload.push(9);
    let corrupt = bincode::serialize(&wire).unwrap();
    assert_eq!(
        DetachedMemoValue::from_bytes(&corrupt, MemoValueLimits::default()),
        Err(MemoValueError::Integrity)
    );
    wire.schema = MEMO_VALUE_SCHEMA_VERSION - 1;
    let stale = bincode::serialize(&wire).unwrap();
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
    let malformed = DetachedMemoValue::encode(MemoValueKind::Tokens, &tokens[..]).unwrap();
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
    let value = DetachedMemoValue::from_diagnostics(&diagnostics).unwrap();
    assert_eq!(
        value.diagnostics(MemoValueLimits::default()).unwrap(),
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

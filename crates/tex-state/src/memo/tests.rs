use super::*;

#[test]
fn tokens_round_trip_into_independent_universe_without_origin_or_handle_identity() {
    let mut source = Universe::new();
    let named = source.intern("memo-name");
    let active = source.intern_active_character('!');
    let id = source.intern_token_list(&[
        Token::Cs(named.symbol()),
        Token::Cs(active.symbol()),
        Token::Char {
            ch: 'β',
            cat: Catcode::Letter,
        },
        Token::param(2),
    ]);
    let detached = source.detach_token_list(id).expect("token detachment");

    let mut target = Universe::new();
    let imported = target
        .import_memo_token_list(&detached, MemoValueLimits::default())
        .expect("token import");
    assert_eq!(target.tokens(imported).len(), 4);
    let Token::Cs(symbol) = target.tokens(imported)[0] else {
        panic!("expected imported control sequence");
    };
    assert_eq!(target.resolve(symbol), "memo-name");
    assert_eq!(detached.kind(), MemoValueKind::Tokens);
}

#[test]
fn envelope_rejects_corruption_schema_kind_and_oversize() {
    let mut universe = Universe::new();
    let id = universe.intern_token_list(&[Token::Char {
        ch: 'x',
        cat: Catcode::Letter,
    }]);
    let detached = universe.detach_token_list(id).expect("token detachment");
    let mut bytes = detached.to_bytes().expect("memo encoding");
    *bytes.last_mut().expect("encoded envelope is nonempty") ^= 1;
    assert!(DetachedMemoValue::from_bytes(&bytes, MemoValueLimits::default()).is_err());

    assert!(matches!(
        universe.import_memo_glue(&detached),
        Err(MemoValueError::Kind { .. })
    ));
    assert!(matches!(
        DetachedMemoValue::from_bytes(
            &detached.to_bytes().expect("memo encoding"),
            MemoValueLimits {
                max_payload_bytes: 0,
                ..MemoValueLimits::default()
            }
        ),
        Err(MemoValueError::Oversized { .. })
    ));
}

#[test]
fn glue_and_macro_round_trip_semantically() {
    let mut source = Universe::new();
    let glue = source.intern_glue(GlueSpec {
        width: crate::scaled::Scaled::from_raw(10),
        stretch: crate::scaled::Scaled::from_raw(20),
        stretch_order: Order::Fil,
        shrink: crate::scaled::Scaled::from_raw(3),
        shrink_order: Order::Normal,
    });
    let detached_glue = source.detach_glue(glue).expect("glue detachment");

    let parameters = source.intern_token_list(&[Token::param(1)]);
    let replacement = source.intern_token_list(&[Token::Char {
        ch: 'z',
        cat: Catcode::Letter,
    }]);
    let definition = source.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        parameters,
        replacement,
    ));
    let detached_macro = source
        .detach_macro_meaning(definition)
        .expect("macro detachment");

    let mut target = Universe::new();
    let imported_glue = target
        .import_memo_glue(&detached_glue)
        .expect("glue import");
    assert_eq!(target.glue(imported_glue), source.glue(glue));
    let imported_macro = target
        .import_memo_macro_meaning(&detached_macro, MemoValueLimits::default())
        .expect("macro import");
    let meaning = target.macro_definition(imported_macro);
    assert_eq!(meaning.flags(), MeaningFlags::LONG);
    assert_eq!(
        target.tokens(meaning.replacement_text())[0],
        Token::Char {
            ch: 'z',
            cat: Catcode::Letter
        }
    );
}

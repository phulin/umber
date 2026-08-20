use tex_state::Universe;
use tex_state::ids::TokenListId;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::{Catcode, OriginId, RootedTracedTokenBuffer, Token, TracedTokenWord};

use super::{
    BackedUpToken, BackupTreatment, InputLevel, InputLevelId, ReplayTrace, RetirementBehavior,
    RootedBackedUpToken, StoredReplayReason, TokenBehavior, TokenCursor, TokenPayload,
    packed_token_frame,
};
use crate::macro_call::MacroArgumentRange;

fn traced(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn packed_transient_chunks_own_only_distinct_structural_origins() {
    let mut universe = Universe::new();
    let root = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let first = tex_state::token::RootedTracedTokenWord::new(
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
        root.clone(),
    );
    let second = tex_state::token::RootedTracedTokenWord::new(
        Token::Char {
            ch: 'b',
            cat: Catcode::Other,
        },
        root,
    );
    let rooted = TokenPayload::transient_rooted([first, second]);
    let direct = TokenPayload::transient([traced('c'), traced('d')]);

    let TokenPayload::Packed(rooted) = rooted else {
        panic!("transient payload is packed");
    };
    let TokenPayload::Packed(direct) = direct else {
        panic!("transient payload is packed");
    };
    assert_eq!(rooted.words.roots().len(), 1);
    assert!(rooted.source_provenance.is_empty());
    assert!(direct.words.roots().is_empty());
    assert!(direct.source_provenance.is_empty());
    drop(universe);
    assert!(
        rooted
            .words
            .rooted_words()
            .all(|word| word.origin_ref().record().is_some())
    );
}

#[test]
fn generated_shared_origin_run_has_one_root_and_no_none_sidecar() {
    let mut universe = Universe::new();
    let root = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let root_id = root.id();
    let token = Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    };
    let payload = TokenPayload::transient_with_shared_origin([token; 32], root);
    let TokenPayload::Packed(chunk) = payload else {
        panic!("transient payload is packed");
    };

    assert_eq!(chunk.words.len(), 32);
    assert_eq!(chunk.words.roots().len(), 1);
    assert_eq!(chunk.words.roots()[0].id(), root_id);
    assert!(chunk.source_provenance.is_empty());
    assert_eq!(chunk.get(31).expect("last generated token").1, None);
}

#[test]
fn packed_rooted_buffer_preserves_words_and_structural_owners() {
    let mut universe = Universe::new();
    let root = universe.synthetic_origin_ref(SyntheticOriginKind::Test);
    let token = tex_state::token::RootedTracedTokenWord::new(
        Token::Char {
            ch: 'a',
            cat: Catcode::Other,
        },
        root,
    );
    let mut source = RootedTracedTokenBuffer::new([token]);
    source.push_unowned(traced('b'));

    let payload = TokenPayload::transient_rooted(source.rooted_words());
    let TokenPayload::Packed(chunk) = payload else {
        panic!("transient payload is packed");
    };

    assert_eq!(chunk.words.words().len(), 2);
    assert_eq!(chunk.words.roots().len(), 1);
    drop(universe);
    assert!(
        chunk
            .words
            .get_rooted(0)
            .expect("first word")
            .origin_ref()
            .record()
            .is_some()
    );
    assert!(
        chunk
            .words
            .get_rooted(1)
            .expect("second word")
            .origin_ref()
            .record()
            .is_none()
    );
}

fn rooted_backup(ch: char) -> RootedBackedUpToken {
    RootedBackedUpToken::unowned(BackedUpToken {
        spelling: traced(ch),
        source_provenance: None,
    })
}

#[test]
fn token_cursor_classifies_orthogonal_ownership_domains() {
    let universe = Universe::new();
    let behavior = TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence);
    let retirement = RetirementBehavior::StopAtEnd;
    let trace = ReplayTrace::Stored(StoredReplayReason::EveryJob);
    let mut frame = packed_token_frame(InputLevelId(5), 3, &behavior, retirement, &trace);
    for _ in 0..3 {
        let _ = frame.advance();
    }
    let cursor = TokenCursor {
        payload: TokenPayload::stored(
            universe.tokens(TokenListId::EMPTY).tokens(),
            tex_state::provenance::OriginListRef::empty(),
        ),
        behavior,
        retirement,
        trace,
        frame,
    };

    let TokenCursor {
        payload,
        behavior,
        retirement,
        trace,
        frame,
    } = cursor;
    assert!(matches!(payload, TokenPayload::Packed(_)));
    assert!(matches!(
        behavior,
        TokenBehavior::BackedUp(BackupTreatment::SuppressExpandableControlSequence)
    ));
    assert_eq!(retirement, RetirementBehavior::StopAtEnd);
    assert_eq!(trace, ReplayTrace::Stored(StoredReplayReason::EveryJob));
    assert_eq!(frame.position(), 3);
    assert_eq!(frame.identity(), 5);
    assert_eq!(frame.position(), 3);
}

#[test]
fn macro_argument_ranges_share_one_contiguous_allocation() {
    let arguments = crate::macro_call::MacroArguments::default();
    let payload = TokenPayload::ArgumentRange {
        arguments,
        range: MacroArgumentRange::new(1, 3).expect("valid argument range"),
    };

    let TokenPayload::ArgumentRange {
        arguments: retained,
        range,
    } = payload
    else {
        panic!("argument payload changed variant");
    };
    assert_eq!(retained, arguments);
    assert_eq!((range.start(), range.end()), (1, 3));
}

#[test]
fn packed_backup_prepends_without_reversing_existing_tokens() {
    let mut payload = TokenPayload::backed_up([
        BackedUpToken {
            spelling: traced('b'),
            source_provenance: None,
        },
        BackedUpToken {
            spelling: traced('c'),
            source_provenance: None,
        },
    ]);

    payload
        .prepend_backed_up([rooted_backup('a')])
        .expect("backup prepends");

    assert_eq!(
        (0..3)
            .map(|index| payload.backed_up_get(index).expect("token exists").spelling)
            .collect::<Vec<_>>(),
        [traced('a'), traced('b'), traced('c')]
    );
}

#[test]
fn single_token_payload_constructors_select_packed_storage() {
    let transient = TokenPayload::transient([traced('a')]);
    assert!(matches!(transient, TokenPayload::Packed(chunk) if chunk.word(0) == Some(traced('a'))));

    let backed_up = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('b'),
        source_provenance: None,
    }]);
    assert!(matches!(
        backed_up,
        TokenPayload::Packed(chunk)
            if chunk.backed_up_token(0).map(|word| word.spelling) == Some(traced('b'))
    ));
}

#[test]
fn multi_token_payload_constructors_select_packed_storage() {
    let transient = TokenPayload::transient([traced('a'), traced('b')]);
    assert!(matches!(
        transient,
        TokenPayload::Packed(chunk) if chunk.words.words() == [traced('a'), traced('b')]
    ));

    let backed_up = TokenPayload::backed_up([
        BackedUpToken {
            spelling: traced('a'),
            source_provenance: None,
        },
        BackedUpToken {
            spelling: traced('b'),
            source_provenance: None,
        },
    ]);
    assert!(matches!(
        backed_up,
        TokenPayload::Packed(chunk)
            if (0..2)
                .map(|index| {
                    chunk
                        .backed_up_token(index)
                        .expect("the constructor must retain each backed-up token")
                        .spelling
                })
                .collect::<Vec<_>>()
                == [traced('a'), traced('b')]
    ));
}

#[test]
fn prepend_extends_packed_backup_without_reordering() {
    let mut payload = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('c'),
        source_provenance: None,
    }]);
    payload
        .prepend_backed_up([rooted_backup('a'), rooted_backup('b')])
        .expect("backed-up payload promotes");

    assert!(matches!(payload, TokenPayload::Packed(_)));
    assert_eq!(
        (0..3)
            .map(|index| payload
                .backed_up_get(index)
                .expect("backed-up word")
                .spelling)
            .collect::<Vec<_>>(),
        [traced('a'), traced('b'), traced('c')]
    );
}

#[test]
fn packed_payload_origin_adoption_preserves_semantics() {
    let mut universe = tex_state::Universe::new();
    let recorded_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let live_origin = universe.synthetic_origin_ref(SyntheticOriginKind::Primitive);
    let token = Token::Char {
        ch: 'a',
        cat: Catcode::Other,
    };
    let mut recorded =
        TokenPayload::transient_rooted([tex_state::token::RootedTracedTokenWord::new(
            token,
            recorded_origin,
        )]);
    let live = TokenPayload::transient_rooted([tex_state::token::RootedTracedTokenWord::new(
        token,
        live_origin.clone(),
    )]);

    recorded
        .adopt_matching_origins(&live)
        .expect("matching inline token adopts live origin");
    assert_eq!(
        match recorded {
            TokenPayload::Packed(chunk) => chunk.word(0).expect("singleton word").origin(),
            _ => panic!("singleton remains packed"),
        },
        live_origin.id()
    );
}

#[test]
fn packed_backup_rehomes_source_provenance_in_place() {
    let old_source = tex_state::SourceId::new(3);
    let new_source = tex_state::SourceId::new(7);
    let provenance =
        crate::SourceProvenance::from_range(crate::SourceRange::new(old_source, 10, 12));
    let mut payload = TokenPayload::backed_up([BackedUpToken {
        spelling: traced('a'),
        source_provenance: Some(provenance),
    }]);

    payload
        .rehome_backed_up_source(new_source, 5)
        .expect("inline source provenance rehomes");
    let rehomed = payload
        .backed_up_get(0)
        .expect("singleton remains packed")
        .source_provenance
        .expect("source provenance remains present");
    assert_eq!(rehomed.range(), crate::SourceRange::new(new_source, 15, 17));
    assert_eq!(
        rehomed.location(),
        crate::SourceLocation::new(new_source, 16)
    );
}

#[test]
fn the_dense_level_enum_has_only_source_and_token_variants() {
    fn classify(level: &InputLevel) -> &'static str {
        match level {
            InputLevel::Source(_) => "source",
            InputLevel::Tokens(_) => "tokens",
        }
    }

    let level = InputLevel::Tokens(TokenCursor {
        payload: TokenPayload::transient(Vec::<TracedTokenWord>::new()),
        behavior: TokenBehavior::Ordinary,
        retirement: RetirementBehavior::Pop,
        trace: ReplayTrace::Inserted,
        frame: packed_token_frame(
            InputLevelId(0),
            0,
            &TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            &ReplayTrace::Inserted,
        ),
    });
    assert_eq!(classify(&level), "tokens");
}

#[test]
fn stored_token_encoding_partitions_characters_controls_and_macro_forms() {
    let mut universe = tex_state::Universe::new();
    let control = universe.intern("control").symbol();
    let cases = [
        Token::Char {
            ch: 'A',
            cat: Catcode::Letter,
        },
        Token::Char {
            ch: ' ',
            cat: Catcode::Space,
        },
        Token::Cs(control),
        Token::param(1),
        Token::param(9),
    ];

    for (index, token) in cases.into_iter().enumerate() {
        let traced = TracedTokenWord::pack(token, OriginId::UNKNOWN);
        assert_eq!(traced.semantic_token(), token, "case {index}");
        assert_eq!(traced.origin(), OriginId::UNKNOWN, "case {index}");
    }
    assert_eq!(core::mem::size_of::<Token>(), 8);
    assert_ne!(cases[0], cases[1]);
    assert_ne!(cases[2], cases[3]);
    assert_ne!(cases[3], cases[4]);
}

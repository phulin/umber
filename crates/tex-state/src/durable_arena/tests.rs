use crate::generation::with_generation;
use crate::glue::GlueSpec;
use crate::provenance::{OriginRecord, SyntheticOrigin, SyntheticOriginKind};
use crate::token::{Token, TokenWord};

#[test]
fn typed_arenas_publish_and_resolve_distinct_rows() {
    with_generation(|mut generation| {
        let words = [TokenWord::pack(Token::frozen_relax())];
        let first_tokens = generation.token_lists_mut().allocate(&words).unwrap();
        let second_tokens = generation.token_lists_mut().allocate(&words).unwrap();
        let first_glue = generation.glue_mut().allocate(GlueSpec::ZERO).unwrap();
        let second_glue = generation.glue_mut().allocate(GlueSpec::ZERO).unwrap();
        let provenance = OriginRecord::Synthetic(SyntheticOrigin::new(SyntheticOriginKind::Test));
        let first_origin = generation.provenance_mut().allocate(provenance).unwrap();
        let second_origin = generation.provenance_mut().allocate(provenance).unwrap();

        assert_ne!(first_tokens, second_tokens);
        assert_ne!(first_glue, second_glue);
        assert_ne!(first_origin, second_origin);
        assert_eq!(generation.token_lists().get(first_tokens), words);
        assert_eq!(generation.glue().get(first_glue), GlueSpec::ZERO);
        assert_eq!(generation.provenance().get(first_origin), provenance);
    });
}

#[test]
fn definition_words_and_durable_lists_have_separate_storage() {
    with_generation(|mut generation| {
        let word = TokenWord::pack(Token::frozen_relax());
        let definition = generation.definitions_mut().allocate(&[], &[word]).unwrap();
        let list = generation.token_lists_mut().allocate(&[word]).unwrap();

        assert_eq!(
            generation.definitions().get(definition).replacement_text(),
            [word]
        );
        assert_eq!(generation.token_lists().get(list), [word]);
        assert_eq!(generation.definitions().len(), 1);
        assert_eq!(generation.token_lists().len(), 1);
    });
}

use crate::generation::with_generation;
use crate::token::{Catcode, Token, TokenWord};

#[test]
fn definition_handle_is_one_thin_non_atomic_owner() {
    assert_eq!(
        std::mem::size_of::<super::DefinitionId<()>>(),
        std::mem::size_of::<usize>()
    );
}

#[test]
fn complete_rows_resolve_by_direct_id() {
    with_generation(|mut generation| {
        let parameter = [
            TokenWord::pack(Token::Char {
                ch: '#',
                cat: Catcode::Parameter,
            }),
            TokenWord::pack(Token::param(1)),
            TokenWord::pack(Token::Char {
                ch: b'x'.into(),
                cat: Catcode::Letter,
            }),
        ];
        let replacement = [TokenWord::pack(Token::param(1))];
        let id = generation
            .definitions_mut()
            .allocate(&parameter, &replacement)
            .expect("test fixture is valid");

        let owners = id.semantic_owner_count();
        assert_eq!(id.parameter_text(), parameter);
        assert_eq!(id.replacement_text(), replacement);
        assert_eq!(id.semantic_owner_count(), owners);

        let view = generation.definitions().get(id);
        assert_eq!(view.parameter_text(), parameter);
        assert_eq!(view.replacement_text(), replacement);
        assert_eq!(view.parameter_pattern().parameter_count(), 1);
        assert_eq!(view.parameter_pattern().marker_index(0), Some(0));
    });
}

#[test]
fn equal_definitions_receive_distinct_ids() {
    with_generation(|mut generation| {
        let text = [TokenWord::pack(Token::frozen_relax())];
        let first = generation
            .definitions_mut()
            .allocate(&[], &text)
            .expect("test fixture is valid");
        let second = generation
            .definitions_mut()
            .allocate(&[], &text)
            .expect("test fixture is valid");

        assert_ne!(first, second);
        assert_eq!(generation.definitions().len(), 2);
        assert_eq!(generation.definitions().get(first).replacement_text(), text);
        assert_eq!(
            generation.definitions().get(second).replacement_text(),
            text
        );
    });
}

#[test]
fn definition_aliases_release_exactly_on_owner_drop() {
    with_generation(|mut generation| {
        let id = generation
            .definitions_mut()
            .allocate(&[], &[TokenWord::pack(Token::frozen_relax())])
            .expect("published definition");
        assert_eq!(id.semantic_owner_count(), 1);

        let alias = id.clone();
        assert_eq!(id.semantic_owner_count(), 2);
        let view = generation.definitions().get(alias);
        assert_eq!(id.semantic_owner_count(), 2);
        drop(view);
        assert_eq!(id.semantic_owner_count(), 1);
    });
}

#[test]
fn invalid_parameter_program_does_not_publish_a_partial_row() {
    with_generation(|mut generation| {
        let too_many = [TokenWord::pack(Token::param(1)); 10];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generation.definitions_mut().allocate(&too_many, &[])
        }));

        assert!(result.is_err());
        assert!(generation.definitions().is_empty());

        let valid = generation
            .definitions_mut()
            .allocate(&[], &[])
            .expect("test fixture is valid");
        assert!(
            generation
                .definitions()
                .get(valid)
                .replacement_text()
                .is_empty()
        );
    });
}

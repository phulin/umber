use super::{JournalEntry, Mutation, MutationKind, SaveJournal, canonical_restore_words};
use crate::env::{StateCell, StateWord};

enum TestGeneration {}

#[test]
fn cursor_is_an_exact_position_in_ordered_history() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let start = journal.cursor();
    journal.push(JournalEntry::Mutation(Mutation {
        cell: StateCell::Count(7),
        before: StateWord::Integer(1),
        before_level: 1,
        after: StateWord::Integer(2),
        after_level: 2,
        saved_at: Some(2),
        kind: MutationKind::Assignment,
    }));
    let end = journal.cursor();
    assert_ne!(start, end);
    assert!(journal.validate_cursor(start));
    assert!(journal.validate_cursor(end));
    journal.truncate(start);
    assert_eq!(journal.len(), 0);
}

#[test]
fn cursor_from_another_state_is_rejected_even_with_the_same_brand() {
    let first = SaveJournal::<TestGeneration>::new();
    let second = SaveJournal::<TestGeneration>::new();
    assert!(!second.validate_cursor(first.cursor()));
}

#[test]
fn null_token_parameter_uses_tex_restore_zero_word() {
    // TeX82 §§240/275: the typed fixed bank represents the canonical
    // level-zero null pointer at level one, but its save form remains the
    // one-word `restore_zero` record.
    assert_eq!(
        canonical_restore_words(&Mutation::<TestGeneration> {
            cell: StateCell::TokenParameter(0),
            before: StateWord::TokenList(None),
            before_level: 1,
            after: StateWord::TokenList(None),
            after_level: 2,
            saved_at: Some(2),
            kind: MutationKind::Assignment,
        }),
        Some(1)
    );
}

use super::cell::JournalCell;
use super::{JournalEntry, Mutation, SaveJournal, canonical_restore_words};
use crate::env::{CodeTableKind, FontRuntimeCell, StateCell, StateWord};

enum TestGeneration {}

#[test]
fn reports_journal_component_widths() {
    eprintln!(
        "journal widths: entry={} mutation={} state_word={} state_cell={} group_frame={} meaning={}",
        core::mem::size_of::<JournalEntry<TestGeneration>>(),
        core::mem::size_of::<Mutation<TestGeneration>>(),
        core::mem::size_of::<StateWord<TestGeneration>>(),
        core::mem::size_of::<crate::env::StateCell>(),
        core::mem::size_of::<crate::env::group::GroupFrame>(),
        core::mem::size_of::<crate::meaning::MeaningWord<TestGeneration>>(),
    );
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(core::mem::size_of::<Mutation<TestGeneration>>(), 56);
        assert_eq!(core::mem::size_of::<JournalEntry<TestGeneration>>(), 64);
    }
}

#[test]
fn packed_cells_round_trip_every_coordinate_family_at_accepted_bounds() {
    let cells = [
        StateCell::Meaning(u32::MAX),
        StateCell::Count(u16::MAX),
        StateCell::Dimension(u16::MAX),
        StateCell::TokenRegister(u16::MAX),
        StateCell::GlueRegister(u16::MAX),
        StateCell::BoxRegister(u16::MAX),
        StateCell::MuGlueRegister(u16::MAX),
        StateCell::IntegerParameter(u16::MAX),
        StateCell::DimensionParameter(u16::MAX),
        StateCell::TokenParameter(u16::MAX),
        StateCell::GlueParameter(u16::MAX),
        StateCell::CurrentFont,
        StateCell::MathFamilyFont(u8::MAX),
        StateCell::Code(CodeTableKind::Catcode, u32::MAX),
        StateCell::Code(CodeTableKind::Lccode, u32::MAX),
        StateCell::Code(CodeTableKind::Uccode, u32::MAX),
        StateCell::Code(CodeTableKind::Sfcode, u32::MAX),
        StateCell::Code(CodeTableKind::Mathcode, u32::MAX),
        StateCell::Code(CodeTableKind::Delcode, u32::MAX),
        StateCell::FontRuntime(FontRuntimeCell::ParameterCount(
            crate::font::MAX_FONT_DIMEN_FONT_ID,
        )),
        StateCell::FontRuntime(FontRuntimeCell::Dimen {
            font: crate::font::MAX_FONT_DIMEN_FONT_ID,
            number: crate::font::MAX_FONT_DIMEN,
        }),
        StateCell::FontRuntime(FontRuntimeCell::HyphenChar(
            crate::font::MAX_FONT_DIMEN_FONT_ID,
        )),
        StateCell::FontRuntime(FontRuntimeCell::SkewChar(
            crate::font::MAX_FONT_DIMEN_FONT_ID,
        )),
        StateCell::FontRuntime(FontRuntimeCell::PdfCode {
            table: 8,
            font: crate::font::MAX_FONT_DIMEN_FONT_ID,
            code: u8::MAX,
        }),
        StateCell::FontRuntime(FontRuntimeCell::LigaturesDisabled(
            crate::font::MAX_FONT_DIMEN_FONT_ID,
        )),
    ];
    for cell in cells {
        assert_eq!(JournalCell::pack(cell).unpack(), cell);
    }
}

#[test]
fn cursor_is_an_exact_position_in_ordered_history() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let start = journal.checkpoint_cursor(0);
    journal.record_mutation(Mutation::new(
        StateCell::Count(7),
        StateWord::Integer(1),
        1,
        None,
    ));
    let end = journal.checkpoint_cursor(0);
    assert_ne!(start, end);
    assert!(journal.validate_cursor(start));
    assert!(journal.validate_cursor(end));
    journal.truncate_checkpoint(start);
    assert_eq!(journal.retained_len(), 0);
}

#[test]
fn cursor_from_another_state_is_rejected_even_with_the_same_brand() {
    let mut first = SaveJournal::<TestGeneration>::new();
    let second = SaveJournal::<TestGeneration>::new();
    assert!(!second.validate_cursor(first.checkpoint_cursor(0)));
}

#[test]
fn released_dense_prefix_invalidates_older_marks_and_reuses_pool_pages() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let root = journal.checkpoint_cursor(0);
    let released_records = journal.checkpoint_pool.chunk_capacity().saturating_mul(16);
    for index in 0..released_records {
        journal.record_mutation(Mutation::new(
            StateCell::Count(index as u16),
            StateWord::Integer(index as i32),
            1,
            None,
        ));
    }
    let floor = journal.checkpoint_cursor(0);
    journal.record_mutation(Mutation::new(
        StateCell::Count(released_records as u16),
        StateWord::Integer(released_records as i32),
        1,
        None,
    ));
    let accepted = journal.checkpoint_cursor(0);
    let pages = journal.checkpoint_pool.page_count();

    assert!(
        journal
            .release_checkpoint_prefix(floor)
            .expect("dense prefix releases")
            >= 16,
        "the complete first payload page is returned to the pool"
    );
    assert!(!journal.validate_cursor(root));
    assert!(journal.validate_cursor(floor));
    assert!(journal.validate_cursor(accepted));

    for index in released_records + 1..released_records.saturating_mul(2) {
        journal.record_mutation(Mutation::new(
            StateCell::Count(index as u16),
            StateWord::Integer(index as i32),
            1,
            None,
        ));
    }
    assert_eq!(
        journal.checkpoint_pool.page_count(),
        pages,
        "released dense chunks satisfy the next accepted suffix"
    );
    journal.truncate_checkpoint(floor);
    assert!(journal.validate_cursor(floor));
}

#[test]
fn checkpoint_intervals_deduplicate_first_before_but_operations_keep_exact_order() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let _start = journal.checkpoint_cursor(0);
    let operation = journal.begin_operation();
    for before in [1, 2] {
        journal.record_mutation(Mutation::new(
            StateCell::Count(7),
            StateWord::Integer(before),
            1,
            None,
        ));
    }
    assert_eq!(journal.checkpoint_entries, 1);
    let cursor = journal.checkpoint_cursor(0);
    let mut checkpoint_values = Vec::new();
    journal.visit_checkpoint_prefix(cursor, |delta| {
        checkpoint_values.push((delta.cell, delta.alternate.clone(), delta.alternate_level));
    });
    assert_eq!(checkpoint_values.len(), 1);
    assert_eq!(checkpoint_values[0].0, StateCell::Count(7));
    assert!(matches!(checkpoint_values[0].1, StateWord::Integer(1)));
    assert_eq!(checkpoint_values[0].2, 1);
    assert_eq!(journal.operation_entries.len(), 2);
    assert!(journal.active_groups.is_empty());

    journal.commit_operation(operation);
    assert!(journal.operation_entries.is_empty());
    assert!(journal.operation_entries.capacity() >= 2);
    let _interval = journal.checkpoint_cursor(0);
    journal.record_mutation(Mutation::new(
        StateCell::Count(7),
        StateWord::Integer(3),
        1,
        None,
    ));
    assert_eq!(journal.checkpoint_entries, 2);
}

#[test]
fn nested_operations_share_one_ordered_lane_and_rollback_only_the_inner_suffix() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let outer = journal.begin_operation();
    journal.record_mutation(Mutation::new(
        StateCell::Count(7),
        StateWord::Integer(1),
        1,
        None,
    ));
    let inner = journal.begin_operation();
    journal.record_mutation(Mutation::new(
        StateCell::Count(8),
        StateWord::Integer(2),
        1,
        None,
    ));
    assert_eq!(journal.operation_suffix(&inner).len(), 1);
    journal.finish_operation_rollback(inner);
    assert_eq!(journal.operation_suffix(&outer).len(), 1);
    journal.commit_operation(outer);
    assert!(journal.operation_entries.is_empty());
}

#[test]
fn null_token_parameter_uses_tex_restore_zero_word() {
    // TeX82 §§240/275: the typed fixed bank represents the canonical
    // level-zero null pointer at level one, but its save form remains the
    // one-word `restore_zero` record.
    assert_eq!(
        canonical_restore_words(&Mutation::<TestGeneration>::new(
            StateCell::TokenParameter(0),
            StateWord::TokenList(None),
            1,
            Some(2),
        )),
        Some(1)
    );
}

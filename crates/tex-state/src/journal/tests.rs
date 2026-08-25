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
    assert_eq!(journal.checkpoint_entries.len(), 1);
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
    assert_eq!(journal.checkpoint_entries.len(), 2);
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

use super::cell::JournalCell;
use super::{JournalEntry, Mutation, SaveJournal, canonical_restore_words};
use crate::env::group::{GroupFrame, GroupKind};
use crate::env::{CodeTableKind, FontRuntimeCell, StateCell, StateWord};

enum TestGeneration {}

fn assert_exact_capacity_accounting(journal: &SaveJournal<TestGeneration>) {
    assert_eq!(
        journal.retained_bytes(),
        journal.retained_bytes_census(),
        "constant-time projection must equal the test-only physical census"
    );
}

fn enter_group(journal: &mut SaveJournal<TestGeneration>, lineage: u64) -> GroupFrame {
    let (save_stack_words_before, latest_save_push_before) = journal.save_stack_projection();
    let frame = GroupFrame::for_journal_test(
        GroupKind::Simple,
        lineage,
        u32::try_from(journal.active_groups.len()).expect("test depth fits u32") + 2,
        save_stack_words_before,
        latest_save_push_before,
    );
    journal.record_group_enter(frame);
    frame
}

fn record_saved_count(journal: &mut SaveJournal<TestGeneration>, cell: u16, before: i32) {
    let level = journal
        .active_groups
        .last()
        .expect("saved mutation has an active group")
        .frame
        .level();
    journal.record_mutation(Mutation::new(
        StateCell::Count(cell),
        StateWord::Integer(before),
        level,
        0,
        Some(level),
    ));
}

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
        assert_eq!(core::mem::size_of::<Mutation<TestGeneration>>(), 64);
        assert_eq!(core::mem::size_of::<JournalEntry<TestGeneration>>(), 64);
    }
}

#[test]
fn dense_state_cell_coordinates_do_not_implement_hash() {
    trait AmbiguousIfHash<Marker> {
        fn marker() {}
    }
    impl<T: ?Sized> AmbiguousIfHash<()> for T {}
    struct Invalid;
    impl<T: ?Sized + core::hash::Hash> AmbiguousIfHash<Invalid> for T {}

    let _ = <StateCell as AmbiguousIfHash<_>>::marker;
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
        0,
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
            0,
            None,
        ));
    }
    let floor = journal.checkpoint_cursor(0);
    journal.record_mutation(Mutation::new(
        StateCell::Count(released_records as u16),
        StateWord::Integer(released_records as i32),
        1,
        0,
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
    assert_exact_capacity_accounting(&journal);

    for index in released_records + 1..released_records.saturating_mul(2) {
        journal.record_mutation(Mutation::new(
            StateCell::Count(index as u16),
            StateWord::Integer(index as i32),
            1,
            0,
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
    assert_exact_capacity_accounting(&journal);
}

#[test]
fn checkpoint_intervals_deduplicate_the_first_prior_value() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    let _start = journal.checkpoint_cursor(0);
    let interval_serial = journal.save_serial();
    for (before, before_save_serial) in [(1, 0), (2, interval_serial)] {
        journal.record_mutation(Mutation::new(
            StateCell::Count(7),
            StateWord::Integer(before),
            1,
            before_save_serial,
            None,
        ));
    }
    assert_eq!(journal.checkpoint_entries, 1);
    let cursor = journal.checkpoint_cursor(0);
    let mut checkpoint_values = Vec::new();
    journal.visit_checkpoint_prefix(cursor, |delta| {
        checkpoint_values.push((
            delta.cell,
            delta.alternate.clone(),
            delta.alternate_level,
            delta.alternate_save_serial,
        ));
    });
    assert_eq!(checkpoint_values.len(), 1);
    assert_eq!(checkpoint_values[0].0, StateCell::Count(7));
    assert!(matches!(checkpoint_values[0].1, StateWord::Integer(1)));
    assert_eq!(checkpoint_values[0].2, 1);
    assert_eq!(checkpoint_values[0].3, 0);
    assert!(journal.active_groups.is_empty());
    let _interval = journal.checkpoint_cursor(0);
    journal.record_mutation(Mutation::new(
        StateCell::Count(7),
        StateWord::Integer(3),
        1,
        interval_serial,
        None,
    ));
    assert_eq!(journal.checkpoint_entries, 2);
    assert_exact_capacity_accounting(&journal);
}

#[test]
fn exact_capacity_accounting_covers_group_reuse_and_checkpoint_settlement() {
    let mut journal = SaveJournal::<TestGeneration>::new();
    assert_exact_capacity_accounting(&journal);

    let outer = enter_group(&mut journal, 1);
    for cell in 0..64 {
        record_saved_count(&mut journal, cell, i32::from(cell));
    }
    assert_exact_capacity_accounting(&journal);

    let _inner = enter_group(&mut journal, 2);
    for cell in 64..128 {
        record_saved_count(&mut journal, cell, i32::from(cell));
    }
    assert_exact_capacity_accounting(&journal);
    journal.record_group_exit(_inner);
    let inner = enter_group(&mut journal, 3);
    record_saved_count(&mut journal, 128, 128);
    journal.record_group_exit(inner);
    assert_exact_capacity_accounting(&journal);

    journal.record_group_exit(outer);
    assert_exact_capacity_accounting(&journal);
    let _reused = enter_group(&mut journal, 4);
    assert_exact_capacity_accounting(&journal);

    let outer_cursor = journal.checkpoint_cursor(1);
    let retained = enter_group(&mut journal, 5);
    let retained_cursor = journal.checkpoint_cursor(2);
    journal.record_group_exit(retained);
    assert_exact_capacity_accounting(&journal);
    let _ = journal.restore_group_cursor(outer_cursor);
    assert_exact_capacity_accounting(&journal);
    let _ = journal.restore_group_cursor(retained_cursor);
    assert_exact_capacity_accounting(&journal);
}

#[test]
fn exact_capacity_accounting_covers_checkpoint_fork_accept_reject_and_release() {
    let mut root_journal = SaveJournal::<TestGeneration>::new();
    let root = root_journal.checkpoint_cursor(0);
    root_journal.record_mutation(Mutation::new(
        StateCell::Count(0),
        StateWord::Integer(0),
        1,
        0,
        None,
    ));
    let tail = root_journal.begin_checkpoint_candidate(root);
    assert!(tail.is_root_candidate());
    root_journal.record_mutation(Mutation::new(
        StateCell::Count(1),
        StateWord::Integer(1),
        1,
        0,
        None,
    ));
    root_journal.reject_checkpoint_candidate(tail);
    assert_exact_capacity_accounting(&root_journal);

    let mut journal = SaveJournal::<TestGeneration>::new();
    let outer = enter_group(&mut journal, 1);
    for cell in 0..64 {
        record_saved_count(&mut journal, cell, i32::from(cell));
    }
    let selected = journal.checkpoint_cursor(1);
    let inner = enter_group(&mut journal, 2);
    for cell in 64..192 {
        record_saved_count(&mut journal, cell, i32::from(cell));
    }
    journal.record_group_exit(inner);
    assert_exact_capacity_accounting(&journal);

    let tail = journal.begin_checkpoint_candidate(selected);
    assert_exact_capacity_accounting(&journal);
    for cell in 192..256 {
        record_saved_count(&mut journal, cell, i32::from(cell));
    }
    journal.reject_checkpoint_candidate(tail);
    assert_exact_capacity_accounting(&journal);

    let tail = journal.begin_checkpoint_candidate(selected);
    assert!(matches!(
        tail.groups,
        super::AcceptedGroupTail::Arbitrary { .. }
    ));
    record_saved_count(&mut journal, 256, 256);
    journal.accept_checkpoint_candidate();
    drop(tail);
    assert_exact_capacity_accounting(&journal);

    journal.record_group_exit(outer);
    let root = journal.checkpoint_cursor(0);
    for cell in 512..768 {
        journal.record_mutation(Mutation::new(
            StateCell::Count(cell),
            StateWord::Integer(i32::from(cell)),
            1,
            0,
            None,
        ));
    }
    let floor = journal.checkpoint_cursor(0);
    journal
        .release_checkpoint_prefix(floor)
        .expect("checkpoint prefix releases");
    assert!(!journal.validate_cursor(root));
    assert_exact_capacity_accounting(&journal);
}

#[test]
fn retained_byte_projection_has_fixed_work_with_many_group_segments() {
    let mut shallow = SaveJournal::<TestGeneration>::new();
    let _ = enter_group(&mut shallow, 1);
    record_saved_count(&mut shallow, 0, 0);

    let mut deep = SaveJournal::<TestGeneration>::new();
    for lineage in 1..=4_096 {
        let _ = enter_group(&mut deep, lineage);
        record_saved_count(
            &mut deep,
            u16::try_from(lineage).expect("test lineage fits u16"),
            i32::try_from(lineage).expect("test lineage fits i32"),
        );
    }
    assert_exact_capacity_accounting(&shallow);
    assert_exact_capacity_accounting(&deep);

    for _ in 0..4_096 {
        std::hint::black_box(shallow.retained_bytes());
        std::hint::black_box(deep.retained_bytes());
    }
    assert_eq!(
        shallow.retained_bytes(),
        shallow
            .group_capacity_bytes
            .saturating_add(shallow.checkpoint_capacity_bytes)
    );
    assert_eq!(
        deep.retained_bytes(),
        deep.group_capacity_bytes
            .saturating_add(deep.checkpoint_capacity_bytes)
    );
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
            0,
            Some(2),
        )),
        Some(1)
    );
}

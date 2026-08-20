use super::{AssignmentScope, CodeTableKind, DenseState};
use crate::env::group::GroupKind;
use crate::interner::{Interner, InternerBudget};
use crate::journal::{JournalEntry, MutationKind};
use crate::meaning::{Meaning, MeaningWord, ResolvedMeaning};
use crate::scaled::Scaled;

enum TestGeneration {}

fn state() -> DenseState<TestGeneration> {
    DenseState::new().expect("state allocation")
}

fn interner() -> Interner {
    Interner::new(InternerBudget::new(64, 64, 1024).expect("budget"))
}

#[test]
fn admitted_meanings_are_direct_dense_slots() {
    let mut names = interner();
    let alpha = names.intern("alpha").expect("intern");
    let beta = names.intern("beta").expect("intern");
    let mut state = state();
    state.admit_symbol(alpha.symbol()).expect("admit alpha");
    state.admit_symbol(beta.symbol()).expect("admit beta");

    assert_eq!(
        state.meaning(alpha.symbol()).expect("read"),
        ResolvedMeaning::Static(Meaning::Undefined)
    );
    state
        .assign_meaning(
            beta.symbol(),
            MeaningWord::from_static(Meaning::Relax),
            AssignmentScope::Global,
        )
        .expect("assign");
    assert_eq!(
        state.meaning(beta.symbol()).expect("read"),
        ResolvedMeaning::Static(Meaning::Relax)
    );
}

#[test]
fn register_overflow_is_page_index_dense() {
    let mut state = state();
    assert_eq!(state.allocated_overflow_pages(), 0);
    state
        .assign_count(40_000, 17, AssignmentScope::Global)
        .expect("assign overflow");
    assert_eq!(state.count(40_000).expect("read overflow"), 17);
    assert_eq!(state.count(39_999).expect("read adjacent"), 0);
    assert_eq!(state.allocated_overflow_pages(), 1);
}

#[test]
fn repeated_local_writes_restore_the_first_prior_value() {
    let mut state = state();
    state
        .assign_count(0, 3, AssignmentScope::Global)
        .expect("base");
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_count(0, 4, AssignmentScope::Local)
        .expect("first local");
    state
        .assign_count(0, 5, AssignmentScope::Local)
        .expect("second local");
    state.end_group(GroupKind::Simple).expect("end group");
    assert_eq!(state.count(0).expect("read"), 3);
}

#[test]
fn ordered_journal_carries_each_exact_prior_word_and_only_the_tex_save() {
    let mut state = state();
    state
        .assign_count(0, 3, AssignmentScope::Global)
        .expect("base");
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_count(0, 4, AssignmentScope::Local)
        .expect("first local");
    state
        .assign_count(0, 5, AssignmentScope::Local)
        .expect("second local");

    let JournalEntry::Mutation(first) = state.journal.entry(2) else {
        panic!("expected first local mutation");
    };
    let JournalEntry::Mutation(second) = state.journal.entry(3) else {
        panic!("expected second local mutation");
    };
    assert_eq!(first.kind, MutationKind::Assignment);
    assert_eq!(first.before, super::StateWord::Integer(3));
    assert_eq!(first.after, super::StateWord::Integer(4));
    assert_eq!(first.saved_at, Some(2));
    assert_eq!(second.before, super::StateWord::Integer(4));
    assert_eq!(second.after, super::StateWord::Integer(5));
    assert_eq!(second.saved_at, None);
}

#[test]
fn later_global_assignment_suppresses_every_applicable_restore() {
    let mut state = state();
    state
        .assign_count(0, 1, AssignmentScope::Global)
        .expect("base");
    state.begin_group(GroupKind::Simple, 1).expect("outer");
    state
        .assign_count(0, 2, AssignmentScope::Local)
        .expect("outer local");
    state.begin_group(GroupKind::SemiSimple, 2).expect("inner");
    state
        .assign_count(0, 3, AssignmentScope::Local)
        .expect("inner local");
    state
        .assign_count(0, 4, AssignmentScope::Global)
        .expect("global");
    state.end_group(GroupKind::SemiSimple).expect("inner end");
    state.end_group(GroupKind::Simple).expect("outer end");
    assert_eq!(state.count(0).expect("read"), 4);
}

#[test]
fn local_after_global_restores_the_global_value() {
    let mut state = state();
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_count(0, 10, AssignmentScope::Local)
        .expect("local");
    state
        .assign_count(0, 20, AssignmentScope::Global)
        .expect("global");
    state
        .assign_count(0, 30, AssignmentScope::Local)
        .expect("second local");
    state.end_group(GroupKind::Simple).expect("end");
    assert_eq!(state.count(0).expect("read"), 20);
}

#[test]
fn journal_cursor_restores_group_exit_and_assignment_exactly() {
    let mut state = state();
    state
        .assign_dimension(0, Scaled::from_raw(5), AssignmentScope::Global)
        .expect("base");
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_dimension(0, Scaled::from_raw(7), AssignmentScope::Local)
        .expect("local");
    let inside = state.journal_cursor();
    state.end_group(GroupKind::Simple).expect("end");
    assert_eq!(state.dimension(0).expect("read"), Scaled::from_raw(5));

    state.restore(inside).expect("restore inside group");
    assert_eq!(state.group_depth(), 1);
    assert_eq!(state.dimension(0).expect("read"), Scaled::from_raw(7));
}

#[test]
fn rollback_to_pre_group_cursor_removes_group_and_writes() {
    let mut state = state();
    let before = state.journal_cursor();
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_count(0, 9, AssignmentScope::Local)
        .expect("local");
    state.restore(before).expect("restore");
    assert_eq!(state.group_depth(), 0);
    assert_eq!(state.count(0).expect("read"), 0);
}

#[test]
fn code_tables_use_initex_defaults_and_the_same_save_journal() {
    let mut state = state();
    assert_eq!(state.code(CodeTableKind::Catcode, '\\').expect("cat"), 0);
    assert_eq!(state.code(CodeTableKind::Catcode, 'A').expect("cat"), 11);
    assert_eq!(state.code(CodeTableKind::Delcode, '.').expect("del"), 0);
    assert_eq!(state.code(CodeTableKind::Delcode, '(').expect("del"), -1);

    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_code(CodeTableKind::Catcode, 'A', 12, AssignmentScope::Local)
        .expect("assign");
    assert_eq!(state.code(CodeTableKind::Catcode, 'A').expect("cat"), 12);
    state.end_group(GroupKind::Simple).expect("end");
    assert_eq!(state.code(CodeTableKind::Catcode, 'A').expect("cat"), 11);
}

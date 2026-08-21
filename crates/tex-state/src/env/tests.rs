use super::{AssignmentScope, CodeTableKind, DenseState};
use crate::env::group::GroupKind;
use crate::font::PdfFontCode;
use crate::ids::FontId;
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

fn state_with_font(parameters: &[Scaled]) -> DenseState<TestGeneration> {
    let mut state = state();
    let prepared = state
        .prepare_font_runtime(parameters, 45, 7)
        .expect("prepare font runtime");
    state
        .install_font_runtime(FontId::new(0), prepared)
        .expect("install font runtime");
    state
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

#[test]
fn font_runtime_values_follow_nested_local_and_global_restore() {
    let font = FontId::new(0);
    let mut state = state_with_font(&[Scaled::from_raw(10), Scaled::from_raw(20)]);
    state.begin_group(GroupKind::Simple, 1).expect("outer");
    state
        .assign_font_dimen(font, 1, Scaled::from_raw(11), AssignmentScope::Local)
        .expect("local fontdimen");
    state
        .assign_font_hyphen_char(font, 46, AssignmentScope::Local)
        .expect("local hyphen char");
    state.begin_group(GroupKind::SemiSimple, 2).expect("inner");
    state
        .assign_font_dimen(font, 1, Scaled::from_raw(12), AssignmentScope::Global)
        .expect("global fontdimen");
    state
        .assign_font_skew_char(font, 8, AssignmentScope::Local)
        .expect("local skew char");
    state.end_group(GroupKind::SemiSimple).expect("inner end");
    assert_eq!(
        state.font_dimen(font, 1).expect("fontdimen"),
        Scaled::from_raw(12)
    );
    assert_eq!(state.font_skew_char(font).expect("skew char"), 7);
    state.end_group(GroupKind::Simple).expect("outer end");
    assert_eq!(
        state.font_dimen(font, 1).expect("fontdimen"),
        Scaled::from_raw(12)
    );
    assert_eq!(state.font_hyphen_char(font).expect("hyphen char"), 45);
}

#[test]
fn font_runtime_growth_pdf_codes_and_ligatures_rollback_exactly() {
    let font = FontId::new(0);
    let mut state = state_with_font(&[Scaled::from_raw(10)]);
    state
        .prepare_pdf_font_code_table(font, PdfFontCode::Ef, [1000; 256])
        .expect("prepare PDF code table");
    let before = state.journal_cursor();
    state.begin_group(GroupKind::Simple, 1).expect("group");
    state
        .assign_font_dimen(font, 3, Scaled::from_raw(30), AssignmentScope::Local)
        .expect("grow fontdimen");
    state
        .assign_pdf_font_code(font, PdfFontCode::Ef, b'A', 750, AssignmentScope::Local)
        .expect("local expansion factor");
    state
        .assign_pdf_font_ligatures_disabled(font, true, AssignmentScope::Local)
        .expect("disable ligatures");
    assert_eq!(state.font_parameter_count(font).expect("count"), 3);
    assert_eq!(
        state
            .pdf_font_code(font, PdfFontCode::Ef, b'A')
            .expect("code"),
        750
    );
    assert!(state.pdf_font_ligatures_disabled(font).expect("ligatures"));

    state.restore(before).expect("restore checkpoint");
    assert_eq!(state.font_parameter_count(font).expect("count"), 1);
    assert_eq!(
        state
            .pdf_font_code(font, PdfFontCode::Ef, b'A')
            .expect("code"),
        1000
    );
    assert!(!state.pdf_font_ligatures_disabled(font).expect("ligatures"));
}

#[test]
fn foreign_checkpoint_rejection_does_not_mutate_font_runtime() {
    let font = FontId::new(0);
    let foreign = state().journal_cursor();
    let mut state = state_with_font(&[Scaled::from_raw(10)]);
    state
        .assign_font_hyphen_char(font, 99, AssignmentScope::Global)
        .expect("set hyphen char");
    let journal_len = state.journal_len();

    assert!(state.restore(foreign).is_err());
    assert_eq!(state.font_hyphen_char(font).expect("hyphen char"), 99);
    assert_eq!(state.journal_len(), journal_len);
}

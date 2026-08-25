use super::{
    AssignmentScope, CodeTableKind, DenseState, FreshParameterDefault, FreshParameterInstallError,
    FreshParameterInstallation, FreshParameterProfile, GroupRestorationCell,
    GroupRestorationOutcome, GroupRestorationValue,
};
use crate::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use crate::env::group::GroupKind;
use crate::font::PdfFontCode;
use crate::ids::FontId;
use crate::interner::{Interner, InternerBudget};
use crate::journal::{JournalEntry, MutationKind};
use crate::meaning::{Meaning, MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::scaled::Scaled;
use crate::token::{Token, TokenWord};

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
fn checkpoint_rollback_releases_macro_and_token_list_carriers() {
    crate::generation::with_generation(|mut generation| {
        let definition = generation
            .definitions_mut()
            .allocate(&[], &[TokenWord::pack(Token::frozen_relax())])
            .expect("definition");
        let tokens = generation
            .token_lists_mut()
            .allocate(&[TokenWord::pack(Token::frozen_relax())])
            .expect("token list");
        let mut names = interner();
        let selector = names.intern("owned").expect("selector");
        let mut state = DenseState::new().expect("state allocation");
        state
            .admit_symbol(selector.symbol())
            .expect("admit selector");
        let before = state.journal_cursor();

        state
            .assign_meaning(
                selector.symbol(),
                MeaningWord::macro_definition(MeaningFlags::from_bits(0), definition.clone()),
                AssignmentScope::Global,
            )
            .expect("assign macro");
        state
            .assign_token_register(0, Some(tokens.clone()), AssignmentScope::Global)
            .expect("assign token list");
        assert_eq!(definition.semantic_owner_count(), 3);
        assert_eq!(tokens.semantic_owner_count(), 3);

        state.restore(before).expect("rollback");
        assert_eq!(definition.semantic_owner_count(), 1);
        assert_eq!(tokens.semantic_owner_count(), 1);
    });
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
fn checked_save_stack_projection_tracks_restore_forms_and_rollback() {
    // TeX82 §§273/275--276 samples before each boundary, restore, and
    // aftergroup push. The command owner supplies aftergroup words and their
    // state-journal-relative ordering; no token payload enters this state
    // projection.
    let mut names = interner();
    let fresh = names.intern("fresh").expect("intern fresh meaning");
    let mut state = state();
    state.admit_symbol(fresh.symbol()).expect("admit meaning");
    state.begin_group(GroupKind::Simple, 1).expect("group");
    assert_eq!(state.checked_save_stack_words(0, None, false), 0);

    state
        .assign_meaning(
            fresh.symbol(),
            MeaningWord::from_static(Meaning::Relax),
            AssignmentScope::Local,
        )
        .expect("level-zero meaning save");
    assert_eq!(state.checked_save_stack_words(0, None, false), 1);

    state
        .assign_count(0, 1, AssignmentScope::Local)
        .expect("two-word count save");
    assert_eq!(state.checked_save_stack_words(0, None, false), 2);

    let aftergroup_position = state.save_stack_order_position();
    assert_eq!(
        state.checked_save_stack_words(1, Some(aftergroup_position), false),
        4,
        "the command-owned one-word push is newer than the tied state record"
    );
    let checkpoint = state.journal_cursor();

    state
        .assign_dimension(0, Scaled::from_raw(1), AssignmentScope::Local)
        .expect("later two-word restore");
    assert_eq!(
        state.checked_save_stack_words(1, Some(aftergroup_position), false),
        5,
        "the later state record supersedes aftergroup ordering"
    );
    state.restore(checkpoint).expect("projection rollback");
    assert_eq!(
        state.checked_save_stack_words(1, Some(aftergroup_position), false),
        4
    );

    state.end_group(GroupKind::Simple).expect("group closes");
    assert_eq!(state.checked_save_stack_words(0, None, false), 0);
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
fn restoration_receipts_preserve_nested_reverse_order_and_exact_font_outcomes() {
    let mut state = state();
    let outer_font = FontId::testing_new(7);
    let inner_font = FontId::testing_new(8);
    let outer_retained_font = FontId::testing_new(10);
    let inner_retained_font = FontId::testing_new(11);
    let global_font = FontId::testing_new(12);
    state
        .assign_count(0, 1, AssignmentScope::Global)
        .expect("base count");

    state.begin_group(GroupKind::Simple, 1).expect("outer");
    state
        .assign_count(0, 2, AssignmentScope::Local)
        .expect("outer count");
    state
        .assign_math_family_font(18, outer_font, AssignmentScope::Local)
        .expect("outer restored font");
    state
        .assign_math_family_font(19, outer_retained_font, AssignmentScope::Local)
        .expect("outer retained font");

    state.begin_group(GroupKind::SemiSimple, 2).expect("inner");
    state
        .assign_count(0, 3, AssignmentScope::Local)
        .expect("inner count");
    state
        .assign_math_family_font(18, inner_font, AssignmentScope::Local)
        .expect("inner restored font");
    state
        .assign_math_family_font(19, inner_retained_font, AssignmentScope::Local)
        .expect("inner retained font");
    state
        .assign_count(0, 4, AssignmentScope::Global)
        .expect("global count");
    state
        .assign_math_family_font(19, global_font, AssignmentScope::Global)
        .expect("global font");

    let inner = state.end_group(GroupKind::SemiSimple).expect("inner end");
    let inner_entries = inner.entries();
    assert_eq!(inner_entries.len(), 3);
    assert_eq!(
        inner_entries[0].cell(),
        GroupRestorationCell::MathFamilyFont(19)
    );
    assert_eq!(
        inner_entries[0].saved_value(),
        GroupRestorationValue::Font(outer_retained_font)
    );
    assert_eq!(
        inner_entries[0].live_value(),
        GroupRestorationValue::Font(global_font)
    );
    assert_eq!(
        inner_entries[0].outcome(),
        GroupRestorationOutcome::Retained
    );
    assert_eq!(
        inner_entries[1].cell(),
        GroupRestorationCell::MathFamilyFont(18)
    );
    assert_eq!(
        inner_entries[1].saved_value(),
        GroupRestorationValue::Font(outer_font)
    );
    assert_eq!(
        inner_entries[1].live_value(),
        GroupRestorationValue::Font(outer_font)
    );
    assert_eq!(
        inner_entries[1].outcome(),
        GroupRestorationOutcome::Restored
    );
    assert_eq!(inner_entries[2].cell(), GroupRestorationCell::Count(0));
    assert_eq!(
        inner_entries[2].saved_value(),
        GroupRestorationValue::Integer(2)
    );
    assert_eq!(
        inner_entries[2].live_value(),
        GroupRestorationValue::Integer(4)
    );
    assert_eq!(
        inner_entries[2].outcome(),
        GroupRestorationOutcome::Retained
    );

    let outer = state.end_group(GroupKind::Simple).expect("outer end");
    let outer_entries = outer.entries();
    assert_eq!(outer_entries.len(), 3);
    assert_eq!(
        outer_entries[0].cell(),
        GroupRestorationCell::MathFamilyFont(19)
    );
    assert_eq!(
        outer_entries[0].live_value(),
        GroupRestorationValue::Font(global_font)
    );
    assert_eq!(
        outer_entries[0].outcome(),
        GroupRestorationOutcome::Retained
    );
    assert_eq!(
        outer_entries[1].cell(),
        GroupRestorationCell::MathFamilyFont(18)
    );
    assert_eq!(
        outer_entries[1].saved_value(),
        GroupRestorationValue::Font(FontId::new(0))
    );
    assert_eq!(
        outer_entries[1].live_value(),
        GroupRestorationValue::Font(FontId::new(0))
    );
    assert_eq!(
        outer_entries[1].outcome(),
        GroupRestorationOutcome::Restored
    );
    assert_eq!(outer_entries[2].cell(), GroupRestorationCell::Count(0));
    assert_eq!(
        outer_entries[2].live_value(),
        GroupRestorationValue::Integer(4)
    );
    assert_eq!(
        outer_entries[2].outcome(),
        GroupRestorationOutcome::Retained
    );
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
fn fresh_profile_batch_installs_mixed_dense_cells_without_journal_history() {
    let mut state = state();
    let before = state.journal_len();
    assert_eq!(
        state.install_fresh_parameter_profile(
            FreshParameterProfile::Tex82,
            &[
                FreshParameterDefault::Integer(IntParam::MAG, 1_000),
                FreshParameterDefault::Dimension(DimenParam::H_OFFSET, Scaled::from_raw(17)),
                FreshParameterDefault::EmptyGlue(GlueParam::LEFT_SKIP),
                FreshParameterDefault::EmptyTokens(TokParam::EVERY_JOB),
            ],
        ),
        Ok(FreshParameterInstallation::Installed)
    );
    assert_eq!(state.integer_parameter(IntParam::MAG).expect("mag"), 1_000);
    assert_eq!(
        state
            .dimension_parameter(DimenParam::H_OFFSET)
            .expect("hoffset"),
        Scaled::from_raw(17)
    );
    assert_eq!(
        state.glue_parameter(GlueParam::LEFT_SKIP).expect("glue"),
        None
    );
    assert_eq!(
        state.token_parameter(TokParam::EVERY_JOB).expect("tokens"),
        None
    );
    assert_eq!(state.journal_len(), before);
}

#[test]
fn repeated_fresh_profile_installation_preserves_later_assignments() {
    let mut state = state();
    let defaults = [FreshParameterDefault::Integer(IntParam::MAG, 1_000)];
    assert_eq!(
        state.install_fresh_parameter_profile(FreshParameterProfile::Tex82, &defaults),
        Ok(FreshParameterInstallation::Installed)
    );
    state
        .assign_integer_parameter(IntParam::MAG, 1_200, AssignmentScope::Global)
        .expect("later format assignment");
    assert_eq!(
        state.install_fresh_parameter_profile(FreshParameterProfile::Tex82, &defaults),
        Ok(FreshParameterInstallation::AlreadyInstalled)
    );
    assert_eq!(state.integer_parameter(IntParam::MAG).expect("mag"), 1_200);
}

#[test]
fn invalid_fresh_profile_batches_are_mutation_free() {
    let mut state = state();
    let duplicate = [
        FreshParameterDefault::Integer(IntParam::MAG, 1_000),
        FreshParameterDefault::Integer(IntParam::MAG, 1_200),
    ];
    assert_eq!(
        state.install_fresh_parameter_profile(FreshParameterProfile::Tex82, &duplicate),
        Err(FreshParameterInstallError::DuplicateCell {
            bank: super::FreshParameterBank::Integer,
            index: IntParam::MAG.raw(),
        })
    );
    assert_eq!(state.integer_parameter(IntParam::MAG).expect("mag"), 0);
    assert_eq!(
        state.install_fresh_parameter_profile(FreshParameterProfile::Pdftex14029, &[]),
        Err(FreshParameterInstallError::MissingTex82Base(
            FreshParameterProfile::Pdftex14029
        ))
    );
    assert_eq!(state.journal_len(), 0);
}

#[test]
fn job_clock_refresh_changes_only_the_four_volatile_cells() {
    let mut state = state();
    state
        .assign_integer_parameter(IntParam::MAG, 1_234, AssignmentScope::Global)
        .expect("format mag");
    state.refresh_job_clock(crate::JobClock {
        time: 817,
        second: 9,
        day: 21,
        month: 8,
        year: 2026,
    });
    assert_eq!(state.integer_parameter(IntParam::TIME).expect("time"), 817);
    assert_eq!(state.integer_parameter(IntParam::DAY).expect("day"), 21);
    assert_eq!(state.integer_parameter(IntParam::MONTH).expect("month"), 8);
    assert_eq!(state.integer_parameter(IntParam::YEAR).expect("year"), 2026);
    assert_eq!(state.integer_parameter(IntParam::MAG).expect("mag"), 1_234);
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

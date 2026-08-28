use super::{DynamicMemoryScratch, StateCore};
use crate::env::AssignmentScope;
use crate::generation::with_generation;
use crate::glue::GlueSpec;
use crate::interner::{Interner, InternerBudget};
use crate::meaning::{MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::node::Node;
use crate::token::{Catcode, Token, TokenWord};

#[cfg(feature = "profiling")]
#[global_allocator]
static PROFILING_ALLOCATOR: crate::measurement::HotCoreAllocator =
    crate::measurement::HotCoreAllocator;

#[test]
fn admitted_view_resolves_every_generation_typed_value_directly() {
    with_generation(|generation| {
        let mut core = StateCore::new(generation).expect("state core");
        let replacement = [TokenWord::pack(Token::frozen_relax())];
        let (definition, tokens, glue) = {
            let mut admitted = core.admit_mut().expect("unique generation");
            let definition = admitted
                .allocate_definition(&[], &replacement)
                .expect("definition");
            let tokens = admitted
                .allocate_token_list(&replacement)
                .expect("token list");
            let glue = admitted.allocate_glue(GlueSpec::ZERO).expect("glue");
            (definition, tokens, glue)
        };

        let admitted = core.admit();
        assert_eq!(
            admitted.definition(definition).replacement_text(),
            replacement
        );
        assert_eq!(
            admitted.token_list(tokens).iter().collect::<Vec<_>>(),
            replacement
        );
        assert_eq!(admitted.glue(glue), GlueSpec::ZERO);
    });
}

#[test]
fn generation_ids_install_in_dense_state_without_per_value_owners() {
    with_generation(|generation| {
        let mut names = Interner::new(InternerBudget::new(8, 8, 128).expect("budget"));
        let symbol = names.intern("macro").expect("intern");
        let mut core = StateCore::new(generation).expect("state core");
        let definition = {
            let mut admitted = core.admit_mut().expect("unique generation");
            admitted
                .state()
                .admit_symbol(symbol.symbol())
                .expect("symbol admission");
            let definition = admitted.allocate_definition(&[], &[]).expect("definition");
            admitted
                .state()
                .assign_meaning(
                    symbol.symbol(),
                    MeaningWord::macro_definition(MeaningFlags::LONG, definition.clone()),
                    AssignmentScope::Global,
                )
                .expect("meaning assignment");
            definition
        };

        assert_eq!(
            core.admit()
                .state()
                .meaning(symbol.symbol())
                .expect("meaning"),
            ResolvedMeaning::Macro {
                flags: MeaningFlags::LONG,
                definition,
            }
        );
    });
}

#[test]
fn retirement_releases_one_complete_generation_bundle() {
    with_generation(|generation| {
        let mut core = StateCore::new(generation).expect("state core");
        {
            let mut admitted = core.admit_mut().expect("unique generation");
            admitted.allocate_definition(&[], &[]).expect("definition");
            admitted.allocate_token_list(&[]).expect("token list");
            admitted.allocate_glue(GlueSpec::ZERO).expect("glue");
            admitted
                .state()
                .assign_count(0, 7, AssignmentScope::Global)
                .expect("assignment");
        }
        let retired = core.retire().expect("unique generation");
        assert_eq!(retired.generation.definitions, 1);
        assert_eq!(retired.generation.token_lists, 1);
        assert_eq!(retired.generation.glue_values, 1);
        assert_eq!(retired.generation.provenance_records, 0);
        assert_eq!(retired.journal_entries, 1);
    });
}

#[test]
fn reusable_marks_scale_with_reachable_high_water_not_sparse_raw_ids() {
    let mut marks = crate::node_arena::StampedIndexMap::default();
    marks.begin();
    assert!(marks.mark(7));
    assert!(marks.mark(1_000_000_007));
    assert!(!marks.mark(7));
    assert_eq!(marks.len(), 2);
    assert_eq!(marks.capacity(), 16);

    marks.begin();
    assert!(marks.mark(usize::MAX - 1));
    assert_eq!(marks.len(), 1);
    assert_eq!(marks.capacity(), 16);
}

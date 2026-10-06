//! Actual semantic-root and physical-payload lifetime proofs.

use super::*;
use crate::env::AssignmentScope;
use crate::generation::{GenerationBrand, with_generation};
use crate::interner::{Interner, InternerBudget, Symbol};
use crate::meaning::{MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::stores::StateCore;
use crate::token::{Catcode, Token};

fn with_core(f: impl for<'id> FnOnce(StateCore<GenerationBrand<'id>>, Interner)) {
    with_generation(|generation| {
        f(
            StateCore::new(generation).expect("state core"),
            Interner::new(InternerBudget::new(64, 64, 1024).expect("budget")),
        );
    });
}

fn name<G>(core: &mut StateCore<G>, names: &mut Interner, name: &str) -> Symbol {
    let symbol = names.intern(name).expect("name").symbol();
    core.state_mut().admit_symbol(symbol).expect("admit name");
    symbol
}

fn word(ch: char) -> TokenWord {
    TokenWord::pack(Token::Char {
        ch,
        cat: Catcode::Letter,
    })
}

fn install<G>(
    core: &mut StateCore<G>,
    target: Symbol,
    ch: char,
    scope: AssignmentScope,
) -> DefinitionRef<G> {
    let mut stores = core.admit_mut().expect("admit stores");
    let publication = stores
        .allocate_definition(&[], &[word(ch)])
        .expect("publish");
    let reference = publication.reference();
    stores
        .assign_meaning(target, publication.meaning(MeaningFlags::EMPTY), scope)
        .expect("assign");
    reference
}

fn roots<G>(core: &StateCore<G>, reference: DefinitionRef<G>) -> usize {
    core.definition_roots().0.borrow().dynamic[reference.row_index()].roots
}

fn assert_live<G>(core: &StateCore<G>, reference: DefinitionRef<G>, live: bool) {
    assert_eq!(
        core.definition_roots()
            .0
            .borrow()
            .entry(reference)
            .is_some(),
        live
    );
}

fn clear<G>(core: &mut StateCore<G>, target: Symbol) {
    core.state_mut()
        .assign_meaning(target, MeaningWord::UNDEFINED, AssignmentScope::Global)
        .expect("clear");
}

#[test]
fn replacement_without_checkpoint_reclaims_and_aliases_share_one_charge() {
    with_core(|mut core, mut names| {
        let target = name(&mut core, &mut names, "target");
        let alias = name(&mut core, &mut names, "alias");
        let accounting = core.memory_accounting();
        let baseline = accounting.words(false);
        let original = install(&mut core, target, 'a', AssignmentScope::Global);
        let original_charge = accounting.words(false);
        let value = *core.state().meaning_word(target).expect("live meaning");
        core.state_mut()
            .assign_meaning(target, value, AssignmentScope::Global)
            .expect("self let");
        assert_eq!(
            roots(&core, original),
            1,
            "self assignment acquires before retiring the cell"
        );
        core.state_mut()
            .assign_meaning(alias, value, AssignmentScope::Global)
            .expect("alias");
        assert_eq!(roots(&core, original), 2);
        assert_eq!(
            accounting.words(false),
            original_charge,
            "aliases charge the actual payload once"
        );
        let replacement = install(&mut core, target, 'b', AssignmentScope::Global);
        assert_live(&core, original, true);
        clear(&mut core, alias);
        assert_live(&core, original, false);
        assert_eq!(roots(&core, original), 0);
        assert_eq!(
            accounting.words(false),
            original_charge,
            "only the equal-sized replacement remains"
        );
        clear(&mut core, target);
        assert_live(&core, replacement, false);
        assert_eq!(accounting.words(false), baseline);
        assert!(
            core.definition_roots().retained_bytes() > 0,
            "stable slot capacity is reported separately"
        );
    });
}

#[test]
fn group_and_checkpoint_keep_distinct_preimages_and_global_alias_keeps_same_key() {
    with_core(|mut core, mut names| {
        let target = name(&mut core, &mut names, "target");
        let alias = name(&mut core, &mut names, "alias");
        let zero = install(&mut core, target, 'z', AssignmentScope::Global);
        let checkpoint = core.state_mut().journal_cursor().expect("checkpoint");
        let one = install(&mut core, target, 'o', AssignmentScope::Global);
        core.state_mut()
            .begin_group(crate::GroupKind::Simple, 1)
            .expect("group");
        let two = install(&mut core, target, 't', AssignmentScope::Local);
        let value = *core
            .state()
            .meaning_word(target)
            .expect("live local meaning");
        core.state_mut()
            .assign_meaning(alias, value, AssignmentScope::Global)
            .expect("global local alias");
        assert_eq!(
            (roots(&core, zero), roots(&core, one), roots(&core, two)),
            (1, 1, 2)
        );
        core.state_mut()
            .end_group(crate::GroupKind::Simple)
            .expect("unsave");
        assert_eq!(
            core.state().meaning(target).expect("meaning"),
            ResolvedMeaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition: one
            }
        );
        assert_eq!(
            core.state().meaning(alias).expect("alias"),
            ResolvedMeaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition: two
            }
        );
        assert_eq!(
            (roots(&core, zero), roots(&core, one), roots(&core, two)),
            (1, 1, 1)
        );
        core.state_mut().restore(checkpoint).expect("restore");
        assert_live(&core, zero, true);
        assert_live(&core, one, false);
        assert_live(&core, two, false);
        clear(&mut core, target);
        core.state_mut()
            .release_checkpoint_root(checkpoint, None)
            .expect("release");
        assert_live(&core, zero, false);
    });
}

#[test]
fn retained_checkpoint_repeated_restore_and_pruning_retire_actual_preimages() {
    with_core(|mut core, mut names| {
        let target = name(&mut core, &mut names, "target");
        let original = install(&mut core, target, 'a', AssignmentScope::Global);
        let checkpoint = core.state_mut().journal_cursor().expect("checkpoint");
        for ch in ['b', 'c', 'd'] {
            let abandoned = install(&mut core, target, ch, AssignmentScope::Global);
            assert_eq!(roots(&core, original), 1);
            core.state_mut()
                .restore(checkpoint)
                .expect("reusable restore");
            assert_live(&core, abandoned, false);
            assert_eq!(roots(&core, original), 1);
            assert_eq!(
                core.admit().definition(original).replacement_text(),
                [word('a')]
            );
        }
        let head = install(&mut core, target, 'h', AssignmentScope::Global);
        let newer = core.state_mut().journal_cursor().expect("new checkpoint");
        core.state_mut()
            .release_checkpoint_root(checkpoint, Some(newer))
            .expect("prune old floor");
        assert_live(&core, original, false);
        core.state_mut()
            .release_checkpoint_root(newer, None)
            .expect("release last root");
        assert_live(&core, head, true);
        clear(&mut core, target);
        assert_live(&core, head, false);
    });
}

#[test]
fn candidate_rejection_acceptance_and_pruning_preserve_stable_definition_identity() {
    with_core(|mut core, mut names| {
        let target = name(&mut core, &mut names, "target");
        let original = install(&mut core, target, 'a', AssignmentScope::Global);
        let restart = core.state_mut().journal_cursor().expect("restart");
        let dense = core.state().checkpoint_cursor();
        let generation = core.generation_cursor();
        let accepted = install(&mut core, target, 'b', AssignmentScope::Global);
        let tail = core
            .begin_checkpoint_candidate(restart, dense, generation)
            .expect("candidate");
        let rejected = install(&mut core, target, 'c', AssignmentScope::Global);
        core.reject_checkpoint_candidate(restart, dense, generation, tail)
            .expect("reject");
        assert_live(&core, rejected, false);
        assert_live(&core, accepted, true);
        assert_eq!(
            core.state().meaning(target).expect("accepted meaning"),
            ResolvedMeaning::Macro {
                flags: MeaningFlags::EMPTY,
                definition: accepted
            }
        );
        let tail = core
            .begin_checkpoint_candidate(restart, dense, generation)
            .expect("second candidate");
        let candidate = install(&mut core, target, 'd', AssignmentScope::Global);
        core.accept_checkpoint_candidate(tail);
        assert_live(&core, accepted, false);
        assert_live(&core, original, true);
        assert_live(&core, candidate, true);
        assert_live(&core, rejected, false);
        assert_ne!(
            rejected, candidate,
            "a rejected coordinate can never identify a later publication"
        );
        core.state_mut()
            .release_checkpoint_root(restart, None)
            .expect("prune");
        assert_live(&core, original, false);
        clear(&mut core, target);
        assert_live(&core, candidate, false);
    });
}

#[test]
fn stale_assignment_fails_before_history() {
    with_core(|mut core, mut names| {
        let target = name(&mut core, &mut names, "target");
        let original = install(&mut core, target, 'a', AssignmentScope::Global);
        let restart = core.state_mut().journal_cursor().expect("checkpoint");
        let publication = core
            .admit_mut()
            .expect("stores")
            .allocate_definition(&[], &[word('b')])
            .expect("publication");
        let stale_meaning = publication.meaning(MeaningFlags::EMPTY);
        drop(publication);
        assert_eq!(
            core.state_mut()
                .assign_meaning(target, stale_meaning, AssignmentScope::Global),
            Err(crate::StateError::InvalidDefinition)
        );
        assert_eq!(
            core.state().journal().checkpoint_entry_count(),
            0,
            "validation precedes history ownership"
        );
        assert_eq!(roots(&core, original), 1);

        core.state_mut()
            .release_checkpoint_root(restart, None)
            .expect("release");
        clear(&mut core, target);
    });
}

#[test]
fn failed_native_publication_assignment_retires_the_uninstalled_root() {
    crate::with_universe(
        InternerBudget::new(64, 64, 1024).expect("budget"),
        |universe| {
            let unadmitted = universe
                .interner_mut()
                .intern("unadmitted")
                .expect("name without cell admission")
                .symbol();
            let publication = universe
                .allocate_definition(&[], &[word('c')])
                .expect("publication");
            let abandoned = publication.reference();
            let roots = universe.core.as_ref().expect("core").definition_roots();
            let result = universe
                .command_context()
                .expect("context")
                .assign_published_definition(
                    unadmitted,
                    publication,
                    MeaningFlags::EMPTY,
                    AssignmentScope::Global,
                );
            assert!(matches!(result, Err(crate::StateError::Bank(_))));
            assert!(roots.0.borrow().entry(abandoned).is_none());
        },
    )
    .expect("universe");
}

#[test]
fn primitive_registry_and_unwinding_unsave_retire_their_actual_definition_positions() {
    crate::with_universe(
        InternerBudget::new(64, 64, 1024).expect("budget"),
        |universe| {
            let target = universe.intern("target").expect("target");
            let original = universe
                .allocate_definition(&[], &[word('a')])
                .expect("publication");
            let reference = original.reference();
            universe.register_primitive_word("frozen", original.meaning(MeaningFlags::FROZEN));
            drop(original);
            let roots = universe.core.as_ref().expect("core").definition_roots();
            assert_eq!(
                roots.0.borrow().dynamic[reference.row_index()].roots,
                1,
                "registry is its actual owner"
            );
            universe
                .begin_group(crate::GroupKind::Simple, 1)
                .expect("group");
            let publication = universe
                .allocate_definition(&[], &[word('b')])
                .expect("local publication");
            let local = publication.reference();
            universe
                .assign_meaning(
                    target,
                    publication.meaning(MeaningFlags::EMPTY),
                    AssignmentScope::Local,
                )
                .expect("local assignment");
            drop(publication);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                universe
                    .command_context()
                    .expect("context")
                    .end_group_observed(crate::GroupKind::Simple, |_, _| panic!("observer unwind"))
                    .expect("group exit");
            }));
            assert!(result.is_err());
            assert!(
                roots.0.borrow().entry(local).is_none(),
                "popped losing owner retires even through observer unwind"
            );
            assert!(
                roots.0.borrow().entry(reference).is_some(),
                "unrelated frozen root stays live"
            );
        },
    )
    .expect("universe");
}

#[test]
fn abandoned_publication_tombstones_but_full_frame_and_cold_view_keep_physical_charge() {
    with_generation(|mut generation| {
        let accounting = generation.memory_accounting();
        let baseline = accounting.words(false);
        let mut sources = crate::source_map::SourceMap::default();
        let position = sources
            .register(
                crate::SourceId::new(0),
                crate::source_map::SourceDescriptor::generated(std::sync::Arc::from(&b"xy"[..])),
            )
            .expect("backed source");
        let origin = crate::token::OriginId::direct_source(position).expect("direct backed origin");
        let arena = generation.definitions_mut();
        let parameter = [TokenWord::pack(Token::param(1)), word('x'), word('x')];
        let replacement = [word('y'), TokenWord::pack(Token::param(1))];
        let build = arena.begin_build(origin).expect("definition build");
        for value in parameter {
            arena.push_parameter(build, value).expect("parameter");
        }
        arena.finish_parameters(build).expect("boundary");
        for value in replacement {
            arena.push_replacement(build, value).expect("replacement");
        }
        let publication = arena.seal_build(build).expect("publication");
        let reference = publication.reference();
        let frame = arena.admit_macro_body(reference).expect("frame");
        let view = arena.get(reference);
        let charge = accounting.words(false);
        drop(publication);
        assert!(
            arena.admit_macro_body(reference).is_none(),
            "zero semantic roots tombstone promptly"
        );
        assert_eq!(
            accounting.words(false),
            charge,
            "payload owners keep the charge after tombstoning"
        );
        assert_eq!(frame.parameter_words(), parameter);
        assert_eq!(
            frame.definition_words(),
            [parameter.as_slice(), replacement.as_slice()].concat()
        );
        assert_eq!(frame.parameter_pattern().parameter_count(), 1);
        assert_eq!(
            frame
                .parameter_pattern()
                .delimiter(0, frame.parameter_words())
                .expect("literal delimiter")
                .words(),
            &parameter[1..]
        );
        assert_eq!(frame.payload.origin, origin);
        assert_eq!(
            frame
                .parameter_pattern()
                .delimiter(0, frame.parameter_words())
                .expect("literal delimiter")
                .overlap(1, word('x')),
            1,
            "sealed overlap table remains usable after slot retirement"
        );
        assert_eq!(view.definition_origin(), origin);
        assert_eq!(view.replacement_text(), replacement);
        drop(view);
        assert_eq!(accounting.words(false), charge);
        drop(frame);
        assert_eq!(
            accounting.words(false),
            baseline,
            "final physical drop releases once"
        );
        let later = arena.allocate(&[], &[]).expect("later publication");
        assert_ne!(reference, later.reference());
        drop(later);
        assert_eq!(accounting.words(false), baseline);
    });
}

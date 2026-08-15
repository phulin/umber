use super::{MacroDefinitionProvenance, MacroMeaning};
use crate::SourceId;
use crate::meaning::{Meaning, MeaningFlags};
use crate::provenance::{OriginListRef, SyntheticOriginKind};
use crate::stores::Stores;
use crate::token::{Catcode, Token};

fn replacement(stores: &mut Stores, index: u32) -> crate::token_store::TokenListRef {
    stores.intern_token_list_ref_in_domain(
        &[
            Token::Char {
                ch: char::from_u32(b'a' as u32 + index % 26).expect("ASCII letter"),
                cat: Catcode::Other,
            },
            Token::Char {
                ch: char::from_u32(0x100 + index).expect("bounded test scalar"),
                cat: Catcode::Other,
            },
        ],
        None,
    )
}

#[test]
fn exact_body_dedup_survives_candidate_collision_and_separates_occurrences() {
    let mut stores = Stores::new();
    stores
        .testing_macro_store_mut()
        .testing_force_candidate_collision();
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    let first_replacement = replacement(&mut stores, 1);
    let second_replacement = replacement(&mut stores, 2);

    let first = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        empty.id(),
        first_replacement.id(),
    ));
    let equivalent = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        empty.id(),
        first_replacement.id(),
    ));
    let distinct = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::PROTECTED,
        empty.id(),
        second_replacement.id(),
    ));

    assert_ne!(
        first.id(),
        equivalent.id(),
        "definition occurrences stay distinct"
    );
    assert!(
        first.body_ptr_eq(&equivalent),
        "equal exact bodies deduplicate"
    );
    assert!(
        !first.body_ptr_eq(&distinct),
        "a candidate collision cannot alias content"
    );
}

#[test]
fn provenance_is_occurrence_local_and_not_part_of_body_identity() {
    let mut stores = Stores::new();
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    let replacement = replacement(&mut stores, 3);
    let first_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Engine);
    let second_origin = stores.synthetic_origin_ref(SyntheticOriginKind::Format);
    let meaning = MacroMeaning::new(MeaningFlags::LONG, empty.id(), replacement.id());
    let first = stores.intern_macro_with_provenance(
        meaning,
        Some(MacroDefinitionProvenance::new(
            first_origin.clone(),
            OriginListRef::empty(),
            OriginListRef::empty(),
        )),
    );
    let second = stores.intern_macro_with_provenance(
        meaning,
        Some(MacroDefinitionProvenance::new(
            second_origin.clone(),
            OriginListRef::empty(),
            OriginListRef::empty(),
        )),
    );

    assert!(first.body_ptr_eq(&second));
    assert_eq!(
        stores
            .macro_definition_provenance(first.id())
            .definition_origin(),
        first_origin.id()
    );
    assert_eq!(
        stores
            .macro_definition_provenance(second.id())
            .definition_origin(),
        second_origin.id()
    );
}

#[test]
fn diagnostic_invocation_provenance_does_not_keep_definition_alive() {
    let mut stores = Stores::new();
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    let definition = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        empty.id(),
        empty.id(),
    ));
    let id = definition.id();
    let definition_operand = stores.macro_definition_observation_operand(id) as u64;
    let source = stores.source_origin(SourceId::new(7), 0, 1, 1);
    let invocation = stores.macro_invocation_origin(
        id,
        source,
        crate::token::OriginId::UNKNOWN,
        crate::token::OriginId::UNKNOWN,
    );
    drop(definition);

    assert!(!stores.testing_macro_store().contains(id));
    let crate::provenance::OriginRecord::MacroInvocation(invocation) = stores.origin(invocation)
    else {
        panic!("expected macro invocation provenance");
    };
    assert_eq!(invocation.definition_operand(), definition_operand);
    assert_eq!(
        stores.testing_macro_store().testing_live_totals(),
        (0, 0, 0, 0)
    );
}

#[test]
fn env_current_undo_and_group_exit_own_exact_definitions() {
    let mut stores = Stores::new();
    let name = stores.intern("owned-macro");
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    let outer_replacement = replacement(&mut stores, 4);
    let inner_replacement = replacement(&mut stores, 5);
    let outer = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::EMPTY,
        empty.id(),
        outer_replacement.id(),
    ));
    stores.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: outer.id(),
        },
    );
    let outer_current = outer.strong_count();

    stores.enter_group();
    let inner = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::LONG,
        empty.id(),
        inner_replacement.id(),
    ));
    stores.set_meaning(
        name,
        Meaning::Macro {
            flags: MeaningFlags::LONG,
            definition: inner.id(),
        },
    );
    assert!(
        outer.strong_count() >= outer_current,
        "undo retains displaced binding"
    );
    let _ = stores.leave_group();
    assert_eq!(
        stores.meaning(name),
        Meaning::Macro {
            flags: MeaningFlags::EMPTY,
            definition: outer.id(),
        }
    );
    assert_eq!(
        inner.strong_count(),
        1,
        "group exit releases current and undo roots"
    );
}

#[test]
fn ten_thousand_bounded_live_redefinitions_plateau_macro_storage() {
    let mut stores = Stores::new();
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    for index in 0..10_000_u32 {
        let body = replacement(&mut stores, index);
        let definition = stores.intern_macro(MacroMeaning::new(
            MeaningFlags::EMPTY,
            empty.id(),
            body.id(),
        ));
        drop(definition);
    }
    let _sentinel = stores.intern_macro(MacroMeaning::new(
        MeaningFlags::OUTER,
        empty.id(),
        empty.id(),
    ));
    let (bodies, _, definitions, _) = stores.testing_macro_store().testing_live_totals();
    let (body_shape, definition_shape) = stores.testing_macro_store().testing_pool_shapes();
    assert_eq!((bodies, definitions), (1, 1));
    assert!(
        body_shape.0 <= 2,
        "dead body slots must be reusable: {body_shape:?}"
    );
    assert!(
        definition_shape.0 <= 2,
        "dead definition slots must be reusable: {definition_shape:?}"
    );
}

#[test]
fn all_roots_live_grow_by_exact_object_and_logical_byte_totals() {
    let mut stores = Stores::new();
    let empty = stores.token_list_ref(crate::ids::TokenListId::EMPTY);
    let baseline = stores.testing_macro_store().testing_live_totals();
    let mut roots = Vec::new();
    for index in 0..128_u32 {
        let body = replacement(&mut stores, index);
        roots.push(stores.intern_macro(MacroMeaning::new(
            MeaningFlags::EMPTY,
            empty.id(),
            body.id(),
        )));
    }
    let live = stores.testing_macro_store().testing_live_totals();
    assert_eq!(live.0 - baseline.0, roots.len());
    assert_eq!(live.2 - baseline.2, roots.len());
    assert_eq!(
        live.1 - baseline.1,
        roots.len() * core::mem::size_of::<super::MacroBodyValue>()
    );
    assert_eq!(
        live.3 - baseline.3,
        roots.len() * core::mem::size_of::<super::MacroDefinitionValue>()
    );
}

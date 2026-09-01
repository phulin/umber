use tex_state::ResolvedMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

use super::*;

#[test]
fn every_primitive_opcode_has_a_capability_classification() {
    let mut classified = 0;
    for operand in 0..=266 {
        let Some(primitive) = UnexpandablePrimitive::from_operand(operand) else {
            continue;
        };
        let _barrier = canonical_command_barrier::<()>(ResolvedMeaning::Static(
            Meaning::UnexpandablePrimitive(primitive),
        ));
        classified += 1;
    }
    assert_eq!(classified, 263);
}

#[test]
fn deferred_effects_stay_on_the_transaction_free_path() {
    for primitive in [
        UnexpandablePrimitive::OpenOut,
        UnexpandablePrimitive::CloseOut,
        UnexpandablePrimitive::Write,
    ] {
        let barrier = canonical_command_barrier::<()>(ResolvedMeaning::Static(
            Meaning::UnexpandablePrimitive(primitive),
        ));
        assert_eq!(barrier, None);
    }
}

#[test]
fn direct_dispatch_materializes_only_uncommon_barriers() {
    for primitive in [
        UnexpandablePrimitive::Def,
        UnexpandablePrimitive::Let,
        UnexpandablePrimitive::CatCode,
        UnexpandablePrimitive::BeginGroup,
        UnexpandablePrimitive::OpenOut,
    ] {
        assert_eq!(
            canonical_command_barrier::<()>(ResolvedMeaning::Static(
                Meaning::UnexpandablePrimitive(primitive),
            )),
            None,
            "ordinary dispatch must not materialize a barrier for {primitive:?}",
        );
    }

    let Some(CommandBarrier::Resource) = canonical_command_barrier::<()>(ResolvedMeaning::Static(
        Meaning::ExpandablePrimitive(ExpandablePrimitive::Input),
    )) else {
        panic!("input must select the resource barrier");
    };

    let Some(CommandBarrier::Transaction(shipout)) =
        canonical_command_barrier::<()>(ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Shipout,
        )))
    else {
        panic!("shipout must retain its late-failure transaction");
    };
    assert_eq!(shipout.projection().owners(), SHIPOUT_TRANSACTION);
}

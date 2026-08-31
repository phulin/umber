use tex_state::ResolvedMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

use super::*;

#[test]
fn every_primitive_opcode_has_a_capability_classification() {
    let mut classified = 0;
    for operand in 0..=265 {
        let Some(primitive) = UnexpandablePrimitive::from_operand(operand) else {
            continue;
        };
        let _preflight = canonical_command_preflight::<()>(ResolvedMeaning::Static(
            Meaning::UnexpandablePrimitive(primitive),
        ));
        classified += 1;
    }
    assert_eq!(classified, 262);
}

#[test]
fn deferred_effects_stay_on_the_transaction_free_path() {
    for primitive in [
        UnexpandablePrimitive::OpenOut,
        UnexpandablePrimitive::CloseOut,
        UnexpandablePrimitive::Write,
    ] {
        let preflight = canonical_command_preflight::<()>(ResolvedMeaning::Static(
            Meaning::UnexpandablePrimitive(primitive),
        ));
        assert_eq!(preflight, ordinary(MATERIAL));
    }
}

#[test]
fn compact_preflight_stores_only_variant_specific_facts() {
    assert_eq!(std::mem::size_of::<NarrowTransactionSpec>(), 2);
    assert_eq!(std::mem::size_of::<CommandPreflight>(), 6);

    let CommandPreflight::Resource(input) = canonical_command_preflight::<()>(
        ResolvedMeaning::Static(Meaning::ExpandablePrimitive(ExpandablePrimitive::Input)),
    ) else {
        panic!("input must retain its resource retry classification");
    };
    assert_eq!(input.resources(), ResourceCapabilities::INPUT);
    assert_eq!(input.retry_transaction().projection().owners(), RETRY_SCAN);

    let CommandPreflight::Transaction(shipout) =
        canonical_command_preflight::<()>(ResolvedMeaning::Static(Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Shipout,
        )))
    else {
        panic!("shipout must retain its late-failure transaction");
    };
    assert_eq!(
        shipout.transaction().projection().owners(),
        SHIPOUT_TRANSACTION
    );
}

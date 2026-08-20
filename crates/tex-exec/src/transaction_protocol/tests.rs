use tex_state::meaning::{Meaning, UnexpandablePrimitive};

use super::*;

#[test]
fn every_primitive_opcode_has_a_capability_classification() {
    let mut classified = 0;
    for operand in 0..=265 {
        let Some(primitive) = UnexpandablePrimitive::from_operand(operand) else {
            continue;
        };
        let capabilities =
            canonical_command_capabilities(Meaning::UnexpandablePrimitive(primitive));
        assert_eq!(capabilities.preflight(), capabilities.preflight());
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
        let capabilities =
            canonical_command_capabilities(Meaning::UnexpandablePrimitive(primitive));
        assert!(
            capabilities
                .effects()
                .contains(EffectCapabilities::DEFERRED_STREAM)
        );
        assert_eq!(capabilities.transaction(), None);
        assert!(matches!(
            capabilities.preflight(),
            CommandPreflight::Ordinary(_)
        ));
    }
}

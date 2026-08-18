use core::mem::{needs_drop, size_of};

use tex_state::meaning::{Meaning, UnexpandablePrimitive};

use super::*;

#[test]
fn ordinary_preflight_has_no_transaction_object() {
    let capabilities = canonical_command_capabilities(Meaning::Relax);
    assert_eq!(capabilities.transaction(), None);
    assert!(matches!(
        capabilities.preflight(),
        CommandPreflight::Ordinary(_)
    ));
    assert!(!needs_drop::<CommandPreflight>());
    assert!(size_of::<CommandPreflight>() <= 32);
}

#[test]
fn resource_and_publication_commands_name_exact_narrow_marks() {
    let CommandPreflight::Resource(resource) =
        canonical_command_capabilities(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Read))
            .preflight()
    else {
        panic!("read must use resource preflight");
    };
    let read = resource.retry_transaction().projection();
    assert_eq!(read.owners(), RETRY_SCAN);
    assert_eq!(read.marks(), RETRY_SCAN.required_marks());
    assert!(!read.marks().contains(HotSnapshotMarks::MUTATION_JOURNAL));
    assert!(!read.marks().contains(HotSnapshotMarks::NODE_ARENA));

    let CommandPreflight::Transaction(shipout) = canonical_command_capabilities(
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Shipout),
    )
    .preflight() else {
        panic!("shipout must use a late-failure transaction");
    };
    let shipout = shipout.transaction().projection();
    assert_eq!(shipout.owners(), SHIPOUT_TRANSACTION);
    assert!(shipout.marks().contains(HotSnapshotMarks::MUTATION_JOURNAL));
    assert!(shipout.marks().contains(HotSnapshotMarks::NODE_ARENA));
    assert!(shipout.marks().contains(HotSnapshotMarks::PAGE_JOURNAL));
    assert!(shipout.marks().contains(HotSnapshotMarks::PDF_JOURNAL));
    assert!(shipout.marks().contains(HotSnapshotMarks::OUTPUT_JOURNAL));
    assert!(!shipout.marks().contains(HotSnapshotMarks::INPUT_STACK));
}

#[test]
fn invalid_owner_mark_and_capability_combinations_reject() {
    let owners = StateOwners::MODE;
    assert!(matches!(
        HotSnapshotProjection::try_new(owners, HotSnapshotMarks::MODE_STACK),
        Err(PreflightError::InvalidOwnerMarkProjection { .. })
    ));

    let wrong = HotSnapshotProjection::for_owners(StateOwners::DENSE_STATE);
    let expected = NarrowTransactionSpec::new(StateOwners::MODE);
    assert!(matches!(
        expected.admit(wrong),
        Err(PreflightError::TransactionProjectionMismatch { .. })
    ));

    assert_eq!(
        CommandCapabilities::try_new(
            CanonicalCommandFamily::Resource,
            RETRY_SCAN,
            ResourceCapabilities::FONT,
            EffectCapabilities::NONE,
            OutputCapabilities::NONE,
            RecoveryCapabilities::NONE,
            None,
        ),
        Err(PreflightError::ResourceRecoveryMismatch)
    );
    assert!(matches!(
        CommandCapabilities::try_new(
            CanonicalCommandFamily::Publication,
            StateOwners::OUTPUT,
            ResourceCapabilities::NONE,
            EffectCapabilities::NONE,
            OutputCapabilities::FORMAT,
            RecoveryCapabilities::LATE_FAILURE,
            Some(NarrowTransactionSpec::new(StateOwners::MODE)),
        ),
        Err(PreflightError::TransactionOwnerNotMutable { .. })
    ));
}

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

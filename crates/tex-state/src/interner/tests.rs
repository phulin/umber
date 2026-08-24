use super::{
    ControlSequenceKind, Interner, InternerAccessError, InternerBudget, InternerBudgetError,
    InternerError, InternerResource, SYMBOL_CAPACITY,
};

fn budget(names: u32, slots: u32, bytes: u32) -> InternerBudget {
    InternerBudget::new(names, slots, bytes).expect("valid test budget")
}

fn roomy() -> Interner {
    Interner::new(budget(128, 256, 4_096))
}

#[test]
fn budget_configuration_is_explicit_and_structurally_validated() {
    let configured = budget(12, 20, 1_024);
    assert_eq!(configured.control_sequence_names(), 12);
    assert_eq!(configured.slots(), 20);
    assert_eq!(configured.bytes(), 1_024);
    assert_eq!(
        InternerBudget::new(3, 2, 10),
        Err(InternerBudgetError::NamesExceedSlots { names: 3, slots: 2 })
    );
    assert_eq!(
        InternerBudget::new(1, SYMBOL_CAPACITY + 1, 10),
        Err(InternerBudgetError::SlotCapacity {
            requested: SYMBOL_CAPACITY + 1,
            maximum: SYMBOL_CAPACITY,
        })
    );
}

#[test]
fn control_sequence_interning_is_dense_idempotent_and_utf8_exact() {
    let mut interner = roomy();
    let ascii = interner.intern("par").expect("ASCII name");
    let unicode = interner.intern("é漢字🙂").expect("Unicode name");
    let repeated = interner.intern("par").expect("repeated name");

    assert_eq!(ascii, repeated);
    assert_eq!(ascii.symbol().raw(), 0);
    assert_eq!(unicode.symbol().raw(), 1);
    assert_eq!(interner.resolve_id(ascii), Ok("par"));
    assert_eq!(interner.resolve_id(unicode), Ok("é漢字🙂"));
    assert_eq!(interner.usage().control_sequence_names(), 2);
    assert_eq!(interner.usage().slots(), 2);
    assert_eq!(interner.usage().bytes(), 15);
}

#[test]
fn control_sequence_names_and_token_spellings_share_only_epoch_resources() {
    let mut interner = roomy();
    let named = interner.intern("mark-text").expect("control sequence");
    let spelling = interner
        .intern_spelling("mark-text")
        .expect("retained token spelling");

    assert_eq!(interner.resolve_id(named), Ok("mark-text"));
    assert_eq!(interner.resolve_spelling(spelling), Ok("mark-text"));
    assert_ne!(named.raw(), spelling.raw());
    assert_eq!(interner.usage().control_sequence_names(), 1);
    assert_eq!(interner.usage().slots(), 2);
    assert_eq!(interner.usage().bytes(), 18);
    assert_eq!(interner.get("mark-text"), Some(named));
    assert_eq!(interner.get_spelling("mark-text"), Some(spelling));
}

#[test]
fn active_named_and_internal_namespaces_never_mutate_an_issued_identity() {
    let mut interner = roomy();
    let named = interner.intern("~").expect("named control symbol");
    let active = interner.intern_active('~').expect("active character");
    let internal = interner.intern_internal("~").expect("internal symbol");

    assert_ne!(named, active);
    assert_ne!(named, internal);
    assert_ne!(active, internal);
    assert_eq!(
        interner.kind_id(named),
        Ok(ControlSequenceKind::SingleCharacter)
    );
    assert_eq!(
        interner.kind_id(active),
        Ok(ControlSequenceKind::ActiveCharacter)
    );
    assert_eq!(
        interner.kind_id(internal),
        Ok(ControlSequenceKind::Internal)
    );
    assert_eq!(interner.resolve_id(named), Ok("~"));
    assert_eq!(interner.resolve_id(active), Ok("~"));
    assert_eq!(interner.resolve_id(internal), Ok("~"));
}

#[test]
fn identities_survive_group_command_and_incremental_rollback_boundaries() {
    let mut interner = roomy();
    let before_group = interner.intern("stable").expect("stable name");

    // These cursors belong to the mutable TeX state, command attempt, and
    // incremental revision owners. The interner deliberately exposes no
    // corresponding rollback cursor or truncation operation.
    let _group_cursor = 7_u32;
    let during_group = interner.intern("group-local").expect("group name");
    let _command_cursor = 11_u32;
    let during_command = interner.intern("failed-command").expect("command name");
    let _revision_cursor = 13_u32;
    let during_revision = interner
        .intern("discarded-revision")
        .expect("revision name");

    assert_eq!(interner.intern("stable"), Ok(before_group));
    assert_eq!(interner.intern("group-local"), Ok(during_group));
    assert_eq!(interner.intern("failed-command"), Ok(during_command));
    assert_eq!(interner.intern("discarded-revision"), Ok(during_revision));
    assert_eq!(interner.usage().control_sequence_names(), 4);
}

#[test]
fn session_qualified_identities_are_rejected_by_foreign_epochs() {
    let mut left = roomy();
    let mut right = roomy();
    let left_symbol = left.intern("same-slot").expect("left symbol");
    let right_symbol = right.intern("same-slot").expect("right symbol");
    let left_spelling = left.intern_spelling("text").expect("left spelling");
    let right_spelling = right.intern_spelling("text").expect("right spelling");

    assert_eq!(left_symbol.symbol(), right_symbol.symbol());
    assert_ne!(left_symbol, right_symbol);
    assert_eq!(
        left.resolve_id(right_symbol),
        Err(InternerAccessError::ForeignEpoch)
    );
    assert_eq!(
        right.resolve_id(left_symbol),
        Err(InternerAccessError::ForeignEpoch)
    );
    assert_eq!(
        left.resolve_spelling(right_spelling),
        Err(InternerAccessError::ForeignEpoch)
    );
    assert_eq!(
        right.resolve_spelling(left_spelling),
        Err(InternerAccessError::ForeignEpoch)
    );
}

#[test]
fn name_budget_does_not_consume_or_recycle_other_epoch_slots() {
    let mut interner = Interner::new(budget(1, 4, 64));
    let first = interner.intern("first").expect("first name");
    let before = interner.usage();

    assert_eq!(
        interner.intern("second"),
        Err(InternerError::BudgetExceeded {
            resource: InternerResource::ControlSequenceNames,
            limit: 1,
            attempted: 2,
        })
    );
    assert_eq!(interner.usage(), before);
    assert_eq!(interner.intern("first"), Ok(first));
    assert!(interner.intern_spelling("second").is_ok());
}

#[test]
fn total_slot_budget_counts_names_and_spellings_together() {
    let mut interner = Interner::new(budget(2, 2, 64));
    interner.intern("name").expect("name slot");
    interner.intern_spelling("text").expect("spelling slot");
    let before = interner.usage();

    assert_eq!(
        interner.intern_spelling("overflow"),
        Err(InternerError::BudgetExceeded {
            resource: InternerResource::Slots,
            limit: 2,
            attempted: 3,
        })
    );
    assert_eq!(interner.usage(), before);
}

#[test]
fn byte_budget_charges_utf8_bytes_and_failed_appends_are_atomic() {
    let mut interner = Interner::new(budget(3, 3, 4));
    let unicode = interner.intern("é").expect("two UTF-8 bytes");
    let spelling = interner.intern_spelling("ab").expect("two ASCII bytes");
    let before = interner.usage();

    assert_eq!(
        interner.intern("x"),
        Err(InternerError::BudgetExceeded {
            resource: InternerResource::Bytes,
            limit: 4,
            attempted: 5,
        })
    );
    assert_eq!(interner.usage(), before);
    assert_eq!(interner.intern("é"), Ok(unicode));
    assert_eq!(interner.intern_spelling("ab"), Ok(spelling));
}

#[test]
fn whole_epoch_retirement_releases_storage_and_invalidates_every_identity() {
    let mut interner = roomy();
    let symbol = interner.intern("retire-me").expect("symbol");
    let spelling = interner.intern_spelling("and-me").expect("spelling");
    let live_usage = interner.usage();
    assert!(interner.arena.capacity() > 0);
    assert!(interner.entries.capacity() > 0);
    assert!(interner.index.capacity() > 0);

    let retirement = interner.retire().expect("first retirement");

    assert_eq!(retirement.usage(), live_usage);
    assert_eq!(interner.usage().slots(), 0);
    assert_eq!(interner.arena.capacity(), 0);
    assert_eq!(interner.entries.capacity(), 0);
    assert_eq!(interner.index.capacity(), 0);
    assert!(interner.is_empty());
    assert!(interner.is_retired());
    assert_eq!(
        interner.resolve_id(symbol),
        Err(InternerAccessError::RetiredEpoch)
    );
    assert_eq!(
        interner.resolve_spelling(spelling),
        Err(InternerAccessError::RetiredEpoch)
    );
    assert_eq!(interner.intern("new"), Err(InternerError::RetiredEpoch));
    assert_eq!(interner.retire(), Err(InternerError::RetiredEpoch));
}

#[test]
fn independent_sessions_share_no_mutable_dynamic_name_registry() {
    let threads = (0..16)
        .map(|_| {
            std::thread::spawn(|| {
                let mut interner = roomy();
                interner.intern("relax").expect("thread-local symbol")
            })
        })
        .collect::<Vec<_>>();
    let identities = threads
        .into_iter()
        .map(|thread| thread.join().expect("interner thread"))
        .collect::<Vec<_>>();

    assert!(identities.iter().all(|id| id.symbol().raw() == 0));
    assert_eq!(
        identities
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        identities.len()
    );
}

#[test]
fn hash_occupancy_is_monotonic_metadata_not_name_reallocation() {
    let mut interner = roomy();
    let null = interner.intern_hash("").expect("null control sequence");
    let x = interner
        .intern_hash("x")
        .expect("one-letter control symbol");
    let active = interner.intern_active('~').expect("active character");
    let internal = interner
        .intern_internal("frozen")
        .expect("frozen control sequence");
    let ordinary = interner.intern("multiletter").expect("retained name");
    assert_eq!(interner.multiletter_len(), 0);
    let hashed = interner
        .intern_hash("multiletter")
        .expect("same name reaches hash");
    let reused = interner
        .intern_hash("multiletter")
        .expect("occupied name is reused");

    assert_eq!(ordinary, hashed);
    assert_eq!(hashed, reused);
    assert_eq!(interner.is_hash_entry(null), Ok(false));
    assert_eq!(interner.is_hash_entry(x), Ok(false));
    assert_eq!(interner.is_hash_entry(active), Ok(false));
    assert_eq!(interner.is_hash_entry(internal), Ok(false));
    assert_eq!(interner.is_hash_entry(hashed), Ok(true));
    assert_eq!(interner.multiletter_len(), 1);
}

#[test]
fn semantic_projection_is_cached_once_per_control_sequence_slot() {
    let mut interner = roomy();
    let symbol = interner
        .intern("large_list_control_sequence")
        .expect("symbol");
    let cached = interner
        .semantic_atom_identity(symbol.symbol())
        .expect("cached projections");

    for _ in 0..10_000 {
        assert_eq!(
            interner.semantic_atom_identity(symbol.symbol()),
            Some(cached)
        );
    }
    assert_eq!(interner.semantic_atom(symbol.symbol()), Some(cached.0));
}

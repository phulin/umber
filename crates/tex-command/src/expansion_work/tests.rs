use tex_state::env::AssignmentScope;
use tex_state::meaning::{MeaningFlags, MeaningWord, ResolvedMeaning};
use tex_state::token::{OriginId, Token, TokenWord, TracedTokenWord};

use super::*;

fn empty_command<G>() -> CurrentCommand<G> {
    CurrentCommand::empty()
}

fn root_command<G>(work: &ExpansionWork<G>, key: &ExpansionWorkKey<G>) -> ExpansionCommandSlot<G> {
    match work.controls.get(key.root).expect("live root control") {
        ExpansionControl::Dispatch { command, .. } => *command,
        _ => panic!("dispatch root"),
    }
}

fn duplicate_key<G>(key: &ExpansionWorkKey<G>) -> ExpansionWorkKey<G> {
    ExpansionWorkKey {
        owner: key.owner,
        root: key.root,
        mark: key.mark,
    }
}

#[test]
fn reviewed_layout_bounds_are_compile_time_invariants() {
    assert_eq!(core::mem::size_of::<ExpansionWorkKey<()>>(), 32);
    assert!(core::mem::size_of::<ExpansionControl<()>>() <= 128);
    assert!(core::mem::size_of::<ExpansionCommandSlot<()>>() <= 16);
    assert!(core::mem::size_of::<ExpansionControlSlot<()>>() <= 16);
    assert!(core::mem::size_of::<ExpansionNameMark>() <= 16);
}

#[test]
fn command_slots_keep_one_address_across_chunk_growth_and_reuse() {
    let mut work = ExpansionWork::<()>::default();
    let key = work.begin_dispatch(empty_command()).expect("root");
    let root = root_command(&work, &key);
    let address = core::ptr::from_ref(work.command(root).expect("root command"));

    for _ in 0..(COMMANDS_PER_CHUNK * 3) {
        work.park_command(empty_command()).expect("parked command");
    }
    assert_eq!(
        core::ptr::from_ref(work.command(root).expect("stable root command")),
        address
    );
    work.abort(key).expect("abort root");

    let replacement = work.begin_dispatch(empty_command()).expect("replacement");
    let replacement_slot = root_command(&work, &replacement);
    assert_eq!(replacement_slot.lane.index(), root.lane.index());
    assert_ne!(replacement_slot.lane.serial(), root.lane.serial());
    assert_eq!(
        core::ptr::from_ref(work.command(replacement_slot).expect("reused address")),
        address
    );
    work.abort(replacement).expect("replacement abort");
}

#[test]
fn complete_marks_abort_nested_controls_commands_and_name_bytes_deepest_first() {
    let mut work = ExpansionWork::<()>::default();
    let key = work.begin_dispatch(empty_command()).expect("root");
    let opener = root_command(&work, &key);
    let name = work.name_mark().expect("name mark");
    for byte in b"control-sequence-name" {
        work.push_name_byte(*byte).expect("name byte");
    }

    let first = work.park_command(empty_command()).expect("saved first");
    let parent = work
        .push_control(ExpansionControl::ExpandAfter(ExpandAfterControl {
            opener,
            saved_first: Some(first),
            phase: ExpandAfterPhase::NeedOperands,
        }))
        .expect("parent control");
    let second = work.park_command(empty_command()).expect("second");
    let child = work
        .push_control(ExpansionControl::Dispatch {
            command: second,
            trace: TraceState::Unseen,
        })
        .expect("child control");
    let ExpansionControl::ExpandAfter(parent_control) =
        work.control_mut(parent).expect("parent control")
    else {
        panic!("expandafter parent")
    };
    parent_control.phase = ExpandAfterPhase::AwaitSecond {
        child: ExpansionChild::new(child, ExpandAfterSecondDestination),
    };

    assert_eq!(
        work.name_bytes(name)
            .expect("live name")
            .collect::<Vec<_>>(),
        b"control-sequence-name"
    );
    assert!(!work.is_quiescent());
    work.abort(key).expect("deep abort");
    assert!(work.is_quiescent());
    assert_eq!(work.counters().aborted_roots, 1);
    assert_eq!(work.counters().whole_control_copies, 0);
    assert_eq!(work.counters().command_clones, 0);
}

#[test]
fn stale_aba_and_foreign_work_keys_are_rejected_without_harming_live_work() {
    let mut first = ExpansionWork::<()>::default();
    let mut second = ExpansionWork::<()>::default();
    let key = first.begin_dispatch(empty_command()).expect("first root");
    let foreign = duplicate_key(&key);
    assert_eq!(second.abort(foreign), Err(ScratchError::InvalidCoordinate));
    assert!(!first.is_quiescent());

    let stale = duplicate_key(&key);
    let old_root = key.root;
    first.abort(key).expect("first abort");
    let replacement = first
        .begin_dispatch(empty_command())
        .expect("replacement root");
    assert_eq!(replacement.root.index(), old_root.index());
    assert_ne!(replacement.root.serial(), old_root.serial());
    assert_eq!(first.abort(stale), Err(ScratchError::InvalidCoordinate));
    assert!(!first.is_quiescent());
    first.abort(replacement).expect("replacement abort");
    assert!(first.is_quiescent());
    assert_eq!(first.counters().stale_key_rejections, 1);
    assert_eq!(second.counters().stale_key_rejections, 1);
}

#[test]
fn same_generation_foreign_lane_coordinates_are_rejected_before_access() {
    let mut first = ExpansionWork::<()>::default();
    let mut second = ExpansionWork::<()>::default();
    let first_key = first.begin_dispatch(empty_command()).expect("first root");
    let second_key = second.begin_dispatch(empty_command()).expect("second root");
    let first_command = root_command(&first, &first_key);
    let second_command = root_command(&second, &second_key);
    let first_control = first
        .push_control(ExpansionControl::Dispatch {
            command: first_command,
            trace: TraceState::Unseen,
        })
        .expect("first child control");
    let second_control = second
        .push_control(ExpansionControl::Dispatch {
            command: second_command,
            trace: TraceState::Unseen,
        })
        .expect("second child control");
    let first_name = first.name_mark().expect("first name mark");
    let second_name = second.name_mark().expect("second name mark");
    first.push_name_byte(b'a').expect("first name byte");
    second.push_name_byte(b'b').expect("second name byte");

    assert_eq!(first_command.lane, second_command.lane);
    assert_eq!(first_control.lane, second_control.lane);
    assert_eq!(first_name.offset, second_name.offset);
    assert_eq!(first_name.root_serial, second_name.root_serial);
    assert_ne!(first_name.owner, second_name.owner);

    assert!(matches!(
        second.command(first_command),
        Err(ScratchError::InvalidCoordinate)
    ));
    assert_eq!(
        second.take_command(first_command),
        Err(ScratchError::InvalidCoordinate)
    );
    assert!(matches!(
        second.control_mut(first_control),
        Err(ScratchError::InvalidCoordinate)
    ));
    assert_eq!(
        second.pop_control(first_control),
        Err(ScratchError::InvalidCoordinate)
    );
    assert!(matches!(
        second.name_bytes(first_name),
        Err(ScratchError::InvalidCoordinate)
    ));

    assert!(second.command(second_command).is_ok());
    assert!(second.control_mut(second_control).is_ok());
    assert_eq!(
        second
            .name_bytes(second_name)
            .expect("second name remains live")
            .collect::<Vec<_>>(),
        b"b"
    );
    assert_eq!(first.commands.len(), 1);
    assert_eq!(first.controls.len(), 2);
    assert_eq!(first.names.len, 1);

    first.abort(first_key).expect("first abort");
    second.abort(second_key).expect("second abort");
}

#[test]
fn stale_lane_and_name_coordinates_are_rejected_after_abort_and_reuse() {
    let mut work = ExpansionWork::<()>::default();
    let first_key = work.begin_dispatch(empty_command()).expect("first root");
    let stale_command = root_command(&work, &first_key);
    let stale_control = work
        .push_control(ExpansionControl::Dispatch {
            command: stale_command,
            trace: TraceState::Unseen,
        })
        .expect("first child control");
    let stale_name = work.name_mark().expect("first name mark");
    work.push_name_byte(b'a').expect("first name byte");
    work.abort(first_key).expect("first abort");

    let replacement = work
        .begin_dispatch(empty_command())
        .expect("replacement root");
    let replacement_command = root_command(&work, &replacement);
    let replacement_control = work
        .push_control(ExpansionControl::Dispatch {
            command: replacement_command,
            trace: TraceState::Complete,
        })
        .expect("replacement child control");
    let replacement_name = work.name_mark().expect("replacement name mark");
    work.push_name_byte(b'b').expect("replacement name byte");

    assert_eq!(stale_command.lane.index(), replacement_command.lane.index());
    assert_eq!(stale_control.lane.index(), replacement_control.lane.index());
    assert_eq!(stale_name.offset, replacement_name.offset);
    assert_ne!(stale_name.root_serial, replacement_name.root_serial);
    assert!(matches!(
        work.command(stale_command),
        Err(ScratchError::InvalidCoordinate)
    ));
    assert_eq!(
        work.take_command(stale_command),
        Err(ScratchError::InvalidCoordinate)
    );
    assert!(matches!(
        work.control_mut(stale_control),
        Err(ScratchError::InvalidCoordinate)
    ));
    assert_eq!(
        work.pop_control(stale_control),
        Err(ScratchError::InvalidCoordinate)
    );
    assert!(matches!(
        work.name_bytes(stale_name),
        Err(ScratchError::InvalidCoordinate)
    ));

    assert!(work.command(replacement_command).is_ok());
    assert!(work.control_mut(replacement_control).is_ok());
    assert_eq!(
        work.name_bytes(replacement_name)
            .expect("replacement name remains live")
            .collect::<Vec<_>>(),
        b"b"
    );
    work.abort(replacement).expect("replacement abort");
}

#[test]
fn move_only_external_destination_restores_the_exact_key_and_route() {
    #[derive(Debug, Eq, PartialEq)]
    struct CollectorOrdinary;

    let mut work = ExpansionWork::<()>::default();
    let key = work.begin_dispatch(empty_command()).expect("root");
    let owned = OwnedExpansionWork::new(key, CollectorOrdinary);
    let (key, destination) = owned.restore();
    assert_eq!(destination, CollectorOrdinary);
    work.abort(key).expect("abort restored owner");
}

#[test]
fn typed_child_restores_only_its_exact_control_and_destination() {
    let mut work = ExpansionWork::<()>::default();
    let key = work.begin_dispatch(empty_command()).expect("root");
    let command = work.park_command(empty_command()).expect("child command");
    let control = work
        .push_control(ExpansionControl::Dispatch {
            command,
            trace: TraceState::Unseen,
        })
        .expect("child control");
    let child = ExpansionChild::new(control, CsNameTokenDestination);
    let (restored, destination) = child.restore();
    assert_eq!(restored, control);
    assert_eq!(destination, CsNameTokenDestination);
    work.abort(key).expect("abort");
}

#[test]
fn generation_owner_is_checked_before_a_foreign_key_can_retire_work() {
    struct FirstGeneration;
    struct SecondGeneration;

    fn belongs_to_first(_: &ExpansionWorkKey<FirstGeneration>) {}
    fn belongs_to_second(_: &ExpansionWorkKey<SecondGeneration>) {}

    let mut first = ExpansionWork::<FirstGeneration>::default();
    let first_key = first.begin_dispatch(empty_command()).expect("first root");
    belongs_to_first(&first_key);

    let mut second = ExpansionWork::<SecondGeneration>::default();
    let second_key = second.begin_dispatch(empty_command()).expect("second root");
    belongs_to_second(&second_key);

    // The two calls above are the executable positive side of the compile-time
    // brand boundary: neither key type is accepted by the other function. The
    // dynamic owner check is independently exercised for two work owners with
    // the same brand in the ABA/foreign-owner test.
    first.abort(first_key).expect("first abort");
    second.abort(second_key).expect("second abort");
}

#[test]
fn name_lane_crosses_chunks_and_reuses_retained_capacity() {
    let mut work = ExpansionWork::<()>::default();
    let run = |work: &mut ExpansionWork<()>| {
        let key = work.begin_dispatch(empty_command()).expect("root");
        let mark = work.name_mark().expect("name mark");
        for index in 0..(NAME_BYTES_PER_CHUNK * 3 + 7) {
            work.push_name_byte((index % 251) as u8).expect("name byte");
        }
        let bytes = work
            .name_bytes(mark)
            .expect("name suffix")
            .collect::<Vec<_>>();
        assert_eq!(bytes.len(), NAME_BYTES_PER_CHUNK * 3 + 7);
        work.abort(key).expect("abort");
    };
    run(&mut work);
    let retained_chunks = work.names.chunks.len();
    run(&mut work);
    assert_eq!(work.names.chunks.len(), retained_chunks);
    assert!(work.is_quiescent());
}

#[test]
fn capacity_failure_rolls_back_partially_parked_root_atomically() {
    let mut work = ExpansionWork::<()>::default();
    work.controls.next_serial = u32::MAX;
    let before = work.mark();
    assert_eq!(
        work.begin_dispatch(empty_command()),
        Err(ScratchError::CapacityOverflow)
    );
    assert_eq!(work.mark(), before);
    assert!(work.is_quiescent());
    assert_eq!(work.counters().command_moves_in, 1);

    work.names.len = u32::MAX;
    assert_eq!(
        work.push_name_byte(b'x'),
        Err(ScratchError::CapacityOverflow)
    );
    assert_eq!(work.names.len, u32::MAX);
}

#[test]
fn failed_production_park_restores_the_exact_pending_owner() {
    let mut work = ExpansionWork::<()>::default();
    work.controls.next_serial = u32::MAX;
    let ownership_before = crate::command::command_ownership_counters();
    let pending = crate::state::PendingExpansion {
        command: empty_command(),
        resume: crate::state::PendingExpansionResume::PdfInsertHeight,
        child: None,
    };
    let (error, pending) = work
        .park_suspension(pending)
        .expect_err("exhausted control serial rejects park");
    assert_eq!(error, ScratchError::CapacityOverflow);
    assert_eq!(
        pending.resume,
        crate::state::PendingExpansionResume::PdfInsertHeight
    );
    assert!(pending.child.is_none());
    assert_eq!(pending.command, empty_command());
    assert!(work.is_quiescent());
    let ownership_after = crate::command::command_ownership_counters();
    assert_eq!(ownership_after.clones - ownership_before.clones, 0);
    assert_eq!(
        ownership_after.expansion_moves_in - ownership_before.expansion_moves_in,
        1
    );
    assert_eq!(
        ownership_after.expansion_moves_out - ownership_before.expansion_moves_out,
        1
    );
}

#[test]
fn nested_suspensions_are_lifo_and_reject_an_out_of_order_key_without_mutation() {
    let mut work = ExpansionWork::<()>::default();
    let outer = work
        .park_suspension(crate::state::PendingExpansion {
            command: empty_command(),
            resume: crate::state::PendingExpansionResume::The,
            child: None,
        })
        .expect("outer suspension");
    let outer_duplicate = duplicate_key(&outer);
    let inner = work
        .park_suspension(crate::state::PendingExpansion {
            command: empty_command(),
            resume: crate::state::PendingExpansionResume::PdfInsertHeight,
            child: None,
        })
        .expect("inner suspension");

    assert_eq!(
        work.resume_suspension(outer_duplicate),
        Err(ScratchError::InvalidCoordinate)
    );
    assert_eq!(work.active_roots.len(), 2);
    let inner = work.resume_suspension(inner).expect("inner resumes first");
    assert_eq!(
        inner.resume,
        crate::state::PendingExpansionResume::PdfInsertHeight
    );
    let outer = work.resume_suspension(outer).expect("outer resumes second");
    assert_eq!(outer.resume, crate::state::PendingExpansionResume::The);
    assert!(work.is_quiescent());
    assert_eq!(work.counters().stale_key_rejections, 1);
}

#[test]
fn parking_and_consuming_macro_command_clones_no_command_or_definition_owner() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'M',
                    cat: tex_state::token::Catcode::Letter,
                })],
            )
            .expect("definition");
        let symbol = universe.intern("parkedmacro").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, definition),
                AssignmentScope::Global,
            )
            .expect("macro meaning");
        let command = CurrentCommand::resolve(
            TracedTokenWord::pack(Token::Cs(symbol.symbol()), OriginId::UNKNOWN),
            crate::DeliveryStamp::new(1, 2),
            None,
            false,
            None,
            &universe.command_context().expect("command context"),
        );
        assert!(matches!(
            command.meaning_ref(),
            ResolvedMeaning::Macro { .. }
        ));
        let owner_before = tex_state::definition_retain_count();
        let commands_before = crate::command::command_ownership_counters();

        let mut work = ExpansionWork::default();
        let key = work.begin_dispatch(command).expect("park command");
        let slot = root_command(&work, &key);
        let command = work.take_command(slot).expect("consume command");
        work.finish(key).expect("finish root");

        let commands_after = crate::command::command_ownership_counters();
        assert_eq!(tex_state::definition_retain_count(), owner_before);
        assert_eq!(commands_after.clones - commands_before.clones, 0);
        assert_eq!(
            commands_after.expansion_moves_in - commands_before.expansion_moves_in,
            1
        );
        assert_eq!(
            commands_after.expansion_moves_out - commands_before.expansion_moves_out,
            1
        );
        assert!(matches!(
            command.meaning_ref(),
            ResolvedMeaning::Macro { .. }
        ));
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_nested_work_and_name_reuse_allocate_zero_heap() {
    fn run(work: &mut ExpansionWork<()>) {
        let key = work.begin_dispatch(empty_command()).expect("root");
        for _ in 0..128 {
            let command = work.park_command(empty_command()).expect("command");
            work.push_control(ExpansionControl::Dispatch {
                command,
                trace: TraceState::Complete,
            })
            .expect("control");
        }
        for _ in 0..4_096 {
            work.push_name_byte(b'n').expect("name byte");
        }
        work.abort(key).expect("abort");
    }

    let mut work = ExpansionWork::default();
    run(&mut work);
    let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        run(&mut work);
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(work.counters().command_clones, 0);
    assert_eq!(work.counters().whole_control_copies, 0);
}

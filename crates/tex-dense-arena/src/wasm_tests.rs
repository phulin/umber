use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

type Wide = [u64; 1_024];

#[wasm_bindgen_test]
fn direct_lookup_and_bounded_fork_run_in_wasm() {
    let mut arena = GenerationArena::<Wide>::default();
    let mut checkpoint = None;
    for index in 0..20 {
        if index == 11 {
            checkpoint = Some(arena.cursor());
        }
        let mut value = [0; 1_024];
        value[0] = index;
        arena
            .push_with(|slot| slot.insert(value))
            .expect("wasm generation push");
    }
    let fork = arena
        .fork(checkpoint.expect("wasm checkpoint"))
        .expect("wasm fork");
    assert_eq!(fork.shape().shared_complete_blocks, 1);
    assert_eq!(fork.metrics().fork_tail_values_copied, 3);
    assert_eq!(fork.metrics().fork_tail_bytes_copied, 24_576);
    assert_eq!(fork.candidate_get(10).expect("wasm lookup")[0], 10);
    let settled = fork.accept().expect("wasm accept");
    assert_eq!(settled.metrics().accepted_payload_copies, 0);
}

#[wasm_bindgen_test]
fn logical_reuse_and_prepared_transfer_run_in_wasm() {
    let mut store = BlockStore::<Wide>::new();
    let mut table = AcceptedBlockTable::new();
    let mut retained = None;
    for index in 0..8 {
        let mut value = [0; 1_024];
        value[0] = index;
        retained = Some(
            table
                .push_with(&mut store, |slot| slot.insert(value))
                .expect("wasm logical push"),
        );
    }
    let boundary = table.rotate_tail().expect("wasm rotation");
    let mut moved = [0; 1_024];
    moved[0] = 99;
    let moved_position = table
        .push_with(&mut store, |slot| slot.insert(moved))
        .expect("wasm moved push");
    let owner = table
        .seal_rotated_suffix(boundary)
        .unwrap_or_else(|(error, _)| panic!("wasm seal: {error}"));
    let destination = table.empty_block_owner();
    let (_source, loan, _receipt) = owner
        .detach_suffix(0)
        .unwrap_or_else(|(error, _)| panic!("wasm detach: {error}"));
    let prepared = prepare_block_range_transfer(destination, loan)
        .unwrap_or_else(|failure| panic!("wasm prepare: {}", failure.error()));
    let destination = prepared.commit();
    assert_eq!(destination.len(), 1);
    let view = table.view(&store);
    assert_eq!(
        view.get(retained.expect("retained")).expect("retained")[0],
        7
    );
    assert_eq!(view.get(moved_position).expect("moved")[0], 99);
}

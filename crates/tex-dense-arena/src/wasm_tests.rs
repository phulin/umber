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

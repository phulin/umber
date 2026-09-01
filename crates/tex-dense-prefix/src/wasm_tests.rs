use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

#[wasm_bindgen_test]
fn exact_allocation_and_dense_prefix_run_in_wasm() {
    let before = SubstrateMetrics::snapshot();
    let mut block = Superblock::<[u8; 168]>::try_new().expect("wasm superblock");
    block
        .push_with(|slot| slot.insert([7; 168]))
        .expect("wasm push");
    block
        .extend_copy_from_slice(&[[9; 168], [11; 168]])
        .expect("wasm copy range");
    assert_eq!(block.len(), 3);
    assert_eq!(block.get(2).expect("wasm resident")[0], 11);
    assert_eq!(Superblock::<[u8; 168]>::capacity(), 390);
    assert_eq!(Superblock::<[u8; 168]>::tail_slack_bytes(), 16);
    let change = SubstrateMetrics::snapshot() - before;
    assert_eq!(change.allocation_attempts, 1);
    assert_eq!(change.requested_bytes, SUPERBLOCK_BYTES as u64);
}

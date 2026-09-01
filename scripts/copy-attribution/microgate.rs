use std::ffi::c_void;
use std::hint::black_box;

unsafe extern "C" {
    fn memcpy(destination: *mut c_void, source: *const c_void, size: usize) -> *mut c_void;
    fn external_memmove(
        destination: *mut c_void,
        source: *const c_void,
        size: usize,
    ) -> *mut c_void;
    fn external_only_thread_gate() -> u8;
}

#[inline(never)]
fn scalar_copy_gate(value: u64) -> u64 {
    let source = black_box(value);
    let mut destination = 0_u64;
    unsafe {
        memcpy(
            (&mut destination as *mut u64).cast(),
            (&source as *const u64).cast(),
            black_box(size_of::<u64>()),
        );
    }
    black_box(destination)
}

#[inline(never)]
fn vec_copy_gate(seed: u8) -> usize {
    let source = black_box(vec![seed; 257]);
    let mut destination = Vec::with_capacity(source.len());
    destination.extend_from_slice(black_box(&source));
    black_box(destination).len()
}

#[inline(never)]
fn external_memmove_ancestor_gate(seed: u8) -> u8 {
    let mut bytes = [seed; 64];
    unsafe {
        external_memmove(bytes.as_mut_ptr().add(1).cast(), bytes.as_ptr().cast(), 63);
    }
    black_box(bytes[1])
}

fn main() {
    let mut result = 0_u64;
    for value in 0..17 {
        result ^= scalar_copy_gate(value);
    }
    for seed in 0..11 {
        result ^= vec_copy_gate(seed) as u64;
    }
    for seed in 0..7 {
        result ^= u64::from(external_memmove_ancestor_gate(seed));
    }
    result ^= u64::from(unsafe { external_only_thread_gate() });
    println!("{result}");
}

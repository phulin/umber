use std::hint::black_box;
use std::time::{Duration, Instant};

use tex_state::{DependencyKey, DependencyValue, Universe};

const ITERATIONS: u64 = 2_000_000;
const SAMPLES: usize = 12;

fn main() {
    let mut universe = Universe::new();
    warm_up(&mut universe);

    let mut controls = Vec::with_capacity(SAMPLES);
    let mut disabled = Vec::with_capacity(SAMPLES);
    let mut enabled = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        match sample % 3 {
            0 => measure_order(&mut universe, &mut controls, &mut disabled, &mut enabled, [0, 1, 2]),
            1 => measure_order(&mut universe, &mut controls, &mut disabled, &mut enabled, [1, 2, 0]),
            _ => measure_order(&mut universe, &mut controls, &mut disabled, &mut enabled, [2, 0, 1]),
        }
    }

    controls.sort_unstable();
    disabled.sort_unstable();
    enabled.sort_unstable();
    let control = controls[SAMPLES / 2];
    let disabled = disabled[SAMPLES / 2];
    let enabled = enabled[SAMPLES / 2];
    println!("iterations={ITERATIONS} samples={SAMPLES}");
    println!("control_ns_per_read={:.3}", per_read(control));
    println!("disabled_ns_per_read={:.3}", per_read(disabled));
    println!("disabled_incremental_ns={:.3}", per_read(disabled.saturating_sub(control)));
    println!("enabled_ns_per_read={:.3}", per_read(enabled));
}

fn warm_up(universe: &mut Universe) {
    black_box(run_control(ITERATIONS / 10));
    black_box(run_disabled(universe, ITERATIONS / 10));
    black_box(run_enabled(universe, ITERATIONS / 10));
}

fn measure_order(
    universe: &mut Universe,
    controls: &mut Vec<Duration>,
    disabled: &mut Vec<Duration>,
    enabled: &mut Vec<Duration>,
    order: [u8; 3],
) {
    for kind in order {
        match kind {
            0 => controls.push(run_control(ITERATIONS)),
            1 => disabled.push(run_disabled(universe, ITERATIONS)),
            2 => enabled.push(run_enabled(universe, ITERATIONS)),
            _ => unreachable!(),
        }
    }
}

fn run_control(iterations: u64) -> Duration {
    let started = Instant::now();
    for index in 0..iterations {
        black_box((DependencyKey::Meaning(index as u32), DependencyValue::Integer(index as i64)));
    }
    started.elapsed()
}

fn run_disabled(universe: &mut Universe, iterations: u64) -> Duration {
    let started = Instant::now();
    for index in 0..iterations {
        black_box(&mut *universe).record_dependency(
            black_box(DependencyKey::Meaning(index as u32)),
            black_box(DependencyValue::Integer(index as i64)),
        );
    }
    started.elapsed()
}

fn run_enabled(universe: &mut Universe, iterations: u64) -> Duration {
    universe.begin_dependency_region();
    let started = Instant::now();
    for index in 0..iterations {
        // A small repeating set models the deterministic deduplication expected
        // inside a scanner or executor dispatch region.
        let index = index as u32 & 31;
        black_box(&mut *universe).record_dependency(
            black_box(DependencyKey::Meaning(index)),
            black_box(DependencyValue::Integer(i64::from(index))),
        );
    }
    let elapsed = started.elapsed();
    black_box(universe.finish_dependency_region());
    elapsed
}

fn per_read(duration: Duration) -> f64 {
    duration.as_nanos() as f64 / ITERATIONS as f64
}

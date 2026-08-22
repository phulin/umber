use std::{
    alloc::System,
    env,
    hint::black_box,
    process::ExitCode,
    time::{Duration, Instant},
};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_exec_benchmarks::{BatchResult, Workload, run_production};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let calls = parse_usize(args.next(), "calls")?;
    let padding = args
        .next()
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| format!("invalid relax padding: {error}"))?
        .unwrap_or(0);
    let shape = args.next().unwrap_or_else(|| "direct".to_owned());
    if args.next().is_some() {
        return Err(usage());
    }
    let workload = match shape.as_str() {
        "direct" => Workload::new(calls, padding),
        "nested" => Workload::nested(calls, padding),
        _ => return Err(format!("invalid workload shape {shape:?}")),
    };
    let (result, elapsed, stats) = measure(|| run_production(&workload))?;
    enforce_allocation_ceiling(&workload, stats)?;
    print_result(&shape, &workload, &result, elapsed, stats);
    Ok(())
}

fn enforce_allocation_ceiling(workload: &Workload, stats: Stats) -> Result<(), String> {
    let calls = workload.calls();
    let allocation_ceiling = 10_000_usize.saturating_add(calls.saturating_mul(64));
    if stats.allocations > allocation_ceiling {
        return Err(format!(
            "canonical hot-path allocation ceiling exceeded: calls={calls} allocations={} ceiling={allocation_ceiling}",
            stats.allocations
        ));
    }
    Ok(())
}

// This process-isolated benchmark measures the host rather than TeX time.
#[allow(clippy::disallowed_methods)]
fn measure<E>(
    run: impl FnOnce() -> Result<BatchResult, E>,
) -> Result<(BatchResult, Duration, Stats), String>
where
    E: std::fmt::Display,
{
    let region = Region::new(GLOBAL);
    let start = Instant::now();
    let result = run().map_err(|error| error.to_string())?;
    let elapsed = start.elapsed();
    let stats = region.change();
    black_box(&result);
    Ok((result, elapsed, stats))
}

fn print_result(
    shape: &str,
    workload: &Workload,
    result: &BatchResult,
    elapsed: Duration,
    stats: Stats,
) {
    let work = result.command_work.unwrap_or_default();
    println!(
        "shape={shape} calls={} padding={} elapsed_ns={} allocations={} bytes_allocated={} max_rss_kib={} fuel={} raw_steps={} expanded={} lookups={} nodes={} artifact_bytes={} dvi_bytes={}",
        workload.calls(),
        workload.relax_padding(),
        elapsed.as_nanos(),
        stats.allocations,
        stats.bytes_allocated,
        max_rss_kib().unwrap_or(0),
        work.fuel_charges,
        work.token_frame_steps,
        work.expanded_deliveries,
        work.meaning_lookups,
        result.calls * 2,
        result.artifact_bytes.len(),
        result.dvi.len(),
    );
}

// Peak process RSS is intentionally a host-side benchmark observation.
#[allow(clippy::disallowed_methods)]
fn max_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")?
            .split_ascii_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn parse_usize(value: Option<String>, name: &str) -> Result<usize, String> {
    value
        .ok_or_else(usage)?
        .parse()
        .map_err(|error| format!("invalid {name}: {error}"))
}

fn usage() -> String {
    "usage: canonical_episode <calls> [relax-padding] [direct|nested]".to_owned()
}

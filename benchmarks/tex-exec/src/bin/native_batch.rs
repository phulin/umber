use std::{
    alloc::System,
    env,
    hint::black_box,
    process::ExitCode,
    time::{Duration, Instant},
};

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, Stats, StatsAlloc};
use tex_exec_benchmarks::{BatchResult, Workload, run_production, run_shared};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[derive(Clone, Copy)]
enum Engine {
    Production,
    Shared,
    Compare,
}

impl Engine {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "production" => Some(Self::Production),
            "shared" => Some(Self::Shared),
            "compare" => Some(Self::Compare),
            _ => None,
        }
    }
}

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
    let engine = args
        .next()
        .as_deref()
        .and_then(Engine::parse)
        .ok_or_else(usage)?;
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
    match engine {
        Engine::Production => {
            let (result, elapsed, stats) = measure(|| run_production(&workload))?;
            print_result("production", &shape, &workload, &result, elapsed, stats);
        }
        Engine::Shared => {
            let (result, elapsed, stats) = measure(|| run_shared(&workload))?;
            print_result("shared", &shape, &workload, &result, elapsed, stats);
        }
        Engine::Compare => {
            let production = run_production(&workload).map_err(|error| error.to_string())?;
            let shared = run_shared(&workload).map_err(|error| error.to_string())?;
            let mut expected = production.clone();
            expected.command_work = None;
            if shared != expected {
                return Err("shared result diverged from canonical stepping".to_owned());
            }
            println!(
                "compare exact=true shape={} calls={} artifact_bytes={} dvi_bytes={} fuel={}",
                shape,
                calls,
                production
                    .artifact
                    .to_bytes()
                    .map_err(|error| error.to_string())?
                    .len(),
                production.dvi.len(),
                production.command_work.map_or(0, |work| work.fuel_charges)
            );
        }
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
    engine: &str,
    shape: &str,
    workload: &Workload,
    result: &BatchResult,
    elapsed: Duration,
    stats: Stats,
) {
    let work = result.command_work.unwrap_or_default();
    println!(
        "engine={engine} shape={shape} calls={} padding={} elapsed_ns={} allocations={} bytes_allocated={} max_rss_kib={} fuel={} raw_steps={} expanded={} lookups={} nodes={} artifact_bytes={} dvi_bytes={}",
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
    "usage: native_batch <production|shared|compare> <calls> [relax-padding] [direct|nested]"
        .to_owned()
}

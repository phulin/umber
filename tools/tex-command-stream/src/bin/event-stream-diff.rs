//! Exhaustive alignment and root-site grouping for two already-captured streams.

#![allow(
    clippy::disallowed_methods,
    reason = "this read-only host diagnostic consumes caller-selected captured streams"
)]

use std::fs;
use std::process::ExitCode;

use tex_command_stream::{AlignmentTuning, Divergence, ObservedEvent, find_divergences, group};
use tex_oracle::ObservationStream;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("event-stream-diff: {error}");
            ExitCode::from(3)
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let expected_path = arguments.next().ok_or_else(usage)?;
    let actual_path = arguments.next().ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }
    let expected_bytes = fs::read(&expected_path)
        .map_err(|error| format!("read expected stream {expected_path}: {error}"))?;
    let actual_bytes = fs::read(&actual_path)
        .map_err(|error| format!("read actual stream {actual_path}: {error}"))?;
    let expected = ObservationStream::from_canonical_json_lines(&expected_bytes)
        .map_err(|error| format!("parse expected stream {expected_path}: {error}"))?;
    let actual = ObservationStream::from_canonical_json_lines(&actual_bytes)
        .map_err(|error| format!("parse actual stream {actual_path}: {error}"))?;
    let actual = actual
        .events
        .into_iter()
        .map(|event| ObservedEvent::new(event.semantic, String::new()))
        .collect::<Vec<_>>();
    let comparison = find_divergences(
        "captured-stream",
        &expected.events,
        &actual,
        usize::MAX,
        AlignmentTuning::default(),
    );
    let divergences = comparison
        .entries
        .into_iter()
        .map(Box::new)
        .map(Divergence::Mismatch)
        .collect::<Vec<_>>();
    let sites = group(&divergences);
    println!(
        "{} ordered divergence(s), {} root site(s), budget_reached={}",
        divergences.len(),
        sites.len(),
        comparison.budget_reached
    );
    for (index, site) in sites.iter().enumerate() {
        println!(
            "\n[{index}] recurrence={} cascade={}\n{}",
            site.count(),
            site.suppressed_cascade(),
            site.representative()
        );
    }
    Ok(())
}

fn usage() -> String {
    "usage: event-stream-diff EXPECTED.jsonl ACTUAL.jsonl".into()
}

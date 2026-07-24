use std::env;
use std::fs;
use std::process::ExitCode;

use tex_oracle::{CommittedFixture, ObservationStream};

#[allow(
    clippy::disallowed_methods,
    reason = "this host-only live-reference validator reads a detached trace, not engine state"
)]
fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(first) = arguments.next() else {
        eprintln!("usage: tex-oracle-validate TRACE.jsonl | --fixture DIRECTORY");
        return ExitCode::from(2);
    };
    let (fixture, path) = if first == "--fixture" {
        let Some(path) = arguments.next() else {
            eprintln!("usage: tex-oracle-validate --fixture DIRECTORY");
            return ExitCode::from(2);
        };
        (true, path)
    } else {
        (false, first)
    };
    if arguments.next().is_some() {
        eprintln!("usage: tex-oracle-validate TRACE.jsonl | --fixture DIRECTORY");
        return ExitCode::from(2);
    }
    if fixture {
        return match CommittedFixture::load(&path) {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("invalid oracle fixture {}: {error}", path.to_string_lossy());
                ExitCode::FAILURE
            }
        };
    }
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read {}: {error}", path.to_string_lossy());
            return ExitCode::FAILURE;
        }
    };
    match ObservationStream::from_canonical_json_lines(&bytes) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("invalid oracle stream {}: {error}", path.to_string_lossy());
            ExitCode::FAILURE
        }
    }
}

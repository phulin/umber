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
        usage();
        return ExitCode::from(2);
    };
    let (fixture, path) = if first == "--fixture" {
        let Some(path) = arguments.next() else {
            usage();
            return ExitCode::from(2);
        };
        (true, path)
    } else {
        (false, first)
    };
    if fixture {
        let mut semantic_matrix = None;
        let mut audit_matrix = None;
        while let Some(flag) = arguments.next() {
            let Some(value) = arguments.next() else {
                usage();
                return ExitCode::from(2);
            };
            if flag == "--semantic-matrix" && semantic_matrix.is_none() {
                semantic_matrix = Some(value);
            } else if flag == "--audit-matrix" && audit_matrix.is_none() {
                audit_matrix = Some(value);
            } else {
                usage();
                return ExitCode::from(2);
            }
        }
        if semantic_matrix.is_some() != audit_matrix.is_some() {
            usage();
            return ExitCode::from(2);
        }
        return match CommittedFixture::load(&path) {
            Ok(fixture) => {
                if let (Some(semantic), Some(audit)) = (semantic_matrix, audit_matrix) {
                    let semantic_bytes = match fs::read(&semantic) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            eprintln!("failed to read {}: {error}", semantic.to_string_lossy());
                            return ExitCode::FAILURE;
                        }
                    };
                    let audit_bytes = match fs::read(&audit) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            eprintln!("failed to read {}: {error}", audit.to_string_lossy());
                            return ExitCode::FAILURE;
                        }
                    };
                    if let Err(error) = fixture.audit_matrices(&semantic_bytes, &audit_bytes) {
                        eprintln!("invalid oracle fixture audit: {error}");
                        return ExitCode::FAILURE;
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("invalid oracle fixture {}: {error}", path.to_string_lossy());
                ExitCode::FAILURE
            }
        };
    }
    if arguments.next().is_some() {
        usage();
        return ExitCode::from(2);
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

fn usage() {
    eprintln!(
        "usage: tex-oracle-validate TRACE.jsonl | --fixture DIRECTORY \
         [--semantic-matrix PATH --audit-matrix PATH]"
    );
}

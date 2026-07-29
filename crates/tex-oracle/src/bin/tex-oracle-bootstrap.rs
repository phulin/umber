use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use tex_oracle::bootstrap_tex82_fixture;

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    let Some(template) = required(&mut arguments, "--template") else {
        usage();
        return ExitCode::from(2);
    };
    let Some(live_directory) = required(&mut arguments, "--live-directory") else {
        usage();
        return ExitCode::from(2);
    };
    let Some(semantic_matrix) = required(&mut arguments, "--semantic-matrix") else {
        usage();
        return ExitCode::from(2);
    };
    let Some(event_stream) = required(&mut arguments, "--event-stream") else {
        usage();
        return ExitCode::from(2);
    };
    let Some(build_record) = required(&mut arguments, "--build-record") else {
        usage();
        return ExitCode::from(2);
    };
    let Some(output) = required(&mut arguments, "--output") else {
        usage();
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        usage();
        return ExitCode::from(2);
    }
    match bootstrap_tex82_fixture(
        &template,
        &live_directory,
        &semantic_matrix,
        &event_stream,
        &build_record,
        &output,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("failed to bootstrap canonical fixture: {error}");
            ExitCode::FAILURE
        }
    }
}

fn required(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    flag: &str,
) -> Option<PathBuf> {
    if arguments.next()?.to_string_lossy() == flag {
        arguments.next().map(PathBuf::from)
    } else {
        None
    }
}

fn usage() {
    eprintln!(
        "usage: tex-oracle-bootstrap --template FIXTURE --live-directory DIRECTORY --semantic-matrix MATRIX --event-stream TRACE --build-record RECORD --output EMPTY-DIRECTORY"
    );
}

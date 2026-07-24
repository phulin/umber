use std::process::ExitCode;

fn main() -> ExitCode {
    match tex_command_stream::run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

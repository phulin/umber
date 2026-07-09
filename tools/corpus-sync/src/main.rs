#![allow(clippy::disallowed_methods)] // host-side acquisition tool

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use corpus_sync::{SyncOptions, sync_corpus};

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<()> {
    let mut options = SyncOptions::default();
    let mut args = env::args_os().skip(1);

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--manifest") => {
                let Some(path) = args.next() else {
                    bail!("missing path after --manifest");
                };
                options.manifest_path = PathBuf::from(path);
            }
            Some("--dest") => {
                let Some(path) = args.next() else {
                    bail!("missing path after --dest");
                };
                options.dest_dir = PathBuf::from(path);
            }
            Some("--offline") => options.offline = true,
            Some("--help") | Some("-h") => {
                print_usage();
                return Ok(());
            }
            Some(flag) if flag.starts_with('-') => bail!("unknown option: {flag}"),
            _ => bail!("unexpected positional argument: {}", arg.to_string_lossy()),
        }
    }

    let report = sync_corpus(&options)?;
    for status in report.documents {
        println!("{status}");
    }
    Ok(())
}

fn print_usage() {
    eprintln!(
        "usage: corpus-sync [--manifest tests/corpus-manifest.toml] [--dest third_party/corpus] [--offline]"
    );
}

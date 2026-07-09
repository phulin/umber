#![allow(clippy::disallowed_methods)] // Small host-side inspection binary.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use tex_out::dvi::disasm::{DviFile, disassemble_page};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut page = None;
    let mut path = None;
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--page") => {
                let Some(value) = args.next() else {
                    return Err("missing value after --page".to_string());
                };
                let value = value
                    .to_str()
                    .ok_or_else(|| "--page must be valid UTF-8".to_string())?;
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid page number: {value}"))?;
                if parsed == 0 {
                    return Err("--page is 1-based and must be positive".to_string());
                }
                page = Some(parsed - 1);
            }
            Some("--help") | Some("-h") => {
                print_usage();
                return Ok(());
            }
            Some(flag) if flag.starts_with('-') => return Err(format!("unknown option: {flag}")),
            _ => {
                if path.is_some() {
                    return Err("expected exactly one DVI file".to_string());
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }

    let path = path.ok_or_else(|| "missing DVI file".to_string())?;
    let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let file = DviFile::parse(&bytes).map_err(|err| err.to_string())?;
    match page {
        Some(page) => {
            print!(
                "{}",
                disassemble_page(&bytes, page).map_err(|err| err.to_string())?
            );
        }
        None => {
            for page in 0..file.pages.len() {
                print!(
                    "{}",
                    disassemble_page(&bytes, page).map_err(|err| err.to_string())?
                );
            }
        }
    }
    Ok(())
}

fn print_usage() {
    eprintln!("usage: texout-dvitype [--page N] file.dvi");
}

#![allow(clippy::disallowed_methods)] // host tool, not engine code

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use refexec::{DviComparison, RefTex, RunOpts};

fn main() -> ExitCode {
    match run_cli() {
        Ok(success) if success => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<bool> {
    let args = env::args_os().skip(1);
    let mut tex_file = None;
    let mut opts = RunOpts::default();
    let mut print_log = false;
    let mut compare_dvi = None;

    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dvi") => opts.dvi = true,
            Some("--ini") => opts.ini = true,
            Some("--print-log") => print_log = true,
            Some("--compare-dvi") => {
                let Some(path) = args.next() else {
                    bail!("missing path after --compare-dvi");
                };
                compare_dvi = Some(PathBuf::from(path));
                opts.dvi = true;
            }
            Some("--extra-input") => {
                let Some(path) = args.next() else {
                    bail!("missing path after --extra-input");
                };
                opts.extra_inputs.push(PathBuf::from(path));
            }
            Some("--help") | Some("-h") => {
                print_usage();
                return Ok(true);
            }
            Some(flag) if flag.starts_with('-') => bail!("unknown option: {flag}"),
            _ => {
                if tex_file.is_some() {
                    bail!("expected exactly one TeX input file");
                }
                tex_file = Some(PathBuf::from(arg));
            }
        }
    }

    let tex_file = tex_file.ok_or_else(|| anyhow::anyhow!("missing TeX input file"))?;
    let ref_tex = RefTex::locate()?;

    if let Some(actual_path) = compare_dvi {
        let actual = std::fs::read(&actual_path)?;
        return match ref_tex.compare_dvi(&tex_file, &actual, &opts)? {
            DviComparison::Equal => Ok(true),
            DviComparison::Different(diff) => {
                eprintln!("DVI mismatch at byte offset {}", diff.offset);
                eprintln!("reference: {}", diff.expected_context);
                eprintln!("actual:    {}", diff.actual_context);
                Ok(false)
            }
        };
    }

    let output = ref_tex.run(&tex_file, &opts)?;

    print!("{}", output.stdout);
    if print_log {
        print!("{}", output.log);
    }
    if let Some(dvi) = output.dvi {
        let dvi_path = tex_file.with_extension("ref.dvi");
        std::fs::write(&dvi_path, dvi)?;
    }

    Ok(output.success)
}

fn print_usage() {
    eprintln!(
        "usage: refexec <file.tex> [--dvi] [--ini] [--print-log] [--extra-input path] [--compare-dvi path]"
    );
}

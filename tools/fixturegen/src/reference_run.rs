//! Direct reference-process command for diagnostic runs and DVI case staging.
//!
//! The publication command remains `--reference-dvi`; this mode writes only
//! the explicit caller-selected output and never updates fixture authority.

use std::fs;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use fixturegen::reference::{RefTex, RunOpts};

pub(crate) fn run_cli(args: Vec<OsString>) -> Result<()> {
    let mut args = args.into_iter();
    let input = args
        .next()
        .context("--reference-run requires a TeX input path")?;
    if input.to_string_lossy().starts_with('-') {
        bail!("--reference-run requires a TeX input path before options");
    }
    let input = PathBuf::from(input);
    let mut opts = RunOpts::default();
    let mut dvi_output = None;
    let mut print_log = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dvi-output") => {
                let path = args.next().context("missing path after --dvi-output")?;
                if dvi_output.replace(PathBuf::from(path)).is_some() {
                    bail!("--dvi-output specified more than once");
                }
                opts.dvi = true;
            }
            Some("--ini") => opts.ini = true,
            Some("--etex") => opts.etex = true,
            Some("--print-log") => print_log = true,
            Some("--extra-input") => {
                opts.extra_inputs.push(PathBuf::from(
                    args.next().context("missing path after --extra-input")?,
                ));
            }
            _ => bail!("unknown --reference-run option: {}", arg.to_string_lossy()),
        }
    }

    let output = RefTex::locate()?.run(&input, &opts)?;
    print!("{}", output.stdout);
    if print_log {
        print!("{}", output.log);
    }
    if let Some(path) = dvi_output {
        let dvi = output
            .dvi
            .context("reference TeX did not produce the requested DVI")?;
        fs::write(&path, dvi)
            .with_context(|| format!("failed to write reference DVI {}", path.display()))?;
    }
    if !output.success {
        bail!("reference TeX exited unsuccessfully for {}", input.display());
    }
    Ok(())
}

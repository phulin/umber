use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use profile_analyzer::{AnalysisOptions, Entry, Report, analyze_profile};

struct Options {
    profile: PathBuf,
    symbols: Option<PathBuf>,
    analysis: AnalysisOptions,
    json: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("profile-analyzer: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(options) = parse_options()? else {
        return Ok(());
    };
    let (report, symbols) = analyze_profile(
        &options.profile,
        options.symbols.as_deref(),
        &options.analysis,
    )?;
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|error| format!("serialize report: {error}"))?
        );
    } else {
        print_text(&options.profile, symbols.as_deref(), &report);
    }
    Ok(())
}

fn parse_options() -> Result<Option<Options>, String> {
    let mut profile = None;
    let mut symbols = None;
    let mut thread_filter = None;
    let mut app_filter = None;
    let mut subtree = None;
    let mut top = 25;
    let mut json = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--symbols" => symbols = Some(PathBuf::from(next_value(&mut args, "--symbols")?)),
            "--thread" => thread_filter = Some(next_value(&mut args, "--thread")?),
            "--app" => app_filter = Some(next_value(&mut args, "--app")?),
            "--subtree" => subtree = Some(next_value(&mut args, "--subtree")?),
            "--top" => {
                let value = next_value(&mut args, "--top")?;
                top = value
                    .parse::<usize>()
                    .map_err(|_| format!("--top requires a positive integer, got {value:?}"))?;
            }
            "--json" => json = true,
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ if profile.is_none() => profile = Some(PathBuf::from(arg)),
            _ => return Err(format!("unexpected positional argument: {arg}")),
        }
    }
    Ok(Some(Options {
        profile: profile.ok_or_else(|| "a profile path is required".to_owned())?,
        symbols,
        analysis: AnalysisOptions {
            thread_filter,
            app_filter,
            subtree,
            top,
        },
        json,
    }))
}

fn print_text(profile: &std::path::Path, symbols: Option<&std::path::Path>, report: &Report) {
    println!("Profile: {}", profile.display());
    println!(
        "Symbols: {}",
        symbols.map_or_else(
            || "none (raw addresses retained)".to_owned(),
            |path| path.display().to_string()
        )
    );
    println!("Application: {}", report.application_library);
    println!("Threads: {}", report.threads.join(", "));
    println!("Total weighted samples: {:.0}", report.total_weight);
    if let Some(subtree) = &report.subtree {
        println!(
            "Subtree {subtree:?}: {:.0} samples ({:.2}% of total)",
            report.selected_weight, report.selected_percent
        );
    }
    println!("Unresolved raw frames: {}", report.unresolved_frames);
    print_entries("Self time", &report.self_time);
    print_entries("Inclusive time", &report.inclusive_time);
    if report.subtree.is_some() {
        print_entries("Immediate callees", &report.immediate_callees);
    }
    print_entries("Runtime self by library", &report.runtime_by_library);
    print_entries(
        "Runtime self by nearest application caller",
        &report.runtime_callers,
    );
}

fn print_entries(title: &str, entries: &[Entry]) {
    println!("\n{title}");
    println!("  % selected   % total    weight  function");
    for entry in entries {
        println!(
            "  {:>9.2}  {:>8.2}  {:>8.1}  {} [{}]",
            entry.percent_selected,
            entry.percent_total,
            entry.weight,
            entry.function,
            entry.library
        );
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn print_help() {
    println!(
        "Usage: profile-analyzer [OPTIONS] PROFILE.json.gz\n\n\
         Options:\n\
           --symbols PATH   Samply .syms.json sidecar (auto-discovered by default)\n\
           --thread TEXT    Include threads whose name/process contains TEXT\n\
           --app TEXT       Select the application library by name substring\n\
           --subtree TEXT   Restrict attribution to descendants of matching frames\n\
           --top N          Rows per report section (default: 25)\n\
           --json           Emit the report as JSON\n\
           -h, --help       Show this help"
    );
}

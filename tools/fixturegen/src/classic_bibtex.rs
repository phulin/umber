//! Bounded live-reference differential generation for classic BibTeX styles.
//!
//! This module is intentionally only reachable through fixture regeneration.
//! Its unit tests exercise generation and coverage accounting without starting
//! either executable, keeping ordinary Cargo tests hermetic.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use bib_engine::{
    BibExitStatus, BibliographyAttempt, BibliographyHistory, BibliographyJob, BibliographySession,
    ClassicBibJob, ClassicBibOptions, FileKind, FileProvisioner, ResolvedFile,
    VfsLimits, VirtualPath,
};
use tempfile::TempDir;

/// Fixed master seed recorded in the differential coverage contract.
pub const MASTER_SEED: u64 = 0xB1B7_EA5E_D1FF_0001;
/// The generator deliberately emits a small, bounded corpus: one case per
/// classic built-in, with every legal top-level command in each style.
pub const MAX_CASES: usize = BUILTINS.len();
pub const MAX_STYLE_BYTES: usize = 4 * 1024;
pub const MAX_DATABASE_BYTES: usize = 2 * 1024;
pub const MAX_AUX_BYTES: usize = 256;

const COMMAND_BRANCHES: &[&str] = &[
    "command.entry",
    "command.execute",
    "command.function",
    "command.integers",
    "command.iterate",
    "command.macro",
    "command.read",
    "command.reverse",
    "command.sort",
    "command.strings",
];

const STATE_TRANSITIONS: &[&str] = &[
    "state.start->declarations",
    "state.declarations->functions",
    "state.functions->read",
    "state.read->execute",
    "state.execute->iterate",
    "state.iterate->reverse",
    "state.reverse->sort",
    "state.sort->iterate",
    "state.no-current-entry->current-entry",
    "state.current-entry->global-state",
];

const BUILTINS: &[(&str, &str, BuiltinContext)] = &[
    ("=", "#1 #1 = pop$", BuiltinContext::Entry),
    (">", "#2 #1 > pop$", BuiltinContext::Entry),
    ("<", "#1 #2 < pop$", BuiltinContext::Entry),
    ("+", "#1 #2 + pop$", BuiltinContext::Entry),
    ("-", "#3 #1 - pop$", BuiltinContext::Entry),
    ("*", "\"a\" \"b\" * pop$", BuiltinContext::Entry),
    (":=", "#7 'n :=", BuiltinContext::Entry),
    ("add.period$", "\"word\" add.period$ pop$", BuiltinContext::Entry),
    ("call.type$", "call.type$", BuiltinContext::Entry),
    ("change.case$", "\"ABC\" \"l\" change.case$ pop$", BuiltinContext::Entry),
    ("chr.to.int$", "\"A\" chr.to.int$ pop$", BuiltinContext::Entry),
    ("cite$", "cite$ pop$", BuiltinContext::Entry),
    ("duplicate$", "#1 duplicate$ pop$ pop$", BuiltinContext::Entry),
    ("empty$", "\"\" empty$ pop$", BuiltinContext::Entry),
    ("format.name$", "\"Doe, Jane\" #1 \"{ff}\" format.name$ pop$", BuiltinContext::Entry),
    ("if$", "#1 { skip$ } { skip$ } if$", BuiltinContext::Entry),
    ("int.to.chr$", "#65 int.to.chr$ pop$", BuiltinContext::Entry),
    ("int.to.str$", "#42 int.to.str$ pop$", BuiltinContext::Entry),
    ("missing$", "missingfield missing$ pop$", BuiltinContext::Entry),
    ("newline$", "newline$", BuiltinContext::Entry),
    ("num.names$", "\"Doe, Jane and Smith, John\" num.names$ pop$", BuiltinContext::Entry),
    ("pop$", "#1 pop$", BuiltinContext::Entry),
    ("preamble$", "preamble$ pop$", BuiltinContext::Entry),
    ("purify$", "\"{A} -- B\" purify$ pop$", BuiltinContext::Entry),
    ("quote$", "quote$ pop$", BuiltinContext::Entry),
    ("skip$", "skip$", BuiltinContext::Entry),
    ("stack$", "#1 stack$ pop$", BuiltinContext::Entry),
    ("substring$", "\"abcd\" #2 #2 substring$ pop$", BuiltinContext::Entry),
    ("swap$", "#1 #2 swap$ pop$ pop$", BuiltinContext::Entry),
    ("text.length$", "\"{A}B\" text.length$ pop$", BuiltinContext::Entry),
    ("text.prefix$", "\"{A}BC\" #2 text.prefix$ pop$", BuiltinContext::Entry),
    ("top$", "#1 top$ pop$", BuiltinContext::Entry),
    ("type$", "type$ pop$", BuiltinContext::Entry),
    ("warning$", "\"generated warning\" warning$", BuiltinContext::Entry),
    ("while$", "{ #0 } { skip$ } while$", BuiltinContext::Entry),
    ("width$", "\"ABC\" width$ pop$", BuiltinContext::Entry),
    ("write$", "\"builtin-write\" write$", BuiltinContext::Entry),
];

#[derive(Clone, Copy)]
enum BuiltinContext {
    Entry,
}

#[derive(Clone, Debug)]
struct GeneratedCase {
    name: String,
    seed: u64,
    aux: Vec<u8>,
    database: Vec<u8>,
    style: Vec<u8>,
    branch: String,
}

#[derive(Default, Debug)]
struct Coverage {
    branches: BTreeSet<String>,
    transitions: BTreeSet<String>,
}

impl Coverage {
    fn record(&mut self, case: &GeneratedCase) {
        self.branches.extend(
            COMMAND_BRANCHES
                .iter()
                .copied()
                .map(str::to_owned),
        );
        self.branches.insert(case.branch.clone());
        self.transitions.extend(
            STATE_TRANSITIONS
                .iter()
                .copied()
                .map(str::to_owned),
        );
    }

    fn verify(&self) -> Result<()> {
        let required = COMMAND_BRANCHES
            .iter()
            .copied()
            .map(str::to_owned)
            .chain(BUILTINS.iter().map(|(name, _, _)| builtin_branch(name)));
        let missing = required
            .filter(|branch| !self.branches.contains(branch))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            bail!("merged-reference branch coverage is incomplete: {}", missing.join(", "));
        }
        let missing = STATE_TRANSITIONS
            .iter()
            .copied()
            .filter(|transition| !self.transitions.contains(*transition))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            bail!(
                "merged-reference state-transition coverage is incomplete: {}",
                missing.join(", ")
            );
        }
        Ok(())
    }
}

pub fn run(repo_root: &Path, args: Vec<String>) -> Result<()> {
    let options = Options::parse(args)?;
    let cases = generate(options.seed)?;
    let mut coverage = Coverage::default();
    for case in &cases {
        coverage.record(case);
    }
    coverage.verify()?;
    eprintln!(
        "classic BST differential: {} cases, branch coverage {}/{}, state transitions {}/{}",
        cases.len(),
        coverage.branches.len(),
        COMMAND_BRANCHES.len() + BUILTINS.len(),
        coverage.transitions.len(),
        STATE_TRANSITIONS.len(),
    );
    for case in &cases {
        run_case(repo_root, &options, case)?;
    }
    Ok(())
}

struct Options {
    reference: PathBuf,
    texmfcnf: PathBuf,
    seed: u64,
}

impl Options {
    fn parse(args: Vec<String>) -> Result<Self> {
        let mut reference = None;
        let mut texmfcnf = None;
        let mut seed = MASTER_SEED;
        let mut values = args.into_iter();
        while let Some(argument) = values.next() {
            match argument.as_str() {
                "--reference" => reference = Some(PathBuf::from(next(&mut values, &argument)?)),
                "--texmfcnf" => texmfcnf = Some(PathBuf::from(next(&mut values, &argument)?)),
                "--seed" => {
                    let raw = next(&mut values, &argument)?;
                    seed = parse_seed(&raw)?;
                }
                "--help" | "-h" => {
                    bail!("usage: fixturegen --classic-bibtex-differential --reference PATH --texmfcnf PATH [--seed N]")
                }
                _ => bail!("unknown classic BST differential option: {argument}"),
            }
        }
        Ok(Self {
            reference: reference
                .context("missing --reference")?
                .canonicalize()
                .context("resolve --reference")?,
            texmfcnf: texmfcnf
                .context("missing --texmfcnf")?
                .canonicalize()
                .context("resolve --texmfcnf")?,
            seed,
        })
    }
}

fn next(values: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    values.next().with_context(|| format!("missing value after {flag}"))
}

fn parse_seed(raw: &str) -> Result<u64> {
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    u64::from_str_radix(raw, if raw.chars().any(|character| character.is_ascii_alphabetic()) { 16 } else { 10 })
        .with_context(|| format!("invalid differential seed {raw}"))
}

fn generate(master_seed: u64) -> Result<Vec<GeneratedCase>> {
    let cases = BUILTINS
        .iter()
        .enumerate()
        .map(|(index, (builtin, snippet, context))| {
            let seed = mix(master_seed, index as u64);
            let name = format!("bst-diff-{index:02}");
            let title = if seed & 1 == 0 { "Alpha" } else { "Beta" };
            let entry_body = match context {
                BuiltinContext::Entry => *snippet,
            };
            let style = format!(
                "% seed {seed:#018x}; merged-reference branch: {}\n\
                 ENTRY {{ author title year missingfield }} {{}} {{ label }}\n\
                 INTEGERS {{ n }}\n\
                 STRINGS {{ s }}\n\
                 MACRO {{ local }} {{ \"macro\" }}\n\
                 FUNCTION {{ init }} {{ \"init\" 's := #1 'n := }}\n\
                 FUNCTION {{ misc }} {{ skip$ }}\n\
                 FUNCTION {{ article }} {{ {entry_body} \"entry\" write$ newline$ }}\n\
                 READ\n\
                 EXECUTE {{ init }}\n\
                 ITERATE {{ article }}\n\
                 REVERSE {{ article }}\n\
                 SORT\n\
                 ITERATE {{ article }}\n",
                builtin_branch(builtin),
            );
            let aux = format!("\\citation{{alpha,beta}}\n\\bibdata{{{name}}}\n\\bibstyle{{{name}}}\n");
            let database = format!(
                "@preamble{{\"preamble\"}}\n\
                 @misc{{alpha, author = {{Doe, Jane}}, title = {{{title}}}, year = {{2024}}}}\n\
                 @misc{{beta, author = {{Smith, John}}, title = {{Second}}, year = {{2025}}}}\n"
            );
            GeneratedCase {
                name,
                seed,
                aux: aux.into_bytes(),
                database: database.into_bytes(),
                style: style.into_bytes(),
                branch: builtin_branch(builtin),
            }
        })
        .collect::<Vec<_>>();
    if cases.len() != MAX_CASES {
        bail!("generator produced {} cases, bound is {MAX_CASES}", cases.len());
    }
    for case in &cases {
        if case.aux.len() > MAX_AUX_BYTES
            || case.database.len() > MAX_DATABASE_BYTES
            || case.style.len() > MAX_STYLE_BYTES
        {
            bail!("generated case {} exceeded its byte bound", case.name);
        }
    }
    Ok(cases)
}

fn builtin_branch(name: &str) -> String {
    format!("builtin.{name}")
}

fn mix(seed: u64, index: u64) -> u64 {
    let mut value = seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn run_case(repo_root: &Path, options: &Options, case: &GeneratedCase) -> Result<()> {
    let reference_dir = TempDir::new().context("create isolated reference directory")?;
    let umber_dir = TempDir::new().context("create isolated Umber directory")?;
    stage_case(reference_dir.path(), case)?;
    stage_case(umber_dir.path(), case)?;
    let reference = run_reference(reference_dir.path(), options, &case.name)?;
    let umber = run_umber(case)?;
    if reference != umber {
        preserve_failure(repo_root, options, case, reference_dir.path(), umber_dir.path(), &reference, &umber)?;
        bail!(
            "classic BST differential mismatch for {} (seed {:#018x}); preserved target/bst-differential/failures/{}",
            case.name,
            case.seed,
            case.name
        );
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct Output {
    status: i32,
    bbl: Vec<u8>,
    blg: Vec<u8>,
}

fn stage_case(directory: &Path, case: &GeneratedCase) -> Result<()> {
    fs::write(directory.join(format!("{}.aux", case.name)), &case.aux)?;
    fs::write(directory.join(format!("{}.bib", case.name)), &case.database)?;
    fs::write(directory.join(format!("{}.bst", case.name)), &case.style)?;
    Ok(())
}

fn run_reference(directory: &Path, options: &Options, name: &str) -> Result<Output> {
    let status = Command::new(&options.reference)
        .current_dir(directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .env("TEXMFCNF", &options.texmfcnf)
        .env("BIBINPUTS", ".")
        .env("BSTINPUTS", ".")
        .arg(name)
        .output()
        .with_context(|| format!("run pinned reference for {name}"))?
        .status
        .code()
        .unwrap_or(-1);
    Ok(Output {
        status,
        bbl: read_optional(&directory.join(format!("{name}.bbl")))?,
        blg: read_optional(&directory.join(format!("{name}.blg")))?,
    })
}

fn run_umber(case: &GeneratedCase) -> Result<Output> {
    let aux = VirtualPath::user(&format!("{}.aux", case.name)).expect("generated virtual path");
    let mut files = FileProvisioner::new(VfsLimits::default()).context("create Umber VFS")?;
    files
        .register_user(aux.clone(), case.aux.clone())
        .context("register generated AUX")?;
    let job = ClassicBibJob::new(aux, ClassicBibOptions::default());
    let mut session = BibliographySession::classic();
    let result = loop {
        match session.process(&BibliographyJob::Classic(job.clone()), &files.snapshot()) {
            BibliographyAttempt::NeedResources(needs) => {
                files.expect(&needs);
                for request in &needs.required {
                    let bytes = match request.key().kind() {
                        FileKind::ClassicBibData => case.database.clone(),
                        FileKind::BibStyle => case.style.clone(),
                        kind => bail!("unexpected generated resource kind {kind:?}"),
                    };
                    files.provision(ResolvedFile {
                        request: request.key().clone(),
                        virtual_path: format!("/texlive/differential/{}", request.key().name()),
                        bytes,
                        expected_digest: None,
                    })?;
                }
            }
            BibliographyAttempt::Finished(result) => break result,
            attempt => bail!("generated Umber case did not finish: {attempt:?}"),
        }
    };
    let status = match result.history() {
        BibliographyHistory::Spotless | BibliographyHistory::Warning => BibExitStatus::Success,
        BibliographyHistory::Error => BibExitStatus::ClassicExecutionError,
        BibliographyHistory::Fatal => BibExitStatus::OperationalFailure,
    }
    .code() as i32;
    let artifacts = result
        .files()
        .chain(result.partial_files())
        .map(|file| (file.path().as_str().to_owned(), file.bytes().to_vec()))
        .collect::<Vec<_>>();
    Ok(Output {
        status,
        bbl: artifact(&artifacts, ".bbl"),
        blg: artifact(&artifacts, ".blg"),
    })
}

fn artifact(artifacts: &[(String, Vec<u8>)], suffix: &str) -> Vec<u8> {
    artifacts
        .iter()
        .find(|(path, _)| path.ends_with(suffix))
        .map_or_else(Vec::new, |(_, bytes)| bytes.clone())
}

fn read_optional(path: &Path) -> Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn preserve_failure(
    repo_root: &Path,
    options: &Options,
    case: &GeneratedCase,
    reference_dir: &Path,
    umber_dir: &Path,
    reference: &Output,
    umber: &Output,
) -> Result<()> {
    let directory = repo_root.join("target/bst-differential/failures").join(&case.name);
    if directory.exists() {
        fs::remove_dir_all(&directory).with_context(|| format!("replace {}", directory.display()))?;
    }
    fs::create_dir_all(&directory)?;
    copy_case(reference_dir, &directory.join("reference"), &case.name)?;
    copy_case(umber_dir, &directory.join("umber"), &case.name)?;
    fs::write(directory.join("reference.status"), format!("{}\n", reference.status))?;
    fs::write(directory.join("umber.status"), format!("{}\n", umber.status))?;
    fs::write(directory.join("reference.bbl"), &reference.bbl)?;
    fs::write(directory.join("umber.bbl"), &umber.bbl)?;
    fs::write(directory.join("reference.blg"), &reference.blg)?;
    fs::write(directory.join("umber.blg"), &umber.blg)?;
    fs::write(
        directory.join("repro.sh"),
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\ncd {}\nscripts/regen-fixtures.sh --area bibtex\n# deterministic master seed: {:#018x}; failing generated case: {} ({:#018x})\n",
            shell_quote(repo_root), options.seed, case.name, case.seed
        ),
    )?;
    Ok(())
}

fn copy_case(source: &Path, destination: &Path, name: &str) -> Result<()> {
    fs::create_dir_all(destination)?;
    for extension in ["aux", "bib", "bst", "bbl", "blg"] {
        let input = source.join(format!("{name}.{extension}"));
        if input.exists() {
            fs::copy(&input, destination.join(format!("{name}.{extension}")))?;
        }
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\\"'\\\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_generation_is_seed_deterministic_and_complete() {
        let first = generate(MASTER_SEED).expect("generated cases");
        let second = generate(MASTER_SEED).expect("generated cases");
        assert_eq!(first.len(), MAX_CASES);
        assert_eq!(
            first.iter().map(|case| &case.style).collect::<Vec<_>>(),
            second.iter().map(|case| &case.style).collect::<Vec<_>>()
        );
        assert!(first.iter().all(|case| case.style.len() <= MAX_STYLE_BYTES));
        for case in &first {
            run_umber(case).unwrap_or_else(|error| {
                panic!("generated case {} must execute: {error:#}", case.name)
            });
        }
        let mut coverage = Coverage::default();
        for case in &first {
            coverage.record(case);
        }
        coverage.verify().expect("all declared coverage is generated");
        assert_eq!(coverage.branches.len(), COMMAND_BRANCHES.len() + BUILTINS.len());
        assert_eq!(coverage.transitions.len(), STATE_TRANSITIONS.len());
    }
}

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use test_support::{
    CorpusCase, assert_matches_fixture, closed_case::FixtureCase, corpus_cases, dvi, normalize,
    read_binary_fixture,
};
use umber_distribution::{ManifestShard, pack_shard};
use umber_hash::{AHash64, HashDomain};

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[path = "cli/entry_and_cache.rs"]
mod entry_and_cache;
#[path = "cli/execution_limits.rs"]
mod execution_limits;
#[path = "cli/formats.rs"]
mod formats;
#[path = "cli/identity_and_clock.rs"]
mod identity_and_clock;
#[path = "cli/pdf_publication.rs"]
mod pdf_publication;
#[path = "cli/recovery.rs"]
mod recovery;
#[path = "cli/resources.rs"]
mod resources;

struct BibInvocationCase {
    argv: Vec<BibArgument>,
    status: i32,
    inputs: BTreeMap<String, String>,
    staged: BTreeMap<String, String>,
    stdout: String,
    stderr: String,
    outputs: BTreeMap<String, BibOutput>,
}

enum BibArgument {
    Literal(String),
    Input(String),
    Output(String),
}

struct BibOutput {
    actual: String,
    expected: String,
}

struct ResolvedBibInvocation {
    argv: Vec<String>,
    artifact: Option<(PathBuf, String)>,
}

impl BibInvocationCase {
    fn parse(metadata: &str) -> Result<Self, String> {
        let mut lines = metadata.lines();
        if lines.next() != Some("bib-invocation-v2") {
            return Err("invocation metadata must begin with bib-invocation-v2".into());
        }
        let mut argv = Vec::new();
        let mut status = None;
        let mut inputs = BTreeMap::new();
        let mut staged = BTreeMap::new();
        let mut stdout = None;
        let mut stderr = None;
        let mut outputs = BTreeMap::new();
        let mut output_names = BTreeSet::new();
        for line in lines {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("unkeyed invocation metadata: {line:?}"))?;
            match key {
                "arg" => {
                    let (kind, value) = value
                        .split_once(':')
                        .ok_or_else(|| format!("untyped invocation argument: {value:?}"))?;
                    argv.push(match kind {
                        "literal" => {
                            validate_literal_argument(value)?;
                            BibArgument::Literal(value.to_owned())
                        }
                        "input" => BibArgument::Input(checked_role("input", value)?),
                        "output" => BibArgument::Output(checked_role("output", value)?),
                        _ => return Err(format!("unknown invocation argument type: {kind:?}")),
                    });
                }
                "status" => {
                    if status.is_some() {
                        return Err("duplicate status role".into());
                    }
                    status = Some(
                        value
                            .parse()
                            .map_err(|_| format!("invalid invocation status: {value:?}"))?,
                    );
                }
                "input" => {
                    let (role, payload) = value
                        .split_once(':')
                        .ok_or_else(|| format!("input must bind role:payload: {value:?}"))?;
                    let role = checked_role("input", role)?;
                    if inputs.insert(role.clone(), payload.to_owned()).is_some() {
                        return Err(format!("duplicate input role: {role}"));
                    }
                }
                "stage" => {
                    let (name, payload) = value
                        .split_once(':')
                        .ok_or_else(|| format!("stage must bind name:payload: {value:?}"))?;
                    let name = checked_role("stage", name)?;
                    if staged.insert(name.clone(), payload.to_owned()).is_some() {
                        return Err(format!("duplicate staged dependency: {name}"));
                    }
                }
                "stdout" => {
                    if stdout.is_some() {
                        return Err("duplicate stdout role".into());
                    }
                    stdout = Some(value.to_owned());
                }
                "stderr" => {
                    if stderr.is_some() {
                        return Err("duplicate stderr role".into());
                    }
                    stderr = Some(value.to_owned());
                }
                "output" => {
                    let mut fields = value.split(':');
                    let role = checked_role("output", fields.next().ok_or("missing output role")?)?;
                    let actual = fields.next().ok_or("missing output artifact name")?;
                    let expected = fields.next().ok_or("missing expected output role")?;
                    if fields.next().is_some() {
                        return Err(format!("output has excess fields: {value:?}"));
                    }
                    if outputs.contains_key(&role) {
                        return Err(format!("duplicate output role: {role}"));
                    }
                    if !output_names.insert(actual.to_owned()) {
                        return Err(format!("conflicting output artifact name: {actual}"));
                    }
                    outputs.insert(
                        role,
                        BibOutput {
                            actual: actual.to_owned(),
                            expected: expected.to_owned(),
                        },
                    );
                }
                _ => return Err(format!("unknown invocation metadata field: {line}")),
            }
        }
        if argv.is_empty() {
            return Err("invocation argv is empty".into());
        }
        let input_roles = inputs.keys().cloned().collect::<BTreeSet<_>>();
        let output_roles = outputs.keys().cloned().collect::<BTreeSet<_>>();
        if let Some(role) = input_roles.intersection(&output_roles).next() {
            return Err(format!("role is declared as both input and output: {role}"));
        }
        let mut input_uses = BTreeMap::<&str, usize>::new();
        let mut output_uses = BTreeMap::<&str, usize>::new();
        for argument in &argv {
            match argument {
                BibArgument::Literal(_) => {}
                BibArgument::Input(role) => {
                    if !input_roles.contains(role) {
                        if output_roles.contains(role) {
                            return Err(format!("output role used as input argument: {role}"));
                        }
                        return Err(format!("undeclared input argument role: {role}"));
                    }
                    *input_uses.entry(role).or_default() += 1;
                }
                BibArgument::Output(role) => {
                    if !output_roles.contains(role) {
                        if input_roles.contains(role) {
                            return Err(format!("input role used as output argument: {role}"));
                        }
                        return Err(format!("undeclared output argument role: {role}"));
                    }
                    *output_uses.entry(role).or_default() += 1;
                }
            }
        }
        for role in &input_roles {
            if input_uses.get(role.as_str()).copied() != Some(1) {
                return Err(format!(
                    "input role must occur exactly once in argv: {role}"
                ));
            }
        }
        for role in &output_roles {
            if output_uses.get(role.as_str()).copied() != Some(1) {
                return Err(format!(
                    "output role must occur exactly once in argv: {role}"
                ));
            }
        }
        Ok(Self {
            argv,
            status: status.ok_or("missing status role")?,
            inputs,
            staged,
            stdout: stdout.ok_or("missing stdout role")?,
            stderr: stderr.ok_or("missing stderr role")?,
            outputs,
        })
    }

    fn expected_channel(&self, case: &FixtureCase, authority: &str) -> Vec<u8> {
        if authority == "empty" {
            Vec::new()
        } else {
            case.read(authority).expect("declared channel authority")
        }
    }

    #[allow(clippy::disallowed_methods)] // Host-only hermetic fixture staging.
    fn resolve(
        &self,
        case: &FixtureCase,
        workspace: &Path,
    ) -> Result<ResolvedBibInvocation, String> {
        let input_bytes = self
            .inputs
            .iter()
            .map(|(role, payload)| {
                case.read(payload)
                    .map(|bytes| (role.clone(), payload.clone(), bytes))
                    .map_err(|error| format!("invalid input role {role:?}: {error:#}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let staged_bytes = self
            .staged
            .iter()
            .map(|(name, payload)| {
                case.read(payload)
                    .map(|bytes| (name.clone(), payload.clone(), bytes))
                    .map_err(|error| {
                        format!("invalid staged dependency {name:?} ({payload:?}): {error:#}")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (channel, role) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            if role != "empty" {
                case.payload_path(role)
                    .map_err(|error| format!("invalid {channel} role {role:?}: {error:#}"))?;
            }
        }
        let mut input_paths = BTreeMap::new();
        for (role, payload, _) in &input_bytes {
            let path = safe_artifact_path(workspace, payload)?;
            input_paths.insert(role.clone(), path);
        }
        let mut staged_paths = BTreeMap::new();
        for (_, payload, _) in &staged_bytes {
            let path = safe_artifact_path(workspace, payload)?;
            if input_paths.values().any(|input| input == &path) {
                return Err(format!(
                    "staged payload collides with input role: {payload}"
                ));
            }
            staged_paths.insert(payload.clone(), path);
        }
        let mut output_paths = BTreeMap::new();
        for (role, output) in &self.outputs {
            case.payload_path(&output.expected).map_err(|error| {
                format!(
                    "invalid expected output role {:?}: {error:#}",
                    output.expected
                )
            })?;
            if input_bytes
                .iter()
                .any(|(_, payload, _)| payload == &output.actual)
            {
                return Err(format!(
                    "output artifact collides with staged input: {}",
                    output.actual
                ));
            }
            output_paths.insert(role.clone(), safe_artifact_path(workspace, &output.actual)?);
        }
        let argv = self
            .argv
            .iter()
            .map(|argument| match argument {
                BibArgument::Literal(value) => Ok(value.clone()),
                BibArgument::Input(role) => input_paths
                    .get(role)
                    .map(|path| path.to_string_lossy().into_owned())
                    .ok_or_else(|| format!("undeclared input argument role: {role}")),
                BibArgument::Output(role) => output_paths
                    .get(role)
                    .map(|path| path.to_string_lossy().into_owned())
                    .ok_or_else(|| format!("undeclared output argument role: {role}")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.outputs.len() > 1 {
            return Err("bibliography invocation supports at most one output role".into());
        }
        for (role, payload, bytes) in input_bytes {
            fs::write(input_paths.get(&role).expect("validated input path"), bytes)
                .map_err(|error| format!("stage isolated input {payload:?}: {error}"))?;
        }
        for (_, payload, bytes) in staged_bytes {
            fs::write(
                staged_paths.get(&payload).expect("validated staged path"),
                bytes,
            )
            .map_err(|error| format!("stage isolated dependency {payload:?}: {error}"))?;
        }
        let artifact = self.outputs.iter().next().map(|(role, output)| {
            (
                output_paths.get(role).expect("validated output").clone(),
                output.expected.clone(),
            )
        });
        Ok(ResolvedBibInvocation { argv, artifact })
    }
}

fn checked_role(kind: &str, role: &str) -> Result<String, String> {
    if role.is_empty()
        || !role
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "{kind} role is not a normalized identifier: {role:?}"
        ));
    }
    Ok(role.to_owned())
}

fn validate_literal_argument(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    let path_bearing = path.is_absolute()
        || value.contains(['/', '\\'])
        || value == "."
        || value == ".."
        || value.starts_with("./")
        || value.starts_with("../")
        || (!value.starts_with('-') && path.extension().is_some());
    if value.is_empty() || path_bearing {
        return Err(format!(
            "literal argument must not carry filesystem authority: {value:?}"
        ));
    }
    Ok(())
}

fn safe_artifact_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    let mut components = relative.components();
    let Some(Component::Normal(file_name)) = components.next() else {
        return Err(format!(
            "artifact name must be a normalized relative filename: {name:?}"
        ));
    };
    if components.next().is_some()
        || matches!(name, "case.inventory" | "invocation.case" | "{output}")
    {
        return Err(format!(
            "artifact name must be a non-reserved relative filename: {name:?}"
        ));
    }
    let metadata =
        fs::symlink_metadata(root).map_err(|error| format!("inspect output root: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("artifact output root is not a non-symlink directory".into());
    }
    let root = root
        .canonicalize()
        .map_err(|error| format!("canonicalize output root: {error}"))?;
    let output = root.join(file_name);
    if output.parent() != Some(root.as_path()) {
        return Err(format!("artifact output escapes isolated root: {name:?}"));
    }
    if fs::symlink_metadata(&output).is_ok() {
        return Err(format!(
            "artifact output collides with an existing path: {}",
            output.display()
        ));
    }
    Ok(output)
}

#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_corpus_matches_committed_terminal_fixtures(
    area: &str,
    show_fixtures: bool,
    excluded_semantic_cases: &[&str],
) {
    for case in corpus_cases(area) {
        if !excluded_semantic_cases.contains(&case.name()) {
            assert_terminal_case_matches_committed_fixture(area, &case, show_fixtures);
        }
    }
}

#[allow(clippy::disallowed_methods)] // host-side command execution and expected-output reads.
fn assert_terminal_case_matches_committed_fixture(
    area: &str,
    case: &CorpusCase,
    show_fixtures: bool,
) {
    let actual = run_diagnostic_case(case, show_fixtures, false);
    assert_matches_fixture(area, case.name(), "terminal", &actual);
}

#[allow(clippy::disallowed_methods)] // host-side command execution and expected-output reads.
fn assert_log_case_matches_committed_fixture(
    area: &str,
    case: &CorpusCase,
    show_fixtures: bool,
    etex: bool,
) {
    let actual = run_diagnostic_case(case, show_fixtures, etex);
    assert_matches_fixture(area, case.name(), "log", &actual);
}

#[allow(clippy::disallowed_methods)] // host-side command execution.
fn run_diagnostic_case(case: &CorpusCase, show_fixtures: bool, etex: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
    command.env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH);
    if etex {
        command.current_dir(
            case.source_path()
                .parent()
                .expect("corpus source has a parent directory"),
        );
    }
    command.arg("run");
    if etex {
        command.arg("--etex");
    }
    if show_fixtures {
        command.arg("--show-fixtures");
    }
    let output = command
        .arg(case.source_path())
        .output()
        .expect("run umber run");
    assert!(
        output.status.success(),
        "umber run failed for {}:\n{}",
        case.source_path().display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let actual_stdout = String::from_utf8(output.stdout).expect("umber run output is utf-8");
    if show_fixtures {
        normalize::box_dump(&actual_stdout)
    } else {
        normalize::exec_log(&actual_stdout)
    }
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_area_matches_committed_fixture(area: &str) {
    for case in corpus_cases(area) {
        assert_dvi_case_matches_committed_fixture(area, case.name());
    }
}

#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn assert_dvi_case_matches_committed_fixture(area: &str, case: &str) {
    let setup = dvi::DviCaseSetup::new(area, case);

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(setup.run_dir())
        .arg("run")
        .arg(setup.source_file_name())
        .arg("--dvi")
        .arg(setup.actual_dvi_file_name())
        .output()
        .expect("run umber DVI smoke");
    assert!(
        output.status.success(),
        "umber DVI smoke failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let actual = fs::read(setup.actual_dvi_path()).expect("read umber DVI");
    let expected = read_binary_fixture(area, case, "dvi");
    dvi::assert_dvi_matches(&expected, &actual, &format!("{area}/{case}"));
}

fn hex_ahash64(bytes: &[u8]) -> String {
    AHash64::for_bytes(HashDomain::DistributionContent, bytes).hex()
}

#[path = "cli/bibliography.rs"]
mod bibliography;
#[path = "cli/corpus.rs"]
mod corpus;
#[path = "cli/dump_commands.rs"]
mod dump_commands;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};
use test_support::{
    CorpusCase, assert_matches_fixture, closed_case::FixtureCase, corpus_cases, dvi, normalize,
    read_binary_fixture,
};
use tex_state::Universe;

const PINNED_SOURCE_DATE_EPOCH: &str = "1783604160";

#[test]
fn exits_successfully() {
    let status = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .status()
        .expect("failed to run umber binary");

    assert!(status.success());
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the explicit verifier.
fn distribution_verifier_is_an_explicit_positive_and_negative_cache_control() {
    let directory = tempfile::tempdir().expect("cache verification fixture");
    let cache = directory.path().join("cache");
    let store = umber_fetch::BlobStore::new(&cache);
    let bytes = b"explicit verifier fixture";
    let digest = format!("{:x}", Sha256::digest(bytes));
    let spec = umber_fetch::VerifiedBlobSpec::content_addressed(
        "objects",
        &digest,
        bytes.len() as u64,
        bytes.len() as u64,
    )
    .expect("cache object specification");
    store.store(&spec, bytes).expect("cache object");

    let positive = Command::new(env!("CARGO_BIN_EXE_distribution-verify"))
        .args(["--cache", cache.to_str().expect("cache path")])
        .output()
        .expect("run explicit verifier");
    assert!(positive.status.success());
    assert_eq!(
        positive.stdout,
        format!(
            "cache blobs=1 objects=1 manifests=0 other=0 payload_bytes={}\n",
            bytes.len()
        )
        .as_bytes()
    );

    let path = store.entry_path(&spec);
    let mut encoded = fs::read(&path).expect("encoded cache object");
    *encoded.last_mut().expect("payload byte") ^= 1;
    fs::write(&path, encoded).expect("mutate cache object");
    let negative = Command::new(env!("CARGO_BIN_EXE_distribution-verify"))
        .args(["--cache", cache.to_str().expect("cache path")])
        .output()
        .expect("run explicit verifier against corruption");
    assert!(!negative.status.success());
    assert!(
        String::from_utf8_lossy(&negative.stderr).contains("envelope digest"),
        "{}",
        String::from_utf8_lossy(&negative.stderr)
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn reserved_format_worker_invocations_are_owned_before_application_dispatch() {
    let directory = tempfile::tempdir().expect("create isolated worker cache root");
    let cache_home = directory.path().join("cache");
    let run = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .args(arguments)
            .env("XDG_CACHE_HOME", &cache_home)
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .output()
            .expect("run production worker route")
    };

    let malformed = run(&["__format-worker", "trailing"]);
    assert_eq!(malformed.status.code(), Some(70));
    assert!(malformed.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&malformed.stderr),
        "umber format worker: reserved __format-worker invocation accepts no trailing arguments\n"
    );
    assert!(
        !cache_home.exists(),
        "malformed reserved invocation must not initialize the application cache"
    );

    let exact = run(&["__format-worker"]);
    assert_eq!(exact.status.code(), Some(70));
    assert!(String::from_utf8_lossy(&exact.stderr).starts_with("umber format worker: "));
    assert!(
        !cache_home.exists(),
        "exact worker dispatch without a request must not initialize the application cache"
    );

    let unrelated = run(&["__format-worker-unrelated"]);
    assert_eq!(unrelated.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unrelated.stderr).starts_with("umber: "));
    assert!(
        !cache_home.exists(),
        "unrelated ordinary parsing must not initialize the application cache"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn format_cache_cli_stores_restores_and_reports_misses() {
    let directory = tempfile::tempdir().expect("create format cache fixture");
    let closure = directory.path().join("closure.index");
    let source_lock = directory.path().join("source.lock");
    let build_configuration = directory.path().join("build.config");
    fs::write(&closure, b"tex:latex.ltx\n").expect("write closure identity");
    fs::write(&source_lock, b"pinned sources\n").expect("write source lock");
    fs::write(&build_configuration, b"profile=release\n").expect("write build config");
    let format_path = directory.path().join("generated.fmt");
    fs::write(
        &format_path,
        Universe::new().dump_format().expect("schema-11 format"),
    )
    .expect("write format image");
    let cache_root = directory.path().join("cache");

    let common = [
        "--engine",
        "latex",
        "--distribution",
        "texlive-test",
        "--closure",
        closure.to_str().expect("closure path"),
        "--source-lock",
        source_lock.to_str().expect("source lock path"),
        "--build-configuration",
        build_configuration.to_str().expect("build config path"),
        "--cache-root",
        cache_root.to_str().expect("cache root path"),
    ];
    let store = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "store"])
        .args(common)
        .args(["--format", format_path.to_str().expect("format path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("store generated format");
    assert!(store.status.success());
    assert_eq!(store.stdout, b"stored\n");
    assert!(String::from_utf8_lossy(&store.stderr).contains("published generated format"));

    let restored = directory.path().join("restored.fmt");
    let restore = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .args(["--format-out", restored.to_str().expect("restore path")])
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .output()
        .expect("restore generated format");
    assert!(restore.status.success());
    assert_eq!(restore.stdout, b"hit\n");
    assert_eq!(
        fs::read(restored).expect("read restored format"),
        fs::read(format_path).expect("read source format")
    );

    let miss = Command::new(env!("CARGO_BIN_EXE_umber"))
        .args(["format-cache", "restore"])
        .args(common)
        .arg("--format-out")
        .arg(directory.path().join("miss.fmt"))
        .env("SOURCE_DATE_EPOCH", "0")
        .output()
        .expect("probe changed-clock format");
    assert!(miss.status.success());
    assert_eq!(miss.stdout, b"miss\n");
    assert!(!directory.path().join("miss.fmt").exists());
}

#[test]
#[allow(clippy::disallowed_methods)] // CLI boundary intentionally launches the built Umber binary.
fn bib_command_has_exact_native_invocation_outputs_and_statuses() {
    let repository =
        test_support::repository_root_at(&std::env::current_dir().expect("current directory"))
            .expect("runtime repository");
    let area_relative = PathBuf::from("tests/corpus/bib/invocation");
    let area = repository.join(&area_relative);
    let mut names = fs::read_dir(&area)
        .expect("read bibliography invocation cases")
        .map(|entry| {
            let entry = entry.expect("read bibliography invocation case");
            assert!(
                entry.file_type().expect("case file type").is_dir(),
                "bibliography invocation area contains a non-case entry: {}",
                entry.path().display()
            );
            entry.file_name().into_string().expect("UTF-8 case name")
        })
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["bcf-success", "invalid-output-format", "tool-mode",]
    );

    for name in names {
        let case_relative = area_relative.join(&name);
        let case = FixtureCase::discover(&case_relative, "invocation.case", "bib-invocation-v2")
            .expect("typed closed bibliography case");
        let invocation =
            BibInvocationCase::parse(&case.read_to_string("invocation.case").expect("metadata"))
                .expect("valid invocation metadata");
        let temp = tempfile::tempdir().expect("create isolated bibliography output directory");
        let resolved = invocation
            .resolve(&case, temp.path())
            .expect("validate bibliography invocation roles");
        let mut command = Command::new(env!("CARGO_BIN_EXE_umber"));
        command.arg("bib");
        command.args(&resolved.argv);
        let output = command.output().expect("run native bibliography case");
        assert_eq!(output.status.code(), Some(invocation.status), "{name}");
        assert_eq!(
            output.stdout,
            invocation.expected_channel(&case, &invocation.stdout),
            "{name} stdout"
        );
        assert_eq!(
            output.stderr,
            invocation.expected_channel(&case, &invocation.stderr),
            "{name} stderr"
        );
        if let Some((artifact, expected)) = &resolved.artifact {
            assert_eq!(
                fs::read(artifact).expect("generated bibliography artifact"),
                case.read(expected).expect("expected bibliography artifact"),
                "{name} artifact"
            );
        }
        FixtureCase::discover(&case_relative, "invocation.case", "bib-invocation-v2")
            .expect("invocation must not publish ambient case outputs");
    }
}

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

#[test]
fn bibliography_typed_arguments_reject_path_authority_and_undeclared_roles() {
    for literal in [
        "ambient.bib",
        "/tmp/ambient.bib",
        "../ambient.bib",
        "./ambient.bib",
        "nested/ambient.bib",
        ".",
    ] {
        let metadata = format!(
            "bib-invocation-v2\narg=literal:{literal}\nstatus=0\nstdout=empty\nstderr=empty\n"
        );
        assert!(
            BibInvocationCase::parse(&metadata).is_err(),
            "path-bearing literal accepted: {literal:?}"
        );
    }

    assert!(
        BibInvocationCase::parse(
            "bib-invocation-v2\n\
         arg=input:ambient\n\
         status=0\n\
         stdout=empty\n\
         stderr=empty\n",
        )
        .is_err()
    );
}

#[test]
fn bibliography_typed_roles_have_one_global_namespace_and_exact_cardinality() {
    for (name, body) in [
        (
            "duplicate input definition",
            "arg=input:source\ninput=source:basic.bcf\ninput=source:basic.bcf\n",
        ),
        (
            "duplicate output definition",
            "arg=output:result\noutput=result:one.bbl:expected.bbl\noutput=result:two.bbl:expected.bbl\n",
        ),
        (
            "cross-kind definition",
            "arg=input:shared\narg=output:shared\ninput=shared:basic.bcf\noutput=shared:result.bbl:expected.bbl\n",
        ),
        (
            "repeated input use",
            "arg=input:source\narg=input:source\ninput=source:basic.bcf\n",
        ),
        (
            "repeated output use",
            "arg=output:result\narg=output:result\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "unused input",
            "arg=literal:ordinary\ninput=source:basic.bcf\n",
        ),
        (
            "unused output",
            "arg=literal:ordinary\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "output used as input",
            "arg=input:result\noutput=result:result.bbl:expected.bbl\n",
        ),
        (
            "input used as output",
            "arg=output:source\ninput=source:basic.bcf\n",
        ),
        (
            "conflicting output artifact",
            "arg=output:first\narg=output:second\noutput=first:result.bbl:expected.bbl\noutput=second:result.bbl:expected.bbl\n",
        ),
    ] {
        let metadata = format!("bib-invocation-v2\n{body}status=0\nstdout=empty\nstderr=empty\n");
        assert!(
            BibInvocationCase::parse(&metadata).is_err(),
            "{name} accepted"
        );
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only isolated staging assertion.
fn bibliography_typed_arguments_materialize_literals_and_declared_roles() {
    let case = FixtureCase::discover(
        "tests/corpus/bib/invocation/bcf-success",
        "invocation.case",
        "bib-invocation-v2",
    )
    .expect("typed closed bibliography case");
    let invocation = BibInvocationCase::parse(
        "bib-invocation-v2\n\
         arg=literal:ordinary\n\
         arg=input:control\n\
         arg=output:artifact\n\
         status=0\n\
         input=control:basic.bcf\n\
         stdout=empty\n\
         stderr=empty\n\
         output=artifact:result.bbl:expected.bbl\n",
    )
    .expect("typed invocation");
    let workspace = tempfile::tempdir().expect("isolated workspace");
    let resolved = invocation
        .resolve(&case, workspace.path())
        .expect("materialized invocation");
    assert_eq!(resolved.argv[0], "ordinary");
    assert_eq!(
        Path::new(&resolved.argv[1]).parent(),
        Some(
            workspace
                .path()
                .canonicalize()
                .expect("workspace")
                .as_path()
        )
    );
    assert_eq!(
        fs::read(&resolved.argv[1]).expect("staged declared input"),
        case.read("basic.bcf").expect("authority input")
    );
    assert_eq!(
        Path::new(&resolved.argv[2]).parent(),
        Some(
            workspace
                .path()
                .canonicalize()
                .expect("workspace")
                .as_path()
        )
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only adversarial sentinel construction.
fn failing_path_literal_cannot_touch_an_ambient_sentinel() {
    let ambient = tempfile::tempdir().expect("ambient directory");
    let sentinel = ambient.path().join("ambient.bib");
    fs::write(&sentinel, b"untouched sentinel\n").expect("ambient sentinel");
    let metadata = format!(
        "bib-invocation-v2\narg=literal:{}\nstatus=0\nstdout=empty\nstderr=empty\n",
        sentinel.display()
    );

    assert!(BibInvocationCase::parse(&metadata).is_err());
    assert_eq!(
        fs::read(&sentinel).expect("ambient sentinel"),
        b"untouched sentinel\n"
    );
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

#[test]
#[allow(clippy::disallowed_methods)] // Hermetic adversarial output-root construction.
fn bibliography_artifact_names_reject_authority_escapes_and_collisions() {
    let temp = tempfile::tempdir().expect("artifact output root");
    for name in [
        "/tmp/ambient.bbl",
        "../ambient.bbl",
        ".",
        "./result.bbl",
        "nested/result.bbl",
        "case.inventory",
        "invocation.case",
        "{output}",
    ] {
        assert!(
            safe_artifact_path(temp.path(), name).is_err(),
            "unsafe artifact name accepted: {name}"
        );
    }
    fs::write(temp.path().join("result.bbl"), "occupied").expect("collision");
    assert!(safe_artifact_path(temp.path(), "result.bbl").is_err());
}

#[cfg(unix)]
#[test]
fn bibliography_artifact_names_reject_symlink_outputs_and_roots() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("artifact output root");
    symlink(
        "/tmp/ambient-bibliography-output",
        temp.path().join("result.bbl"),
    )
    .expect("artifact symlink");
    assert!(safe_artifact_path(temp.path(), "result.bbl").is_err());

    let parent = tempfile::tempdir().expect("symlink root parent");
    symlink(temp.path(), parent.path().join("output")).expect("output root symlink");
    assert!(safe_artifact_path(&parent.path().join("output"), "result.bbl").is_err());
}

#[test]
fn bibliography_artifact_name_accepts_a_fresh_safe_filename() {
    let temp = tempfile::tempdir().expect("artifact output root");
    assert_eq!(
        safe_artifact_path(temp.path(), "result.bbl").expect("safe artifact"),
        temp.path()
            .canonicalize()
            .expect("canonical output root")
            .join("result.bbl")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Regression exercises the native command with pinned files.
fn bibtex_command_runs_the_pinned_classic_smoke_case_in_process() {
    let fixture = test_support::repository_root()
        .join("crates/umber")
        .join("../../tests/corpus/bibtex/cases/smoke");
    let temp_dir = tempfile::tempdir().expect("create classic output directory");
    for extension in ["aux", "bib", "bst"] {
        fs::copy(
            fixture.join(format!("smoke.{extension}")),
            temp_dir.path().join(format!("smoke.{extension}")),
        )
        .expect("stage classic fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("bibtex")
        .arg(temp_dir.path().join("smoke"))
        .output()
        .expect("run native classic BibTeX command");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(temp_dir.path().join("smoke.bbl")).expect("generated BBL"),
        fs::read(fixture.join("smoke.bbl")).expect("pinned BBL")
    );
    assert_eq!(
        output.stdout,
        fs::read(fixture.join("smoke.terminal")).expect("pinned terminal output")
    );
    assert_eq!(
        fs::read(temp_dir.path().join("smoke.blg")).expect("generated BLG"),
        fs::read(fixture.join("smoke.blg")).expect("pinned BLG")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Regression exercises the native command with pinned files.
fn bib_command_processes_pinned_full_bibtex_unicode_names() {
    let fixture = test_support::repository_root()
        .join("crates/umber")
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let temp_dir = tempfile::tempdir().expect("create full BibTeX output directory");
    let output_path = temp_dir.path().join("full.bib");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("bib")
        .arg("--noconf")
        .arg("--nolog")
        .arg("--output-format=bibtex")
        .arg("--output-align")
        .arg("--output-file")
        .arg(&output_path)
        .arg(fixture.join("full-bibtex.bcf"))
        .output()
        .expect("run pinned full BibTeX command");

    assert_eq!(
        output.status.code(),
        Some(0),
        "native bib command failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let generated = fs::read_to_string(output_path).expect("generated full BibTeX output");
    assert!(generated.contains("H{ü}nenberger, Philippe H."));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_publishes_a_dumped_format_from_the_resource_session() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\catcode`@=11 \\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run format dump");

    assert!(
        output.status.success(),
        "format dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fs::read(&format).expect("read dumped format").is_empty());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("Beginning to dump on file {}", format.display()))
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn initex_dump_survives_futurelet_reusing_a_control_sequence_that_means_space() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("futurelet-space.tex");
    let format = temp_dir.path().join("futurelet-space.fmt");
    fs::write(
        &source,
        concat!(
            "\\catcode`\\@=11\n",
            "\\long\\def\\@ifnextchar#1#2#3{\\let\\reserved@d=#1",
            "\\def\\reserved@a{#2}\\def\\reserved@b{#3}",
            "\\futurelet\\@let@token\\@ifnch}\n",
            "\\def\\@ifnch{\\ifx\\@let@token\\@sptoken",
            "\\let\\reserved@c\\@xifnch\\else",
            "\\ifx\\@let@token\\reserved@d\\let\\reserved@c\\reserved@a",
            "\\else\\let\\reserved@c\\reserved@b\\fi\\fi\\reserved@c}\n",
            "\\def\\:{\\let\\@sptoken= } \\: \n",
            "\\def\\:{\\@xifnch} \\expandafter\\def\\: ",
            "{\\futurelet\\@let@token\\@ifnch}\n",
            "\\def\\consume[#1]{\\def\\result{yes}}\n",
            "\\def\\bad{\\def\\result{no}}\n",
            "\\def\\probe{\\@ifnextchar[{\\consume}{\\bad}}\n",
            "\\probe\n  [1]\n",
            "\\def\\expected{yes}\n",
            "\\ifx\\result\\expected\\else\\errmessage{lookahead failed}\\fi\n",
            "\\dump\n",
        ),
    )
    .expect("write futurelet format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run futurelet format dump");

    assert!(
        output.status.success(),
        "futurelet format dump failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!fs::read(&format).expect("read dumped format").is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_publishes_dump_to_default_tex82_name_before_announcing_it() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("plain.tex");
    fs::write(&source, "\\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(temp_dir.path())
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "plain.tex"])
        .output()
        .expect("run default format dump");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !fs::read(temp_dir.path().join("plain.fmt"))
            .expect("read default dumped format")
            .is_empty()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Beginning to dump on file plain.fmt")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn failed_format_publication_never_announces_success() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("plain.tex");
    fs::write(&source, "\\dump\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out", "/proc/umber-unwritable.fmt"])
        .arg(&source)
        .output()
        .expect("run failed format publication");

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Beginning to dump on file"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn format_output_rejects_a_successful_run_that_did_not_dump() {
    let temp_dir = tempfile::tempdir().expect("create format output temp dir");
    let source = temp_dir.path().join("format.tex");
    let format = temp_dir.path().join("format.fmt");
    fs::write(&source, "\\message{NO-DUMP}\\end\n").expect("write format source");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args(["run", "--format-out"])
        .arg(&format)
        .arg(&source)
        .output()
        .expect("run missing format dump");

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "umber: --format-out requires the input to execute \\dump\n"
    );
    assert!(!format.exists());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_rule_page_is_published_only_to_an_explicit_distinct_pdf_path() {
    let temp_dir = tempfile::tempdir().expect("create PDF output temp dir");
    let source = temp_dir.path().join("rule.tex");
    let pdf = temp_dir.path().join("rule.pdf");
    let dvi = temp_dir.path().join("rule.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write PDF rule fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run pdfTeX PDF fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read published PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
    assert!(fs::metadata(&dvi).expect("published DVI").len() > 0);
    let telemetry = String::from_utf8_lossy(&output.stderr);
    for marker in [
        "RESOURCE_STARTUP_TELEMETRY",
        "RESOURCE_ENGINE_ACCEPTED",
        "RESOURCE_HOST_TELEMETRY",
        "PDF_TELEMETRY",
        "PDF_DRIVER_BUILD",
        "PDF_DRIVER_TELEMETRY",
    ] {
        assert!(telemetry.contains(marker), "missing {marker}:\n{telemetry}");
    }

    let rejected = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdf")
        .arg(temp_dir.path().join("wrong-mode.pdf"))
        .arg(&source)
        .output()
        .expect("reject PDF without pdfTeX mode");
    assert!(!rejected.status.success());
    assert_eq!(
        String::from_utf8(rejected.stderr).expect("stderr is utf-8"),
        "umber: --pdf requires --pdftex or --pdflatex\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_cli_keeps_deferred_effect_pages_in_the_published_page_tree() {
    let temp_dir = tempfile::tempdir().expect("create prepared PDF output temp dir");
    let source = temp_dir.path().join("prepared-pages.tex");
    let pdf = temp_dir.path().join("prepared-pages.pdf");
    fs::write(
        &source,
        concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfobjcompresslevel=0",
            "\\shipout\\vbox{\\hrule width1pt height1pt}",
            "\\shipout\\vbox{\\openout0=side-effect.txt",
            "\\write0{page-two}\\hrule width2pt height2pt}\\end\n",
        ),
    )
    .expect("write prepared PDF fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(temp_dir.path())
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run prepared PDF fixture");
    assert!(
        output.status.success(),
        "prepared PDF run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(temp_dir.path().join("side-effect.txt"))
            .expect("deferred page effect committed"),
        "page-two\n"
    );
    let pdf = fs::read(pdf).expect("read prepared PDF");
    let parsed = test_support::pdf_query::PdfQuery::new(
        &pdf,
        test_support::pdf_query::QueryLimits::default(),
    )
    .expect("independent parser accepts prepared PDF");
    assert_eq!(parsed.pages().expect("prepared page tree").len(), 2);
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn pdflatex_mode_composes_latex_compatibility_with_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create pdfLaTeX output temp dir");
    let source = temp_dir.path().join("composed.tex");
    let pdf = temp_dir.path().join("composed.pdf");
    fs::write(
        &source,
        "\\catcode123=1\\catcode125=2\\pdfoutput=1\\ifnum\\strcmp{same}{same}=0\\shipout\\vbox{\\hrule width10pt height5pt}\\fi\\end\n",
    )
    .expect("write composed pdfLaTeX fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdflatex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run composed pdfLaTeX fixture");

    assert!(
        output.status.success(),
        "pdfLaTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pdf_bytes = fs::read(&pdf).expect("read composed pdfLaTeX PDF");
    assert!(pdf_bytes.starts_with(b"%PDF-1.4"));
    assert!(pdf_bytes.ends_with(b"%%EOF"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdfdraftmode_does_not_replace_the_requested_pdf_output() {
    let temp_dir = tempfile::tempdir().expect("create draft-mode output temp dir");
    let source = temp_dir.path().join("draft.tex");
    let pdf = temp_dir.path().join("draft.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfdraftmode=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end\n",
    )
    .expect("write draft-mode fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg(&source)
        .output()
        .expect("run draft-mode fixture");

    assert!(
        output.status.success(),
        "draft-mode run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr is utf-8"),
        "pdfTeX warning: \\pdfdraftmode enabled, not changing output pdf\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read unchanged output"),
        b"existing output\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn fatal_pdf_finalization_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create fatal-finalization output temp dir");
    let source = temp_dir.path().join("fatal-finalization.tex");
    let pdf = temp_dir.path().join("fatal-finalization.pdf");
    let closure = temp_dir.path().join("fatal-finalization.font-closure");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfobj reserveobjnum\\pdfrefobj 1\\end\n",
    )
    .expect("write fatal-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg("--pdf-font-closure-out")
            .arg(&closure)
            .arg(&source)
            .output()
            .expect("run fatal-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: referenced PDF object 1 was reserved but never initialized\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal detached finalization must publish no partial artifact"
    );
    assert_eq!(
        fs::read(&closure).expect("read accepted font closure"),
        b"umber-pdf-font-closure-v1\n",
        "accepted resource evidence survives a later detached-driver failure"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn unfinished_pdf_thread_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create thread-finalization output temp dir");
    let source = temp_dir.path().join("thread-finalization.tex");
    let pdf = temp_dir.path().join("thread-finalization.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\pdfstartthread name{open}}\\end\n",
    )
    .expect("write thread-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run thread-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: page 1 ends with PDF thread object 1 still running\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal thread finalization must not replace the requested PDF"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn unavailable_delayed_pdf_image_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create missing-image output temp dir");
    let source = temp_dir.path().join("missing-image.tex");
    let pdf = temp_dir.path().join("missing-image.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfximage{missing.png}\\shipout\\hbox{\\pdfrefximage1}\\end\n",
    )
    .expect("write missing-image fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run missing-image fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        concat!(
            "umber: incremental execution failed: pdfTeX error (ext5): ",
            "cannot read image file missing.png: image is unavailable\n",
        )
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "failed delayed acquisition must publish no replacement artifact"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn fatal_annotation_action_finalization_does_not_replace_the_requested_output() {
    let temp_dir = tempfile::tempdir().expect("create annotation-finalization output temp dir");
    let source = temp_dir.path().join("annotation-finalization.tex");
    let pdf = temp_dir.path().join("annotation-finalization.pdf");
    fs::write(
        &source,
        "\\pdfoutput=1\\pdfcatalog{} openaction goto page 2 {/Fit}\\shipout\\hbox{}\\end\n",
    )
    .expect("write annotation-finalization fixture");
    fs::write(&pdf, b"existing output\n").expect("seed existing PDF path");

    let run = || {
        Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("run")
            .arg("--pdftex")
            .arg("--pdf")
            .arg(&pdf)
            .arg(&source)
            .output()
            .expect("run annotation-finalization fixture")
    };
    let first = run();
    let second = run();

    assert!(!first.status.success());
    assert!(!second.status.success());
    assert_eq!(first.stderr, second.stderr, "fatal diagnostics are stable");
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr is utf-8"),
        "umber: PDF open action references missing page 2\n"
    );
    assert_eq!(
        fs::read(&pdf).expect("read preserved output"),
        b"existing output\n",
        "fatal detached finalization must publish no partial annotation artifact"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdf_lowering_omits_dvi_special_and_publishes_all_driver_output() {
    let temp_dir = tempfile::tempdir().expect("create DVI-special temp dir");
    let source = temp_dir.path().join("text.tex");
    let pdf = temp_dir.path().join("text.pdf");
    let dvi = temp_dir.path().join("text.dvi");
    fs::write(
        &source,
        "\\pdfoutput=1\\shipout\\vbox{\\special{dvi-only-payload}}\\end\n",
    )
    .expect("write DVI-special fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("UMBER_RESOURCE_TELEMETRY", "1")
        .arg("run")
        .arg("--pdftex")
        .arg("--pdf")
        .arg(&pdf)
        .arg("--dvi")
        .arg(&dvi)
        .arg(&source)
        .output()
        .expect("run DVI-special PDF fixture");

    assert!(
        output.status.success(),
        "DVI-special PDF run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RESOURCE_ENGINE_ACCEPTED"),
        "accepted-engine telemetry must precede detached finalization"
    );
    let pdf_bytes = fs::read(&pdf).expect("PDF output was published");
    assert!(
        !pdf_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI-only special leaked into PDF output"
    );
    let dvi_bytes = fs::read(&dvi).expect("DVI peer output was published");
    assert!(
        dvi_bytes
            .windows(b"dvi-only-payload".len())
            .any(|window| window == b"dvi-only-payload"),
        "DVI peer output lost its special payload"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn lex_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("lexer") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("lex-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber lex-dump");

        assert!(
            output.status.success(),
            "lex-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("lex-dump output is utf-8");
        assert_matches_fixture("lexer", case.name(), "tokens", &actual);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture discovery and expected-output reads.
fn expand_dump_prints_stable_token_format_for_corpus() {
    for case in corpus_cases("expand") {
        let output = Command::new(env!("CARGO_BIN_EXE_umber"))
            .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
            .arg("expand-dump")
            .arg(case.source_path())
            .output()
            .expect("run umber expand-dump");

        assert!(
            output.status.success(),
            "expand-dump failed for {}:\n{}",
            case.source_path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let actual = String::from_utf8(output.stdout).expect("expand-dump output is utf-8");
        assert_matches_fixture("expand", case.name(), "tokens", &actual);
    }
}

#[test]
fn expand_dump_usage_errors_follow_lex_dump_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .output()
        .expect("run umber expand-dump without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for expand-dump\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber expand-dump with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: expand-dump accepts exactly one input path\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_expansion_error_renders_primary_source_context() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump diagnostic fixture");

    assert!(
        !output.status.success(),
        "undefined expand-dump should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("undefined.tex:1:1"));
    assert!(stderr.contains("  1 | \\undefined"));
    assert!(stderr.contains("    | ^"));
    assert!(!stderr.contains("unknown origin"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_macro_error_renders_bounded_expansion_trace() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\undefined X}\\a\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump macro diagnostic fixture");

    assert!(!output.status.success(), "macro expand-dump should fail");
    let stderr = String::from_utf8(output.stderr).expect("stderr is utf-8");
    assert!(stderr.contains("Undefined control sequence \\undefined"));
    assert!(stderr.contains("macro.tex:1:8"));
    assert!(stderr.contains("expansion trace:"));
    assert!(stderr.contains("invoked at"));
    assert!(stderr.contains("defined at"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn expand_dump_recovered_execution_error_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create execution diagnostic temp dir");
    let source = temp_dir.path().join("prefix.tex");
    fs::write(&source, "\\global X\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("expand-dump")
        .arg(&source)
        .output()
        .expect("run umber expand-dump execution diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary fixture setup and command execution.
fn run_recovered_diagnostic_after_tfm_load_exits_successfully() {
    let temp_dir = tempfile::tempdir().expect("create font provenance temp dir");
    let source = temp_dir.path().join("after-font.tex");
    let child = temp_dir.path().join("child.tex");
    let tfm = temp_dir.path().join("cmr10.tfm");
    fs::write(&source, "\\font\\f=cmr10 \\relax\n\\input child\n\\end\n")
        .expect("write main fixture");
    fs::write(&child, "\\global X\n").expect("write diagnostic fixture");
    fs::copy(
        test_support::repository_root().join("crates/tex-fonts/tests/fixtures/cm/cmr10.tfm"),
        &tfm,
    )
    .expect("copy TFM fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run font provenance fixture");

    assert!(
        output.status.success(),
        "recovered prefix error should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("You can't use a prefix"), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn run_exec_corpus_matches_committed_diagnostics() {
    run_corpus_matches_committed_terminal_fixtures(
        "exec",
        false,
        &["hmode_material_primitives"], // umber2-johp.757
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
fn run_etex_exec_corpus_matches_committed_diagnostics() {
    for case in corpus_cases("etex_exec") {
        assert_log_case_matches_committed_fixture("etex_exec", &case, false, true);
    }
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side corpus discovery and command execution.
#[ignore = "manual compatibility/parity tier: not a cutover closure gate"]
fn run_typeset_corpus_matches_committed_box_dumps() {
    run_corpus_matches_committed_terminal_fixtures(
        "typeset",
        true,
        &[
            "alignment_showlists_unset", // umber2-johp.758
            "material_primitives",       // umber2-johp.757
        ],
    );
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

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_math_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("math");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_align_corpus_matches_committed_dvi() {
    assert_dvi_area_matches_committed_fixture("align");
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side command execution.
fn removed_html_font_directory_names_the_typed_replacement() {
    // Any small committed source will do: this asserts an argument-parsing
    // rejection that never reaches the engine. `dvi`/`page` were retired
    // into the minifixture corpus, so it names a surviving area.
    let setup = dvi::DviCaseSetup::new("math", "accents");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .current_dir(setup.run_dir())
        .args([
            "run",
            setup.source_file_name(),
            "--html",
            "actual.html",
            "--html-font-dir",
            "web-fonts",
        ])
        .output()
        .expect("run removed HTML font option");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("--html-font-dir was removed"));
    assert!(error.contains("typed resource resolver API"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_initializes_clock_parameters_from_source_date_epoch() {
    let temp_dir = tempfile::tempdir().expect("create clock temp dir");
    let source = temp_dir.path().join("clock.tex");
    fs::write(
        &source,
        "\\message{clock=\\the\\time/\\the\\day/\\the\\month/\\the\\year}\\end\n",
    )
    .expect("write clock fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run umber clock fixture");

    assert!(
        output.status.success(),
        "clock run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("clock=816/9/7/2026"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn latex_creationdate_uses_the_source_date_epoch_job_clock() {
    let temp_dir = tempfile::tempdir().expect("create creation-date temp dir");
    let source = temp_dir.path().join("creationdate.tex");
    fs::write(
        &source,
        "\\catcode123=1 \\catcode125=2 \\message{created=\\creationdate}\\end\n",
    )
    .expect("write creation-date fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--latex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber LaTeX creation-date fixture");

    assert!(
        output.status.success(),
        "creation-date run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("created=D:20260709133600Z"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn pdftex_mode_reports_the_pinned_engine_identity() {
    let temp_dir = tempfile::tempdir().expect("create pdfTeX identity temp dir");
    let source = temp_dir.path().join("identity.tex");
    fs::write(
        &source,
        "\\message{engine=\\the\\pdftexversion.\\pdftexrevision}\\end\n",
    )
    .expect("write pdfTeX identity fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .arg("run")
        .arg("--pdftex")
        .arg("--show-fixtures")
        .arg(&source)
        .output()
        .expect("run Umber pdfTeX identity fixture");

    assert!(
        output.status.success(),
        "pdfTeX run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .expect("stdout is utf-8")
            .contains("engine=140.27")
    );
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

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_deadcycles_overflow() {
    let temp_dir = tempfile::tempdir().expect("create deadcycles temp dir");
    let source = temp_dir.path().join("deadcycles.tex");
    fs::write(
        &source,
        "\\maxdeadcycles=1 \\output={\\setbox1=\\box255}\n\
         \\topskip=0pt \\setbox0=\\hbox{}\n\
         \\copy0 \\penalty-10000\n\
         \\copy0 \\penalty-10000\n\
         \\end\n",
    )
    .expect("write deadcycles fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber deadcycles fixture");

    assert!(
        output.status.success(),
        "recovered deadcycles overflow should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Output loop---1 consecutive dead cycles"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_right_brace() {
    let temp_dir = tempfile::tempdir().expect("create diagnostic temp dir");
    let source = temp_dir.path().join("brace.tex");
    fs::write(&source, "}\n\\end\n").expect("write diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra brace should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Too many }'s."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_undefined_control_sequence() {
    let temp_dir = tempfile::tempdir().expect("create expansion diagnostic temp dir");
    let source = temp_dir.path().join("undefined.tex");
    fs::write(&source, "\\undefined\n\\end\n").expect("write expansion diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber expansion diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered undefined control sequence should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    // tex.web §370 prints the message alone; §82's `show_context` display is
    // what names the offending control sequence, on its own `l.N` line.
    assert!(stdout.contains("! Undefined control sequence."), "{stdout}");
    assert!(stdout.contains("l.1 \\undefined"), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_show_publishes_canonical_error_context() {
    let temp_dir = tempfile::tempdir().expect("create show-context temp dir");
    let source = temp_dir.path().join("show.tex");
    fs::write(&source, "\\def\\foo{bar}\\show\\foo\\end\n").expect("write show fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber show fixture");

    assert!(output.status.success(), "recovered show should succeed");
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    // TeX82 §§82, 310 publish the live input context after `show_eqtb`'s
    // diagnostic and before returning from the recoverable error.
    assert!(stdout.contains("> \\foo=macro:\n->bar."), "{stdout}");
    assert!(
        stdout.contains("l.1 \\def\\foo{bar}\\show\\foo"),
        "{stdout}"
    );
    assert!(
        output.stderr.is_empty(),
        "recovered diagnostic must stay on the terminal channel"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_recovers_from_extra_endgroup_in_macro() {
    let temp_dir = tempfile::tempdir().expect("create macro diagnostic temp dir");
    let source = temp_dir.path().join("macro.tex");
    fs::write(&source, "\\def\\a{\\endgroup}\\a\n\\end\n").expect("write macro diagnostic fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .output()
        .expect("run umber macro diagnostic fixture");

    assert!(
        output.status.success(),
        "recovered extra endgroup should succeed"
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("Extra \\endgroup."));
    assert!(
        output.stderr.is_empty(),
        "recovered error must not reach stderr"
    );
}

#[test]
fn run_usage_errors_follow_existing_shape() {
    let missing = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .output()
        .expect("run umber run without path");
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let extra = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("two.tex")
        .output()
        .expect("run umber run with extra path");
    assert!(!extra.status.success());
    assert_eq!(
        String::from_utf8(extra.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let removed_plain_format = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--plain-format")
        .output()
        .expect("run umber run with removed --plain-format flag");
    assert!(!removed_plain_format.status.success());
    assert_eq!(
        String::from_utf8(removed_plain_format.stderr).expect("stderr is utf-8"),
        "umber: run accepts one input path with optional --show-fixtures and --dvi <path>\n"
    );

    let missing_show_fixtures = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("--show-fixtures")
        .output()
        .expect("run umber run with show-fixtures but without path");
    assert!(!missing_show_fixtures.status.success());
    assert_eq!(
        String::from_utf8(missing_show_fixtures.stderr).expect("stderr is utf-8"),
        "umber: missing input path for run\n"
    );

    let missing_dvi_path = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg("one.tex")
        .arg("--dvi")
        .output()
        .expect("run umber run with --dvi but without output path");
    assert!(!missing_dvi_path.status.success());
    assert_eq!(
        String::from_utf8(missing_dvi_path.stderr).expect("stderr is utf-8"),
        "umber: missing output path for --dvi\n"
    );

    let conflicting_outputs = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .args([
            "run",
            "one.tex",
            "--dvi",
            "same.out",
            "--format-out",
            "same.out",
        ])
        .output()
        .expect("run umber with conflicting output paths");
    assert!(!conflicting_outputs.status.success());
    assert_eq!(
        String::from_utf8(conflicting_outputs.stderr).expect("stderr is utf-8"),
        "umber: --dvi and --format-out must use different output paths\n"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_input_through_texinputs_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX input search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let search_dir = temp_dir.path().join("generic/hyphen");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX input search directory");
    let source = job_dir.join("plain.tex");
    fs::write(&source, "\\input hyphen \\message{after-hyphen}\\end\n")
        .expect("write principal input");
    fs::write(search_dir.join("hyphen.tex"), "\\message{loaded-hyphen}\n")
        .expect("write searched input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run input search smoke");

    assert!(
        output.status.success(),
        "input search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("loaded-hyphen"));
    assert!(stdout.contains("after-hyphen"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_acquires_from_a_local_distribution_then_reuses_cache_offline() {
    let temp_dir = tempfile::tempdir().expect("create distribution temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let cache = temp_dir.path().join("cache");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(&source, "\\input remote \\message{after-remote}\\end\n").expect("write source");
    let remote = b"\\message{from-distribution}\n";
    let object_digest = hex_sha256(remote);
    let object = format!("sha256-{object_digest}");
    fs::write(objects.join(&object), remote).expect("write object");
    let shard = format!(
        "{{\"schema\":1,\"distribution\":\"test-snapshot\",\"index\":0,\"files\":{{\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/tex/remote.tex\",\"object\":\"{object}\",\"sha256\":\"{object_digest}\",\"bytes\":{}}}}}}}\n",
        remote.len()
    );
    let shard_digest = hex_sha256(shard.as_bytes());
    fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("write shard");
    let manifest = format!(
        "{{\"schema\":2,\"distribution\":\"test-snapshot\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
    );
    fs::write(distribution.join("manifest-v2.json"), &manifest).expect("write manifest");

    let first = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .args(["run", "--show-fixtures", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run cold local distribution");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("from-distribution"));
    assert_eq!(
        String::from_utf8(first.stderr).expect("stderr UTF-8"),
        "umber: acquired 1 distribution resource(s)\n"
    );

    fs::remove_file(objects.join(object)).expect("remove source object after warming");
    let second = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("XDG_CACHE_HOME", &cache)
        .env("UMBER_OFFLINE", "1")
        .args(["run", "--show-fixtures", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run warm offline distribution");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(second.stderr.is_empty());
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_rejects_a_manifest_that_mismatches_its_pin() {
    let temp_dir = tempfile::tempdir().expect("create manifest mismatch temp dir");
    let source = temp_dir.path().join("main.tex");
    let manifest = temp_dir.path().join("manifest.json");
    fs::write(&source, "\\input absent \\end\n").expect("write source");
    fs::write(
        &manifest,
        "{\"schema\":1,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/\",\"files\":{}}",
    )
    .expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("cache"))
        .args([
            "run",
            "--distribution",
            manifest.to_str().expect("UTF-8 path"),
            "--distribution-sha256",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .arg(&source)
        .output()
        .expect("run mismatched manifest");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("distribution manifest digest mismatch")
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary distribution and command execution.
fn run_offline_local_mirror_miss_names_the_exact_object_digest() {
    let temp_dir = tempfile::tempdir().expect("create offline miss temp dir");
    let source = temp_dir.path().join("main.tex");
    let distribution = temp_dir.path().join("distribution");
    let objects = distribution.join("objects");
    fs::create_dir_all(&objects).expect("create distribution");
    fs::write(&source, "\\input remote \\end\n").expect("write source");
    let bytes = b"\\relax\n";
    let digest = hex_sha256(bytes);
    let entry = format!(
        "\"tex:remote.tex\":{{\"virtualPath\":\"/texlive/remote.tex\",\"object\":\"sha256-{digest}\",\"sha256\":\"{digest}\",\"bytes\":{}}}",
        bytes.len()
    );
    let shard =
        format!("{{\"schema\":1,\"distribution\":\"test\",\"index\":0,\"files\":{{{entry}}}}}\n");
    let shard_digest = hex_sha256(shard.as_bytes());
    fs::write(objects.join(format!("sha256-{shard_digest}")), shard).expect("write shard");
    fs::write(
        distribution.join("manifest-v2.json"),
        format!(
            "{{\"schema\":2,\"distribution\":\"test\",\"objectsBaseUrl\":\"https://example.invalid/objects/\",\"shardBits\":0,\"shardCount\":1,\"shards\":[\"{shard_digest}\"]}}\n"
        ),
    )
    .expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("XDG_CACHE_HOME", temp_dir.path().join("empty-cache"))
        .args(["run", "--offline", "--distribution"])
        .arg(&distribution)
        .arg(&source)
        .output()
        .expect("run offline miss");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to read ")
            && stderr.contains(&format!("sha256-{digest}"))
            && !stderr.contains("tex:remote.tex"),
        "{stderr}"
    );
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_writes_a_sorted_deduplicated_input_record_receipt() {
    let temp_dir = tempfile::tempdir().expect("create input receipt temp dir");
    let source = temp_dir.path().join("main.tex");
    let helper = temp_dir.path().join("helper.tex");
    let nested = temp_dir.path().join("nested.tex");
    let receipt = temp_dir.path().join("inputs.tsv");
    let source_bytes = b"\\input helper \\input helper \\end\n";
    let helper_bytes = b"\\input nested \\relax\n";
    let nested_bytes = b"\\relax\n";
    fs::write(&source, source_bytes).expect("write principal input");
    fs::write(&helper, helper_bytes).expect("write included input");
    fs::write(&nested, nested_bytes).expect("write nested input");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .arg("run")
        .arg(&source)
        .arg("--input-records-out")
        .arg(&receipt)
        .output()
        .expect("run input receipt smoke");

    assert!(
        output.status.success(),
        "input receipt run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "{}\t{}\n{}\t{}\n{}\t{}\n",
        helper_bytes.len(),
        helper.display(),
        source_bytes.len(),
        source.display(),
        nested_bytes.len(),
        nested.display()
    );
    assert_eq!(
        fs::read_to_string(receipt).expect("read input receipt"),
        expected
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_quoted_openin_through_texinputs() {
    let temp_dir = tempfile::tempdir().expect("create TeX stream search temp dir");
    let job_dir = temp_dir.path().join("latex/base");
    let search_dir = temp_dir.path().join("latex/l3kernel");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&search_dir).expect("create TeX stream search directory");
    let source = job_dir.join("stream-search.tex");
    fs::write(
        &source,
        "\\openin1=\"probe.tex\" \\ifeof1 \\errmessage{missing-probe}\\else \\message{found-probe}\\fi \\end\n",
    )
    .expect("write stream search input");
    fs::write(search_dir.join("probe.tex"), "found\n").expect("write searched stream");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXINPUTS", &search_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run stream search smoke");

    assert!(
        output.status.success(),
        "stream search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("found-probe"));
    assert!(!stdout.contains("missing-probe"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side temporary files and command execution.
fn run_resolves_area_less_tfm_through_texfonts_and_advances() {
    let temp_dir = tempfile::tempdir().expect("create TeX font search temp dir");
    let job_dir = temp_dir.path().join("plain/base");
    let font_dir = temp_dir.path().join("fonts/tfm/public/cm");
    fs::create_dir_all(&job_dir).expect("create principal input directory");
    fs::create_dir_all(&font_dir).expect("create TeX font search directory");
    let source = job_dir.join("font-search.tex");
    fs::write(
        &source,
        "\\font\\tenrm=cmr10 \\relax \\message{after-font}\\end\n",
    )
    .expect("write font search input");
    let cmr10 = test_support::repository_root()
        .join("crates/umber")
        .join("../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    fs::copy(cmr10, font_dir.join("cmr10.tfm")).expect("copy searched TFM");

    let output = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .env("TEXFONTS", &font_dir)
        .arg("run")
        .arg(&source)
        .arg("--show-fixtures")
        .output()
        .expect("run font search smoke");

    assert!(
        output.status.success(),
        "font search run failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is utf-8");
    assert!(stdout.contains("after-font"));
}

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture command execution and file checks.
fn run_show_fixtures_harvests_without_committing_immediate_stream_effects() {
    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let normal_dir = temp_dir.path().join("normal");
    let fixture_dir = temp_dir.path().join("fixture");
    fs::create_dir_all(&normal_dir).expect("create normal dir");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    let input = temp_dir.path().join("stream_effect.tex");
    fs::write(
        &input,
        "\\immediate\\openout0=side-effect.txt\n\
         \\immediate\\write0{immediate-effect}\n\
         \\immediate\\closeout0\n\\end\n",
    )
    .expect("write input");

    let normal = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&normal_dir)
        .arg("run")
        .arg(&input)
        .output()
        .expect("run ordinary umber run");
    assert!(
        normal.status.success(),
        "ordinary run failed:\n{}",
        String::from_utf8_lossy(&normal.stderr)
    );
    assert!(
        normal_dir.join("side-effect.txt").exists(),
        "ordinary run should commit immediate stream effects at final commit"
    );

    let fixture = Command::new(env!("CARGO_BIN_EXE_umber"))
        .env("SOURCE_DATE_EPOCH", PINNED_SOURCE_DATE_EPOCH)
        .current_dir(&fixture_dir)
        .arg("run")
        .arg("--show-fixtures")
        .arg(&input)
        .output()
        .expect("run umber fixture harvest");
    assert!(
        fixture.status.success(),
        "fixture run failed:\n{}",
        String::from_utf8_lossy(&fixture.stderr)
    );
    assert!(
        !fixture_dir.join("side-effect.txt").exists(),
        "--show-fixtures must not run the final commit for pending immediate effects"
    );
}

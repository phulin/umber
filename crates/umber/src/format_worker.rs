//! Authenticated process boundary for bounded format construction.

use std::io::{Read, Write};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tex_command::RegisteredSourceKind;
use tex_state::{JobClock, Universe, World};

use crate::EngineMode;
use crate::format_fixture::{
    FormatFixtureError, FormatGenerationGuards, FormatRecipe, FormatResource,
    construct_format_in_worker,
};

const PROTOCOL: u32 = 1;
const MAX_WORKER_STDOUT_BYTES: usize = crate::SessionLimits::FORMAT_IMAGE_BYTES + 64 * 1024;
const MAX_WORKER_STDERR_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Request {
    protocol: u32,
    identity: [u8; 32],
    engine: u8,
    format_name: String,
    source_name: String,
    source: Vec<u8>,
    resources: Vec<Resource>,
    distribution: Vec<u8>,
    clock: [i32; 5],
    fuel: u64,
    wall_ns: u64,
    resident_bytes: u64,
}

#[derive(Serialize, Deserialize)]
enum Resource {
    Input(u8, String, Vec<u8>),
    Tfm(String, Vec<u8>),
}

#[derive(Serialize, Deserialize)]
struct Response {
    protocol: u32,
    identity: [u8; 32],
    image_sha256: [u8; 32],
    result: Result<Vec<u8>, String>,
}

pub(crate) fn construct(recipe: &FormatRecipe) -> Result<Vec<u8>, FormatFixtureError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = recipe;
        return Err(FormatFixtureError::ResidentSetUnsupported);
    }
    #[cfg(target_os = "linux")]
    {
        let identity = recipe.identity()?.key().bytes();
        let request = Request::from_recipe(recipe, identity)?;
        let request_bytes = bincode::serialize(&request)
            .map_err(|error| FormatFixtureError::WorkerProtocol(error.to_string()))?;
        let executable = worker_executable()?;
        let mut child = Command::new(executable)
            .arg("__format-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| FormatFixtureError::WorkerSpawn(error.to_string()))?;
        let Some(mut stdin) = child.stdin.take() else {
            terminate(&mut child);
            return Err(FormatFixtureError::WorkerProtocol(
                "missing worker stdin".into(),
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate(&mut child);
            return Err(FormatFixtureError::WorkerProtocol(
                "missing worker stdout".into(),
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate(&mut child);
            return Err(FormatFixtureError::WorkerProtocol(
                "missing worker stderr".into(),
            ));
        };
        let writer = std::thread::spawn(move || {
            stdin
                .write_all(&request_bytes)
                .map_err(|error| error.to_string())
        });
        let collected = supervise_and_collect(
            &mut child,
            recipe.guards,
            stdout,
            stderr,
            MAX_WORKER_STDOUT_BYTES,
            MAX_WORKER_STDERR_BYTES,
        );
        let writer_result = writer.join();
        let collected = match collected {
            Ok(collected) => collected,
            Err(error) => {
                let _ = writer_result;
                return Err(error);
            }
        };
        match writer_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                terminate(&mut child);
                return Err(FormatFixtureError::WorkerProtocol(format!(
                    "worker stdin: {error}"
                )));
            }
            Err(_) => {
                terminate(&mut child);
                return Err(FormatFixtureError::WorkerProtocol(
                    "worker request writer panicked".into(),
                ));
            }
        }
        let response: Response = bincode::deserialize(&collected.stdout)
            .map_err(|error| FormatFixtureError::WorkerProtocol(error.to_string()))?;
        if response.protocol != PROTOCOL || response.identity != identity {
            return Err(FormatFixtureError::WorkerIdentityMismatch);
        }
        let image = response.result.map_err(FormatFixtureError::Worker)?;
        if <[u8; 32]>::from(Sha256::digest(&image)) != response.image_sha256 {
            return Err(FormatFixtureError::WorkerIdentityMismatch);
        }
        Universe::from_format(World::memory(), &image)
            .map_err(|error| FormatFixtureError::Format(error.to_string()))?;
        Ok(image)
    }
}

#[cfg(target_os = "linux")]
struct CollectedWorkerOutput {
    stdout: Vec<u8>,
    #[allow(dead_code)]
    stderr: Vec<u8>,
}

#[cfg(target_os = "linux")]
fn read_bounded(
    mut input: impl Read,
    limit: usize,
    stream: &'static str,
) -> Result<Vec<u8>, FormatFixtureError> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = input
            .read(&mut chunk)
            .map_err(|error| FormatFixtureError::WorkerProtocol(format!("{stream}: {error}")))?;
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > limit {
            return Err(FormatFixtureError::WorkerProtocol(format!(
                "{stream} exceeded {limit} bytes"
            )));
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::disallowed_methods,
    reason = "native format-worker supervisor measures real wall time outside deterministic engine state"
)]
fn supervise_and_collect(
    child: &mut Child,
    guards: FormatGenerationGuards,
    stdout: ChildStdout,
    stderr: ChildStderr,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<CollectedWorkerOutput, FormatFixtureError> {
    let (sender, receiver) = mpsc::channel();
    let stdout_sender = sender.clone();
    let stdout_reader = std::thread::spawn(move || {
        let _ = stdout_sender.send((true, read_bounded(stdout, stdout_limit, "worker stdout")));
    });
    let stderr_reader = std::thread::spawn(move || {
        let _ = sender.send((false, read_bounded(stderr, stderr_limit, "worker stderr")));
    });
    let started = Instant::now();
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    let result = 'supervision: loop {
        while let Ok((is_stdout, result)) = receiver.try_recv() {
            match result {
                Ok(bytes) if is_stdout => stdout = Some(bytes),
                Ok(bytes) => stderr = Some(bytes),
                Err(error) => {
                    terminate(child);
                    break 'supervision Err(error);
                }
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(observed)) => status = Some(observed),
                Ok(None) => {}
                Err(error) => {
                    terminate(child);
                    break Err(FormatFixtureError::Worker(error.to_string()));
                }
            }
        }
        if started.elapsed() > guards.wall_time {
            terminate(child);
            break Err(FormatFixtureError::WallTimeExceeded);
        }
        if let Some(observed) = status {
            if let (Some(stdout), Some(stderr)) = (stdout.take(), stderr.take()) {
                if observed.success() {
                    break Ok(CollectedWorkerOutput { stdout, stderr });
                }
                break Err(FormatFixtureError::WorkerCrashed(
                    observed.code(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                ));
            }
        } else {
            match worker_rss(child.id()) {
                Ok(rss) if rss > guards.resident_bytes => {
                    terminate(child);
                    break Err(FormatFixtureError::ResidentSetExceeded);
                }
                Ok(_) => {}
                Err(error) => {
                    terminate(child);
                    break Err(error);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    if result.is_err() {
        terminate(child);
    }
    let stdout_joined = stdout_reader.join();
    let stderr_joined = stderr_reader.join();
    if stdout_joined.is_err() || stderr_joined.is_err() {
        return Err(FormatFixtureError::WorkerProtocol(
            "worker output reader panicked".into(),
        ));
    }
    result
}

#[cfg(target_os = "linux")]
fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::disallowed_methods,
    reason = "native format-worker host policy reads the supervised child RSS counter"
)]
fn worker_rss(pid: u32) -> Result<u64, FormatFixtureError> {
    let path = format!("/proc/{pid}/statm");
    crate::linux_rss::resident_bytes(std::path::Path::new(&path))
        .ok_or(FormatFixtureError::ResidentSetUnsupported)
}

fn worker_executable() -> Result<std::path::PathBuf, FormatFixtureError> {
    if let Some(path) = std::env::var_os("UMBER_FORMAT_WORKER") {
        return Ok(path.into());
    }
    let current = std::env::current_exe()
        .map_err(|error| FormatFixtureError::WorkerSpawn(error.to_string()))?;
    if current
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        == Some("deps")
    {
        return Ok(current
            .parent()
            .and_then(|path| path.parent())
            .expect("deps has parent")
            .join("umber"));
    }
    Ok(current)
}

impl Request {
    fn from_recipe(recipe: &FormatRecipe, identity: [u8; 32]) -> Result<Self, FormatFixtureError> {
        Ok(Self {
            protocol: PROTOCOL,
            identity,
            engine: engine_tag(recipe.engine),
            format_name: recipe.format_name.clone(),
            source_name: recipe.construction_source_name.clone(),
            source: recipe.construction_source.to_vec(),
            resources: recipe.resources.iter().map(Resource::from).collect(),
            distribution: recipe.distribution_identity.to_vec(),
            clock: [
                recipe.clock.time,
                recipe.clock.second,
                recipe.clock.day,
                recipe.clock.month,
                recipe.clock.year,
            ],
            fuel: recipe.guards.command_fuel,
            wall_ns: recipe
                .guards
                .wall_time
                .as_nanos()
                .try_into()
                .map_err(|_| FormatFixtureError::UnboundedGuard)?,
            resident_bytes: recipe.guards.resident_bytes,
        })
    }

    fn into_recipe(self) -> Result<FormatRecipe, String> {
        let recipe = FormatRecipe {
            engine: decode_engine(self.engine)?,
            format_name: self.format_name,
            construction_source_name: self.source_name,
            construction_source: Arc::from(self.source),
            resources: self
                .resources
                .into_iter()
                .map(Resource::into_resource)
                .collect::<Result<_, _>>()?,
            distribution_identity: Arc::from(self.distribution),
            clock: JobClock {
                time: self.clock[0],
                second: self.clock[1],
                day: self.clock[2],
                month: self.clock[3],
                year: self.clock[4],
            },
            guards: FormatGenerationGuards {
                command_fuel: self.fuel,
                wall_time: Duration::from_nanos(self.wall_ns),
                resident_bytes: self.resident_bytes,
            },
        };
        recipe
            .guards
            .validate()
            .map_err(|error| error.to_string())?;
        Ok(recipe)
    }
}

impl From<&FormatResource> for Resource {
    fn from(value: &FormatResource) -> Self {
        match value {
            FormatResource::Input {
                logical_name,
                source_kind,
                bytes,
            } => Self::Input(
                source_kind_tag(*source_kind),
                logical_name.clone(),
                bytes.to_vec(),
            ),
            FormatResource::Tfm {
                logical_name,
                bytes,
            } => Self::Tfm(logical_name.clone(), bytes.to_vec()),
        }
    }
}

impl Resource {
    fn into_resource(self) -> Result<FormatResource, String> {
        match self {
            Self::Input(tag, logical_name, bytes) => Ok(FormatResource::Input {
                logical_name,
                source_kind: decode_source_kind(tag)?,
                bytes: Arc::from(bytes),
            }),
            Self::Tfm(logical_name, bytes) => Ok(FormatResource::Tfm {
                logical_name,
                bytes: Arc::from(bytes),
            }),
        }
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "native format-worker IPC owns its process standard streams"
)]
pub fn run_format_worker() -> Result<(), String> {
    let request: Request =
        bincode::deserialize_from(std::io::stdin()).map_err(|error| error.to_string())?;
    if request.protocol != PROTOCOL {
        return Err("unsupported format-worker protocol".into());
    }
    let claimed_identity = request.identity;
    let recipe = request.into_recipe()?;
    let actual_identity = recipe
        .identity()
        .map_err(|error| error.to_string())?
        .key()
        .bytes();
    if actual_identity != claimed_identity {
        return Err("format-worker request identity mismatch".into());
    }
    let result = construct_format_in_worker(&recipe).map_err(|error| error.to_string());
    let image_sha256 = result
        .as_ref()
        .map_or([0; 32], |image| Sha256::digest(image).into());
    let response = Response {
        protocol: PROTOCOL,
        identity: actual_identity,
        image_sha256,
        result,
    };
    bincode::serialize_into(std::io::stdout(), &response).map_err(|error| error.to_string())
}

const fn engine_tag(engine: EngineMode) -> u8 {
    match engine {
        EngineMode::Tex82 => 1,
        EngineMode::ETex => 2,
        EngineMode::PdfTex => 3,
        EngineMode::Latex => 4,
        EngineMode::PdfLatex => 5,
    }
}
fn decode_engine(tag: u8) -> Result<EngineMode, String> {
    match tag {
        1 => Ok(EngineMode::Tex82),
        2 => Ok(EngineMode::ETex),
        3 => Ok(EngineMode::PdfTex),
        4 => Ok(EngineMode::Latex),
        5 => Ok(EngineMode::PdfLatex),
        _ => Err("invalid engine tag".into()),
    }
}
const fn source_kind_tag(kind: RegisteredSourceKind) -> u8 {
    match kind {
        RegisteredSourceKind::World => 1,
        RegisteredSourceKind::Generated => 2,
        RegisteredSourceKind::EditorFragment => 3,
        RegisteredSourceKind::ReadLine => 4,
    }
}
fn decode_source_kind(tag: u8) -> Result<RegisteredSourceKind, String> {
    match tag {
        1 => Ok(RegisteredSourceKind::World),
        2 => Ok(RegisteredSourceKind::Generated),
        3 => Ok(RegisteredSourceKind::EditorFragment),
        4 => Ok(RegisteredSourceKind::ReadLine),
        _ => Err("invalid source-kind tag".into()),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn helper(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn bounded helper")
    }

    fn collect(
        child: &mut Child,
        guards: FormatGenerationGuards,
        stdout_limit: usize,
        stderr_limit: usize,
    ) -> Result<CollectedWorkerOutput, FormatFixtureError> {
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        supervise_and_collect(child, guards, stdout, stderr, stdout_limit, stderr_limit)
    }

    fn guards(wall_time: Duration, resident_bytes: u64) -> FormatGenerationGuards {
        FormatGenerationGuards {
            command_fuel: 1,
            wall_time,
            resident_bytes,
        }
    }

    #[test]
    fn supervisor_kills_an_unresponsive_worker_inside_one_command() {
        let mut child = helper("exec sleep 30");
        assert!(matches!(
            collect(
                &mut child,
                guards(Duration::from_millis(10), u64::MAX),
                1024,
                1024
            ),
            Err(FormatFixtureError::WallTimeExceeded)
        ));
        assert!(child.try_wait().expect("reaped").is_some());
    }

    #[test]
    fn supervisor_rejects_crash_and_a_later_worker_can_succeed() {
        let mut crashed = helper("printf bounded-diagnostic >&2; exit 9");
        assert!(matches!(
            collect(
                &mut crashed,
                guards(Duration::from_secs(1), u64::MAX),
                1024,
                1024
            ),
            Err(FormatFixtureError::WorkerCrashed(Some(9), diagnostics))
                if diagnostics == "bounded-diagnostic"
        ));
        let mut retry = helper("exit 0");
        collect(
            &mut retry,
            guards(Duration::from_secs(1), u64::MAX),
            1024,
            1024,
        )
        .expect("retry is independent");
    }

    #[test]
    fn supervisor_enforces_rss_without_dangerous_allocation() {
        let mut child = helper("exec sleep 30");
        assert!(matches!(
            collect(&mut child, guards(Duration::from_secs(1), 1), 1024, 1024),
            Err(FormatFixtureError::ResidentSetExceeded)
        ));
        assert!(child.try_wait().expect("reaped").is_some());
    }

    #[test]
    fn drains_stdout_larger_than_pipe_capacity_while_supervising() {
        let mut child = helper("head -c 262144 /dev/zero");
        let output = collect(
            &mut child,
            guards(Duration::from_secs(2), 64 * 1024 * 1024),
            262144,
            1024,
        )
        .expect("large response completes");
        assert_eq!(output.stdout.len(), 262144);
    }

    #[test]
    fn drains_saturated_stderr_without_deadlock() {
        let mut child = helper("head -c 262144 /dev/zero >&2; printf response");
        let output = collect(
            &mut child,
            guards(Duration::from_secs(2), 64 * 1024 * 1024),
            1024,
            262144,
        )
        .expect("stderr pressure completes");
        assert_eq!(output.stdout, b"response");
        assert_eq!(output.stderr.len(), 262144);
    }

    #[test]
    fn output_limit_failure_kills_and_reaps_worker() {
        let mut child = helper("head -c 262144 /dev/zero; exec sleep 30");
        assert!(matches!(
            collect(
                &mut child,
                guards(Duration::from_secs(2), 64 * 1024 * 1024),
                64 * 1024,
                1024
            ),
            Err(FormatFixtureError::WorkerProtocol(message))
                if message.contains("stdout exceeded")
        ));
        assert!(child.try_wait().expect("reaped").is_some());
    }

    #[test]
    fn request_round_trip_recomputes_authenticated_identity_and_fuel() {
        let recipe = FormatRecipe::raw_tex82();
        let identity = recipe.identity().expect("identity").key().bytes();
        let bytes = bincode::serialize(&Request::from_recipe(&recipe, identity).expect("request"))
            .expect("encode");
        let decoded: Request = bincode::deserialize(&bytes).expect("decode");
        let rebuilt = decoded.into_recipe().expect("recipe");
        assert_eq!(
            rebuilt.identity().expect("identity").key().bytes(),
            identity
        );
        assert_eq!(rebuilt.guards.command_fuel, recipe.guards.command_fuel);
        assert_eq!(rebuilt.resources, recipe.resources);
    }
}

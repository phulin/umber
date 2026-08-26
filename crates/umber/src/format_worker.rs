//! Authenticated process boundary for bounded format construction.

use std::io::ErrorKind;
use std::io::{Read, Write};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bincode::Options;
#[cfg(target_os = "linux")]
use rustix::event::{PollFd, PollFlags, Timespec, poll};
#[cfg(target_os = "linux")]
use rustix::fd::OwnedFd;
#[cfg(target_os = "linux")]
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(target_os = "linux")]
use subtle::ConstantTimeEq;
use tex_command::RegisteredSourceKind;
#[cfg(test)]
use tex_state::World;
use tex_state::{DetachedFormatImage, JobClock};
#[cfg(target_os = "linux")]
use zeroize::Zeroizing;

use crate::EngineMode;
use crate::format_fixture::{
    ConstructionResult, FormatFixtureError, FormatGenerationGuards, FormatRecipe, FormatResource,
    construct_format_in_worker,
};

const PROTOCOL: u32 = 4;
const AUTH_KEY_BYTES: usize = 32;
const REQUEST_PREFIX: &[u8] = b"\0UMBER-FORMAT-WORKER-REQUEST-V3\0";
const RESPONSE_PREFIX: &[u8] = b"\0UMBER-FORMAT-WORKER-RESPONSE-V3\0";
const TEST_WORKER_ENV: &str = "UMBER_INTERNAL_CURRENT_IMAGE_TEST_WORKER";
const PRODUCTION_WORKER_ARGUMENT: &str = "__format-worker";
const AUTH_DOMAIN: &[u8] = b"umber.format-worker.response.v3\0";
const FRAME_LENGTH_BYTES: usize = size_of::<u64>();
const MAX_WORKER_REQUEST_BYTES: usize = crate::SessionLimits::FORMAT_IMAGE_BYTES + 16 * 1024 * 1024;
const MAX_WORKER_STDOUT_BYTES: usize =
    crate::SessionLimits::FORMAT_IMAGE_BYTES + tex_oracle::MAX_BUNDLE_BYTES + 64 * 1024;
const MAX_WORKER_RESPONSE_BYTES: usize =
    MAX_WORKER_STDOUT_BYTES - RESPONSE_PREFIX.len() - FRAME_LENGTH_BYTES;
const MAX_WORKER_STDERR_BYTES: usize = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Request {
    protocol: u32,
    identity: [u8; 32],
    engine: u8,
    hyphenation_exception_capacity: u64,
    format_name: String,
    format_ident_name: String,
    source_name: String,
    source: Vec<u8>,
    resources: Vec<Resource>,
    distribution: Vec<u8>,
    clock: [i32; 5],
    construction_interaction: u8,
    construction_error_context_widths: [u64; 2],
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
    evidence_sha256: [u8; 32],
    result: Result<ConstructionResultWire, String>,
    authenticator: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
struct ConstructionResultWire {
    image: Vec<u8>,
    evidence: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct FormatWorkerLauncher {
    route: WorkerRoute,
    #[cfg(test)]
    spawn_counter: Option<Arc<AtomicUsize>>,
}

impl PartialEq for FormatWorkerLauncher {
    fn eq(&self, other: &Self) -> bool {
        self.route == other.route
    }
}

impl Eq for FormatWorkerLauncher {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum WorkerRoute {
    Production,
    Libtest(&'static str),
}

impl FormatWorkerLauncher {
    /// Declares that the current image calls [`dispatch_format_worker`] before
    /// parsing ordinary application arguments.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            route: WorkerRoute::Production,
            #[cfg(test)]
            spawn_counter: None,
        }
    }

    #[doc(hidden)]
    #[must_use]
    pub const fn registered_libtest(test_name: &'static str) -> Self {
        Self {
            route: WorkerRoute::Libtest(test_name),
            #[cfg(test)]
            spawn_counter: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_spawn_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.spawn_counter = Some(counter);
        self
    }
}

/// Dispatches an authenticated production worker invocation at process entry.
///
/// Returns `None` for an ordinary application invocation. A caller must handle
/// `Some` immediately and must not continue into its normal main function.
#[must_use]
pub fn dispatch_format_worker() -> Option<Result<(), String>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    match classify_production_worker_arguments(arguments) {
        ProductionWorkerInvocation::Exact => Some(run_format_worker()),
        ProductionWorkerInvocation::Malformed => Some(Err(
            "reserved __format-worker invocation accepts no trailing arguments".into(),
        )),
        ProductionWorkerInvocation::Unrelated => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProductionWorkerInvocation {
    Exact,
    Malformed,
    Unrelated,
}

fn classify_production_worker_arguments(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> ProductionWorkerInvocation {
    match arguments.next() {
        Some(argument) if argument == PRODUCTION_WORKER_ARGUMENT => {
            if arguments.next().is_none() {
                ProductionWorkerInvocation::Exact
            } else {
                ProductionWorkerInvocation::Malformed
            }
        }
        _ => ProductionWorkerInvocation::Unrelated,
    }
}

pub(crate) fn construct(
    launcher: Option<&FormatWorkerLauncher>,
    recipe: &FormatRecipe,
) -> Result<ConstructionResult, FormatFixtureError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = recipe;
        return Err(FormatFixtureError::ResidentSetUnsupported);
    }
    #[cfg(target_os = "linux")]
    {
        let launcher = launcher.ok_or(FormatFixtureError::WorkerBootstrapUnregistered)?;
        #[cfg(test)]
        if let Some(counter) = &launcher.spawn_counter {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        let identity = recipe.identity()?.key().bytes();
        let request = Request::from_recipe(recipe, identity)?;
        let request_bytes = bincode::serialize(&request)
            .map_err(|error| FormatFixtureError::WorkerProtocol(error.to_string()))?;
        if request_bytes.len() > MAX_WORKER_REQUEST_BYTES {
            return Err(FormatFixtureError::WorkerProtocol(
                "format-worker request exceeds protocol limit".into(),
            ));
        }
        let executable = worker_executable()?;
        let executable_path = format!("/proc/self/fd/{}", executable.as_raw_fd());
        let mut auth_key = Zeroizing::new([0_u8; AUTH_KEY_BYTES]);
        getrandom::fill(&mut *auth_key)
            .map_err(|error| FormatFixtureError::WorkerSpawn(error.to_string()))?;
        let mut command = Command::new(executable_path);
        configure_worker_command(&mut command, launcher);
        let mut child = command
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
        if let Err(error) = arm_resident_set_guard(&mut child, recipe.guards.resident_bytes) {
            terminate(&mut child);
            return Err(error);
        }
        let writer_key = Zeroizing::new(*auth_key);
        let writer = std::thread::spawn(move || {
            stdin
                .write_all(&*writer_key)
                .and_then(|()| write_frame(&mut stdin, REQUEST_PREFIX, &request_bytes))
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
        let response_bytes = find_frame(
            &collected.stdout,
            RESPONSE_PREFIX,
            MAX_WORKER_RESPONSE_BYTES,
        )
        .map_err(|error| FormatFixtureError::WorkerProtocol(error.to_owned()))?;
        let response: Response = deserialize_bounded(response_bytes, MAX_WORKER_RESPONSE_BYTES)
            .map_err(FormatFixtureError::WorkerProtocol)?;
        validate_response(response, identity, &auth_key)
    }
}

#[cfg(target_os = "linux")]
struct CollectedWorkerOutput {
    stdout: Vec<u8>,
    #[allow(dead_code)]
    stderr: Vec<u8>,
}

#[cfg(target_os = "linux")]
enum SupervisionAction {
    Complete(Result<CollectedWorkerOutput, FormatFixtureError>),
    WallTimeExceeded,
    CheckResidentSet,
    WaitForPipes,
}

#[cfg(target_os = "linux")]
enum WorkerResidentSetError {
    ProcessVanished,
    Unsupported,
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct ReaderCompletions {
    stdout: Option<Result<Vec<u8>, FormatFixtureError>>,
    stderr: Option<Result<Vec<u8>, FormatFixtureError>>,
}

#[cfg(target_os = "linux")]
struct ReaderEvents {
    completions: Mutex<ReaderCompletions>,
    published: Condvar,
}

#[cfg(target_os = "linux")]
struct ProcessExitEvent {
    pidfd: OwnedFd,
}

#[cfg(target_os = "linux")]
impl ProcessExitEvent {
    fn open(child: &Child) -> Result<Self, FormatFixtureError> {
        let pid =
            Pid::from_raw(child.id() as i32).ok_or(FormatFixtureError::ResidentSetUnsupported)?;
        let pidfd = pidfd_open(pid, PidfdFlags::empty())
            .map_err(|_| FormatFixtureError::ResidentSetUnsupported)?;
        Ok(Self { pidfd })
    }

    fn is_ready(&self) -> Result<bool, FormatFixtureError> {
        let mut descriptor = [PollFd::new(&self.pidfd, PollFlags::IN)];
        poll(
            &mut descriptor,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .map(|ready| ready == 1)
        .map_err(|error| FormatFixtureError::Worker(error.to_string()))
    }

    #[cfg(test)]
    fn wait_until_ready(&self) -> Result<bool, FormatFixtureError> {
        let mut descriptor = [PollFd::new(&self.pidfd, PollFlags::IN)];
        poll(
            &mut descriptor,
            Some(&Timespec {
                tv_sec: 1,
                tv_nsec: 0,
            }),
        )
        .map(|ready| ready == 1)
        .map_err(|error| FormatFixtureError::Worker(error.to_string()))
    }
}

#[cfg(target_os = "linux")]
impl ReaderEvents {
    fn new() -> Self {
        Self {
            completions: Mutex::new(ReaderCompletions::default()),
            published: Condvar::new(),
        }
    }

    fn publish(
        &self,
        is_stdout: bool,
        result: Result<Vec<u8>, FormatFixtureError>,
    ) -> Result<(), FormatFixtureError> {
        let mut completions = self.lock()?;
        let slot = if is_stdout {
            &mut completions.stdout
        } else {
            &mut completions.stderr
        };
        *slot = Some(result);
        self.published.notify_one();
        Ok(())
    }

    fn decide(
        &self,
        mut status: Option<std::process::ExitStatus>,
        wall_time_exceeded: bool,
        observe_deadline_exit: impl FnOnce() -> Result<
            Option<std::process::ExitStatus>,
            FormatFixtureError,
        >,
    ) -> Result<SupervisionAction, FormatFixtureError> {
        let mut completions = self.lock()?;
        if wall_time_exceeded && status.is_none() {
            status = observe_deadline_exit()?;
        }
        if matches!(completions.stdout, Some(Err(_)))
            && let Some(Err(error)) = completions.stdout.take()
        {
            return Ok(SupervisionAction::Complete(Err(error)));
        }
        if matches!(completions.stderr, Some(Err(_)))
            && let Some(Err(error)) = completions.stderr.take()
        {
            return Ok(SupervisionAction::Complete(Err(error)));
        }
        if let Some(observed) = status
            && completions.stdout.is_some()
            && completions.stderr.is_some()
        {
            let stdout = completions
                .stdout
                .take()
                .expect("checked resolved stdout")
                .expect("checked successful stdout");
            let stderr = completions
                .stderr
                .take()
                .expect("checked resolved stderr")
                .expect("checked successful stderr");
            return Ok(SupervisionAction::Complete(if observed.success() {
                Ok(CollectedWorkerOutput { stdout, stderr })
            } else {
                Err(FormatFixtureError::WorkerCrashed(
                    observed.code(),
                    String::from_utf8_lossy(&stderr).into_owned(),
                ))
            }));
        }
        if wall_time_exceeded {
            return Ok(SupervisionAction::WallTimeExceeded);
        }
        Ok(if status.is_some() {
            SupervisionAction::WaitForPipes
        } else {
            SupervisionAction::CheckResidentSet
        })
    }

    fn wait_for_publication(&self, duration: Duration) -> Result<(), FormatFixtureError> {
        let completions = self.lock()?;
        let _ = self
            .published
            .wait_timeout(completions, duration)
            .map_err(|_| reader_state_poisoned())?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ReaderCompletions>, FormatFixtureError> {
        self.completions.lock().map_err(|_| reader_state_poisoned())
    }
}

#[cfg(target_os = "linux")]
fn reader_state_poisoned() -> FormatFixtureError {
    FormatFixtureError::WorkerProtocol("worker reader state poisoned".into())
}

#[cfg(target_os = "linux")]
fn reconcile_process_disappearance(
    status: &mut Option<std::process::ExitStatus>,
    observe_exit: impl FnOnce() -> std::io::Result<Option<std::process::ExitStatus>>,
) -> Result<(), FormatFixtureError> {
    match observe_exit() {
        Ok(Some(observed)) => {
            *status = Some(observed);
            Ok(())
        }
        Ok(None) => Err(FormatFixtureError::ResidentSetUnsupported),
        Err(error) => Err(FormatFixtureError::Worker(error.to_string())),
    }
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
    stdout: impl Read + Send + 'static,
    stderr: ChildStderr,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<CollectedWorkerOutput, FormatFixtureError> {
    let exit_event = match ProcessExitEvent::open(child) {
        Ok(exit_event) => exit_event,
        Err(error) => {
            terminate(child);
            return Err(error);
        }
    };
    let events = Arc::new(ReaderEvents::new());
    let stdout_events = Arc::clone(&events);
    let stdout_reader = std::thread::spawn(move || {
        let _ = stdout_events.publish(true, read_bounded(stdout, stdout_limit, "worker stdout"));
    });
    let stderr_events = Arc::clone(&events);
    let stderr_reader = std::thread::spawn(move || {
        let _ = stderr_events.publish(false, read_bounded(stderr, stderr_limit, "worker stderr"));
    });
    let started = Instant::now();
    let mut status = None;
    let result = loop {
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
        let wall_time_exceeded = started.elapsed() >= guards.wall_time;
        let action = match events.decide(status, wall_time_exceeded, || {
            if exit_event.is_ready()? {
                child
                    .wait()
                    .map(Some)
                    .map_err(|error| FormatFixtureError::Worker(error.to_string()))
            } else {
                Ok(None)
            }
        }) {
            Ok(action) => action,
            Err(error) => {
                terminate(child);
                break Err(error);
            }
        };
        match action {
            SupervisionAction::Complete(completed) => break completed,
            SupervisionAction::WallTimeExceeded => {
                terminate(child);
                break Err(FormatFixtureError::WallTimeExceeded);
            }
            SupervisionAction::CheckResidentSet => match worker_rss(child.id()) {
                Ok(rss) if rss > guards.resident_bytes => {
                    terminate(child);
                    break Err(FormatFixtureError::ResidentSetExceeded);
                }
                Ok(_) => {}
                Err(WorkerResidentSetError::ProcessVanished) => {
                    if let Err(error) =
                        reconcile_process_disappearance(&mut status, || child.try_wait())
                    {
                        terminate(child);
                        break Err(error);
                    }
                    continue;
                }
                Err(WorkerResidentSetError::Unsupported) => {
                    terminate(child);
                    break Err(FormatFixtureError::ResidentSetUnsupported);
                }
            },
            SupervisionAction::WaitForPipes => {}
        }
        let remaining = guards.wall_time.saturating_sub(started.elapsed());
        if let Err(error) = events.wait_for_publication(remaining.min(Duration::from_millis(2))) {
            terminate(child);
            break Err(error);
        }
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
fn worker_rss(pid: u32) -> Result<u64, WorkerResidentSetError> {
    let path = format!("/proc/{pid}/statm");
    resident_set_from_statm_result(std::fs::read_to_string(path))
}

#[cfg(target_os = "linux")]
fn arm_resident_set_guard(
    child: &mut Child,
    resident_bytes: u64,
) -> Result<(), FormatFixtureError> {
    match worker_rss(child.id()) {
        Ok(rss) if rss > resident_bytes => Err(FormatFixtureError::ResidentSetExceeded),
        Ok(_) => Ok(()),
        Err(WorkerResidentSetError::ProcessVanished) => {
            reconcile_process_disappearance(&mut None, || child.try_wait())
        }
        Err(WorkerResidentSetError::Unsupported) => Err(FormatFixtureError::ResidentSetUnsupported),
    }
}

#[cfg(target_os = "linux")]
fn resident_set_from_statm_result(
    statm: std::io::Result<String>,
) -> Result<u64, WorkerResidentSetError> {
    let statm = statm.map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            WorkerResidentSetError::ProcessVanished
        } else {
            WorkerResidentSetError::Unsupported
        }
    })?;
    crate::linux_rss::resident_bytes_from_statm(&statm).ok_or(WorkerResidentSetError::Unsupported)
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::disallowed_methods,
    reason = "the native supervisor opens the trusted executable once to remove pathname substitution and TOCTOU"
)]
fn worker_executable() -> Result<File, FormatFixtureError> {
    open_trusted_current_image("/proc/self/exe")
}

#[cfg(target_os = "linux")]
#[allow(
    clippy::disallowed_methods,
    reason = "native format-worker attestation opens the kernel-owned current-image link"
)]
fn open_trusted_current_image(path: &str) -> Result<File, FormatFixtureError> {
    File::open(path).map_err(|error| FormatFixtureError::WorkerSpawn(error.to_string()))
}

#[allow(
    clippy::disallowed_methods,
    reason = "native attestation distinguishes Cargo test harness images from the production CLI"
)]
fn configure_worker_command(command: &mut Command, launcher: &FormatWorkerLauncher) {
    match launcher.route {
        WorkerRoute::Production => {
            command.arg(PRODUCTION_WORKER_ARGUMENT);
        }
        WorkerRoute::Libtest(test_name) => {
            command.env(TEST_WORKER_ENV, "1");
            command.args([test_name, "--exact", "--test-threads=1"]);
        }
    }
}

pub(crate) fn run_test_bootstrap() {
    if std::env::var_os(TEST_WORKER_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let status = match run_format_worker() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("umber format worker: {error}");
            70
        }
    };
    std::process::exit(status);
}

fn write_frame(output: &mut impl Write, prefix: &[u8], payload: &[u8]) -> std::io::Result<()> {
    let length = u64::try_from(payload.len())
        .map_err(|_| std::io::Error::other("format-worker frame length overflow"))?;
    output.write_all(prefix)?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(payload)
}

fn read_frame(input: &mut impl Read, prefix: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut observed_prefix = vec![0_u8; prefix.len()];
    input
        .read_exact(&mut observed_prefix)
        .map_err(|_| "truncated format-worker frame prefix")?;
    if observed_prefix != prefix {
        return Err("invalid format-worker frame prefix".into());
    }
    let mut encoded_length = [0_u8; FRAME_LENGTH_BYTES];
    input
        .read_exact(&mut encoded_length)
        .map_err(|_| "truncated format-worker frame length")?;
    let length = usize::try_from(u64::from_le_bytes(encoded_length))
        .map_err(|_| "format-worker frame length does not fit this host")?;
    if length > limit {
        return Err("format-worker frame exceeds protocol limit".into());
    }
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(length)
        .map_err(|_| "format-worker frame allocation failed")?;
    payload.resize(length, 0);
    input
        .read_exact(&mut payload)
        .map_err(|_| "truncated format-worker frame payload")?;
    Ok(payload)
}

fn find_frame<'a>(input: &'a [u8], prefix: &[u8], limit: usize) -> Result<&'a [u8], &'static str> {
    let prefix_start = input
        .windows(prefix.len())
        .position(|window| window == prefix)
        .ok_or("missing format-worker response frame")?;
    let length_start = prefix_start
        .checked_add(prefix.len())
        .ok_or("format-worker response frame overflow")?;
    let length_end = length_start
        .checked_add(FRAME_LENGTH_BYTES)
        .ok_or("format-worker response frame overflow")?;
    let encoded_length: [u8; FRAME_LENGTH_BYTES] = input
        .get(length_start..length_end)
        .ok_or("truncated format-worker response length")?
        .try_into()
        .expect("fixed-width slice");
    let length = usize::try_from(u64::from_le_bytes(encoded_length))
        .map_err(|_| "format-worker response length does not fit this host")?;
    if length > limit {
        return Err("format-worker response exceeds protocol limit");
    }
    let payload_end = length_end
        .checked_add(length)
        .ok_or("format-worker response frame overflow")?;
    input
        .get(length_end..payload_end)
        .ok_or("truncated format-worker response payload")
}

fn deserialize_bounded<T: for<'de> Deserialize<'de>>(
    payload: &[u8],
    limit: usize,
) -> Result<T, String> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(u64::try_from(limit).map_err(|_| "format-worker protocol limit overflow")?)
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(|error| error.to_string())
}

fn response_authenticator(
    key: &[u8; AUTH_KEY_BYTES],
    protocol: u32,
    identity: &[u8; 32],
    image_sha256: &[u8; 32],
    evidence_sha256: &[u8; 32],
    result: &Result<ConstructionResultWire, String>,
) -> Result<[u8; 32], String> {
    let encoded = bincode::serialize(&(protocol, identity, image_sha256, evidence_sha256, result))
        .map_err(|error| error.to_string())?;
    let mut ipad = Zeroizing::new([0x36_u8; 64]);
    let mut opad = Zeroizing::new([0x5c_u8; 64]);
    for (index, byte) in key.iter().enumerate() {
        ipad[index] ^= byte;
        opad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(ipad.as_slice());
    inner.update(AUTH_DOMAIN);
    inner.update(encoded);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad.as_slice());
    outer.update(inner);
    Ok(outer.finalize().into())
}

fn validate_response(
    response: Response,
    identity: [u8; 32],
    auth_key: &[u8; AUTH_KEY_BYTES],
) -> Result<ConstructionResult, FormatFixtureError> {
    let expected_authenticator = response_authenticator(
        auth_key,
        response.protocol,
        &response.identity,
        &response.image_sha256,
        &response.evidence_sha256,
        &response.result,
    )
    .map_err(FormatFixtureError::WorkerProtocol)?;
    if response.protocol != PROTOCOL
        || response.identity != identity
        || !bool::from(response.authenticator.ct_eq(&expected_authenticator))
    {
        return Err(FormatFixtureError::WorkerIdentityMismatch);
    }
    let result = response.result.map_err(FormatFixtureError::Worker)?;
    if <[u8; 32]>::from(Sha256::digest(&result.image)) != response.image_sha256
        || <[u8; 32]>::from(Sha256::digest(&result.evidence)) != response.evidence_sha256
    {
        return Err(FormatFixtureError::WorkerIdentityMismatch);
    }
    DetachedFormatImage::try_from_bytes(result.image.clone())
        .map_err(|error| FormatFixtureError::Format(error.to_string()))?;
    tex_oracle::decode_oracle_bundle(&result.evidence).map_err(FormatFixtureError::Evidence)?;
    Ok(ConstructionResult {
        image: result.image,
        evidence: result.evidence,
    })
}

fn read_auth_key(channel: &mut impl Read) -> Result<Zeroizing<[u8; AUTH_KEY_BYTES]>, String> {
    let mut key = Zeroizing::new([0_u8; AUTH_KEY_BYTES]);
    channel
        .read_exact(&mut *key)
        .map_err(|_| "invalid worker authentication channel")?;
    Ok(key)
}

impl Request {
    fn from_recipe(recipe: &FormatRecipe, identity: [u8; 32]) -> Result<Self, FormatFixtureError> {
        Ok(Self {
            protocol: PROTOCOL,
            identity,
            engine: engine_tag(recipe.engine),
            hyphenation_exception_capacity: recipe
                .hyphenation_exception_capacity
                .try_into()
                .map_err(|_| FormatFixtureError::UnboundedGuard)?,
            format_name: recipe.format_name.clone(),
            format_ident_name: recipe.format_ident_name.clone(),
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
            construction_interaction: interaction_tag(recipe.construction_interaction),
            construction_error_context_widths: [
                recipe.construction_error_context_widths.error_line() as u64,
                recipe.construction_error_context_widths.half_error_line() as u64,
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
            hyphenation_exception_capacity: self
                .hyphenation_exception_capacity
                .try_into()
                .map_err(|_| "hyphenation exception capacity is not host-representable")?,
            format_name: self.format_name,
            format_ident_name: self.format_ident_name,
            construction_source_name: self.source_name,
            construction_source: self.source,
            resources: self
                .resources
                .into_iter()
                .map(Resource::into_resource)
                .collect::<Result<_, _>>()?,
            distribution_identity: self.distribution,
            clock: JobClock {
                time: self.clock[0],
                second: self.clock[1],
                day: self.clock[2],
                month: self.clock[3],
                year: self.clock[4],
            },
            construction_interaction: decode_interaction(self.construction_interaction)?,
            construction_error_context_widths: tex_state::print::ErrorContextWidths::new(
                self.construction_error_context_widths[0]
                    .try_into()
                    .map_err(|_| "construction error-line width is not host-representable")?,
                self.construction_error_context_widths[1]
                    .try_into()
                    .map_err(|_| "construction half-error-line width is not host-representable")?,
            )
            .ok_or_else(|| "invalid construction error-context widths".to_owned())?,
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
                bytes,
            }),
            Self::Tfm(logical_name, bytes) => Ok(FormatResource::Tfm {
                logical_name,
                bytes,
            }),
        }
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "native format-worker IPC owns its process standard streams"
)]
pub fn run_format_worker() -> Result<(), String> {
    let mut request_channel = std::io::stdin();
    let auth_key = read_auth_key(&mut request_channel)?;
    let request_bytes = read_frame(
        &mut request_channel,
        REQUEST_PREFIX,
        MAX_WORKER_REQUEST_BYTES,
    )?;
    let request: Request = deserialize_bounded(&request_bytes, MAX_WORKER_REQUEST_BYTES)?;
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
    let result = construct_format_in_worker(&recipe)
        .map(|result| ConstructionResultWire {
            image: result.image,
            evidence: result.evidence,
        })
        .map_err(|error| error.to_string());
    let image_sha256 = result
        .as_ref()
        .map_or([0; 32], |result| Sha256::digest(&result.image).into());
    let evidence_sha256 = result
        .as_ref()
        .map_or([0; 32], |result| Sha256::digest(&result.evidence).into());
    let authenticator = response_authenticator(
        &auth_key,
        PROTOCOL,
        &actual_identity,
        &image_sha256,
        &evidence_sha256,
        &result,
    )?;
    let response = Response {
        protocol: PROTOCOL,
        identity: actual_identity,
        image_sha256,
        evidence_sha256,
        result,
        authenticator,
    };
    let mut response_channel = std::io::stdout().lock();
    let response_bytes = bincode::serialize(&response).map_err(|error| error.to_string())?;
    if response_bytes.len() > MAX_WORKER_RESPONSE_BYTES {
        return Err("format-worker response exceeds protocol limit".into());
    }
    write_frame(&mut response_channel, RESPONSE_PREFIX, &response_bytes)
        .map_err(|error| error.to_string())
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
const fn interaction_tag(mode: tex_state::InteractionMode) -> u8 {
    match mode {
        tex_state::InteractionMode::Batch => 0,
        tex_state::InteractionMode::Nonstop => 1,
        tex_state::InteractionMode::Scroll => 2,
        tex_state::InteractionMode::ErrorStop => 3,
    }
}
fn decode_interaction(tag: u8) -> Result<tex_state::InteractionMode, String> {
    match tag {
        0 => Ok(tex_state::InteractionMode::Batch),
        1 => Ok(tex_state::InteractionMode::Nonstop),
        2 => Ok(tex_state::InteractionMode::Scroll),
        3 => Ok(tex_state::InteractionMode::ErrorStop),
        _ => Err("invalid construction interaction tag".into()),
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
    use crate::FormatCacheStore;
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Seek, SeekFrom};
    use std::os::unix::process::ExitStatusExt;
    use std::sync::Barrier;
    use tempfile::TempDir;

    #[test]
    fn production_worker_argument_classification_is_fail_closed() {
        let classify = |arguments: &[&str]| {
            classify_production_worker_arguments(arguments.iter().map(OsString::from))
        };

        assert_eq!(
            classify(&[PRODUCTION_WORKER_ARGUMENT]),
            ProductionWorkerInvocation::Exact
        );
        assert_eq!(
            classify(&[PRODUCTION_WORKER_ARGUMENT, "trailing"]),
            ProductionWorkerInvocation::Malformed
        );
        assert_eq!(classify(&[]), ProductionWorkerInvocation::Unrelated);
        assert_eq!(
            classify(&["__format-worker-unrelated", "trailing"]),
            ProductionWorkerInvocation::Unrelated
        );
    }

    #[test]
    fn test_image_bootstrap_selects_exactly_one_worker_entry() {
        let mut command = Command::new("/proc/self/exe");
        configure_worker_command(
            &mut command,
            &FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap"),
        );
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            [
                "umber_format_worker_bootstrap",
                "--exact",
                "--test-threads=1",
            ]
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == TEST_WORKER_ENV)
                .and_then(|(_, value)| value),
            Some(std::ffi::OsStr::new("1"))
        );
    }

    #[test]
    fn unregistered_consumer_fails_before_executable_selection() {
        assert!(matches!(
            construct(None, &FormatRecipe::raw_tex82()),
            Err(FormatFixtureError::WorkerBootstrapUnregistered)
        ));
    }

    #[test]
    fn production_and_libtest_routes_are_explicit_and_distinct() {
        let mut production = Command::new("/proc/self/exe");
        configure_worker_command(&mut production, &FormatWorkerLauncher::production());
        assert_eq!(
            production.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new(PRODUCTION_WORKER_ARGUMENT)]
        );
        assert!(
            production
                .get_envs()
                .all(|(name, _)| name != TEST_WORKER_ENV)
        );

        let mut libtest = Command::new("/proc/self/exe");
        configure_worker_command(
            &mut libtest,
            &FormatWorkerLauncher::registered_libtest("downstream_bootstrap"),
        );
        assert_eq!(
            libtest.get_args().collect::<Vec<_>>(),
            [
                std::ffi::OsStr::new("downstream_bootstrap"),
                std::ffi::OsStr::new("--exact"),
                std::ffi::OsStr::new("--test-threads=1"),
            ]
        );
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "adversarial executable-selection test owns a temporary native filesystem layout"
    )]
    fn current_image_attestation_ignores_stale_and_wrong_siblings() {
        let root = TempDir::new().expect("temporary executable layout");
        let trusted_path = root.path().join("current-image");
        let sibling_path = root.path().join("umber");
        fs::write(&trusted_path, b"trusted-current-image").expect("trusted image");
        fs::write(&sibling_path, b"stale-same-version-worker").expect("stale sibling");

        let mut anchored =
            open_trusted_current_image(trusted_path.to_str().expect("utf8 path")).expect("open");
        let replacement_path = root.path().join("replacement");
        fs::write(&replacement_path, b"replacement-after-open").expect("replacement image");
        fs::rename(&replacement_path, &trusted_path).expect("replace pathname");
        fs::write(&sibling_path, b"wrong-build-worker").expect("replace wrong sibling");

        let mut selected = Vec::new();
        anchored.seek(SeekFrom::Start(0)).expect("rewind");
        anchored.read_to_end(&mut selected).expect("read anchor");
        assert_eq!(selected, b"trusted-current-image");
        assert_ne!(selected, fs::read(&sibling_path).expect("read sibling"));
    }

    #[test]
    fn forged_decoder_valid_response_cannot_publish() {
        let recipe = FormatRecipe::raw_tex82();
        let identity = recipe.identity().expect("identity");
        let identity_bytes = identity.key().bytes();
        let constructed = construct_format_in_worker(&recipe).expect("decoder-valid image");
        let image = tex_state::DetachedFormatImage::try_from_bytes(constructed.image.clone())
            .expect("forgery is a detached format image");
        tex_state::with_materialized_format(
            crate::engine_interner_budget(),
            World::memory(),
            image,
            |_| (),
        )
        .expect("forgery is decoder-valid");

        let parent_key = [0x19; AUTH_KEY_BYTES];
        let attacker_key = [0x73; AUTH_KEY_BYTES];
        let image_sha256 = Sha256::digest(&constructed.image).into();
        let evidence_sha256 = Sha256::digest(&constructed.evidence).into();
        let result = Ok(ConstructionResultWire {
            image: constructed.image,
            evidence: constructed.evidence,
        });
        let authenticator = response_authenticator(
            &attacker_key,
            PROTOCOL,
            &identity_bytes,
            &image_sha256,
            &evidence_sha256,
            &result,
        )
        .expect("attacker response");
        let response = Response {
            protocol: PROTOCOL,
            identity: identity_bytes,
            image_sha256,
            evidence_sha256,
            result,
            authenticator,
        };

        let cache_root = TempDir::new().expect("cache root");
        let cache = FormatCacheStore::new(cache_root.path());
        let accepted = validate_response(response, identity_bytes, &parent_key);
        if let Ok(result) = &accepted {
            cache
                .store_entry(&identity, &result.image, &result.evidence, |bytes| {
                    tex_oracle::decode_oracle_bundle(bytes).map(|_| ())
                })
                .expect("publish accepted image");
        }
        assert!(matches!(
            accepted,
            Err(FormatFixtureError::WorkerIdentityMismatch)
        ));
        assert!(
            cache
                .load_entry(&identity, |bytes| tex_oracle::decode_oracle_bundle(bytes)
                    .map(|_| ()))
                .expect("cache remains readable")
                .is_none(),
            "a forged decoder-valid worker response must leave no cache entry"
        );
    }

    #[test]
    fn missing_tampered_and_cross_identity_evidence_are_rejected() {
        let recipe = FormatRecipe::raw_tex82();
        let identity = recipe.identity().expect("identity").key().bytes();
        let other_identity = FormatRecipe::raw_etex26()
            .identity()
            .expect("other identity")
            .key()
            .bytes();
        let constructed = construct_format_in_worker(&recipe).expect("construction");
        let key = [0x2d; AUTH_KEY_BYTES];

        let result = Ok(ConstructionResultWire {
            image: constructed.image.clone(),
            evidence: Vec::new(),
        });
        let image_sha256 = Sha256::digest(&constructed.image).into();
        let evidence_sha256 = Sha256::digest([]).into();
        let authenticator = response_authenticator(
            &key,
            PROTOCOL,
            &identity,
            &image_sha256,
            &evidence_sha256,
            &result,
        )
        .expect("missing-evidence authenticator");
        assert!(matches!(
            validate_response(
                Response {
                    protocol: PROTOCOL,
                    identity,
                    image_sha256,
                    evidence_sha256,
                    result,
                    authenticator,
                },
                identity,
                &key,
            ),
            Err(FormatFixtureError::Evidence(_))
        ));

        let valid_result = Ok(ConstructionResultWire {
            image: constructed.image,
            evidence: constructed.evidence,
        });
        let valid_payload = valid_result.as_ref().expect("valid construction result");
        let image_sha256 = Sha256::digest(&valid_payload.image).into();
        let evidence_sha256 = Sha256::digest(&valid_payload.evidence).into();
        let authenticator = response_authenticator(
            &key,
            PROTOCOL,
            &other_identity,
            &image_sha256,
            &evidence_sha256,
            &valid_result,
        )
        .expect("cross-identity authenticator");
        assert!(matches!(
            validate_response(
                Response {
                    protocol: PROTOCOL,
                    identity: other_identity,
                    image_sha256,
                    evidence_sha256,
                    result: valid_result.clone(),
                    authenticator,
                },
                identity,
                &key,
            ),
            Err(FormatFixtureError::WorkerIdentityMismatch)
        ));

        let authenticator = response_authenticator(
            &key,
            PROTOCOL,
            &identity,
            &image_sha256,
            &evidence_sha256,
            &valid_result,
        )
        .expect("valid authenticator");
        let mut tampered = valid_result;
        tampered
            .as_mut()
            .expect("valid construction result")
            .evidence
            .push(0);
        assert!(matches!(
            validate_response(
                Response {
                    protocol: PROTOCOL,
                    identity,
                    image_sha256,
                    evidence_sha256,
                    result: tampered,
                    authenticator,
                },
                identity,
                &key,
            ),
            Err(FormatFixtureError::WorkerIdentityMismatch)
        ));
    }

    #[test]
    fn request_and_response_frames_are_bounded_before_payload_allocation() {
        let mut valid = Vec::new();
        write_frame(&mut valid, REQUEST_PREFIX, b"request").expect("write request");
        assert_eq!(
            read_frame(&mut valid.as_slice(), REQUEST_PREFIX, 7).expect("read request"),
            b"request"
        );

        for malformed in [
            REQUEST_PREFIX[..REQUEST_PREFIX.len() - 1].to_vec(),
            [REQUEST_PREFIX, &[1, 2, 3]].concat(),
            [REQUEST_PREFIX, &8_u64.to_le_bytes(), b"short"].concat(),
            [REQUEST_PREFIX, &u64::MAX.to_le_bytes()].concat(),
        ] {
            assert!(read_frame(&mut malformed.as_slice(), REQUEST_PREFIX, 7).is_err());
        }

        let response = [
            b"harness noise".as_slice(),
            RESPONSE_PREFIX,
            &4_u64.to_le_bytes(),
            b"body",
        ]
        .concat();
        assert_eq!(
            find_frame(&response, RESPONSE_PREFIX, 4).expect("response frame"),
            b"body"
        );
        assert!(find_frame(&response, RESPONSE_PREFIX, 3).is_err());
        assert!(find_frame(&response[..response.len() - 1], RESPONSE_PREFIX, 4).is_err());

        let mut malformed_request = vec![0_u8; 4 + 32 + 1];
        malformed_request.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(
            deserialize_bounded::<Request>(&malformed_request, malformed_request.len()).is_err()
        );
    }

    #[test]
    fn authentication_key_reads_are_exact_and_zeroizing() {
        assert!(read_auth_key(&mut [0_u8; AUTH_KEY_BYTES - 1].as_slice()).is_err());
        let bytes = [0x5a_u8; AUTH_KEY_BYTES];
        let key = read_auth_key(&mut bytes.as_slice()).expect("complete key");
        assert_eq!(*key, bytes);
    }

    #[test]
    fn completed_exit_and_drains_resolve_before_expired_wall_time() {
        let success = std::process::ExitStatus::from_raw(0);
        let events = ReaderEvents::new();
        events
            .publish(true, Ok(b"response".to_vec()))
            .expect("stdout publication");
        events
            .publish(false, Ok(Vec::new()))
            .expect("stderr publication");
        assert!(matches!(
            events
                .decide(Some(success), true, || Ok(None))
                .expect("decision"),
            SupervisionAction::Complete(Ok(CollectedWorkerOutput {
                stdout: response,
                ..
            })) if response == b"response"
        ));

        let crash = std::process::ExitStatus::from_raw(9 << 8);
        let events = ReaderEvents::new();
        events
            .publish(true, Ok(Vec::new()))
            .expect("stdout publication");
        events
            .publish(false, Ok(b"diagnostic".to_vec()))
            .expect("stderr publication");
        assert!(matches!(
            events
                .decide(Some(crash), true, || Ok(None))
                .expect("decision"),
            SupervisionAction::Complete(Err(FormatFixtureError::WorkerCrashed(
                Some(9),
                diagnostic
            ))) if diagnostic == "diagnostic"
        ));
    }

    #[test]
    fn expired_wall_time_still_bounds_open_pipes_and_live_children() {
        let success = std::process::ExitStatus::from_raw(0);
        let events = ReaderEvents::new();
        events
            .publish(true, Ok(Vec::new()))
            .expect("stdout publication");
        assert!(matches!(
            events
                .decide(Some(success), false, || Ok(None))
                .expect("decision"),
            SupervisionAction::WaitForPipes
        ));
        assert!(matches!(
            events
                .decide(Some(success), true, || Ok(None))
                .expect("decision"),
            SupervisionAction::WallTimeExceeded
        ));

        let events = ReaderEvents::new();
        assert!(matches!(
            events.decide(None, false, || Ok(None)).expect("decision"),
            SupervisionAction::CheckResidentSet
        ));
        assert!(matches!(
            events.decide(None, true, || Ok(None)).expect("decision"),
            SupervisionAction::WallTimeExceeded
        ));
    }

    #[test]
    fn deadline_classification_precedes_late_reader_publication() {
        let success = std::process::ExitStatus::from_raw(0);
        let events = ReaderEvents::new();
        events
            .publish(true, Ok(Vec::new()))
            .expect("stdout publication");
        assert!(matches!(
            events
                .decide(Some(success), true, || Ok(None))
                .expect("decision"),
            SupervisionAction::WallTimeExceeded
        ));
        events
            .publish(false, Ok(Vec::new()))
            .expect("late stderr publication remains memory-safe");
    }

    #[test]
    fn reader_publication_after_old_final_drain_wins_before_deadline_classification() {
        let events = Arc::new(ReaderEvents::new());
        let release = Arc::new(Barrier::new(2));
        let published = Arc::new(Barrier::new(2));
        let producer_events = Arc::clone(&events);
        let sender_release = Arc::clone(&release);
        let sender_published = Arc::clone(&published);
        let producer = std::thread::spawn(move || {
            sender_release.wait();
            producer_events
                .publish(true, Ok(b"response".to_vec()))
                .expect("stdout publication");
            producer_events
                .publish(false, Ok(Vec::new()))
                .expect("stderr publication");
            sender_published.wait();
        });

        // This empty decision is the state the old final `try_recv` drain
        // observed. The barriers place publication after that observation and
        // before the expired-deadline classification.
        assert!(matches!(
            events
                .decide(None, false, || Ok(None))
                .expect("pre-exit decision"),
            SupervisionAction::CheckResidentSet
        ));
        release.wait();
        published.wait();
        let status = Some(std::process::ExitStatus::from_raw(0));
        assert!(matches!(
            events
                .decide(status, true, || Ok(None))
                .expect("atomic classification"),
            SupervisionAction::Complete(Ok(CollectedWorkerOutput {
                stdout: response,
                ..
            })) if response == b"response"
        ));
        producer.join().expect("completion producer");
    }

    #[test]
    fn exit_event_reconciles_stale_live_sample_at_deadline() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read ignored || exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn synchronized helper");
        let exit_event = ProcessExitEvent::open(&child).expect("open process exit event");
        let stale_status = child.try_wait().expect("prior live observation");
        assert!(stale_status.is_none());

        let events = ReaderEvents::new();
        events
            .publish(true, Ok(b"response".to_vec()))
            .expect("stdout publication");
        events
            .publish(false, Ok(Vec::new()))
            .expect("stderr publication");
        drop(child.stdin.take());
        assert!(
            exit_event
                .wait_until_ready()
                .expect("bounded kernel exit wait"),
            "helper did not exit"
        );

        assert!(matches!(
            events
                .decide(stale_status, true, || {
                    assert!(exit_event.is_ready()?);
                    child
                        .wait()
                        .map(Some)
                        .map_err(|error| FormatFixtureError::Worker(error.to_string()))
                })
                .expect("deadline arbitration"),
            SupervisionAction::Complete(Ok(CollectedWorkerOutput {
                stdout: response,
                ..
            })) if response == b"response"
        ));
    }

    #[test]
    fn vanished_proc_after_live_observation_reconciles_exit_and_drains() {
        let mut initial_status = None;
        assert!(initial_status.is_none());

        reconcile_process_disappearance(&mut initial_status, || {
            Ok(Some(std::process::ExitStatus::from_raw(0)))
        })
        .expect("fresh exit observation reconciles vanished proc entry");
        let events = ReaderEvents::new();
        events
            .publish(true, Ok(b"authenticated-response".to_vec()))
            .expect("stdout publication");
        events
            .publish(false, Ok(Vec::new()))
            .expect("stderr publication");

        assert!(matches!(
            events
                .decide(initial_status, true, || Ok(None))
                .expect("decision"),
            SupervisionAction::Complete(Ok(CollectedWorkerOutput {
                stdout: response,
                ..
            })) if response == b"authenticated-response"
        ));
    }

    #[test]
    fn vanished_proc_without_exit_remains_fail_closed() {
        let mut status = None;
        assert!(matches!(
            reconcile_process_disappearance(&mut status, || Ok(None)),
            Err(FormatFixtureError::ResidentSetUnsupported)
        ));
        assert!(status.is_none());
    }

    #[test]
    fn only_missing_proc_entry_is_reconcilable() {
        assert!(matches!(
            resident_set_from_statm_result(Err(std::io::Error::from(ErrorKind::NotFound))),
            Err(WorkerResidentSetError::ProcessVanished)
        ));
        assert!(matches!(
            resident_set_from_statm_result(Err(std::io::Error::from(ErrorKind::PermissionDenied))),
            Err(WorkerResidentSetError::Unsupported)
        ));
        assert!(matches!(
            resident_set_from_statm_result(Ok("malformed".into())),
            Err(WorkerResidentSetError::Unsupported)
        ));
        assert!(matches!(
            resident_set_from_statm_result(Ok(format!("0 {}", u64::MAX))),
            Err(WorkerResidentSetError::Unsupported)
        ));
    }

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
            // This regression owns bounded concurrent pipe draining, not an
            // allocation ceiling. Before `exec`, the helper may transiently
            // inherit the test process's suite-dependent resident mappings.
            guards(Duration::from_secs(2), u64::MAX),
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
            // This regression owns bounded concurrent pipe draining, not an
            // allocation ceiling. Before `exec`, the helper may transiently
            // inherit the test process's suite-dependent resident mappings.
            guards(Duration::from_secs(2), u64::MAX),
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
                // This regression owns the stdout byte ceiling. A transient
                // pre-exec shell may inherit the suite process's resident
                // mappings, so RSS is tested independently.
                guards(Duration::from_secs(2), u64::MAX),
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
        let decoded: Request =
            deserialize_bounded(&bytes, MAX_WORKER_REQUEST_BYTES).expect("decode");
        let rebuilt = decoded.into_recipe().expect("recipe");
        assert_eq!(
            rebuilt.identity().expect("identity").key().bytes(),
            identity
        );
        assert_eq!(rebuilt.guards.command_fuel, recipe.guards.command_fuel);
        assert_eq!(rebuilt.resources, recipe.resources);
    }
}

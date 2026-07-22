use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tex_incr::RevisionId;
use umber::EngineMode;
use umber::cli_resource::{NativeCompileSession, NativeRunError, NativeRunOptions};
use umber_fetch::FetchCancellation;

#[allow(clippy::disallowed_methods)] // Host-side polling and latency reporting.
pub(super) fn run(mut args: impl Iterator<Item = String>) -> Result<(), WatchError> {
    let input = args
        .next()
        .map(PathBuf::from)
        .ok_or(WatchError::Usage("missing input path for watch"))?;
    let mut output = input.with_extension("dvi");
    let mut poll = Duration::from_millis(100);
    let mut format = None;
    let mut distribution = None;
    let mut distribution_sha256 = None;
    let mut offline = env::var_os("UMBER_OFFLINE").is_some_and(|value| value == "1");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dvi" => {
                output = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or(WatchError::Usage("missing output path for --dvi"))?;
            }
            "--poll-ms" => {
                let value = args
                    .next()
                    .ok_or(WatchError::Usage("missing milliseconds for --poll-ms"))?;
                poll = Duration::from_millis(
                    value
                        .parse()
                        .map_err(|_| WatchError::Usage("--poll-ms must be an integer"))?,
                );
            }
            "--format" => {
                format = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or(WatchError::Usage("missing input path for --format"))?,
                );
            }
            "--distribution" => {
                distribution = Some(
                    args.next()
                        .ok_or(WatchError::Usage("missing URL or path for --distribution"))?,
                );
            }
            "--distribution-sha256" => {
                distribution_sha256 = Some(args.next().ok_or(WatchError::Usage(
                    "missing digest for --distribution-sha256",
                ))?);
            }
            "--offline" => offline = true,
            _ => {
                return Err(WatchError::Usage(
                    "watch accepts --dvi, --format, --poll-ms, --distribution, --distribution-sha256, and --offline",
                ));
            }
        }
    }

    let options = NativeRunOptions {
        input: input.clone(),
        format,
        initial_prefetch_keys: Vec::new(),
        engine: EngineMode::Tex82,
        outputs: umber::OutputCapabilitySet::DVI,
        html_asset_directory: None,
        distribution,
        distribution_sha256,
        offline,
        expansion_fuel: None,
    };
    let interrupted = Arc::new(AtomicBool::new(false));
    let active = Arc::new(Mutex::new(None::<FetchCancellation>));
    install_interrupt_handler(Arc::clone(&interrupted), Arc::clone(&active))?;

    let mut candidate_source = std::fs::read_to_string(&input)?;
    let startup_cancellation = FetchCancellation::new();
    set_active(&active, Some(startup_cancellation.clone()));
    let mut session = NativeCompileSession::new(&options, &startup_cancellation)?;
    set_active(&active, None);
    if interrupted.load(Ordering::Acquire) {
        return Ok(());
    }

    let mut next_revision = 1_u64;
    let mut announced = false;
    loop {
        let total_started = Instant::now();
        match compile_monitored(
            &mut session,
            &input,
            &candidate_source,
            poll,
            &interrupted,
            &active,
        )? {
            CompileRound::Complete(run) => {
                let dvi_started = Instant::now();
                std::fs::write(&output, &run.dvi)?;
                let dvi_latency = dvi_started.elapsed();
                if !announced {
                    eprintln!("watching {} -> {}", input.display(), output.display());
                    announced = true;
                } else if let Some(reuse) = session.reuse_metrics() {
                    eprintln!(
                        "revision={next_revision} total_us={} fork_us={} reexecute_us={} trace_validation_us={} trace_replay_us={} splice_us={} dvi_write_us={} pages_retained_prefix={} pages_reused={} pages_retyped={} reexecuted_bytes={} reexecuted_tokens={} reexecuted_commands={} reexecuted_macro_text_span_tokens={} reexecuted_source_text_span_tokens={} reexecuted_paragraphs={} trace_nodes_walked={} trace_leaf_hits={} trace_subtree_hits={} trace_bytes={} same_history_attempts={} same_history_hash_mismatches={} same_history_stop={:?}",
                        total_started.elapsed().as_micros(),
                        reuse.restart_fork_latency.as_micros(),
                        reuse.reexecution_latency.as_micros(),
                        reuse.trace_validation_latency.as_micros(),
                        reuse.trace_replay_latency.as_micros(),
                        reuse.splice_latency.as_micros(),
                        dvi_latency.as_micros(),
                        reuse.pages_retained_prefix,
                        reuse.pages_reused,
                        reuse.pages_retyped,
                        reuse.reexecuted_bytes,
                        reuse.reexecuted_tokens,
                        reuse.reexecuted_commands,
                        reuse.reexecuted_macro_text_span_tokens,
                        reuse.reexecuted_source_text_span_tokens,
                        reuse.reexecuted_paragraphs,
                        reuse.trace_nodes_walked,
                        reuse.trace_leaf_hits,
                        reuse.trace_subtree_hits,
                        reuse.trace_retained_bytes,
                        reuse.same_history_attempts,
                        reuse.same_history_hash_mismatches,
                        reuse.same_history_stop,
                    );
                }
                match wait_for_edit(&input, &candidate_source, poll, &interrupted)? {
                    Some(next) => {
                        next_revision += 1;
                        session.apply_source(RevisionId::new(next_revision), &next)?;
                        candidate_source = next;
                    }
                    None => return Ok(()),
                }
            }
            CompileRound::Superseded(next) => {
                if session.cancel_pending_revision() {
                    next_revision += 1;
                    session.apply_source(RevisionId::new(next_revision), &next)?;
                } else {
                    let cancellation = FetchCancellation::new();
                    set_active(&active, Some(cancellation.clone()));
                    session = NativeCompileSession::new(&options, &cancellation)?;
                    set_active(&active, None);
                }
                candidate_source = next;
            }
            CompileRound::Interrupted => return Ok(()),
        }
    }
}

enum CompileRound {
    Complete(umber::MemoryRunOutput),
    Superseded(String),
    Interrupted,
}

#[allow(clippy::disallowed_methods)] // Host-side polling is the watch contract.
fn compile_monitored(
    session: &mut NativeCompileSession,
    input: &Path,
    candidate_source: &str,
    poll: Duration,
    interrupted: &AtomicBool,
    active: &Mutex<Option<FetchCancellation>>,
) -> Result<CompileRound, WatchError> {
    let cancellation = FetchCancellation::new();
    set_active(active, Some(cancellation.clone()));
    let mut superseded = None;
    let mut read_error = None;
    let result = std::thread::scope(|scope| {
        let worker_cancellation = cancellation.clone();
        let worker = scope.spawn(move || session.compile(&worker_cancellation));
        while !worker.is_finished() {
            std::thread::sleep(poll);
            if interrupted.load(Ordering::Acquire) {
                cancellation.cancel();
                continue;
            }
            match std::fs::read_to_string(input) {
                Ok(next) if next != candidate_source => {
                    superseded = Some(next);
                    cancellation.cancel();
                }
                Ok(_) => {}
                Err(error) => {
                    read_error = Some(error);
                    cancellation.cancel();
                }
            }
        }
        worker.join().map_err(|_| WatchError::WorkerPanic)
    })?;
    set_active(active, None);
    if let Some(error) = read_error {
        return Err(error.into());
    }
    if interrupted.load(Ordering::Acquire) {
        return Ok(CompileRound::Interrupted);
    }
    if let Some(next) = superseded {
        return Ok(CompileRound::Superseded(next));
    }
    result
        .map(CompileRound::Complete)
        .map_err(WatchError::Native)
}

#[allow(clippy::disallowed_methods)] // Host-side polling is the watch contract.
fn wait_for_edit(
    input: &Path,
    current: &str,
    poll: Duration,
    interrupted: &AtomicBool,
) -> Result<Option<String>, std::io::Error> {
    loop {
        std::thread::sleep(poll);
        if interrupted.load(Ordering::Acquire) {
            return Ok(None);
        }
        let next = std::fs::read_to_string(input)?;
        if next != current {
            return Ok(Some(next));
        }
    }
}

fn install_interrupt_handler(
    interrupted: Arc<AtomicBool>,
    active: Arc<Mutex<Option<FetchCancellation>>>,
) -> Result<(), WatchError> {
    umber_interrupt::set_handler(move || {
        interrupted.store(true, Ordering::Release);
        if let Ok(active) = active.lock()
            && let Some(cancellation) = active.as_ref()
        {
            cancellation.cancel();
        }
    })
    .map_err(WatchError::InterruptHandler)
}

fn set_active(active: &Mutex<Option<FetchCancellation>>, cancellation: Option<FetchCancellation>) {
    if let Ok(mut active) = active.lock() {
        *active = cancellation;
    }
}

#[derive(Debug)]
pub(super) enum WatchError {
    Usage(&'static str),
    Io(std::io::Error),
    Native(NativeRunError),
    InterruptHandler(umber_interrupt::InstallError),
    WorkerPanic,
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(error) => write!(f, "watch I/O failed: {error}"),
            Self::Native(error) => write!(f, "watch execution failed: {error}"),
            Self::InterruptHandler(error) => {
                write!(f, "watch interrupt handler failed: {error}")
            }
            Self::WorkerPanic => f.write_str("watch compile worker panicked"),
        }
    }
}

impl std::error::Error for WatchError {}

impl From<std::io::Error> for WatchError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<NativeRunError> for WatchError {
    fn from(value: NativeRunError) -> Self {
        Self::Native(value)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Host-side watch polling fixture.
mod tests {
    use super::*;

    #[test]
    fn wait_for_edit_stops_after_interrupt() {
        let directory = tempfile::tempdir().expect("temporary input directory");
        let path = directory.path().join("watch.tex");
        std::fs::write(&path, "original").expect("write input");
        let interrupted = AtomicBool::new(true);

        assert!(
            wait_for_edit(&path, "original", Duration::ZERO, &interrupted)
                .expect("wait")
                .is_none()
        );
    }
}

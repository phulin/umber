use std::path::PathBuf;
use std::time::{Duration, Instant};

use tex_incr::{AcceptedOutput, Edit, RevisionId, Session};
use tex_state::{Universe, World};

#[allow(clippy::disallowed_methods)] // Host-side polling and latency reporting.
pub(super) fn run(mut args: impl Iterator<Item = String>) -> Result<(), WatchError> {
    let input = args
        .next()
        .map(PathBuf::from)
        .ok_or(WatchError::Usage("missing input path for watch"))?;
    let mut output = input.with_extension("dvi");
    let mut poll = Duration::from_millis(100);
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
            _ => return Err(WatchError::Usage("watch accepts --dvi and --poll-ms")),
        }
    }

    let source = std::fs::read_to_string(&input)?;
    let mut template = Universe::with_world(World::real());
    umber::prepare_run_stores(&mut template);
    let job_name = input
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("texput");
    let mut session = Session::start(
        template,
        job_name,
        RevisionId::new(1),
        source.clone(),
        usize::MAX,
    )?;
    let cold = session.cold()?;
    std::fs::write(&output, cold.dvi_bytes()?)?;
    eprintln!("watching {} -> {}", input.display(), output.display());

    let mut revision = 1;
    loop {
        std::thread::sleep(poll);
        let next = std::fs::read_to_string(&input)?;
        if next == session.source() {
            continue;
        }
        revision += 1;
        let total_started = Instant::now();
        let (accepted, dvi_latency) =
            advance_and_write(&mut session, RevisionId::new(revision), &next, &output)?;
        eprintln!(
            "revision={revision} total_us={} fork_us={} reexecute_us={} splice_us={} dvi_write_us={} pages_reused={} pages_retyped={}",
            total_started.elapsed().as_micros(),
            accepted.reuse.restart_fork_latency.as_micros(),
            accepted.reuse.reexecution_latency.as_micros(),
            accepted.reuse.splice_latency.as_micros(),
            dvi_latency.as_micros(),
            accepted.reuse.pages_reused,
            accepted.reuse.pages_retyped,
        );
    }
}

#[allow(clippy::disallowed_methods)] // Host-side DVI write latency reporting.
fn advance_and_write(
    session: &mut Session,
    revision: RevisionId,
    next: &str,
    output: &std::path::Path,
) -> Result<(AcceptedOutput, Duration), WatchError> {
    let edit = contiguous_edit(
        session.source(),
        next,
        session.revision(),
        session.content_hash(),
    );
    let accepted = session.advance(revision, edit)?;
    let dvi_started = Instant::now();
    std::fs::write(output, accepted.dvi_bytes()?)?;
    Ok((accepted, dvi_started.elapsed()))
}

fn contiguous_edit(
    old: &str,
    new: &str,
    revision: RevisionId,
    expected_hash: tex_state::ContentHash,
) -> Edit {
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    Edit {
        base_revision: revision,
        expected_hash,
        range: prefix..old.len() - suffix,
        replacement: new[prefix..new.len() - suffix].to_owned(),
    }
}

#[derive(Debug)]
pub(super) enum WatchError {
    Usage(&'static str),
    Io(std::io::Error),
    Session(tex_incr::SessionError),
    Dvi(tex_out::dvi::DviError),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(error) => write!(f, "watch I/O failed: {error}"),
            Self::Session(error) => write!(f, "watch execution failed: {error}"),
            Self::Dvi(error) => write!(f, "watch DVI failed: {error}"),
        }
    }
}

impl std::error::Error for WatchError {}

impl From<std::io::Error> for WatchError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tex_incr::SessionError> for WatchError {
    fn from(value: tex_incr::SessionError) -> Self {
        Self::Session(value)
    }
}

impl From<tex_out::dvi::DviError> for WatchError {
    fn from(value: tex_out::dvi::DviError) -> Self {
        Self::Dvi(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_edit_preserves_unicode_boundaries() {
        let edit = contiguous_edit(
            "abécd",
            "abΩcd",
            RevisionId::new(7),
            tex_state::ContentHash::from_bytes("abécd".as_bytes()),
        );
        assert_eq!(&"abécd"[edit.range], "é");
        assert_eq!(edit.replacement, "Ω");
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // Host-side watch-output smoke test.
    fn one_watched_edit_writes_cold_identical_dvi() {
        let original = "\\shipout\\vbox{\\hrule height 1pt}\\end";
        let edited = "\\shipout\\vbox{\\hrule height 2pt}\\end";
        let mut template = Universe::with_world(World::memory());
        umber::prepare_run_stores(&mut template);
        let mut watched = Session::start(
            template.clone(),
            "watch",
            RevisionId::new(1),
            original,
            usize::MAX,
        )
        .expect("watch session");
        watched.cold().expect("initial run");
        let directory = tempfile::tempdir().expect("temporary output directory");
        let path = directory.path().join("watch.dvi");
        let (incremental, _) = advance_and_write(&mut watched, RevisionId::new(2), edited, &path)
            .expect("watched edit");

        let mut cold = Session::start(template, "watch", RevisionId::new(2), edited, usize::MAX)
            .expect("cold session");
        let cold = cold.cold().expect("cold run");
        assert_eq!(
            std::fs::read(path).expect("watched output"),
            cold.dvi_bytes().expect("cold DVI")
        );
        assert_eq!(
            incremental.dvi_bytes().expect("incremental DVI"),
            cold.dvi_bytes().expect("cold DVI")
        );
    }
}

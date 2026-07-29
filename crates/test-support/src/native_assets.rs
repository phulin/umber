//! Provision the pinned, gitignored assets the conformance gates consume.
//!
//! The byte-exact end-to-end DVI oracles, the TRIP inputs, and the shared
//! font/hyphenation inputs are deliberately not committed: that is a licensing
//! decision. They exist only where someone ran
//! `scripts/setup-conformance-tests.sh`. A linked worktree is therefore born
//! without them, and its conformance gates would fail for a reason that has
//! nothing to do with the code under test.
//!
//! [`provision`] copies them from the owning checkout, and every consumer
//! calls it before touching an asset, so a fresh worktree needs no setup step
//! beyond `cargo test`. This ran as a preamble inside
//! `scripts/run-native-tests.py` until `cargo test --tests` became the routine
//! gate directly; making it lazy and idempotent is what let that wrapper go.
//!
//! Only paths in `tests/native-test-assets.lock` may be copied, and every
//! source and destination is verified against that committed SHA-256 before
//! use. Copies are used rather than symlinks or hard links so a test in one
//! worktree cannot mutate the owning checkout's evidence.

#[allow(clippy::disallowed_methods)] // host-side asset provisioning
mod imp {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read as _, Write as _};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::OnceLock;

    use anyhow::{Context, Result, anyhow, bail};

    const LOCK: &str = "tests/native-test-assets.lock";

    /// Provisions every pinned asset once per process, returning how many were
    /// copied. Repeat calls return the first result, including its error text.
    ///
    /// Consumers call this before reading an asset. It is deliberately
    /// idempotent and cheap on the common path: when every asset is already
    /// present and correct it hashes them and copies nothing.
    pub fn provision(repo_root: &Path) -> Result<usize> {
        static ONCE: OnceLock<Result<usize, String>> = OnceLock::new();
        ONCE.get_or_init(|| provision_once(repo_root).map_err(|error| format!("{error:#}")))
            .as_ref()
            .map_err(|error| anyhow!(error.clone()))
            .copied()
    }

    fn provision_once(repo_root: &Path) -> Result<usize> {
        let repo_root = repo_root
            .canonicalize()
            .with_context(|| format!("resolve repository root {}", repo_root.display()))?;
        let assets = read_lock(&repo_root)?;

        let mut missing = Vec::new();
        for (relative, expected) in &assets {
            let destination = repo_root.join(relative);
            if destination.symlink_metadata().is_ok() {
                verify(&destination, expected, "existing asset")?;
            } else {
                missing.push((relative.clone(), expected.clone()));
            }
        }
        if missing.is_empty() {
            return Ok(0);
        }

        let owner = owning_checkout(&repo_root)?;
        let listed = |paths: &[(PathBuf, String)]| {
            paths
                .iter()
                .map(|(path, _)| format!("  {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if owner == repo_root {
            bail!(
                "the primary checkout is missing pinned conformance assets:\n{}\n\
                 Materialize them with scripts/setup-conformance-tests.sh.",
                listed(&missing)
            );
        }

        let absent: Vec<(PathBuf, String)> = missing
            .iter()
            .filter(|(relative, _)| !owner.join(relative).is_file())
            .cloned()
            .collect();
        if !absent.is_empty() {
            bail!(
                "the owning checkout {} is missing pinned conformance assets:\n{}\n\
                 Run scripts/setup-conformance-tests.sh there, then rerun this suite.",
                owner.display(),
                listed(&absent)
            );
        }

        for (relative, expected) in &missing {
            let source = owner.join(relative);
            verify(&source, expected, "owning asset")?;
            copy_verified(&source, &repo_root.join(relative), expected)?;
        }
        Ok(missing.len())
    }

    fn read_lock(repo_root: &Path) -> Result<BTreeMap<PathBuf, String>> {
        let path = repo_root.join(LOCK);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read asset allowlist {}", path.display()))?;
        let mut assets = BTreeMap::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_whitespace();
            let (Some(expected), Some(relative), None) =
                (fields.next(), fields.next(), fields.next())
            else {
                bail!(
                    "{}:{}: expected SHA-256 and path",
                    path.display(),
                    number + 1
                );
            };
            let relative = PathBuf::from(relative);
            let unsafe_entry = expected.len() != 64
                || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
                || relative.is_absolute()
                || relative
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
                || assets.contains_key(&relative);
            if unsafe_entry {
                bail!(
                    "{}:{}: unsafe or duplicate asset entry",
                    path.display(),
                    number + 1
                );
            }
            assets.insert(relative, expected.to_ascii_lowercase());
        }
        if assets.is_empty() {
            bail!("{}: asset allowlist is empty", path.display());
        }
        Ok(assets)
    }

    fn git(repo_root: &Path, arguments: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(arguments)
            .output()
            .context("inspect Git worktree metadata")?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn common_dir(path: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(git(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)
        .canonicalize()?)
    }

    /// The checkout that owns the shared Git directory, which is where
    /// `setup-conformance-tests.sh` materializes the assets.
    fn owning_checkout(repo_root: &Path) -> Result<PathBuf> {
        let shared = common_dir(repo_root)?;
        for line in git(repo_root, &["worktree", "list", "--porcelain"])?.lines() {
            let Some(candidate) = line.strip_prefix("worktree ") else {
                continue;
            };
            let candidate = PathBuf::from(candidate);
            let Ok(candidate) = candidate.canonicalize() else {
                continue;
            };
            if candidate.join(".git").is_dir() && common_dir(&candidate).is_ok_and(|d| d == shared)
            {
                return Ok(candidate);
            }
        }
        bail!(
            "Git's worktree registry has no primary checkout for {}",
            shared.display()
        )
    }

    fn sha256_of(path: &Path) -> Result<String> {
        use sha2::{Digest, Sha256};
        let mut file = fs::File::open(path)
            .with_context(|| format!("open asset {} for hashing", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0_u8; 1 << 20];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    fn verify(path: &Path, expected: &str, role: &str) -> Result<()> {
        let metadata = path
            .symlink_metadata()
            .with_context(|| format!("{role} is not present: {}", path.display()))?;
        if !metadata.is_file() {
            bail!("{role} is not a regular file: {}", path.display());
        }
        let actual = sha256_of(path)?;
        if actual != expected {
            bail!(
                "SHA-256 mismatch for {role} {}: expected {expected}, got {actual}",
                path.display()
            );
        }
        Ok(())
    }

    /// Writes through a temporary file in the destination directory, verifies
    /// the copy, then renames: a torn write can never be observed as an asset.
    fn copy_verified(source: &Path, destination: &Path, expected: &str) -> Result<()> {
        let parent = destination
            .parent()
            .ok_or_else(|| anyhow!("asset destination has no directory"))?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".{}.provision-{}",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            std::process::id()
        ));
        let result = (|| -> Result<()> {
            let mut input = fs::File::open(source)?;
            let mut output = fs::File::create(&temporary)?;
            std::io::copy(&mut input, &mut output)?;
            output.flush()?;
            output.sync_all()?;
            drop(output);
            verify(&temporary, expected, "copied asset")?;
            let mut permissions = fs::metadata(&temporary)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&temporary, permissions)?;
            fs::rename(&temporary, destination)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.with_context(|| format!("provision asset {}", destination.display()))
    }
}

pub use imp::provision;

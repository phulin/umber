use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::RootConfig;

#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) kind: &'static str,
    pub(crate) relative: String,
    pub(crate) source: PathBuf,
    pub(crate) sha256: String,
}

impl Candidate {
    pub(crate) fn logical_names(&self) -> Vec<&str> {
        let basename = self.relative.rsplit('/').next().unwrap_or(&self.relative);
        if basename == self.relative {
            vec![basename]
        } else {
            vec![basename, &self.relative]
        }
    }
}

pub(crate) fn scan_roots(roots: &[RootConfig]) -> Result<Vec<Candidate>> {
    let mut all = Vec::new();
    let mut physical_casefold = BTreeMap::<String, String>::new();
    for root in roots {
        validate_digest(&root.tree_sha256)?;
        let entries = supported_files(&root.path)?;
        let actual = digest_entries(&entries)?;
        if actual != root.tree_sha256 {
            bail!(
                "TEXMF root {:?} digest mismatch: expected {}, got {}",
                root.name,
                root.tree_sha256,
                actual
            );
        }
        for (relative, source) in entries {
            let fold = relative.to_lowercase();
            if let Some(previous) = physical_casefold.get(&fold)
                && previous != &relative
            {
                bail!("case-fold path collision between {previous:?} and {relative:?}");
            }
            physical_casefold.insert(fold, relative.clone());
            let kind = kind_for(&source).context("supported file lost its extension")?;
            let bytes = fs::read(&source)
                .with_context(|| format!("read source object {}", source.display()))?;
            all.push(Candidate {
                kind,
                relative,
                source,
                sha256: hex_sha256(&bytes),
            });
        }
    }
    Ok(all)
}

pub fn tree_sha256(root: &Path) -> Result<String> {
    digest_entries(&supported_files(root)?)
}

fn supported_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect TEXMF root {}", root.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("TEXMF root must be a real directory: {}", root.display());
    }
    let mut found = Vec::new();
    visit(root, root, &mut found)?;
    found.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(found)
}

fn visit(root: &Path, directory: &Path, found: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("read directory {}", directory.display()))?
    {
        let entry = entry.context("read directory entry")?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("symlinks are not publishable: {}", path.display());
        }
        if metadata.is_dir() {
            visit(root, &path, found)?;
        } else if metadata.is_file() && kind_for(&path).is_some() {
            let relative = normalize_relative(path.strip_prefix(root).context("strip root")?)?;
            found.push((relative, path));
        }
    }
    Ok(())
}

fn normalize_relative(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().context("non-UTF-8 TEXMF path")?;
                if value.is_empty() || value.contains(['\\', '\0', ':']) {
                    bail!("invalid TEXMF path component {value:?}");
                }
                parts.push(value);
            }
            _ => bail!("non-normal TEXMF path is not publishable: {path:?}"),
        }
    }
    if parts.is_empty() {
        bail!("empty TEXMF relative path");
    }
    Ok(parts.join("/"))
}

fn digest_entries(entries: &[(String, PathBuf)]) -> Result<String> {
    let mut digest = Sha256::new();
    for (relative, source) in entries {
        let bytes = fs::read(source).with_context(|| format!("read {}", source.display()))?;
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(Sha256::digest(&bytes));
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn kind_for(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(OsStr::to_str) {
        Some(extension)
            if ["tex", "ltx", "sty", "cls", "clo", "cfg", "def", "fd", "dfu"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported)) =>
        {
            Some("tex")
        }
        Some(extension) if extension.eq_ignore_ascii_case("tfm") => Some("tfm"),
        _ => None,
    }
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("treeSha256 must contain exactly 64 hexadecimal characters");
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        bail!("treeSha256 must use lowercase hexadecimal");
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

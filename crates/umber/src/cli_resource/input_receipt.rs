//! Exact accepted file-resource identities for one native CLI run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use tex_state::ContentIdentity;
use umber_hash::{AHash64, HashDomain};

use crate::{FileRequestKey, ResolvedFile};

use super::NativeRunError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContentRecord {
    bytes: usize,
    digest: AHash64,
    identity: ContentIdentity,
}

impl ContentRecord {
    fn for_bytes(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.len(),
            digest: AHash64::for_bytes(HashDomain::DistributionContent, bytes),
            identity: ContentIdentity::from_bytes(bytes),
        }
    }
}

/// Records files only after the typed VFS transaction accepts their payloads.
/// Host-staged predictions enter this receipt only if later demanded.
pub(super) struct InputAdmissions {
    main: ContentRecord,
    files: BTreeMap<FileRequestKey, (ContentRecord, BTreeSet<String>)>,
}

impl InputAdmissions {
    pub(super) fn new(main: &[u8]) -> Self {
        Self {
            main: ContentRecord::for_bytes(main),
            files: BTreeMap::new(),
        }
    }

    pub(super) fn record_files(
        &mut self,
        files: &[(crate::FileRequest, ResolvedFile)],
    ) -> Result<(), NativeRunError> {
        for (request, file) in files {
            let key = request.key();
            if file.request != *key {
                return Err(NativeRunError::Selection(format!(
                    "input admission request differs from resolved file: {}:{}",
                    key.kind().wire_name(),
                    key.name()
                )));
            }
            let record = ContentRecord::for_bytes(&file.bytes);
            match self.files.entry(key.clone()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((record, BTreeSet::from([file.virtual_path.clone()])));
                }
                std::collections::btree_map::Entry::Occupied(mut entry)
                    if entry.get().0 == record =>
                {
                    entry.get_mut().1.insert(file.virtual_path.clone());
                }
                std::collections::btree_map::Entry::Occupied(entry) => {
                    return Err(NativeRunError::Selection(format!(
                        "input admission changed content for {}:{}",
                        entry.key().kind().wire_name(),
                        entry.key().name()
                    )));
                }
            }
        }
        Ok(())
    }

    pub(super) fn to_bytes(
        &self,
        used_resources: &[(PathBuf, ContentIdentity)],
    ) -> Result<Vec<u8>, NativeRunError> {
        let used_paths = used_resources.iter().cloned().collect::<BTreeSet<_>>();
        let admitted_paths = self
            .files
            .values()
            .flat_map(|(record, paths)| paths.iter().map(move |path| (PathBuf::from(path), record)))
            .collect::<Vec<_>>();
        for (path, identity) in &used_paths {
            if !admitted_paths.iter().any(|(admitted_path, record)| {
                admitted_path == path && record.identity == *identity
            }) {
                return Err(NativeRunError::Selection(format!(
                    "consumed external input has no matching accepted admission: {}",
                    path.display()
                )));
            }
        }
        let mut receipt = format!(
            "umber-input-admissions-v1\nmain\t{}\t{}\n",
            self.main.bytes,
            self.main.digest.hex()
        )
        .into_bytes();
        for (key, (record, paths)) in &self.files {
            let kind = key.kind().wire_name();
            let name = key.name();
            let disposition = if paths
                .iter()
                .any(|path| used_paths.contains(&(PathBuf::from(path), record.identity)))
            {
                "used"
            } else {
                "admitted"
            };
            if name.contains(['\t', '\n', '\r']) {
                return Err(NativeRunError::Selection(format!(
                    "input admission request contains a TSV delimiter: {kind}:{name:?}"
                )));
            }
            receipt.extend_from_slice(
                format!(
                    "file\t{disposition}\t{kind}:{name}\t{}\t{}\n",
                    record.bytes,
                    record.digest.hex()
                )
                .as_bytes(),
            );
        }
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests;

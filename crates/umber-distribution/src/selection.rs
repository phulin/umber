use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use crate::manifest::{
    Manifest, ManifestParseError, ObjectEntry, validate_file_key, validate_font_name,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FileKind {
    Tex,
    Tfm,
    BibAux,
    ClassicBib,
    BibStyle,
}

impl FileKind {
    #[must_use]
    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::Tex => "tex",
            Self::Tfm => "tfm",
            Self::BibAux => "bib-aux",
            Self::ClassicBib => "classic-bib",
            Self::BibStyle => "bst",
        }
    }

    pub fn from_manifest_name(value: &str) -> Result<Self, SelectionError> {
        match value {
            "tex" => Ok(Self::Tex),
            "tfm" => Ok(Self::Tfm),
            "bib-aux" => Ok(Self::BibAux),
            "classic-bib" => Ok(Self::ClassicBib),
            "bst" => Ok(Self::BibStyle),
            _ => Err(SelectionError::new(format!(
                "unsupported distribution file kind {value:?}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileRequestKey {
    kind: FileKind,
    normalized_name: String,
}

impl FileRequestKey {
    pub fn new(kind: FileKind, normalized_name: impl Into<String>) -> Result<Self, SelectionError> {
        let key = Self {
            kind,
            normalized_name: normalized_name.into(),
        };
        validate_file_key(&key.manifest_key().0).map_err(SelectionError::from_manifest)?;
        Ok(key)
    }

    #[must_use]
    pub const fn kind(&self) -> FileKind {
        self.kind
    }

    #[must_use]
    pub fn normalized_name(&self) -> &str {
        &self.normalized_name
    }

    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        ManifestLogicalKey(format!(
            "{}:{}",
            self.kind.manifest_name(),
            self.normalized_name
        ))
    }

    pub fn from_manifest_key(value: &str) -> Result<Self, SelectionError> {
        validate_file_key(value).map_err(SelectionError::from_manifest)?;
        let (kind, name) = value
            .split_once(':')
            .expect("validated manifest file keys contain a colon");
        Self::new(FileKind::from_manifest_name(kind)?, name)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FontRequestKey {
    logical_name: String,
}

impl FontRequestKey {
    pub fn new(logical_name: impl Into<String>) -> Result<Self, SelectionError> {
        let logical_name = logical_name.into();
        validate_font_name(&logical_name).map_err(SelectionError::from_manifest)?;
        Ok(Self { logical_name })
    }

    #[must_use]
    pub fn logical_name(&self) -> &str {
        &self.logical_name
    }

    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        ManifestLogicalKey(self.logical_name.clone())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManifestLogicalKey(String);

impl ManifestLogicalKey {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ManifestLogicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ManifestRequest {
    File(FileRequestKey),
    Font(FontRequestKey),
}

impl ManifestRequest {
    #[must_use]
    pub fn manifest_key(&self) -> ManifestLogicalKey {
        match self {
            Self::File(key) => key.manifest_key(),
            Self::Font(key) => key.manifest_key(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRequirement {
    Required,
    DependencyHint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquisitionJob {
    pub request: ManifestRequest,
    pub manifest_key: ManifestLogicalKey,
    pub requirement: JobRequirement,
    pub object: ObjectEntry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestMiss {
    File(FileRequestKey),
    Font(FontRequestKey),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub jobs: Vec<AcquisitionJob>,
    pub misses: Vec<ManifestMiss>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionError {
    message: String,
}

impl SelectionError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_manifest(error: ManifestParseError) -> Self {
        Self::new(error.to_string())
    }
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for SelectionError {}

/// Selects required jobs in request order, followed by breadth-first transitive
/// dependency hints in manifest order. Duplicate requests and hints are omitted.
#[must_use]
pub fn select(manifest: &Manifest, requests: &[ManifestRequest]) -> Selection {
    let mut selection = Selection::default();
    let mut seen_files = BTreeSet::<String>::new();
    let mut seen_fonts = BTreeSet::<String>::new();
    let mut dependency_roots = Vec::<String>::new();

    for request in requests {
        match request {
            ManifestRequest::File(key) => {
                let manifest_key = key.manifest_key();
                if !seen_files.insert(manifest_key.0.clone()) {
                    continue;
                }
                let Some(entry) = manifest.files.get(manifest_key.as_str()) else {
                    selection.misses.push(ManifestMiss::File(key.clone()));
                    continue;
                };
                dependency_roots.push(manifest_key.0.clone());
                selection.jobs.push(AcquisitionJob {
                    request: request.clone(),
                    manifest_key,
                    requirement: JobRequirement::Required,
                    object: entry.object_entry(),
                });
            }
            ManifestRequest::Font(key) => {
                let manifest_key = key.manifest_key();
                if !seen_fonts.insert(manifest_key.0.clone()) {
                    continue;
                }
                let Some(entry) = manifest.fonts.get(manifest_key.as_str()) else {
                    selection.misses.push(ManifestMiss::Font(key.clone()));
                    continue;
                };
                selection.jobs.push(AcquisitionJob {
                    request: request.clone(),
                    manifest_key,
                    requirement: JobRequirement::Required,
                    object: entry.object_entry(),
                });
            }
        }
    }

    let mut cursor = 0;
    while cursor < dependency_roots.len() {
        let parent = &dependency_roots[cursor];
        cursor += 1;
        let entry = manifest
            .files
            .get(parent)
            .expect("validated dependency roots exist in the manifest");
        for dependency in &entry.dependencies {
            if !seen_files.insert(dependency.clone()) {
                continue;
            }
            let dependency_entry = manifest
                .files
                .get(dependency)
                .expect("manifest parsing validates dependency references");
            let key = FileRequestKey::from_manifest_key(dependency)
                .expect("manifest parsing validates file keys");
            dependency_roots.push(dependency.clone());
            selection.jobs.push(AcquisitionJob {
                request: ManifestRequest::File(key),
                manifest_key: ManifestLogicalKey(dependency.clone()),
                requirement: JobRequirement::DependencyHint,
                object: dependency_entry.object_entry(),
            });
        }
    }
    selection
}

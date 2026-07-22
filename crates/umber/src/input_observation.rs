use std::path::Path;

use tex_state::{ContentHash, InputDependencyAccess, InputDependencyOutcome};
use umber_vfs::{FileKind, FileOrigin, VfsSnapshot, VirtualPath};

use crate::RevisionId;

pub const ACCEPTED_INPUT_OBSERVATION_SCHEMA_VERSION: u32 = 1;
pub const MAX_ACCEPTED_INPUT_OBSERVATIONS: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedInputObservationLedger {
    schema_version: u32,
    revision: RevisionId,
    observations: Vec<AcceptedInputObservation>,
}

impl AcceptedInputObservationLedger {
    pub(crate) fn new(revision: RevisionId, observations: Vec<AcceptedInputObservation>) -> Self {
        Self {
            schema_version: ACCEPTED_INPUT_OBSERVATION_SCHEMA_VERSION,
            revision,
            observations,
        }
    }
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }
    #[must_use]
    pub fn observations(&self) -> &[AcceptedInputObservation] {
        &self.observations
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputObservationNamespace {
    Authored,
    Generated,
    Distribution,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputObservationOutcome {
    Present(ContentHash),
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputObservationPhase {
    Tex,
    BibliographyDetection,
    Bibliography,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InputObservationOwner {
    TexEngine,
    BibliographyDetector,
    Biblatex,
    ClassicBibtex,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AcceptedInputObservation {
    path: VirtualPath,
    namespace: InputObservationNamespace,
    outcome: InputObservationOutcome,
    access: InputDependencyAccess,
    resource_kind: FileKind,
    phase: InputObservationPhase,
    revision: RevisionId,
    project_pass: Option<u32>,
    requesting_source: Option<VirtualPath>,
    owner: InputObservationOwner,
}

impl AcceptedInputObservation {
    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }
    #[must_use]
    pub const fn namespace(&self) -> InputObservationNamespace {
        self.namespace
    }
    #[must_use]
    pub const fn outcome(&self) -> InputObservationOutcome {
        self.outcome
    }
    #[must_use]
    pub const fn access(&self) -> InputDependencyAccess {
        self.access
    }
    #[must_use]
    pub const fn resource_kind(&self) -> FileKind {
        self.resource_kind
    }
    #[must_use]
    pub const fn phase(&self) -> InputObservationPhase {
        self.phase
    }
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }
    #[must_use]
    pub const fn project_pass(&self) -> Option<u32> {
        self.project_pass
    }
    #[must_use]
    pub const fn requesting_source(&self) -> Option<&VirtualPath> {
        self.requesting_source.as_ref()
    }
    #[must_use]
    pub const fn owner(&self) -> InputObservationOwner {
        self.owner
    }
}

pub(crate) fn tex_observations(
    dependencies: impl Iterator<Item = (VirtualPath, InputDependencyOutcome, InputDependencyAccess)>,
    snapshot: &VfsSnapshot,
    revision: RevisionId,
    project_pass: Option<u32>,
) -> Vec<AcceptedInputObservation> {
    dependencies
        .map(|(path, outcome, access)| {
            let file = snapshot.get(&path).ok().flatten();
            let namespace = file.map_or_else(
                || namespace_from_path(&path),
                |file| namespace_from_origin(file.origin()),
            );
            let resource_kind = file
                .and_then(|file| match file.origin() {
                    FileOrigin::Resolved(key) => Some(key.kind()),
                    FileOrigin::User | FileOrigin::Generated { .. } => None,
                })
                .unwrap_or_else(|| infer_kind(path.as_path()));
            AcceptedInputObservation {
                path,
                namespace,
                outcome: match outcome {
                    InputDependencyOutcome::Present(hash) => InputObservationOutcome::Present(hash),
                    InputDependencyOutcome::Missing => InputObservationOutcome::Missing,
                },
                access,
                resource_kind,
                phase: InputObservationPhase::Tex,
                revision,
                project_pass,
                requesting_source: None,
                owner: InputObservationOwner::TexEngine,
            }
        })
        .collect()
}

pub(crate) fn virtual_path(path: &Path) -> Option<VirtualPath> {
    let value = path.to_str()?;
    if value.starts_with("/texlive/") {
        VirtualPath::distribution(value).ok()
    } else {
        VirtualPath::user(value).ok()
    }
}

pub(crate) fn bibliography_observations(
    inputs: &[bib_engine::BibliographyInput],
    snapshot: &VfsSnapshot,
    revision: RevisionId,
    project_pass: u32,
    owner: InputObservationOwner,
) -> Vec<AcceptedInputObservation> {
    inputs
        .iter()
        .filter_map(|input| {
            let file = snapshot.get(input.path()).ok().flatten()?;
            Some(AcceptedInputObservation {
                path: input.path().clone(),
                namespace: namespace_from_origin(file.origin()),
                outcome: InputObservationOutcome::Present(ContentHash::from_bytes(file.bytes())),
                access: InputDependencyAccess::RequiredRead,
                resource_kind: input.kind(),
                phase: InputObservationPhase::Bibliography,
                revision,
                project_pass: Some(project_pass),
                requesting_source: None,
                owner,
            })
        })
        .collect()
}

pub(crate) fn bibliography_detection_observations(
    mode: &bib_engine::BibliographyMode,
    snapshot: &VfsSnapshot,
    revision: RevisionId,
    project_pass: u32,
) -> Vec<AcceptedInputObservation> {
    let bib_engine::BibliographyMode::Auto { job_path } = mode else {
        return Vec::new();
    };
    let raw = job_path.as_str().strip_prefix("/job/").expect("job path");
    let stem = raw.rsplit_once('.').map_or(raw, |(stem, _)| stem);
    [("bcf", FileKind::BibControl), ("aux", FileKind::BibAux)]
        .into_iter()
        .map(|(extension, kind)| {
            let path = VirtualPath::user(&format!("{stem}.{extension}"))
                .expect("auto-detection companion path");
            let file = snapshot.get(&path).ok().flatten();
            AcceptedInputObservation {
                path,
                namespace: file.map_or(InputObservationNamespace::Generated, |file| {
                    namespace_from_origin(file.origin())
                }),
                outcome: file.map_or(InputObservationOutcome::Missing, |file| {
                    InputObservationOutcome::Present(ContentHash::from_bytes(file.bytes()))
                }),
                access: InputDependencyAccess::AuthoritativeProbe,
                resource_kind: kind,
                phase: InputObservationPhase::BibliographyDetection,
                revision,
                project_pass: Some(project_pass),
                requesting_source: None,
                owner: InputObservationOwner::BibliographyDetector,
            }
        })
        .collect()
}

fn namespace_from_origin(origin: &FileOrigin) -> InputObservationNamespace {
    match origin {
        FileOrigin::User => InputObservationNamespace::Authored,
        FileOrigin::Generated { .. } => InputObservationNamespace::Generated,
        FileOrigin::Resolved(_) => InputObservationNamespace::Distribution,
    }
}

fn namespace_from_path(path: &VirtualPath) -> InputObservationNamespace {
    if path.as_str().starts_with("/texlive/") {
        InputObservationNamespace::Distribution
    } else {
        InputObservationNamespace::Authored
    }
}

fn infer_kind(path: &Path) -> FileKind {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("tfm") => FileKind::Tfm,
        Some("png" | "jpg" | "jpeg" | "pdf") => FileKind::Image,
        Some("vf") => FileKind::VirtualFont,
        Some("map") => FileKind::PdfFontMap,
        Some("enc") => FileKind::PdfEncoding,
        Some("pfb" | "pfa") => FileKind::PdfFontProgram,
        _ => FileKind::TexInput,
    }
}

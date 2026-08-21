//! Portable detached format images and atomic destination materialization.
//!
//! Images are validated before a destination exists. Materialization then
//! rewrites handle-free contents into one fresh branded generation and moves
//! the complete candidate through a destination-identity barrier.

use core::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::generation::{GenerationBrand, with_generation};
use crate::interner::{Interner, InternerBudget};
use crate::pdf::PdfState;
use crate::stores::StateCore;
use crate::{InteractionMode, ProvenanceBudgets, ProvenanceDemand, Universe, World};

#[cfg(test)]
#[path = "format/tests.rs"]
mod tests;

/// Portable format container schema selected by this build.
pub const FORMAT_SCHEMA_VERSION: u32 = crate::format_container::SCHEMA_VERSION;
/// Fingerprint of the portable format container ABI selected by this build.
pub const FORMAT_ABI_FINGERPRINT: u64 = crate::format_container::ABI_FINGERPRINT;
/// Fingerprint of the immutable lookup configuration selected by this build.
pub const FORMAT_LOOKUP_CONFIGURATION_FINGERPRINT: u64 =
    crate::format_container::LOOKUP_CONFIGURATION_FINGERPRINT;

const REQUIRED_SECTION_KINDS: [u32; 11] = [1, 256, 257, 272, 288, 304, 320, 336, 352, 512, 528];
const SECTION_VERSION: u32 = 1;
const SECTION_HEADER_LEN: usize = 16;

static NEXT_DESTINATION: AtomicU64 = AtomicU64::new(1);

/// Validation, capture, or destination-staging failure for a format image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    OpenGroups(u32),
    NonEmptyPage,
    NonEmptyPdfDocument,
    BadMagic,
    UnsupportedVersion(u32),
    Truncated,
    TrailingBytes,
    Checksum,
    IncompatibleAbi(u64),
    IncompatibleLookupConfiguration(u64),
    InvalidInteractionMode(u8),
    InvalidState(String),
    DestinationConsumed,
    AllocationFailed,
}

impl core::fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OpenGroups(depth) => {
                write!(
                    formatter,
                    "cannot capture a format with {depth} open groups"
                )
            }
            Self::NonEmptyPage => {
                formatter.write_str("cannot capture a format with page-builder material")
            }
            Self::NonEmptyPdfDocument => {
                formatter.write_str("cannot capture a format with non-format PDF document state")
            }
            Self::BadMagic => formatter.write_str("not an Umber format file"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported Umber format version {version}")
            }
            Self::Truncated => formatter.write_str("truncated Umber format file"),
            Self::TrailingBytes => formatter.write_str("trailing bytes in Umber format file"),
            Self::Checksum => formatter.write_str("Umber format checksum mismatch"),
            Self::IncompatibleAbi(found) => {
                write!(
                    formatter,
                    "incompatible Umber format ABI fingerprint {found:#018x}"
                )
            }
            Self::IncompatibleLookupConfiguration(found) => write!(
                formatter,
                "incompatible Umber format lookup configuration {found:#018x}"
            ),
            Self::InvalidInteractionMode(mode) => {
                write!(formatter, "invalid interaction mode {mode}")
            }
            Self::InvalidState(message) => formatter.write_str(message),
            Self::DestinationConsumed => {
                formatter.write_str("format destination has already staged an image")
            }
            Self::AllocationFailed => formatter.write_str("format destination allocation failed"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<crate::format_container::ContainerError> for FormatError {
    fn from(error: crate::format_container::ContainerError) -> Self {
        use crate::format_container::ContainerError;
        match error {
            ContainerError::BadMagic => Self::BadMagic,
            ContainerError::UnsupportedVersion(version) => Self::UnsupportedVersion(version),
            ContainerError::Truncated => Self::Truncated,
            ContainerError::TrailingBytes => Self::TrailingBytes,
            ContainerError::Checksum => Self::Checksum,
            ContainerError::IncompatibleAbi(found) => Self::IncompatibleAbi(found),
            ContainerError::IncompatibleLookupConfiguration(found) => {
                Self::IncompatibleLookupConfiguration(found)
            }
            ContainerError::Invalid(message) => Self::InvalidState(message.to_owned()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FormatMetadata {
    version: u32,
    interaction_mode: u8,
    pdf: Vec<u8>,
}

#[derive(Clone, Debug)]
struct DecodedFormat {
    metadata: FormatMetadata,
}

/// Reusable, fully validated, handle-free format bytes.
pub struct DetachedFormatImage {
    bytes: Vec<u8>,
    decoded: DecodedFormat,
}

impl core::fmt::Debug for DetachedFormatImage {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DetachedFormatImage")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl DetachedFormatImage {
    /// Validates a complete portable image without constructing a runtime.
    pub fn try_from_bytes(bytes: Vec<u8>) -> Result<Self, FormatError> {
        let decoded = decode_image(&bytes)?;
        Ok(Self { bytes, decoded })
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn capture<G>(universe: &Universe<G>) -> Result<Self, FormatError> {
        let metadata = bincode::serialize(&universe.capture_format_metadata()?)
            .map_err(|error| FormatError::InvalidState(error.to_string()))?;
        let empty = empty_section();
        let sections = REQUIRED_SECTION_KINDS.map(|kind| crate::format_container::SectionInput {
            kind,
            alignment: 8,
            bytes: if kind == crate::format_container::TRANSITIONAL_SEMANTIC_SECTION {
                &metadata
            } else {
                &empty
            },
        });
        Self::try_from_bytes(crate::format_container::encode(&sections)?)
    }
}

fn decode_image(bytes: &[u8]) -> Result<DecodedFormat, FormatError> {
    let container = crate::format_container::decode(bytes)?;
    if container.sections.len() != REQUIRED_SECTION_KINDS.len()
        || container
            .sections
            .iter()
            .map(|section| section.kind)
            .ne(REQUIRED_SECTION_KINDS)
    {
        return Err(FormatError::InvalidState(
            "schema-11 format requires the canonical section set".to_owned(),
        ));
    }
    let metadata: FormatMetadata = bincode::deserialize(
        &container
            .section(crate::format_container::TRANSITIONAL_SEMANTIC_SECTION)
            .expect("required metadata section")
            .bytes,
    )
    .map_err(|error| FormatError::InvalidState(error.to_string()))?;
    if metadata.version != SECTION_VERSION {
        return Err(FormatError::InvalidState(
            "unsupported semantic metadata section".to_owned(),
        ));
    }
    decode_interaction_mode(metadata.interaction_mode)?;
    for section in container.sections.iter().skip(1) {
        validate_empty_section(&section.bytes)?;
    }
    PdfState::<()>::restore_format_bytes(
        &metadata.pdf,
        |_| Err("format PDF token roots require the token section".to_owned()),
        |_| Err("format PDF node roots require the node section".to_owned()),
    )
    .map_err(FormatError::InvalidState)?;
    Ok(DecodedFormat { metadata })
}

fn empty_section() -> [u8; SECTION_HEADER_LEN] {
    let mut bytes = [0; SECTION_HEADER_LEN];
    bytes[..4].copy_from_slice(&SECTION_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&(SECTION_HEADER_LEN as u32).to_le_bytes());
    bytes
}

fn validate_empty_section(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes != empty_section() {
        return Err(FormatError::InvalidState(
            "unsupported or non-canonical format section".to_owned(),
        ));
    }
    Ok(())
}

fn encode_interaction_mode(mode: InteractionMode) -> u8 {
    match mode {
        InteractionMode::Batch => 0,
        InteractionMode::Nonstop => 1,
        InteractionMode::Scroll => 2,
        InteractionMode::ErrorStop => 3,
    }
}

fn decode_interaction_mode(mode: u8) -> Result<InteractionMode, FormatError> {
    match mode {
        0 => Ok(InteractionMode::Batch),
        1 => Ok(InteractionMode::Nonstop),
        2 => Ok(InteractionMode::Scroll),
        3 => Ok(InteractionMode::ErrorStop),
        _ => Err(FormatError::InvalidInteractionMode(mode)),
    }
}

/// Explicit cold provenance policy installed on a materialized job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatMaterializationConfig {
    pub provenance_demand: ProvenanceDemand,
    pub provenance_budgets: ProvenanceBudgets,
}

/// One fresh opaque destination for a single staged image.
pub struct FormatDestination<G> {
    identity: u64,
    budget: InternerBudget,
    core: Option<StateCore<G>>,
    world: Option<World>,
    provenance: FormatMaterializationConfig,
}

impl<G> FormatDestination<G> {
    /// Sets the explicit cold provenance policy before staging.
    pub fn set_provenance_config(&mut self, config: FormatMaterializationConfig) {
        self.provenance = config;
    }

    /// Rewrites a validated image into this destination's fresh generation.
    pub fn stage(&mut self, image: &DetachedFormatImage) -> Result<FormatStaging<G>, FormatError> {
        let core = self.core.take().ok_or(FormatError::DestinationConsumed)?;
        let mut universe = Universe::new_format_candidate(Interner::new(self.budget), core);
        universe.interaction_mode =
            decode_interaction_mode(image.decoded.metadata.interaction_mode)?;
        universe.install_format_pdf(&image.decoded.metadata.pdf);
        universe.set_format_provenance(self.provenance);
        Ok(FormatStaging {
            destination: self.identity,
            universe,
        })
    }

    /// Atomically moves a complete candidate through the identity barrier.
    pub fn materialize<R>(
        &mut self,
        staging: FormatStaging<G>,
        use_universe: impl FnOnce(&mut Universe<G>) -> R,
    ) -> Result<R, FormatPublicationError> {
        if staging.destination != self.identity {
            return Err(FormatPublicationError::ForeignDestination);
        }
        let mut universe = staging.universe;
        universe.world = self
            .world
            .take()
            .expect("matching destination publishes its caller World once");
        universe
            .refresh_job_clock_parameters()
            .expect("staged format candidate retains a live core");
        Ok(use_universe(&mut universe))
    }
}

/// Complete unpublished destination-local state. Deliberately non-Clone.
pub struct FormatStaging<G> {
    destination: u64,
    universe: Universe<G>,
}

/// Rejection at the final destination-identity barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatPublicationError {
    ForeignDestination,
}

/// Introduces a fresh generation and destination for one cold load episode.
pub fn with_format_destination<R>(
    budget: InternerBudget,
    world: World,
    use_destination: impl for<'id> FnOnce(
        &mut FormatDestination<GenerationBrand<'id>>,
    ) -> Result<R, FormatError>,
) -> Result<R, FormatError> {
    with_generation(|generation| {
        let core = StateCore::new(generation).map_err(|_| FormatError::AllocationFailed)?;
        let identity = NEXT_DESTINATION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("format destination identity space exhausted");
        let mut destination = FormatDestination {
            identity,
            budget,
            core: Some(core),
            world: Some(world),
            provenance: FormatMaterializationConfig {
                provenance_demand: ProvenanceDemand::default(),
                provenance_budgets: ProvenanceBudgets::default(),
            },
        };
        use_destination(&mut destination)
    })
}

/// Materializes one image entirely inside a fresh HRTB scope.
pub fn with_materialized_format<R>(
    budget: InternerBudget,
    world: World,
    image: &DetachedFormatImage,
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, FormatError> {
    with_format_destination(budget, world, |destination| {
        let staging = destination.stage(image)?;
        destination
            .materialize(staging, use_universe)
            .map_err(|_| FormatError::InvalidState("foreign format destination".to_owned()))
    })
}

impl<G> Universe<G> {
    fn capture_format_metadata(&self) -> Result<FormatMetadata, FormatError> {
        let core = self
            .core
            .as_ref()
            .ok_or_else(|| FormatError::InvalidState("retired Universe".to_owned()))?;
        let depth = u32::try_from(core.state().group_depth())
            .map_err(|_| FormatError::InvalidState("group depth exceeds u32".to_owned()))?;
        if depth != 0 {
            return Err(FormatError::OpenGroups(depth));
        }
        if !self.page.is_format_empty() {
            return Err(FormatError::NonEmptyPage);
        }
        let pdf = self
            .pdf
            .capture_format_bytes(
                |_| Err("format PDF token roots require the token section".to_owned()),
                |_| Err("format PDF node roots require the node section".to_owned()),
            )
            .map_err(FormatError::InvalidState)?
            .ok_or(FormatError::NonEmptyPdfDocument)?;
        Ok(FormatMetadata {
            version: SECTION_VERSION,
            interaction_mode: encode_interaction_mode(self.interaction_mode),
            pdf,
        })
    }

    pub(crate) fn new_format_candidate(interner: Interner, core: StateCore<G>) -> Self {
        Self::new(interner, core)
    }

    fn install_format_pdf(&mut self, bytes: &[u8]) {
        self.pdf = PdfState::restore_format_bytes(
            bytes,
            |_| unreachable!("validated format has no PDF token roots"),
            |_| unreachable!("validated format has no PDF node roots"),
        )
        .expect("detached image PDF envelope was validated before staging");
    }

    fn set_format_provenance(&mut self, config: FormatMaterializationConfig) {
        self.provenance_demand = config.provenance_demand;
        self.provenance_budgets = config.provenance_budgets;
    }

    /// Captures allocation-independent format state without naming a dump transition.
    pub fn capture_format_image(&self) -> Result<DetachedFormatImage, FormatError> {
        DetachedFormatImage::capture(self)
    }
}

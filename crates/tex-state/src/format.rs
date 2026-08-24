//! Portable detached format images and atomic destination materialization.
//!
//! Images are validated before a destination exists. Materialization then
//! rewrites handle-free contents into one fresh branded generation and moves
//! the complete candidate through a destination-identity barrier.

use core::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::generation::{GenerationBrand, with_generation};
use crate::interner::InternerBudget;
use crate::pdf::PdfState;
use crate::session_epoch::SessionInternerEpoch;
use crate::stores::StateCore;
use crate::{InteractionMode, ProvenanceBudgets, ProvenanceDemand, Universe, World};

pub(crate) mod schema;
use schema::{
    FormatCell, FormatCode, FormatDefinition, FormatFont, FormatGlue, FormatMeaning, FormatName,
    FormatNodeList, VersionedRows,
};

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
    string_pool: crate::command_context::StringPoolFormatState,
    pdf: Vec<u8>,
}

#[derive(Clone, Debug)]
struct DecodedFormat {
    metadata: FormatMetadata,
    names: Vec<FormatName>,
    token_lists: Vec<Vec<u32>>,
    definitions: Vec<FormatDefinition>,
    glue: Vec<FormatGlue>,
    fonts: Vec<FormatFont>,
    hyphenation: crate::hyphenation::HyphenationTable,
    node_lists: Vec<FormatNodeList>,
    cells: Vec<FormatCell>,
}

struct FormatNodeCollector<'a, G> {
    admitted: crate::stores::AdmittedState<'a, G>,
    rows: Vec<FormatNodeList>,
    indices: std::collections::HashMap<crate::node_arena::DurableListId<G>, u32>,
}

impl<'a, G> FormatNodeCollector<'a, G> {
    fn new(admitted: crate::stores::AdmittedState<'a, G>) -> Self {
        Self {
            admitted,
            rows: Vec::new(),
            indices: std::collections::HashMap::new(),
        }
    }

    fn capture_root(&mut self, root: crate::node_arena::DurableListId<G>) -> Result<u32, String> {
        if root.is_empty() {
            return Ok(0);
        }
        if let Some(&row) = self.indices.get(&root) {
            return Ok(row);
        }
        let nodes = self
            .admitted
            .node_list(root)
            .map_err(|_| "format node root is not live".to_owned())?
            .nodes()
            .to_vec();
        for node in &nodes {
            let mut children = Vec::new();
            node.visit_node_lists(|child| children.push(*child));
            for child in children {
                self.capture_root(child)?;
            }
        }
        let encoded = nodes
            .into_iter()
            .map(|mut node| {
                node.erase_diagnostic_sidecars();
                let node = node
                    .map_lists(|child| {
                        if child.is_empty() {
                            0
                        } else {
                            self.indices[&child]
                        }
                    })
                    .map_payloads(
                        crate::GlueId::format_index,
                        crate::TokenListId::format_index,
                    );
                bincode::serialize(&node)
                    .map_err(|error| format!("cannot encode format node: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let row = u32::try_from(self.rows.len())
            .ok()
            .and_then(|row| row.checked_add(1))
            .ok_or_else(|| "format node row count exceeds u32".to_owned())?;
        self.rows.push(FormatNodeList { nodes: encoded });
        self.indices.insert(root, row);
        Ok(row)
    }

    fn finish(self) -> Vec<FormatNodeList> {
        self.rows
    }
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
        let captured_names = universe.interner().capture_format_names();
        let names = encode_rows(captured_names.clone())?;
        let names_lookup =
            crate::frozen_lookup::encode(captured_names.iter().enumerate().map(|(slot, name)| {
                let mut key = Vec::with_capacity(name.text.len() + 1);
                key.push(name.kind);
                key.extend_from_slice(name.text.as_bytes());
                (key, slot as u32)
            }))
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        let core = universe
            .core
            .as_ref()
            .ok_or_else(|| FormatError::InvalidState("retired Universe".to_owned()))?;
        universe.validate_format_capture_state()?;
        let (definitions, token_lists, glue) = core.capture_format_values();
        let fonts = universe
            .fonts
            .capture_format_fonts(|font| core.state().capture_format_font_runtime(font))
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        let admitted = core.admit();
        let mut node_lists = FormatNodeCollector::new(admitted);
        let pdf = universe
            .pdf
            .capture_format_bytes(
                |tokens| {
                    bincode::serialize(&tokens.format_index())
                        .map_err(|error| format!("cannot encode format token root: {error}"))
                },
                |nodes| {
                    let row = node_lists.capture_root(nodes)?;
                    bincode::serialize(&row)
                        .map_err(|error| format!("cannot encode format node root: {error}"))
                },
            )
            .map_err(FormatError::InvalidState)?
            .ok_or(FormatError::NonEmptyPdfDocument)?;
        let mut cells = core
            .state()
            .capture_format_cells(|nodes| node_lists.capture_root(nodes))
            .map_err(FormatError::InvalidState)?;
        // e-TeX change 17.11's `Dump the e-TeX state` writes the extended-mode
        // bit to the format header, then disables every optional enhancement
        // before tex.web's table-of-equivalents dump. `TeXXeTstate` is the
        // sole e-TeX state cell, so it must not enter the portable eqtb image.
        // Filter the detached rows rather than mutating the admitted source:
        // validation failures and allocation failures therefore remain
        // mutation-free, as required by the format-capture boundary.
        cells.retain(|cell| {
            !matches!(
                cell,
                FormatCell::IntegerParameter(index, _)
                    if *index == crate::env::banks::IntParam::TEX_XET_STATE.raw()
            )
        });
        let (cells, codes): (Vec<_>, Vec<_>) = cells
            .into_iter()
            .partition(|cell| !matches!(cell, FormatCell::Code { .. }));
        let codes = codes
            .into_iter()
            .map(|cell| match cell {
                FormatCell::Code {
                    kind,
                    scalar,
                    value,
                } => FormatCode {
                    kind,
                    scalar,
                    value,
                },
                _ => unreachable!("partition retains only code cells"),
            })
            .collect::<Vec<_>>();
        let node_lists = node_lists.finish();
        let (variable_memory_words, dynamic_memory_words) = format_main_memory_live_words(
            &definitions,
            &token_lists,
            &glue,
            &node_lists,
            &cells,
            universe.engine_usage.uses_etex_node_sizes(),
        )?;
        let metadata = bincode::serialize(&FormatMetadata {
            version: SECTION_VERSION,
            interaction_mode: encode_interaction_mode(universe.interaction_mode),
            string_pool: universe
                .engine_usage
                .capture_format_state(variable_memory_words, dynamic_memory_words),
            pdf,
        })
        .map_err(|error| FormatError::InvalidState(error.to_string()))?;
        let token_lists = encode_rows(token_lists)?;
        let definitions = encode_rows(definitions)?;
        let glue = encode_rows(glue)?;
        let fonts = encode_rows(fonts)?;
        let codes = encode_rows(codes)?;
        let hyphenation = encode_rows(universe.hyphenation.clone())?;
        let node_lists = encode_rows(node_lists)?;
        let cells = encode_rows(cells)?;
        let empty = empty_section();
        let sections = REQUIRED_SECTION_KINDS.map(|kind| crate::format_container::SectionInput {
            kind,
            alignment: 8,
            bytes: match kind {
                1 => &metadata,
                256 => &names,
                257 => &names_lookup,
                272 => &token_lists,
                288 => &definitions,
                304 => &glue,
                320 => &fonts,
                336 => &codes,
                352 => &hyphenation,
                512 => &node_lists,
                528 => &cells,
                _ => &empty,
            },
        });
        Self::try_from_bytes(crate::format_container::encode(&sections)?)
    }
}

fn format_main_memory_live_words(
    definitions: &[FormatDefinition],
    token_lists: &[Vec<u32>],
    _glue: &[FormatGlue],
    node_lists: &[FormatNodeList],
    cells: &[FormatCell],
    etex_node_sizes: bool,
) -> Result<(usize, usize), FormatError> {
    // Immutable stores retain overwritten rows, whereas TeX frees them. The
    // detached environment cells are the format's actual owners, so only
    // their referenced rows enter the dumped `var_used`/`dyn_used` baseline.
    let mut owned_definitions = std::collections::BTreeSet::new();
    let mut owned_tokens = std::collections::BTreeSet::new();
    let mut owned_glue = std::collections::BTreeSet::new();
    let mut pending_nodes = Vec::new();
    for cell in cells {
        match *cell {
            FormatCell::Meaning(_, FormatMeaning::Macro { definition, .. }) => {
                owned_definitions.insert(definition as usize);
            }
            FormatCell::TokenRegister(_, row) | FormatCell::TokenParameter(_, row) => {
                owned_tokens.insert(row as usize);
            }
            FormatCell::GlueRegister(_, row)
            | FormatCell::MuGlueRegister(_, row)
            | FormatCell::GlueParameter(_, row) => {
                owned_glue.insert(row as usize);
            }
            FormatCell::BoxRegister(_, row) => pending_nodes.push(row as usize),
            _ => {}
        }
    }

    let mut owned_nodes = std::collections::BTreeSet::new();
    while let Some(row) = pending_nodes.pop() {
        if row == 0 || !owned_nodes.insert(row) {
            continue;
        }
        let list = node_lists
            .get(row - 1)
            .ok_or_else(|| FormatError::InvalidState("invalid format node owner".to_owned()))?;
        for encoded in &list.nodes {
            let node: crate::node::Node<u32, u32, u32> = bincode::deserialize(encoded)
                .map_err(|error| FormatError::InvalidState(error.to_string()))?;
            node.visit_node_lists(|child| pending_nodes.push(*child as usize));
            node.visit_payloads(
                |value| {
                    owned_glue.insert(*value as usize);
                },
                |value| {
                    owned_tokens.insert(*value as usize);
                },
            );
        }
    }

    // TeX82 §§130/133 reserve the five static glue specifications and the
    // fixed high-memory list heads before any format-owned value is loaded.
    let mut variable = 20_usize.saturating_add(owned_glue.len().saturating_mul(4));
    let mut dynamic = 14_usize;
    for index in owned_tokens {
        let tokens = &token_lists[index];
        dynamic = dynamic.saturating_add(tokens.len().saturating_add(1));
    }
    for index in owned_definitions {
        let definition = &definitions[index];
        dynamic = dynamic
            .saturating_add(definition.parameter_text.len())
            .saturating_add(definition.replacement_text.len())
            .saturating_add(2)
            // A nonempty definition has a live environment link beside the
            // reference head and `end_match`; the shared empty definition
            // does not allocate another one-word node.
            .saturating_add(usize::from(
                !definition.parameter_text.is_empty() || !definition.replacement_text.is_empty(),
            ));
    }
    for index in owned_nodes {
        let row = &node_lists[index - 1];
        for encoded in &row.nodes {
            let node: crate::node::Node<u32, u32, u32> = bincode::deserialize(encoded)
                .map_err(|error| FormatError::InvalidState(error.to_string()))?;
            let (node_variable, node_dynamic) = node.tex_memory_words(etex_node_sizes);
            variable = variable.saturating_add(node_variable);
            dynamic = dynamic.saturating_add(node_dynamic);
        }
    }
    Ok((variable, dynamic))
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
    crate::command_context::EngineUsageRuntime::restore_format_state(&metadata.string_pool)
        .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
    let names: Vec<FormatName> = decode_rows(required_section(&container, 256)?)?;
    let names_lookup =
        crate::frozen_lookup::decode(&required_section(&container, 257)?.bytes, names.len())
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
    for (slot, name) in names.iter().enumerate() {
        if names_lookup.get_prefixed(name.kind, name.text.as_bytes()) != Some(slot as u32) {
            return Err(FormatError::InvalidState(
                "format name lookup does not match names".to_owned(),
            ));
        }
    }
    let token_lists: Vec<Vec<u32>> = decode_rows(required_section(&container, 272)?)?;
    let definitions: Vec<FormatDefinition> = decode_rows(required_section(&container, 288)?)?;
    let glue: Vec<FormatGlue> = decode_rows(required_section(&container, 304)?)?;
    let fonts: Vec<FormatFont> = decode_rows(required_section(&container, 320)?)?;
    let codes: Vec<FormatCode> = decode_rows(required_section(&container, 336)?)?;
    let hyphenation: crate::hyphenation::HyphenationTable =
        decode_rows(required_section(&container, 352)?)?;
    hyphenation
        .validate_frozen()
        .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
    let node_lists: Vec<FormatNodeList> = decode_rows(required_section(&container, 512)?)?;
    let mut cells: Vec<FormatCell> = decode_rows(required_section(&container, 528)?)?;
    cells.extend(codes.into_iter().map(|code| FormatCell::Code {
        kind: code.kind,
        scalar: code.scalar,
        value: code.value,
    }));
    validate_logical_rows(
        &names,
        &token_lists,
        &definitions,
        &glue,
        &fonts,
        &node_lists,
        &cells,
    )?;
    validate_pdf_format_roots(&metadata.pdf, token_lists.len(), node_lists.len())?;
    Ok(DecodedFormat {
        metadata,
        names,
        token_lists,
        definitions,
        glue,
        fonts,
        hyphenation,
        node_lists,
        cells,
    })
}

fn required_section(
    container: &crate::format_container::DecodedContainer,
    kind: u32,
) -> Result<&crate::format_container::DecodedSection, FormatError> {
    container
        .section(kind)
        .ok_or_else(|| FormatError::InvalidState("missing required format section".to_owned()))
}

fn encode_rows<T: Serialize>(rows: T) -> Result<Vec<u8>, FormatError> {
    bincode::serialize(&VersionedRows {
        version: SECTION_VERSION,
        rows,
    })
    .map_err(|error| FormatError::InvalidState(error.to_string()))
}

fn decode_rows<T: for<'de> Deserialize<'de>>(
    section: &crate::format_container::DecodedSection,
) -> Result<T, FormatError> {
    let payload: VersionedRows<T> = bincode::deserialize(&section.bytes)
        .map_err(|error| FormatError::InvalidState(error.to_string()))?;
    if payload.version != SECTION_VERSION {
        return Err(FormatError::InvalidState(
            "unsupported format section version".to_owned(),
        ));
    }
    Ok(payload.rows)
}

fn validate_logical_rows(
    names: &[FormatName],
    token_lists: &[Vec<u32>],
    definitions: &[FormatDefinition],
    glue: &[FormatGlue],
    fonts: &[FormatFont],
    node_lists: &[FormatNodeList],
    cells: &[FormatCell],
) -> Result<(), FormatError> {
    use std::collections::BTreeSet;
    let mut distinct_names = BTreeSet::new();
    for name in names {
        if name.kind > 5 || !distinct_names.insert((name.kind, name.text.as_str())) {
            return Err(FormatError::InvalidState(
                "duplicate or unknown format name".to_owned(),
            ));
        }
        if name.kind == 3 && name.text.chars().count() != 1 {
            return Err(FormatError::InvalidState(
                "active format name is not one scalar".to_owned(),
            ));
        }
        if name.kind == 5 && name.hash_entry {
            return Err(FormatError::InvalidState(
                "format spelling cannot be a hash entry".to_owned(),
            ));
        }
    }
    let validate_words = |words: &[u32]| {
        for &raw in words {
            let token = crate::token::TokenWord::from_raw(raw)
                .token()
                .ok_or_else(|| FormatError::InvalidState("invalid format token".to_owned()))?;
            if let crate::token::Token::Cs(symbol) = token
                && names
                    .get(symbol.raw() as usize)
                    .is_none_or(|name| name.kind == 5)
            {
                return Err(FormatError::InvalidState(
                    "format token name reference is out of range".to_owned(),
                ));
            }
        }
        Ok(())
    };
    for words in token_lists {
        validate_words(words)?;
    }
    for definition in definitions {
        validate_words(&definition.parameter_text)?;
        validate_words(&definition.replacement_text)?;
    }
    for value in glue {
        if value.stretch_order > 3 || value.shrink_order > 3 {
            return Err(FormatError::InvalidState(
                "unknown format glue order".to_owned(),
            ));
        }
    }
    if fonts.is_empty() || fonts.len() > crate::font::MAX_FONT_COUNT {
        return Err(FormatError::InvalidState(
            "format font count is outside bank capacity".to_owned(),
        ));
    }
    for (raw, font) in fonts.iter().enumerate() {
        if raw == 0 && (font.name != "nullfont" || font.size != 0) {
            return Err(FormatError::InvalidState(
                "format font zero is not nullfont".to_owned(),
            ));
        }
        if font
            .identifier
            .is_some_and(|name| names.get(name as usize).is_none_or(|row| row.kind == 5))
        {
            return Err(FormatError::InvalidState(
                "format font identifier is out of range".to_owned(),
            ));
        }
        if font.font_info_words as usize > crate::font::WEB2C_FONT_INFO_CAPACITY {
            return Err(FormatError::InvalidState(
                "format font-info extent exceeds capacity".to_owned(),
            ));
        }
        if font.runtime.pdf_codes.len() != 9
            || font
                .runtime
                .pdf_codes
                .iter()
                .flatten()
                .any(|table| table.len() != 256)
            || font.runtime.parameters.len() > crate::font::WEB2C_FONT_INFO_CAPACITY
        {
            return Err(FormatError::InvalidState(
                "invalid format font runtime".to_owned(),
            ));
        }
    }
    validate_node_rows(names, token_lists, glue, fonts, node_lists)?;
    let mut keys = BTreeSet::new();
    for &cell in cells {
        let (key, reference) = match cell {
            FormatCell::Meaning(index, meaning) => {
                if names.get(index as usize).is_none_or(|name| name.kind == 5) {
                    return Err(FormatError::InvalidState(
                        "format meaning name reference is out of range".to_owned(),
                    ));
                }
                match meaning {
                    FormatMeaning::Macro { definition, .. }
                        if definition as usize >= definitions.len() =>
                    {
                        return Err(FormatError::InvalidState(
                            "format macro reference is out of range".to_owned(),
                        ));
                    }
                    FormatMeaning::Font(font) if font as usize >= fonts.len() => {
                        return Err(FormatError::InvalidState(
                            "format references an unloaded font".to_owned(),
                        ));
                    }
                    _ => {}
                }
                ((0, index), None)
            }
            FormatCell::Count(index, _) => ((1, u32::from(index)), None),
            FormatCell::Dimension(index, _) => ((2, u32::from(index)), None),
            FormatCell::TokenRegister(index, row) => ((3, u32::from(index)), Some((true, row))),
            FormatCell::GlueRegister(index, row) => ((4, u32::from(index)), Some((false, row))),
            FormatCell::MuGlueRegister(index, row) => ((5, u32::from(index)), Some((false, row))),
            FormatCell::BoxRegister(index, row) => {
                if row == 0 || row as usize > node_lists.len() {
                    return Err(FormatError::InvalidState(
                        "format box node reference is out of range".to_owned(),
                    ));
                }
                ((18, u32::from(index)), None)
            }
            FormatCell::IntegerParameter(index, _) if index < 128 => ((6, u32::from(index)), None),
            FormatCell::DimensionParameter(index, _) if index < 128 => {
                ((7, u32::from(index)), None)
            }
            FormatCell::TokenParameter(index, row) if index < 128 => {
                ((8, u32::from(index)), Some((true, row)))
            }
            FormatCell::GlueParameter(index, row) if index < 128 => {
                ((9, u32::from(index)), Some((false, row)))
            }
            FormatCell::CurrentFont(font) if (font as usize) < fonts.len() => ((10, 0), None),
            FormatCell::MathFamilyFont(index, font)
                if index < 48 && (font as usize) < fonts.len() =>
            {
                ((11, u32::from(index)), None)
            }
            FormatCell::Code { kind, scalar, .. } if kind < 6 && scalar <= 0x10ffff => {
                ((12 + u32::from(kind), scalar), None)
            }
            _ => {
                return Err(FormatError::InvalidState(
                    "invalid format environment cell".to_owned(),
                ));
            }
        };
        if !keys.insert(key) {
            return Err(FormatError::InvalidState(
                "duplicate format environment cell".to_owned(),
            ));
        }
        if let Some((tokens, row)) = reference {
            let len = if tokens {
                token_lists.len()
            } else {
                glue.len()
            };
            if row as usize >= len {
                return Err(FormatError::InvalidState(
                    "format environment reference is out of range".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_node_rows(
    names: &[FormatName],
    token_lists: &[Vec<u32>],
    glue: &[FormatGlue],
    fonts: &[FormatFont],
    rows: &[FormatNodeList],
) -> Result<(), FormatError> {
    for (index, row) in rows.iter().enumerate() {
        for bytes in &row.nodes {
            let node: crate::node::Node<u32, u32, u32> = bincode::deserialize(bytes)
                .map_err(|error| FormatError::InvalidState(error.to_string()))?;
            let mut valid_lists = true;
            node.visit_node_lists(|child| {
                valid_lists &= *child == 0 || (*child as usize) <= index;
            });
            let mut valid_glue = true;
            let mut valid_tokens = true;
            node.visit_payloads(
                |value| valid_glue &= (*value as usize) < glue.len(),
                |value| valid_tokens &= (*value as usize) < token_lists.len(),
            );
            let mut valid_fonts = true;
            node.visit_fonts(|font| valid_fonts &= (font.raw() as usize) < fonts.len());
            if !(valid_lists && valid_glue && valid_tokens && valid_fonts) {
                return Err(FormatError::InvalidState(
                    "format node reference is out of range or not topological".to_owned(),
                ));
            }
            validate_node_embedded_tokens(&node, names)?;
        }
    }
    Ok(())
}

fn validate_node_embedded_tokens(
    node: &crate::node::Node<u32, u32, u32>,
    names: &[FormatName],
) -> Result<(), FormatError> {
    let mut valid = true;
    node.visit_embedded_token_words(|word| {
        if let Some(crate::token::Token::Cs(symbol)) = word.token() {
            valid &= names
                .get(symbol.raw() as usize)
                .is_some_and(|name| name.kind != 5);
        }
    });
    if valid {
        Ok(())
    } else {
        Err(FormatError::InvalidState(
            "format node token name reference is out of range".to_owned(),
        ))
    }
}

fn validate_pdf_format_roots(
    bytes: &[u8],
    token_count: usize,
    node_count: usize,
) -> Result<(), FormatError> {
    PdfState::<()>::restore_format_bytes(
        bytes,
        |recipe| {
            let row: u32 = bincode::deserialize(recipe)
                .map_err(|error| format!("invalid PDF token root: {error}"))?;
            if row as usize >= token_count {
                return Err("PDF token root is out of range".to_owned());
            }
            let tokens = crate::TokenListId::format_validation_coordinate(row)
                .ok_or_else(|| "PDF token root overflows its coordinate".to_owned())?;
            Ok(crate::pdf::PdfTokenParameter {
                tokens,
                semantic_id: crate::state_hash::StateHashFragment::from_builder(
                    0x666d_745f_746f_6b6e,
                    |hasher| hasher.u32(row),
                ),
            })
        },
        |recipe| {
            let row: u32 = bincode::deserialize(recipe)
                .map_err(|error| format!("invalid PDF node root: {error}"))?;
            if row as usize > node_count {
                return Err("PDF node root is out of range".to_owned());
            }
            Ok((
                crate::node_arena::DurableListId::format_validation_coordinate(row),
                crate::state_hash::StateHashFragment::from_builder(
                    0x666d_745f_6e6f_6465,
                    |hasher| hasher.u32(row),
                ),
            ))
        },
    )
    .map(|_| ())
    .map_err(FormatError::InvalidState)
}

fn empty_section() -> [u8; SECTION_HEADER_LEN] {
    let mut bytes = [0; SECTION_HEADER_LEN];
    bytes[..4].copy_from_slice(&SECTION_VERSION.to_le_bytes());
    bytes[8..12].copy_from_slice(&(SECTION_HEADER_LEN as u32).to_le_bytes());
    bytes
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
        let epoch = SessionInternerEpoch::new(self.budget);
        let interner = epoch.lease().map_err(|_| FormatError::AllocationFailed)?;
        drop(epoch);
        self.stage_with_interner(image, interner)
    }

    fn stage_in_epoch(
        &mut self,
        image: &DetachedFormatImage,
        epoch: &SessionInternerEpoch,
    ) -> Result<FormatStaging<G>, FormatError> {
        let interner = epoch.lease().map_err(|error| {
            FormatError::InvalidState(format!("session epoch is not available: {error:?}"))
        })?;
        self.stage_with_interner(image, interner)
    }

    fn stage_with_interner(
        &mut self,
        image: &DetachedFormatImage,
        interner: crate::session_epoch::InternerLease,
    ) -> Result<FormatStaging<G>, FormatError> {
        let core = self.core.take().ok_or(FormatError::DestinationConsumed)?;
        let mut universe = Universe::new_format_candidate(interner, core);
        universe.install_format_logical_rows(&image.decoded)?;
        universe.interaction_mode =
            decode_interaction_mode(image.decoded.metadata.interaction_mode)?;
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
        let core = {
            #[cfg(feature = "profiling")]
            let _allocation_scope = crate::measurement::hot_core_allocation_scope(
                crate::measurement::HotCoreAllocationOwner::GenerationBoundary,
            );
            StateCore::new(generation).map_err(|_| FormatError::AllocationFailed)?
        };
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
        #[cfg(feature = "profiling")]
        let _allocation_scope = crate::measurement::hot_core_allocation_scope(
            crate::measurement::HotCoreAllocationOwner::ColdMaterialization,
        );
        let staging = destination.stage(image)?;
        destination
            .materialize(staging, use_universe)
            .map_err(|_| FormatError::InvalidState("foreign format destination".to_owned()))
    })
}

/// Materializes one image into a fresh revision generation while preserving
/// the caller's exact session interning epoch.
pub fn with_materialized_format_in_epoch<R>(
    budget: InternerBudget,
    world: World,
    epoch: &SessionInternerEpoch,
    image: &DetachedFormatImage,
    use_universe: impl for<'id> FnOnce(&mut Universe<GenerationBrand<'id>>) -> R,
) -> Result<R, FormatError> {
    with_format_destination(budget, world, |destination| {
        let staging = destination.stage_in_epoch(image, epoch)?;
        destination
            .materialize(staging, use_universe)
            .map_err(|_| FormatError::InvalidState("foreign format destination".to_owned()))
    })
}

pub(crate) fn materialize_retained_format<G>(
    interner: crate::session_epoch::InternerLease,
    generation: crate::generation::Generation<G>,
    world: World,
    image: &DetachedFormatImage,
) -> Result<Universe<G>, FormatError> {
    let core = StateCore::new(generation).map_err(|_| FormatError::AllocationFailed)?;
    let mut universe = Universe::new_format_candidate(interner, core);
    universe.install_format_logical_rows(&image.decoded)?;
    universe.interaction_mode = decode_interaction_mode(image.decoded.metadata.interaction_mode)?;
    universe.world = world;
    universe.refresh_job_clock_parameters().map_err(|error| {
        FormatError::InvalidState(format!("retained format clock refresh failed: {error:?}"))
    })?;
    Ok(universe)
}

impl<G> Universe<G> {
    fn validate_format_capture_state(&self) -> Result<(), FormatError> {
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
        Ok(())
    }

    pub(crate) fn new_format_candidate(
        interner: crate::session_epoch::InternerLease,
        core: StateCore<G>,
    ) -> Self {
        Self::new(interner, core)
    }

    fn install_format_pdf(
        &mut self,
        bytes: &[u8],
        token_lists: &[crate::TokenListId<G>],
        node_lists: &[crate::node_arena::DurableListId<G>],
    ) -> Result<(), FormatError> {
        self.pdf = PdfState::restore_format_bytes(
            bytes,
            |recipe| {
                let row: u32 = bincode::deserialize(recipe)
                    .map_err(|error| format!("invalid PDF token root: {error}"))?;
                let tokens = *token_lists
                    .get(row as usize)
                    .ok_or_else(|| "PDF token root is out of range".to_owned())?;
                let admitted = self
                    .core
                    .as_ref()
                    .expect("format candidate retains core")
                    .admit();
                let words = admitted.token_list(tokens);
                Ok(crate::pdf::PdfTokenParameter {
                    tokens,
                    semantic_id: crate::state_hash::StateHashFragment::from_exact_builder(
                        0x7064_665f_746f_6b70,
                        |hasher| {
                            hasher.usize(words.len());
                            for word in words {
                                hasher.u32(word.raw());
                            }
                        },
                    ),
                })
            },
            |recipe| {
                let row: u32 = bincode::deserialize(recipe)
                    .map_err(|error| format!("invalid PDF node root: {error}"))?;
                let nodes = if row == 0 {
                    crate::node_arena::DurableListId::empty()
                } else {
                    *node_lists
                        .get(row as usize - 1)
                        .ok_or_else(|| "PDF node root is out of range".to_owned())?
                };
                Ok((
                    nodes,
                    crate::state_hash::StateHashFragment::from_exact_builder(
                        0x666d_745f_6e6f_6465,
                        |hasher| hasher.u32(row),
                    ),
                ))
            },
        )
        .map_err(FormatError::InvalidState)?;
        Ok(())
    }

    fn install_format_logical_rows(&mut self, format: &DecodedFormat) -> Result<(), FormatError> {
        self.engine_usage = crate::command_context::EngineUsageRuntime::restore_format_state(
            &format.metadata.string_pool,
        )
        .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        for (slot, row) in format.names.iter().enumerate() {
            if let Some(symbol) = self
                .interner_mut()
                .install_format_name(slot as u32, row)
                .map_err(|message| FormatError::InvalidState(message.to_owned()))?
            {
                self.core
                    .as_mut()
                    .expect("format candidate retains core")
                    .state_mut()
                    .admit_symbol(symbol)
                    .map_err(|_| FormatError::AllocationFailed)?;
            }
        }
        self.fonts = crate::font::FontStore::restore_format_fonts(&format.fonts, self.interner())
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        let fonts = (0..self.fonts.len())
            .map(|slot| {
                self.fonts
                    .id_at(slot as u32)
                    .expect("restored format font prefix is dense")
            })
            .collect::<Vec<_>>();
        self.core
            .as_mut()
            .expect("format candidate retains core")
            .state_mut()
            .install_format_font_runtimes(&format.fonts)
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        let token_rows = format
            .token_lists
            .iter()
            .map(|words| {
                words
                    .iter()
                    .copied()
                    .map(crate::token::TokenWord::from_raw)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let definition_rows = format
            .definitions
            .iter()
            .map(|definition| {
                (
                    definition
                        .parameter_text
                        .iter()
                        .copied()
                        .map(crate::token::TokenWord::from_raw)
                        .collect::<Vec<_>>(),
                    definition
                        .replacement_text
                        .iter()
                        .copied()
                        .map(crate::token::TokenWord::from_raw)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let definition_promotions = definition_rows
            .iter()
            .map(
                |(parameter_text, replacement_text)| crate::DefinitionPromotion {
                    parameter_text,
                    replacement_text,
                },
            )
            .collect::<Vec<_>>();
        let token_promotions = token_rows
            .iter()
            .map(|words| crate::TokenListPromotion { words })
            .collect::<Vec<_>>();
        let glue = format
            .glue
            .iter()
            .map(|value| {
                Ok(crate::glue::GlueSpec {
                    width: crate::scaled::Scaled::from_raw(value.width),
                    stretch: crate::scaled::Scaled::from_raw(value.stretch),
                    stretch_order: decode_order(value.stretch_order)?,
                    shrink: crate::scaled::Scaled::from_raw(value.shrink),
                    shrink_order: decode_order(value.shrink_order)?,
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;
        let promoted = self
            .promote_values(&definition_promotions, &token_promotions, &glue, &[])
            .map_err(|_| FormatError::AllocationFailed)?;
        let node_lists = self.install_format_node_lists(
            &format.node_lists,
            &promoted.token_lists,
            &promoted.glue,
            &fonts,
        )?;
        self.core
            .as_mut()
            .expect("format candidate retains core")
            .state_mut()
            .install_format_cells(
                &format.cells,
                &promoted.definitions,
                &promoted.token_lists,
                &promoted.glue,
                &node_lists,
                &fonts,
            )
            .map_err(|message| FormatError::InvalidState(message.to_owned()))?;
        self.hyphenation = format.hyphenation.clone();
        self.install_format_pdf(&format.metadata.pdf, &promoted.token_lists, &node_lists)?;
        Ok(())
    }

    fn install_format_node_lists(
        &mut self,
        rows: &[FormatNodeList],
        token_lists: &[crate::TokenListId<G>],
        glue: &[crate::GlueId<G>],
        fonts: &[crate::ids::FontId],
    ) -> Result<Vec<crate::node_arena::DurableListId<G>>, FormatError> {
        let mut installed = Vec::with_capacity(rows.len());
        for row in rows {
            let nodes = row
                .nodes
                .iter()
                .map(|bytes| {
                    let node: crate::node::Node<u32, u32, u32> = bincode::deserialize(bytes)
                        .map_err(|error| FormatError::InvalidState(error.to_string()))?;
                    let node = node
                        .map_lists(|child| {
                            if child == 0 {
                                crate::node_arena::DurableListId::empty()
                            } else {
                                installed[child as usize - 1]
                            }
                        })
                        .map_payloads(
                            |value| glue[value as usize],
                            |value| token_lists[value as usize],
                        )
                        .map_fonts(|font| fonts[font.raw() as usize]);
                    Ok(node)
                })
                .collect::<Result<Vec<_>, FormatError>>()?;
            let id = self
                .core
                .as_mut()
                .expect("format candidate retains core")
                .admit_mut()
                .map_err(|_| FormatError::AllocationFailed)?
                .nodes_mut()
                .publish(nodes)
                .map_err(|_| FormatError::AllocationFailed)?;
            installed.push(id);
        }
        Ok(installed)
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

fn decode_order(raw: u8) -> Result<crate::glue::Order, FormatError> {
    match raw {
        0 => Ok(crate::glue::Order::Normal),
        1 => Ok(crate::glue::Order::Fil),
        2 => Ok(crate::glue::Order::Fill),
        3 => Ok(crate::glue::Order::Filll),
        _ => Err(FormatError::InvalidState(
            "unknown format glue order".to_owned(),
        )),
    }
}

//! Detached PDF assembly from checkpointed shipout receipts.

use tex_arith::Scaled;
use tex_out::pdf::{
    PdfContentRectangle, PdfDictionary, PdfIndirectObject, PdfModelError, PdfNumber, PdfObject,
    PdfObjectId, PdfSerializationOptions, PdfSerializeError, PdfStreamCompression, PdfValue,
    PdfVersion, UnvalidatedPdfDocument, filled_rectangle_content,
};
use tex_out::positioned::{PositionedError, PositionedEvent};
use tex_state::env::banks::IntParam;
use tex_state::{
    CommittedArtifact, ContentHash, PDF_CATALOG_OBJECT_ID, PDF_PAGES_OBJECT_ID, Universe,
    WorldError,
};

/// Builds one deterministic PDF from the current checkpointed page ledger.
pub fn pdf_from_committed_artifacts(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
) -> Result<Vec<u8>, PdfBuildError> {
    if stores.int_param(IntParam::PDF_OUTPUT) <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(stores)?;
    let options = serialization_options(stores)?;
    let catalog_id = object_id(PDF_CATALOG_OBJECT_ID)?;
    let pages_id = object_id(PDF_PAGES_OBJECT_ID)?;
    let page_records = stores.pdf_pages();
    let mut objects = Vec::with_capacity(2 + page_records.len() * 3);
    let mut kids = Vec::with_capacity(page_records.len());

    let mut catalog = PdfDictionary::new();
    catalog.insert("Type", PdfValue::Name("Catalog".into()))?;
    catalog.insert("Pages", PdfValue::Reference(pages_id))?;
    objects.push(indirect_dictionary(catalog_id, catalog));

    for (page_index, record) in page_records.iter().copied().enumerate() {
        let bytes = artifact_bytes(stores, artifacts, record.artifact())?;
        let artifact = tex_out::PageArtifact::from_bytes(&bytes)?;
        let positioned = tex_out::positioned::lower_page(&artifact, page_index as u32)?;
        let page_width = positive_extent(positioned.width);
        let page_height = positive_extent(positioned.height);
        let mut rectangles = Vec::new();
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => rectangles.push(PdfContentRectangle {
                    x: scaled_to_bp_f32(rule.x),
                    y: scaled_to_bp_f32(
                        page_height
                            .checked_sub(rule.y)
                            .and_then(|value| value.checked_sub(rule.height))
                            .ok_or(PositionedError::PositionOverflow)?,
                    ),
                    width: scaled_to_bp_f32(rule.width),
                    height: scaled_to_bp_f32(rule.height),
                }),
                PositionedEvent::TextRun(run) if !run.units.is_empty() => {
                    return Err(PdfBuildError::TextRequiresFontResources);
                }
                PositionedEvent::Special(special) => {
                    return Err(PdfBuildError::UnsupportedSpecial(special.class));
                }
                PositionedEvent::Box(_) | PositionedEvent::TextRun(_) => {}
            }
        }

        let resources_id = object_id(record.resources_object())?;
        let contents_id = object_id(record.contents_object())?;
        let page_id = object_id(record.page_object())?;
        kids.push(PdfValue::Reference(page_id));
        objects.push(indirect_dictionary(resources_id, PdfDictionary::new()));
        objects.push(PdfIndirectObject {
            id: contents_id,
            object: PdfObject::Stream {
                dictionary: PdfDictionary::new(),
                data: filled_rectangle_content(&rectangles),
            },
        });

        let mut page = PdfDictionary::new();
        page.insert("Type", PdfValue::Name("Page".into()))?;
        page.insert("Parent", PdfValue::Reference(pages_id))?;
        page.insert(
            "MediaBox",
            PdfValue::Array(vec![
                PdfValue::Integer(0),
                PdfValue::Integer(0),
                PdfValue::Number(scaled_to_bp_number(page_width)?),
                PdfValue::Number(scaled_to_bp_number(page_height)?),
            ]),
        )?;
        page.insert("Resources", PdfValue::Reference(resources_id))?;
        page.insert("Contents", PdfValue::Reference(contents_id))?;
        objects.push(indirect_dictionary(page_id, page));
    }

    let mut pages = PdfDictionary::new();
    pages.insert("Type", PdfValue::Name("Pages".into()))?;
    pages.insert("Count", PdfValue::Integer(page_records.len() as i64))?;
    pages.insert("Kids", PdfValue::Array(kids))?;
    objects.push(indirect_dictionary(pages_id, pages));

    let document = UnvalidatedPdfDocument {
        version,
        catalog: catalog_id,
        objects,
    }
    .validate()?;
    Ok(document.to_pdf_bytes_with_options(options)?)
}

fn artifact_bytes(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
    hash: ContentHash,
) -> Result<Vec<u8>, PdfBuildError> {
    if let Some(artifact) = artifacts.iter().find(|artifact| artifact.hash() == hash) {
        return Ok(artifact.bytes().to_vec());
    }
    stores
        .world()
        .read_artifact(hash)?
        .ok_or(PdfBuildError::MissingArtifact(hash))
}

fn pdf_version(stores: &Universe) -> Result<PdfVersion, PdfBuildError> {
    let major = u8::try_from(stores.int_param(IntParam::PDF_MAJOR_VERSION))
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    let minor = u8::try_from(stores.int_param(IntParam::PDF_MINOR_VERSION))
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    Ok(PdfVersion::new(major, minor)?)
}

fn serialization_options(stores: &Universe) -> Result<PdfSerializationOptions, PdfBuildError> {
    let level = stores.int_param(IntParam::PDF_COMPRESS_LEVEL);
    let stream_compression = match level {
        0 => PdfStreamCompression::None,
        1..=9 => PdfStreamCompression::Flate { level: level as u8 },
        _ => return Err(PdfBuildError::InvalidCompressionLevel(level)),
    };
    Ok(PdfSerializationOptions {
        pretty: false,
        stream_compression,
    })
}

fn object_id(raw: u32) -> Result<PdfObjectId, PdfBuildError> {
    PdfObjectId::new(raw).ok_or(PdfBuildError::InvalidObjectId(raw))
}

fn indirect_dictionary(id: PdfObjectId, dictionary: PdfDictionary) -> PdfIndirectObject {
    PdfIndirectObject {
        id,
        object: PdfObject::Value(PdfValue::Dictionary(dictionary)),
    }
}

fn positive_extent(value: Scaled) -> Scaled {
    if value.raw() > 0 {
        value
    } else {
        Scaled::from_raw(1)
    }
}

fn scaled_to_bp_f32(value: Scaled) -> f32 {
    value.raw() as f32 * 7200.0 / (7227.0 * 65536.0)
}

fn scaled_to_bp_number(value: Scaled) -> Result<PdfNumber, PdfModelError> {
    const SCALE: i128 = 100_000;
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value.raw()) * NUMERATOR * SCALE;
    let rounded = if numerator >= 0 {
        (numerator + DENOMINATOR / 2) / DENOMINATOR
    } else {
        (numerator - DENOMINATOR / 2) / DENOMINATOR
    };
    PdfNumber::new(rounded as i64, 5)
}

#[derive(Debug)]
pub enum PdfBuildError {
    PdfOutputDisabled,
    MissingArtifact(ContentHash),
    InvalidVersionParameters,
    InvalidCompressionLevel(i32),
    InvalidObjectId(u32),
    TextRequiresFontResources,
    UnsupportedSpecial(String),
    World(WorldError),
    Parse(tex_out::ParseError),
    Positioned(PositionedError),
    Model(PdfModelError),
    Serialize(PdfSerializeError),
}

impl std::fmt::Display for PdfBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PdfOutputDisabled => {
                f.write_str("PDF output requires \\pdfoutput greater than zero")
            }
            Self::MissingArtifact(hash) => {
                write!(f, "shipped page artifact {} is missing", hash.hex())
            }
            Self::InvalidVersionParameters => {
                f.write_str("pdfTeX PDF version parameters are outside 0..=255")
            }
            Self::InvalidCompressionLevel(level) => {
                write!(f, "invalid \\pdfcompresslevel {level}; expected 0..=9")
            }
            Self::InvalidObjectId(id) => write!(f, "invalid PDF object id {id}"),
            Self::TextRequiresFontResources => {
                f.write_str("PDF text output requires embedded font resources")
            }
            Self::UnsupportedSpecial(class) => {
                write!(f, "PDF output does not support special class {class:?}")
            }
            Self::World(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Positioned(error) => error.fmt(f),
            Self::Model(error) => error.fmt(f),
            Self::Serialize(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PdfBuildError {}

impl From<WorldError> for PdfBuildError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
    }
}
impl From<tex_out::ParseError> for PdfBuildError {
    fn from(value: tex_out::ParseError) -> Self {
        Self::Parse(value)
    }
}
impl From<PositionedError> for PdfBuildError {
    fn from(value: PositionedError) -> Self {
        Self::Positioned(value)
    }
}
impl From<PdfModelError> for PdfBuildError {
    fn from(value: PdfModelError) -> Self {
        Self::Model(value)
    }
}
impl From<PdfSerializeError> for PdfBuildError {
    fn from(value: PdfSerializeError) -> Self {
        Self::Serialize(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DirectFontResolver, RejectingMemoryInputResolver, RunResult, dvi_from_page_plans,
        prepare_pdftex_run_stores, run_input_collecting_artifacts,
    };
    use tex_exec::ExecutionContext;
    use tex_lex::{InputStack, MemoryInput};

    fn run_in(stores: &mut Universe, source: &str) -> RunResult {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let context =
            ExecutionContext::with_resolvers("pdf-test", &mut input_resolver, &mut font_resolver);
        run_input_collecting_artifacts(&mut input, stores, context).expect("minimal page ships")
    }

    fn run(source: &str) -> (Universe, RunResult) {
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        let result = run_in(&mut stores, source);
        (stores, result)
    }

    #[test]
    fn minimal_rule_page_emits_deterministic_valid_pdf_structure() {
        let source =
            "\\pdfoutput=1\\pdfcompresslevel=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end";
        let mut stores = Universe::default();
        prepare_pdftex_run_stores(&mut stores);
        stores
            .begin_retained_session()
            .expect("retained test session starts");
        let before = stores.snapshot();
        let first_run = run_in(&mut stores, source);
        let first = pdf_from_committed_artifacts(&stores, &first_run.committed_artifacts)
            .expect("PDF assembles");
        let first_pages = stores.pdf_pages().to_vec();
        let first_hash = stores.snapshot().state_hash();

        stores.rollback(&before);
        let second_run = run_in(&mut stores, source);
        let second = pdf_from_committed_artifacts(&stores, &second_run.committed_artifacts)
            .expect("PDF replay assembles");

        assert_eq!(first, second);
        assert_eq!(stores.pdf_pages(), first_pages);
        assert_eq!(stores.snapshot().state_hash(), first_hash);
        assert!(first.starts_with(b"%PDF-1.4"));
        assert!(first.windows(2).any(|window| window == b"re"));
        assert_eq!(stores.pdf_pages().len(), 1);
        assert_eq!(stores.pdf_pages()[0].resources_object(), 3);
        assert_eq!(stores.pdf_pages()[0].contents_object(), 4);
        assert_eq!(stores.pdf_pages()[0].page_object(), 5);
    }

    #[test]
    fn enabling_pdf_mode_does_not_change_dvi_page_bytes() {
        let (_, dvi_run) = run("\\pdfoutput=0\\shipout\\vbox{\\hrule width10pt height5pt}\\end");
        let (_, pdf_run) = run("\\pdfoutput=1\\shipout\\vbox{\\hrule width10pt height5pt}\\end");
        assert_eq!(
            dvi_from_page_plans(&dvi_run.dvi_pages).expect("DVI assembles"),
            dvi_from_page_plans(&pdf_run.dvi_pages).expect("DVI assembles"),
        );
    }
}

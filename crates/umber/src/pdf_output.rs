//! Detached PDF assembly from checkpointed shipout receipts.

use tex_arith::Scaled;
use tex_expand::append_token_string_text;
use tex_out::PageNode;
use tex_out::pdf::{
    PdfContentRectangle, PdfDictionary, PdfIndirectObject, PdfModelError, PdfNumber, PdfObject,
    PdfObjectCompression, PdfObjectId, PdfSerializationOptions, PdfSerializeError,
    PdfStreamCompression, PdfValue, PdfVersion, UnvalidatedPdfDocument, filled_rectangle_content,
};
use tex_out::positioned::{PositionedError, PositionedEvent};
use tex_state::env::banks::{IntParam, TokParam};
use tex_state::ids::TokenListId;
use tex_state::{
    CommittedArtifact, ContentHash, PDF_CATALOG_OBJECT_ID, PDF_PAGES_OBJECT_ID,
    PdfOutputParameters, Universe, WorldError,
};

/// Builds one deterministic PDF from the current checkpointed page ledger.
pub fn pdf_from_committed_artifacts(
    stores: &Universe,
    artifacts: &[CommittedArtifact],
) -> Result<Vec<u8>, PdfBuildError> {
    let parameters = output_parameters(stores);
    if parameters.output <= 0 {
        return Err(PdfBuildError::PdfOutputDisabled);
    }
    let version = pdf_version(parameters)?;
    let options = serialization_options(parameters)?;
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
        let (page_width, page_height) = pdf_page_extents(&artifact, record)?;
        let mut rectangles = Vec::new();
        for event in positioned.events {
            match event {
                PositionedEvent::Rule(rule) => rectangles.push(PdfContentRectangle {
                    x: scaled_to_bp_f32(
                        rule.x
                            .checked_add(record.h_origin())
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    ),
                    y: scaled_to_bp_f32(
                        page_height
                            .checked_sub(rule.y)
                            .and_then(|value| value.checked_sub(record.v_origin()))
                            .and_then(|value| value.checked_sub(rule.height))
                            .ok_or(PdfBuildError::PageGeometryOverflow)?,
                        parameters.decimal_digits,
                    ),
                    width: scaled_to_bp_f32(rule.width, parameters.decimal_digits),
                    height: scaled_to_bp_f32(rule.height, parameters.decimal_digits),
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
        let mut resources = PdfDictionary::new();
        resources.insert(
            "ProcSet",
            PdfValue::Array(vec![PdfValue::Name("PDF".into())]),
        )?;
        resources.set_raw_entries(token_list_bytes(stores, record.resources()));
        objects.push(indirect_dictionary(resources_id, resources));
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
        let page_attr = token_list_bytes(stores, record.page_attr());
        if !page_attr
            .windows(b"/MediaBox".len())
            .any(|window| window == b"/MediaBox")
        {
            page.insert(
                "MediaBox",
                PdfValue::Array(vec![
                    PdfValue::Integer(0),
                    PdfValue::Integer(0),
                    PdfValue::Number(scaled_to_bp_number(page_width, parameters.decimal_digits)?),
                    PdfValue::Number(scaled_to_bp_number(page_height, parameters.decimal_digits)?),
                ]),
            )?;
        }
        page.insert("Resources", PdfValue::Reference(resources_id))?;
        page.insert("Contents", PdfValue::Reference(contents_id))?;
        page.set_raw_entries(page_attr);
        objects.push(indirect_dictionary(page_id, page));
    }

    let mut pages = PdfDictionary::new();
    pages.insert("Type", PdfValue::Name("Pages".into()))?;
    pages.insert("Count", PdfValue::Integer(page_records.len() as i64))?;
    pages.insert("Kids", PdfValue::Array(kids))?;
    pages.set_raw_entries(token_list_bytes(
        stores,
        stores.tok_param(TokParam::PDF_PAGES_ATTR),
    ));
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

fn output_parameters(stores: &Universe) -> PdfOutputParameters {
    stores.fixed_pdf_output_parameters().unwrap_or_else(|| {
        PdfOutputParameters {
            output: stores.int_param(IntParam::PDF_OUTPUT),
            major_version: stores.int_param(IntParam::PDF_MAJOR_VERSION),
            minor_version: stores.int_param(IntParam::PDF_MINOR_VERSION),
            compress_level: stores.int_param(IntParam::PDF_COMPRESS_LEVEL),
            object_compress_level: stores.int_param(IntParam::PDF_OBJ_COMPRESS_LEVEL),
            decimal_digits: stores.int_param(IntParam::PDF_DECIMAL_DIGITS),
            gamma: stores.int_param(IntParam::PDF_GAMMA),
            image_gamma: stores.int_param(IntParam::PDF_IMAGE_GAMMA),
            image_hicolor: stores.int_param(IntParam::PDF_IMAGE_HICOLOR),
            image_apply_gamma: stores.int_param(IntParam::PDF_IMAGE_APPLY_GAMMA),
            draft_mode: stores.int_param(IntParam::PDF_DRAFT_MODE),
            inclusion_copy_fonts: stores.int_param(IntParam::PDF_INCLUSION_COPY_FONTS),
            pk_resolution: stores.int_param(IntParam::PDF_PK_RESOLUTION),
            unique_resource_names: stores.int_param(IntParam::PDF_UNIQUE_RESNAME),
        }
        .normalized()
    })
}

fn pdf_version(parameters: PdfOutputParameters) -> Result<PdfVersion, PdfBuildError> {
    let major = u8::try_from(parameters.major_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    let minor = u8::try_from(parameters.minor_version)
        .map_err(|_| PdfBuildError::InvalidVersionParameters)?;
    Ok(PdfVersion::new(major, minor)?)
}

fn serialization_options(
    parameters: PdfOutputParameters,
) -> Result<PdfSerializationOptions, PdfBuildError> {
    let level = parameters.compress_level;
    let stream_compression = match level {
        ..=0 => PdfStreamCompression::None,
        1..=9 => PdfStreamCompression::Flate { level: level as u8 },
        _ => return Err(PdfBuildError::InvalidCompressionLevel(level)),
    };
    let object_compression = match parameters.object_compress_level {
        0 => PdfObjectCompression::None,
        level @ 1..=3 => PdfObjectCompression::ObjectStreams { level: level as u8 },
        level => return Err(PdfBuildError::InvalidObjectCompressionLevel(level)),
    };
    Ok(PdfSerializationOptions {
        pretty: false,
        stream_compression,
        object_compression,
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

fn pdf_page_extents(
    artifact: &tex_out::PageArtifact,
    record: tex_state::PdfPageRecord,
) -> Result<(Scaled, Scaled), PdfBuildError> {
    let root = match &artifact.root {
        PageNode::HList(root) | PageNode::VList(root) => root,
        _ => unreachable!("validated artifact root is a box"),
    };
    let h_offset = record
        .h_origin()
        .checked_add(artifact.job.h_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let v_offset = record
        .v_origin()
        .checked_add(artifact.job.v_offset)
        .ok_or(PdfBuildError::PageGeometryOverflow)?;
    let width = if record.width().raw() == 0 {
        root.width
            .checked_add(h_offset)
            .and_then(|value| value.checked_add(h_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.width()
    };
    let height = if record.height().raw() == 0 {
        root.height
            .checked_add(root.depth)
            .and_then(|value| value.checked_add(v_offset))
            .and_then(|value| value.checked_add(v_offset))
            .ok_or(PdfBuildError::PageGeometryOverflow)?
    } else {
        record.height()
    };
    Ok((width, height))
}

fn token_list_bytes(stores: &Universe, id: TokenListId) -> Vec<u8> {
    let mut text = String::new();
    for &token in stores.tokens(id) {
        append_token_string_text(stores, token, &mut text);
    }
    text.into_bytes()
}

fn scaled_to_bp_f32(value: Scaled, decimal_digits: i32) -> f32 {
    let scale = 10_f32.powi(decimal_digits);
    scaled_to_bp_coefficient(value, decimal_digits) as f32 / scale
}

fn scaled_to_bp_number(value: Scaled, decimal_digits: i32) -> Result<PdfNumber, PdfModelError> {
    PdfNumber::new(
        scaled_to_bp_coefficient(value, decimal_digits),
        decimal_digits as u8,
    )
}

fn scaled_to_bp_coefficient(value: Scaled, decimal_digits: i32) -> i64 {
    let scale = 10_i128.pow(decimal_digits as u32);
    const NUMERATOR: i128 = 7_200;
    const DENOMINATOR: i128 = 7_227 * 65_536;
    let numerator = i128::from(value.raw()) * NUMERATOR * scale;
    let rounded = if numerator >= 0 {
        (numerator + DENOMINATOR / 2) / DENOMINATOR
    } else {
        (numerator - DENOMINATOR / 2) / DENOMINATOR
    };
    rounded as i64
}

#[derive(Debug)]
pub enum PdfBuildError {
    PdfOutputDisabled,
    MissingArtifact(ContentHash),
    InvalidVersionParameters,
    InvalidCompressionLevel(i32),
    InvalidObjectCompressionLevel(i32),
    PageGeometryOverflow,
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
            Self::InvalidObjectCompressionLevel(level) => {
                write!(f, "invalid \\pdfobjcompresslevel {level}; expected 0..=3")
            }
            Self::PageGeometryOverflow => f.write_str("pdfTeX page geometry arithmetic overflowed"),
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

    fn try_run_in(stores: &mut Universe, source: &str) -> Result<RunResult, tex_exec::ExecError> {
        let mut input = InputStack::new(MemoryInput::new(source));
        let mut input_resolver = RejectingMemoryInputResolver;
        let mut font_resolver = DirectFontResolver;
        let context =
            ExecutionContext::with_resolvers("pdf-test", &mut input_resolver, &mut font_resolver);
        run_input_collecting_artifacts(&mut input, stores, context)
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
        assert!(
            first
                .windows(b"/ProcSet[/PDF]".len())
                .any(|window| window == b"/ProcSet[/PDF]")
        );
        assert!(first.windows(2).any(|window| window == b"re"));
        assert_eq!(stores.pdf_pages().len(), 1);
        assert_eq!(stores.pdf_pages()[0].resources_object(), 3);
        assert_eq!(stores.pdf_pages()[0].contents_object(), 4);
        assert_eq!(stores.pdf_pages()[0].page_object(), 5);
    }

    fn pdf_number(object: &lopdf::Object) -> f32 {
        match object {
            lopdf::Object::Integer(value) => *value as f32,
            lopdf::Object::Real(value) => *value,
            other => panic!("expected PDF number, got {other:?}"),
        }
    }

    #[test]
    fn page_parameters_are_consumed_at_pdftex_scopes() {
        let (stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfdecimaldigits=3",
            "\\pdfpagesattr{/Lang (early)}",
            "\\pdfpagewidth=100bp\\pdfpageheight=200bp",
            "\\pdfhorigin=10bp\\pdfvorigin=20bp",
            "\\pdfpageattr{/Rotate 90}",
            "\\pdfpageresources{/ExtGState << /A << /Type /ExtGState >> >>}",
            "\\shipout\\vbox{\\hrule width1bp height2bp}",
            "\\pdfpagewidth=300bp\\pdfpageheight=400bp",
            "\\pdfhorigin=30bp\\pdfvorigin=40bp",
            "\\pdfpageattr{/Rotate 180}",
            "\\pdfpageresources{/ColorSpace << /C /DeviceRGB >>}",
            "\\shipout\\vbox{\\hrule width3bp height4bp}",
            "\\pdfpagesattr{/Lang (final)}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let pages = parsed.get_pages();
        assert_eq!(pages.len(), 2);

        let pages_root = parsed
            .get_object((PDF_PAGES_OBJECT_ID, 0))
            .expect("pages root")
            .as_dict()
            .expect("pages dictionary");
        assert_eq!(
            pages_root
                .get(b"Lang")
                .expect("final pages attribute")
                .as_str()
                .expect("language string"),
            b"final"
        );

        for (number, expected_box, expected_rotate, resource_key) in [
            (1, [0.0, 0.0, 100.0, 200.0], 90, b"ExtGState".as_slice()),
            (2, [0.0, 0.0, 300.0, 400.0], 180, b"ColorSpace".as_slice()),
        ] {
            let page_id = pages[&number];
            let page = parsed
                .get_object(page_id)
                .expect("page")
                .as_dict()
                .expect("page dictionary");
            let media_box = page
                .get(b"MediaBox")
                .expect("MediaBox")
                .as_array()
                .expect("MediaBox array");
            for (actual, expected) in media_box.iter().map(pdf_number).zip(expected_box) {
                assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
            }
            assert_eq!(
                page.get(b"Rotate")
                    .expect("rotation")
                    .as_i64()
                    .expect("integer rotation"),
                expected_rotate
            );
            let resources_id = page
                .get(b"Resources")
                .expect("resources")
                .as_reference()
                .expect("resources reference");
            let resources = parsed
                .get_object(resources_id)
                .expect("resources")
                .as_dict()
                .expect("resources dictionary");
            assert!(resources.has(resource_key));
        }

        assert!(
            pdf.windows(b"10 178 1 2 re".len())
                .any(|window| { window == b"10 178 1 2 re" })
        );
        assert!(
            pdf.windows(b"30 356 3 4 re".len())
                .any(|window| { window == b"30 356 3 4 re" })
        );
    }

    #[test]
    fn raw_media_box_overrides_automatic_box_and_pk_mode_freezes() {
        let (stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfpkmode{first}",
            "\\pdfpagewidth=100bp\\pdfpageheight=200bp",
            "\\pdfpageattr{/MediaBox [1 2 3 4] /Rotate 90}",
            "\\shipout\\vbox{\\hrule width1bp height1bp}",
            "\\pdfpkmode{second}\\end",
        ));
        let fixed_pk_mode = stores.fixed_pdf_pk_mode().expect("PK mode frozen");
        assert_eq!(token_list_bytes(&stores, fixed_pk_mode), b"first");
        assert_eq!(
            token_list_bytes(&stores, stores.tok_param(TokParam::PDF_PK_MODE)),
            b"second"
        );

        let pdf = pdf_from_committed_artifacts(&stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        assert_eq!(
            pdf.windows(b"/MediaBox".len())
                .filter(|window| *window == b"/MediaBox")
                .count(),
            1
        );
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page")
            .as_dict()
            .expect("page dictionary");
        let media_box = page
            .get(b"MediaBox")
            .expect("raw MediaBox")
            .as_array()
            .expect("MediaBox array");
        assert_eq!(
            media_box.iter().map(pdf_number).collect::<Vec<_>>(),
            vec![1.0, 2.0, 3.0, 4.0]
        );
    }

    #[test]
    fn zero_page_dimensions_fall_back_to_box_plus_twice_the_origins() {
        let (stores, run_result) = run(concat!(
            "\\pdfoutput=1\\pdfcompresslevel=0\\pdfdecimaldigits=3",
            "\\pdfpagewidth=0pt\\pdfpageheight=0pt",
            "\\pdfhorigin=10bp\\pdfvorigin=20bp",
            "\\shipout\\vbox{\\hrule width1bp height2bp}\\end",
        ));
        let pdf = pdf_from_committed_artifacts(&stores, &run_result.committed_artifacts)
            .expect("PDF assembles");
        let parsed = lopdf::Document::load_mem(&pdf).expect("lopdf parses output");
        let page_id = parsed.get_pages()[&1];
        let page = parsed
            .get_object(page_id)
            .expect("page")
            .as_dict()
            .expect("page dictionary");
        let media_box = page
            .get(b"MediaBox")
            .expect("MediaBox")
            .as_array()
            .expect("MediaBox array");
        let actual = media_box.iter().map(pdf_number).collect::<Vec<_>>();
        for (actual, expected) in actual.iter().zip([0.0, 0.0, 21.0, 42.0]) {
            assert!((*actual - expected).abs() < 0.002, "{actual} != {expected}");
        }
        assert!(
            pdf.windows(b"10 20 1 2 re".len())
                .any(|window| window == b"10 20 1 2 re")
        );
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

    #[test]
    fn fixed_policy_drives_version_compression_and_decimal_output() {
        let (stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfmajorversion=1\\pdfminorversion=5",
            "\\pdfcompresslevel=0\\pdfobjcompresslevel=1\\pdfdecimaldigits=0",
            "\\shipout\\vbox{\\hrule width10pt height5pt}",
            "\\pdfcompresslevel=9\\pdfobjcompresslevel=0\\pdfdecimaldigits=4",
            "\\shipout\\vbox{\\hrule width10pt height5pt}\\end",
        ));
        let bytes = pdf_from_committed_artifacts(&stores, &run.committed_artifacts)
            .expect("fixed-policy PDF assembles");

        assert!(bytes.starts_with(b"%PDF-1.5"));
        assert!(bytes.windows(12).any(|window| window == b"/Type/ObjStm"));
        let parsed = lopdf::Document::load_mem(&bytes).expect("fixed-policy PDF parses");
        assert_eq!(parsed.get_pages().len(), 2);
        let contents = parsed
            .get_object((4, 0))
            .expect("first contents")
            .as_stream()
            .expect("contents stream");
        assert!(contents.dict.get(b"Filter").is_err());
    }

    #[test]
    fn frozen_output_mode_and_version_changes_are_fatal_setup_errors() {
        for (assignment, expected) in [
            ("\\pdfminorversion=7", "PDF version cannot be changed"),
            ("\\pdfoutput=0", "\\pdfoutput can only be changed"),
            (
                "\\pdfdraftmode=1",
                "\\pdfdraftmode can only be changed before anything is written",
            ),
        ] {
            let mut stores = Universe::default();
            prepare_pdftex_run_stores(&mut stores);
            let source = format!(
                "\\pdfoutput=1\\pdfminorversion=5\\shipout\\vbox{{\\hrule width1pt height1pt}}{assignment}\\shipout\\vbox{{\\hrule width1pt height1pt}}\\end"
            );
            let error = try_run_in(&mut stores, &source).expect_err("setup error must succumb");
            assert!(error.to_string().contains(expected), "{error}");
            assert_eq!(stores.pdf_pages().len(), 1);
        }
    }

    #[test]
    fn object_compression_levels_one_through_three_emit_type_two_xrefs() {
        for level in 1..=3 {
            let (stores, run) = run(&format!(
                "\\pdfoutput=1\\pdfminorversion=5\\pdfcompresslevel=6\\pdfobjcompresslevel={level}\\shipout\\vbox{{\\hrule width10pt height5pt}}\\end"
            ));
            let first = pdf_from_committed_artifacts(&stores, &run.committed_artifacts)
                .expect("object-stream PDF assembles");
            let second = pdf_from_committed_artifacts(&stores, &run.committed_artifacts)
                .expect("object-stream PDF repeats");
            assert_eq!(first, second);
            assert!(first.windows(12).any(|window| window == b"/Type/ObjStm"));
            assert!(first.windows(10).any(|window| window == b"/Type/XRef"));

            let parsed = lopdf::Document::load_mem(&first).expect("object-stream PDF parses");
            assert_eq!(parsed.get_pages().len(), 1);
            let contents = parsed
                .get_object((4, 0))
                .expect("ordinary content stream")
                .as_stream()
                .expect("contents stream");
            assert_eq!(
                contents
                    .dict
                    .get(b"Filter")
                    .expect("flate filter")
                    .as_name()
                    .expect("filter name"),
                b"FlateDecode"
            );
        }
    }

    #[test]
    fn invalid_version_and_object_policy_recover_like_pdftex() {
        let (stores, run) = run(concat!(
            "\\pdfoutput=1\\pdfmajorversion=0\\pdfminorversion=12",
            "\\pdfobjcompresslevel=9\\pdfdecimaldigits=9",
            "\\shipout\\vbox{\\hrule width10pt height5pt}\\end",
        ));
        let fixed = stores
            .fixed_pdf_output_parameters()
            .expect("shipout freezes recovered values");
        assert_eq!(fixed.major_version, 1);
        assert_eq!(fixed.minor_version, 4);
        assert_eq!(fixed.object_compress_level, 0);
        assert_eq!(fixed.decimal_digits, 4);
        let diagnostics = String::from_utf8_lossy(
            stores
                .world()
                .memory_terminal_output()
                .expect("memory terminal output"),
        );
        assert!(
            diagnostics.contains("pdfTeX error (invalid pdfmajorversion)"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("pdfTeX error (invalid pdfminorversion)"),
            "{diagnostics}"
        );
        assert!(
            diagnostics.contains("Object streams disabled now"),
            "{diagnostics}"
        );
        let bytes = pdf_from_committed_artifacts(&stores, &run.committed_artifacts)
            .expect("recovered PDF assembles");
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(!bytes.windows(12).any(|window| window == b"/Type/ObjStm"));
    }
}

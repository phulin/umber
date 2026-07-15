//! Canonical adapter from Umber's validated graph to `pdf_writer`.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use pdf_writer::types::{ActionType, AnnotationType, Predictor};
use pdf_writer::{
    Dict, Filter, Finish, Name, Null, Obj, Pdf, Raw, Rect, Ref, Settings, Str, XRefFilter,
};

use super::{
    PdfAnnotationAction, PdfAnnotationObject, PdfAnnotationType, PdfDestinationActionKind,
    PdfDestinationPage, PdfDestinationStructure, PdfDestinationTarget, PdfDictionary, PdfDocument,
    PdfImageColorSpace, PdfImageFilter, PdfImageXObject, PdfNumber, PdfObject, PdfObjectId,
    PdfValue,
};

/// Deterministic stream encoding selected at final serialization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfStreamCompression {
    /// Preserve the validated stream bytes without adding a filter.
    #[default]
    None,
    /// Compress every unfiltered stream with deterministic zlib/DEFLATE.
    Flate { level: u8 },
}

/// Object-stream policy corresponding to `\pdfobjcompresslevel`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfObjectCompression {
    /// Write all indirect values as ordinary type-1 objects.
    #[default]
    None,
    /// Place eligible non-stream objects in a true object stream.
    ObjectStreams { level: u8 },
}

/// Byte-format policy applied without changing document semantic identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PdfSerializationOptions {
    /// Ask `pdf_writer` for human-readable whitespace and indentation.
    pub pretty: bool,
    /// Encoding policy for stream payloads.
    pub stream_compression: PdfStreamCompression,
    /// Encoding policy for eligible non-stream indirect values.
    pub object_compression: PdfObjectCompression,
}

/// Typed failure raised before any private output buffer is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfSerializeError {
    ObjectIdOutOfRange(PdfObjectId),
    IntegerOutOfRange(i64),
    InvalidCompressionLevel(u8),
    InvalidObjectCompressionLevel(u8),
    ObjectStreamsRequirePdf15,
    ObjectIdSpaceExhausted,
    CompressionFilterConflict(PdfObjectId),
    Compression(std::io::ErrorKind),
}

impl std::fmt::Display for PdfSerializeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot serialize detached PDF model: {self:?}")
    }
}

impl std::error::Error for PdfSerializeError {}

impl PdfDocument {
    /// Serializes with compact, uncompressed deterministic defaults.
    pub fn to_pdf_bytes(&self) -> Result<Vec<u8>, PdfSerializeError> {
        self.to_pdf_bytes_with_options(PdfSerializationOptions::default())
    }

    /// Serializes the validated graph exclusively through `pdf_writer`.
    pub fn to_pdf_bytes_with_options(
        &self,
        options: PdfSerializationOptions,
    ) -> Result<Vec<u8>, PdfSerializeError> {
        validate_serialization_inputs(self, options)?;

        let mut pdf = Pdf::with_settings(Settings {
            pretty: options.pretty,
        });
        pdf.set_version(self.version().major(), self.version().minor());
        if let Some(file_id) = &self.trailer().file_id {
            pdf.set_file_id(file_id.clone());
        }
        pdf.set_trailer_raw_entries(self.trailer().raw_entries.clone());

        for indirect in self.objects() {
            let reference = writer_ref(indirect.id)?;
            if indirect.id == self.catalog() {
                let PdfObject::Value(PdfValue::Dictionary(dictionary)) = &indirect.object else {
                    unreachable!("validated PDF catalog is a dictionary")
                };
                let mut catalog = pdf.catalog(reference);
                write_dictionary_entries(&mut catalog, dictionary, Some(b"Type"))?;
                catalog.finish();
                continue;
            }
            if self.trailer().info == Some(indirect.id) {
                let PdfObject::Value(PdfValue::Dictionary(dictionary)) = &indirect.object else {
                    unreachable!("validated PDF info object is a dictionary")
                };
                let mut info = pdf.document_info(reference);
                write_dictionary_entries(&mut info, dictionary, None)?;
                info.finish();
                continue;
            }
            if let PdfObject::Value(PdfValue::Dictionary(dictionary)) = &indirect.object
                && matches!(dictionary.get(b"Type"), Some(PdfValue::Name(name)) if name.as_bytes() == b"Page")
            {
                let mut page = pdf.page(reference);
                if let Some(PdfValue::Array(ids)) = dictionary.get(b"Annots") {
                    page.annotations(ids.iter().map(|value| match value {
                        PdfValue::Reference(id) => {
                            writer_ref(*id).expect("validated annotation ref")
                        }
                        _ => unreachable!("validated page annotation is a reference"),
                    }));
                }
                if let Some(PdfValue::Reference(resources)) = dictionary.get(b"Resources") {
                    page.resources_ref(writer_ref(*resources)?);
                }
                if let Some(PdfValue::Reference(group)) = dictionary.get(b"Group") {
                    page.group_ref(writer_ref(*group)?);
                }
                write_dictionary_entries_skipping(
                    &mut page,
                    dictionary,
                    &[b"Type", b"Annots", b"Resources", b"Group"],
                )?;
                page.finish();
                continue;
            }

            match &indirect.object {
                PdfObject::Value(value)
                    if matches!(options.object_compression, PdfObjectCompression::None) =>
                {
                    write_value(pdf.indirect(reference), value)?
                }
                PdfObject::Value(_) => {}
                PdfObject::Raw(data)
                    if matches!(options.object_compression, PdfObjectCompression::None) =>
                {
                    pdf.indirect(reference).primitive(Raw(data));
                }
                PdfObject::Raw(_) => {}
                PdfObject::Annotation(annotation) => {
                    write_annotation(&mut pdf, reference, annotation)?;
                }
                PdfObject::Stream { dictionary, data } => write_stream(
                    &mut pdf,
                    reference,
                    dictionary,
                    data,
                    options.stream_compression,
                )?,
                PdfObject::FormXObject {
                    dictionary,
                    data,
                    bbox,
                    matrix,
                } => write_form_xobject(
                    &mut pdf,
                    reference,
                    dictionary,
                    data,
                    *bbox,
                    *matrix,
                    options.stream_compression,
                )?,
                PdfObject::ImageXObject { image, data } => {
                    write_image_xobject(&mut pdf, reference, *image, data)?
                }
            }
        }

        match options.object_compression {
            PdfObjectCompression::None => Ok(pdf.finish()),
            PdfObjectCompression::ObjectStreams { .. } => {
                let (object_stream_id, xref_id) = auxiliary_refs(self)?;
                let mut object_stream = pdf.object_stream(object_stream_id);
                for indirect in self.objects() {
                    if indirect.id == self.catalog() || self.trailer().info == Some(indirect.id) {
                        continue;
                    }
                    if let PdfObject::Value(PdfValue::Dictionary(dictionary)) = &indirect.object
                        && matches!(dictionary.get(b"Type"), Some(PdfValue::Name(name)) if name.as_bytes() == b"Page")
                    {
                        continue;
                    }
                    match &indirect.object {
                        PdfObject::Value(value) => {
                            write_value(object_stream.object(writer_ref(indirect.id)?), value)?;
                        }
                        PdfObject::Raw(data) => {
                            object_stream
                                .object(writer_ref(indirect.id)?)
                                .primitive(Raw(data));
                        }
                        PdfObject::Stream { .. }
                        | PdfObject::FormXObject { .. }
                        | PdfObject::ImageXObject { .. }
                        | PdfObject::Annotation(_) => {}
                    }
                }
                match options.stream_compression {
                    PdfStreamCompression::None => object_stream.finish(),
                    PdfStreamCompression::Flate { level } => {
                        object_stream.finish_with_filter(
                            Filter::FlateDecode,
                            |data| match deflate(data, level) {
                                Ok(bytes) => bytes,
                                Err(_) => unreachable!("writing zlib data to Vec cannot fail"),
                            },
                        );
                    }
                }
                Ok(match options.stream_compression {
                    PdfStreamCompression::None => pdf.finish_with_xref_stream(xref_id),
                    PdfStreamCompression::Flate { level } => pdf
                        .finish_with_xref_stream_and_filter(xref_id, |data| {
                            (
                                match deflate(data, level) {
                                    Ok(bytes) => bytes,
                                    Err(_) => {
                                        unreachable!("writing zlib data to Vec cannot fail")
                                    }
                                },
                                XRefFilter::Single(Filter::FlateDecode),
                            )
                        }),
                })
            }
        }
    }
}

fn write_image_xobject(
    pdf: &mut Pdf,
    reference: Ref,
    image: PdfImageXObject,
    data: &[u8],
) -> Result<(), PdfSerializeError> {
    let width = i32::try_from(image.width)
        .map_err(|_| PdfSerializeError::IntegerOutOfRange(i64::from(image.width)))?;
    let height = i32::try_from(image.height)
        .map_err(|_| PdfSerializeError::IntegerOutOfRange(i64::from(image.height)))?;
    let mut writer = pdf.image_xobject(reference, data);
    writer
        .width(width)
        .height(height)
        .bits_per_component(i32::from(image.bits_per_component))
        .color_space_name(Name(match image.color_space {
            PdfImageColorSpace::DeviceGray => b"DeviceGray",
            PdfImageColorSpace::DeviceRgb => b"DeviceRGB",
            PdfImageColorSpace::DeviceCmyk => b"DeviceCMYK",
        }));
    match image.filter {
        PdfImageFilter::Dct => {
            writer.filter(Filter::DctDecode);
        }
        PdfImageFilter::Flate => {
            writer.filter(Filter::FlateDecode);
        }
        PdfImageFilter::FlatePngPredictor { colors } => {
            writer.filter(Filter::FlateDecode);
            writer
                .decode_parms()
                .predictor(Predictor::PngOptimum)
                .colors(i32::from(colors))
                .bits_per_component(i32::from(image.bits_per_component))
                .columns(width);
        }
    }
    if let Some(mask) = image.soft_mask {
        writer.s_mask(writer_ref(mask)?);
    }
    writer.finish();
    Ok(())
}

fn validate_serialization_inputs(
    document: &PdfDocument,
    options: PdfSerializationOptions,
) -> Result<(), PdfSerializeError> {
    if let PdfStreamCompression::Flate { level } = options.stream_compression
        && level > 9
    {
        return Err(PdfSerializeError::InvalidCompressionLevel(level));
    }
    if let PdfObjectCompression::ObjectStreams { level } = options.object_compression
        && !(1..=3).contains(&level)
    {
        return Err(PdfSerializeError::InvalidObjectCompressionLevel(level));
    }
    if matches!(
        options.object_compression,
        PdfObjectCompression::ObjectStreams { .. }
    ) && document.version().major() == 1
        && document.version().minor() < 5
    {
        return Err(PdfSerializeError::ObjectStreamsRequirePdf15);
    }
    for indirect in document.objects() {
        writer_ref(indirect.id)?;
        validate_object_scalars(&indirect.object)?;
        if matches!(
            options.stream_compression,
            PdfStreamCompression::Flate { .. }
        ) && let PdfObject::Stream { dictionary, .. } = &indirect.object
            && (dictionary.get(b"Filter").is_some() || dictionary.get(b"DecodeParms").is_some())
        {
            return Err(PdfSerializeError::CompressionFilterConflict(indirect.id));
        }
    }
    Ok(())
}

fn auxiliary_refs(document: &PdfDocument) -> Result<(Ref, Ref), PdfSerializeError> {
    let last = document
        .objects()
        .last()
        .map_or(0, |object| object.id.get());
    let object_stream = last
        .checked_add(1)
        .and_then(PdfObjectId::new)
        .ok_or(PdfSerializeError::ObjectIdSpaceExhausted)?;
    let xref = last
        .checked_add(2)
        .and_then(PdfObjectId::new)
        .ok_or(PdfSerializeError::ObjectIdSpaceExhausted)?;
    Ok((writer_ref(object_stream)?, writer_ref(xref)?))
}

fn validate_object_scalars(object: &PdfObject) -> Result<(), PdfSerializeError> {
    let mut stack = Vec::new();
    match object {
        PdfObject::Value(value) => stack.push(value),
        PdfObject::Stream { dictionary, .. } | PdfObject::FormXObject { dictionary, .. } => {
            stack.extend(dictionary.iter().map(|(_, value)| value));
        }
        PdfObject::Raw(_) => {}
        PdfObject::Annotation(_) => {}
        PdfObject::ImageXObject { .. } => {}
    }
    while let Some(value) = stack.pop() {
        match value {
            PdfValue::Integer(value) => {
                i32::try_from(*value).map_err(|_| PdfSerializeError::IntegerOutOfRange(*value))?;
            }
            PdfValue::Reference(id) => {
                writer_ref(*id)?;
            }
            PdfValue::Array(values) => stack.extend(values),
            PdfValue::Dictionary(dictionary) => {
                stack.extend(dictionary.iter().map(|(_, value)| value));
            }
            _ => {}
        }
    }
    Ok(())
}

fn write_form_xobject(
    pdf: &mut Pdf,
    reference: Ref,
    dictionary: &PdfDictionary,
    data: &[u8],
    bbox: [PdfNumber; 4],
    matrix: [PdfNumber; 6],
    compression: PdfStreamCompression,
) -> Result<(), PdfSerializeError> {
    let bbox = Rect::new(
        number_as_f32(bbox[0]),
        number_as_f32(bbox[1]),
        number_as_f32(bbox[2]),
        number_as_f32(bbox[3]),
    );
    let matrix = matrix.map(number_as_f32);
    match compression {
        PdfStreamCompression::None => {
            let mut form = pdf.form_xobject(reference, data);
            form.bbox(bbox).matrix(matrix);
            write_form_entries(&mut form, dictionary)?;
            form.finish();
        }
        PdfStreamCompression::Flate { level } => {
            let compressed = deflate(data, level)?;
            let mut form = pdf.form_xobject(reference, &compressed);
            form.bbox(bbox).matrix(matrix).filter(Filter::FlateDecode);
            write_form_entries(&mut form, dictionary)?;
            form.finish();
        }
    }
    Ok(())
}

fn write_form_entries(
    form: &mut pdf_writer::writers::FormXObject<'_>,
    dictionary: &PdfDictionary,
) -> Result<(), PdfSerializeError> {
    if let Some(PdfValue::Dictionary(resources)) = dictionary.get(b"Resources") {
        let mut writer = form.resources();
        write_dictionary_entries(&mut writer, resources, None)?;
        writer.finish();
    } else if let Some(PdfValue::Reference(resources)) = dictionary.get(b"Resources") {
        form.resources_ref(writer_ref(*resources)?);
    }
    if let Some(PdfValue::Reference(group)) = dictionary.get(b"Group") {
        form.group_ref(writer_ref(*group)?);
    }
    write_dictionary_entries_skipping(form, dictionary, &[b"Resources", b"Group"])
}

fn writer_ref(id: PdfObjectId) -> Result<Ref, PdfSerializeError> {
    let raw = i32::try_from(id.get()).map_err(|_| PdfSerializeError::ObjectIdOutOfRange(id))?;
    Ok(Ref::new(raw))
}

fn write_stream(
    pdf: &mut Pdf,
    reference: Ref,
    dictionary: &PdfDictionary,
    data: &[u8],
    compression: PdfStreamCompression,
) -> Result<(), PdfSerializeError> {
    match compression {
        PdfStreamCompression::None => {
            let mut stream = pdf.stream(reference, data);
            write_dictionary_entries(&mut stream, dictionary, None)?;
            stream.finish();
        }
        PdfStreamCompression::Flate { level } => {
            let compressed = deflate(data, level)?;
            let mut stream = pdf.stream(reference, &compressed);
            stream.filter(Filter::FlateDecode);
            write_dictionary_entries(&mut stream, dictionary, None)?;
            stream.finish();
        }
    }
    Ok(())
}

fn deflate(data: &[u8], level: u8) -> Result<Vec<u8>, PdfSerializeError> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(u32::from(level)));
    encoder
        .write_all(data)
        .map_err(|error| PdfSerializeError::Compression(error.kind()))?;
    encoder
        .finish()
        .map_err(|error| PdfSerializeError::Compression(error.kind()))
}

fn write_annotation(
    pdf: &mut Pdf,
    reference: Ref,
    annotation: &PdfAnnotationObject,
) -> Result<(), PdfSerializeError> {
    let mut writer = pdf.annotation(reference);
    writer.rect(Rect::new(
        number_as_f32(annotation.rect[0]),
        number_as_f32(annotation.rect[1]),
        number_as_f32(annotation.rect[2]),
        number_as_f32(annotation.rect[3]),
    ));
    if let Some(kind) = annotation.subtype {
        writer.subtype(match kind {
            PdfAnnotationType::Link => AnnotationType::Link,
        });
    }
    writer.raw_entries(&annotation.raw_entries);
    match &annotation.action {
        Some(PdfAnnotationAction::UserEntries(entries)) => {
            writer.raw_entries(entries);
        }
        Some(PdfAnnotationAction::Destination(spec)) => {
            let external = spec.file.is_some();
            let mut action = writer.action();
            action.action_type(match (spec.kind, external) {
                (PdfDestinationActionKind::GoTo, false) => ActionType::GoTo,
                (PdfDestinationActionKind::GoTo, true) => ActionType::RemoteGoTo,
                (PdfDestinationActionKind::Thread, _) => ActionType::Thread,
            });
            if let Some(file) = &spec.file {
                action.file_spec().path(Str(file));
            }
            if let Some(new_window) = spec.new_window {
                action.new_window(new_window);
            }
            match &spec.target {
                PdfDestinationTarget::Page { page, view } => match page {
                    PdfDestinationPage::Internal(id) => {
                        action.destination().page(writer_ref(*id)?).raw_view(view)
                    }
                    PdfDestinationPage::External(number) => action
                        .destination()
                        .page_number(i32::try_from(*number).map_err(|_| {
                            PdfSerializeError::IntegerOutOfRange(i64::from(*number))
                        })?)
                        .raw_view(view),
                },
                PdfDestinationTarget::Name(name) => {
                    action.destination_string(Str(name));
                }
                PdfDestinationTarget::Number(number) => {
                    action.destination_number(
                        i32::try_from(*number).map_err(|_| {
                            PdfSerializeError::IntegerOutOfRange(i64::from(*number))
                        })?,
                    );
                }
            }
            match &spec.structure {
                Some(PdfDestinationStructure::Internal(id)) => {
                    action.structure_destination(writer_ref(*id)?);
                }
                Some(PdfDestinationStructure::External(value)) => {
                    action.structure_destination_raw(value);
                }
                None => {}
            }
            action.finish();
        }
        None => {}
    }
    writer.finish();
    Ok(())
}

fn write_value(object: Obj<'_>, value: &PdfValue) -> Result<(), PdfSerializeError> {
    match value {
        PdfValue::Null => object.primitive(Null),
        PdfValue::Bool(value) => object.primitive(*value),
        PdfValue::Integer(value) => object.primitive(
            i32::try_from(*value).map_err(|_| PdfSerializeError::IntegerOutOfRange(*value))?,
        ),
        PdfValue::Number(value) => object.primitive(number_as_f32(*value)),
        PdfValue::Name(name) => object.primitive(Name(name.as_bytes())),
        PdfValue::String(value) => object.primitive(Str(value)),
        PdfValue::Reference(id) => object.primitive(writer_ref(*id)?),
        PdfValue::Array(values) => {
            let mut array = object.array();
            for value in values {
                write_value(array.push(), value)?;
            }
            array.finish();
        }
        PdfValue::Dictionary(dictionary) => {
            let mut dictionary_writer = object.dict();
            write_dictionary_entries(&mut dictionary_writer, dictionary, None)?;
            dictionary_writer.finish();
        }
    }
    Ok(())
}

fn write_dictionary_entries(
    writer: &mut Dict<'_>,
    dictionary: &PdfDictionary,
    skip: Option<&[u8]>,
) -> Result<(), PdfSerializeError> {
    for (key, value) in dictionary.iter() {
        if skip == Some(key.as_bytes()) {
            continue;
        }
        write_value(writer.insert(Name(key.as_bytes())), value)?;
    }
    writer.raw_entries(dictionary.raw_entries());
    Ok(())
}

fn write_dictionary_entries_skipping(
    writer: &mut Dict<'_>,
    dictionary: &PdfDictionary,
    skip: &[&[u8]],
) -> Result<(), PdfSerializeError> {
    for (key, value) in dictionary.iter() {
        if skip.contains(&key.as_bytes()) {
            continue;
        }
        write_value(writer.insert(Name(key.as_bytes())), value)?;
    }
    writer.raw_entries(dictionary.raw_entries());
    Ok(())
}

fn number_as_f32(number: PdfNumber) -> f32 {
    let divisor = 10_f32.powi(i32::from(number.decimal_places()));
    number.coefficient() as f32 / divisor
}

#[cfg(test)]
mod tests;

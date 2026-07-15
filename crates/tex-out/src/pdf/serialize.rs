//! Canonical adapter from Umber's validated graph to `pdf_writer`.

use std::io::Write as _;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use pdf_writer::{Dict, Filter, Finish, Name, Null, Obj, Pdf, Ref, Settings, Str};

use super::{PdfDictionary, PdfDocument, PdfNumber, PdfObject, PdfObjectId, PdfValue};

/// Deterministic stream encoding selected at final serialization.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfStreamCompression {
    /// Preserve the validated stream bytes without adding a filter.
    #[default]
    None,
    /// Compress every unfiltered stream with deterministic zlib/DEFLATE.
    Flate { level: u8 },
}

/// Byte-format policy applied without changing document semantic identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PdfSerializationOptions {
    /// Ask `pdf_writer` for human-readable whitespace and indentation.
    pub pretty: bool,
    /// Encoding policy for stream payloads.
    pub stream_compression: PdfStreamCompression,
}

/// Typed failure raised before any private output buffer is returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfSerializeError {
    ObjectIdOutOfRange(PdfObjectId),
    IntegerOutOfRange(i64),
    InvalidCompressionLevel(u8),
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

            match &indirect.object {
                PdfObject::Value(value) => write_value(pdf.indirect(reference), value)?,
                PdfObject::Stream { dictionary, data } => write_stream(
                    &mut pdf,
                    reference,
                    dictionary,
                    data,
                    options.stream_compression,
                )?,
            }
        }

        Ok(pdf.finish())
    }
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

fn validate_object_scalars(object: &PdfObject) -> Result<(), PdfSerializeError> {
    let mut stack = Vec::new();
    match object {
        PdfObject::Value(value) => stack.push(value),
        PdfObject::Stream { dictionary, .. } => {
            stack.extend(dictionary.iter().map(|(_, value)| value));
        }
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
    Ok(())
}

fn number_as_f32(number: PdfNumber) -> f32 {
    let divisor = 10_f32.powi(i32::from(number.decimal_places()));
    number.coefficient() as f32 / divisor
}

#[cfg(test)]
mod tests;

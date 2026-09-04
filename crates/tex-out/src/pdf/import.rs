use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    PdfDictionary, PdfIndirectObject, PdfName, PdfNumber, PdfObject, PdfObjectId, PdfValue,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::{Array, Dict, FromBytes, MaybeRef, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};
use hayro_syntax::reader::{Reader, ReaderExt};

#[cfg(test)]
mod tests;

pub(crate) struct ImportedPdfPage {
    pub(crate) data: Vec<u8>,
    pub(crate) resources: PdfDictionary,
    pub(crate) dependencies: Vec<PdfIndirectObject>,
    pub(crate) group: Option<PdfObjectId>,
}

pub(crate) fn import_pdf_page(
    bytes: tex_content::SharedBytes,
    page_number: u32,
    next_object: &mut u32,
    limits: super::PdfFinalizationLimits,
) -> Result<ImportedPdfPage, String> {
    let pdf = load_pdf(bytes.clone())?;
    let page = selected_page(&pdf, page_number)?;
    let data = match page.page_stream() {
        Some(data) => {
            let mut data = data.to_vec();
            data.push(b'\n');
            data
        }
        None if page.raw().contains_key(b"Contents") => {
            return Err("PDF page content stream could not be decoded".to_owned());
        }
        None => Vec::new(),
    };
    let mut importer = Importer {
        xref: page.xref(),
        source_bytes: bytes.as_ref(),
        next_object,
        imported: BTreeMap::new(),
        objects: Vec::new(),
        values: 0,
        stream_bytes: 0,
        limits,
    };
    let resources = importer.import_resources(page)?;
    let group = raw_dictionary_value(page.raw().data(), b"Group")?
        .map(|value| importer.import_group(value))
        .transpose()?;
    Ok(ImportedPdfPage {
        data,
        resources,
        dependencies: importer.objects,
        group,
    })
}

fn load_pdf(bytes: tex_content::SharedBytes) -> Result<Pdf, String> {
    Pdf::new(Arc::new(bytes)).map_err(|error| format!("{error:?}"))
}

fn selected_page(pdf: &Pdf, page_number: u32) -> Result<&Page<'_>, String> {
    let index = page_number
        .checked_sub(1)
        .and_then(|page| usize::try_from(page).ok())
        .ok_or_else(|| format!("page {page_number} does not exist"))?;
    pdf.pages()
        .get(index)
        .ok_or_else(|| format!("page {page_number} does not exist"))
}

struct Importer<'a, 'next> {
    xref: &'a hayro_syntax::xref::XRef,
    source_bytes: &'a [u8],
    next_object: &'next mut u32,
    imported: BTreeMap<ObjectIdentifier, PdfObjectId>,
    objects: Vec<PdfIndirectObject>,
    values: usize,
    stream_bytes: usize,
    limits: super::PdfFinalizationLimits,
}

struct RawDictionaryEntry<'a> {
    name: Vec<u8>,
    value: &'a [u8],
}

impl<'a> Importer<'a, '_> {
    fn import_resources(&mut self, page: &Page<'a>) -> Result<PdfDictionary, String> {
        let resources = page.resources();
        let mut output = PdfDictionary::new();
        self.import_resource_category(&mut output, b"ExtGState", resources, |value| {
            &value.ext_g_states
        })?;
        self.import_resource_category(&mut output, b"Font", resources, |value| &value.fonts)?;
        self.import_resource_category(&mut output, b"ColorSpace", resources, |value| {
            &value.color_spaces
        })?;
        self.import_resource_category(&mut output, b"XObject", resources, |value| {
            &value.x_objects
        })?;
        self.import_resource_category(&mut output, b"Pattern", resources, |value| &value.patterns)?;
        self.import_resource_category(&mut output, b"Shading", resources, |value| &value.shadings)?;
        self.import_resource_category(&mut output, b"Properties", resources, |value| {
            &value.properties
        })?;
        if let Some(resources) = nearest_resource_dictionary(page)
            && let Some(proc_set) = raw_dictionary_value(resources.data(), b"ProcSet")?
        {
            output
                .insert("ProcSet", self.convert_raw_maybe_ref(proc_set)?)
                .map_err(|error| error.to_string())?;
        }
        Ok(output)
    }

    fn import_resource_category<F>(
        &mut self,
        output: &mut PdfDictionary,
        category: &'static [u8],
        resources: &Resources<'a>,
        select: F,
    ) -> Result<(), String>
    where
        F: Copy + for<'r> Fn(&'r Resources<'a>) -> &'r Dict<'a>,
    {
        let mut entries = PdfDictionary::new();
        let mut seen = BTreeSet::<Vec<u8>>::new();
        let mut level = Some(resources);
        while let Some(current) = level {
            let selected = select(current);
            if selected.is_empty() {
                level = current.parent();
                continue;
            }
            for entry in raw_dictionary_entries(selected.data())? {
                if seen.insert(entry.name.clone()) {
                    entries
                        .insert(
                            PdfName::new(entry.name),
                            self.convert_raw_maybe_ref(entry.value)?,
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            level = current.parent();
        }
        if !entries.is_empty() {
            output
                .insert(PdfName::new(category), PdfValue::Dictionary(entries))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn import_group(&mut self, source: &'a [u8]) -> Result<PdfObjectId, String> {
        match self.convert_raw_maybe_ref(source)? {
            PdfValue::Reference(id) => Ok(id),
            PdfValue::Dictionary(dictionary) => {
                let id = self.allocate_object()?;
                self.objects.push(PdfIndirectObject {
                    id,
                    object: PdfObject::Value(PdfValue::Dictionary(dictionary)),
                });
                Ok(id)
            }
            _ => Err("page Group is not a dictionary".to_owned()),
        }
    }

    fn convert_raw_maybe_ref(&mut self, source: &'a [u8]) -> Result<PdfValue, String> {
        self.convert_raw_maybe_ref_at(source, 0)
    }

    fn convert_raw_maybe_ref_at(
        &mut self,
        source: &'a [u8],
        depth: usize,
    ) -> Result<PdfValue, String> {
        self.check_value(depth)?;
        match MaybeRef::<Object<'_>>::from_bytes(source) {
            None => Err("invalid PDF resource value".to_owned()),
            Some(MaybeRef::Ref(reference)) => {
                Ok(PdfValue::Reference(self.import_indirect(reference.into())?))
            }
            Some(MaybeRef::NotRef(Object::Number(_))) => number_value(source),
            Some(MaybeRef::NotRef(value)) => self.convert_parsed_value(value, depth),
        }
    }

    fn convert_value(&mut self, source: Object<'a>) -> Result<PdfValue, String> {
        self.check_value(0)?;
        self.convert_parsed_value(source, 0)
    }

    fn convert_parsed_value(
        &mut self,
        source: Object<'a>,
        depth: usize,
    ) -> Result<PdfValue, String> {
        Ok(match source {
            Object::Null(_) => PdfValue::Null,
            Object::Boolean(value) => PdfValue::Bool(value),
            Object::Number(_) => {
                return Err("PDF resource number source bytes are unavailable".to_owned());
            }
            Object::String(value) => PdfValue::String(value.as_bytes().to_vec()),
            Object::Name(value) => PdfValue::Name(PdfName::new(value.as_ref())),
            Object::Array(values) => PdfValue::Array(self.convert_array(&values, depth + 1)?),
            Object::Dict(dictionary) => {
                PdfValue::Dictionary(self.convert_dictionary_at(&dictionary, depth + 1)?)
            }
            Object::Stream(_) => {
                return Err("direct resource streams are unsupported".to_owned());
            }
        })
    }

    fn convert_array(&mut self, source: &Array<'a>, depth: usize) -> Result<Vec<PdfValue>, String> {
        raw_array_values(source.data())?
            .into_iter()
            .map(|value| self.convert_raw_maybe_ref_at(value, depth))
            .collect()
    }

    fn check_value(&mut self, depth: usize) -> Result<(), String> {
        if depth > self.limits.max_imported_resource_depth {
            return Err(format!(
                "PDF resource nesting exceeds limit {}",
                self.limits.max_imported_resource_depth
            ));
        }
        self.values = self
            .values
            .checked_add(1)
            .ok_or_else(|| "PDF resource value capacity exhausted".to_owned())?;
        if self.values > self.limits.max_imported_resource_values {
            return Err(format!(
                "PDF resource values exceed limit {}",
                self.limits.max_imported_resource_values
            ));
        }
        Ok(())
    }

    fn convert_dictionary_at(
        &mut self,
        source: &Dict<'a>,
        depth: usize,
    ) -> Result<PdfDictionary, String> {
        self.convert_dictionary_skipping_at(source, &[], depth)
    }

    fn convert_dictionary_skipping(
        &mut self,
        source: &Dict<'a>,
        skipped: &[&[u8]],
    ) -> Result<PdfDictionary, String> {
        self.convert_dictionary_skipping_at(source, skipped, 0)
    }

    fn convert_dictionary_skipping_at(
        &mut self,
        source: &Dict<'a>,
        skipped: &[&[u8]],
        depth: usize,
    ) -> Result<PdfDictionary, String> {
        let mut dictionary = PdfDictionary::new();
        for entry in raw_dictionary_entries(source.data())? {
            if skipped.contains(&entry.name.as_slice()) {
                continue;
            }
            dictionary
                .insert(
                    PdfName::new(entry.name),
                    self.convert_raw_maybe_ref_at(entry.value, depth)?,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(dictionary)
    }

    fn import_indirect(&mut self, source_id: ObjectIdentifier) -> Result<PdfObjectId, String> {
        if let Some(id) = self.imported.get(&source_id) {
            return Ok(*id);
        }
        if self.imported.len() >= self.limits.max_imported_resource_objects {
            return Err(format!(
                "PDF resource objects exceed limit {}",
                self.limits.max_imported_resource_objects
            ));
        }
        let id = self.allocate_object()?;
        self.imported.insert(source_id, id);
        let source = self
            .xref
            .get::<Object<'_>>(source_id)
            .ok_or_else(|| format!("referenced PDF object {source_id:?} is missing"))?;
        let object = match source {
            Object::Stream(stream) => self.import_stream(stream)?,
            value => PdfObject::Value(
                match find_raw_indirect_object(self.source_bytes, source_id) {
                    Some(raw) => self.convert_raw_maybe_ref(raw)?,
                    None => self.convert_value(value)?,
                },
            ),
        };
        self.objects.push(PdfIndirectObject { id, object });
        Ok(id)
    }

    fn import_stream(&mut self, stream: Stream<'a>) -> Result<PdfObject, String> {
        let raw_data = stream.raw_data();
        self.stream_bytes = self
            .stream_bytes
            .checked_add(raw_data.len())
            .ok_or_else(|| "PDF resource stream capacity exhausted".to_owned())?;
        if self.stream_bytes > self.limits.max_imported_resource_stream_bytes {
            return Err(format!(
                "PDF resource streams exceed limit {} bytes",
                self.limits.max_imported_resource_stream_bytes
            ));
        }
        Ok(PdfObject::EncodedStream {
            dictionary: self.convert_dictionary_skipping(stream.dict(), &[b"Length"])?,
            data: match raw_data {
                Cow::Borrowed(data) => data.to_vec(),
                Cow::Owned(data) => data,
            },
        })
    }

    fn allocate_object(&mut self) -> Result<PdfObjectId, String> {
        let raw = *self.next_object;
        let id = PdfObjectId::new(raw).ok_or_else(|| "PDF object capacity exhausted".to_owned())?;
        *self.next_object = raw
            .checked_add(1)
            .ok_or_else(|| "PDF object capacity exhausted".to_owned())?;
        Ok(id)
    }
}

fn nearest_resource_dictionary<'a>(page: &Page<'a>) -> Option<Dict<'a>> {
    let mut dictionary = page.raw().clone();
    loop {
        if let Some(resources) = dictionary.get::<Dict<'_>>(b"Resources") {
            return Some(resources);
        }
        let parent = dictionary.get_ref(b"Parent")?;
        dictionary = page.xref().get(parent.into())?;
    }
}

fn find_raw_indirect_object<'a>(data: &'a [u8], target: ObjectIdentifier) -> Option<&'a [u8]> {
    let mut reader = Reader::new(data);
    while !reader.at_end() {
        let start = reader.offset();
        if let Some(identifier) = reader.read_without_context::<ObjectIdentifier>() {
            if identifier == target {
                reader.skip_white_spaces_and_comments();
                if let Some(value) = reader.skip::<MaybeRef<Object<'a>>>(false) {
                    reader.skip_white_spaces_and_comments();
                    // Object identifiers can occur in strings or stream
                    // payloads when scanning without xref offsets. Accept a
                    // candidate only when the parsed value is followed by its
                    // object terminator; this keeps those bytes from becoming
                    // a resource dictionary.
                    if reader.forward_tag(b"endobj").is_some() {
                        return Some(value);
                    }
                }
            }
            reader.jump(start.saturating_add(1));
        } else {
            reader.forward();
        }
    }
    None
}

fn raw_dictionary_entries<'a>(data: &'a [u8]) -> Result<Vec<RawDictionaryEntry<'a>>, String> {
    let mut reader = Reader::new(data);
    reader.skip_white_spaces_and_comments();
    reader
        .forward_tag(b"<<")
        .ok_or_else(|| "invalid PDF resource dictionary".to_owned())?;
    let mut entries = Vec::<RawDictionaryEntry<'a>>::new();
    loop {
        reader.skip_white_spaces_and_comments();
        if reader.forward_tag(b">>").is_some() {
            break;
        }
        let raw_name = reader
            .skip::<hayro_syntax::object::Name<'a>>(false)
            .ok_or_else(|| "invalid PDF resource dictionary name".to_owned())?;
        let name = hayro_syntax::object::Name::from_bytes(raw_name)
            .ok_or_else(|| "invalid PDF resource dictionary name".to_owned())?;
        reader.skip_white_spaces_and_comments();
        let value = reader
            .skip::<MaybeRef<Object<'a>>>(false)
            .ok_or_else(|| "invalid PDF resource dictionary value".to_owned())?;
        let name = name.as_ref().to_vec();
        if let Some(previous) = entries.iter_mut().find(|entry| entry.name == name) {
            // hayro's dictionary index follows the last occurrence of a key;
            // retain that behavior while keeping this parser's value slice
            // tied to the original source bytes.
            previous.value = value;
        } else {
            entries.push(RawDictionaryEntry { name, value });
        }
    }
    Ok(entries)
}

fn raw_dictionary_value<'a>(data: &'a [u8], key: &[u8]) -> Result<Option<&'a [u8]>, String> {
    Ok(raw_dictionary_entries(data)?
        .into_iter()
        .find(|entry| entry.name == key)
        .map(|entry| entry.value))
}

fn raw_array_values<'a>(data: &'a [u8]) -> Result<Vec<&'a [u8]>, String> {
    let mut reader = Reader::new(data);
    let mut values = Vec::new();
    loop {
        reader.skip_white_spaces_and_comments();
        if reader.at_end() {
            break;
        }
        values.push(
            reader
                .skip::<MaybeRef<Object<'a>>>(false)
                .ok_or_else(|| "invalid PDF resource array value".to_owned())?,
        );
    }
    Ok(values)
}

fn number_value(source: &[u8]) -> Result<PdfValue, String> {
    let source = trim_pdf_whitespace(source);
    let mut index = 0;
    let negative = match source.first().copied() {
        Some(b'-') => {
            index = 1;
            true
        }
        Some(b'+') => {
            index = 1;
            false
        }
        _ => false,
    };
    let integer_start = index;
    while source.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    let integer_end = index;
    let mut fraction_start = index;
    let mut fraction_end = index;
    if source.get(index) == Some(&b'.') {
        index += 1;
        fraction_start = index;
        while source.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        fraction_end = index;
    }
    if index != source.len() || (integer_start == integer_end && fraction_start == fraction_end) {
        return Err("page resource contains an invalid number".to_owned());
    }
    let decimal_places = fraction_end - fraction_start;
    if decimal_places > 9 {
        return Err(format!(
            "page resource number precision exceeds limit 9: {decimal_places}"
        ));
    }
    let mut magnitude = 0_i128;
    for digit in source[integer_start..integer_end]
        .iter()
        .chain(source[fraction_start..fraction_end].iter())
    {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(i128::from(*digit - b'0')))
            .ok_or_else(|| "page resource number is out of range".to_owned())?;
    }
    let coefficient = if negative {
        magnitude
            .checked_neg()
            .ok_or_else(|| "page resource number is out of range".to_owned())?
    } else {
        magnitude
    };
    let coefficient = i64::try_from(coefficient)
        .map_err(|_| "page resource number is out of range".to_owned())?;
    canonical_imported_number(coefficient, decimal_places as u8).map(PdfValue::Number)
}

/// Applies pdfTeX 1.40.29's `pdftoepdf.cc::convertNumToPDF` policy (lines
/// 501-545, called by `copyObject` at lines 557-565) to an imported real
/// spelling. The supported PDF numeric range is defined by Appendix C.1.
///
/// The reference implementation adds `0.5E-6` to the non-negative magnitude
/// before taking six fractional digits. For an input with more than six
/// decimal places, the same operation is exact integer division: the divisor
/// is the number of discarded decimal units and its half is the epsilon. A
/// zero quotient also covers pdfTeX's strict `fabs(n) < epsilon` check, while
/// an exact half rounds away from zero. Inputs with at most six decimal places
/// are already on the output grid and need no arithmetic.
fn canonical_imported_number(coefficient: i64, decimal_places: u8) -> Result<PdfNumber, String> {
    if decimal_places <= 6 {
        return PdfNumber::new(coefficient, decimal_places).map_err(|error| error.to_string());
    }

    let discarded_places = u32::from(decimal_places - 6);
    let divisor = 10_i128.pow(discarded_places);
    let magnitude = i128::from(coefficient.unsigned_abs());
    let rounded = (magnitude + divisor / 2) / divisor;
    let coefficient = if coefficient < 0 {
        rounded
            .checked_neg()
            .ok_or_else(|| "page resource number is out of range".to_owned())?
    } else {
        rounded
    };
    let coefficient = i64::try_from(coefficient)
        .map_err(|_| "page resource number is out of range".to_owned())?;
    PdfNumber::new(coefficient, 6).map_err(|error| error.to_string())
}

fn trim_pdf_whitespace(source: &[u8]) -> &[u8] {
    let start = source
        .iter()
        .position(|byte| !matches!(*byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .unwrap_or(source.len());
    let end = source
        .iter()
        .rposition(|byte| !matches!(*byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .map_or(start, |index| index + 1);
    &source[start..end]
}

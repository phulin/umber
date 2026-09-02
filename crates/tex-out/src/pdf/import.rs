use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{
    PdfDictionary, PdfIndirectObject, PdfName, PdfNumber, PdfObject, PdfObjectId, PdfValue,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, MaybeRef, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};

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
    let pdf = load_pdf(bytes)?;
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
        next_object,
        imported: BTreeMap::new(),
        objects: Vec::new(),
        values: 0,
        stream_bytes: 0,
        limits,
    };
    let resources = importer.import_resources(page)?;
    let group = page
        .raw()
        .get_raw::<Object<'_>>(b"Group")
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
    next_object: &'next mut u32,
    imported: BTreeMap<ObjectIdentifier, PdfObjectId>,
    objects: Vec<PdfIndirectObject>,
    values: usize,
    stream_bytes: usize,
    limits: super::PdfFinalizationLimits,
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
            && let Some(proc_set) = resources.get_raw::<Object<'_>>(b"ProcSet")
        {
            output
                .insert("ProcSet", self.convert_maybe_ref(proc_set)?)
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
            for (name, value) in select(current).entries() {
                if seen.insert(name.as_ref().to_vec()) {
                    entries
                        .insert(PdfName::new(name.as_ref()), self.convert_maybe_ref(value)?)
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

    fn import_group(&mut self, source: MaybeRef<Object<'a>>) -> Result<PdfObjectId, String> {
        match self.convert_maybe_ref(source)? {
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

    fn convert_maybe_ref(&mut self, source: MaybeRef<Object<'a>>) -> Result<PdfValue, String> {
        self.convert_maybe_ref_at(source, 0)
    }

    fn convert_maybe_ref_at(
        &mut self,
        source: MaybeRef<Object<'a>>,
        depth: usize,
    ) -> Result<PdfValue, String> {
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
        match source {
            MaybeRef::Ref(reference) => {
                Ok(PdfValue::Reference(self.import_indirect(reference.into())?))
            }
            MaybeRef::NotRef(value) => self.convert_value_at(value, depth),
        }
    }

    fn convert_value(&mut self, source: Object<'a>) -> Result<PdfValue, String> {
        self.convert_value_at(source, 0)
    }

    fn convert_value_at(&mut self, source: Object<'a>, depth: usize) -> Result<PdfValue, String> {
        Ok(match source {
            Object::Null(_) => PdfValue::Null,
            Object::Boolean(value) => PdfValue::Bool(value),
            Object::Number(value) => number_value(value.as_f64())?,
            Object::String(value) => PdfValue::String(value.as_bytes().to_vec()),
            Object::Name(value) => PdfValue::Name(PdfName::new(value.as_ref())),
            Object::Array(values) => PdfValue::Array(
                values
                    .raw_iter()
                    .map(|value| self.convert_maybe_ref_at(value, depth + 1))
                    .collect::<Result<_, _>>()?,
            ),
            Object::Dict(dictionary) => {
                PdfValue::Dictionary(self.convert_dictionary_at(&dictionary, depth + 1)?)
            }
            Object::Stream(_) => {
                return Err("direct resource streams are unsupported".to_owned());
            }
        })
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
        for (name, value) in source.entries() {
            if skipped.contains(&name.as_ref()) {
                continue;
            }
            dictionary
                .insert(
                    PdfName::new(name.as_ref()),
                    self.convert_maybe_ref_at(value, depth)?,
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
            value => PdfObject::Value(self.convert_value(value)?),
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

fn number_value(value: f64) -> Result<PdfValue, String> {
    if !value.is_finite() {
        return Err("page resource contains a non-finite number".to_owned());
    }
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return Ok(PdfValue::Integer(value as i64));
    }
    let coefficient = (value * 1_000_000_000.0).round();
    if coefficient < i64::MIN as f64 || coefficient > i64::MAX as f64 {
        return Err("page resource number is out of range".to_owned());
    }
    PdfNumber::new(coefficient as i64, 9)
        .map(PdfValue::Number)
        .map_err(|error| error.to_string())
}

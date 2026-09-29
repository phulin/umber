//! One-pass positioned lowering of every page and form artifact.
//!
//! Lowering is a pure function of artifact bytes. A caller that needs the
//! positioned events before finalization, such as virtual-font use
//! collection, lowers once and hands the result to
//! [`super::finalize_pdf_positioned`], which admits it only when it was built
//! from the very artifacts the input carries.

use super::*;

/// Positioned events and PDF media extents lowered from each committed page
/// and form artifact, in input order.
#[derive(Debug)]
pub struct PdfPositionedArtifacts {
    pub(super) pages: Vec<PositionedPage>,
    pub(super) page_extents: Vec<(Scaled, Scaled)>,
    page_sources: Vec<tex_content::SharedBytes>,
    pub(super) forms: Vec<(u32, PositionedPage)>,
    form_sources: Vec<tex_content::SharedBytes>,
}

impl PdfPositionedArtifacts {
    /// Decodes and lowers each page and form artifact exactly once.
    pub fn lower(
        pages: &[super::super::PdfCommittedPageInput],
        forms: &BTreeMap<u32, super::super::PdfFormInput>,
    ) -> Result<Self, PdfBuildError> {
        let mut lowered = Self {
            pages: Vec::with_capacity(pages.len()),
            page_extents: Vec::with_capacity(pages.len()),
            page_sources: Vec::with_capacity(pages.len()),
            forms: Vec::with_capacity(forms.len()),
            form_sources: Vec::with_capacity(forms.len()),
        };
        for (page_index, record) in pages.iter().enumerate() {
            let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
            lowered
                .page_extents
                .push(pdf_page_extents(&artifact, record)?);
            lowered.pages.push(crate::positioned::lower_page(
                &artifact,
                u32::try_from(page_index).unwrap_or(u32::MAX),
            )?);
            lowered.page_sources.push(record.artifact_bytes.clone());
        }
        for form in forms.values() {
            let artifact = PageArtifact::from_bytes(&form.artifact_bytes)?;
            lowered
                .forms
                .push((form.object, crate::positioned::lower_page(&artifact, 0)?));
            lowered.form_sources.push(form.artifact_bytes.clone());
        }
        Ok(lowered)
    }

    /// Lowered pages, in input page order.
    pub fn pages(&self) -> &[PositionedPage] {
        &self.pages
    }

    /// Lowered forms, in input form-object order.
    pub fn forms(&self) -> impl Iterator<Item = &PositionedPage> {
        self.forms.iter().map(|(_, form)| form)
    }

    /// Whether this lowering was built from exactly `input`'s artifacts.
    pub(super) fn matches(&self, input: &PdfFinalizationInput) -> bool {
        let same = |left: &tex_content::SharedBytes, right: &tex_content::SharedBytes| {
            tex_content::SharedBytes::ptr_eq(left, right) || left == right
        };
        self.page_sources.len() == input.pages.len()
            && self.form_sources.len() == input.forms.len()
            && self
                .page_sources
                .iter()
                .zip(&input.pages)
                .all(|(source, page)| same(source, &page.artifact_bytes))
            && self
                .forms
                .iter()
                .zip(&self.form_sources)
                .zip(input.forms.values())
                .all(|(((object, _), source), form)| {
                    *object == form.object && same(source, &form.artifact_bytes)
                })
    }
}

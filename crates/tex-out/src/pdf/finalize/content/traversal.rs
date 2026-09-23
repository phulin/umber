//! Positioned traversal and page/form resource classification.

use super::*;

#[derive(Clone, Copy)]
pub(in crate::pdf::finalize) struct PdfFormTraversalLimits {
    pub(in crate::pdf::finalize) max_depth: usize,
    pub(in crate::pdf::finalize) max_work: usize,
}

/// Page/form-local resource classes from pdftex.web sections 766--768.
///
/// pdfTeX derives these compatibility names from the generated font and image
/// resource lists, not from the content operator stream. In particular,
/// ordinary graphics and nested forms do not contribute a ProcSet class.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::pdf::finalize) struct PdfProcSetUsage {
    text: bool,
    image_b: bool,
    image_c: bool,
    image_i: bool,
}

impl PdfProcSetUsage {
    pub(in crate::pdf::finalize) fn include_text(&mut self, used: bool) {
        self.text |= used;
    }

    pub(in crate::pdf::finalize) fn include_image(&mut self, metadata: PdfImageMetadataInput) {
        let PdfImageMetadataInput::Raster {
            format,
            color_space,
            png_color_type,
            ..
        } = metadata
        else {
            // writeimg.c leaves imported PDF pages at the zero color mask.
            return;
        };
        // writepng.c classifies palette images as both color and indexed,
        // independently of the decoded palette's eventual device space.
        if format == PdfRasterFormatInput::Png && png_color_type == Some(3) {
            self.image_c = true;
            self.image_i = true;
            return;
        }
        match color_space {
            PdfRasterColorSpaceInput::Gray => self.image_b = true,
            PdfRasterColorSpaceInput::Rgb | PdfRasterColorSpaceInput::Cmyk => {
                self.image_c = true;
            }
        }
    }

    pub(in crate::pdf::finalize) fn into_pdf_array(self) -> PdfValue {
        let mut names = vec![PdfValue::Name("PDF".into())];
        if self.text {
            names.push(PdfValue::Name("Text".into()));
        }
        if self.image_b {
            names.push(PdfValue::Name("ImageB".into()));
        }
        if self.image_c {
            names.push(PdfValue::Name("ImageC".into()));
        }
        if self.image_i {
            names.push(PdfValue::Name("ImageI".into()));
        }
        PdfValue::Array(names)
    }
}

pub(in crate::pdf::finalize) fn parse_pdf_matrix(
    payload: &[u8],
) -> Result<[f32; 4], PdfBuildError> {
    let text =
        std::str::from_utf8(payload).map_err(|_| PdfBuildError::InvalidMatrix(payload.to_vec()))?;
    let mut values = text.split_ascii_whitespace();
    let mut matrix = [0.0; 4];
    for value in &mut matrix {
        *value = values
            .next()
            .and_then(|word| word.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| PdfBuildError::InvalidMatrix(payload.to_vec()))?;
    }
    if values.next().is_some() {
        return Err(PdfBuildError::InvalidMatrix(payload.to_vec()));
    }
    Ok(matrix)
}

pub(in crate::pdf::finalize) fn validate_form_graph(
    input: &PdfFinalizationInput,
    pages: &[PositionedPage],
    forms: &BTreeMap<u32, PositionedPage>,
    limits: PdfFormTraversalLimits,
) -> Result<(), PdfBuildError> {
    fn references(page: &PositionedPage) -> impl Iterator<Item = u32> + '_ {
        page.events.iter().filter_map(|event| match event {
            PositionedEvent::PdfGraphics(graphics) => match graphics.effect {
                crate::PageEffect::PdfRefXForm { object, .. } => Some(object),
                _ => None,
            },
            _ => None,
        })
    }

    struct Traversal<'a> {
        input: &'a PdfFinalizationInput,
        forms: &'a BTreeMap<u32, PositionedPage>,
        limits: PdfFormTraversalLimits,
        work: usize,
        active: BTreeSet<u32>,
        complete: BTreeSet<u32>,
    }

    impl Traversal<'_> {
        fn visit(&mut self, object: u32, depth: usize) -> Result<(), PdfBuildError> {
            self.work =
                self.work
                    .checked_add(1)
                    .ok_or(PdfBuildError::FormTraversalWorkExceeded(
                        self.limits.max_work,
                    ))?;
            if self.work > self.limits.max_work {
                return Err(PdfBuildError::FormTraversalWorkExceeded(
                    self.limits.max_work,
                ));
            }
            if depth > self.limits.max_depth {
                return Err(PdfBuildError::FormTraversalDepthExceeded(
                    self.limits.max_depth,
                ));
            }
            if self.active.contains(&object) {
                return Err(PdfBuildError::FormCycle(object));
            }
            if self.complete.contains(&object) {
                return Ok(());
            }
            self.input
                .forms
                .get(&object)
                .ok_or(PdfBuildError::ReferencedFormNotFound(object))?;
            let page = self
                .forms
                .get(&object)
                .ok_or(PdfBuildError::MissingFormArtifact(object))?;
            let nested = references(page).collect::<Vec<_>>();
            self.active.insert(object);
            for nested in nested {
                if nested == object {
                    return Err(PdfBuildError::RecursiveForm(object));
                }
                self.visit(nested, depth + 1)?;
            }
            self.active.remove(&object);
            self.complete.insert(object);
            Ok(())
        }
    }

    let roots = pages.iter().flat_map(references).collect::<BTreeSet<_>>();
    let mut traversal = Traversal {
        input,
        forms,
        limits,
        work: 0,
        active: BTreeSet::new(),
        complete: BTreeSet::new(),
    };
    for object in roots {
        traversal.visit(object, 1)?;
    }
    Ok(())
}

pub(in crate::pdf::finalize) fn collect_font_usage(
    input: &PdfFinalizationInput,
    positioned_pages: &[PositionedPage],
    positioned_forms: &BTreeMap<u32, PositionedPage>,
) -> Result<BTreeMap<u32, BTreeSet<u8>>, PdfBuildError> {
    let mut font_metadata = BTreeMap::new();
    for font in positioned_pages
        .iter()
        .chain(positioned_forms.values())
        .flat_map(|positioned| &positioned.fonts)
    {
        if font_metadata.contains_key(&font.semantic_identity) {
            continue;
        }
        let resource = input
            .fonts
            .get(&font.semantic_identity)
            .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
        font_metadata.insert(
            font.semantic_identity,
            (
                resource,
                resource.included_codes.clone(),
                font_has_explicit_space(resource),
            ),
        );
    }
    let mut usage = BTreeMap::<u32, BTreeSet<u8>>::new();
    let mut interword_space_enabled = false;
    for positioned in positioned_pages {
        let fonts = positioned
            .fonts
            .iter()
            .map(|font| (font.font_id, font))
            .collect::<BTreeMap<_, _>>();
        for event in &positioned.events {
            let PositionedEvent::TextRun(run) = event else {
                if let PositionedEvent::PdfAccessibility(control) = event {
                    match control.control {
                        crate::PdfAccessibilityEffect::InterwordSpaceOn => {
                            interword_space_enabled = true;
                        }
                        crate::PdfAccessibilityEffect::InterwordSpaceOff => {
                            interword_space_enabled = false;
                        }
                        crate::PdfAccessibilityEffect::FakeSpace => {}
                    }
                }
                continue;
            };
            let font = fonts
                .get(&run.font_id)
                .copied()
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            let (resource, included, has_explicit_space) = font_metadata
                .get(&font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
            let codes = usage.entry(resource.object_number).or_default();
            let explicit_space = interword_space_enabled && *has_explicit_space;
            codes.extend(run.units.iter().zip(&run.physical_codes).filter_map(
                |(unit, physical_code)| match unit {
                    crate::positioned::TextUnit::Code(_) => *physical_code,
                    crate::positioned::TextUnit::Space if explicit_space => Some(b' '),
                    crate::positioned::TextUnit::Space => None,
                },
            ));
            codes.extend(included);
        }
    }
    for positioned in positioned_forms.values() {
        let fonts = positioned
            .fonts
            .iter()
            .map(|font| (font.font_id, font))
            .collect::<BTreeMap<_, _>>();
        for event in &positioned.events {
            let PositionedEvent::TextRun(run) = event else {
                continue;
            };
            let font = fonts
                .get(&run.font_id)
                .copied()
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            let (resource, included, _) = font_metadata
                .get(&font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingFontResource(font.name.clone()))?;
            let codes = usage.entry(resource.object_number).or_default();
            for unit in &run.units {
                let code = match unit {
                    crate::positioned::TextUnit::Code(code) => {
                        u8::try_from(*code).map_err(|_| {
                            PdfBuildError::PositionedCharacterOutOfRange {
                                font: font.name.clone(),
                                code: *code,
                            }
                        })?
                    }
                    crate::positioned::TextUnit::Space => b' ',
                };
                codes.insert(code);
            }
            codes.extend(included);
        }
    }
    Ok(usage)
}

pub(in crate::pdf::finalize) fn positioned_pages(
    input: &PdfFinalizationInput,
) -> Result<Vec<PositionedPage>, PdfBuildError> {
    input
        .pages
        .iter()
        .enumerate()
        .map(|(page_index, record)| {
            let artifact = PageArtifact::from_bytes(&record.artifact_bytes)?;
            Ok(crate::positioned::lower_page(&artifact, page_index as u32)?)
        })
        .collect()
}

pub(in crate::pdf::finalize) fn positioned_forms(
    input: &PdfFinalizationInput,
) -> Result<Vec<(u32, PositionedPage)>, PdfBuildError> {
    input
        .forms
        .values()
        .map(|form| {
            let artifact = PageArtifact::from_bytes(&form.artifact_bytes)?;
            Ok((form.object, crate::positioned::lower_page(&artifact, 0)?))
        })
        .collect()
}

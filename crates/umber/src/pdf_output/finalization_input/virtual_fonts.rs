//! Destination-local font identities needed while lowering virtual packets.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

use tex_arith::{FontSizeSpec, Scaled, tfm_fix_word_to_scaled};
use tex_fonts::{FontSourceIdentity, VfCommand};
use tex_out::pdf::{PdfFontInput, PdfFontMetricsInput, PdfFontProgramInput};
use tex_out::{FontResource, FontResourceConstruction};
use tex_state::{
    DetachedPdfCompletion, DetachedPdfFontOperation, FontArtifactConstructionRecipe,
    FontArtifactRecipe,
};

use super::{detached_encoding, detached_font_program, glyph_to_unicode_mappings};
use crate::PdfBuildError;

#[derive(Clone, Debug)]
struct LocalInstance {
    identity: FontSourceIdentity,
    name: String,
    size: Scaled,
    expansion_ratio: i16,
}

#[derive(Clone, Debug)]
struct PendingCharacter {
    font: LocalInstance,
    code: u8,
    depth: usize,
    font_watermark: Option<u32>,
}

pub(super) struct DestinationFontUse {
    pub(super) identity: FontSourceIdentity,
    pub(super) code: u8,
    pub(super) font_watermark: u32,
}

/// pdfTeX section 32e's single internal-font-number timeline.
///
/// Umber loads ordinary engine fonts before detached PDF lowering, while
/// pdfTeX can interleave VF-local loads with later `\font` definitions.  Keep
/// the engine order separately and let a later engine definition reuse an
/// identity that destination-time VF loading already installed.
struct FontNumberTimeline<'a> {
    engine_identities: &'a [FontSourceIdentity],
    numbers_by_identity: BTreeMap<FontSourceIdentity, u32>,
    engine_numbers: Vec<u32>,
    engine_watermark: u32,
    next_number: u32,
}

impl<'a> FontNumberTimeline<'a> {
    fn new(engine_identities: &'a [FontSourceIdentity]) -> Self {
        let nullfont = *engine_identities
            .first()
            .expect("the detached engine font timeline contains nullfont");
        Self {
            engine_identities,
            numbers_by_identity: BTreeMap::from([(nullfont, 0)]),
            engine_numbers: vec![0; engine_identities.len()],
            engine_watermark: 0,
            next_number: 1,
        }
    }

    fn advance_engine_to(&mut self, watermark: u32) -> Result<(), PdfBuildError> {
        for raw in self.engine_watermark.saturating_add(1)..=watermark {
            let identity = *self
                .engine_identities
                .get(raw as usize)
                .expect("page font watermark belongs to the detached engine timeline");
            let number = self.register(identity)?;
            self.engine_numbers[raw as usize] = number;
        }
        self.engine_watermark = self.engine_watermark.max(watermark);
        Ok(())
    }

    fn register(&mut self, identity: FontSourceIdentity) -> Result<u32, PdfBuildError> {
        if let Some(number) = self.numbers_by_identity.get(&identity) {
            return Ok(*number);
        }
        let number = self.next_number;
        self.next_number = self
            .next_number
            .checked_add(1)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        self.numbers_by_identity.insert(identity, number);
        Ok(number)
    }

    fn number(&self, identity: FontSourceIdentity) -> u32 {
        *self
            .numbers_by_identity
            .get(&identity)
            .expect("font identity was registered on the unified timeline")
    }

    fn engine_number(&self, raw: u32) -> u32 {
        *self
            .engine_numbers
            .get(raw as usize)
            .expect("PDF font resource owner belongs to the engine timeline")
    }
}

/// Materializes the font instances selected by reachable virtual packets.
///
/// The engine completion owns only fonts that execution itself selected. A
/// virtual packet selects its local fonts later, while the detached PDF is
/// being lowered. Allocate those resource and object identities here, inside
/// the destination input transaction, instead of mutating the completed
/// engine or retaining runtime font handles across the completion boundary.
pub(super) fn materialize_destination_font_instances(
    pdf: &DetachedPdfCompletion,
    resources: &crate::PdfVirtualFontResources,
    driver_dpi: i32,
    artifact_font_uses: &[DestinationFontUse],
    fonts: &mut BTreeMap<FontSourceIdentity, PdfFontInput>,
    next_object: &mut u32,
) -> Result<(), PdfBuildError> {
    let roots = artifact_font_uses
        .iter()
        .filter_map(|font_use| {
            let font = fonts.get(&font_use.identity)?;
            resources
                .virtual_fonts
                .contains_key(font.artifact_resource.name.as_str())
                .then_some(PendingCharacter {
                    font: LocalInstance {
                        identity: font_use.identity,
                        name: font.artifact_resource.name.clone(),
                        size: font.artifact_resource.at_size,
                        expansion_ratio: expansion_ratio(&font.artifact_resource.construction),
                    },
                    code: font_use.code,
                    depth: 0,
                    font_watermark: Some(font_use.font_watermark),
                })
        })
        .collect::<VecDeque<_>>();
    let resolved_map = crate::virtual_compile::resolved_font_map_lines(pdf, resources)
        .into_iter()
        .map(|entry| (entry.tex_name.clone(), entry))
        .collect::<BTreeMap<_, _>>();
    let glyph_mappings = pdf
        .font_operations()
        .iter()
        .filter_map(|operation| match operation {
            DetachedPdfFontOperation::GlyphToUnicode(mapping) => Some(mapping),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut pending = roots;
    let mut visited = BTreeSet::new();
    let mut loaded_virtual_fonts = BTreeSet::new();
    let engine_pdf_fonts = fonts.keys().copied().collect::<BTreeSet<_>>();
    let mut font_numbers = FontNumberTimeline::new(pdf.engine_font_identities());
    let recursion_limit = tex_out::pdf::PdfFinalizationLimits::default().max_virtual_font_recursion;

    while let Some(PendingCharacter {
        font,
        code,
        depth,
        font_watermark,
    }) = pending.pop_front()
    {
        if let Some(font_watermark) = font_watermark {
            font_numbers.advance_engine_to(font_watermark)?;
        }
        if !visited.insert((font.identity, code)) {
            continue;
        }
        if depth > recursion_limit {
            // The selected instance is already present so the pure finalizer
            // can report its canonical depth error before executing it.
            continue;
        }
        let Some(program) = resources.virtual_fonts.get(&font.name) else {
            continue;
        };
        if loaded_virtual_fonts.insert(font.identity) {
            register_virtual_local_fonts(resources, &font, program, &mut font_numbers)?;
        }
        let Some(packet) = program.program.packet(u32::from(code)) else {
            // The pure finalizer owns the canonical missing-packet error.
            continue;
        };
        let Some(default) = program.program.local_fonts().first() else {
            // Likewise, let the finalizer report a VF with no local fonts.
            continue;
        };
        let mut current = materialize_local_instance(
            pdf,
            resources,
            driver_dpi,
            &resolved_map,
            &glyph_mappings,
            &font,
            default.number,
            fonts,
            &font_numbers,
            next_object,
        )?;
        for command in &packet.commands {
            match command {
                VfCommand::SelectFont(number) => {
                    current = materialize_local_instance(
                        pdf,
                        resources,
                        driver_dpi,
                        &resolved_map,
                        &glyph_mappings,
                        &font,
                        *number,
                        fonts,
                        &font_numbers,
                        next_object,
                    )?;
                }
                VfCommand::SetCharacter { code, .. } => {
                    let Ok(code) = u8::try_from(*code) else {
                        // Preserve the pure finalizer's exact range error.
                        continue;
                    };
                    fonts
                        .get_mut(&current.identity)
                        .expect("a selected local instance was materialized")
                        .included_codes
                        .insert(code);
                    if resources.virtual_fonts.contains_key(&current.name) {
                        pending.push_back(PendingCharacter {
                            font: current.clone(),
                            code,
                            depth: depth.saturating_add(1),
                            font_watermark: None,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    let final_engine_watermark = u32::try_from(pdf.engine_font_identities().len() - 1)
        .expect("engine font capacity is bounded by u32");
    font_numbers.advance_engine_to(final_engine_watermark)?;
    for identity in engine_pdf_fonts {
        let font = fonts
            .get_mut(&identity)
            .expect("engine PDF font remains in the detached font map");
        font.resource_number = font_numbers.engine_number(font.resource_number);
    }
    Ok(())
}

fn register_virtual_local_fonts(
    resources: &crate::PdfVirtualFontResources,
    parent: &LocalInstance,
    program: &crate::CachedVirtualFont,
    font_numbers: &mut FontNumberTimeline<'_>,
) -> Result<(), PdfBuildError> {
    // pdftex.web §32e's `do_vf` processes every local font definition before
    // interpreting any character packet. `vf_def_font` reuses an equal
    // name-and-size TFM; otherwise `read_font_info` consumes the next TeX
    // internal font number. Keep that numbering ledger separate from PDF
    // resource materialization: an unselected local definition consumes a
    // font number even though it never creates a PDF font dictionary.
    for local in program.program.local_fonts() {
        let (instance, _, _) = load_local_instance(resources, parent, local.number)?;
        font_numbers.register(instance.identity)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn materialize_local_instance(
    pdf: &DetachedPdfCompletion,
    resources: &crate::PdfVirtualFontResources,
    driver_dpi: i32,
    resolved_map: &BTreeMap<Vec<u8>, tex_fonts::PdfFontMapEntry>,
    glyph_mappings: &[&tex_state::PdfGlyphToUnicode],
    parent: &LocalInstance,
    number: i32,
    fonts: &mut BTreeMap<FontSourceIdentity, PdfFontInput>,
    font_numbers: &FontNumberTimeline<'_>,
    next_object: &mut u32,
) -> Result<LocalInstance, PdfBuildError> {
    let (instance, loaded, base) = load_local_instance(resources, parent, number)?;
    let LocalInstance {
        identity,
        ref name,
        size,
        expansion_ratio,
    } = instance;
    let resource_number = font_numbers.number(identity);
    let name = name.clone();
    if !fonts.contains_key(&identity) {
        let source_identity = base.as_ref().map(tex_fonts::LoadedFont::source_identity);
        let construction =
            source_identity.map_or(FontArtifactConstructionRecipe::Loaded, |source_identity| {
                FontArtifactConstructionRecipe::Expanded {
                    source_identity,
                    ratio: expansion_ratio,
                }
            });
        let recipe = FontArtifactRecipe {
            name: name.clone(),
            tfm_content_hash: loaded.content_hash(),
            tfm_checksum: loaded.checksum(),
            design_size: loaded.design_size(),
            at_size: loaded.size(),
            layout_policy: loaded.layout_policy(),
            mapping_fallback: loaded.mapping_fallback(),
            opentype: None,
            semantic_identity: identity,
            construction,
        };
        let map_entry = resolved_map.get(name.as_bytes()).cloned();
        let encoding = map_entry
            .as_ref()
            .and_then(|entry| entry.encoding_files.first())
            .map(|encoding_name| {
                detached_encoding(pdf, resources, encoding_name)
                    .ok_or_else(|| PdfBuildError::MissingEncoding(encoding_name.clone()))
            })
            .transpose()?;
        let program = if resources.virtual_fonts.contains_key(&name) {
            PdfFontProgramInput::Resident
        } else {
            detached_font_program(pdf, resources, &recipe, map_entry.as_ref(), driver_dpi)?
        };
        let mut glyph_names = encoding
            .as_ref()
            .map(|encoding| {
                encoding
                    .glyph_names()
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        if let PdfFontProgramInput::Type1(type1) = &program {
            glyph_names.extend((0..=255).filter_map(|code| type1.builtin_glyph_name(code)));
        }
        let glyph_to_unicode =
            glyph_to_unicode_mappings(glyph_mappings, name.as_bytes(), glyph_names);
        let metrics = loaded.metrics();
        let widths = *metrics.widths();
        let heights = std::array::from_fn(|code| {
            metrics
                .character(code as u8)
                .map_or(Scaled::from_raw(0), |metric| metric.height)
        });
        let depths = std::array::from_fn(|code| {
            metrics
                .character(code as u8)
                .map_or(Scaled::from_raw(0), |metric| metric.depth)
        });
        // pdftex.web §32e's `pdf_init_font` shares one scalable font
        // dictionary when the TFM name and resolved map entry are equal,
        // even when the same VF leaf was selected at different sizes. Keep
        // the size-specific realized identity for positioning and metrics,
        // but point every such instance at the first destination resource.
        let shared_resource = map_entry.as_ref().and_then(|_| {
            fonts.values().find_map(|font| {
                (font.artifact_resource.name == name && font.map_entry == map_entry)
                    .then_some((font.resource_number, font.object_number))
            })
        });
        let (resource_number, object_number) = if let Some(shared) = shared_resource {
            shared
        } else {
            let object_number = (*next_object <= i32::MAX as u32)
                .then_some(*next_object)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            *next_object = next_object
                .checked_add(1)
                .ok_or(PdfBuildError::ObjectCapacity)?;
            (resource_number, object_number)
        };
        let configuration = pdf.font_configuration();
        let font_input = PdfFontInput {
            artifact_resource: FontResource {
                font_id: 0,
                name: name.clone(),
                tfm_content_hash: loaded.content_hash(),
                tfm_checksum: loaded.checksum(),
                design_size: loaded.design_size(),
                at_size: loaded.size(),
                layout_policy: loaded.layout_policy(),
                mapping_fallback: loaded.mapping_fallback(),
                opentype: None,
                semantic_identity: identity,
                construction: source_identity.map_or(
                    FontResourceConstruction::Loaded,
                    |source_identity| FontResourceConstruction::Expanded {
                        source_font_id: 0,
                        source_identity,
                        ratio: expansion_ratio,
                    },
                ),
            },
            resource_number,
            object_number,
            metrics: PdfFontMetricsInput {
                widths,
                heights,
                depths,
                x_height: loaded
                    .parameters()
                    .get(4)
                    .copied()
                    .unwrap_or_else(|| Scaled::from_raw(0)),
            },
            included_codes: BTreeSet::new(),
            descriptor_entries: Vec::new(),
            generate_to_unicode: configuration.generates_to_unicode(),
            disable_builtin_to_unicode: false,
            infer_builtin_glyph_unicode: !glyph_mappings.is_empty(),
            omit_charset: configuration.omits_charset(),
            glyph_to_unicode,
            map_entry,
            encoding,
            program,
        };
        if let (Some(base), Some(source_identity)) = (base, source_identity) {
            let mut source_input = font_input.clone();
            source_input.artifact_resource.semantic_identity = source_identity;
            source_input.artifact_resource.construction = FontResourceConstruction::Loaded;
            let metrics = base.metrics();
            source_input.metrics = PdfFontMetricsInput {
                widths: *metrics.widths(),
                heights: std::array::from_fn(|code| {
                    metrics
                        .character(code as u8)
                        .map_or(Scaled::from_raw(0), |metric| metric.height)
                }),
                depths: std::array::from_fn(|code| {
                    metrics
                        .character(code as u8)
                        .map_or(Scaled::from_raw(0), |metric| metric.depth)
                }),
                x_height: base
                    .parameters()
                    .get(4)
                    .copied()
                    .unwrap_or_else(|| Scaled::from_raw(0)),
            };
            fonts.entry(source_identity).or_insert(source_input);
        }
        fonts.insert(identity, font_input);
    }
    Ok(LocalInstance {
        identity,
        name,
        size,
        expansion_ratio,
    })
}

fn load_local_instance(
    resources: &crate::PdfVirtualFontResources,
    parent: &LocalInstance,
    number: i32,
) -> Result<
    (
        LocalInstance,
        tex_fonts::LoadedFont,
        Option<tex_fonts::LoadedFont>,
    ),
    PdfBuildError,
> {
    let program = resources
        .virtual_fonts
        .get(&parent.name)
        .expect("parent was selected from the detached VF catalogue");
    let local = program
        .program
        .local_fonts()
        .iter()
        .find(|local| local.number == number)
        .ok_or_else(|| PdfBuildError::MissingVirtualLocalFont {
            font: parent.name.clone(),
            number,
        })?;
    let name = String::from_utf8(local.logical_name())
        .map_err(|_| PdfBuildError::InvalidVirtualLocalFontName(parent.name.clone()))?;
    let cached = resources
        .local_tfms
        .get(&name)
        .ok_or_else(|| PdfBuildError::MissingVirtualLocalTfm(name.clone()))?;
    let size = tfm_fix_word_to_scaled(local.scaled_size.to_be_bytes(), parent.size)
        .map_err(|_| PdfBuildError::VirtualFontArithmeticOverflow)?;
    let tfm = tex_fonts::TfmFont::parse_with_size(&cached.bytes, FontSizeSpec::At(size)).map_err(
        |error| PdfBuildError::InvalidVirtualLocalTfm {
            font: name.clone(),
            message: format!("{error:?}"),
        },
    )?;
    let base = tfm.into_loaded_font(
        name.clone(),
        PathBuf::from(format!("{name}.tfm")),
        tex_fonts::font_content_hash(&cached.bytes),
    );
    // pdftex.web's `vf_expand_local_fonts` recursively copies the enclosing
    // virtual font's expansion parameters to its local fonts. Materialize the
    // expanded leaf and retain its base resource for §690's shared PDF font.
    let (loaded, base) = if parent.expansion_ratio == 0 {
        (base, None)
    } else {
        (base.expanded(parent.expansion_ratio), Some(base))
    };
    let identity = loaded.source_identity();
    Ok((
        LocalInstance {
            identity,
            name,
            size,
            expansion_ratio: parent.expansion_ratio,
        },
        loaded,
        base,
    ))
}

fn expansion_ratio(construction: &FontResourceConstruction) -> i16 {
    match construction {
        FontResourceConstruction::Expanded { ratio, .. } => *ratio,
        FontResourceConstruction::Loaded
        | FontResourceConstruction::Copied { .. }
        | FontResourceConstruction::Letterspaced { .. } => 0,
    }
}

#[cfg(test)]
mod tests;

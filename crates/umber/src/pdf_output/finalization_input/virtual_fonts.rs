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

use super::{detached_encoding, detached_font_program, glyph_to_unicode_mapping};
use crate::PdfBuildError;

#[derive(Clone, Debug)]
struct LocalInstance {
    identity: FontSourceIdentity,
    name: String,
    size: Scaled,
}

#[derive(Clone, Debug)]
struct PendingCharacter {
    font: LocalInstance,
    code: u8,
    depth: usize,
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
    artifact_font_usage: &BTreeMap<FontSourceIdentity, BTreeSet<u8>>,
    fonts: &mut BTreeMap<FontSourceIdentity, PdfFontInput>,
    next_object: &mut u32,
) -> Result<(), PdfBuildError> {
    let mut next_resource = fonts
        .values()
        .map(|font| font.resource_number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(PdfBuildError::ObjectCapacity)?;
    let roots = artifact_font_usage
        .iter()
        .filter_map(|(identity, codes)| {
            let font = fonts.get(identity)?;
            resources
                .virtual_fonts
                .contains_key(font.artifact_resource.name.as_str())
                .then_some((*identity, font, codes))
        })
        .flat_map(|(identity, font, codes)| {
            let instance = LocalInstance {
                identity,
                name: font.artifact_resource.name.clone(),
                size: font.artifact_resource.at_size,
            };
            codes.iter().copied().map(move |code| PendingCharacter {
                font: instance.clone(),
                code,
                depth: 0,
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
    let recursion_limit = tex_out::pdf::PdfFinalizationLimits::default().max_virtual_font_recursion;

    while let Some(PendingCharacter { font, code, depth }) = pending.pop_front() {
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
            &mut next_resource,
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
                        &mut next_resource,
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
                        });
                    }
                }
                _ => {}
            }
        }
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
    next_resource: &mut u32,
    next_object: &mut u32,
) -> Result<LocalInstance, PdfBuildError> {
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
    let loaded = tfm.into_loaded_font(
        name.clone(),
        PathBuf::from(format!("{name}.tfm")),
        tex_fonts::font_content_hash(&cached.bytes),
    );
    let identity = loaded.source_identity();
    if let std::collections::btree_map::Entry::Vacant(e) = fonts.entry(identity) {
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
            construction: FontArtifactConstructionRecipe::Loaded,
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
        let glyph_to_unicode = glyph_names
            .into_iter()
            .filter_map(|glyph_name| {
                glyph_to_unicode_mapping(glyph_mappings, name.as_bytes(), &glyph_name)
                    .map(|mapping| (glyph_name, mapping.unicode.clone()))
            })
            .collect();
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
        let resource_number = *next_resource;
        *next_resource = next_resource
            .checked_add(1)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        let object_number = (*next_object <= i32::MAX as u32)
            .then_some(*next_object)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        *next_object = next_object
            .checked_add(1)
            .ok_or(PdfBuildError::ObjectCapacity)?;
        let configuration = pdf.font_configuration();
        e.insert(PdfFontInput {
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
                construction: FontResourceConstruction::Loaded,
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
        });
    }
    Ok(LocalInstance {
        identity,
        name,
        size,
    })
}

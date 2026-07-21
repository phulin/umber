//! Bounded recursive virtual-font lowering for detached PDF finalization.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use tex_arith::{FontSizeSpec, Scaled, tfm_fix_word_to_scaled};
use tex_fonts::{LoadedFont, PDFTEX_VF_MAX_RECURSION, VfCommand};
use tex_out::positioned::{
    PositionedEvent, PositionedPage, PositionedPdfGraphics, PositionedRule, PositionedTextRun,
    TextUnit,
};
use tex_out::{FontResource, FontResourceConstruction, PageEffect, PdfLiteralMode};
use tex_state::Universe;
use tex_state::ids::FontId;

use crate::{PdfBuildError, PdfVirtualFontResources};

const PDFTEX_VF_STACK_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PdfVfLimits {
    pub max_recursion: usize,
    pub max_stack_depth: usize,
    pub max_packet_commands: usize,
    pub max_output_operations: usize,
    pub max_special_bytes: usize,
}

impl Default for PdfVfLimits {
    fn default() -> Self {
        Self {
            max_recursion: PDFTEX_VF_MAX_RECURSION,
            max_stack_depth: PDFTEX_VF_STACK_SIZE,
            max_packet_commands: 1_000_000,
            max_output_operations: 1_000_000,
            max_special_bytes: 8 * 1024 * 1024,
        }
    }
}

pub(crate) fn lower_pages(
    stores: &mut Universe,
    pages: &mut [PositionedPage],
    resources: &PdfVirtualFontResources,
    limits: PdfVfLimits,
) -> Result<(), PdfBuildError> {
    if resources.virtual_fonts.is_empty() {
        return Ok(());
    }
    let mut lowerer = Lowerer {
        stores,
        resources,
        limits,
        instances: BTreeMap::new(),
        active: BTreeSet::new(),
        commands: 0,
        output_operations: 0,
        special_bytes: 0,
        stack_depth: 0,
    };
    for page in pages {
        lowerer.lower_page(page)?;
    }
    Ok(())
}

struct Lowerer<'a> {
    stores: &'a mut Universe,
    resources: &'a PdfVirtualFontResources,
    limits: PdfVfLimits,
    instances: BTreeMap<(String, i32), FontId>,
    active: BTreeSet<(String, i32, u32)>,
    commands: usize,
    output_operations: usize,
    special_bytes: usize,
    stack_depth: usize,
}

impl Lowerer<'_> {
    fn lower_page(&mut self, page: &mut PositionedPage) -> Result<(), PdfBuildError> {
        let original = std::mem::take(&mut page.events);
        let mut lowered = Vec::with_capacity(original.len());
        for event in original {
            let PositionedEvent::TextRun(run) = &event else {
                lowered.push(event);
                continue;
            };
            let font = page
                .fonts
                .iter()
                .find(|font| font.font_id == run.font_id)
                .ok_or(PdfBuildError::MissingPositionedFont(run.font_id))?;
            if !self.resources.virtual_fonts.contains_key(&font.name) {
                lowered.push(event);
                continue;
            }
            let root = self
                .stores
                .font_by_source_identity(font.semantic_identity)
                .ok_or_else(|| PdfBuildError::MissingLiveFont(font.name.clone()))?;
            let mut run_lowered = Vec::new();
            let mut pending_spaces = Vec::new();
            let mut leaf_font = None;
            for index in 0..run.units.len() {
                match (run.units[index], run.physical_codes[index]) {
                    (TextUnit::Code(_), Some(code)) => {
                        let expansion_start = run_lowered.len();
                        self.expand_character(
                            page,
                            &mut run_lowered,
                            root,
                            code,
                            (run.positions[index], run.baseline),
                            1,
                        )?;
                        let first_leaf = run_lowered[expansion_start..].iter().find_map(|event| {
                            let PositionedEvent::TextRun(run) = event else {
                                return None;
                            };
                            Some(run.font_id)
                        });
                        if let Some(first_leaf) = first_leaf {
                            if !pending_spaces.is_empty() {
                                let spaces = pending_spaces
                                    .drain(..)
                                    .map(|(position, source)| {
                                        virtual_space(first_leaf, position, run.baseline, source)
                                    })
                                    .collect::<Vec<_>>();
                                run_lowered.splice(expansion_start..expansion_start, spaces);
                            }
                            leaf_font = run_lowered[expansion_start..]
                                .iter()
                                .filter_map(|event| {
                                    let PositionedEvent::TextRun(run) = event else {
                                        return None;
                                    };
                                    Some(run.font_id)
                                })
                                .next_back()
                                .or(leaf_font);
                        }
                    }
                    (TextUnit::Space, _) => {
                        let space = (run.positions[index], run.sources[index]);
                        if let Some(font_id) = leaf_font {
                            run_lowered.push(virtual_space(
                                font_id,
                                space.0,
                                run.baseline,
                                space.1,
                            ));
                        } else {
                            pending_spaces.push(space);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(font_id) = leaf_font {
                run_lowered.extend(pending_spaces.into_iter().map(|(position, source)| {
                    virtual_space(font_id, position, run.baseline, source)
                }));
            }
            lowered.extend(run_lowered);
        }
        page.events = lowered;
        Ok(())
    }

    fn expand_character(
        &mut self,
        page: &mut PositionedPage,
        output: &mut Vec<PositionedEvent>,
        font_id: FontId,
        code: u8,
        origin: (Scaled, Scaled),
        depth: usize,
    ) -> Result<(), PdfBuildError> {
        let font = self.stores.font(font_id);
        let name = font.name().to_owned();
        let size = font.size();
        let Some(cached) = self.resources.virtual_fonts.get(&name) else {
            self.emit_character(page, output, font_id, code, origin.0, origin.1)?;
            return Ok(());
        };
        if depth > self.limits.max_recursion {
            return Err(PdfBuildError::VirtualFontDepthExceeded(
                self.limits.max_recursion,
            ));
        }
        let key = (name.clone(), size.raw(), u32::from(code));
        if !self.active.insert(key.clone()) {
            return Err(PdfBuildError::VirtualFontCycle { font: name, code });
        }
        let result = self.execute_packet(
            page,
            output,
            &name,
            size,
            cached.program.clone(),
            u32::from(code),
            origin.0,
            origin.1,
            depth,
        );
        self.active.remove(&key);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_packet(
        &mut self,
        page: &mut PositionedPage,
        output: &mut Vec<PositionedEvent>,
        name: &str,
        size: Scaled,
        program: tex_fonts::VfProgram,
        code: u32,
        mut h: Scaled,
        mut v: Scaled,
        depth: usize,
    ) -> Result<(), PdfBuildError> {
        let packet = program.packet(code).cloned().ok_or_else(|| {
            PdfBuildError::MissingVirtualFontPacket {
                font: name.to_owned(),
                code,
            }
        })?;
        let default_number = program
            .local_fonts()
            .first()
            .ok_or_else(|| PdfBuildError::VirtualFontHasNoLocalFonts(name.to_owned()))?
            .number;
        let mut current = self.local_instance(&program, name, size, default_number)?;
        let mut w = Scaled::from_raw(0);
        let mut x = Scaled::from_raw(0);
        let mut y = Scaled::from_raw(0);
        let mut z = Scaled::from_raw(0);
        let mut stack = Vec::with_capacity(packet.metadata.max_stack_depth);

        for command in packet.commands {
            self.commands =
                self.commands
                    .checked_add(1)
                    .ok_or(PdfBuildError::VirtualFontWorkExceeded(
                        self.limits.max_packet_commands,
                    ))?;
            if self.commands > self.limits.max_packet_commands {
                return Err(PdfBuildError::VirtualFontWorkExceeded(
                    self.limits.max_packet_commands,
                ));
            }
            match command {
                VfCommand::SetCharacter { code, move_cursor } => {
                    let code = u8::try_from(code).map_err(|_| {
                        PdfBuildError::VirtualFontCharacterOutOfRange {
                            font: name.to_owned(),
                            code,
                        }
                    })?;
                    self.expand_character(page, output, current, code, (h, v), depth + 1)?;
                    if move_cursor {
                        h = checked_add(h, self.character_width(current, code)?)?;
                    }
                }
                VfCommand::Rule {
                    height,
                    width,
                    move_cursor,
                } => {
                    let height = scale_fix(height, size)?;
                    let width = scale_fix(width, size)?;
                    if height.raw() > 0 && width.raw() > 0 {
                        self.count_output()?;
                        output.push(PositionedEvent::Rule(PositionedRule {
                            x: h,
                            y: checked_sub(v, height)?,
                            width,
                            height,
                        }));
                    }
                    if move_cursor {
                        h = checked_add(h, width)?;
                    }
                }
                VfCommand::Nop => {}
                VfCommand::Push => {
                    self.stack_depth += 1;
                    if self.stack_depth > self.limits.max_stack_depth {
                        return Err(PdfBuildError::VirtualFontStackExceeded(
                            self.limits.max_stack_depth,
                        ));
                    }
                    stack.push((h, v, w, x, y, z));
                }
                VfCommand::Pop => {
                    let state = stack
                        .pop()
                        .ok_or(PdfBuildError::VirtualFontStackUnderflow)?;
                    (h, v, w, x, y, z) = state;
                    self.stack_depth -= 1;
                }
                VfCommand::MoveRight(value) => h = checked_add(h, scale_fix(value, size)?)?,
                VfCommand::MoveW => h = checked_add(h, w)?,
                VfCommand::SetW(value) => {
                    w = scale_fix(value, size)?;
                    h = checked_add(h, w)?;
                }
                VfCommand::MoveX => h = checked_add(h, x)?,
                VfCommand::SetX(value) => {
                    x = scale_fix(value, size)?;
                    h = checked_add(h, x)?;
                }
                VfCommand::MoveDown(value) => v = checked_add(v, scale_fix(value, size)?)?,
                VfCommand::MoveY => v = checked_add(v, y)?,
                VfCommand::SetY(value) => {
                    y = scale_fix(value, size)?;
                    v = checked_add(v, y)?;
                }
                VfCommand::MoveZ => v = checked_add(v, z)?,
                VfCommand::SetZ(value) => {
                    z = scale_fix(value, size)?;
                    v = checked_add(v, z)?;
                }
                VfCommand::SelectFont(number) => {
                    current = self.local_instance(&program, name, size, number)?;
                }
                VfCommand::Special(bytes) => self.emit_special(output, h, v, bytes)?,
            }
        }
        debug_assert!(stack.is_empty());
        Ok(())
    }

    fn local_instance(
        &mut self,
        program: &tex_fonts::VfProgram,
        parent: &str,
        parent_size: Scaled,
        number: i32,
    ) -> Result<FontId, PdfBuildError> {
        let local = program
            .local_fonts()
            .iter()
            .find(|local| local.number == number)
            .ok_or_else(|| PdfBuildError::MissingVirtualLocalFont {
                font: parent.to_owned(),
                number,
            })?;
        let name = String::from_utf8(local.logical_name())
            .map_err(|_| PdfBuildError::InvalidVirtualLocalFontName(parent.to_owned()))?;
        let size = scale_fix(local.scaled_size, parent_size)?;
        let key = (name.clone(), size.raw());
        if let Some(font) = self.instances.get(&key) {
            return Ok(*font);
        }
        let cached = self
            .resources
            .local_tfms
            .get(&name)
            .ok_or_else(|| PdfBuildError::MissingVirtualLocalTfm(name.clone()))?;
        let tfm = tex_fonts::TfmFont::parse_with_size(&cached.bytes, FontSizeSpec::At(size))
            .map_err(|error| PdfBuildError::InvalidVirtualLocalTfm {
                font: name.clone(),
                message: format!("{error:?}"),
            })?;
        let loaded = LoadedFont::new(
            name.clone(),
            PathBuf::from(format!("{name}.tfm")),
            cached.content_id.bytes(),
            tfm.header.checksum,
            tfm.header.design_size,
            tfm.font_size,
            tfm.parameters
                .values
                .iter()
                .map(|parameter| parameter.value)
                .collect(),
            tfm.font_metrics(),
        );
        let font = self.stores.try_intern_font(loaded).map_err(|error| {
            PdfBuildError::InvalidVirtualLocalTfm {
                font: name.clone(),
                message: format!("{error:?}"),
            }
        })?;
        self.stores
            .ensure_pdf_font_resource(font)
            .map_err(|_| PdfBuildError::ObjectCapacity)?;
        self.instances.insert(key, font);
        Ok(font)
    }

    fn emit_character(
        &mut self,
        page: &mut PositionedPage,
        output: &mut Vec<PositionedEvent>,
        font_id: FontId,
        code: u8,
        x: Scaled,
        baseline: Scaled,
    ) -> Result<(), PdfBuildError> {
        let artifact_font_id = if let Some(font) = page
            .fonts
            .iter()
            .find(|font| self.stores.font(font_id).source_identity() == font.semantic_identity)
        {
            font.font_id
        } else {
            let next = page
                .fonts
                .iter()
                .map(|font| font.font_id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(PdfBuildError::VirtualFontOutputExceeded(
                    self.limits.max_output_operations,
                ))?;
            let font = self.stores.font(font_id);
            page.fonts.push(FontResource {
                font_id: next,
                name: font.name().to_owned(),
                tfm_content_hash: tex_out::ContentIdentity::new(font.content_hash()),
                tfm_checksum: font.checksum(),
                design_size: font.design_size(),
                at_size: font.size(),
                opentype: None,
                semantic_identity: font.source_identity(),
                construction: FontResourceConstruction::Loaded,
            });
            next
        };
        self.count_output()?;
        output.push(PositionedEvent::TextRun(PositionedTextRun {
            x,
            baseline,
            font_id: artifact_font_id,
            units: vec![TextUnit::Code(code)],
            positions: vec![x],
            physical_codes: vec![Some(code)],
            sources: vec![None],
        }));
        Ok(())
    }

    fn emit_special(
        &mut self,
        output: &mut Vec<PositionedEvent>,
        x: Scaled,
        y: Scaled,
        bytes: Vec<u8>,
    ) -> Result<(), PdfBuildError> {
        let Some(payload) = bytes
            .strip_prefix(b"PDF:")
            .or_else(|| bytes.strip_prefix(b"pdf:"))
        else {
            return Ok(());
        };
        let (mode, payload) = if let Some(payload) = payload.strip_prefix(b"direct:") {
            (PdfLiteralMode::Direct, payload)
        } else if let Some(payload) = payload.strip_prefix(b"page:") {
            (PdfLiteralMode::Page, payload)
        } else {
            (PdfLiteralMode::Origin, payload)
        };
        self.special_bytes = self
            .special_bytes
            .checked_add(payload.len())
            .filter(|bytes| *bytes <= self.limits.max_special_bytes)
            .ok_or(PdfBuildError::VirtualFontSpecialBytesExceeded(
                self.limits.max_special_bytes,
            ))?;
        self.count_output()?;
        output.push(PositionedEvent::PdfGraphics(PositionedPdfGraphics {
            x,
            y,
            effect: PageEffect::PdfLiteral {
                mode,
                payload: payload.to_vec(),
            },
        }));
        Ok(())
    }

    fn character_width(&self, font: FontId, code: u8) -> Result<Scaled, PdfBuildError> {
        self.stores
            .font_char_metrics(font, code)
            .map(|metrics| metrics.width)
            .ok_or_else(|| PdfBuildError::MissingVirtualCharacter {
                font: self.stores.font(font).name().to_owned(),
                code,
            })
    }

    fn count_output(&mut self) -> Result<(), PdfBuildError> {
        self.output_operations = self.output_operations.checked_add(1).ok_or(
            PdfBuildError::VirtualFontOutputExceeded(self.limits.max_output_operations),
        )?;
        if self.output_operations > self.limits.max_output_operations {
            return Err(PdfBuildError::VirtualFontOutputExceeded(
                self.limits.max_output_operations,
            ));
        }
        Ok(())
    }
}

fn virtual_space(
    font_id: u32,
    position: Scaled,
    baseline: Scaled,
    source: Option<tex_out::positioned::PositionedSourceRef>,
) -> PositionedEvent {
    PositionedEvent::TextRun(PositionedTextRun {
        x: position,
        baseline,
        font_id,
        units: vec![TextUnit::Space],
        positions: vec![position],
        physical_codes: vec![None],
        sources: vec![source],
    })
}

fn scale_fix(value: i32, size: Scaled) -> Result<Scaled, PdfBuildError> {
    tfm_fix_word_to_scaled(value.to_be_bytes(), size)
        .map_err(|_| PdfBuildError::VirtualFontArithmeticOverflow)
}

fn checked_add(left: Scaled, right: Scaled) -> Result<Scaled, PdfBuildError> {
    left.checked_add(right)
        .ok_or(PdfBuildError::VirtualFontArithmeticOverflow)
}

fn checked_sub(left: Scaled, right: Scaled) -> Result<Scaled, PdfBuildError> {
    left.checked_sub(right)
        .ok_or(PdfBuildError::VirtualFontArithmeticOverflow)
}

//! Bounded recursive virtual-font lowering over the detached PDF input.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use tex_arith::{FontSizeSpec, Scaled, tfm_fix_word_to_scaled};
use tex_fonts::{FontSourceIdentity, TfmFont, VfCommand, VfProgram};

use crate::positioned::{
    PositionedEvent, PositionedPage, PositionedPdfGraphics, PositionedRule, PositionedTextRun,
    TextUnit,
};
use crate::{PageEffect, PdfLiteralMode};

use super::{PdfBuildError, PdfFinalizationInput};

pub(super) fn lower_pages(
    input: &PdfFinalizationInput,
    pages: &mut [PositionedPage],
) -> Result<(), PdfBuildError> {
    if input.virtual_fonts.is_empty() {
        return Ok(());
    }
    let mut lowerer = Lowerer {
        input,
        programs: input
            .virtual_fonts
            .iter()
            .map(|(name, input)| (name.clone(), Arc::new(input.program.clone())))
            .collect(),
        instances: BTreeMap::new(),
        instance_metrics: BTreeMap::new(),
        font_programs: BTreeMap::new(),
        real_fonts: BTreeSet::new(),
        active: Vec::new(),
        page_font_ids: BTreeMap::new(),
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
    input: &'a PdfFinalizationInput,
    programs: BTreeMap<Vec<u8>, Arc<VfProgram>>,
    instances: BTreeMap<(FontSourceIdentity, i32), FontSourceIdentity>,
    instance_metrics: BTreeMap<FontSourceIdentity, TfmFont>,
    font_programs: BTreeMap<FontSourceIdentity, (Vec<u8>, Arc<VfProgram>)>,
    real_fonts: BTreeSet<FontSourceIdentity>,
    active: Vec<(FontSourceIdentity, u32)>,
    page_font_ids: BTreeMap<FontSourceIdentity, u32>,
    commands: usize,
    output_operations: usize,
    special_bytes: usize,
    stack_depth: usize,
}

impl Lowerer<'_> {
    fn lower_page(&mut self, page: &mut PositionedPage) -> Result<(), PdfBuildError> {
        let original = std::mem::take(&mut page.events);
        self.page_font_ids = page
            .fonts
            .iter()
            .map(|font| (font.semantic_identity, font.font_id))
            .collect();
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
            if !self.input.virtual_fonts.contains_key(font.name.as_bytes()) {
                lowered.push(event);
                continue;
            }
            let root = font.semantic_identity;
            let mut run_lowered = Vec::new();
            let mut pending_spaces = Vec::new();
            let mut leaf_font = None;
            for index in 0..run.units.len() {
                match (run.units[index], run.physical_codes[index]) {
                    (TextUnit::Code(_), Some(code)) => {
                        let start = run_lowered.len();
                        self.expand_character(
                            page,
                            &mut run_lowered,
                            root,
                            code,
                            (run.positions[index], run.baseline),
                            1,
                        )?;
                        let first = run_lowered[start..].iter().find_map(text_font_id);
                        if let Some(first) = first {
                            if !pending_spaces.is_empty() {
                                let spaces = pending_spaces
                                    .drain(..)
                                    .map(|(position, source)| {
                                        virtual_space(first, position, run.baseline, source)
                                    })
                                    .collect::<Vec<_>>();
                                run_lowered.splice(start..start, spaces);
                            }
                            leaf_font = run_lowered[start..]
                                .iter()
                                .filter_map(text_font_id)
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
        font: FontSourceIdentity,
        code: u8,
        origin: (Scaled, Scaled),
        depth: usize,
    ) -> Result<(), PdfBuildError> {
        if self.real_fonts.contains(&font) {
            return self.emit_character(page, output, font, code, origin.0, origin.1);
        }
        let resource = self
            .input
            .fonts
            .get(&font)
            .ok_or_else(|| PdfBuildError::MissingFontResource(format!("{font:?}")))?;
        let (name, program) = if let Some(cached) = self.font_programs.get(&font) {
            cached.clone()
        } else {
            let name = resource.artifact_resource.name.as_bytes().to_vec();
            let Some(program) = self.programs.get(&name).cloned() else {
                self.real_fonts.insert(font);
                return self.emit_character(page, output, font, code, origin.0, origin.1);
            };
            if resource.artifact_resource.opentype.is_some() {
                return Err(PdfBuildError::UnsupportedMappedVirtualFont(
                    resource.artifact_resource.name.clone(),
                ));
            }
            self.font_programs
                .insert(font, (name.clone(), program.clone()));
            (name, program)
        };
        if depth > self.input.limits.max_virtual_font_recursion {
            return Err(PdfBuildError::VirtualFontDepthExceeded(
                self.input.limits.max_virtual_font_recursion,
            ));
        }
        let key = (font, u32::from(code));
        if self.active.contains(&key) {
            return Err(PdfBuildError::VirtualFontCycle {
                font: String::from_utf8_lossy(&name).into_owned(),
                code,
            });
        }
        self.active.push(key);
        let result = self.execute_packet(
            page,
            output,
            &name,
            font,
            resource.artifact_resource.at_size,
            program,
            u32::from(code),
            origin.0,
            origin.1,
            depth,
        );
        self.active.pop();
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_packet(
        &mut self,
        page: &mut PositionedPage,
        output: &mut Vec<PositionedEvent>,
        name: &[u8],
        parent_font: FontSourceIdentity,
        size: Scaled,
        program: Arc<VfProgram>,
        code: u32,
        mut h: Scaled,
        mut v: Scaled,
        depth: usize,
    ) -> Result<(), PdfBuildError> {
        let display = String::from_utf8_lossy(name).into_owned();
        let packet =
            program
                .packet(code)
                .ok_or_else(|| PdfBuildError::MissingVirtualFontPacket {
                    font: display.clone(),
                    code,
                })?;
        let default = program
            .local_fonts()
            .first()
            .ok_or_else(|| PdfBuildError::VirtualFontHasNoLocalFonts(display.clone()))?
            .number;
        let mut current = self.local_instance(&program, parent_font, name, size, default)?;
        let mut w = Scaled::from_raw(0);
        let mut x = Scaled::from_raw(0);
        let mut y = Scaled::from_raw(0);
        let mut z = Scaled::from_raw(0);
        let mut stack = Vec::with_capacity(packet.metadata.max_stack_depth);
        for command in &packet.commands {
            self.commands =
                self.commands
                    .checked_add(1)
                    .ok_or(PdfBuildError::VirtualFontWorkExceeded(
                        self.input.limits.max_virtual_font_packet_commands,
                    ))?;
            if self.commands > self.input.limits.max_virtual_font_packet_commands {
                return Err(PdfBuildError::VirtualFontWorkExceeded(
                    self.input.limits.max_virtual_font_packet_commands,
                ));
            }
            match command {
                VfCommand::SetCharacter { code, move_cursor } => {
                    let code = u8::try_from(*code).map_err(|_| {
                        PdfBuildError::VirtualFontCharacterOutOfRange {
                            font: display.clone(),
                            code: *code,
                        }
                    })?;
                    self.expand_character(page, output, current, code, (h, v), depth + 1)?;
                    if *move_cursor {
                        h = checked_add(h, self.character_width(current, code)?)?;
                    }
                }
                VfCommand::Rule {
                    height,
                    width,
                    move_cursor,
                } => {
                    let height = scale_fix(*height, size)?;
                    let width = scale_fix(*width, size)?;
                    if height.raw() > 0 && width.raw() > 0 {
                        self.count_output()?;
                        output.push(PositionedEvent::Rule(PositionedRule {
                            x: h,
                            y: checked_sub(v, height)?,
                            width,
                            height,
                        }));
                    }
                    if *move_cursor {
                        h = checked_add(h, width)?;
                    }
                }
                VfCommand::Nop => {}
                VfCommand::Push => {
                    self.stack_depth += 1;
                    if self.stack_depth > self.input.limits.max_virtual_font_stack_depth {
                        return Err(PdfBuildError::VirtualFontStackExceeded(
                            self.input.limits.max_virtual_font_stack_depth,
                        ));
                    }
                    stack.push((h, v, w, x, y, z));
                }
                VfCommand::Pop => {
                    (h, v, w, x, y, z) = stack
                        .pop()
                        .ok_or(PdfBuildError::VirtualFontStackUnderflow)?;
                    self.stack_depth -= 1;
                }
                VfCommand::MoveRight(value) => h = checked_add(h, scale_fix(*value, size)?)?,
                VfCommand::MoveW => h = checked_add(h, w)?,
                VfCommand::SetW(value) => {
                    w = scale_fix(*value, size)?;
                    h = checked_add(h, w)?;
                }
                VfCommand::MoveX => h = checked_add(h, x)?,
                VfCommand::SetX(value) => {
                    x = scale_fix(*value, size)?;
                    h = checked_add(h, x)?;
                }
                VfCommand::MoveDown(value) => v = checked_add(v, scale_fix(*value, size)?)?,
                VfCommand::MoveY => v = checked_add(v, y)?,
                VfCommand::SetY(value) => {
                    y = scale_fix(*value, size)?;
                    v = checked_add(v, y)?;
                }
                VfCommand::MoveZ => v = checked_add(v, z)?,
                VfCommand::SetZ(value) => {
                    z = scale_fix(*value, size)?;
                    v = checked_add(v, z)?;
                }
                VfCommand::SelectFont(number) => {
                    current = self.local_instance(&program, parent_font, name, size, *number)?;
                }
                VfCommand::Special(bytes) => self.emit_special(output, h, v, bytes)?,
            }
        }
        debug_assert!(stack.is_empty());
        Ok(())
    }

    fn local_instance(
        &mut self,
        program: &VfProgram,
        parent_font: FontSourceIdentity,
        parent: &[u8],
        parent_size: Scaled,
        number: i32,
    ) -> Result<FontSourceIdentity, PdfBuildError> {
        let key = (parent_font, number);
        if let Some(font) = self.instances.get(&key) {
            return Ok(*font);
        }
        let parent_display = String::from_utf8_lossy(parent).into_owned();
        let local = program
            .local_fonts()
            .iter()
            .find(|local| local.number == number)
            .ok_or_else(|| PdfBuildError::MissingVirtualLocalFont {
                font: parent_display.clone(),
                number,
            })?;
        let name = String::from_utf8(local.logical_name())
            .map_err(|_| PdfBuildError::InvalidVirtualLocalFontName(parent_display))?;
        let size = scale_fix(local.scaled_size, parent_size)?;
        let enclosing = self
            .input
            .virtual_fonts
            .get(parent)
            .expect("program came from detached virtual-font input");
        let cached = enclosing
            .local_tfms
            .get(name.as_bytes())
            .ok_or_else(|| PdfBuildError::MissingVirtualLocalTfm(name.clone()))?;
        let actual_content_hash = tex_fonts::font_content_hash(&cached.bytes);
        if actual_content_hash != cached.content_hash {
            return Err(PdfBuildError::InvalidVirtualLocalTfm {
                font: name,
                message: "detached content hash does not match bytes".into(),
            });
        }
        let design = TfmFont::parse(&cached.bytes).map_err(|error| {
            PdfBuildError::InvalidVirtualLocalTfm {
                font: name.clone(),
                message: format!("{error:?}"),
            }
        })?;
        if design != cached.design_font {
            return Err(PdfBuildError::InvalidVirtualLocalTfm {
                font: name,
                message: "detached design-size validation receipt does not match bytes".into(),
            });
        }
        let tfm =
            TfmFont::parse_with_size(&cached.bytes, FontSizeSpec::At(size)).map_err(|error| {
                PdfBuildError::InvalidVirtualLocalTfm {
                    font: name.clone(),
                    message: format!("{error:?}"),
                }
            })?;
        let identity = tfm
            .clone()
            .into_loaded_font(
                name.clone(),
                PathBuf::from(format!("{name}.tfm")),
                cached.content_hash,
            )
            .source_identity();
        if !self.input.fonts.contains_key(&identity) {
            return Err(PdfBuildError::MissingFontResource(name));
        }
        self.instance_metrics.insert(identity, tfm);
        self.instances.insert(key, identity);
        Ok(identity)
    }

    fn emit_character(
        &mut self,
        page: &mut PositionedPage,
        output: &mut Vec<PositionedEvent>,
        font: FontSourceIdentity,
        code: u8,
        x: Scaled,
        baseline: Scaled,
    ) -> Result<(), PdfBuildError> {
        let artifact_font_id = if let Some(font_id) = self.page_font_ids.get(&font) {
            *font_id
        } else {
            let next = page
                .fonts
                .iter()
                .map(|font| font.font_id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(PdfBuildError::VirtualFontOutputExceeded(
                    self.input.limits.max_virtual_font_output_operations,
                ))?;
            let resource = self
                .input
                .fonts
                .get(&font)
                .ok_or_else(|| PdfBuildError::MissingFontResource(format!("{font:?}")))?;
            let mut artifact = resource.artifact_resource.clone();
            artifact.font_id = next;
            page.fonts.push(artifact);
            self.page_font_ids.insert(font, next);
            next
        };
        self.count_output()?;
        if let Some(PositionedEvent::TextRun(run)) = output.last_mut()
            && run.font_id == artifact_font_id
            && run.baseline == baseline
        {
            run.units.push(TextUnit::Code(u32::from(code)));
            run.positions.push(x);
            run.physical_codes.push(Some(code));
            run.sources.push(None);
            return Ok(());
        }
        output.push(PositionedEvent::TextRun(PositionedTextRun {
            x,
            baseline,
            font_id: artifact_font_id,
            units: vec![TextUnit::Code(u32::from(code))],
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
        bytes: &[u8],
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
            .filter(|bytes| *bytes <= self.input.limits.max_virtual_font_special_bytes)
            .ok_or(PdfBuildError::VirtualFontSpecialBytesExceeded(
                self.input.limits.max_virtual_font_special_bytes,
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

    fn character_width(&self, font: FontSourceIdentity, code: u8) -> Result<Scaled, PdfBuildError> {
        self.instance_metrics
            .get(&font)
            .and_then(|tfm| tfm.metrics().character(code))
            .map(|metrics| metrics.width)
            .or_else(|| {
                self.input
                    .fonts
                    .get(&font)
                    .map(|font| font.metrics.widths[usize::from(code)])
            })
            .ok_or_else(|| PdfBuildError::MissingVirtualCharacter {
                font: self.input.fonts.get(&font).map_or_else(
                    || format!("{font:?}"),
                    |font| font.artifact_resource.name.clone(),
                ),
                code,
            })
    }

    fn count_output(&mut self) -> Result<(), PdfBuildError> {
        self.output_operations = self.output_operations.checked_add(1).ok_or(
            PdfBuildError::VirtualFontOutputExceeded(
                self.input.limits.max_virtual_font_output_operations,
            ),
        )?;
        if self.output_operations > self.input.limits.max_virtual_font_output_operations {
            return Err(PdfBuildError::VirtualFontOutputExceeded(
                self.input.limits.max_virtual_font_output_operations,
            ));
        }
        Ok(())
    }
}

fn text_font_id(event: &PositionedEvent) -> Option<u32> {
    let PositionedEvent::TextRun(run) = event else {
        return None;
    };
    Some(run.font_id)
}

fn virtual_space(
    font_id: u32,
    position: Scaled,
    baseline: Scaled,
    source: Option<crate::positioned::PositionedSourceRef>,
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

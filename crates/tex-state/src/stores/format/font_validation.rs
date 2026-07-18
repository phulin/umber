use super::*;

const FONT_DIMEN_BITS: u32 = 17;
const FONT_DIMEN_MASK: u32 = (1 << FONT_DIMEN_BITS) - 1;
const MAX_FONT_COUNT: usize = crate::font::MAX_FONT_COUNT;
const MAX_FONT_PARAMETERS: usize = crate::font::MAX_FONT_DIMEN as usize;

impl StoreFormat {
    pub(super) fn validate_font_state(&self) -> Result<(), StoreFormatError> {
        if self.fonts.is_empty() {
            return Err(StoreFormatError::Invalid("missing null font"));
        }
        if self.fonts.len() > MAX_FONT_COUNT {
            return Err(StoreFormatError::Invalid(
                "font count exceeds bank capacity",
            ));
        }

        let canonical_fonts = FontStore::new();
        let mut encoded_null = self.fonts[0].clone();
        encoded_null.identifier = None;
        if encoded_null != FormatFont::capture(&canonical_fonts, NULL_FONT) {
            return Err(StoreFormatError::Invalid("non-canonical null font"));
        }

        let mut source_identities = std::collections::BTreeSet::new();
        for (raw, font) in self.fonts.iter().enumerate() {
            if font.parameters.len() < tex_fonts::metrics::MIN_TEX_FONT_PARAMETERS {
                return Err(StoreFormatError::Invalid(
                    "immutable font has fewer than seven parameters",
                ));
            }
            if font.parameters.len() > MAX_FONT_PARAMETERS {
                return Err(StoreFormatError::Invalid(
                    "immutable font parameter count exceeds bank capacity",
                ));
            }
            if font.source_parameters.len() < tex_fonts::metrics::MIN_TEX_FONT_PARAMETERS {
                return Err(StoreFormatError::Invalid(
                    "font source has fewer than seven parameters",
                ));
            }
            if font.source_parameters.len() > MAX_FONT_PARAMETERS {
                return Err(StoreFormatError::Invalid(
                    "font source parameter count exceeds bank capacity",
                ));
            }
            if font
                .identifier
                .is_some_and(|symbol| symbol as usize >= self.names.len())
            {
                return Err(StoreFormatError::Invalid("font identifier is not live"));
            }
            font.metrics()
                .validate()
                .map_err(|source| StoreFormatError::InvalidFontMetrics { font: raw, source })?;
            let source = match font.construction {
                FormatFontConstruction::Loaded => None,
                FormatFontConstruction::Copied { source }
                | FormatFontConstruction::Letterspaced { source, .. }
                | FormatFontConstruction::Expanded { source, .. } => Some(source),
            };
            if source.is_some_and(|source| !source_identities.contains(&source)) {
                return Err(StoreFormatError::Invalid(
                    "generated font source is not an earlier font",
                ));
            }
            source_identities.insert(font.clone().restore().source_identity().bytes());
        }
        if self.last_loaded_font as usize >= self.fonts.len() {
            return Err(StoreFormatError::Invalid("last loaded font is not live"));
        }

        let mut parameter_counts = vec![None; self.fonts.len()];
        let mut dimension_slots = Vec::new();
        let mut seen_font_cells = std::collections::BTreeSet::new();
        for entry in &self.env {
            let cell = crate::cell::CellId::from_raw(entry.cell)
                .ok_or(StoreFormatError::Invalid("unknown environment cell"))?;
            let bank = cell.bank();
            let is_font_cell = matches!(
                bank,
                crate::cell::BankTag::FontDimen
                    | crate::cell::BankTag::FontParamLen
                    | crate::cell::BankTag::FontHyphenChar
                    | crate::cell::BankTag::FontSkewChar
                    | crate::cell::BankTag::CurrentFont
                    | crate::cell::BankTag::MathFamilyFont
            );
            if !is_font_cell {
                continue;
            }
            let word = match entry.value {
                FormatEnvValue::Raw(word) => word,
                FormatEnvValue::Box(_) => {
                    return Err(StoreFormatError::Invalid(
                        "box value in environment font bank",
                    ));
                }
            };
            if cell.is_global() {
                return Err(StoreFormatError::Invalid(
                    "format environment contains a global font cell",
                ));
            }
            let index = cell.index();
            if !seen_font_cells.insert((bank as u8, index)) {
                return Err(StoreFormatError::Invalid("duplicate environment font cell"));
            }
            match bank {
                crate::cell::BankTag::FontDimen => {
                    if word > u64::from(u32::MAX) {
                        return Err(StoreFormatError::Invalid("non-canonical fontdimen word"));
                    }
                    let font = (index >> FONT_DIMEN_BITS) as usize;
                    if font >= self.fonts.len() {
                        return Err(StoreFormatError::Invalid("fontdimen font is not live"));
                    }
                    let slot = ((index & FONT_DIMEN_MASK) + 1) as usize;
                    dimension_slots.push((font, slot));
                }
                crate::cell::BankTag::FontParamLen => {
                    let font = index as usize;
                    if font >= self.fonts.len() {
                        return Err(StoreFormatError::Invalid(
                            "font parameter-count font is not live",
                        ));
                    }
                    let count = usize::try_from(word).map_err(|_| {
                        StoreFormatError::Invalid("font parameter count exceeds usize")
                    })?;
                    if !(tex_fonts::metrics::MIN_TEX_FONT_PARAMETERS..=MAX_FONT_PARAMETERS)
                        .contains(&count)
                    {
                        return Err(StoreFormatError::Invalid(
                            "font parameter count is outside bank bounds",
                        ));
                    }
                    parameter_counts[font] = Some(count);
                }
                crate::cell::BankTag::FontHyphenChar | crate::cell::BankTag::FontSkewChar => {
                    if index as usize >= self.fonts.len() {
                        return Err(StoreFormatError::Invalid(
                            "font integer-bank font is not live",
                        ));
                    }
                    if word > u64::from(u32::MAX) {
                        return Err(StoreFormatError::Invalid("non-canonical font integer word"));
                    }
                }
                crate::cell::BankTag::CurrentFont => {
                    if index != 0 {
                        return Err(StoreFormatError::Invalid("current-font cell index"));
                    }
                    if word as u32 as usize >= self.fonts.len() {
                        return Err(StoreFormatError::Invalid("current font is not live"));
                    }
                    let symbol_plus_one = word >> 32;
                    if symbol_plus_one != 0 && (symbol_plus_one - 1) as usize >= self.names.len() {
                        return Err(StoreFormatError::Invalid(
                            "current-font identifier is not live",
                        ));
                    }
                }
                crate::cell::BankTag::MathFamilyFont => {
                    let count = 3 * u32::from(crate::math::MATH_FAMILY_COUNT);
                    if index >= count {
                        return Err(StoreFormatError::Invalid("math-family font cell index"));
                    }
                    if word > u64::from(u32::MAX) || word as u32 as usize >= self.fonts.len() {
                        return Err(StoreFormatError::Invalid("math-family font is not live"));
                    }
                }
                _ => unreachable!("match is restricted to font cells"),
            }
        }

        for (font, encoded) in self.fonts.iter().zip(&parameter_counts) {
            let count = encoded.ok_or(StoreFormatError::Invalid(
                "missing font parameter-count bank value",
            ))?;
            if count < font.parameters.len() {
                return Err(StoreFormatError::Invalid(
                    "font parameter count is shorter than immutable parameters",
                ));
            }
        }
        for (font, slot) in dimension_slots {
            if slot > parameter_counts[font].expect("all font counts validated above") {
                return Err(StoreFormatError::Invalid(
                    "fontdimen slot exceeds font parameter count",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum TestingFontFormatCorruption {
    TooManyCharacters,
    OversizedLigKernProgram,
    LigKernProgramLength { len: usize, start: u16 },
    LigKernStart,
    ExtensibleRecipeIndex,
    FontIdentifier,
    FontParameterCount,
    FontDimenSlot,
    CurrentFont,
    LastLoadedFont,
}

#[cfg(test)]
pub(crate) fn testing_corrupt_font_format(
    environment: &[u8],
    frozen_fonts: &[u8],
    corruption: TestingFontFormatCorruption,
) -> (Vec<u8>, Vec<u8>) {
    const HEADER: usize = 32;
    let mut env = super::frozen_env::decode(environment).expect("test frozen environment payload");
    let mut fonts: Vec<FormatFont> =
        bincode::deserialize(&frozen_fonts[HEADER..]).expect("test frozen font payload");
    let font = fonts.get_mut(1).expect("test format has a loaded font");
    match corruption {
        TestingFontFormatCorruption::TooManyCharacters => font.characters.resize(257, None),
        TestingFontFormatCorruption::OversizedLigKernProgram => {
            let instruction = font.lig_kern_program[0];
            font.lig_kern_program.resize(
                tex_fonts::metrics::MAX_LIG_KERN_PROGRAM_LEN + 1,
                instruction,
            );
        }
        TestingFontFormatCorruption::LigKernProgramLength { len, start } => {
            let instruction = font.lig_kern_program[0];
            font.lig_kern_program.resize(len, instruction);
            let character = font
                .characters
                .iter_mut()
                .flatten()
                .find(|character| matches!(character.tag, tex_fonts::MetricCharTag::LigKern { .. }))
                .expect("test font has a lig/kern character");
            let tex_fonts::MetricCharTag::LigKern {
                ref mut start_index,
                ..
            } = character.tag
            else {
                unreachable!()
            };
            *start_index = start;
        }
        TestingFontFormatCorruption::LigKernStart => {
            let character = font
                .characters
                .iter_mut()
                .flatten()
                .find(|character| matches!(character.tag, tex_fonts::MetricCharTag::LigKern { .. }))
                .expect("test font has a lig/kern character");
            let tex_fonts::MetricCharTag::LigKern {
                ref mut start_index,
                ..
            } = character.tag
            else {
                unreachable!()
            };
            *start_index = u16::MAX;
        }
        TestingFontFormatCorruption::ExtensibleRecipeIndex => {
            let character = font
                .characters
                .iter_mut()
                .flatten()
                .find(|character| matches!(character.tag, tex_fonts::MetricCharTag::Extensible(_)))
                .expect("test font has an extensible character");
            character.tag = tex_fonts::MetricCharTag::Extensible(u8::MAX);
        }
        TestingFontFormatCorruption::FontIdentifier => font.identifier = Some(u32::MAX),
        TestingFontFormatCorruption::FontParameterCount => {
            let raw = crate::cell::CellId::new(crate::cell::BankTag::FontParamLen, 1).raw();
            env.iter_mut()
                .find(|entry| entry.cell == raw)
                .expect("test format has a font parameter count")
                .value = FormatEnvValue::Raw(6);
        }
        TestingFontFormatCorruption::FontDimenSlot => {
            let index = (1 << FONT_DIMEN_BITS) | 7;
            let raw = crate::cell::CellId::new(crate::cell::BankTag::FontDimen, index).raw();
            env.push(FormatEnvEntry {
                cell: raw,
                value: FormatEnvValue::Raw(1),
            });
        }
        TestingFontFormatCorruption::CurrentFont => {
            let raw = crate::cell::CellId::new(crate::cell::BankTag::CurrentFont, 0).raw();
            let word = u64::from(u32::MAX);
            if let Some(entry) = env.iter_mut().find(|entry| entry.cell == raw) {
                entry.value = FormatEnvValue::Raw(word);
            } else {
                env.push(FormatEnvEntry {
                    cell: raw,
                    value: FormatEnvValue::Raw(word),
                });
            }
        }
        TestingFontFormatCorruption::LastLoadedFont => {
            // The last-loaded font index is fixed metadata in the section header.
        }
    }
    let mut frozen = frozen_fonts[..HEADER].to_vec();
    let font_payload = bincode::serialize(&fonts).expect("corrupted frozen fonts serialize");
    frozen[4..8].copy_from_slice(&(fonts.len() as u32).to_le_bytes());
    frozen[12..16].copy_from_slice(&(font_payload.len() as u32).to_le_bytes());
    if matches!(corruption, TestingFontFormatCorruption::LastLoadedFont) {
        frozen[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
    }
    frozen.extend_from_slice(&font_payload);
    env.sort_unstable_by_key(|entry| entry.cell);
    (
        super::frozen_env::encode(&env).expect("corrupted frozen environment serializes"),
        frozen,
    )
}

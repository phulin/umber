use tex_arith::{
    FontSizeSpec, Scaled, tfm_design_size_from_fix_word, tfm_fix_word_to_scaled, tfm_font_size,
    tfm_slant_fix_word_to_scaled_ratio,
};

use crate::metrics::MIN_TEX_FONT_PARAMETERS;

use super::error::ParseError;
use super::types::{
    Character, CharacterBounds, CharacterTag, ExtensibleRecipe, FontParameter, FontParameterKind,
    FontParameters, Header, Kern, LigKernAction, LigKernStep, Ligature, LigatureDeletes, TfmFont,
    TfmTable,
};

const PREAMBLE_WORDS: usize = 6;
const PREAMBLE_BYTES: usize = PREAMBLE_WORDS * 4;
const HEADER_WORDS_FOR_CODING_SCHEME: usize = 12;
const HEADER_WORDS_FOR_FAMILY: usize = 17;
const HEADER_WORDS_FOR_FACE: usize = 18;
const CHARACTER_SLOTS: usize = 256;
const STOP_FLAG: u8 = 128;
const KERN_FLAG: u8 = 128;

pub(super) fn parse_tfm(bytes: &[u8], size_spec: FontSizeSpec) -> Result<TfmFont, ParseError> {
    if bytes.len() < PREAMBLE_BYTES {
        return Err(ParseError::TooShort {
            actual_bytes: bytes.len(),
        });
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(ParseError::LengthNotMultipleOfFour {
            actual_bytes: bytes.len(),
        });
    }

    let words = parse_words(bytes);
    let counts = Counts::parse(bytes)?;
    counts.validate(words.len())?;

    let mut cursor = PREAMBLE_WORDS;
    let header_words = take_words(&words, &mut cursor, counts.lh);
    let char_info_words = take_words(&words, &mut cursor, counts.char_count);
    let width_words = take_words(&words, &mut cursor, counts.nw);
    let height_words = take_words(&words, &mut cursor, counts.nh);
    let depth_words = take_words(&words, &mut cursor, counts.nd);
    let italic_words = take_words(&words, &mut cursor, counts.ni);
    let lig_kern_words = take_words(&words, &mut cursor, counts.nl);
    let kern_words = take_words(&words, &mut cursor, counts.nk);
    let extensible_words = take_words(&words, &mut cursor, counts.ne);
    let param_words = take_words(&words, &mut cursor, counts.np);

    let design_size =
        tfm_design_size_from_fix_word(header_words[1]).map_err(ParseError::InvalidDesignSize)?;
    let font_size = tfm_font_size(design_size, size_spec).map_err(ParseError::InvalidDesignSize)?;
    let header = parse_header(header_words, design_size)?;

    let widths = parse_metric_table(TfmTable::Width, width_words, font_size, true)?;
    let heights = parse_metric_table(TfmTable::Height, height_words, font_size, true)?;
    let depths = parse_metric_table(TfmTable::Depth, depth_words, font_size, true)?;
    let italic_corrections = parse_metric_table(TfmTable::Italic, italic_words, font_size, true)?;
    let kerns = parse_metric_table(TfmTable::Kern, kern_words, font_size, false)?;
    let parameters = parse_parameters(param_words, font_size)?;

    let mut characters = vec![None; CHARACTER_SLOTS];
    parse_characters(
        &mut characters,
        &counts,
        char_info_words,
        &widths,
        &heights,
        &depths,
        &italic_corrections,
        counts.nl,
        counts.ne,
    )?;

    let lig_kern_program = parse_lig_kern_program(lig_kern_words, &kerns)?;
    let right_boundary_char = lig_kern_program
        .first()
        .filter(|step| step.skip_byte == u8::MAX)
        .map(|step| step.next_char);
    let left_boundary_program = if lig_kern_program.len() > 1 {
        lig_kern_program
            .last()
            .filter(|step| step.skip_byte == u8::MAX)
            .and_then(|step| step.restart_index)
    } else {
        None
    };

    resolve_lig_kern_starts(&mut characters, &lig_kern_program)?;
    validate_next_larger_chains(&characters)?;
    let extensible_recipes = parse_extensible_recipes(extensible_words, &characters)?;

    Ok(TfmFont {
        header,
        bounds: CharacterBounds {
            bc: counts.bc,
            ec: counts.ec,
        },
        font_size,
        characters,
        widths,
        heights,
        depths,
        italic_corrections,
        lig_kern_program,
        right_boundary_char,
        left_boundary_program,
        kerns,
        extensible_recipes,
        parameters,
    })
}

#[derive(Clone, Copy, Debug)]
struct Counts {
    lf: u16,
    lh: usize,
    bc: u8,
    ec: u8,
    char_count: usize,
    nw: usize,
    nh: usize,
    nd: usize,
    ni: usize,
    nl: usize,
    nk: usize,
    ne: usize,
    np: usize,
}

impl Counts {
    fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let lf = read_u16(bytes, 0);
        let lh = read_u16(bytes, 2);
        let bc = read_u16(bytes, 4);
        let ec = read_u16(bytes, 6);
        let nw = read_u16(bytes, 8);
        let nh = read_u16(bytes, 10);
        let nd = read_u16(bytes, 12);
        let ni = read_u16(bytes, 14);
        let nl = read_u16(bytes, 16);
        let nk = read_u16(bytes, 18);
        let ne = read_u16(bytes, 20);
        let np = read_u16(bytes, 22);

        if lh < 2 {
            return Err(ParseError::MissingRequiredHeader { lh });
        }
        if ec > 255 || bc > ec.saturating_add(1) {
            return Err(ParseError::InvalidCharacterBounds { bc, ec });
        }
        let char_count = if bc <= ec {
            usize::from(ec - bc + 1)
        } else {
            0
        };
        let (stored_bc, stored_ec) = if char_count == 0 {
            (1, 0)
        } else {
            (
                u8::try_from(bc).map_err(|_| ParseError::InvalidCharacterBounds { bc, ec })?,
                u8::try_from(ec).map_err(|_| ParseError::InvalidCharacterBounds { bc, ec })?,
            )
        };

        Ok(Self {
            lf,
            lh: usize::from(lh),
            bc: stored_bc,
            ec: stored_ec,
            char_count,
            nw: usize::from(nw),
            nh: usize::from(nh),
            nd: usize::from(nd),
            ni: usize::from(ni),
            nl: usize::from(nl),
            nk: usize::from(nk),
            ne: usize::from(ne),
            np: usize::from(np),
        })
    }

    fn validate(self, actual_words: usize) -> Result<(), ParseError> {
        if usize::from(self.lf) != actual_words {
            return Err(ParseError::DeclaredLengthMismatch {
                declared_words: self.lf,
                actual_words,
            });
        }
        for (table, len) in [
            (TfmTable::Width, self.nw),
            (TfmTable::Height, self.nh),
            (TfmTable::Depth, self.nd),
            (TfmTable::Italic, self.ni),
        ] {
            if len == 0 {
                return Err(ParseError::EmptyMetricTable(table));
            }
        }

        let computed = [
            PREAMBLE_WORDS,
            self.lh,
            self.char_count,
            self.nw,
            self.nh,
            self.nd,
            self.ni,
            self.nl,
            self.nk,
            self.ne,
            self.np,
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .ok_or(ParseError::SectionLengthOverflow)?;

        if computed != usize::from(self.lf) {
            return Err(ParseError::SectionLengthMismatch {
                declared_words: self.lf,
                computed_words: computed,
            });
        }
        Ok(())
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn parse_words(bytes: &[u8]) -> Vec<[u8; 4]> {
    bytes
        .chunks_exact(4)
        .map(|chunk| [chunk[0], chunk[1], chunk[2], chunk[3]])
        .collect()
}

fn take_words<'a>(words: &'a [[u8; 4]], cursor: &mut usize, count: usize) -> &'a [[u8; 4]] {
    let start = *cursor;
    *cursor += count;
    &words[start..*cursor]
}

fn parse_header(words: &[[u8; 4]], design_size: Scaled) -> Result<Header, ParseError> {
    let checksum = u32::from_be_bytes(words[0]);
    let header_bytes: Vec<u8> = words.iter().flatten().copied().collect();
    let coding_scheme = if words.len() >= HEADER_WORDS_FOR_CODING_SCHEME {
        Some(parse_bcpl_string(
            &header_bytes[8..HEADER_WORDS_FOR_CODING_SCHEME * 4],
            "coding scheme",
        )?)
    } else {
        None
    };
    let family = if words.len() >= HEADER_WORDS_FOR_FAMILY {
        Some(parse_bcpl_string(
            &header_bytes[HEADER_WORDS_FOR_CODING_SCHEME * 4..HEADER_WORDS_FOR_FAMILY * 4],
            "family",
        )?)
    } else {
        None
    };
    let (seven_bit_safe, face) = if words.len() >= HEADER_WORDS_FOR_FACE {
        let word = words[17];
        (Some(word[0] >= 128), Some(word[3]))
    } else {
        (None, None)
    };
    let additional_words = if words.len() > HEADER_WORDS_FOR_FACE {
        words[HEADER_WORDS_FOR_FACE..].to_vec()
    } else {
        Vec::new()
    };

    Ok(Header {
        checksum,
        design_size,
        coding_scheme,
        family,
        seven_bit_safe,
        face,
        additional_words,
    })
}

fn parse_bcpl_string(bytes: &[u8], field: &'static str) -> Result<String, ParseError> {
    let length = bytes[0];
    let capacity = bytes.len() - 1;
    if usize::from(length) > capacity {
        return Err(ParseError::InvalidHeaderString {
            field,
            length,
            capacity,
        });
    }
    Ok(bytes[1..=usize::from(length)]
        .iter()
        .map(|&byte| char::from(byte))
        .collect())
}

fn parse_metric_table(
    table: TfmTable,
    words: &[[u8; 4]],
    font_size: Scaled,
    require_zero_first: bool,
) -> Result<Vec<Scaled>, ParseError> {
    if words.is_empty() {
        return Ok(Vec::new());
    }
    if require_zero_first && words[0] != [0, 0, 0, 0] {
        return Err(ParseError::NonZeroFirstMetric(table));
    }
    words
        .iter()
        .enumerate()
        .map(|(index, &word)| {
            tfm_fix_word_to_scaled(word, font_size).map_err(|source| ParseError::InvalidFixWord {
                table,
                index,
                source,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn parse_characters(
    characters: &mut [Option<Character>],
    counts: &Counts,
    words: &[[u8; 4]],
    widths: &[Scaled],
    heights: &[Scaled],
    depths: &[Scaled],
    italic_corrections: &[Scaled],
    nl: usize,
    ne: usize,
) -> Result<(), ParseError> {
    for (offset, &word) in words.iter().enumerate() {
        let code = counts.bc + offset as u8;
        let width_index = word[0];
        let height_index = word[1] >> 4;
        let depth_index = word[1] & 0x0f;
        let italic_index = word[2] >> 2;
        let tag = word[2] & 0x03;
        let remainder = word[3];

        let width = table_value(code, TfmTable::Width, width_index, widths)?;
        let height = table_value(code, TfmTable::Height, height_index, heights)?;
        let depth = table_value(code, TfmTable::Depth, depth_index, depths)?;
        let italic_correction =
            table_value(code, TfmTable::Italic, italic_index, italic_corrections)?;

        if width_index == 0 {
            if tag != 0 {
                return Err(ParseError::MissingCharacterHasTag { code, tag });
            }
            continue;
        }
        let tag = match tag {
            0 => CharacterTag::None,
            1 => {
                if usize::from(remainder) >= nl {
                    return Err(ParseError::LigKernProgramIndexOutOfBounds {
                        code,
                        index: remainder,
                        len: nl,
                    });
                }
                CharacterTag::LigKern {
                    program_index: remainder,
                    start_index: u16::from(remainder),
                }
            }
            2 => CharacterTag::NextLarger(remainder),
            3 => {
                if usize::from(remainder) >= ne {
                    return Err(ParseError::ExtensibleRecipeIndexOutOfBounds {
                        code,
                        index: remainder,
                        len: ne,
                    });
                }
                CharacterTag::Extensible(remainder)
            }
            _ => unreachable!("tag is masked to two bits"),
        };

        characters[usize::from(code)] = Some(Character {
            code,
            width_index,
            height_index,
            depth_index,
            italic_index,
            width,
            height,
            depth,
            italic_correction,
            tag,
        });
    }
    Ok(())
}

fn table_value(
    code: u8,
    table: TfmTable,
    index: u8,
    values: &[Scaled],
) -> Result<Scaled, ParseError> {
    values
        .get(usize::from(index))
        .copied()
        .ok_or(ParseError::CharMetricIndexOutOfBounds {
            code,
            table,
            index,
            len: values.len(),
        })
}

fn parse_lig_kern_program(
    words: &[[u8; 4]],
    kerns: &[Scaled],
) -> Result<Vec<LigKernStep>, ParseError> {
    words
        .iter()
        .enumerate()
        .map(|(index, &word)| {
            let [skip_byte, next_char, op_byte, remainder] = word;
            let restart_index = if skip_byte > STOP_FLAG {
                let target = u16::from(op_byte) * 256 + u16::from(remainder);
                if usize::from(target) >= words.len() {
                    return Err(ParseError::LigKernRestartOutOfBounds {
                        index,
                        target,
                        len: words.len(),
                    });
                }
                Some(target)
            } else {
                None
            };

            if skip_byte < STOP_FLAG {
                let target = index + usize::from(skip_byte) + 1;
                if target >= words.len() {
                    return Err(ParseError::LigKernSkipOutOfBounds {
                        index,
                        target,
                        len: words.len(),
                    });
                }
            }

            let action = if skip_byte > STOP_FLAG {
                None
            } else if op_byte >= KERN_FLAG {
                let kern_index = u16::from(op_byte - KERN_FLAG) * 256 + u16::from(remainder);
                let amount = kerns.get(usize::from(kern_index)).copied().ok_or(
                    ParseError::KernIndexOutOfBounds {
                        instruction: index,
                        index: kern_index,
                        len: kerns.len(),
                    },
                )?;
                Some(LigKernAction::Kern(Kern { kern_index, amount }))
            } else {
                let deletes = LigatureDeletes {
                    current: op_byte & 0b10 == 0,
                    next: op_byte & 0b01 == 0,
                };
                Some(LigKernAction::Ligature(Ligature {
                    replacement: remainder,
                    deletes,
                    pass_over: op_byte >> 2,
                }))
            };

            Ok(LigKernStep {
                skip_byte,
                next_char,
                op_byte,
                remainder,
                restart_index,
                action,
            })
        })
        .collect()
}

fn resolve_lig_kern_starts(
    characters: &mut [Option<Character>],
    program: &[LigKernStep],
) -> Result<(), ParseError> {
    for character in characters.iter_mut().flatten() {
        if let CharacterTag::LigKern { program_index, .. } = character.tag {
            let step = &program[usize::from(program_index)];
            let start_index = step.restart_index.unwrap_or(u16::from(program_index));
            character.tag = CharacterTag::LigKern {
                program_index,
                start_index,
            };
        }
    }
    Ok(())
}

fn validate_next_larger_chains(characters: &[Option<Character>]) -> Result<(), ParseError> {
    for character in characters.iter().flatten() {
        if matches!(character.tag, CharacterTag::NextLarger(_)) {
            let mut seen = [false; CHARACTER_SLOTS];
            let mut code = character.code;
            loop {
                if seen[usize::from(code)] {
                    return Err(ParseError::NextLargerCycle {
                        code: character.code,
                    });
                }
                seen[usize::from(code)] = true;
                let current = characters[usize::from(code)].as_ref().ok_or(
                    ParseError::NextLargerCharacterMissing {
                        code: character.code,
                        next: code,
                    },
                )?;
                let CharacterTag::NextLarger(next) = current.tag else {
                    break;
                };
                if characters[usize::from(next)].is_none() {
                    return Err(ParseError::NextLargerCharacterMissing {
                        code: current.code,
                        next,
                    });
                }
                code = next;
            }
        }
    }
    Ok(())
}

fn parse_extensible_recipes(
    words: &[[u8; 4]],
    characters: &[Option<Character>],
) -> Result<Vec<ExtensibleRecipe>, ParseError> {
    words
        .iter()
        .enumerate()
        .map(|(recipe, &word)| {
            let [top, middle, bottom, repeated] = word;
            let top = optional_recipe_piece(recipe, "top", top, characters)?;
            let middle = optional_recipe_piece(recipe, "middle", middle, characters)?;
            let bottom = optional_recipe_piece(recipe, "bottom", bottom, characters)?;
            require_recipe_piece(recipe, "repeated", repeated, characters)?;
            Ok(ExtensibleRecipe {
                top,
                middle,
                bottom,
                repeated,
            })
        })
        .collect()
}

fn optional_recipe_piece(
    recipe: usize,
    field: &'static str,
    code: u8,
    characters: &[Option<Character>],
) -> Result<Option<u8>, ParseError> {
    if code == 0 {
        return Ok(None);
    }
    require_recipe_piece(recipe, field, code, characters)?;
    Ok(Some(code))
}

fn require_recipe_piece(
    recipe: usize,
    field: &'static str,
    code: u8,
    characters: &[Option<Character>],
) -> Result<(), ParseError> {
    if characters[usize::from(code)].is_none() {
        return Err(ParseError::ExtensibleRecipeCharacterMissing {
            recipe,
            field,
            code,
        });
    }
    Ok(())
}

fn parse_parameters(words: &[[u8; 4]], font_size: Scaled) -> Result<FontParameters, ParseError> {
    let mut values = words
        .iter()
        .enumerate()
        .map(|(index, &word)| {
            let kind = if index == 0 {
                FontParameterKind::SlantRatio
            } else {
                FontParameterKind::Dimension
            };
            let size = if index == 0 {
                let value = tfm_slant_fix_word_to_scaled_ratio(word);
                return Ok(FontParameter {
                    number: (index + 1) as u16,
                    value,
                    kind,
                });
            } else {
                font_size
            };
            let value = tfm_fix_word_to_scaled(word, size).map_err(|source| {
                ParseError::InvalidFixWord {
                    table: TfmTable::Param,
                    index,
                    source,
                }
            })?;
            Ok(FontParameter {
                number: (index + 1) as u16,
                value,
                kind,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    while values.len() < MIN_TEX_FONT_PARAMETERS {
        let index = values.len();
        values.push(FontParameter {
            number: (index + 1) as u16,
            value: Scaled::from_raw(0),
            kind: if index == 0 {
                FontParameterKind::SlantRatio
            } else {
                FontParameterKind::Dimension
            },
        });
    }
    Ok(FontParameters { values })
}

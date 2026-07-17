//! Strict, owned projection of the OpenType `MATH` table.

use std::collections::{BTreeMap, BTreeSet};

use super::FontParseError;

/// The 51 `MathValueRecord` constants, in OpenType wire order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum MathConstant {
    MathLeading,
    AxisHeight,
    AccentBaseHeight,
    FlattenedAccentBaseHeight,
    SubscriptShiftDown,
    SubscriptTopMax,
    SubscriptBaselineDropMin,
    SuperscriptShiftUp,
    SuperscriptShiftUpCramped,
    SuperscriptBottomMin,
    SuperscriptBaselineDropMax,
    SubSuperscriptGapMin,
    SuperscriptBottomMaxWithSubscript,
    SpaceAfterScript,
    UpperLimitGapMin,
    UpperLimitBaselineRiseMin,
    LowerLimitGapMin,
    LowerLimitBaselineDropMin,
    StackTopShiftUp,
    StackTopDisplayStyleShiftUp,
    StackBottomShiftDown,
    StackBottomDisplayStyleShiftDown,
    StackGapMin,
    StackDisplayStyleGapMin,
    StretchStackTopShiftUp,
    StretchStackBottomShiftDown,
    StretchStackGapAboveMin,
    StretchStackGapBelowMin,
    FractionNumeratorShiftUp,
    FractionNumeratorDisplayStyleShiftUp,
    FractionDenominatorShiftDown,
    FractionDenominatorDisplayStyleShiftDown,
    FractionNumeratorGapMin,
    FractionNumeratorDisplayStyleGapMin,
    FractionRuleThickness,
    FractionDenominatorGapMin,
    FractionDenominatorDisplayStyleGapMin,
    SkewedFractionHorizontalGap,
    SkewedFractionVerticalGap,
    OverbarVerticalGap,
    OverbarRuleThickness,
    OverbarExtraAscender,
    UnderbarVerticalGap,
    UnderbarRuleThickness,
    UnderbarExtraDescender,
    RadicalVerticalGap,
    RadicalDisplayStyleVerticalGap,
    RadicalRuleThickness,
    RadicalExtraAscender,
    RadicalKernBeforeDegree,
    RadicalKernAfterDegree,
}

/// A decoded OpenType device table or variation-index adjustment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MathAdjustment {
    Device {
        start_size: u16,
        end_size: u16,
        delta_format: u16,
        deltas: Vec<i8>,
    },
    VariationIndex {
        outer_index: u16,
        inner_index: u16,
    },
}

/// An OpenType MathValueRecord, in font design units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathValue {
    pub value: i16,
    pub adjustment: Option<MathAdjustment>,
}

/// Complete MathConstants data without a TeX-fontdimen projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathConstants {
    pub script_percent_scale_down: i16,
    pub script_script_percent_scale_down: i16,
    pub delimited_sub_formula_min_height: u16,
    pub display_operator_min_height: u16,
    values: [MathValue; 51],
    pub radical_degree_bottom_raise_percent: i16,
}

impl MathConstants {
    #[must_use]
    pub fn value(&self, constant: MathConstant) -> &MathValue {
        &self.values[constant as usize]
    }

    #[must_use]
    pub fn values(&self) -> &[MathValue; 51] {
        &self.values
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathKern {
    pub correction_heights: Vec<MathValue>,
    pub kern_values: Vec<MathValue>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathKernInfo {
    pub top_right: Option<MathKern>,
    pub top_left: Option<MathKern>,
    pub bottom_right: Option<MathKern>,
    pub bottom_left: Option<MathKern>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathGlyphInfo {
    pub italic_corrections: BTreeMap<u16, MathValue>,
    pub top_accent_attachments: BTreeMap<u16, MathValue>,
    pub extended_shapes: BTreeSet<u16>,
    pub kern_info: BTreeMap<u16, MathKernInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathGlyphVariant {
    pub glyph_id: u16,
    pub advance_measurement: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathGlyphPart {
    pub glyph_id: u16,
    pub start_connector_length: u16,
    pub end_connector_length: u16,
    pub full_advance: u16,
    pub extender: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathGlyphAssembly {
    pub italic_correction: MathValue,
    pub parts: Vec<MathGlyphPart>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathGlyphConstruction {
    pub assembly: Option<MathGlyphAssembly>,
    pub variants: Vec<MathGlyphVariant>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MathVariants {
    pub min_connector_overlap: u16,
    pub vertical: BTreeMap<u16, MathGlyphConstruction>,
    pub horizontal: BTreeMap<u16, MathGlyphConstruction>,
}

/// Immutable, validated OpenType MATH semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MathTables {
    pub constants: MathConstants,
    pub glyph_info: Option<MathGlyphInfo>,
    pub variants: Option<MathVariants>,
}

pub(super) fn parse_math(
    data: &[u8],
    glyph_count: u16,
    record_limit: usize,
    part_limit: usize,
) -> Result<MathTables, FontParseError> {
    if read_u32(data, 0)? != 0x0001_0000 {
        return Err(invalid("unsupported MATH version"));
    }
    let constants_offset = required_offset(data, 4, data.len(), "MathConstants")?;
    let glyph_info_offset = optional_offset(data, 6, data.len())?;
    let variants_offset = optional_offset(data, 8, data.len())?;
    require_separate_subtable(constants_offset, 10)?;
    if let Some(offset) = glyph_info_offset {
        require_separate_subtable(offset, 10)?;
    }
    if let Some(offset) = variants_offset {
        require_separate_subtable(offset, 10)?;
    }
    let mut budget = Budget::new(record_limit, part_limit);
    let constants = parse_constants(data, constants_offset, &mut budget)?;
    let glyph_info = glyph_info_offset
        .map(|offset| parse_glyph_info(data, offset, glyph_count, &mut budget))
        .transpose()?;
    let variants = variants_offset
        .map(|offset| parse_variants(data, offset, glyph_count, &mut budget))
        .transpose()?;
    Ok(MathTables {
        constants,
        glyph_info,
        variants,
    })
}

struct Budget {
    records: usize,
    record_limit: usize,
    parts: usize,
    part_limit: usize,
}

impl Budget {
    fn new(record_limit: usize, part_limit: usize) -> Self {
        Self {
            records: 0,
            record_limit,
            parts: 0,
            part_limit,
        }
    }

    fn records(&mut self, count: usize) -> Result<(), FontParseError> {
        self.records = self
            .records
            .checked_add(count)
            .ok_or(FontParseError::ArithmeticOverflow)?;
        if self.records > self.record_limit {
            return Err(FontParseError::LimitExceeded {
                resource: "MATH records",
                limit: self.record_limit,
                attempted: self.records,
            });
        }
        Ok(())
    }

    fn parts(&mut self, count: usize) -> Result<(), FontParseError> {
        self.parts = self
            .parts
            .checked_add(count)
            .ok_or(FontParseError::ArithmeticOverflow)?;
        if self.parts > self.part_limit {
            return Err(FontParseError::LimitExceeded {
                resource: "MATH assembly parts",
                limit: self.part_limit,
                attempted: self.parts,
            });
        }
        Ok(())
    }
}

fn parse_constants(
    data: &[u8],
    base: usize,
    budget: &mut Budget,
) -> Result<MathConstants, FontParseError> {
    checked_range(data, base, 214)?;
    budget.records(51)?;
    let mut values = Vec::with_capacity(51);
    for index in 0..51 {
        values.push(parse_value(data, base, 8 + index * 4, base + 214)?);
    }
    Ok(MathConstants {
        script_percent_scale_down: read_i16(data, base)?,
        script_script_percent_scale_down: read_i16(data, base + 2)?,
        delimited_sub_formula_min_height: read_u16(data, base + 4)?,
        display_operator_min_height: read_u16(data, base + 6)?,
        values: values
            .try_into()
            .map_err(|_| invalid("MathConstants length"))?,
        radical_degree_bottom_raise_percent: read_i16(data, base + 212)?,
    })
}

fn parse_glyph_info(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<MathGlyphInfo, FontParseError> {
    checked_range(data, base, 8)?;
    let child = |field| -> Result<Option<usize>, FontParseError> {
        let offset = optional_relative(data, base, field, data.len())?;
        if let Some(offset) = offset {
            require_separate_subtable(offset, base + 8)?;
        }
        Ok(offset)
    };
    let italic = child(base)?
        .map(|offset| parse_math_values(data, offset, glyph_count, budget))
        .transpose()?
        .unwrap_or_default();
    let accent = child(base + 2)?
        .map(|offset| parse_math_values(data, offset, glyph_count, budget))
        .transpose()?
        .unwrap_or_default();
    let extended = child(base + 4)?
        .map(|offset| parse_coverage(data, offset, glyph_count, budget))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .collect();
    let kern_info = child(base + 6)?
        .map(|offset| parse_kern_infos(data, offset, glyph_count, budget))
        .transpose()?
        .unwrap_or_default();
    Ok(MathGlyphInfo {
        italic_corrections: italic,
        top_accent_attachments: accent,
        extended_shapes: extended,
        kern_info,
    })
}

fn parse_math_values(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<BTreeMap<u16, MathValue>, FontParseError> {
    checked_range(data, base, 4)?;
    let coverage_offset = required_relative(data, base, base, data.len(), "MATH value coverage")?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 4)?)?;
    let records_end = base + 4 + count * 4;
    require_separate_subtable(coverage_offset, records_end)?;
    let coverage = parse_coverage(data, coverage_offset, glyph_count, budget)?;
    correspondence(coverage.len(), count)?;
    coverage
        .into_iter()
        .enumerate()
        .map(|(index, glyph)| Ok((glyph, parse_value(data, base, 4 + index * 4, records_end)?)))
        .collect()
}

fn parse_kern_infos(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<BTreeMap<u16, MathKernInfo>, FontParseError> {
    checked_range(data, base, 4)?;
    let coverage_offset = required_relative(data, base, base, data.len(), "MathKernInfo coverage")?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 8)?)?;
    let records_end = base + 4 + count * 8;
    require_separate_subtable(coverage_offset, records_end)?;
    let coverage = parse_coverage(data, coverage_offset, glyph_count, budget)?;
    correspondence(coverage.len(), count)?;
    let mut result = BTreeMap::new();
    for (index, glyph) in coverage.into_iter().enumerate() {
        let record = base + 4 + index * 8;
        let mut parse_corner = |field| {
            optional_relative(data, base, field, data.len())?
                .map(|offset| {
                    require_separate_subtable(offset, records_end)?;
                    parse_kern(data, offset, budget)
                })
                .transpose()
        };
        result.insert(
            glyph,
            MathKernInfo {
                top_right: parse_corner(record)?,
                top_left: parse_corner(record + 2)?,
                bottom_right: parse_corner(record + 4)?,
                bottom_left: parse_corner(record + 6)?,
            },
        );
    }
    Ok(result)
}

fn parse_kern(data: &[u8], base: usize, budget: &mut Budget) -> Result<MathKern, FontParseError> {
    let count = usize::from(read_u16(data, base)?);
    budget.records(
        count
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(FontParseError::ArithmeticOverflow)?,
    )?;
    checked_range(data, base + 2, checked_mul(count * 2 + 1, 4)?)?;
    let records_end = base + 2 + (count * 2 + 1) * 4;
    let mut heights = Vec::with_capacity(count);
    let mut kerns = Vec::with_capacity(count + 1);
    for index in 0..count {
        heights.push(parse_value(data, base, 2 + index * 4, records_end)?);
    }
    if heights
        .windows(2)
        .any(|pair| pair[0].value >= pair[1].value)
    {
        return Err(invalid("MathKern heights are not increasing"));
    }
    let kern_base = 2 + count * 4;
    for index in 0..=count {
        kerns.push(parse_value(data, base, kern_base + index * 4, records_end)?);
    }
    Ok(MathKern {
        correction_heights: heights,
        kern_values: kerns,
    })
}

fn parse_variants(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<MathVariants, FontParseError> {
    checked_range(data, base, 10)?;
    let vertical_coverage = optional_relative(data, base, base + 2, data.len())?;
    let horizontal_coverage = optional_relative(data, base, base + 4, data.len())?;
    let vertical_count = usize::from(read_u16(data, base + 6)?);
    let horizontal_count = usize::from(read_u16(data, base + 8)?);
    let offsets_base = base + 10;
    checked_range(
        data,
        offsets_base,
        checked_mul(vertical_count + horizontal_count, 2)?,
    )?;
    let records_end = offsets_base + (vertical_count + horizontal_count) * 2;
    if let Some(offset) = vertical_coverage {
        require_separate_subtable(offset, records_end)?;
    }
    if let Some(offset) = horizontal_coverage {
        require_separate_subtable(offset, records_end)?;
    }
    let vertical = parse_constructions(
        data,
        base,
        ConstructionGroup {
            offsets_base,
            subtables_min: records_end,
            count: vertical_count,
            coverage_offset: vertical_coverage,
        },
        glyph_count,
        budget,
    )?;
    let horizontal = parse_constructions(
        data,
        base,
        ConstructionGroup {
            offsets_base: offsets_base + vertical_count * 2,
            subtables_min: records_end,
            count: horizontal_count,
            coverage_offset: horizontal_coverage,
        },
        glyph_count,
        budget,
    )?;
    Ok(MathVariants {
        min_connector_overlap: read_u16(data, base)?,
        vertical,
        horizontal,
    })
}

struct ConstructionGroup {
    offsets_base: usize,
    subtables_min: usize,
    count: usize,
    coverage_offset: Option<usize>,
}

fn parse_constructions(
    data: &[u8],
    variants_base: usize,
    group: ConstructionGroup,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<BTreeMap<u16, MathGlyphConstruction>, FontParseError> {
    let ConstructionGroup {
        offsets_base,
        subtables_min,
        count,
        coverage_offset,
    } = group;
    if (count == 0) != coverage_offset.is_none() {
        return Err(invalid("construction coverage/count mismatch"));
    }
    if count == 0 {
        return Ok(BTreeMap::new());
    }
    budget.records(count)?;
    let coverage = parse_coverage(
        data,
        coverage_offset.ok_or_else(|| invalid("missing construction coverage"))?,
        glyph_count,
        budget,
    )?;
    correspondence(coverage.len(), count)?;
    let mut result = BTreeMap::new();
    for (index, glyph) in coverage.into_iter().enumerate() {
        let offset = required_relative(
            data,
            variants_base,
            offsets_base + index * 2,
            data.len(),
            "MathGlyphConstruction",
        )?;
        require_separate_subtable(offset, subtables_min)?;
        result.insert(
            glyph,
            parse_construction(data, offset, glyph_count, budget)?,
        );
    }
    Ok(result)
}

fn parse_construction(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<MathGlyphConstruction, FontParseError> {
    checked_range(data, base, 4)?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 4)?)?;
    let records_end = base + 4 + count * 4;
    let assembly = optional_relative(data, base, base, data.len())?
        .map(|offset| {
            require_separate_subtable(offset, records_end)?;
            parse_assembly(data, offset, glyph_count, budget)
        })
        .transpose()?;
    let mut variants = Vec::with_capacity(count);
    for index in 0..count {
        let at = base + 4 + index * 4;
        let glyph_id = checked_glyph(read_u16(data, at)?, glyph_count)?;
        variants.push(MathGlyphVariant {
            glyph_id,
            advance_measurement: read_u16(data, at + 2)?,
        });
    }
    if variants
        .windows(2)
        .any(|pair| pair[0].advance_measurement >= pair[1].advance_measurement)
    {
        return Err(invalid("variant advances are not increasing"));
    }
    if assembly.is_none() && variants.is_empty() {
        return Err(invalid("empty MathGlyphConstruction"));
    }
    Ok(MathGlyphConstruction { assembly, variants })
}

fn parse_assembly(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<MathGlyphAssembly, FontParseError> {
    checked_range(data, base, 6)?;
    let count = usize::from(read_u16(data, base + 4)?);
    if count == 0 {
        return Err(invalid("empty GlyphAssembly"));
    }
    budget.parts(count)?;
    checked_range(data, base + 6, checked_mul(count, 10)?)?;
    let records_end = base + 6 + count * 10;
    let italic_correction = parse_value(data, base, 0, records_end)?;
    let mut parts = Vec::with_capacity(count);
    for index in 0..count {
        let at = base + 6 + index * 10;
        let flags = read_u16(data, at + 8)?;
        if flags & !1 != 0 {
            return Err(invalid("reserved GlyphPart flags"));
        }
        let part = MathGlyphPart {
            glyph_id: checked_glyph(read_u16(data, at)?, glyph_count)?,
            start_connector_length: read_u16(data, at + 2)?,
            end_connector_length: read_u16(data, at + 4)?,
            full_advance: read_u16(data, at + 6)?,
            extender: flags == 1,
        };
        parts.push(part);
    }
    Ok(MathGlyphAssembly {
        italic_correction,
        parts,
    })
}

fn parse_value(
    data: &[u8],
    parent: usize,
    relative: usize,
    child_min: usize,
) -> Result<MathValue, FontParseError> {
    let at = parent
        .checked_add(relative)
        .ok_or(FontParseError::ArithmeticOverflow)?;
    let value = read_i16(data, at)?;
    let adjustment = optional_relative(data, parent, at + 2, data.len())?
        .map(|offset| {
            require_separate_subtable(offset, child_min)?;
            parse_adjustment(data, offset)
        })
        .transpose()?;
    Ok(MathValue { value, adjustment })
}

fn parse_adjustment(data: &[u8], base: usize) -> Result<MathAdjustment, FontParseError> {
    checked_range(data, base, 6)?;
    let first = read_u16(data, base)?;
    let second = read_u16(data, base + 2)?;
    let format = read_u16(data, base + 4)?;
    if format == 0x8000 {
        return Ok(MathAdjustment::VariationIndex {
            outer_index: first,
            inner_index: second,
        });
    }
    let bits = match format {
        1 => 2,
        2 => 4,
        3 => 8,
        _ => return Err(invalid("invalid device delta format")),
    };
    if first > second {
        return Err(invalid("device start size exceeds end size"));
    }
    let count = usize::from(second - first) + 1;
    let per_word = 16 / bits;
    let words = count.div_ceil(per_word);
    checked_range(data, base + 6, checked_mul(words, 2)?)?;
    let mut deltas = Vec::with_capacity(count);
    let mask = (1_u16 << bits) - 1;
    for index in 0..count {
        let word = read_u16(data, base + 6 + (index / per_word) * 2)?;
        let shift = 16 - bits * ((index % per_word) + 1);
        let raw = ((word >> shift) & mask) as i16;
        let signed = if raw & (1 << (bits - 1)) != 0 {
            raw - (1 << bits)
        } else {
            raw
        };
        deltas.push(signed as i8);
    }
    Ok(MathAdjustment::Device {
        start_size: first,
        end_size: second,
        delta_format: format,
        deltas,
    })
}

fn parse_coverage(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<Vec<u16>, FontParseError> {
    let format = read_u16(data, base)?;
    let count = usize::from(read_u16(data, base + 2)?);
    let mut glyphs = Vec::new();
    match format {
        1 => {
            budget.records(count)?;
            checked_range(data, base + 4, checked_mul(count, 2)?)?;
            for index in 0..count {
                glyphs.push(checked_glyph(
                    read_u16(data, base + 4 + index * 2)?,
                    glyph_count,
                )?);
            }
            if glyphs.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(invalid("coverage glyphs are not sorted and unique"));
            }
        }
        2 => {
            checked_range(data, base + 4, checked_mul(count, 6)?)?;
            let mut expected_index = 0_usize;
            let mut previous_end = None;
            for index in 0..count {
                let at = base + 4 + index * 6;
                let start = checked_glyph(read_u16(data, at)?, glyph_count)?;
                let end = checked_glyph(read_u16(data, at + 2)?, glyph_count)?;
                if start > end || previous_end.is_some_and(|value| start <= value) {
                    return Err(invalid("invalid coverage ranges"));
                }
                if usize::from(read_u16(data, at + 4)?) != expected_index {
                    return Err(invalid("invalid coverage start index"));
                }
                let range_count = usize::from(end - start) + 1;
                budget.records(range_count)?;
                expected_index = expected_index
                    .checked_add(range_count)
                    .ok_or(FontParseError::ArithmeticOverflow)?;
                glyphs.extend(start..=end);
                previous_end = Some(end);
            }
        }
        _ => return Err(invalid("invalid coverage format")),
    }
    Ok(glyphs)
}

fn correspondence(coverage: usize, records: usize) -> Result<(), FontParseError> {
    if coverage != records {
        Err(invalid("MATH coverage/record count mismatch"))
    } else {
        Ok(())
    }
}

fn require_separate_subtable(offset: usize, records_end: usize) -> Result<(), FontParseError> {
    if offset < records_end {
        Err(invalid("cyclic or overlapping MATH offset graph"))
    } else {
        Ok(())
    }
}

fn checked_glyph(glyph: u16, glyph_count: u16) -> Result<u16, FontParseError> {
    if glyph < glyph_count {
        Ok(glyph)
    } else {
        Err(invalid("MATH glyph id out of range"))
    }
}

fn required_offset(
    data: &[u8],
    at: usize,
    limit: usize,
    name: &'static str,
) -> Result<usize, FontParseError> {
    let offset = usize::from(read_u16(data, at)?);
    if offset == 0 || offset >= limit {
        Err(invalid(name))
    } else {
        Ok(offset)
    }
}

fn optional_offset(data: &[u8], at: usize, limit: usize) -> Result<Option<usize>, FontParseError> {
    let offset = usize::from(read_u16(data, at)?);
    if offset == 0 {
        Ok(None)
    } else if offset < limit {
        Ok(Some(offset))
    } else {
        Err(invalid("MATH offset out of range"))
    }
}

fn required_relative(
    data: &[u8],
    parent: usize,
    at: usize,
    limit: usize,
    name: &'static str,
) -> Result<usize, FontParseError> {
    optional_relative(data, parent, at, limit)?.ok_or_else(|| invalid(name))
}

fn optional_relative(
    data: &[u8],
    parent: usize,
    at: usize,
    limit: usize,
) -> Result<Option<usize>, FontParseError> {
    let offset = usize::from(read_u16(data, at)?);
    if offset == 0 {
        return Ok(None);
    }
    let absolute = parent
        .checked_add(offset)
        .ok_or(FontParseError::ArithmeticOverflow)?;
    if absolute >= limit {
        Err(invalid("MATH relative offset out of range"))
    } else {
        Ok(Some(absolute))
    }
}

fn read_u16(data: &[u8], at: usize) -> Result<u16, FontParseError> {
    let bytes: [u8; 2] = checked_range(data, at, 2)?
        .try_into()
        .map_err(|_| invalid("truncated MATH table"))?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_i16(data: &[u8], at: usize) -> Result<i16, FontParseError> {
    Ok(read_u16(data, at)? as i16)
}

fn read_u32(data: &[u8], at: usize) -> Result<u32, FontParseError> {
    let bytes: [u8; 4] = checked_range(data, at, 4)?
        .try_into()
        .map_err(|_| invalid("truncated MATH table"))?;
    Ok(u32::from_be_bytes(bytes))
}

fn checked_range(data: &[u8], at: usize, len: usize) -> Result<&[u8], FontParseError> {
    let end = at
        .checked_add(len)
        .ok_or(FontParseError::ArithmeticOverflow)?;
    data.get(at..end)
        .ok_or_else(|| invalid("truncated MATH table"))
}

fn checked_mul(left: usize, right: usize) -> Result<usize, FontParseError> {
    left.checked_mul(right)
        .ok_or(FontParseError::ArithmeticOverflow)
}

fn invalid(detail: &'static str) -> FontParseError {
    FontParseError::InvalidMath(detail)
}

#[cfg(test)]
mod tests;

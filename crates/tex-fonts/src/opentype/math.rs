//! Strict eager validation for the lazily queried OpenType `MATH` table.

use super::FontParseError;

macro_rules! math_constants {
    ($($variant:ident => $query:ident),+ $(,)?) => {
        /// The 51 `MathValueRecord` constants, in OpenType wire order.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum MathConstant {
            $($variant),+
        }

        impl MathConstant {
            pub(crate) fn query<'a>(
                self,
                constants: ttf_parser::math::Constants<'a>,
            ) -> ttf_parser::math::MathValue<'a> {
                match self {
                    $(Self::$variant => constants.$query()),+
                }
            }
        }
    };
}

math_constants! {
    MathLeading => math_leading,
    AxisHeight => axis_height,
    AccentBaseHeight => accent_base_height,
    FlattenedAccentBaseHeight => flattened_accent_base_height,
    SubscriptShiftDown => subscript_shift_down,
    SubscriptTopMax => subscript_top_max,
    SubscriptBaselineDropMin => subscript_baseline_drop_min,
    SuperscriptShiftUp => superscript_shift_up,
    SuperscriptShiftUpCramped => superscript_shift_up_cramped,
    SuperscriptBottomMin => superscript_bottom_min,
    SuperscriptBaselineDropMax => superscript_baseline_drop_max,
    SubSuperscriptGapMin => sub_superscript_gap_min,
    SuperscriptBottomMaxWithSubscript => superscript_bottom_max_with_subscript,
    SpaceAfterScript => space_after_script,
    UpperLimitGapMin => upper_limit_gap_min,
    UpperLimitBaselineRiseMin => upper_limit_baseline_rise_min,
    LowerLimitGapMin => lower_limit_gap_min,
    LowerLimitBaselineDropMin => lower_limit_baseline_drop_min,
    StackTopShiftUp => stack_top_shift_up,
    StackTopDisplayStyleShiftUp => stack_top_display_style_shift_up,
    StackBottomShiftDown => stack_bottom_shift_down,
    StackBottomDisplayStyleShiftDown => stack_bottom_display_style_shift_down,
    StackGapMin => stack_gap_min,
    StackDisplayStyleGapMin => stack_display_style_gap_min,
    StretchStackTopShiftUp => stretch_stack_top_shift_up,
    StretchStackBottomShiftDown => stretch_stack_bottom_shift_down,
    StretchStackGapAboveMin => stretch_stack_gap_above_min,
    StretchStackGapBelowMin => stretch_stack_gap_below_min,
    FractionNumeratorShiftUp => fraction_numerator_shift_up,
    FractionNumeratorDisplayStyleShiftUp => fraction_numerator_display_style_shift_up,
    FractionDenominatorShiftDown => fraction_denominator_shift_down,
    FractionDenominatorDisplayStyleShiftDown => fraction_denominator_display_style_shift_down,
    FractionNumeratorGapMin => fraction_numerator_gap_min,
    FractionNumeratorDisplayStyleGapMin => fraction_num_display_style_gap_min,
    FractionRuleThickness => fraction_rule_thickness,
    FractionDenominatorGapMin => fraction_denominator_gap_min,
    FractionDenominatorDisplayStyleGapMin => fraction_denom_display_style_gap_min,
    SkewedFractionHorizontalGap => skewed_fraction_horizontal_gap,
    SkewedFractionVerticalGap => skewed_fraction_vertical_gap,
    OverbarVerticalGap => overbar_vertical_gap,
    OverbarRuleThickness => overbar_rule_thickness,
    OverbarExtraAscender => overbar_extra_ascender,
    UnderbarVerticalGap => underbar_vertical_gap,
    UnderbarRuleThickness => underbar_rule_thickness,
    UnderbarExtraDescender => underbar_extra_descender,
    RadicalVerticalGap => radical_vertical_gap,
    RadicalDisplayStyleVerticalGap => radical_display_style_vertical_gap,
    RadicalRuleThickness => radical_rule_thickness,
    RadicalExtraAscender => radical_extra_ascender,
    RadicalKernBeforeDegree => radical_kern_before_degree,
    RadicalKernAfterDegree => radical_kern_after_degree,
}

pub(super) fn validate_math(
    data: &[u8],
    glyph_count: u16,
    record_limit: usize,
    part_limit: usize,
) -> Result<(), FontParseError> {
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
    validate_constants(data, constants_offset, &mut budget)?;
    if let Some(offset) = glyph_info_offset {
        validate_glyph_info(data, offset, glyph_count, &mut budget)?;
    }
    if let Some(offset) = variants_offset {
        validate_variants(data, offset, glyph_count, &mut budget)?;
    }
    ttf_parser::math::Table::parse(data).ok_or_else(|| invalid("invalid MATH table"))?;
    Ok(())
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

fn validate_constants(data: &[u8], base: usize, budget: &mut Budget) -> Result<(), FontParseError> {
    checked_range(data, base, 214)?;
    budget.records(51)?;
    for index in 0..51 {
        validate_value(data, base, 8 + index * 4, base + 214)?;
    }
    Ok(())
}

fn validate_glyph_info(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
    checked_range(data, base, 8)?;
    let child = |field| -> Result<Option<usize>, FontParseError> {
        let offset = optional_relative(data, base, field, data.len())?;
        if let Some(offset) = offset {
            require_separate_subtable(offset, base + 8)?;
        }
        Ok(offset)
    };
    if let Some(offset) = child(base)? {
        validate_math_values(data, offset, glyph_count, budget)?;
    }
    if let Some(offset) = child(base + 2)? {
        validate_math_values(data, offset, glyph_count, budget)?;
    }
    if let Some(offset) = child(base + 4)? {
        validate_coverage(data, offset, glyph_count, budget)?;
    }
    if let Some(offset) = child(base + 6)? {
        validate_kern_infos(data, offset, glyph_count, budget)?;
    }
    Ok(())
}

fn validate_math_values(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
    checked_range(data, base, 4)?;
    let coverage_offset = required_relative(data, base, base, data.len(), "MATH value coverage")?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 4)?)?;
    let records_end = base + 4 + count * 4;
    require_separate_subtable(coverage_offset, records_end)?;
    let coverage = validate_coverage(data, coverage_offset, glyph_count, budget)?;
    correspondence(coverage.len(), count)?;
    for index in 0..count {
        validate_value(data, base, 4 + index * 4, records_end)?;
    }
    Ok(())
}

fn validate_kern_infos(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
    checked_range(data, base, 4)?;
    let coverage_offset = required_relative(data, base, base, data.len(), "MathKernInfo coverage")?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 8)?)?;
    let records_end = base + 4 + count * 8;
    require_separate_subtable(coverage_offset, records_end)?;
    let coverage = validate_coverage(data, coverage_offset, glyph_count, budget)?;
    correspondence(coverage.len(), count)?;
    for index in 0..count {
        let record = base + 4 + index * 8;
        for field in [record, record + 2, record + 4, record + 6] {
            if let Some(offset) = optional_relative(data, base, field, data.len())? {
                require_separate_subtable(offset, records_end)?;
                validate_kern(data, offset, budget)?;
            }
        }
    }
    Ok(())
}

fn validate_kern(data: &[u8], base: usize, budget: &mut Budget) -> Result<(), FontParseError> {
    let count = usize::from(read_u16(data, base)?);
    budget.records(
        count
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or(FontParseError::ArithmeticOverflow)?,
    )?;
    checked_range(data, base + 2, checked_mul(count * 2 + 1, 4)?)?;
    let records_end = base + 2 + (count * 2 + 1) * 4;
    let mut previous_height = None;
    for index in 0..count {
        let at = base + 2 + index * 4;
        let height = read_i16(data, at)?;
        validate_value(data, base, 2 + index * 4, records_end)?;
        if previous_height.is_some_and(|previous| previous >= height) {
            return Err(invalid("MathKern heights are not increasing"));
        }
        previous_height = Some(height);
    }
    let kern_base = 2 + count * 4;
    for index in 0..=count {
        validate_value(data, base, kern_base + index * 4, records_end)?;
    }
    Ok(())
}

fn validate_variants(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
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
    validate_constructions(
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
    validate_constructions(
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
    Ok(())
}

struct ConstructionGroup {
    offsets_base: usize,
    subtables_min: usize,
    count: usize,
    coverage_offset: Option<usize>,
}

fn validate_constructions(
    data: &[u8],
    variants_base: usize,
    group: ConstructionGroup,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
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
        return Ok(());
    }
    budget.records(count)?;
    let coverage = validate_coverage(
        data,
        coverage_offset.ok_or_else(|| invalid("missing construction coverage"))?,
        glyph_count,
        budget,
    )?;
    correspondence(coverage.len(), count)?;
    for index in 0..count {
        let offset = required_relative(
            data,
            variants_base,
            offsets_base + index * 2,
            data.len(),
            "MathGlyphConstruction",
        )?;
        require_separate_subtable(offset, subtables_min)?;
        validate_construction(data, offset, glyph_count, budget)?;
    }
    Ok(())
}

fn validate_construction(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
    checked_range(data, base, 4)?;
    let count = usize::from(read_u16(data, base + 2)?);
    budget.records(count)?;
    checked_range(data, base + 4, checked_mul(count, 4)?)?;
    let records_end = base + 4 + count * 4;
    let assembly = optional_relative(data, base, base, data.len())?;
    if let Some(offset) = assembly {
        require_separate_subtable(offset, records_end)?;
        validate_assembly(data, offset, glyph_count, budget)?;
    }
    let mut previous_advance = None;
    for index in 0..count {
        let at = base + 4 + index * 4;
        checked_glyph(read_u16(data, at)?, glyph_count)?;
        let advance = read_u16(data, at + 2)?;
        if previous_advance.is_some_and(|previous| previous >= advance) {
            return Err(invalid("variant advances are not increasing"));
        }
        previous_advance = Some(advance);
    }
    if assembly.is_none() && count == 0 {
        return Err(invalid("empty MathGlyphConstruction"));
    }
    Ok(())
}

fn validate_assembly(
    data: &[u8],
    base: usize,
    glyph_count: u16,
    budget: &mut Budget,
) -> Result<(), FontParseError> {
    checked_range(data, base, 6)?;
    let count = usize::from(read_u16(data, base + 4)?);
    if count == 0 {
        return Err(invalid("empty GlyphAssembly"));
    }
    budget.parts(count)?;
    checked_range(data, base + 6, checked_mul(count, 10)?)?;
    let records_end = base + 6 + count * 10;
    validate_value(data, base, 0, records_end)?;
    for index in 0..count {
        let at = base + 6 + index * 10;
        let flags = read_u16(data, at + 8)?;
        if flags & !1 != 0 {
            return Err(invalid("reserved GlyphPart flags"));
        }
        checked_glyph(read_u16(data, at)?, glyph_count)?;
    }
    Ok(())
}

fn validate_value(
    data: &[u8],
    parent: usize,
    relative: usize,
    child_min: usize,
) -> Result<(), FontParseError> {
    let at = parent
        .checked_add(relative)
        .ok_or(FontParseError::ArithmeticOverflow)?;
    read_i16(data, at)?;
    if let Some(offset) = optional_relative(data, parent, at + 2, data.len())? {
        require_separate_subtable(offset, child_min)?;
        validate_adjustment(data, offset)?;
    }
    Ok(())
}

fn validate_adjustment(data: &[u8], base: usize) -> Result<(), FontParseError> {
    checked_range(data, base, 6)?;
    let first = read_u16(data, base)?;
    let second = read_u16(data, base + 2)?;
    let format = read_u16(data, base + 4)?;
    if format == 0x8000 {
        return Ok(());
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
    Ok(())
}

fn validate_coverage(
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

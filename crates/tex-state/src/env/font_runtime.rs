//! Direct-index mutable per-font TeX and pdfTeX state.

use crate::env::banks::{BankCell, BankError};
use crate::font::PdfFontCode;
use crate::scaled::Scaled;
use crate::state_hash::StateHasher;

const PDF_CODE_TABLES: usize = 9;
const PDF_CODE_COUNT: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FontRuntimeCell {
    ParameterCount(u32),
    Dimen { font: u32, number: u32 },
    HyphenChar(u32),
    SkewChar(u32),
    PdfCode { table: u8, font: u32, code: u8 },
    LigaturesDisabled(u32),
}

pub(crate) struct DerivedFontRuntimeRequest<'a, F> {
    pub source: F,
    pub parameters: &'a [Scaled],
    pub preserve_character_settings: bool,
    pub preserve_pdf_settings: bool,
    pub disable_ligatures: bool,
    pub default_hyphen_char: i32,
    pub default_skew_char: i32,
}

pub(crate) struct PreparedFontRuntime {
    row: FontRuntimeRow,
}

#[derive(Clone)]
struct FontRuntimeRow {
    parameter_count: BankCell<i32>,
    parameters: Vec<BankCell<Scaled>>,
    hyphen_char: BankCell<i32>,
    skew_char: BankCell<i32>,
    pdf_codes: [Option<Box<[BankCell<i32>; PDF_CODE_COUNT]>>; PDF_CODE_TABLES],
    ligatures_disabled: BankCell<i32>,
}

#[derive(Clone)]
pub(crate) struct FontRuntimeBank {
    rows: Vec<FontRuntimeRow>,
}

pub(super) struct AcceptedFontRuntimeTail {
    rows: Vec<FontRuntimeRow>,
}

impl FontRuntimeBank {
    pub(crate) const fn new() -> Self {
        Self { rows: Vec::new() }
    }

    pub(crate) fn capture_format(
        &self,
        font: u32,
    ) -> Result<crate::format::schema::FormatFontRuntime, BankError> {
        let row = self.row(font)?;
        let parameter_count =
            usize::try_from(row.parameter_count.value).map_err(|_| BankError::IndexOutOfBounds)?;
        if parameter_count > row.parameters.len() {
            return Err(BankError::IndexOutOfBounds);
        }
        Ok(crate::format::schema::FormatFontRuntime {
            parameters: row.parameters[..parameter_count]
                .iter()
                .map(|cell| cell.value.raw())
                .collect(),
            hyphen_char: row.hyphen_char.value,
            skew_char: row.skew_char.value,
            pdf_codes: row
                .pdf_codes
                .iter()
                .map(|table| {
                    table
                        .as_ref()
                        .map(|values| values.iter().map(|cell| cell.value).collect())
                })
                .collect(),
            ligatures_disabled: row.ligatures_disabled.value != 0,
        })
    }

    pub(crate) fn install_format(
        &mut self,
        font: u32,
        format: &crate::format::schema::FormatFontRuntime,
    ) -> Result<(), BankError> {
        if format.pdf_codes.len() != PDF_CODE_TABLES
            || format
                .pdf_codes
                .iter()
                .flatten()
                .any(|values| values.len() != PDF_CODE_COUNT)
        {
            return Err(BankError::IndexOutOfBounds);
        }
        let mut prepared = self.prepare(
            &format
                .parameters
                .iter()
                .copied()
                .map(Scaled::from_raw)
                .collect::<Vec<_>>(),
            format.hyphen_char,
            format.skew_char,
        )?;
        prepared.row.pdf_codes = core::array::from_fn(|table| {
            format.pdf_codes[table].as_ref().map(|values| {
                values
                    .iter()
                    .copied()
                    .map(BankCell::level_one)
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
                    .try_into()
                    .expect("validated PDF font-code table has 256 entries")
            })
        });
        prepared.row.ligatures_disabled = BankCell::level_one(i32::from(format.ligatures_disabled));
        self.install(font, prepared)
    }

    pub(crate) fn prepare(
        &mut self,
        parameters: &[Scaled],
        hyphen_char: i32,
        skew_char: i32,
    ) -> Result<PreparedFontRuntime, BankError> {
        self.rows
            .try_reserve(1)
            .map_err(|_| BankError::AllocationFailed)?;
        let mut runtime_parameters = Vec::new();
        runtime_parameters
            .try_reserve_exact(parameters.len())
            .map_err(|_| BankError::AllocationFailed)?;
        runtime_parameters.extend(parameters.iter().copied().map(BankCell::level_one));
        Ok(PreparedFontRuntime {
            row: FontRuntimeRow {
                parameter_count: BankCell::level_one(
                    i32::try_from(parameters.len()).unwrap_or(i32::MAX),
                ),
                parameters: runtime_parameters,
                hyphen_char: BankCell::level_one(hyphen_char),
                skew_char: BankCell::level_one(skew_char),
                pdf_codes: core::array::from_fn(|_| None),
                ligatures_disabled: BankCell::level_one(0),
            },
        })
    }

    pub(crate) fn prepare_derived(
        &mut self,
        request: DerivedFontRuntimeRequest<'_, u32>,
    ) -> Result<PreparedFontRuntime, BankError> {
        let DerivedFontRuntimeRequest {
            source,
            parameters,
            preserve_character_settings,
            preserve_pdf_settings,
            disable_ligatures,
            default_hyphen_char,
            default_skew_char,
        } = request;
        self.rows
            .try_reserve(1)
            .map_err(|_| BankError::AllocationFailed)?;
        let source = self.row(source)?;
        let (hyphen_char, skew_char) = if preserve_character_settings {
            (source.hyphen_char.value, source.skew_char.value)
        } else {
            (default_hyphen_char, default_skew_char)
        };
        let mut runtime_parameters = Vec::new();
        runtime_parameters
            .try_reserve_exact(parameters.len())
            .map_err(|_| BankError::AllocationFailed)?;
        runtime_parameters.extend(parameters.iter().copied().map(BankCell::level_one));
        let pdf_codes = if preserve_pdf_settings {
            let mut tables = core::array::from_fn(|_| None);
            for (destination, source) in tables.iter_mut().zip(&source.pdf_codes) {
                let Some(source) = source else {
                    continue;
                };
                let mut values = Vec::new();
                values
                    .try_reserve_exact(PDF_CODE_COUNT)
                    .map_err(|_| BankError::AllocationFailed)?;
                values.extend(source.iter().map(|cell| BankCell::level_one(cell.value)));
                let values: Box<[BankCell<i32>; PDF_CODE_COUNT]> = values
                    .into_boxed_slice()
                    .try_into()
                    .map_err(|_| BankError::IndexOutOfBounds)?;
                *destination = Some(values);
            }
            tables
        } else {
            core::array::from_fn(|_| None)
        };
        Ok(PreparedFontRuntime {
            row: FontRuntimeRow {
                parameter_count: BankCell::level_one(
                    i32::try_from(parameters.len()).unwrap_or(i32::MAX),
                ),
                parameters: runtime_parameters,
                hyphen_char: BankCell::level_one(hyphen_char),
                skew_char: BankCell::level_one(skew_char),
                pdf_codes,
                ligatures_disabled: BankCell::level_one(i32::from(
                    disable_ligatures
                        || preserve_pdf_settings && source.ligatures_disabled.value != 0,
                )),
            },
        })
    }

    pub(crate) fn install(
        &mut self,
        font: u32,
        prepared: PreparedFontRuntime,
    ) -> Result<(), BankError> {
        if font as usize != self.rows.len() {
            return Err(BankError::IndexOutOfBounds);
        }
        self.rows.push(prepared.row);
        Ok(())
    }

    pub(crate) fn parameter_count(&self, font: u32) -> Result<u32, BankError> {
        let count = self.row(font)?.parameter_count.value;
        u32::try_from(count).map_err(|_| BankError::IndexOutOfBounds)
    }

    pub(crate) fn parameter_words(&self) -> usize {
        self.rows
            .iter()
            .map(|row| usize::try_from(row.parameter_count.value).unwrap_or(usize::MAX))
            .fold(0, usize::saturating_add)
    }

    pub(super) fn cursor(&self) -> usize {
        self.rows.len()
    }

    pub(super) fn truncate(&mut self, cursor: usize) {
        assert!(cursor <= self.rows.len());
        self.rows.truncate(cursor);
    }

    pub(super) fn begin_checkpoint_candidate(&mut self, cursor: usize) -> AcceptedFontRuntimeTail {
        assert!(cursor <= self.rows.len());
        AcceptedFontRuntimeTail {
            rows: self.rows.split_off(cursor),
        }
    }

    pub(super) fn reject_checkpoint_candidate(
        &mut self,
        cursor: usize,
        mut tail: AcceptedFontRuntimeTail,
    ) {
        self.truncate(cursor);
        self.rows.append(&mut tail.rows);
    }

    pub(crate) fn prepare_dimen_growth(&mut self, font: u32, number: u32) -> Result<(), BankError> {
        let target = usize::try_from(number).map_err(|_| BankError::IndexOutOfBounds)?;
        let row = self.row_mut(font)?;
        if target <= row.parameters.len() {
            return Ok(());
        }
        row.parameters
            .try_reserve_exact(target - row.parameters.len())
            .map_err(|_| BankError::AllocationFailed)?;
        row.parameters
            .resize(target, BankCell::level_one(Scaled::from_raw(0)));
        Ok(())
    }

    pub(crate) fn ensure_pdf_table(
        &mut self,
        font: u32,
        table: PdfFontCode,
        defaults: [i32; PDF_CODE_COUNT],
    ) -> Result<(), BankError> {
        let row = self.row_mut(font)?;
        let table = table_index(table);
        if row.pdf_codes[table].is_none() {
            row.pdf_codes[table] = Some(Box::new(core::array::from_fn(|code| {
                BankCell::level_one(defaults[code])
            })));
        }
        Ok(())
    }

    pub(crate) fn read(&self, cell: FontRuntimeCell) -> Result<BankCellValue, BankError> {
        let value = match cell {
            FontRuntimeCell::ParameterCount(font) => {
                BankCellValue::Integer(self.row(font)?.parameter_count.clone())
            }
            FontRuntimeCell::Dimen { font, number } => {
                let index = number
                    .checked_sub(1)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or(BankError::IndexOutOfBounds)?;
                BankCellValue::Dimension(
                    self.row(font)?
                        .parameters
                        .get(index)
                        .ok_or(BankError::IndexOutOfBounds)?
                        .clone(),
                )
            }
            FontRuntimeCell::HyphenChar(font) => {
                BankCellValue::Integer(self.row(font)?.hyphen_char.clone())
            }
            FontRuntimeCell::SkewChar(font) => {
                BankCellValue::Integer(self.row(font)?.skew_char.clone())
            }
            FontRuntimeCell::PdfCode { table, font, code } => {
                let row = self.row(font)?;
                let values = row.pdf_codes[usize::from(table)]
                    .as_ref()
                    .ok_or(BankError::IndexOutOfBounds)?;
                BankCellValue::Integer(values[usize::from(code)].clone())
            }
            FontRuntimeCell::LigaturesDisabled(font) => {
                BankCellValue::Integer(self.row(font)?.ligatures_disabled.clone())
            }
        };
        Ok(value)
    }

    pub(crate) fn write(
        &mut self,
        cell: FontRuntimeCell,
        value: BankCellValue,
    ) -> Result<(), BankError> {
        match (cell, value) {
            (FontRuntimeCell::ParameterCount(font), BankCellValue::Integer(value)) => {
                self.row_mut(font)?.parameter_count = value;
            }
            (FontRuntimeCell::Dimen { font, number }, BankCellValue::Dimension(value)) => {
                let index = number
                    .checked_sub(1)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or(BankError::IndexOutOfBounds)?;
                *self
                    .row_mut(font)?
                    .parameters
                    .get_mut(index)
                    .ok_or(BankError::IndexOutOfBounds)? = value;
            }
            (FontRuntimeCell::HyphenChar(font), BankCellValue::Integer(value)) => {
                self.row_mut(font)?.hyphen_char = value;
            }
            (FontRuntimeCell::SkewChar(font), BankCellValue::Integer(value)) => {
                self.row_mut(font)?.skew_char = value;
            }
            (FontRuntimeCell::PdfCode { table, font, code }, BankCellValue::Integer(value)) => {
                let values = self.row_mut(font)?.pdf_codes[usize::from(table)]
                    .as_mut()
                    .ok_or(BankError::IndexOutOfBounds)?;
                values[usize::from(code)] = value;
            }
            (FontRuntimeCell::LigaturesDisabled(font), BankCellValue::Integer(value)) => {
                self.row_mut(font)?.ligatures_disabled = value;
            }
            _ => return Err(BankError::IndexOutOfBounds),
        }
        Ok(())
    }

    pub(crate) fn allocated_pages(&self) -> usize {
        self.rows
            .iter()
            .map(|row| row.pdf_codes.iter().filter(|table| table.is_some()).count())
            .sum()
    }

    pub(crate) fn hash_semantic(
        &self,
        font: u32,
        loaded: &tex_fonts::LoadedFont,
        hasher: &mut StateHasher,
    ) -> Result<(), BankError> {
        let row = self.row(font)?;
        let parameter_count =
            usize::try_from(row.parameter_count.value).map_err(|_| BankError::IndexOutOfBounds)?;
        hasher.usize(parameter_count);
        for parameter in row.parameters.iter().take(parameter_count) {
            hasher.i32(parameter.value.raw());
        }
        hasher.i32(row.hyphen_char.value);
        hasher.i32(row.skew_char.value);
        hasher.u8(u8::from(row.ligatures_disabled.value != 0));
        for (table, values) in row.pdf_codes.iter().enumerate() {
            let Some(values) = values else {
                continue;
            };
            let table = table_from_index(table);
            let mut deviations = values.iter().enumerate().filter_map(|(code, value)| {
                let code = code as u8;
                (value.value != default_pdf_code(table, loaded, code))
                    .then_some((code, value.value))
            });
            let count = deviations.clone().count();
            if count == 0 {
                continue;
            }
            hasher.u8(table_index(table) as u8);
            hasher.usize(count);
            for (code, value) in &mut deviations {
                hasher.u8(code);
                hasher.i32(value);
            }
        }
        Ok(())
    }

    fn row(&self, font: u32) -> Result<&FontRuntimeRow, BankError> {
        self.rows
            .get(font as usize)
            .ok_or(BankError::IndexOutOfBounds)
    }

    fn row_mut(&mut self, font: u32) -> Result<&mut FontRuntimeRow, BankError> {
        self.rows
            .get_mut(font as usize)
            .ok_or(BankError::IndexOutOfBounds)
    }
}

#[derive(Clone)]
pub(crate) enum BankCellValue {
    Integer(BankCell<i32>),
    Dimension(BankCell<Scaled>),
}

pub(crate) const fn table_index(table: PdfFontCode) -> usize {
    match table {
        PdfFontCode::Lp => 0,
        PdfFontCode::Rp => 1,
        PdfFontCode::Ef => 2,
        PdfFontCode::Tag => 3,
        PdfFontCode::Knbs => 4,
        PdfFontCode::Stbs => 5,
        PdfFontCode::Shbs => 6,
        PdfFontCode::Knbc => 7,
        PdfFontCode::Knac => 8,
    }
}

const fn table_from_index(table: usize) -> PdfFontCode {
    match table {
        0 => PdfFontCode::Lp,
        1 => PdfFontCode::Rp,
        2 => PdfFontCode::Ef,
        3 => PdfFontCode::Tag,
        4 => PdfFontCode::Knbs,
        5 => PdfFontCode::Stbs,
        6 => PdfFontCode::Shbs,
        7 => PdfFontCode::Knbc,
        8 => PdfFontCode::Knac,
        _ => unreachable!(),
    }
}

fn default_pdf_code(table: PdfFontCode, loaded: &tex_fonts::LoadedFont, code: u8) -> i32 {
    match table {
        PdfFontCode::Ef => 1000,
        PdfFontCode::Tag => loaded
            .character_metrics(char::from(code))
            .map_or(0, |metrics| match metrics.tag {
                crate::font::CharTag::None => 0,
                crate::font::CharTag::LigKern { .. } => 1,
                crate::font::CharTag::NextLarger(_) => 2,
                crate::font::CharTag::Extensible(_) => 4,
            }),
        _ => 0,
    }
}

pub(crate) const fn table_cell(table: PdfFontCode, font: u32, code: u8) -> FontRuntimeCell {
    FontRuntimeCell::PdfCode {
        table: table_index(table) as u8,
        font,
        code,
    }
}

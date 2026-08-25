//! Packed coordinates stored by narrow save-journal records.

use crate::env::{CodeTableKind, FontRuntimeCell, StateCell};

const TAG_SHIFT: u32 = 58;
const PAYLOAD_MASK: u64 = (1_u64 << TAG_SHIFT) - 1;

#[derive(Clone, Copy)]
pub(super) struct JournalCell(u64);

impl JournalCell {
    pub(super) fn pack(cell: StateCell) -> Self {
        let (tag, payload) = match cell {
            StateCell::Meaning(index) => (0, u64::from(index)),
            StateCell::Count(index) => (1, u64::from(index)),
            StateCell::Dimension(index) => (2, u64::from(index)),
            StateCell::TokenRegister(index) => (3, u64::from(index)),
            StateCell::GlueRegister(index) => (4, u64::from(index)),
            StateCell::BoxRegister(index) => (5, u64::from(index)),
            StateCell::MuGlueRegister(index) => (6, u64::from(index)),
            StateCell::IntegerParameter(index) => (7, u64::from(index)),
            StateCell::DimensionParameter(index) => (8, u64::from(index)),
            StateCell::TokenParameter(index) => (9, u64::from(index)),
            StateCell::GlueParameter(index) => (10, u64::from(index)),
            StateCell::CurrentFont => (11, 0),
            StateCell::MathFamilyFont(index) => (12, u64::from(index)),
            StateCell::Code(kind, index) => (13, (u64::from(kind as u8) << 32) | u64::from(index)),
            StateCell::FontRuntime(FontRuntimeCell::ParameterCount(font)) => (14, u64::from(font)),
            StateCell::FontRuntime(FontRuntimeCell::Dimen { font, number }) => {
                (15, (u64::from(font) << 18) | u64::from(number))
            }
            StateCell::FontRuntime(FontRuntimeCell::HyphenChar(font)) => (16, u64::from(font)),
            StateCell::FontRuntime(FontRuntimeCell::SkewChar(font)) => (17, u64::from(font)),
            StateCell::FontRuntime(FontRuntimeCell::PdfCode { table, font, code }) => (
                18,
                (u64::from(table) << 40) | (u64::from(font) << 8) | u64::from(code),
            ),
            StateCell::FontRuntime(FontRuntimeCell::LigaturesDisabled(font)) => {
                (19, u64::from(font))
            }
        };
        debug_assert!(payload <= PAYLOAD_MASK);
        Self((tag << TAG_SHIFT) | payload)
    }

    pub(super) fn unpack(self) -> StateCell {
        let tag = self.0 >> TAG_SHIFT;
        let payload = self.0 & PAYLOAD_MASK;
        let low = payload as u32;
        match tag {
            0 => StateCell::Meaning(low),
            1 => StateCell::Count(low as u16),
            2 => StateCell::Dimension(low as u16),
            3 => StateCell::TokenRegister(low as u16),
            4 => StateCell::GlueRegister(low as u16),
            5 => StateCell::BoxRegister(low as u16),
            6 => StateCell::MuGlueRegister(low as u16),
            7 => StateCell::IntegerParameter(low as u16),
            8 => StateCell::DimensionParameter(low as u16),
            9 => StateCell::TokenParameter(low as u16),
            10 => StateCell::GlueParameter(low as u16),
            11 => StateCell::CurrentFont,
            12 => StateCell::MathFamilyFont(low as u8),
            13 => StateCell::Code(code_table_kind((payload >> 32) as u8), low),
            14 => StateCell::FontRuntime(FontRuntimeCell::ParameterCount(low)),
            15 => StateCell::FontRuntime(FontRuntimeCell::Dimen {
                font: (payload >> 18) as u32,
                number: (payload & ((1 << 18) - 1)) as u32,
            }),
            16 => StateCell::FontRuntime(FontRuntimeCell::HyphenChar(low)),
            17 => StateCell::FontRuntime(FontRuntimeCell::SkewChar(low)),
            18 => StateCell::FontRuntime(FontRuntimeCell::PdfCode {
                table: (payload >> 40) as u8,
                font: (payload >> 8) as u32,
                code: payload as u8,
            }),
            19 => StateCell::FontRuntime(FontRuntimeCell::LigaturesDisabled(low)),
            _ => unreachable!("journal stored a validated state-cell tag"),
        }
    }
}

fn code_table_kind(tag: u8) -> CodeTableKind {
    match tag {
        0 => CodeTableKind::Catcode,
        1 => CodeTableKind::Lccode,
        2 => CodeTableKind::Uccode,
        3 => CodeTableKind::Sfcode,
        4 => CodeTableKind::Mathcode,
        5 => CodeTableKind::Delcode,
        _ => unreachable!("journal stored a validated code-table kind"),
    }
}

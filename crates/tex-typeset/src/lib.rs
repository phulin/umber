//! Pure TeX typesetting kernels.
//!
//! This crate owns list-in/list-out algorithms only. Public packing entry
//! points take immutable state access, copy all parameters into plain structs
//! before doing arithmetic, and never mutate `Universe`.

pub mod alignment;
pub mod expansion;
pub mod linebreak;
pub mod math;
mod metrics;
mod packing;
pub mod protrusion;
#[cfg(test)]
mod test_state;
mod vertical_break;

use tex_state::font::PdfFontCode;
use tex_state::ids::FontId;
use tex_state::scaled::Scaled;

pub use packing::{
    HpackParams, HpackPlan, PackDiagnostic, PackSpec, PackedBox, UnsetMetrics, VpackParams, hpack,
    measure_unset, plan_hpack_nodes, readjust_vtop, vpack, vtop,
};
pub use vertical_break::{VerticalBreak, VerticalBreakError, vert_break};

/// TeX's infinite badness sentinel.
pub const INF_BAD: i32 = 10_000;

/// TeX's `\badness` value for a nonempty overfull packed box.
pub const OVERFULL_BADNESS: i32 = 1_000_000;

/// Immutable state access needed by the packing kernels.
pub trait TypesetState {
    fn page_nodes(
        &self,
        list: tex_state::node_arena::PageListId,
    ) -> tex_state::node_arena::NodeCursor<'_>;
    fn page_node_sequence(
        &self,
        _sequence: tex_state::node_arena::PageNodeSequenceId,
    ) -> Option<tex_state::node_arena::ArenaNodeSequence<'_, tex_state::node_arena::PageLifetime>>
    {
        None
    }
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics>;
    fn font_character_metrics(&self, font: FontId, ch: char) -> Option<tex_fonts::CharMetrics> {
        self.font_char_metrics(font, u8::try_from(ch as u32).ok()?)
    }
    fn font_uses_tfm_metrics(&self, _font: FontId) -> bool {
        true
    }
    fn font_widths(&self, font: FontId) -> &[Scaled; 256];
    fn font_characters(&self, font: FontId) -> &[Option<tex_fonts::CharMetrics>];
    fn font_parameter_value(&self, _font: FontId, _number: u32) -> Scaled {
        Scaled::from_raw(0)
    }
    fn pdf_font_code(&self, table: PdfFontCode, _font: FontId, _code: u8) -> i32 {
        if table == PdfFontCode::Ef { 1000 } else { 0 }
    }
    fn font_kern(&self, _font: FontId, _left: u8, _right: u8) -> Option<Scaled> {
        None
    }
    fn font_expansion_spec(&self, _font: FontId) -> Option<expansion::FontExpansionSpec> {
        None
    }
}

/// TeX.web section 108 `badness` function.
#[must_use]
pub fn badness(t: Scaled, s: Scaled) -> i32 {
    let t = t.raw();
    let s = s.raw();
    if t == 0 {
        0
    } else if s <= 0 {
        INF_BAD
    } else {
        let r = if t <= 7_230_584 {
            (t * 297) / s
        } else if s >= 1_663_497 {
            t / (s / 297)
        } else {
            t
        };
        if r > 1290 {
            INF_BAD
        } else {
            ((r * r * r + 0o400000) / 0o1000000).min(INF_BAD)
        }
    }
}

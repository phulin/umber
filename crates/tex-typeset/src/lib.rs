//! Pure TeX typesetting kernels.
//!
//! This crate owns list-in/list-out algorithms only. Public packing entry
//! points take immutable state access, copy all parameters into plain structs
//! before doing arithmetic, and never mutate `Universe`.

pub mod linebreak;
pub mod math;
mod packing;
mod vertical_break;

use tex_state::Universe;
use tex_state::glue::GlueSpec;
use tex_state::ids::{FontId, NodeListId};
use tex_state::node::Node;
use tex_state::scaled::Scaled;

pub use packing::{
    HpackParams, PackDiagnostic, PackSpec, PackedBox, UnsetMetrics, VpackParams, hpack,
    measure_unset, vpack, vtop,
};
pub use vertical_break::{VerticalBreak, VerticalBreakError, vert_break};

/// TeX's infinite badness sentinel.
pub const INF_BAD: i32 = 10_000;

/// Immutable state access needed by the packing kernels.
pub trait TypesetState {
    fn nodes(&self, id: NodeListId) -> &[Node];
    fn glue(&self, id: tex_state::ids::GlueId) -> GlueSpec;
    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics>;
}

impl TypesetState for Universe {
    fn nodes(&self, id: NodeListId) -> &[Node] {
        Universe::nodes(self, id)
    }

    fn glue(&self, id: tex_state::ids::GlueId) -> GlueSpec {
        Universe::glue(self, id)
    }

    fn font_char_metrics(&self, font: FontId, code: u8) -> Option<tex_fonts::CharMetrics> {
        Universe::font_char_metrics(self, font, code)
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

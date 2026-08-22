//! Operation-local publication of canonical packing geometry.
//!
//! TeX82's `hpack` and `vpack` commit their final box dimensions in
//! §§649--668 and §§668--676. Geometry observation is instrumentation, not
//! engine state: the sink is borrowed from the enclosing command operation,
//! and its records are published only when that operation commits. No
//! `Universe` queue or node-arena coordinate crosses the boundary.

use tex_state::scaled::Scaled;

/// Narrow execution-kernel boundary for committed packing geometry.
pub(crate) trait PackGeometrySink {
    fn committed_hpack(&mut self, width: Scaled, height: Scaled, depth: Scaled);
    fn committed_vpack(&mut self, width: Scaled, height: Scaled, depth: Scaled);
}

/// Explicit sink for source-free unit kernels and unobserved operations.
#[derive(Default)]
pub(crate) struct IgnorePackGeometry;

impl PackGeometrySink for IgnorePackGeometry {
    fn committed_hpack(&mut self, _width: Scaled, _height: Scaled, _depth: Scaled) {}

    fn committed_vpack(&mut self, _width: Scaled, _height: Scaled, _depth: Scaled) {}
}

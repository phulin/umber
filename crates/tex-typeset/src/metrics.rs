//! Shared metric events and accumulators for pure typesetting kernels.
//!
//! This module owns the arithmetic meaning of node dimensions. Callers still
//! own domain policy: packing chooses glue settings and diagnostics, line
//! breaking adds font-expansion capacity and scores routes, vertical breaking
//! chooses legal breakpoints, and Appendix G decides which packs are observed.

use tex_arith::WideScaled;
use tex_state::glue::GlueSpec;
use tex_state::scaled::Scaled;

#[derive(Clone, Copy)]
pub(crate) struct MetricOverflow {
    addition: &'static str,
    subtraction: &'static str,
}

impl MetricOverflow {
    pub(crate) const PACKING: Self = Self {
        addition: "packed dimension overflow must be reported, not saturated",
        subtraction: "packed dimension overflow must be reported, not saturated",
    };
    pub(crate) const APPENDIX_G: Self = Self {
        addition: "Appendix G scaled addition overflow",
        subtraction: "Appendix G scaled subtraction overflow",
    };
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum MetricEvent {
    Glyph {
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    Box {
        width: Scaled,
        height: Scaled,
        depth: Scaled,
        shift: Scaled,
    },
    Rule {
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    Image {
        width: Scaled,
        height: Scaled,
        depth: Scaled,
    },
    Kern(Scaled),
    Glue(GlueSpec),
    Math(Scaled),
}

/// A cursor over a domain-specific projection into the common metric IR.
pub(crate) struct MetricsCursor<I> {
    events: I,
}

impl<I> MetricsCursor<I> {
    pub(crate) const fn new(events: I) -> Self {
        Self { events }
    }
}

impl<I: Iterator> Iterator for MetricsCursor<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.events.next()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ListMetrics {
    pub(crate) width: Scaled,
    pub(crate) height: Scaled,
    pub(crate) depth: Scaled,
    pub(crate) stretch: [Scaled; 4],
    pub(crate) shrink: [Scaled; 4],
    pub(crate) has_glue: bool,
}

impl ListMetrics {
    pub(crate) const ZERO: Self = Self {
        width: Scaled::from_raw(0),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        stretch: [Scaled::from_raw(0); 4],
        shrink: [Scaled::from_raw(0); 4],
        has_glue: false,
    };

    pub(crate) fn observe_horizontal(&mut self, event: MetricEvent, overflow: MetricOverflow) {
        match event {
            MetricEvent::Glyph {
                width,
                height,
                depth,
            }
            | MetricEvent::Rule {
                width,
                height,
                depth,
            }
            | MetricEvent::Image {
                width,
                height,
                depth,
            } => {
                self.width = add(self.width, width, overflow);
                self.height = self.height.max(height);
                self.depth = self.depth.max(depth);
            }
            MetricEvent::Box {
                width,
                height,
                depth,
                shift,
            } => {
                self.width = add(self.width, width, overflow);
                self.height = self.height.max(sub(height, shift, overflow));
                self.depth = self.depth.max(add(depth, shift, overflow));
            }
            MetricEvent::Kern(amount) | MetricEvent::Math(amount) => {
                self.width = add(self.width, amount, overflow);
            }
            MetricEvent::Glue(spec) => {
                self.width = add(self.width, spec.width, overflow);
                self.add_glue(spec, overflow);
            }
        }
    }

    pub(crate) fn observe_vertical(&mut self, event: MetricEvent, overflow: MetricOverflow) {
        self.try_observe_vertical(event)
            .unwrap_or_else(|| panic!("{}", overflow.addition));
        if let MetricEvent::Glue(spec) = event {
            self.add_glue(spec, overflow);
        }
    }

    pub(crate) fn try_observe_vertical(&mut self, event: MetricEvent) -> Option<()> {
        match event {
            MetricEvent::Box {
                width,
                height,
                depth,
                shift,
            } => self.try_append_vertical_box(width, height, depth, shift)?,
            MetricEvent::Rule {
                width,
                height,
                depth,
            }
            | MetricEvent::Image {
                width,
                height,
                depth,
            } => self.try_append_vertical_box(width, height, depth, Scaled::from_raw(0))?,
            MetricEvent::Kern(amount) => self.try_append_vertical_spacing(amount)?,
            MetricEvent::Glue(spec) => {
                self.try_append_vertical_spacing(spec.width)?;
            }
            MetricEvent::Glyph { .. } | MetricEvent::Math(_) => {}
        }
        Some(())
    }

    pub(crate) fn merge_horizontal_dimensions(&mut self, other: Self, overflow: MetricOverflow) {
        self.width = add(self.width, other.width, overflow);
        self.height = self.height.max(other.height);
        self.depth = self.depth.max(other.depth);
        self.has_glue |= other.has_glue;
    }

    fn try_append_vertical_spacing(&mut self, amount: Scaled) -> Option<()> {
        self.height = self.height.checked_add(self.depth.checked_add(amount)?)?;
        self.depth = Scaled::from_raw(0);
        Some(())
    }

    fn try_append_vertical_box(
        &mut self,
        width: Scaled,
        height: Scaled,
        depth: Scaled,
        shift: Scaled,
    ) -> Option<()> {
        self.height = self.height.checked_add(self.depth)?.checked_add(height)?;
        self.depth = depth;
        self.width = self.width.max(width.checked_add(shift)?);
        Some(())
    }

    fn add_glue(&mut self, spec: GlueSpec, overflow: MetricOverflow) {
        self.has_glue = true;
        self.stretch[spec.stretch_order as usize] = add(
            self.stretch[spec.stretch_order as usize],
            spec.stretch,
            overflow,
        );
        self.shrink[spec.shrink_order as usize] = add(
            self.shrink[spec.shrink_order as usize],
            spec.shrink,
            overflow,
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WideMetricTotals {
    pub(crate) natural: WideScaled,
    pub(crate) stretch: [WideScaled; 4],
    pub(crate) shrink: [WideScaled; 4],
}

impl WideMetricTotals {
    pub(crate) const ZERO: Self = Self {
        natural: WideScaled::ZERO,
        stretch: [WideScaled::ZERO; 4],
        shrink: [WideScaled::ZERO; 4],
    };

    pub(crate) fn observe(&mut self, event: MetricEvent) {
        let width = match event {
            MetricEvent::Glyph { width, .. }
            | MetricEvent::Box { width, .. }
            | MetricEvent::Rule { width, .. }
            | MetricEvent::Image { width, .. } => width,
            MetricEvent::Kern(width) | MetricEvent::Math(width) => width,
            MetricEvent::Glue(spec) => {
                self.stretch[spec.stretch_order as usize] =
                    wide_add_scaled(self.stretch[spec.stretch_order as usize], spec.stretch);
                self.shrink[spec.shrink_order as usize] =
                    wide_add_scaled(self.shrink[spec.shrink_order as usize], spec.shrink);
                spec.width
            }
        };
        self.natural = wide_add_scaled(self.natural, width);
    }

    pub(crate) fn add_assign(&mut self, other: Self) {
        self.natural = wide_add(self.natural, other.natural);
        for order in 0..4 {
            self.stretch[order] = wide_add(self.stretch[order], other.stretch[order]);
            self.shrink[order] = wide_add(self.shrink[order], other.shrink[order]);
        }
    }

    pub(crate) fn sub(self, other: Self) -> Self {
        let mut out = Self::ZERO;
        out.natural = wide_sub(self.natural, other.natural);
        for order in 0..4 {
            out.stretch[order] = wide_sub(self.stretch[order], other.stretch[order]);
            out.shrink[order] = wide_sub(self.shrink[order], other.shrink[order]);
        }
        out
    }
}

fn add(left: Scaled, right: Scaled, overflow: MetricOverflow) -> Scaled {
    left.checked_add(right)
        .unwrap_or_else(|| panic!("{}", overflow.addition))
}

fn sub(left: Scaled, right: Scaled, overflow: MetricOverflow) -> Scaled {
    left.checked_sub(right)
        .unwrap_or_else(|| panic!("{}", overflow.subtraction))
}

fn wide_add(left: WideScaled, right: WideScaled) -> WideScaled {
    left.checked_add(right)
        .expect("scaled accumulator exceeds the addressable node-list domain")
}

fn wide_sub(left: WideScaled, right: WideScaled) -> WideScaled {
    left.checked_sub(right)
        .expect("scaled accumulator exceeds the addressable node-list domain")
}

pub(crate) fn wide_add_scaled(total: WideScaled, value: Scaled) -> WideScaled {
    wide_add(total, WideScaled::from_scaled(value))
}

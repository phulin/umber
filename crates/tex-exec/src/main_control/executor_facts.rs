//! Borrow-scoped projection of authoritative executor facts.

use super::*;
use std::borrow::Borrow;

pub(super) struct EffectiveTailFacts {
    pub(super) last_node: Option<tex_command::LastNodeItem>,
    pub(super) last_node_type: i32,
    pub(super) traversed: bool,
}

/// One processor-episode borrow of live executor facts.
///
/// This provider owns no sampled values. Each trait call reads exactly one
/// requested fact from the authoritative mode/page state, records the focused
/// operational counter, and returns before the processor can suspend.
pub(super) struct ExecutorHostFacts<'episode, G, Modes = &'episode ModeNest> {
    pub(super) modes: Modes,
    pub(super) pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
    pub(super) telemetry: &'episode mut crate::EpisodeTelemetry,
}

impl<G, Modes: Borrow<ModeNest>> tex_command::CommandHostFacts<G>
    for ExecutorHostFacts<'_, G, Modes>
{
    fn conditional_state(&mut self) -> tex_command::ConditionalState {
        self.telemetry.record_host_fact_query();
        self.modes.borrow().conditional_state()
    }

    fn diagnostic_mode_name(&mut self) -> &'static str {
        self.telemetry.record_host_fact_query();
        match self.modes.borrow().current_mode() {
            Mode::Vertical => "vertical mode",
            Mode::InternalVertical => "internal vertical mode",
            Mode::Horizontal => "horizontal mode",
            Mode::RestrictedHorizontal => "restricted horizontal mode",
            Mode::Math => "math mode",
            Mode::DisplayMath => "display math mode",
        }
    }

    fn space_factor(&mut self) -> Option<i32> {
        self.telemetry.record_host_fact_query();
        matches!(
            self.modes.borrow().current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        )
        .then(|| self.modes.borrow().current_list().space_factor())
    }

    fn prev_depth(&mut self, stores: &CommandContext<'_, G>) -> Option<Scaled> {
        self.telemetry.record_host_fact_query();
        let mode = self.modes.borrow().current_mode();
        let ignored_depth = crate::mode::ignored_depth_with_handle(stores, self.pdf_ignore_depth);
        // tex.web §418's `set_aux` twin of `space_factor`: `\prevdepth` is
        // readable only in vertical mode, where an unset `prev_depth` is
        // §215's `ignore_depth` initial value.
        matches!(mode, Mode::Vertical | Mode::InternalVertical).then(|| {
            self.modes
                .borrow()
                .current_list()
                .prev_depth()
                .unwrap_or(ignored_depth)
        })
    }

    fn prev_graf(&mut self) -> Option<i32> {
        self.telemetry.record_host_fact_query();
        // tex.web §422 walks to the nearest enclosing vertical level rather
        // than testing the current mode.
        Some(self.modes.borrow().enclosing_vertical_prev_graf())
    }

    fn last_node(&mut self, stores: &CommandContext<'_, G>) -> Option<tex_command::LastNodeItem> {
        self.telemetry.record_host_fact_query();
        let tail = effective_tail_facts(self.modes.borrow(), stores);
        self.telemetry
            .record_effective_tail_traversal(tail.traversed);
        tail.last_node
    }

    fn last_node_type(&mut self, stores: &CommandContext<'_, G>) -> i32 {
        self.telemetry.record_host_fact_query();
        let tail = effective_tail_facts(self.modes.borrow(), stores);
        self.telemetry
            .record_effective_tail_traversal(tail.traversed);
        tail.last_node_type
    }
}

/// Samples TeX82 §424 and e-TeX [26.424] from one effective-tail walk.
///
/// The outer vertical list is special (matching `\unskip`'s existing
/// `is_outer_vertical`/`page_has_last_glue` precedent): contributions move
/// directly to the page builder, so that list or its swept-tail memo is the
/// authoritative source.
fn effective_tail_facts<G>(
    modes: &ModeNest,
    context: &CommandContext<'_, G>,
) -> EffectiveTailFacts {
    if is_outer_vertical(modes) {
        let contributions = context.page_contributions();
        let mut nodes = contributions.iter();
        let tail = crate::effective_tail::EffectiveTail::find(&mut nodes);
        return match tail {
            Some(tail) => EffectiveTailFacts {
                last_node: classify_last_node(context, tail.node()),
                last_node_type: tail.node().etex_type(),
                traversed: true,
            },
            None => {
                let last_node_type = context.page_last_node_type();
                let last_node = match last_node_type {
                    11 => context
                        .page_last_skip()
                        .zip(context.page_last_skip_origin())
                        .map(|(spec, origin)| tex_command::LastNodeItem::Glue(spec, origin)),
                    12 => Some(tex_command::LastNodeItem::Kern(context.page_last_kern())),
                    13 => Some(tex_command::LastNodeItem::Penalty(
                        context.page_last_penalty(),
                    )),
                    _ => None,
                };
                EffectiveTailFacts {
                    last_node,
                    last_node_type,
                    traversed: true,
                }
            }
        };
    }
    if modes.current_list().pending_hchars().is_some() {
        return EffectiveTailFacts {
            last_node: None,
            last_node_type: 0,
            traversed: false,
        };
    }
    let current = modes.current_list().nodes(context);
    let mut nodes = current.iter();
    let tail = crate::effective_tail::EffectiveTail::find(&mut nodes);
    match tail {
        Some(tail) => EffectiveTailFacts {
            last_node: classify_last_node(context, tail.node()),
            last_node_type: tail.node().etex_type(),
            traversed: true,
        },
        None => EffectiveTailFacts {
            last_node: None,
            last_node_type: -1,
            traversed: true,
        },
    }
}

/// Classifies one real node as a `\lastpenalty`/`\lastkern`/`\lastskip`
/// tail, resolving a glue node's stored specification and distinguishing
/// TeX82's `mu_glue` subtype (an explicit `\mskip`, matched here by
/// [`GlueKind::MuSkip`]) so `\lastskip` reads it at `mu_val` level. Any
/// other node shape (including a character, which tex.web excludes via
/// `is_char_node`) has no matching case, exactly like tex.web's
/// `case cur_chr of ... end {there are no other cases}`.
fn classify_last_node<G>(
    stores: &tex_state::CommandContext<'_, G>,
    node: tex_state::node_view::NodeView<'_>,
) -> Option<tex_command::LastNodeItem> {
    match node {
        tex_state::node_view::NodeView::Penalty(value) => {
            Some(tex_command::LastNodeItem::Penalty(value))
        }
        tex_state::node_view::NodeView::Kern { amount, .. } => {
            Some(tex_command::LastNodeItem::Kern(amount))
        }
        tex_state::node_view::NodeView::Glue {
            spec,
            kind: GlueKind::MuSkip,
            origin,
            ..
        } => Some(tex_command::LastNodeItem::MuGlue(spec, origin)),
        tex_state::node_view::NodeView::Glue { spec, origin, .. } => {
            Some(tex_command::LastNodeItem::Glue(spec, origin))
        }
        // TeX82 keeps a discretionary's no-break replacement nodes in
        // the surrounding list (§1119), immediately after the disc node.
        // Umber freezes that physical suffix as the disc's `replace`
        // child list, so §424's tail enquiry must look through the
        // container to preserve TeX's physical-tail view.  This is
        // intentionally distinct from §1105 deletion, which refuses to
        // remove a discretionary replacement suffix.
        tex_state::node_view::NodeView::Disc { replace, .. } => stores
            .page_node_list(replace)
            .expect("discretionary replacement belongs to the live page arena")
            .nodes()
            .last()
            .and_then(|node| classify_last_node(stores, node)),
        _ => None,
    }
}

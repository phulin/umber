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
    pub(super) shown_mode: &'episode mut Option<Mode>,
    pub(super) output_routine_active: &'episode bool,
    pub(super) pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
    pub(super) telemetry: &'episode mut crate::EpisodeTelemetry,
}

// Both implementations read their actual owner. The macro shares query
// semantics without constructing a projection for each request.
macro_rules! implement_host_facts {
    (($($implementation:tt)*), $this:ident, $modes:expr, $telemetry:expr, $ignore_depth:expr, $shown_mode:expr, $output_active:expr) => {
        $($implementation)* {
    fn output_routine_active(&mut $this) -> bool {
        ($telemetry).record_host_fact_query();
        $output_active
    }
    fn conditional_state(&mut $this) -> tex_command::ConditionalState {
        ($telemetry).record_host_fact_query();
        ($modes).conditional_state()
    }

    fn diagnostic_mode_name(&mut $this) -> &'static str {
        ($telemetry).record_host_fact_query();
        match ($modes).current_mode() {
            Mode::Vertical => "vertical mode",
            Mode::InternalVertical => "internal vertical mode",
            Mode::Horizontal => "horizontal mode",
            Mode::RestrictedHorizontal => "restricted horizontal mode",
            Mode::Math => "math mode",
            Mode::DisplayMath => "display math mode",
        }
    }

    fn command_trace_mode_prefix(&mut $this, deferred_write: bool) -> Option<&'static str> {
        ($telemetry).record_host_fact_query();
        let mode = if deferred_write { None } else { Some(($modes).current_mode()) };
        let shown = $shown_mode;
        if *shown == mode {
            None
        } else {
            *shown = mode;
            Some(mode.map_or("no mode", mode_text_for_command_trace))
        }
    }

    fn space_factor(&mut $this) -> Option<i32> {
        ($telemetry).record_host_fact_query();
        matches!(
            ($modes).current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        )
        .then(|| ($modes).current_list().space_factor())
    }

    fn prev_depth(&mut $this, stores: &CommandContext<'_, G>) -> Option<Scaled> {
        ($telemetry).record_host_fact_query();
        let mode = ($modes).current_mode();
        let ignored_depth = crate::mode::ignored_depth_with_handle(stores, ($ignore_depth));
        // tex.web §418's `set_aux` twin of `space_factor`: `\prevdepth` is
        // readable only in vertical mode, where an unset `prev_depth` is
        // §215's `ignore_depth` initial value.
        matches!(mode, Mode::Vertical | Mode::InternalVertical).then(|| {
            ($modes)
                .current_list()
                .prev_depth()
                .unwrap_or(ignored_depth)
        })
    }

    fn prev_graf(&mut $this) -> Option<i32> {
        ($telemetry).record_host_fact_query();
        // tex.web §422 walks to the nearest enclosing vertical level rather
        // than testing the current mode.
        Some(($modes).enclosing_vertical_prev_graf())
    }

    fn last_node(&mut $this, stores: &CommandContext<'_, G>) -> Option<tex_command::LastNodeItem> {
        ($telemetry).record_host_fact_query();
        let tail = effective_tail_facts(($modes), stores);
        ($telemetry)
            .record_effective_tail_traversal(tail.traversed);
        tail.last_node
    }

    fn last_node_type(&mut $this, stores: &CommandContext<'_, G>) -> i32 {
        ($telemetry).record_host_fact_query();
        let tail = effective_tail_facts(($modes), stores);
        ($telemetry)
            .record_effective_tail_traversal(tail.traversed);
        tail.last_node_type
    }
        }
    };
}

implement_host_facts!(
    (impl<G, Modes: Borrow<ModeNest>> tex_command::CommandHostFacts<G>
        for ExecutorHostFacts<'_, G, Modes>),
    self,
    self.modes.borrow(),
    self.telemetry,
    self.pdf_ignore_depth,
    &mut *self.shown_mode,
    *self.output_routine_active
);

implement_host_facts!(
    (impl<G> tex_command::CommandHostFacts<G> for ExecutionState<G>),
    self,
    &self.modes,
    self.episode_telemetry,
    self.pdf_ignore_depth,
    &mut self.shown_mode,
    self.boxes.output_routine_active
);

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

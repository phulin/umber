//! Borrow-scoped projection of authoritative executor facts.

use super::*;

/// Stack-owned brand for one command-processing interval.
///
/// Delivery preparation may cross synchronous processor borrows, but cannot
/// enter a suspension or persistent operation frame. Live executor facts use
/// a separate processor-episode borrow and are never stored here.
pub(super) struct OperationPreparationScope;

/// Copy-free delivery/retry preparation for one operation.
pub(super) struct OperationPreparation<'operation, G> {
    checked_save_stack_words: Option<usize>,
    delivery: Option<OperationDelivery>,
    resume: Option<OperationResume<G>>,
    delivery_status: Option<tex_command::DeliveryStatus>,
    trace_reported: bool,
    pub(super) _scope: PhantomData<&'operation mut OperationPreparationScope>,
}

struct OperationResume<G> {
    scanner: Option<tex_command::ScannerFrameKey<G>>,
    expansion: Option<tex_command::ExpansionWorkKey<G>>,
}

impl<'operation, G> OperationPreparation<'operation, G> {
    pub(super) fn new(_scope: &'operation mut OperationPreparationScope) -> Self {
        Self {
            checked_save_stack_words: None,
            delivery: None,
            resume: None,
            delivery_status: None,
            trace_reported: false,
            _scope: PhantomData,
        }
    }

    pub(super) fn fill_delivery(
        &mut self,
        delivery: OperationDelivery,
        scanner: Option<tex_command::ScannerFrameKey<G>>,
        expansion: Option<tex_command::ExpansionWorkKey<G>>,
    ) {
        assert!(
            self.delivery.is_none() && self.resume.is_none(),
            "one operation preparation owns one direct dispatch result"
        );
        self.delivery = Some(delivery);
        if scanner.is_some() || expansion.is_some() {
            self.resume = Some(OperationResume { scanner, expansion });
        }
    }

    pub(super) fn has_delivery(&self) -> bool {
        self.delivery.is_some()
    }

    pub(super) fn delivery(&self) -> &OperationDelivery {
        self.delivery
            .as_ref()
            .expect("operation preparation owns one delivery")
    }

    pub(super) fn take_delivery(&mut self) -> OperationDelivery {
        self.delivery
            .take()
            .expect("operation preparation drains one delivery")
    }

    pub(super) fn take_scanner(&mut self) -> Option<tex_command::ScannerFrameKey<G>> {
        self.resume
            .as_mut()
            .and_then(|resume| resume.scanner.take())
    }

    pub(super) fn take_expansion(&mut self) -> Option<tex_command::ExpansionWorkKey<G>> {
        self.resume.take().and_then(|resume| resume.expansion)
    }

    pub(super) fn record_delivery_status(
        &mut self,
        status: tex_command::DeliveryStatus,
        trace_reported: bool,
    ) {
        assert!(
            self.delivery_status.replace(status).is_none(),
            "one operation preparation owns one raw delivery status"
        );
        self.trace_reported = trace_reported;
    }

    pub(super) fn take_delivery_status(&mut self) -> tex_command::DeliveryStatus {
        self.delivery_status
            .take()
            .expect("raw preflight fills one delivery status")
    }

    pub(super) fn take_trace_reported(&mut self) -> bool {
        std::mem::take(&mut self.trace_reported)
    }

    pub(super) fn discard_delivery_status(&mut self) {
        self.delivery_status = None;
        self.trace_reported = false;
    }

    pub(super) fn record_checked_save_stack_words(&mut self, words: usize) {
        self.checked_save_stack_words = Some(words);
    }

    pub(super) fn take_checked_save_stack_words(&mut self) -> Option<usize> {
        self.checked_save_stack_words.take()
    }
}

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
pub(super) struct ExecutorHostFacts<'episode, G> {
    pub(super) modes: &'episode ModeNest,
    pub(super) pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
    pub(super) telemetry: &'episode mut crate::EpisodeTelemetry,
}

impl<G> tex_command::CommandHostFacts<G> for ExecutorHostFacts<'_, G> {
    fn conditional_state(&mut self) -> tex_command::ConditionalState {
        self.telemetry.record_host_fact_query();
        self.modes.conditional_state()
    }

    fn space_factor(&mut self) -> Option<i32> {
        self.telemetry.record_host_fact_query();
        matches!(
            self.modes.current_mode(),
            Mode::Horizontal | Mode::RestrictedHorizontal
        )
        .then(|| self.modes.current_list().space_factor())
    }

    fn prev_depth(&mut self, stores: &CommandContext<'_, G>) -> Option<Scaled> {
        self.telemetry.record_host_fact_query();
        let mode = self.modes.current_mode();
        let ignored_depth = crate::mode::ignored_depth_with_handle(stores, self.pdf_ignore_depth);
        // tex.web §418's `set_aux` twin of `space_factor`: `\prevdepth` is
        // readable only in vertical mode, where an unset `prev_depth` is
        // §215's `ignore_depth` initial value.
        matches!(mode, Mode::Vertical | Mode::InternalVertical).then(|| {
            self.modes
                .current_list()
                .prev_depth()
                .unwrap_or(ignored_depth)
        })
    }

    fn prev_graf(&mut self) -> Option<i32> {
        self.telemetry.record_host_fact_query();
        // tex.web §422 walks to the nearest enclosing vertical level rather
        // than testing the current mode.
        Some(self.modes.enclosing_vertical_prev_graf())
    }

    fn last_node(&mut self, stores: &CommandContext<'_, G>) -> Option<tex_command::LastNodeItem> {
        self.telemetry.record_host_fact_query();
        let tail = effective_tail_facts(self.modes, stores);
        self.telemetry
            .record_effective_tail_traversal(tail.traversed);
        tail.last_node
    }

    fn last_node_type(&mut self, stores: &CommandContext<'_, G>) -> i32 {
        self.telemetry.record_host_fact_query();
        let tail = effective_tail_facts(self.modes, stores);
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
                        .map(tex_command::LastNodeItem::Glue),
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
    node: tex_state::node_arena::NodeView<'_>,
) -> Option<tex_command::LastNodeItem> {
    match node {
        tex_state::node_arena::NodeView::Penalty(value) => {
            Some(tex_command::LastNodeItem::Penalty(value))
        }
        tex_state::node_arena::NodeView::Kern { amount, .. } => {
            Some(tex_command::LastNodeItem::Kern(amount))
        }
        tex_state::node_arena::NodeView::Glue {
            spec,
            kind: GlueKind::MuSkip,
            ..
        } => Some(tex_command::LastNodeItem::MuGlue(spec)),
        tex_state::node_arena::NodeView::Glue { spec, .. } => {
            Some(tex_command::LastNodeItem::Glue(spec))
        }
        // TeX82 keeps a discretionary's no-break replacement nodes in
        // the surrounding list (§1119), immediately after the disc node.
        // Umber freezes that physical suffix as the disc's `replace`
        // child list, so §424's tail enquiry must look through the
        // container to preserve TeX's physical-tail view.  This is
        // intentionally distinct from §1105 deletion, which refuses to
        // remove a discretionary replacement suffix.
        tex_state::node_arena::NodeView::Disc { replace, .. } => stores
            .page_node_list(replace)
            .expect("discretionary replacement belongs to the live page arena")
            .nodes()
            .last()
            .and_then(|node| classify_last_node(stores, node)),
        _ => None,
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn operation_preparation_initializes_only_direct_delivery_state() {
        let mut scope = OperationPreparationScope;
        let mut preparation: OperationPreparation<'_, ()> = OperationPreparation::new(&mut scope);
        preparation.fill_delivery(OperationDelivery::Replay, None, None);
        assert!(preparation.resume.is_none());
    }
}

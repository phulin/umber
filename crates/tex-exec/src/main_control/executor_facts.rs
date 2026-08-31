//! Borrow-scoped projection of authoritative executor facts.

use super::*;

/// Stack-owned brand for one topology-stable command-processing interval.
///
/// A preparation may cross delivery and scanning borrows, but cannot enter a
/// suspension or persistent operation frame. Semantic application consumes
/// the preparation before it can mutate the mode/page topology.
pub(super) struct OperationPreparationScope;

/// Copy-free executor facts sampled once for one topology-stable operation.
pub(super) struct OperationHostPreparation<'operation, G> {
    mode: Option<Mode>,
    last_node_type: i32,
    pdf_output: i32,
    innermost_group: Option<GroupKind>,
    checked_save_stack_words: Option<usize>,
    delivery: Option<OperationDelivery>,
    preflight: Option<crate::transaction_protocol::CommandPreflight>,
    resume: Option<OperationResume<G>>,
    delivery_status: Option<tex_command::DeliveryStatus>,
    trace_reported: bool,
    pub(super) _scope: PhantomData<&'operation mut OperationPreparationScope>,
}

struct OperationResume<G> {
    scanner: Option<tex_command::ScannerFrameKey<G>>,
    expansion: Option<tex_command::ExpansionWorkKey<G>>,
}

impl<'operation, G> OperationHostPreparation<'operation, G> {
    pub(super) fn new(_scope: &'operation mut OperationPreparationScope) -> Self {
        Self {
            mode: None,
            last_node_type: -1,
            pdf_output: 0,
            innermost_group: None,
            checked_save_stack_words: None,
            delivery: None,
            preflight: None,
            resume: None,
            delivery_status: None,
            trace_reported: false,
            _scope: PhantomData,
        }
    }

    pub(super) fn fill_preflight(
        &mut self,
        delivery: OperationDelivery,
        preflight: crate::transaction_protocol::CommandPreflight,
        scanner: Option<tex_command::ScannerFrameKey<G>>,
        expansion: Option<tex_command::ExpansionWorkKey<G>>,
    ) {
        assert!(
            self.delivery.is_none() && self.preflight.is_none() && self.resume.is_none(),
            "one host preparation owns one preflight result"
        );
        self.delivery = Some(delivery);
        self.preflight = Some(preflight);
        if scanner.is_some() || expansion.is_some() {
            self.resume = Some(OperationResume { scanner, expansion });
        }
    }

    pub(super) fn record_command_preflight(
        &mut self,
        preflight: crate::transaction_protocol::CommandPreflight,
    ) {
        assert!(
            self.delivery.is_none() && self.preflight.replace(preflight).is_none(),
            "one command classification has one prepared destination"
        );
    }

    pub(super) fn take_recorded_preflight(
        &mut self,
    ) -> Option<crate::transaction_protocol::CommandPreflight> {
        assert!(
            self.delivery.is_none(),
            "completed delivery owns its command preflight"
        );
        self.preflight.take()
    }

    pub(super) fn has_preflight(&self) -> bool {
        self.delivery.is_some()
    }

    pub(super) fn delivery(&self) -> &OperationDelivery {
        self.delivery
            .as_ref()
            .expect("prepared host facts own one delivery")
    }

    pub(super) fn preflight(&self) -> &crate::transaction_protocol::CommandPreflight {
        self.preflight
            .as_ref()
            .expect("prepared host facts own one command preflight")
    }

    pub(super) fn take_preflight(&mut self) -> crate::transaction_protocol::CommandPreflight {
        self.preflight
            .take()
            .expect("operation preparation drains one command preflight")
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
            "one host preparation owns one raw delivery status"
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

    pub(super) fn mode(&self) -> Mode {
        self.mode.expect("operation host facts are prepared once")
    }

    pub(super) fn last_node_type(&self) -> i32 {
        assert!(
            self.mode.is_some(),
            "operation host facts are prepared once"
        );
        self.last_node_type
    }

    pub(super) fn pdf_output(&self) -> i32 {
        assert!(
            self.mode.is_some(),
            "operation host facts are prepared once"
        );
        self.pdf_output
    }

    pub(super) fn innermost_group(&self) -> Option<GroupKind> {
        assert!(
            self.mode.is_some(),
            "operation host facts are prepared once"
        );
        self.innermost_group
    }

    pub(super) fn record_checked_save_stack_words(&mut self, words: usize) {
        self.checked_save_stack_words = Some(words);
    }

    pub(super) fn take_checked_save_stack_words(&mut self) -> Option<usize> {
        self.checked_save_stack_words.take()
    }

    pub(super) fn refresh_transaction_facts(&mut self, stores: &CommandContext<'_, G>) {
        assert!(
            self.mode.is_some(),
            "operation host facts are prepared once"
        );
        self.pdf_output = stores.int_param(IntParam::PDF_OUTPUT);
        self.innermost_group = stores.innermost_group_kind();
    }
}

pub(super) struct EffectiveTailFacts {
    pub(super) last_node: Option<tex_command::LastNodeItem>,
    pub(super) last_node_type: i32,
    pub(super) traversed: bool,
    pub(super) descriptor_visits: usize,
}

impl<G> MainControl<G> {
    pub(super) fn prepare_host_capabilities(
        &mut self,
        stores: &CommandContext<'_, G>,
        preparation: &mut OperationHostPreparation<'_, G>,
    ) {
        let mode = self.modes.current_mode();
        let tail = self.effective_tail_facts(stores);
        self.capabilities
            .set_conditional_state(self.modes.conditional_state());
        self.capabilities.set_space_factor(
            matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                .then(|| self.modes.current_list().space_factor()),
        );
        let ignored_depth = crate::mode::ignored_depth_with_handle(stores, self.pdf_ignore_depth);
        // tex.web §418's `set_aux` twin of `space_factor`: `\prevdepth` is
        // readable only in vertical mode, where an unset `prev_depth` is
        // §215's `ignore_depth` initial value.
        self.capabilities.set_prev_depth(
            matches!(mode, Mode::Vertical | Mode::InternalVertical).then(|| {
                self.modes
                    .current_list()
                    .prev_depth()
                    .unwrap_or(ignored_depth)
            }),
        );
        // tex.web §422's `set_prev_graf` walks up to the nearest enclosing
        // vertical level rather than testing the current mode.
        self.capabilities
            .set_prev_graf(Some(self.modes.enclosing_vertical_prev_graf()));
        self.capabilities.set_last_node(tail.last_node);
        self.capabilities.set_last_node_type(tail.last_node_type);
        self.episode_telemetry
            .record_host_preparation(tail.traversed, tail.descriptor_visits);
        preparation.mode = Some(mode);
        preparation.last_node_type = tail.last_node_type;
        preparation.pdf_output = stores.int_param(IntParam::PDF_OUTPUT);
        preparation.innermost_group = stores.innermost_group_kind();
        preparation.checked_save_stack_words = None;
    }

    /// Samples TeX82 §424 and e-TeX [26.424] from one effective-tail walk.
    ///
    /// The outer vertical list is special (matching `\unskip`'s existing
    /// `is_outer_vertical`/`page_has_last_glue` precedent from
    /// umber2-johp.81, reused here rather than duplicated):
    /// `append_vertical_contribution` moves every node contributed at that
    /// level straight to the page builder's contribution list instead of
    /// `ModeNest`'s own list, so this mode nest's list is never the right
    /// place to look. tex.web's real tail there is `contrib_head`, a fixed
    /// address in `is_char_node`'s address range, which is why its
    /// `scan_something_internal` falls through to `last_penalty`/
    /// `last_kern`/`last_glue` (updated together by §996 whenever the page
    /// builder sweeps a node onto the page) exactly when the contribution
    /// list has been swept empty; while it is nonempty, the real
    /// contribution tail governs, just as it does for `\unskip`.
    pub(super) fn effective_tail_facts(
        &self,
        context: &CommandContext<'_, G>,
    ) -> EffectiveTailFacts {
        if is_outer_vertical(&self.modes) {
            let contributions = context.page_contributions();
            let mut nodes = contributions.iter();
            let tail = crate::effective_tail::EffectiveTail::find(&mut nodes);
            let descriptor_visits = nodes.reverse_descriptor_visits();
            return match tail {
                Some(tail) => EffectiveTailFacts {
                    last_node: Self::classify_last_node(context, tail.node()),
                    last_node_type: tail.node().etex_type(),
                    traversed: true,
                    descriptor_visits,
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
                        descriptor_visits,
                    }
                }
            };
        }
        if self.modes.current_list().pending_hchars().is_some() {
            return EffectiveTailFacts {
                last_node: None,
                last_node_type: 0,
                traversed: false,
                descriptor_visits: 0,
            };
        }
        let current = self.modes.current_list().nodes(context);
        let mut nodes = current.iter();
        let tail = crate::effective_tail::EffectiveTail::find(&mut nodes);
        let descriptor_visits = nodes.reverse_descriptor_visits();
        match tail {
            Some(tail) => EffectiveTailFacts {
                last_node: Self::classify_last_node(context, tail.node()),
                last_node_type: tail.node().etex_type(),
                traversed: true,
                descriptor_visits,
            },
            None => EffectiveTailFacts {
                last_node: None,
                last_node_type: -1,
                traversed: true,
                descriptor_visits,
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
    pub(super) fn classify_last_node(
        stores: &tex_state::CommandContext<'_, G>,
        node: &Node,
    ) -> Option<tex_command::LastNodeItem> {
        match node {
            Node::Penalty(value) => Some(tex_command::LastNodeItem::Penalty(*value)),
            Node::Kern { amount, .. } => Some(tex_command::LastNodeItem::Kern(*amount)),
            Node::Glue {
                spec,
                kind: GlueKind::MuSkip,
                ..
            } => Some(tex_command::LastNodeItem::MuGlue(*spec)),
            Node::Glue { spec, .. } => Some(tex_command::LastNodeItem::Glue(*spec)),
            // TeX82 keeps a discretionary's no-break replacement nodes in
            // the surrounding list (§1119), immediately after the disc node.
            // Umber freezes that physical suffix as the disc's `replace`
            // child list, so §424's tail enquiry must look through the
            // container to preserve TeX's physical-tail view.  This is
            // intentionally distinct from §1105 deletion, which refuses to
            // remove a discretionary replacement suffix.
            Node::Disc { replace, .. } => stores
                .page_node_list(*replace)
                .expect("discretionary replacement belongs to the live page arena")
                .nodes()
                .last()
                .and_then(|node| Self::classify_last_node(stores, node)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn operation_preparation_keeps_cold_resume_storage_out_of_direct_initialization() {
        let mut scope = OperationPreparationScope;
        let mut preparation: OperationHostPreparation<'_, ()> =
            OperationHostPreparation::new(&mut scope);
        preparation.fill_preflight(
            OperationDelivery::Replay,
            crate::transaction_protocol::canonical_static_command_preflight(Meaning::Relax),
            None,
            None,
        );
        assert!(preparation.resume.is_none());
        assert_eq!(
            std::mem::size_of::<OperationHostPreparation<'static, ()>>(),
            136
        );
    }
}

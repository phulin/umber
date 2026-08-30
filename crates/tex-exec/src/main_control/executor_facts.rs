//! Borrow-scoped projection of authoritative executor facts.

use super::*;

/// Stack-owned brand for one topology-stable command-processing interval.
///
/// A preparation may cross delivery and scanning borrows, but cannot enter a
/// suspension or persistent operation frame. Semantic application consumes
/// the preparation before it can mutate the mode/page topology.
pub(super) struct OperationPreparationScope;

/// Copy-free executor facts sampled once for one topology-stable operation.
pub(super) struct OperationHostPreparation<'operation> {
    mode: Option<Mode>,
    last_node_type: i32,
    pdf_output: i32,
    innermost_group: Option<GroupKind>,
    checked_save_stack_words: Option<usize>,
    pub(super) _scope: PhantomData<&'operation mut OperationPreparationScope>,
}

impl<'operation> OperationHostPreparation<'operation> {
    pub(super) fn new(_scope: &'operation mut OperationPreparationScope) -> Self {
        Self {
            mode: None,
            last_node_type: -1,
            pdf_output: 0,
            innermost_group: None,
            checked_save_stack_words: None,
            _scope: PhantomData,
        }
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

    pub(super) fn refresh_transaction_facts<G>(&mut self, stores: &CommandContext<'_, G>) {
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
        preparation: &mut OperationHostPreparation<'_>,
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

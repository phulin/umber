//! Page-builder projections of the admitted command episode.

use super::*;

impl<'a, G> CommandContext<'a, G> {
    #[must_use]
    pub fn page_dimension(&self, dimension: crate::page::PageDimension) -> Scaled {
        self.page.dimension(dimension, false)
    }

    #[must_use]
    pub fn page_dimension_with_output_routine(
        &self,
        dimension: crate::page::PageDimension,
        output_routine_active: bool,
    ) -> Scaled {
        self.page.dimension(dimension, output_routine_active)
    }

    pub fn set_page_dimension(&mut self, dimension: crate::page::PageDimension, value: Scaled) {
        self.page.set_dimension(dimension, value);
    }

    #[must_use]
    pub fn page_integer(&self, integer: crate::page::PageInteger) -> i32 {
        self.page.integer(integer)
    }

    pub fn set_page_integer(&mut self, integer: crate::page::PageInteger, value: i32) {
        self.page.set_integer(integer, value);
    }

    #[must_use]
    pub fn page_contents(&self) -> crate::page::PageContents {
        self.page.contents()
    }

    pub fn set_page_contents(&mut self, contents: crate::page::PageContents) {
        self.page.set_contents(contents);
    }

    #[must_use]
    pub fn page_max_depth(&self) -> Scaled {
        self.page.page_max_depth()
    }

    #[must_use]
    pub fn insert_penalties(&self) -> i32 {
        self.page.insert_penalties()
    }

    #[must_use]
    pub fn least_page_cost(&self) -> i32 {
        self.page.least_page_cost()
    }

    pub fn freeze_page_specs(
        &mut self,
        contents: crate::page::PageContents,
        vsize: Scaled,
        max_depth: Scaled,
    ) {
        self.page.freeze_specs(contents, vsize, max_depth);
    }

    pub fn record_best_page_break(&mut self, index: usize, best_size: Scaled, cost: i32) {
        self.page.record_best_break(index, best_size, cost);
    }

    pub fn record_page_fire_up(&mut self, trigger_index: usize) {
        self.page.record_fire_up(trigger_index);
    }

    #[must_use]
    pub fn page_fire_up(&self) -> Option<crate::page::PageFireUp> {
        self.page.fire_up()
    }

    /// Begins TeX82 §1054's next end-job ejection at a page-builder
    /// position different from the preceding attempt.
    pub fn begin_end_job_ejection(
        &mut self,
    ) -> Result<crate::page::PageBuilderProgressToken, crate::page::PageBuilderProgressToken> {
        self.page.begin_end_job_ejection()
    }

    /// Reports whether the bracketed §994 builder invocation changed the
    /// canonical page-builder position.
    #[must_use]
    pub fn complete_end_job_ejection(
        &mut self,
        started: crate::page::PageBuilderProgressToken,
    ) -> bool {
        self.page.complete_end_job_ejection(started)
    }

    /// Retires the page-owned replay fence once §1054 accepts the final stop.
    pub fn finish_end_job(&mut self) {
        self.page.finish_end_job();
    }

    #[must_use]
    pub fn page_builder_resume_after_output_pending(&self) -> bool {
        self.page.resume_after_output_pending()
    }

    /// Consumes TeX82 §1012's continuation back into the same §994
    /// `build_page` invocation.
    pub fn take_page_builder_resume_after_output(&mut self) -> bool {
        self.page.take_resume_after_output()
    }

    pub fn start_page_after_output(&mut self) {
        self.page.start_page_after_output(&self.page_nodes);
    }

    /// Opens the production next-page closure after box255 has settled.
    pub fn arm_page_region_successor(&mut self) {
        let mark = self
            .page_nodes
            .begin_closure_build()
            .expect("page output successor boundary is available");
        self.page.arm_output_successor_build(mark);
    }

    pub fn start_new_page(&mut self) {
        self.page.start_new_page(&self.page_nodes);
    }

    #[must_use]
    pub fn page_contributions(&self) -> crate::page::PageContributionView<'_> {
        self.page.contribution(&self.page_nodes)
    }

    pub fn append_page_contribution(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.push_contribution(&mut self.page_nodes, node);
    }

    pub fn prepend_page_contribution(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.prepend_contribution(&mut self.page_nodes, node);
    }

    pub fn prepend_page_contributions(&mut self, nodes: PageListId) {
        self.page.prepend_contributions(&mut self.page_nodes, nodes);
    }

    pub fn append_page_contributions(&mut self, nodes: PageListId) {
        self.page.append_contributions(&mut self.page_nodes, nodes);
    }

    pub fn append_unique_page_contributions(
        &mut self,
        nodes: crate::page_node_arena::UniquePageList,
    ) {
        self.page
            .append_unique_contributions(&mut self.page_nodes, nodes);
    }

    pub fn remove_page_contribution_range(
        &mut self,
        range: std::ops::RangeInclusive<usize>,
    ) -> crate::page::PageNodeCarrier {
        self.page
            .remove_contribution_range(&mut self.page_nodes, range)
    }

    #[must_use]
    pub fn page_contribution_front(&self) -> Option<crate::NodeView<'_>> {
        self.page.contribution_front(&self.page_nodes)
    }

    #[must_use]
    pub fn page_contribution_second(&self) -> Option<crate::NodeView<'_>> {
        self.page.contribution_second(&self.page_nodes)
    }

    pub fn pop_page_contribution_front(&mut self) -> Option<crate::page::PageNodeCarrier> {
        self.page.pop_contribution_front(&mut self.page_nodes)
    }

    pub fn discard_page_node(&mut self, carrier: crate::page::PageNodeCarrier) {
        self.page.discard_carrier(carrier);
    }

    #[must_use]
    pub fn page_carrier_node<'b>(
        &'b self,
        carrier: &crate::page::PageNodeCarrier,
    ) -> crate::NodeView<'b> {
        self.page_nodes
            .node_cursor(carrier.list())
            .expect("page carrier belongs to the live arena")
            .get(0)
            .expect("page carrier contains one node")
    }

    #[must_use]
    pub fn current_page_len(&self) -> usize {
        self.page.current_page_len()
    }

    #[must_use]
    pub fn current_page_tail(&self) -> Option<crate::node_view::NodeView<'_>> {
        self.page.current_page_tail(&self.page_nodes)
    }

    pub fn push_current_page_node(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.push_current_page(&mut self.page_nodes, node);
    }

    pub fn push_current_page_carrier(&mut self, carrier: crate::page::PageNodeCarrier) {
        {
            let node = self
                .page_nodes
                .node_cursor(carrier.list())
                .expect("page carrier belongs to the live arena")
                .get(0)
                .expect("page carrier contains one node");
            let node = node.to_owned();
            self.assert_live_node_font_roots(&node);
        }
        self.page
            .push_current_page_carrier(&mut self.page_nodes, carrier);
    }

    pub fn push_current_page_list(&mut self, list: PageListId) {
        self.page.push_current_page_list(&mut self.page_nodes, list);
    }

    pub fn push_current_page_replacement(
        &mut self,
        carrier: crate::page::PageNodeCarrier,
        replacement: crate::node::Node,
    ) {
        self.assert_live_node_font_roots(&replacement);
        self.page
            .push_current_page_replacement(&mut self.page_nodes, carrier, replacement);
    }

    pub fn take_current_page_prefix(&mut self, split_index: usize) -> (PageListId, PageListId) {
        self.page
            .take_current_page_prefix(&mut self.page_nodes, split_index)
    }

    pub fn update_page_last_from_node(&mut self, node: &crate::node::Node) {
        self.page.update_last_from_node(node);
    }

    #[must_use]
    pub fn page_has_last_glue(&self) -> bool {
        self.page.has_last_glue()
    }

    #[must_use]
    pub fn page_last_skip(&self) -> Option<GlueSpec> {
        self.page.last_skip_ref()
    }

    #[must_use]
    pub fn page_last_penalty(&self) -> i32 {
        self.page.last_penalty()
    }

    #[must_use]
    pub fn page_last_kern(&self) -> Scaled {
        self.page.last_kern()
    }

    #[must_use]
    pub fn page_last_node_type(&self) -> i32 {
        self.page.last_node_type()
    }

    pub fn push_page_discard(&mut self, node: crate::node::Node) {
        self.assert_live_node_font_roots(&node);
        self.page.push_page_discard(&mut self.page_nodes, node);
    }

    pub fn push_page_discard_carrier(&mut self, carrier: crate::page::PageNodeCarrier) {
        {
            let node = self
                .page_nodes
                .node_cursor(carrier.list())
                .expect("page carrier belongs to the live arena")
                .get(0)
                .expect("page carrier contains one node");
            let node = node.to_owned();
            self.assert_live_node_font_roots(&node);
        }
        self.page
            .push_page_discard_carrier(&mut self.page_nodes, carrier);
    }

    pub fn take_page_discards(&mut self) -> PageListId {
        self.page.take_page_discards(&self.page_nodes)
    }

    pub fn clear_page_discards(&mut self) {
        self.page.clear_page_discards(&self.page_nodes);
    }

    pub fn set_split_discards(&mut self, nodes: PageListId) {
        self.page.set_split_discards(&self.page_nodes, nodes);
    }

    pub fn take_split_discards(&mut self) -> PageListId {
        self.page.take_split_discards(&self.page_nodes)
    }

    pub fn clear_split_discards(&mut self) {
        self.page.clear_split_discards(&self.page_nodes);
    }

    #[must_use]
    pub fn page_insertions(&self) -> crate::page::PageInsertionView<'_> {
        self.page.page_insertions()
    }

    #[must_use]
    pub fn page_insertion(&self, class: u16) -> Option<crate::page::PageInsertion> {
        self.page.page_insertion(class)
    }

    pub fn upsert_page_insertion(&mut self, insertion: crate::page::PageInsertion) {
        self.page.upsert_page_insertion(insertion);
    }

    #[must_use]
    pub fn page_mark(&self, mark: crate::page::PageMark) -> crate::node::NodeTokenList {
        self.page.mark(mark)
    }

    #[must_use]
    pub fn page_mark_value(
        &self,
        mark: crate::page::PageMark,
    ) -> Option<&crate::node::NodeTokenList> {
        self.page.mark_value(mark)
    }

    #[must_use]
    pub fn page_mark_class_value(
        &self,
        mark: crate::page::PageMark,
        class: u16,
    ) -> Option<&crate::node::NodeTokenList> {
        self.page.mark_class_value(mark, class)
    }

    pub fn set_page_mark(
        &mut self,
        mark: crate::page::PageMark,
        value: crate::node::NodeTokenList,
    ) {
        self.page.set_mark(mark, value);
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    pub fn clear_page_mark(&mut self, mark: crate::page::PageMark) {
        self.page.clear_mark(mark);
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMark(mark.index()));
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class: 0,
            });
    }

    pub fn set_page_mark_class(
        &mut self,
        mark: crate::page::PageMark,
        class: u16,
        value: crate::node::NodeTokenList,
    ) {
        self.page.set_mark_class(mark, class, value);
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.resident
                .dependencies
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn clear_page_mark_class(&mut self, mark: crate::page::PageMark, class: u16) {
        self.page.clear_mark_class(mark, class);
        self.resident
            .dependencies
            .mark_changed(DependencyKey::PageMarkClass {
                mark: mark.index(),
                class,
            });
        if class == 0 {
            self.resident
                .dependencies
                .mark_changed(DependencyKey::PageMark(mark.index()));
        }
    }

    pub fn page_mark_classes(&self) -> impl Iterator<Item = u16> + '_ {
        self.page.mark_class_ids()
    }
}

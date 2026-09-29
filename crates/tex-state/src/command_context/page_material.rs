//! page commands operations on the existing impl owner.

use super::*;

impl<'a, G> CommandContext<'a, G> {
    /// Publishes one complete page-lifetime list inside this admitted episode.
    pub fn publish_page_nodes(&mut self, nodes: Vec<crate::node::Node>) -> PageListId {
        let etex_node_sizes = self.resident.engine_usage.uses_etex_node_sizes();
        let words = nodes.iter().fold((0_usize, 0_usize), |words, node| {
            let node_words = node.tex_memory_words(etex_node_sizes);
            (
                words.0.saturating_add(node_words.0),
                words.1.saturating_add(node_words.1),
            )
        });
        for node in &nodes {
            self.assert_live_node_font_roots(node);
        }
        let list = self
            .page_nodes
            .publish_owned(nodes)
            .expect("page construction contains only live page-arena children");
        self.resident
            .engine_usage
            .observe_transient_memory(words.0, words.1);
        list
    }

    /// Constructs one generated page node in its final resident slot.
    pub fn construct_page_node(
        &mut self,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) -> PageListId {
        let mut builder = crate::page_node_arena::PageMaterialActiveListBuilder::default();
        self.open_page_active_list(&mut builder);
        self.construct_page_active_list(&mut builder, initialize);
        self.finalize_page_active_list(&mut builder)
    }

    /// Publishes a move-only whole list for one direct-chain suffix splice.
    pub fn publish_unique_page_nodes(
        &mut self,
        nodes: Vec<crate::node::Node>,
    ) -> crate::page_node_arena::UniquePageList {
        let etex_node_sizes = self.resident.engine_usage.uses_etex_node_sizes();
        let words = nodes.iter().fold((0_usize, 0_usize), |words, node| {
            let node_words = node.tex_memory_words(etex_node_sizes);
            (
                words.0.saturating_add(node_words.0),
                words.1.saturating_add(node_words.1),
            )
        });
        for node in &nodes {
            self.assert_live_node_font_roots(node);
        }
        let list = self
            .page_nodes
            .publish_owned_unique(nodes)
            .expect("page construction contains only live page-arena children");
        self.resident
            .engine_usage
            .observe_transient_memory(words.0, words.1);
        list
    }

    pub fn append_unique_page_nodes(
        &mut self,
        left: crate::page_node_arena::PageListSpan,
        right: crate::page_node_arena::UniquePageList,
    ) -> crate::page_node_arena::PageListSpan {
        self.page_nodes
            .append_unique_to_span(left, right)
            .expect("unique page suffix and checked left root share one live owner")
    }

    pub fn reclaim_unique_page_nodes(
        &self,
        span: crate::page_node_arena::PageListSpan,
    ) -> crate::page_node_arena::UniquePageList {
        self.page_nodes
            .reclaim_unique_span(span)
            .expect("consumed page-list owner retains an unlinked direct head")
    }

    pub fn reclaim_unique_page_list(
        &self,
        list: PageListId,
    ) -> crate::page_node_arena::UniquePageList {
        let span = self
            .page_nodes
            .admit_span(list)
            .expect("consumed page list belongs to the live owner");
        self.reclaim_unique_page_nodes(span)
    }

    /// Reclaims a consumed list only when its admitted direct head is
    /// unlinked. A sliced page-successor list remains valid but must be
    /// projected before splicing; stale or foreign lists remain errors.
    pub fn reclaim_unlinked_page_list(
        &self,
        list: PageListId,
    ) -> Result<Option<crate::page_node_arena::UniquePageList>, ForkArenaError> {
        let span = self.page_nodes.admit_span(list)?;
        match self.page_nodes.reclaim_unique_span(span) {
            Ok(unique) => Ok(Some(unique)),
            Err(ForkArenaError::InvalidRange) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Claims one consumed page box's child list for a single shallow unbox
    /// projection. The source wrapper was taken from a register or freshly
    /// copied to page material before this call.
    pub fn consumed_box_children(
        &mut self,
        wrapper: PageListId,
    ) -> crate::page_node_arena::ConsumedBoxChildren {
        self.page_nodes
            .consumed_box_children(wrapper)
            .expect("consumed unbox wrapper is one admitted page box")
    }

    /// Returns an exclusively page-owned equivalent of `list` for an
    /// operation that splits or splices it, materializing a borrowed box
    /// body (see `docs/shared_box_closures.md`).
    pub fn owned_page_list(&mut self, list: PageListId) -> PageListId {
        self.page_nodes
            .materialize_list(list)
            .expect("box body belongs to the page or a frozen region")
    }

    /// Converts move-only whole-list authority into an immutable embedded
    /// root without copying its nodes.
    pub fn publish_unique_page_list(
        &self,
        list: crate::page_node_arena::UniquePageList,
    ) -> PageListId {
        self.page_nodes.publish_unique_list(list)
    }

    /// Returns allocation/copy accounting for the canonical page-material
    /// arena owned by this admitted execution episode.
    #[must_use]
    pub fn page_material_counters(&self) -> crate::fork_arena::ForkArenaCounters {
        self.page_nodes.counters()
    }

    pub fn open_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) {
        self.page_nodes
            .open_active_list(builder)
            .expect("page active-list builder opens against its live owner");
    }

    pub fn push_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        node: crate::node::Node,
    ) {
        self.assert_live_node_font_roots(&node);
        self.page_nodes
            .push_active_list(builder, node)
            .expect("page active-list builder belongs to its live owner");
    }

    /// Initializes one generated node in the reserved final page slot.
    pub fn construct_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        initialize: impl FnOnce(crate::NodeDestination<'_>),
    ) {
        let metadata = self
            .page_nodes
            .construct_active_list(builder, initialize)
            .expect("page active-list builder belongs to its live owner");
        if let Some(font) = metadata.font {
            assert!(
                self.resident.fonts.contains(font),
                "durable node contains a font outside the admitted timeline"
            );
        }
        let words = if self.resident.engine_usage.uses_etex_node_sizes() {
            metadata.etex_words
        } else {
            metadata.tex82_words
        };
        self.resident
            .engine_usage
            .observe_transient_memory(words.0, words.1);
    }

    pub fn append_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        list: PageListId,
    ) {
        self.page_nodes
            .append_to_active_list(builder, list)
            .expect("page active-list source belongs to its live owner");
    }

    pub fn append_page_active_span(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        span: crate::page_node_arena::PageListSpan,
    ) {
        self.page_nodes
            .append_span_to_active_list(builder, span)
            .expect("checked page active-list span remains in its live owner");
    }

    pub fn append_unique_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        list: crate::page_node_arena::UniquePageList,
    ) {
        self.page_nodes
            .append_unique_active_list(builder, list)
            .expect("unique page active-list suffix belongs to its live owner");
    }

    pub fn append_page_active_list_range(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        list: PageListId,
        selected: core::ops::Range<usize>,
    ) {
        self.page_nodes
            .append_range_to_active_list(builder, list, selected)
            .expect("page active-list source range belongs to its live owner");
    }

    pub fn project_consumed_box_children(
        &mut self,
        source: crate::page_node_arena::ConsumedBoxChildren,
        remove_margin_kerns: bool,
    ) -> crate::page_node_arena::UniquePageList {
        self.page_nodes
            .project_consumed_box_children(source, remove_margin_kerns)
            .expect("consumed box child projection remains in its live page owner")
    }

    pub fn append_page_active_span_range(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
        span: crate::page_node_arena::PageListSpan,
        selected: core::ops::Range<usize>,
    ) {
        self.page_nodes
            .append_span_range_to_active_list(builder, span, selected)
            .expect("checked page active-list span range remains in its live owner");
    }

    pub fn finalize_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) -> PageListId {
        self.page_nodes
            .finalize_active_list(builder)
            .expect("page active-list builder belongs to its live owner")
    }

    pub fn finalize_unique_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) -> crate::page_node_arena::UniquePageList {
        self.page_nodes
            .finalize_unique_active_list(builder)
            .expect("page active-list builder belongs to its live owner")
    }

    pub fn finalize_generated_page_active_segment(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) -> crate::page_node_arena::FreshGeneratedSegment {
        self.page_nodes
            .finalize_generated_active_segment(builder)
            .expect("generated segment is finalized by its active builder")
    }

    pub fn finalize_page_active_span(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) -> crate::page_node_arena::PageListSpan {
        self.page_nodes
            .finalize_active_span(builder)
            .expect("page active-list builder belongs to its live owner")
    }

    pub fn rollback_page_active_list(
        &mut self,
        builder: &mut crate::page_node_arena::PageMaterialActiveListBuilder,
    ) {
        self.page_nodes
            .rollback_active_list(builder)
            .expect("page active-list rollback belongs to its live owner");
    }

    /// Flattens direct/composite descriptors into this generation's compact
    /// piece stream without copying node payload.
    pub fn compose_page_node_sequences(
        &mut self,
        inputs: &[crate::page_node_arena::PageListId],
    ) -> crate::page_node_arena::PageListId {
        self.page_nodes
            .compose_sequences(inputs)
            .expect("page sequence inputs belong to the live page arena")
    }

    /// Borrows an immutable logical subrange by publishing descriptors only.
    pub fn slice_page_node_sequence(
        &mut self,
        sequence: crate::page_node_arena::PageListId,
        range: core::ops::Range<usize>,
    ) -> crate::page_node_arena::PageListId {
        self.page_nodes
            .slice_sequence(sequence, range)
            .expect("page sequence range belongs to the live page arena")
    }

    /// Borrows an already-admitted logical subrange by publishing descriptors
    /// only and carries the checked result for its owner-local lifetime.
    pub fn slice_page_node_span(
        &mut self,
        span: crate::page_node_arena::PageListSpan,
        range: core::ops::Range<usize>,
    ) -> crate::page_node_arena::PageListSpan {
        self.page_nodes
            .slice_span(span, range)
            .expect("checked page span belongs to the live page arena")
    }

    /// Removes the sole semantic mode-list root and indexes its direct chunks
    /// once for monotonic generated-box windows. Callers must pass the actual
    /// mode-list slot after journaling its prior value, not a copied span.
    pub fn take_generated_mode_source(
        &mut self,
        slot: &mut crate::page_node_arena::ModePageListSlot,
    ) -> (
        crate::page_node_arena::PageListId,
        Option<crate::page_node_arena::ConsumedPageSource>,
    ) {
        self.page_nodes
            .take_generated_mode_source(slot)
            .expect("consumed mode source belongs to the live page arena")
    }

    pub fn append_fresh_mode_segment(
        &mut self,
        slot: &mut crate::page_node_arena::ModePageListSlot,
        suffix: crate::page_node_arena::FreshGeneratedSegment,
    ) {
        self.page_nodes
            .append_fresh_mode_segment(slot, suffix)
            .expect("fresh mode suffix belongs to the live page region");
    }

    pub fn replace_mode_with_fresh_segment(
        &mut self,
        slot: &mut crate::page_node_arena::ModePageListSlot,
        segment: crate::page_node_arena::FreshGeneratedSegment,
    ) {
        self.page_nodes
            .replace_mode_with_fresh_segment(slot, segment)
            .expect("fresh replacement belongs to the live page region");
    }

    pub fn truncate_mode_slot(
        &mut self,
        slot: &mut crate::page_node_arena::ModePageListSlot,
        end: usize,
    ) {
        self.page_nodes
            .truncate_mode_slot(slot, end)
            .expect("truncated mode root belongs to the live page region");
    }

    pub fn finish_generated_source_segments(
        &mut self,
        old: Option<crate::page_node_arena::ConsumedPageSource>,
        segments: Vec<crate::page_node_arena::FreshGeneratedSegment>,
    ) -> (
        crate::page_node_arena::PageListId,
        Option<crate::page_node_arena::ConsumedPageSource>,
    ) {
        self.page_nodes
            .finish_generated_source_segments(old, segments)
            .expect("fresh generated source segments belong to the live page region")
    }

    /// Indexes the final consumed semantic source only when its tape retains
    /// the original removed root.
    pub fn index_consumed_page_source(
        &mut self,
        source: &mut crate::page_node_arena::ConsumedPageSource,
    ) {
        self.page_nodes
            .index_consumed_source(source)
            .expect("consumed source remains in the live page region");
    }

    pub fn page_closure_transition_counters(
        &self,
    ) -> crate::node_region::ClosureTransitionCounters {
        self.page_nodes.closure_transition_counters()
    }

    /// Captures the final generated child chain's direct chunk geometry.
    #[must_use]
    pub fn generated_box_direct_chunk_selection(
        &self,
        root: crate::page_node_arena::PageListId,
    ) -> crate::page_node_arena::PageDirectChunkSelection {
        self.page_nodes
            .direct_root_chunk_selection(root)
            .expect("generated child root remains in the live page region")
    }

    pub fn append_generated_line_body(
        &mut self,
        window: crate::page_node_arena::ConsumedPageWindow,
        suffix: crate::page_node_arena::FreshGeneratedSegment,
    ) -> (
        crate::page_node_arena::PageListId,
        crate::page_node_arena::GeneratedLineBody,
    ) {
        self.page_nodes
            .append_generated_line_body(window, suffix)
            .expect("generated line source window and suffix belong to the page")
    }

    pub fn publish_generated_line_body_descriptor(
        &mut self,
        body: crate::page_node_arena::GeneratedLineBody,
        final_child: crate::page_node_arena::PageListId,
    ) -> Option<crate::page_node_arena::PublishedGeneratedBoxBody> {
        if !body.matches_final_child(final_child) {
            return None;
        }
        self.page_nodes
            .publish_generated_line_body_descriptor(body, final_child)
            .expect(
                "matched generated line receipt must publish or decline its unsupported geometry",
            )
    }

    pub fn stamp_generated_box_body(
        &mut self,
        root: crate::page_node_arena::PageListId,
        publication: crate::page_node_arena::PublishedGeneratedBoxBody,
    ) {
        self.page_nodes
            .stamp_published_generated_box_body(root, publication)
            .expect("generated box wrapper binds its positive selection");
    }

    #[must_use]
    pub fn page_node_semantic_identity_enabled(&self) -> bool {
        self.page_nodes.semantic_identity_enabled()
    }

    /// Reports canonical page-material lifecycle work for focused ownership
    /// and allocation gates. A retained source range changes only list
    /// descriptors, so `source_nodes_copied` remains an independently visible
    /// zero while genuinely new semantic output advances its own counter.
    #[must_use]
    pub fn page_node_arena_counters(&self) -> crate::fork_arena::ForkArenaCounters {
        self.page_nodes.counters()
    }

    /// Opens one nested structural suffix in the live page arena.
    #[must_use]
    pub fn begin_page_node_region(
        &mut self,
    ) -> crate::node_region::ClosureBuildMark<crate::node_region::PageRole> {
        self.page_nodes
            .begin_closure_build()
            .expect("page closure-build boundary is available")
    }

    /// Closes an anonymous box's paired construction ranges after its wrapper
    /// has been linked into the owning mode or page list.
    pub fn close_page_box_segment(
        &mut self,
        start: crate::node_region::PageClosureBuildMark,
    ) -> crate::node_region::PageBoxSegment {
        self.page_nodes
            .close_box_segment(start)
            .expect("completed box remains in its construction region")
    }

    pub fn rotate_page_box_wrapper_tail(&mut self) {
        self.page_nodes
            .rotate_box_wrapper_tail()
            .expect("box wrapper starts in its own paired chunks");
    }

    pub fn publish_page_box_migration_segments(
        &mut self,
        exclusions: &[crate::node_region::PageBoxSegment],
        obsolete_input_positions: &[usize],
    ) -> Option<crate::page_node_arena::PageBoxMigrationKey> {
        self.page_nodes
            .publish_box_migration_segments(exclusions, obsolete_input_positions)
            .expect("box migration exclusions belong to this page region")
    }

    pub fn stamp_page_box_segment(
        &mut self,
        start: &crate::node_region::PageClosureBuildMark,
        root: crate::page_node_arena::PageListId,
        migrations: Option<crate::page_node_arena::PageBoxMigrationKey>,
    ) -> crate::node_region::PageBoxSegment {
        self.page_nodes
            .stamp_box_segment(start, root, migrations)
            .expect("new box wrapper admits its original construction segment")
    }

    #[must_use]
    pub fn page_box_migration_metadata(
        &self,
        root: crate::page_node_arena::PageListId,
    ) -> Option<crate::page_node_arena::PageBoxMigrationMetadata> {
        self.page_nodes.box_migration_metadata(root)
    }

    #[must_use]
    pub fn page_box_segment(
        &self,
        root: crate::page_node_arena::PageListId,
    ) -> Option<crate::node_region::PageBoxSegment> {
        self.page_nodes.box_segment(root)
    }

    /// Checks the move-only boundary supplied by a consumed anonymous box.
    /// Invalid or shared intervals continue through the structural copy path.
    pub fn can_transfer_interleaved_page_box(
        &mut self,
        root: crate::page_node_arena::PageListId,
        metadata: &crate::page_node_arena::PageBoxMigrationMetadata,
        reset_shift: bool,
    ) -> bool {
        self.page_nodes
            .can_finish_interleaved_page_box(root, metadata, reset_shift)
    }

    /// Releases a complete structural suffix after its survivor has been
    /// promoted or detached.
    pub fn release_page_node_region(
        &mut self,
        region: crate::node_region::ClosureBuildMark<crate::node_region::PageRole>,
    ) -> Result<(), ForkArenaError> {
        self.page_nodes.cancel_closure_build(region)
    }

    /// Resolves one page-lifetime list while the admitted context is live.
    ///
    /// While a `\shipout\copy` source is pinned, lists inside its durable
    /// closure resolve too, so shipout reads the register in place.
    pub fn page_node_list(
        &self,
        list: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        match self.durable_boxes.shipout_source() {
            Some(source) => self
                .page_nodes
                .durable_node_cursor(source, list)
                .or_else(|_| self.page_nodes.node_cursor(list)),
            None => self.page_nodes.node_cursor(list),
        }
    }

    /// Reads box register `index` for `\shipout\copy` without copying it.
    ///
    /// Returns the root box with its child lists still in the register's
    /// durable closure, which stays pinned as a readable source until
    /// [`crate::Universe::release_shipout_source`]. The shipped page is not
    /// reachable state, so its lists need no page-minted semantic identity.
    /// Returns `None` when the caller must copy instead: a void register or
    /// the output-box carrier.
    pub fn pin_box_for_shipout(&mut self, index: u16) -> Option<crate::node::Node> {
        if index == u16::from(u8::MAX) || self.durable_boxes.shipout_source().is_some() {
            return None;
        }
        let source = self.durable_boxes.pin_shipout_source(index)?;
        let root = self
            .page_nodes
            .durable_list(source)
            .ok()
            .and_then(|list| list.get(0).map(|node| node.to_owned()));
        if root.is_none() {
            self.durable_boxes.release_shipout_source();
        }
        root
    }

    /// Exact top-level chunk coordinates of one admitted page list. This is
    /// used at hpack's consumed-input boundary, not as a live-root registry.
    pub fn page_list_chunk_positions(
        &self,
        list: PageListId,
    ) -> Result<Vec<usize>, ForkArenaError> {
        self.page_nodes.list_chunk_positions(list)
    }

    /// Validates one transport coordinate at a semantic ownership boundary.
    pub fn admit_page_node_span(
        &self,
        list: PageListId,
    ) -> Result<crate::page_node_arena::PageListSpan, ForkArenaError> {
        self.page_nodes.admit_span(list)
    }

    /// Admits one operation-local page list and retains its resolved compact
    /// endpoint proof for all traversal helpers in that operation.
    pub fn admit_page_node_list(
        &self,
        list: PageListId,
    ) -> Result<crate::page_node_arena::AdmittedPageList, ForkArenaError> {
        self.page_nodes.admit_page_list(list)
    }

    pub fn admitted_page_nodes(
        &self,
        list: crate::page_node_arena::AdmittedPageList,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        self.page_nodes.admitted_node_cursor(list)
    }

    pub fn admitted_page_tail_chunk(
        &self,
        list: crate::page_node_arena::AdmittedPageList,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, ForkArenaError> {
        self.page_nodes.admitted_tail_chunk(list)
    }

    /// Resolves a previously admitted span without rescanning its topology.
    pub fn page_node_span(
        &self,
        span: crate::page_node_arena::PageListSpan,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        self.page_nodes.span_node_cursor(span)
    }

    /// Starts a stack-resident direct packed-chunk walk of an admitted span.
    pub fn page_node_span_tail_chunk(
        &self,
        span: crate::page_node_arena::PageListSpan,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, ForkArenaError> {
        self.page_nodes.span_tail_chunk(span)
    }

    /// Follows one admitted page span's predecessor topology directly.
    pub fn page_node_span_previous_chunk(
        &self,
        cursor: &crate::page_node_arena::PageListChunkCursor,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, ForkArenaError> {
        self.page_nodes.span_previous_chunk(cursor)
    }

    /// Borrows one node from an already-admitted packed chunk without a
    /// logical-index lookup or repeated owner admission.
    pub fn page_node_span_chunk_node(
        &self,
        cursor: &crate::page_node_arena::PageListChunkCursor,
        offset: usize,
    ) -> (usize, crate::page_node_arena::PageMaterialNodeRef<'_>) {
        self.page_nodes.span_chunk_node_at(cursor, offset)
    }

    /// Advances one admitted page chunk by one resident record.
    pub fn page_node_span_next_chunk_node(
        &self,
        cursor: &mut crate::page_node_arena::PageListChunkCursor,
    ) -> Option<(usize, crate::page_node_arena::PageMaterialNodeRef<'_>)> {
        self.page_nodes.span_next_chunk_node(cursor)
    }

    pub fn page_node_sequence(
        &self,
        sequence: crate::page_node_arena::PageListId,
    ) -> Result<crate::node_view::NodeCursor<'_>, crate::fork_arena::ForkArenaError> {
        self.page_node_list(sequence)
    }

    /// Resolves shipout-only derived nodes while the aggregate transaction is
    /// live. No semantic state API accepts this coordinate type.
    pub fn shipout_scratch_nodes(
        &self,
        list: ShipoutScratchListId,
    ) -> Option<&[crate::ShipoutScratchNode]> {
        self.shipout_scratch.get(list)
    }

    /// Materializes one page closure as a self-contained shipout-scratch
    /// closure. The page coordinates are borrowed only during this call and
    /// cannot escape through the scratch node type.
    pub fn copy_page_list_to_shipout_scratch(
        &mut self,
        root: PageListId,
    ) -> Result<ShipoutScratchListId, ForkArenaError> {
        fn copy<G>(
            context: &mut CommandContext<'_, G>,
            source: PageListId,
            copied: &mut std::collections::HashMap<PageListId, ShipoutScratchListId>,
        ) -> Result<ShipoutScratchListId, ForkArenaError> {
            if let Some(destination) = copied.get(&source) {
                return Ok(*destination);
            }
            let nodes = context
                .page_node_list(source)?
                .iter()
                .map(|node| node.to_owned())
                .collect::<Vec<_>>();
            let destination = context.begin_shipout_scratch_list();
            copied.insert(source, destination);
            for node in nodes {
                let node = node.map_lists(|child| {
                    copy(context, child, copied)
                        .expect("page closure contains only live page-arena children")
                });
                context.push_shipout_scratch_node(destination, node);
            }
            Ok(destination)
        }

        copy(self, root, &mut std::collections::HashMap::new())
    }

    /// Opens one final shipout-scratch row for direct construction.
    pub fn begin_shipout_scratch_list(&mut self) -> ShipoutScratchListId {
        self.shipout_scratch.begin_list()
    }

    /// Appends directly to a final shipout-scratch row.
    pub fn push_shipout_scratch_node(
        &mut self,
        list: ShipoutScratchListId,
        node: crate::ShipoutScratchNode,
    ) {
        self.shipout_scratch.push(list, node);
    }

    /// Visits a deferred shipout token payload without materializing it.
    pub fn visit_shipout_tokens<E>(
        &self,
        source: crate::ShipoutTokenSource<G>,
        mut visit: impl FnMut(TokenWord) -> Result<(), E>,
    ) -> Result<(), E> {
        fn payload<'a, List, Glue, Tokens: Copy>(
            node: crate::NodeView<'a, List, Glue, Tokens>,
            field: crate::ShipoutTokenField,
        ) -> Option<Tokens> {
            match (node, field) {
                (
                    crate::NodeView::Whatsit(crate::node::Whatsit::DeferredWrite {
                        tokens, ..
                    }),
                    crate::ShipoutTokenField::DeferredWrite,
                )
                | (
                    crate::NodeView::Whatsit(crate::node::Whatsit::DeferredSpecial {
                        tokens, ..
                    }),
                    crate::ShipoutTokenField::DeferredSpecial,
                )
                | (
                    crate::NodeView::Whatsit(crate::node::Whatsit::DeferredPdfLiteral {
                        tokens,
                        ..
                    }),
                    crate::ShipoutTokenField::DeferredPdfLiteral,
                ) => Some(tokens),
                (
                    crate::NodeView::Whatsit(crate::node::Whatsit::PdfThread(thread)),
                    crate::ShipoutTokenField::PdfThreadAttributes,
                ) => Some(thread.attributes),
                _ => None,
            }
        }

        fn identifier<'a, List, Glue, Tokens: Copy>(
            node: crate::NodeView<'a, List, Glue, Tokens>,
            field: crate::ShipoutTokenField,
        ) -> Option<Tokens> {
            let identifier = match (node, field) {
                (
                    crate::NodeView::Whatsit(crate::node::Whatsit::PdfThread(thread)),
                    crate::ShipoutTokenField::PdfThreadIdentifier,
                ) => thread.identifier,
                (
                    crate::NodeView::Whatsit(crate::node::Whatsit::PdfDestination(destination)),
                    crate::ShipoutTokenField::PdfDestinationIdentifier,
                ) => destination.identifier,
                _ => return None,
            };
            match identifier {
                crate::node::NodePdfActionIdentifier::Name(tokens)
                | crate::node::NodePdfActionIdentifier::Raw(tokens) => Some(tokens),
                crate::node::NodePdfActionIdentifier::Number(_) => None,
            }
        }

        match source.list {
            crate::ShipoutListId::Page(list) => {
                let tokens = self
                    .page_node_list(list)
                    .ok()
                    .and_then(|list| list.get(source.index))
                    .expect("shipout token source belongs to the live page row");
                let key = identifier(tokens.clone(), source.field)
                    .or_else(|| payload(tokens, source.field))
                    .expect("shipout token field belongs to its page source");
                self.admitted
                    .node_token_words(key)
                    .expect("page shipout token key belongs to the admitted generation")
                    .iter()
                    .copied()
                    .try_for_each(&mut visit)
            }
            crate::ShipoutListId::Scratch(list) => {
                let node = self
                    .shipout_scratch
                    .get(list)
                    .and_then(|nodes| nodes.get(source.index))
                    .expect("shipout token source belongs to the active scratch row");
                let node = crate::NodeView::from(node);
                let key = identifier(node.clone(), source.field)
                    .or_else(|| payload(node, source.field))
                    .expect("shipout token field belongs to its scratch source");
                self.admitted
                    .node_token_words(key)
                    .expect("scratch shipout token key belongs to the admitted generation")
                    .iter()
                    .copied()
                    .try_for_each(&mut visit)
            }
        }
    }

    /// Admits a deferred page/scratch shipout token source to command
    /// expansion by streaming it once into its final durable semantic escape.
    pub fn admit_shipout_tokens(
        &mut self,
        source: crate::ShipoutTokenSource<G>,
    ) -> Result<TokenListId<G>, DurableAllocationError> {
        let builder = self.begin_token_list_builder()?;
        match source.list {
            crate::ShipoutListId::Page(list) => {
                let node = self
                    .page_node_list(list)
                    .expect("page shipout token row is live")
                    .get(source.index)
                    .expect("page shipout token index is live");
                let tokens = match (node, source.field) {
                    (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredWrite {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredWrite,
                    )
                    | (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredSpecial {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredSpecial,
                    )
                    | (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredPdfLiteral {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredPdfLiteral,
                    ) => tokens,
                    _ => panic!("page shipout token field matches its source node"),
                };
                self.admitted
                    .append_node_tokens_to_builder(&builder, tokens)?;
            }
            crate::ShipoutListId::Scratch(list) => {
                let node = self
                    .shipout_scratch
                    .get(list)
                    .and_then(|nodes| nodes.get(source.index))
                    .expect("scratch shipout token source is live");
                let tokens = match (crate::NodeView::from(node), source.field) {
                    (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredWrite {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredWrite,
                    )
                    | (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredSpecial {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredSpecial,
                    )
                    | (
                        crate::NodeView::Whatsit(crate::node::Whatsit::DeferredPdfLiteral {
                            tokens,
                            ..
                        }),
                        crate::ShipoutTokenField::DeferredPdfLiteral,
                    ) => tokens,
                    _ => panic!("scratch shipout token field matches its source node"),
                };
                self.admitted
                    .append_node_tokens_to_builder(&builder, tokens)?;
            }
        }
        self.seal_token_list_builder(builder)
    }

    /// Resolves the node slice consumed by pure typesetting kernels.
    pub fn page_nodes(
        &self,
        list: PageListId,
    ) -> Result<crate::node_view::NodeCursor<'_>, ForkArenaError> {
        self.page_node_list(list)
    }

    /// Returns the generation-checked owner of every page-list coordinate
    /// admitted by this command episode.
    #[must_use]
    pub const fn page_node_region_id(&self) -> crate::node_region::NodeRegionId {
        self.page_nodes.region_id()
    }

    /// Admits one complete node closure against this episode's page region.
    /// Direct parent/child links are checked when nodes are published, so a
    /// constant-time root check proves closure membership without a payload
    /// scan or a root census.
    #[must_use]
    pub fn admits_page_node_closure(&self, root: PageListId) -> bool {
        self.page_nodes.contains(root)
    }

    /// Exposes the immutable payload address only for cross-crate stability
    /// controls. Semantic code must use the borrowed node cursor instead.
    #[doc(hidden)]
    #[must_use]
    pub fn page_node_address_for_test(
        &self,
        root: PageListId,
        index: usize,
    ) -> Option<*const crate::node::Node> {
        self.page_nodes
            .node_cursor(root)
            .ok()?
            .testing_node_address(index)
    }

    /// Seals the owner identity after the executor has proved that its mode
    /// nest retains no page-list root. The resulting move-only receipt is the
    /// only production input accepted by page-region succession.
    #[doc(hidden)]
    #[must_use]
    pub fn seal_mode_list_region_preflight(&self) -> crate::page::ModeListRegionPreflight {
        crate::page::ModeListRegionPreflight {
            region: self.page_nodes.region_id(),
        }
    }

    /// Borrows the live page-builder sequence for diagnostic rendering only.
    pub fn current_page_nodes(&self) -> crate::node_view::NodeCursorIter<'_> {
        self.page.current_page(&self.page_nodes)
    }
}

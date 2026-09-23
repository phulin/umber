//! page commands operations on the existing impl owner.

use super::*;

impl<'a, G> CommandContext<'a, G> {
    /// Publishes one complete page-lifetime list inside this admitted episode.
    pub fn publish_page_nodes(&mut self, nodes: Vec<crate::node::Node>) -> PageListId {
        self.publish_page_node_range(nodes).list()
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

    /// Publishes one immutable payload segment inside this admitted episode.
    pub fn publish_page_node_range(
        &mut self,
        nodes: Vec<crate::node::Node>,
    ) -> crate::node_arena::PageNodeRange {
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
        let range = self
            .page_nodes
            .publish_range(nodes)
            .expect("page construction contains only live page-arena children");
        self.resident
            .engine_usage
            .observe_transient_memory(words.0, words.1);
        range
    }

    /// Flattens direct/composite descriptors into this generation's compact
    /// piece stream without copying node payload.
    pub fn compose_page_node_sequences(
        &mut self,
        inputs: &[crate::node_arena::PageNodeSequenceId],
    ) -> crate::node_arena::PageNodeSequenceId {
        self.page_nodes
            .compose_sequences(inputs)
            .expect("page sequence inputs belong to the live page arena")
    }

    /// Borrows an immutable logical subrange by publishing descriptors only.
    pub fn slice_page_node_sequence(
        &mut self,
        sequence: crate::node_arena::PageNodeSequenceId,
        range: core::ops::Range<usize>,
        scratch: &mut Vec<crate::node_arena::PageNodeSequenceId>,
    ) -> crate::node_arena::PageNodeSequenceId {
        self.page_nodes
            .slice_sequence(sequence, range, scratch)
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

    /// Starts a descriptor-only transform in caller-owned reusable scratch.
    pub fn begin_page_node_transform(
        &self,
        scratch: &mut crate::node_arena::PageNodeTransformScratch,
    ) {
        scratch.begin();
    }

    /// Appends an unchanged source range by coordinate only.
    pub fn retain_page_node_source_range(
        &mut self,
        scratch: &mut crate::node_arena::PageNodeTransformScratch,
        source: crate::node_arena::PageNodeSequenceId,
        range: core::ops::Range<usize>,
    ) {
        let piece = self
            .page_nodes
            .slice_sequence(source, range, &mut scratch.slices)
            .expect("retained source range belongs to the live page arena");
        if !piece.is_empty() {
            scratch.pieces.push(piece);
        }
    }

    /// Publishes genuinely new transform output once and appends its range.
    pub fn append_new_page_nodes(
        &mut self,
        scratch: &mut crate::node_arena::PageNodeTransformScratch,
        nodes: Vec<crate::node::Node>,
    ) {
        if nodes.is_empty() {
            return;
        }
        scratch.new_semantic_nodes = scratch.new_semantic_nodes.saturating_add(nodes.len());
        let range = self.publish_page_node_range(nodes);
        scratch.pieces.push(range);
    }

    /// Completes a transform by flattening only its compact descriptors.
    pub fn finish_page_node_transform(
        &mut self,
        scratch: &mut crate::node_arena::PageNodeTransformScratch,
    ) -> crate::node_arena::PageNodeSequenceId {
        let sequence = self
            .page_nodes
            .compose_sequences(&scratch.pieces)
            .expect("transform pieces belong to the live page arena");
        scratch.pieces.clear();
        scratch.slices.clear();
        sequence
    }

    /// Returns a whole payload segment to operation-local ownership without
    /// cloning it. Partial or shared-row extraction is deliberately rejected.
    pub fn take_page_node_range(
        &mut self,
        _range: crate::node_arena::PageNodeRange,
    ) -> Vec<crate::node::Node> {
        panic!("page-material ranges are immutable; callers must consume builders before sealing")
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

    /// Releases a complete structural suffix after its survivor has been
    /// promoted or detached.
    pub fn release_page_node_region(
        &mut self,
        region: crate::node_region::ClosureBuildMark<crate::node_region::PageRole>,
    ) -> Result<(), NodeArenaError> {
        self.page_nodes
            .cancel_closure_build(region)
            .map_err(|_| NodeArenaError::ForeignCursor)
    }

    /// Resolves one page-lifetime list while the admitted context is live.
    pub fn page_node_list(
        &self,
        list: PageListId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, NodeArenaError> {
        self.page_nodes
            .node_cursor(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Validates one transport coordinate at a semantic ownership boundary.
    pub fn admit_page_node_span(
        &self,
        list: PageListId,
    ) -> Result<crate::page_node_arena::PageListSpan, NodeArenaError> {
        self.page_nodes
            .admit_span(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Admits one operation-local page list and retains its resolved compact
    /// endpoint proof for all traversal helpers in that operation.
    pub fn admit_page_node_list(
        &self,
        list: PageListId,
    ) -> Result<crate::page_node_arena::AdmittedPageList, NodeArenaError> {
        self.page_nodes
            .admit_page_list(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    pub fn admitted_page_nodes(
        &self,
        list: crate::page_node_arena::AdmittedPageList,
    ) -> Result<crate::node_arena::NodeCursor<'_>, NodeArenaError> {
        self.page_nodes
            .admitted_node_cursor(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    pub fn admitted_page_tail_chunk(
        &self,
        list: crate::page_node_arena::AdmittedPageList,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, NodeArenaError> {
        self.page_nodes
            .admitted_tail_chunk(list)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Resolves a previously admitted span without rescanning its topology.
    pub fn page_node_span(
        &self,
        span: crate::page_node_arena::PageListSpan,
    ) -> Result<crate::node_arena::NodeCursor<'_>, NodeArenaError> {
        self.page_nodes
            .span_node_cursor(span)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Starts a stack-resident direct packed-chunk walk of an admitted span.
    pub fn page_node_span_tail_chunk(
        &self,
        span: crate::page_node_arena::PageListSpan,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, NodeArenaError> {
        self.page_nodes
            .span_tail_chunk(span)
            .map_err(|_| NodeArenaError::InvalidList)
    }

    /// Follows one admitted page span's predecessor topology directly.
    pub fn page_node_span_previous_chunk(
        &self,
        cursor: &crate::page_node_arena::PageListChunkCursor,
    ) -> Result<Option<crate::page_node_arena::PageListChunkCursor>, NodeArenaError> {
        self.page_nodes
            .span_previous_chunk(cursor)
            .map_err(|_| NodeArenaError::InvalidList)
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
        sequence: crate::node_arena::PageNodeSequenceId,
    ) -> Result<crate::node_arena::NodeCursor<'_>, crate::node_arena::NodeArenaError> {
        self.page_nodes
            .get_sequence(sequence)
            .map_err(|_| NodeArenaError::InvalidList)
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
    ) -> Result<ShipoutScratchListId, NodeArenaError> {
        fn copy<G>(
            context: &mut CommandContext<'_, G>,
            source: PageListId,
            copied: &mut std::collections::HashMap<PageListId, ShipoutScratchListId>,
        ) -> Result<ShipoutScratchListId, NodeArenaError> {
            if let Some(destination) = copied.get(&source) {
                return Ok(*destination);
            }
            let nodes = context
                .page_node_list(source)?
                .iter()
                .map(|node| node.to_owned_with(std::convert::identity))
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
                    .page_nodes
                    .node_cursor(list)
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
                    .page_nodes
                    .node_cursor(list)
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
    ) -> Result<crate::node_arena::NodeCursor<'_>, NodeArenaError> {
        self.page_nodes
            .node_cursor(list)
            .map_err(|_| NodeArenaError::InvalidList)
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
    pub fn current_page_nodes(&self) -> crate::node_arena::NodeCursorIter<'_> {
        self.page.current_page(&self.page_nodes)
    }
}

//! pdf commands operations on the existing impl owner.

use super::*;

impl<'a, G> CommandContext<'a, G> {
    pub fn pdf_uniform_deviate(&mut self, bound: i32) -> i32 {
        self.resident.world.pdf_uniform_deviate(bound)
    }

    pub fn pdf_normal_deviate(&mut self) -> i32 {
        self.resident.world.pdf_normal_deviate()
    }

    #[must_use]
    pub fn pdf_external_image(
        &self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageMetadata> {
        self.resident.pdf.external_image(id)
    }

    #[must_use]
    pub fn pdf_external_image_record(
        &self,
        id: crate::PdfExternalImageId,
    ) -> Option<crate::PdfExternalImageRecord> {
        self.resident.pdf.external_image_record(id)
    }

    pub fn allocate_pdf_external_image(
        &mut self,
        source: crate::PdfExternalImageSource,
        dimensions: crate::PdfExternalImageDimensions,
        color_space_object: i32,
        attributes: Vec<u8>,
    ) -> Result<crate::PdfExternalImageRecord, crate::PdfObjectCapacityError> {
        self.resident.pdf.allocate_external_image(
            source,
            dimensions,
            color_space_object,
            attributes,
        )
    }

    pub fn reserve_pdf_annotation(
        &mut self,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfObjectCapacityError> {
        self.resident.pdf.reserve_annotation()
    }

    pub fn initialize_pdf_annotation(
        &mut self,
        object: u32,
        data: crate::PdfAnnotationData<G>,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfAnnotationInitializeError> {
        let semantic_id = self.token_semantic_id(data.entries.clone());
        self.resident
            .pdf
            .initialize_annotation(object, data, semantic_id)
    }

    pub fn create_pdf_annotation(
        &mut self,
        data: crate::PdfAnnotationData<G>,
    ) -> Result<crate::PdfAnnotationRecord<G>, crate::PdfObjectCapacityError> {
        let object = self.resident.pdf.reserve_annotation()?.object();
        self.initialize_pdf_annotation(object, data)
            .map_err(|_| crate::PdfObjectCapacityError)
    }

    pub fn create_pdf_link(
        &mut self,
        dimensions: crate::PdfAnnotationDimensions,
        attributes: TokenListId<G>,
        action: crate::PdfActionSpec<G>,
        nesting_depth: usize,
    ) -> Result<crate::PdfLinkRecord<G>, crate::PdfObjectCapacityError> {
        let attributes_semantic_id = self.token_semantic_id(attributes.clone());
        let action_semantic_id = action.fingerprint(|tokens| self.token_semantic_id(tokens));
        self.resident.pdf.create_link(
            dimensions,
            attributes,
            action,
            attributes_semantic_id,
            action_semantic_id,
            u32::try_from(nesting_depth).unwrap_or(u32::MAX),
        )
    }

    pub fn end_pdf_link(&mut self) -> Option<crate::PdfOpenLink<G>> {
        self.resident.pdf.end_link()
    }

    pub fn create_pdf_outline(
        &mut self,
        attributes: TokenListId<G>,
        action: crate::PdfActionSpec<G>,
        count: i32,
        title: TokenListId<G>,
    ) -> Result<crate::PdfOutlineRecord<G>, crate::PdfObjectCapacityError> {
        let semantic_ids = [
            self.token_semantic_id(attributes.clone()),
            action.fingerprint(|tokens| self.token_semantic_id(tokens)),
            self.token_semantic_id(title.clone()),
        ];
        self.resident
            .pdf
            .create_outline(attributes, action, count, title, semantic_ids)
    }

    #[must_use]
    pub fn pdf_destination(
        &self,
        identity: &crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Option<crate::PdfDestinationRecord> {
        self.resident.pdf.destination(identity, structure)
    }

    pub fn reserve_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure: bool,
    ) -> Result<crate::PdfDestinationRecord, crate::PdfObjectCapacityError> {
        self.resident.pdf.reserve_destination(identity, structure)
    }

    pub fn reserve_pdf_thread(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<crate::PdfThreadRecord, crate::PdfObjectCapacityError> {
        self.resident.pdf.reserve_thread(identity)
    }

    #[must_use]
    pub fn pdf_raw_object(&self, object: u32) -> Option<crate::PdfRawObjectRecord<G>> {
        self.resident
            .pdf
            .raw_object(crate::PdfRawObjectId::from_allocated(object))
    }

    pub fn reserve_pdf_raw_object(
        &mut self,
    ) -> Result<crate::PdfRawObjectId, crate::PdfObjectCapacityError> {
        self.resident.pdf.reserve_raw_object()
    }

    pub fn initialize_pdf_raw_object(
        &mut self,
        id: crate::PdfRawObjectId,
        stream: bool,
        stream_attr: Option<TokenListId<G>>,
        file: bool,
        data: TokenListId<G>,
        immediate: bool,
    ) -> Result<(), crate::PdfRawObjectInitializeError> {
        let stream_attr = stream_attr.map(|tokens| self.pdf_token_parameter(tokens));
        let data = self.pdf_token_parameter(data);
        self.resident.pdf.initialize_raw_object(
            id,
            crate::PdfRawObjectData::new(stream, stream_attr, file, data),
            immediate,
        )
    }

    pub fn set_pdf_space_font_name(&mut self, name: Vec<u8>) {
        self.resident.pdf.set_space_font_name(name);
    }

    pub fn set_pdf_return_value(&mut self, value: i32) {
        self.resident.pdf.set_return_value(value);
    }

    pub fn has_pdf_color_stack(&mut self, id: u32) -> bool {
        self.resident.pdf.has_color_stack(id)
    }

    pub fn push_pdf_font_map(&mut self, operation: crate::PdfFontMapOperation) {
        self.resident.pdf.push_font_map(operation);
    }

    #[must_use]
    pub fn pdf_font_map_duplicate_names(&self) -> Vec<Vec<u8>> {
        self.resident.pdf.font_map_duplicate_names()
    }

    pub fn set_pdf_font_attribute(&mut self, font: crate::ids::FontId, bytes: Vec<u8>) {
        self.validate_font_root(font)
            .expect("PDF font attribute retains a live admitted font");
        self.resident.pdf.set_font_attribute(font, bytes);
    }

    pub fn include_pdf_font_chars(&mut self, font: crate::ids::FontId, chars: Vec<u8>) {
        self.validate_font_root(font)
            .expect("PDF character inclusion retains a live admitted font");
        self.resident.pdf.include_font_chars(font, chars);
    }

    pub fn disable_pdf_builtin_to_unicode(&mut self, font: crate::ids::FontId) {
        self.validate_font_root(font)
            .expect("PDF font configuration retains a live admitted font");
        self.resident.pdf.disable_builtin_to_unicode(font);
    }

    pub fn set_pdf_glyph_to_unicode(&mut self, mapping: crate::PdfGlyphToUnicode) {
        self.resident.pdf.set_glyph_to_unicode(mapping);
    }

    #[must_use]
    pub fn pdf_form_resource(&self, object: u32) -> Option<u32> {
        self.resident.pdf.form(object).map(|form| form.resource())
    }

    pub fn ensure_pdf_font_resource(
        &mut self,
        font: crate::ids::FontId,
    ) -> Result<crate::PdfFontResourceRecord, crate::PdfObjectCapacityError> {
        self.validate_font_root(font)
            .map_err(|_| crate::PdfObjectCapacityError)?;
        let recipe = self.resident.fonts.artifact_recipe(font);
        let identity = tex_fonts::PdfFontResourceIdentity::new(
            recipe.tfm_content_hash,
            recipe.opentype.map(|opentype| opentype.program_identity),
        );
        self.resident
            .pdf
            .ensure_font_resource(font, recipe.semantic_identity, identity)
    }

    pub fn reference_pdf_raw_object(
        &mut self,
        raw: u32,
    ) -> Result<(), crate::PdfRawObjectInitializeError> {
        self.resident
            .pdf
            .reference_raw_object(crate::PdfRawObjectId::from_allocated(raw))
    }

    #[must_use]
    pub fn pdf_form(&self, object: u32) -> Option<crate::PdfFormRecord<G>> {
        self.resident.pdf.form(object)
    }

    pub fn reserve_pdf_form(&mut self) -> Result<(u32, u32), crate::PdfObjectCapacityError> {
        self.resident.pdf.reserve_form()
    }

    pub fn initialize_pdf_form(
        &mut self,
        identity: (u32, u32),
        box_list: PageListId,
        dimensions: (Scaled, Scaled, Scaled),
        attr: Option<TokenListId<G>>,
        resources: Option<TokenListId<G>>,
        immediate: bool,
    ) -> Result<crate::PdfFormRecord<G>, crate::PdfObjectCapacityError> {
        let semantic_id = page_list_semantic_id(
            &self.page_nodes,
            &self.resident.fonts,
            &self.admitted,
            box_list,
        );
        let owner = self
            .page_nodes
            .copy_page_root_to_durable(box_list)
            .map_err(|_| crate::PdfObjectCapacityError)?;
        let attr = attr.map(|tokens| self.pdf_token_parameter(tokens));
        let resources = resources.map(|tokens| self.pdf_token_parameter(tokens));
        let record = self.resident.pdf.initialize_form(
            identity,
            semantic_id,
            dimensions,
            (attr, resources),
            immediate,
        );
        match record {
            Ok(record) => {
                self.durable_forms.insert(record.object(), owner);
                Ok(record)
            }
            Err(error) => {
                self.page_nodes
                    .retire_durable(owner)
                    .expect("rejected PDF form owner remains live");
                Err(error)
            }
        }
    }

    /// Initializes a PDF form by consuming the exact suffix opened before
    /// the source box left its durable register. A unique register therefore
    /// moves durable -> page -> form ownership without copying or relocating
    /// node payload; retained history pays only its already-counted copy.
    pub fn initialize_built_pdf_form(
        &mut self,
        identity: (u32, u32),
        source: (PageListId, crate::node_region::PageClosureBuildMark),
        dimensions: (Scaled, Scaled, Scaled),
        attr: Option<TokenListId<G>>,
        resources: Option<TokenListId<G>>,
        immediate: bool,
    ) -> Result<crate::PdfFormRecord<G>, crate::PdfObjectCapacityError> {
        let (box_list, build) = source;
        let semantic_id = page_list_semantic_id(
            &self.page_nodes,
            &self.resident.fonts,
            &self.admitted,
            box_list,
        );
        let owner = self
            .page_nodes
            .finish_built_page_root_to_durable_preserving_roots(
                build,
                box_list,
                self.page.payload_root_lists(),
            )
            .map_err(|_| crate::PdfObjectCapacityError)?;
        let attr = attr.map(|tokens| self.pdf_token_parameter(tokens));
        let resources = resources.map(|tokens| self.pdf_token_parameter(tokens));
        let record = self.resident.pdf.initialize_form(
            identity,
            semantic_id,
            dimensions,
            (attr, resources),
            immediate,
        );
        match record {
            Ok(record) => {
                self.durable_forms.insert(record.object(), owner);
                Ok(record)
            }
            Err(error) => {
                self.page_nodes
                    .retire_durable(owner)
                    .expect("rejected PDF form owner remains live");
                Err(error)
            }
        }
    }

    pub fn copy_pdf_form_to_page(&mut self, object: u32) -> Option<PageListId> {
        self.durable_forms
            .copy_to_page(&mut self.page_nodes, object)
            .expect("PDF form copy must succeed")
    }

    pub fn append_pdf_document_fragment(
        &mut self,
        kind: crate::PdfDocumentFragmentKind,
        tokens: TokenListId<G>,
    ) {
        let parameter = self.pdf_token_parameter(tokens);
        self.resident.pdf.append_document_fragment(kind, parameter);
    }

    #[must_use]
    pub fn pdf_catalog_open_action(&self) -> Option<crate::PdfActionRecord<G>> {
        self.resident.pdf.catalog_open_action()
    }

    pub fn set_pdf_catalog_open_action_with_targets(
        &mut self,
        action: crate::PdfActionSpec<G>,
        destination: Option<crate::PdfDestinationIdentity>,
        structure: Option<crate::PdfDestinationIdentity>,
        thread: Option<crate::PdfDestinationIdentity>,
    ) -> Result<crate::PdfActionRecord<G>, crate::PdfObjectCapacityError> {
        let fingerprint = action.fingerprint(|tokens| self.token_semantic_id(tokens));
        self.resident.pdf.set_catalog_open_action(
            action,
            fingerprint,
            destination,
            structure,
            thread,
        )
    }

    fn token_semantic_id(&self, tokens: TokenListId<G>) -> crate::state_hash::StateHashFragment {
        let words = self.token_list(tokens);
        crate::state_hash::StateHashFragment::from_exact_builder(0x7064_665f_746f_6b70, |hasher| {
            hasher.usize(words.len());
            for word in words {
                hasher.u32(word.raw());
            }
        })
    }

    fn pdf_token_parameter(&self, tokens: TokenListId<G>) -> crate::pdf::PdfTokenParameter<G> {
        crate::pdf::PdfTokenParameter {
            semantic_id: self.token_semantic_id(tokens.clone()),
            tokens,
        }
    }

    pub fn define_pdf_destination(
        &mut self,
        identity: crate::PdfDestinationIdentity,
        structure_target: Option<u32>,
    ) -> Result<crate::PdfDestinationDefinition, crate::PdfObjectCapacityError> {
        self.resident
            .pdf
            .define_destination(identity, structure_target)
    }

    pub fn append_pdf_thread_bead(
        &mut self,
        identity: crate::PdfDestinationIdentity,
    ) -> Result<(crate::PdfThreadRecord, crate::PdfThreadBeadRecord), crate::PdfObjectCapacityError>
    {
        self.resident.pdf.append_thread_bead(identity)
    }

    #[must_use]
    pub fn pdf_page_object(&self, page: u32) -> Option<u32> {
        page.checked_sub(1)
            .and_then(|index| self.resident.pdf.pages().get(index as usize))
            .map(crate::PdfPageRecord::page_object)
    }

    #[must_use]
    pub fn pdf_page_count(&self) -> usize {
        self.resident.pdf.pages().len()
    }

    /// Resolves the complete terminal PDF ledger while this generation is
    /// admitted. The result owns every token spelling, resource payload, and
    /// font identity needed after the generation is retired.
    pub fn detach_pdf_completion(
        &self,
    ) -> Result<crate::DetachedPdfCompletion, crate::PdfCompletionError> {
        let pages_entries = self
            .token_parameter(crate::env::banks::TokParam::PDF_PAGES_ATTR)
            .ok()
            .flatten()
            .map(|tokens| self.pdf_completion_token_bytes(tokens))
            .unwrap_or_default();
        let scalars = crate::pdf::completion::PdfCompletionScalars {
            engine_font_identities: self
                .font_artifact_recipes()
                .into_iter()
                .map(|recipe| recipe.semantic_identity)
                .collect(),
            font_configuration: self.pdf_font_configuration(),
            pages_entries,
            include_info_dictionary: self.int_param(IntParam::PDF_OMIT_INFO_DICT) == 0,
            include_dates: self.int_param(IntParam::PDF_INFO_OMIT_DATE) == 0,
            suppress_ptex_info: self.int_param(IntParam::PDF_SUPPRESS_PTEX_INFO),
            ptex_use_underscore: self.int_param(IntParam::PDF_PTEX_USE_UNDERSCORE) > 0,
            form_omit_procset: self.int_param(IntParam::PDF_OMIT_PROCSET),
            suppress_page_group_warning: self.int_param(IntParam::PDF_SUPPRESS_WARNING_PAGE_GROUP)
                != 0,
            clock: self.resident.world.job_clock(),
        };
        crate::pdf::completion::detach(
            &self.resident.pdf,
            scalars,
            |tokens| Ok(self.pdf_completion_token_bytes(tokens)),
            |font| self.resident.fonts.artifact_recipe(font),
            |font, code| self.resident.fonts.get(font).metrics().character(code),
            |font, number| self.font_parameter(font, number),
            |hash| {
                self.resident
                    .world
                    .read_artifact(hash)
                    .map_err(|error| error.to_string())
            },
        )
    }

    fn pdf_completion_token_bytes(&self, tokens: TokenListId<G>) -> Vec<u8> {
        let mut text = String::new();
        for word in self.token_list(tokens) {
            self.append_token_string_text(word.semantic_token(), &mut text);
        }
        text.into_bytes()
    }

    /// Detaches pdfTeX's unresolved navigation diagnostics before the
    /// admission is released for terminal publication.
    #[must_use]
    pub fn detach_pdf_navigation_warnings(&self) -> Vec<crate::PdfNavigationWarning> {
        self.resident.pdf.unresolved_navigation_warnings()
    }

    pub fn set_pdf_match_state(
        &mut self,
        haystack: Vec<u8>,
        captures: Vec<Option<(u32, u32)>>,
        slots: u32,
        matched: bool,
    ) {
        self.resident
            .pdf
            .set_match(haystack, captures, slots, matched);
    }

    #[must_use]
    pub fn pdf_match_capture(&self, index: u32) -> Option<(u32, &[u8])> {
        self.resident.pdf.match_capture(index)
    }

    pub fn allocate_pdf_color_stack(
        &mut self,
        mode: crate::PdfColorStackMode,
        restore_at_page_start: bool,
        initial: Vec<u8>,
    ) -> Result<u32, crate::PdfColorStackCapacityError> {
        self.resident
            .pdf
            .allocate_color_stack(mode, restore_at_page_start, initial)
    }

    pub fn apply_pdf_color_stack(
        &mut self,
        id: u32,
        target: crate::PdfColorStackTarget,
        action: &crate::PdfColorStackAction,
    ) -> Result<crate::PdfColorStackEmission, crate::PdfColorStackApplyError> {
        self.resident.pdf.apply_color_stack(id, target, action)
    }

    /// Applies a color-stack whatsit through its typed shipout coordinate.
    /// The source bytes remain in their semantic arena; `PdfState` allocates
    /// only the runtime value and detached emission that genuinely escape.
    pub fn apply_shipout_pdf_color_stack(
        &mut self,
        source: crate::ShipoutNodeSource<G>,
        id: u32,
        target: crate::PdfColorStackTarget,
    ) -> Result<crate::PdfColorStackEmission, crate::PdfColorStackApplyError> {
        fn action<List, Glue, Tokens>(
            node: crate::NodeView<'_, List, Glue, Tokens>,
            expected_id: u32,
        ) -> crate::PdfColorStackAction {
            let crate::NodeView::Whatsit(crate::node::Whatsit::PdfColorStack { id, action }) = node
            else {
                panic!("shipout color source is not a color-stack whatsit")
            };
            assert_eq!(id, expected_id, "shipout color source id changed");
            action
        }

        match source.list {
            crate::ShipoutListId::Page(list) => {
                let action = action(
                    self.page_nodes
                        .node_cursor(list)
                        .expect("page shipout color row is live")
                        .get(source.index)
                        .expect("page shipout color index is live"),
                    id,
                );
                self.resident.pdf.apply_color_stack(id, target, &action)
            }
            crate::ShipoutListId::Scratch(list) => {
                let action = action(
                    crate::NodeView::from(
                        self.shipout_scratch
                            .get(list)
                            .and_then(|nodes| nodes.get(source.index))
                            .expect("scratch shipout color source is live"),
                    ),
                    id,
                );
                self.resident.pdf.apply_color_stack(id, target, &action)
            }
        }
    }

    pub fn pdf_page_color_stack_restorations(&mut self) -> Vec<crate::PdfColorStackEmission> {
        self.resident.pdf.page_color_stack_restorations()
    }

    #[must_use]
    pub fn pdf_snap_reference(&self) -> (Scaled, Scaled) {
        self.resident.pdf.snap_reference()
    }

    pub fn publish_pdf_traversal_positions(
        &mut self,
        last_position: Option<(Scaled, Scaled)>,
        snap_reference: (Scaled, Scaled),
    ) {
        self.resident
            .pdf
            .publish_traversal_positions(last_position, snap_reference);
    }

    #[must_use]
    pub fn pdf_font_resource(
        &self,
        font: crate::ids::FontId,
    ) -> Option<crate::PdfFontResourceRecord> {
        self.resident.pdf.font_resource(font)
    }

    pub fn set_pdf_form_artifact(&mut self, object: u32, artifact: crate::PdfFormArtifact) {
        self.resident.pdf.set_form_artifact(object, artifact);
    }

    #[must_use]
    pub fn pdf_form_artifact(&self, object: u32) -> Option<crate::PdfFormArtifact> {
        self.resident.pdf.form_artifact(object)
    }

    #[must_use]
    pub fn pdf_form_color_rollback(&self) -> crate::PdfFormColorRollback {
        self.resident.pdf.form_color_rollback()
    }

    pub fn rollback_pdf_form_colors(&mut self, rollback: crate::PdfFormColorRollback) {
        self.resident.pdf.rollback_form_colors(rollback);
    }
}

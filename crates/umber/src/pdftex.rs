//! Pinned pdfTeX 1.40.29 engine-layer inventory and mode registration.

#[cfg(test)]
use tex_state::TokenListId;
use tex_state::Universe;
#[cfg(test)]
use tex_state::env::banks::{DimenParam, IntParam, TokParam};
#[cfg(test)]
use tex_state::meaning::{InternalInteger, Meaning, UnexpandablePrimitive};
#[cfg(test)]
use tex_state::scaled::Scaled;

pub(crate) fn install_pdftex_layer<G>(stores: &mut Universe<G>) {
    tex_command::install_pdftex_unexpandable_primitives(stores);
    tex_command::install_pdftex_expandable_primitives(stores);
}

/// Reconstructs pdfTeX's original primitive table after a format load without
/// replacing live meanings restored from the format image.
pub(crate) fn register_pdftex_layer<G>(stores: &mut Universe<G>) {
    tex_command::register_pdftex_unexpandable_primitives(stores);
    tex_command::register_pdftex_expandable_primitives(stores);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{
        prepare_etex_run_stores, prepare_latex_run_stores, prepare_pdftex_run_stores,
        prepare_run_stores,
    };
    use tex_state::meaning::ExpandablePrimitive;
    use tex_state::token::{Catcode, Token};
    use tex_state::{
        FileModificationDate, GenerationBrand, JobClock, PdfDocumentFragmentKind,
        ShellEscapePolicy, World,
    };

    #[derive(Clone, Copy)]
    enum StorePreparation {
        Tex,
        Etex,
        Pdftex,
        Latex,
    }

    impl StorePreparation {
        fn apply<G>(self, stores: &mut Universe<G>) {
            match self {
                Self::Tex => prepare_run_stores(stores),
                Self::Etex => prepare_etex_run_stores(stores),
                Self::Pdftex => prepare_pdftex_run_stores(stores),
                Self::Latex => prepare_latex_run_stores(stores),
            }
        }
    }

    struct PdftexTestStores<'a, G>(&'a mut Universe<G>);

    impl<G> core::ops::Deref for PdftexTestStores<'_, G> {
        type Target = Universe<G>;

        fn deref(&self) -> &Self::Target {
            self.0
        }
    }

    impl<G> core::ops::DerefMut for PdftexTestStores<'_, G> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.0
        }
    }

    impl<G> PdftexTestStores<'_, G> {
        fn context<R>(
            &mut self,
            use_context: impl FnOnce(&mut tex_state::CommandContext<'_, G>) -> R,
        ) -> R {
            let mut context = self.0.command_context().expect("admit pdfTeX test context");
            use_context(&mut context)
        }

        fn intern(&mut self, name: &str) -> tex_state::interner::SymbolId {
            self.0.intern(name).expect("intern pdfTeX test symbol")
        }

        fn meaning(
            &mut self,
            symbol: tex_state::interner::SymbolId,
        ) -> tex_state::ResolvedMeaning<G> {
            self.context(|context| context.meaning(symbol.symbol()))
        }

        fn set_meaning(&mut self, symbol: tex_state::interner::SymbolId, meaning: Meaning) {
            self.0
                .assign_meaning(
                    symbol,
                    tex_state::MeaningWord::from_static(meaning),
                    tex_state::AssignmentScope::Local,
                )
                .expect("assign pdfTeX test meaning");
        }

        fn snapshot(&mut self) -> tex_state::RuntimeCheckpoint<G> {
            self.0.runtime_checkpoint().expect("pdfTeX test checkpoint")
        }

        fn rollback(&mut self, checkpoint: &tex_state::RuntimeCheckpoint<G>) {
            self.0
                .restore_runtime_checkpoint_with_roots(checkpoint, || {})
                .expect("restore pdfTeX test checkpoint");
        }

        fn box_reg_ref(&mut self, index: u16) -> Option<tex_state::DurableNodeMetadata> {
            self.context(|context| context.box_register(index))
        }

        fn box_line_dimensions(&mut self, index: u16) -> Vec<(Scaled, Scaled)> {
            self.context(|context| {
                let root = context.copy_box_to_page(index).expect("setbox result");
                let root = context
                    .page_node_list(root)
                    .expect("box-register node list");
                let Some(tex_state::node::Node::VList(vbox)) = root.nodes().first() else {
                    panic!("box register is not a vbox");
                };
                context
                    .page_node_list(vbox.children)
                    .expect("vbox child list")
                    .nodes()
                    .iter()
                    .filter_map(|node| match node {
                        tex_state::node::Node::HList(line) => Some((line.height, line.depth)),
                        _ => None,
                    })
                    .collect()
            })
        }

        fn pdf_form(&mut self, object: u32) -> Option<tex_state::PdfFormRecord<G>> {
            self.context(|context| context.pdf_form(object))
        }

        fn pdf_form_artifact(&mut self, object: u32) -> Option<tex_state::PdfFormArtifact> {
            self.context(|context| context.pdf_form_artifact(object))
        }

        fn pdf_raw_object(&mut self, object: u32) -> Option<tex_state::PdfRawObjectRecord<G>> {
            self.context(|context| context.pdf_raw_object(object))
        }

        fn pdf_catalog_open_action(&mut self) -> Option<tex_state::PdfActionRecord<G>> {
            self.context(|context| context.pdf_catalog_open_action())
        }

        fn pdf_destination(
            &mut self,
            identity: &tex_state::PdfDestinationIdentity,
            structure: bool,
        ) -> Option<tex_state::PdfDestinationRecord> {
            self.context(|context| context.pdf_destination(identity, structure))
        }

        fn set_pdf_return_value(&mut self, value: i32) {
            self.context(|context| context.set_pdf_return_value(value));
        }

        fn pdf_internal(&mut self, integer: InternalInteger) -> i32 {
            self.context(|context| context.internal_integer(integer).expect("PDF integer"))
        }

        fn pdf_last_object(&mut self) -> u32 {
            self.pdf_internal(InternalInteger::PdfLastObject) as u32
        }

        fn pdf_last_form(&mut self) -> u32 {
            self.pdf_internal(InternalInteger::PdfLastXForm) as u32
        }

        fn pdf_last_position(&mut self) -> (Scaled, Scaled) {
            (
                Scaled::from_raw(self.pdf_internal(InternalInteger::PdfLastXPos)),
                Scaled::from_raw(self.pdf_internal(InternalInteger::PdfLastYPos)),
            )
        }

        fn pdf_return_value(&mut self) -> i32 {
            self.pdf_internal(InternalInteger::PdfReturnValue)
        }

        fn pdf_last_ximage(&mut self) -> u32 {
            self.pdf_internal(InternalInteger::PdfLastXImage) as u32
        }

        fn pdf_last_ximage_pages(&mut self) -> i32 {
            self.pdf_internal(InternalInteger::PdfLastXImagePages)
        }

        fn pdf_last_ximage_color_depth(&mut self) -> i32 {
            self.pdf_internal(InternalInteger::PdfLastXImageColorDepth)
        }

        fn allocate_pdf_external_image(
            &mut self,
            source: tex_state::PdfExternalImageSource,
            dimensions: tex_state::PdfExternalImageDimensions,
            color_space_object: i32,
        ) -> Result<tex_state::PdfExternalImageRecord, tex_state::PdfObjectCapacityError> {
            self.context(|context| {
                context.allocate_pdf_external_image(source, dimensions, color_space_object)
            })
        }

        fn set_int_param(&mut self, parameter: IntParam, value: i32) {
            self.context(|context| {
                context
                    .assign_int_param(parameter, value, tex_state::AssignmentScope::Local)
                    .expect("assign integer parameter")
            });
        }

        fn int_param(&mut self, parameter: IntParam) -> i32 {
            self.context(|context| context.int_param(parameter))
        }

        fn set_int_param_global(&mut self, parameter: IntParam, value: i32) {
            self.context(|context| {
                context
                    .assign_int_param(parameter, value, tex_state::AssignmentScope::Global)
                    .expect("assign global integer parameter")
            });
        }

        fn set_dimen_param(&mut self, parameter: DimenParam, value: Scaled) {
            self.context(|context| {
                context
                    .assign_dimen_param(parameter, value, tex_state::AssignmentScope::Local)
                    .expect("assign dimension parameter")
            });
        }

        fn dimen_param(&mut self, parameter: DimenParam) -> Scaled {
            self.context(|context| context.dimen_param(parameter))
        }

        fn tok_param(&mut self, parameter: TokParam) -> Option<TokenListId<G>> {
            self.context(|context| context.token_parameter(parameter).expect("token parameter"))
        }

        fn set_tok_param(&mut self, parameter: TokParam, value: TokenListId<G>) {
            self.context(|context| {
                context
                    .assign_token_parameter(
                        parameter,
                        Some(value),
                        tex_state::AssignmentScope::Local,
                    )
                    .expect("assign token parameter")
            });
        }

        fn intern_token_list(&mut self, tokens: &[Token]) -> TokenListId<G> {
            let words = tokens
                .iter()
                .copied()
                .map(tex_state::token::TokenWord::pack)
                .collect::<Vec<_>>();
            self.context(|context| {
                context
                    .allocate_token_list(&words)
                    .expect("allocate test token list")
            })
        }

        fn page_insertion_height(&mut self, class: u16) -> Option<Scaled> {
            self.context(|context| context.page_insertion(class).map(|row| row.height()))
        }

        fn pdf_font_configuration(&mut self) -> tex_state::PdfFontConfiguration {
            self.context(|context| context.pdf_font_configuration())
        }

        fn detached_pdf(&mut self) -> tex_state::DetachedPdfCompletion {
            self.context(|context| {
                context
                    .detach_pdf_completion()
                    .expect("detach pdfTeX test completion")
            })
        }

        fn pdf_next_object_id(&mut self) -> u32 {
            self.detached_pdf().next_object()
        }

        fn pdf_forms(&mut self) -> Vec<tex_state::DetachedPdfForm> {
            self.detached_pdf().forms().to_vec()
        }

        fn pdf_raw_objects(&mut self) -> Vec<tex_state::DetachedPdfRawObject> {
            self.detached_pdf().raw_objects().to_vec()
        }

        fn pdf_pages(&mut self) -> Vec<tex_state::DetachedPdfPage> {
            self.detached_pdf().pages().to_vec()
        }

        fn pdf_annotations(&mut self) -> Vec<tex_state::DetachedPdfAnnotation> {
            self.detached_pdf().annotations().to_vec()
        }

        fn pdf_links(&mut self) -> Vec<tex_state::DetachedPdfLink> {
            self.detached_pdf().links().to_vec()
        }

        fn pdf_destinations(&mut self, structure: bool) -> Vec<tex_state::PdfDestinationRecord> {
            let completion = self.detached_pdf();
            if structure {
                completion.structure_destinations().to_vec()
            } else {
                completion.destinations().to_vec()
            }
        }

        fn pdf_threads(&mut self) -> Vec<tex_state::PdfThreadRecord> {
            self.detached_pdf().threads().to_vec()
        }

        fn pdf_document_fragment(&mut self, kind: PdfDocumentFragmentKind) -> Vec<u8> {
            let completion = self.detached_pdf();
            let fragments = &completion.document().fragments;
            match kind {
                PdfDocumentFragmentKind::Info => fragments.info.clone(),
                PdfDocumentFragmentKind::Catalog => fragments.catalog.clone(),
                PdfDocumentFragmentKind::Names => fragments.names.clone(),
                PdfDocumentFragmentKind::Trailer => fragments.trailer.clone(),
                PdfDocumentFragmentKind::TrailerId => fragments.trailer_id.clone(),
            }
        }

        fn dump_format(&mut self) -> Result<tex_state::DetachedFormatImage, String> {
            crate::run_memory_collecting_initex_artifacts_with_profile(
                "\\dump",
                self,
                tex_command::CommandProfile::PDFTEX14029,
            )
            .map_err(|error| error.to_string())?
            .format_dump
            .map(|dump| dump.image)
            .ok_or_else(|| "format dump did not complete".to_owned())
        }
    }

    fn pdftex_primitive_names() -> Vec<&'static str> {
        tex_command::primitive_names(tex_command::PrimitiveProfile::Pdftex14029)
    }

    fn pdftex_parameters() -> Vec<tex_command::PrimitiveParameterView> {
        tex_command::primitive_parameter_views(tex_command::PrimitiveProfile::Pdftex14029)
    }

    fn run_pdf_memory<G>(
        source: &str,
        stores: &mut Universe<G>,
    ) -> Result<String, crate::SessionError> {
        // Every pinned oracle consumed by this module was generated with
        // `-interaction=nonstopmode`. Own that harness fact here instead of
        // weakening TeX82 §82's ErrorStop dialog or changing the production
        // retained-root runner, both of which must preserve caller policy.
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        crate::run_memory_with_stores_and_profile(
            source,
            stores,
            tex_command::CommandProfile::PDFTEX14029,
            false,
        )
    }

    fn run_pdf_memory_result<G>(
        source: &str,
        stores: &mut Universe<G>,
    ) -> Result<crate::RunResult, crate::SessionError> {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        crate::run_memory_collecting_artifacts_with_profile(
            source,
            stores,
            tex_command::CommandProfile::PDFTEX14029,
            false,
        )
    }

    fn complete_memory_terminal<G>(returned: &str, stores: &Universe<G>) -> String {
        let mut terminal =
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default())
                .into_owned();
        terminal.push_str(returned);
        terminal
    }

    /// The engine state these oracle comparisons run against.
    ///
    /// The pinned pdfTeX references are captured with
    /// `-interaction=nonstopmode`, and tex.web §75 would otherwise start the
    /// job in `error_stop_mode`: §82 enters §83's dialog on that alone, and
    /// §71 answers a memory terminal that has nothing in it with
    /// `fatal_error`, ending the run at its first recoverable diagnostic.
    fn with_pdftex_stores<R>(
        use_stores: impl for<'id> FnOnce(&mut PdftexTestStores<'_, GenerationBrand<'id>>) -> R,
    ) -> R {
        crate::with_engine_universe(|universe| use_stores(&mut PdftexTestStores(universe)))
            .expect("fresh pdfTeX test universe")
    }

    fn with_pdftex_oracle_stores<R>(
        use_stores: impl for<'id> FnOnce(&mut PdftexTestStores<'_, GenerationBrand<'id>>) -> R,
    ) -> R {
        with_pdftex_stores(|stores| {
            stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
            use_stores(stores)
        })
    }

    #[test]
    fn vbox_uses_signed_local_box_max_depth_saved_before_group_restore() {
        // TeX82 §§668/1086: `package` saves `box_max_depth` before
        // `unsave`, and `vpackage` may therefore assign a negative depth to
        // the new box. The positive and zero boxes guard ordinary paths.
        with_pdftex_oracle_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                "\\tracingonline=1\\showboxbreadth=10\\showboxdepth=10\
             \\boxmaxdepth=100pt\
             \\setbox0=\\vbox to10pt{\\boxmaxdepth=-1pt\\mark{negative}}\
             \\setbox1=\\vbox{\\boxmaxdepth=3pt\\hbox{\\vrule height10pt depth8pt}}\
             \\setbox2=\\vbox{}\\showbox0\\showbox1\\showbox2\\end",
                stores,
            )
            .expect("signed local boxmaxdepth run");
            let output = complete_memory_terminal(&output, stores);

            assert!(output.contains("\\vbox(10.0+-1.0)x0.0"), "{output}");
            assert!(output.contains("\\vbox(15.0+3.0)x0.4"), "{output}");
            assert!(output.contains("\\vbox(0.0+0.0)x0.0"), "{output}");
        });
    }

    #[test]
    fn source_derived_inventory_is_the_exact_pinned_158_name_set() {
        let document = include_str!("../../../docs/pdftex_primitives.md");
        let table = document
            .split("| PDF token-list parameters")
            .nth(1)
            .expect("source checklist starts")
            .split("Counts in the table sum to 158")
            .next()
            .expect("source checklist ends");
        let mut source_names = table
            .split('`')
            .skip(1)
            .step_by(2)
            .filter_map(|quoted| quoted.strip_prefix('\\'))
            .collect::<Vec<_>>();
        source_names.sort_unstable();
        source_names.dedup();

        let catalogue_names = pdftex_primitive_names();
        assert_eq!(catalogue_names.len(), 160);
        assert_eq!(
            catalogue_names
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            160,
            "the registered inventory must not contain duplicates",
        );
        let source_derived_catalogue = catalogue_names
            .into_iter()
            .filter(|name| !matches!(*name, "partokencontext" | "mubyte"))
            .collect::<Vec<_>>();
        assert_eq!(source_derived_catalogue, source_names);
    }

    #[test]
    fn form_names_have_exact_append_only_identity() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            for (name, expected) in [
                ("pdfxform", UnexpandablePrimitive::PdfXForm),
                ("pdfrefxform", UnexpandablePrimitive::PdfRefXForm),
            ] {
                let symbol = stores.intern(name);
                assert_eq!(
                    stores.meaning(symbol),
                    Meaning::UnexpandablePrimitive(expected),
                );
            }
            assert_eq!(UnexpandablePrimitive::PdfXForm.operand(), 251);
            assert_eq!(UnexpandablePrimitive::PdfRefXForm.operand(), 252);
            assert_eq!(InternalInteger::PdfLastXForm.operand(), 16);
            assert_eq!(ExpandablePrimitive::PdfXFormName.operand(), 84);
        });
    }

    #[test]
    fn pdfxform_consumes_box_and_captures_options_and_dimensions() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\setbox0=\\hbox to 10pt{}",
                    "\\pdfxform attr {/A 1} resources {/R 2} 0",
                    "\\message{last=\\the\\pdflastxform,name=\\pdfxformname\\pdflastxform}",
                    "\\pdfrefxform 1\\end",
                ),
                stores,
            )
            .expect("scan and reference a PDF form");
            assert!(stores.box_reg_ref(0).is_none());
            let form = stores.pdf_form(1).expect("captured form");
            assert_eq!(form.resource(), 1);
            assert_eq!(form.width(), Scaled::from_raw(10 * 65_536));
            assert!(form.attr().is_some());
            assert!(form.resources().is_some());
            let output = run_pdf_memory(
                "\\message{name=\\pdfxformname1,last=\\the\\pdflastxform}\\end",
                stores,
            )
            .expect("expand form enquiries");
            assert_eq!(output, " name=1,last=1");
        });
    }

    #[test]
    fn pdfxform_rejects_void_boxes_and_dvi_mode() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let error = run_pdf_memory("\\pdfoutput=1\\pdfxform0\\end", stores)
                .expect_err("void form box must fail");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (ext1): \\pdfxform cannot be used with a void box"
            );
            run_pdf_memory("\\setbox0=\\hbox{}\\pdfxform0\\end", stores)
                .expect("form allocation continues after the failed reserved identity");
            let form = stores
                .pdf_form(3)
                .expect("second object and resource are retained");
            assert_eq!(form.resource(), 2);

            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let error = run_pdf_memory("\\pdfxform0\\end", stores)
                    .expect_err("DVI mode must reject forms");
                assert_eq!(
                    error.to_string(),
                    "pdfTeX error (\\pdfxform): not allowed in DVI mode (\\pdfoutput <= 0)."
                );
            });
        });
    }

    #[test]
    fn void_form_error_commits_reservation_but_checkpoint_rollback_reclaims_it() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let baseline = stores.snapshot();

            let error = run_pdf_memory("\\pdfoutput=1\\pdfxform0\\end", stores)
                .expect_err("void form reports ext1");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (ext1): \\pdfxform cannot be used with a void box"
            );
            assert_eq!(stores.pdf_next_object_id(), 3);
            assert_eq!(stores.pdf_last_form(), 0);

            stores.rollback(&baseline);
            assert_eq!(stores.pdf_next_object_id(), 1);

            run_pdf_memory("\\pdfoutput=1\\pdfxform0\\end", stores)
                .expect_err("replayed void form reserves the same identities");
            run_pdf_memory("\\setbox0=\\hbox{}\\pdfxform0\\end", stores)
                .expect("recovery allocates after the retained reservation");
            let form = stores.pdf_form(3).expect("recovered form uses object 3");
            assert_eq!(form.resource(), 2);
        });
    }

    #[test]
    fn pdf_forms_rollback_and_replay_reuse_canonical_identity() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let snapshot = stores.snapshot();
            let source = "\\pdfoutput=1\\setbox0=\\hbox{}\\pdfxform0\\end";
            run_pdf_memory(source, stores).expect("first form run");
            assert_eq!(stores.pdf_last_form(), 1);
            stores.rollback(&snapshot);
            assert_eq!(stores.pdf_last_form(), 0);
            assert!(stores.pdf_forms().into_iter().next().is_none());
            run_pdf_memory(source, stores).expect("replayed form run");
            assert_eq!(stores.pdf_last_form(), 1);
        });
    }

    #[test]
    fn pdf_xform_name_enquiry_is_checkpointed_and_does_not_allocate() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let baseline = stores.snapshot();
            let output = run_pdf_memory(
                concat!(
                    "\\pdfoutput=1\\setbox0=\\hbox{}\\pdfxform0",
                    "\\message{name=\\pdfxformname1,missing=\\pdfxformname2}\\end",
                ),
                stores,
            )
            .expect("form resource enquiries expand");
            assert!(output.contains("name=1,missing=0"), "{output}");
            assert_eq!(stores.pdf_last_form(), 1);
            assert_eq!(
                stores.pdf_last_object(),
                0,
                "form resource enquiries do not alter the raw-object enquiry"
            );

            stores.rollback(&baseline);
            assert!(stores.pdf_forms().into_iter().next().is_none());
            assert_eq!(stores.pdf_last_object(), 0);
        });
    }

    #[test]
    fn lazy_pdf_form_created_inside_box_build_survives_until_shipout_normalization() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1 ",
                    "\\setbox9=\\hbox{\\setbox0=\\hbox{}\\pdfxform0} ",
                    "\\shipout\\hbox{\\pdfrefxform1}\\end",
                ),
                stores,
            )
            .expect("the form ledger retains its consumed box through outer box teardown");

            assert!(stores.pdf_form_artifact(1).is_some());
            assert_eq!(stores.world().artifact_commits().len(), 1);
        });
    }

    #[test]
    fn pdf_form_state_and_diagnostics_match_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_form_state", "ref");
        let expected = [
            "initial=0",
            "h-form=1/1/131072,131072/void=yes",
            "v-form=3/0,262144",
            "math-form=5/65536,131072",
            "lazy-before=7/65536,131072",
            "lazy-after=7/4/196608,131072",
        ];
        for line in expected {
            assert!(
                reference.contains(line),
                "oracle missing {line:?}: {reference}"
            );
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            stores
                .world_mut()
                .set_memory_file(
                    "cmsy10.tfm",
                    include_bytes!("../../tex-fonts/tests/fixtures/cm/cmsy10.tfm").to_vec(),
                )
                .expect("seed symbol font fixture");
            stores
                .world_mut()
                .set_memory_file(
                    "cmex10.tfm",
                    include_bytes!("../../tex-fonts/tests/fixtures/cm/cmex10.tfm").to_vec(),
                )
                .expect("seed extension font fixture");
            let output = run_pdf_memory(
                include_str!("../../../tests/corpus/tex_exec/pdf_form_state/pdf_form_state.tex"),
                stores,
            )
            .expect("execute pinned form-state fixture");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
            for line in expected {
                assert!(
                    observed.contains(line),
                    "Umber missing {line:?}: {observed}"
                );
            }
            assert_eq!(
                stores
                    .pdf_forms()
                    .into_iter()
                    .map(|form| (form.object, form.resource))
                    .collect::<Vec<_>>(),
                [(1, 1), (3, 2), (5, 3), (7, 4)]
            );

            let diagnostic = test_support::read_fixture("tex_exec", "pdf_form_diagnostics", "ref");
            assert!(
                diagnostic
                    .contains("pdfTeX error (ext1): \\pdfxform cannot be used with a void box.")
            );
            let traversal_diagnostic =
                test_support::read_fixture("tex_exec", "pdf_form_traversal_diagnostics", "ref");
            assert!(traversal_diagnostic.contains("1 unmatched \\pdfsave after form ship"));
            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let error = run_pdf_memory(
            include_str!(
                "../../../tests/corpus/tex_exec/pdf_form_diagnostics/pdf_form_diagnostics.tex"
            ),
            stores,
        )
        .expect_err("void form fixture must fail");
                assert_eq!(
                    error.to_string(),
                    "pdfTeX error (ext1): \\pdfxform cannot be used with a void box"
                );
            });
        });
    }

    #[test]
    fn immediate_and_lazy_form_positions_publish_and_rollback_together() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let baseline = stores.snapshot();

            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\setbox0=\\hbox{\\kern2pt\\pdfsavepos}",
                    "\\immediate\\pdfxform0\\end",
                ),
                stores,
            )
            .expect("immediate form traverses at creation");
            let immediate_position = stores.pdf_last_position();
            assert_eq!(
                immediate_position,
                (Scaled::from_raw(2 * 65_536), Scaled::from_raw(0))
            );
            assert_eq!(
                stores
                    .pdf_form_artifact(1)
                    .expect("immediate artifact is published")
                    .last_position(),
                Some(immediate_position)
            );

            run_pdf_memory(
                "\\setbox0=\\hbox{\\kern3pt\\pdfsavepos}\\pdfxform0\\end",
                stores,
            )
            .expect("lazy form is captured");
            assert!(stores.pdf_form_artifact(3).is_none());
            assert_eq!(stores.pdf_last_position(), immediate_position);

            stores.rollback(&baseline);
            assert_eq!(
                stores.pdf_last_position(),
                (Scaled::from_raw(0), Scaled::from_raw(0))
            );
            assert!(stores.pdf_forms().into_iter().next().is_none());
            assert!(stores.pdf_form_artifact(1).is_none());
            assert!(stores.pdf_form_artifact(3).is_none());

            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                run_pdf_memory(
                    concat!(
                        "\\pdfoutput=1",
                        "\\setbox0=\\hbox{\\kern3pt\\pdfsavepos}\\pdfxform0",
                        "\\shipout\\hbox{\\pdfrefxform1}\\end",
                    ),
                    stores,
                )
                .expect("lazy form traverses on first reference");
                let lazy_position = stores
                    .pdf_form_artifact(1)
                    .expect("lazy artifact is published")
                    .last_position()
                    .expect("lazy form publishes its save position");
                assert_eq!(
                    lazy_position,
                    (Scaled::from_raw(3 * 65_536), Scaled::from_raw(0))
                );
                assert_eq!(stores.pdf_last_position(), lazy_position);
            });
        });
    }

    #[test]
    fn pdf_objects_reserve_initialize_reference_and_report_last_object() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\pdfobj reserveobjnum",
                    "\\pdfobj useobjnum 1 stream attr {/Subtype /XML} {payload}",
                    "\\pdfrefobj 1",
                    "\\immediate\\pdfobj {42}",
                    "\\end",
                ),
                stores,
            )
            .expect("execute raw PDF objects");

            assert_eq!(stores.pdf_last_object(), 2);
            let records = stores.pdf_raw_objects();
            assert_eq!(records.len(), 2);
            let first = &records[0];
            assert_eq!(first.object, 1);
            assert!(matches!(
                first.payload,
                Some(tex_state::DetachedPdfRawObjectPayload::Stream { .. })
            ));
            assert!(first.referenced);
            assert!(!first.immediate);
            assert_eq!(records[1].object, 2);
            assert!(records[1].immediate);
        });
    }

    #[test]
    fn pdf_accessibility_controls_scan_globally_and_reject_dvi_mode() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\def\\spacename{fixture}",
                    "{\\pdfspacefont{\\spacename-space}}",
                    "\\shipout\\hbox{a\\pdfinterwordspaceon b\\pdffakespace",
                    "\\pdfinterwordspaceoff c}",
                    "\\end",
                ),
                stores,
            )
            .expect("execute PDF accessibility controls");
            let pages = stores.pdf_pages();
            let page = &pages[0];
            assert_eq!(page.space_font_name, b"fixture-space");

            for primitive in [
                "\\pdfinterwordspaceon",
                "\\pdfinterwordspaceoff",
                "\\pdffakespace",
                "\\pdfspacefont{fixture}",
            ] {
                with_pdftex_stores(|stores| {
                    prepare_pdftex_run_stores(stores);
                    let error = run_pdf_memory(&format!("\\pdfoutput=0{primitive}\\end"), stores)
                        .expect_err("PDF-only accessibility primitive must fail in DVI mode");
                    assert!(
                        error.to_string().contains("not allowed in DVI mode"),
                        "{primitive}: {error}"
                    );
                });
            }
        });
    }

    #[test]
    fn pdf_annotations_and_links_allocate_pair_and_anchor_typed_effects() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\pdfannot reserveobjnum",
                    "\\message{a=\\the\\pdflastannot/l=\\the\\pdflastlink}",
                    "\\shipout\\hbox{",
                    "\\pdfannot useobjnum 1 width 10pt {/Subtype /Text}",
                    "\\pdfstartlink height 6pt attr{/Border [0 0 0]}",
                    "user{/Subtype /Link /A << /S /URI /URI (u) >>}",
                    "\\pdfrunninglinkoff X\\pdfrunninglinkon\\pdfendlink}",
                    "\\message{A=\\the\\pdflastannot/L=\\the\\pdflastlink}",
                    "\\end",
                ),
                stores,
            )
            .expect("annotation and link lifecycle");
            assert!(output.contains("A=1/L=2"), "{output}");
            assert_eq!(stores.pdf_annotations().len(), 1);
            assert_eq!(stores.pdf_links().len(), 1);

            let hash = stores.world().artifact_commits()[0];
            let bytes = stores
                .world()
                .read_artifact(hash)
                .expect("artifact read")
                .expect("artifact exists");
            let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
            assert_eq!(
                artifact
                    .effects
                    .iter()
                    .filter_map(|effect| match effect {
                        tex_out::PageEffect::PdfAnnotation(marker) => Some(*marker),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                vec![
                    tex_out::PdfAnnotationEffect::Annotation { object: 1 },
                    tex_out::PdfAnnotationEffect::LinkStart { object: 2 },
                    tex_out::PdfAnnotationEffect::RunningLink(false),
                    tex_out::PdfAnnotationEffect::RunningLink(true),
                    tex_out::PdfAnnotationEffect::LinkEnd { object: 2 },
                ]
            );
        });
    }

    #[test]
    fn pdf_link_level_mismatch_warns_and_closes_the_active_link() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfoutput=1 X\\hbox{\\pdfstartlink user{/Subtype /Link}",
                    "inside}\\pdfendlink\\end",
                ),
                stores,
            )
            .expect("level mismatch is recoverable");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            // §58 breaks this warning at `max_print_line`; the subject here is
            // the diagnostic's content, not its layout.
            let observed = tex_state::print::without_line_breaks(&format!(
                "{}{}",
                String::from_utf8_lossy(terminal),
                output
            ));
            assert!(
            observed.contains(
                "pdfTeX warning: \\pdfendlink ended up in different nesting level than \\pdfstartlink"
            ),
            "{observed}"
        );
        });
    }

    #[test]
    fn pdf_destinations_claim_on_ship_and_use_positive_only_duplicate_suppression() {
        const WARNING: &str = "\npdfTeX warning (ext4): destination with the same identifier (name{same}) has been already used, duplicate ignored\n";
        for (suppression, warns) in [(-1, true), (0, true), (1, false)] {
            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let output = run_pdf_memory(
                &format!(
                    "\\pdfoutput=1\\setbox0=\\hbox{{\\pdfdest name{{same}} fit\\pdfdest name{{same}} fit}}\\pdfsuppresswarningdupdest={suppression}\\shipout\\box0\\end"
                ),
                stores,
            )
            .expect("destination duplicate is recoverable");
                let terminal = tex_state::print::without_line_breaks(&complete_memory_terminal(
                    &output, stores,
                ));
                let expected = if warns {
                    format!("(texput [0{WARNING}]")
                } else {
                    "(texput [0]".to_owned()
                };
                assert_eq!(terminal, expected);
                assert_eq!(stores.pdf_destinations(false).len(), 1);
                assert!(stores.pdf_destinations(false)[0].defined());
                let bytes = stores
                    .world()
                    .read_artifact(stores.world().artifact_commits()[0])
                    .expect("artifact read")
                    .expect("artifact exists");
                let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
                assert_eq!(
                    artifact
                        .effects
                        .iter()
                        .filter(|effect| matches!(effect, tex_out::PageEffect::PdfDestination(_)))
                        .count(),
                    1
                );
            });
        }
    }

    #[test]
    fn pdf_destination_duplicates_keep_identity_order_grouping_and_output_routing() {
        const NAME_WARNING: &str = "\npdfTeX warning (ext4): destination with the same identifier (name{7}) has been already used, duplicate ignored\n";
        const NUMBER_WARNING: &str = "\npdfTeX warning (ext4): destination with the same identifier (num7) has been already used, duplicate ignored\n";
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\shipout\\hbox{\\pdfdest name{7} fit\\pdfdest num 7 fit}",
                    "{\\pdfsuppresswarningdupdest=1",
                    "\\shipout\\hbox{\\pdfdest name{7} fit}}",
                    "\\shipout\\hbox{\\pdfdest num 7 fit}",
                    "\\shipout\\hbox{\\pdfdest name{7} fit}\\end",
                ),
                stores,
            )
            .expect("destination duplicates are recoverable");

            let expected = format!("(texput [0] [0]{NUMBER_WARNING}[0]{NAME_WARNING}[0]");
            assert_eq!(
                tex_state::print::without_line_breaks(&complete_memory_terminal(&output, stores)),
                expected
            );
            assert_eq!(
                tex_state::print::without_line_breaks(&String::from_utf8_lossy(
                    stores.world().memory_log_output().unwrap_or_default(),
                )),
                expected
                    .strip_prefix("(texput")
                    .expect("terminal-only startup framing")
            );

            assert_eq!(stores.pdf_destinations(false).len(), 2);
            assert!(
                stores
                    .pdf_destinations(false)
                    .iter()
                    .all(|record| record.defined())
            );
            let destination_effects = stores
                .world()
                .artifact_commits()
                .iter()
                .map(|&hash| {
                    stores
                        .world()
                        .read_artifact(hash)
                        .expect("artifact read")
                        .expect("artifact exists")
                })
                .map(|bytes| tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses"))
                .map(|artifact| {
                    artifact
                        .effects
                        .iter()
                        .filter(|effect| matches!(effect, tex_out::PageEffect::PdfDestination(_)))
                        .count()
                })
                .collect::<Vec<_>>();
            assert_eq!(destination_effects, [2, 0, 0, 0]);
        });
    }

    #[test]
    fn pdfpageref_expands_to_shipped_page_object_and_zero_for_missing_pages() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let source = "\\pdfoutput=1\\shipout\\hbox{}\\message{page=\\pdfpageref1,missing=\\pdfpageref2}\\end";
            let output = run_pdf_memory(source, stores).expect("pdfpageref run");
            let page_object = stores.pdf_pages()[0].page_object;
            assert!(
                output.contains(&format!("page={page_object},missing=0")),
                "{output}"
            );

            with_pdftex_stores(|missing_stores| {
                prepare_pdftex_run_stores(missing_stores);
                let missing = run_pdf_memory(
                    "\\pdfoutput=1\\message{rolledback=\\pdfpageref1}\\end",
                    missing_stores,
                )
                .expect("absent page enquiry");
                assert!(missing.contains("rolledback=0"), "{missing}");

                with_pdftex_stores(|replay_stores| {
                    prepare_pdftex_run_stores(replay_stores);
                    let replay =
                        run_pdf_memory(source, replay_stores).expect("replay shipped page enquiry");
                    assert_eq!(replay_stores.pdf_pages()[0].page_object, page_object);
                    assert_eq!(replay, output);

                    with_pdftex_stores(|invalid| {
                        prepare_pdftex_run_stores(invalid);
                        let error = run_pdf_memory(
                            "\\pdfoutput=1\\message{invalid=\\pdfpageref0}\\end",
                            invalid,
                        )
                        .expect_err("nonpositive page references are fatal");
                        assert_eq!(
                            error.to_string(),
                            "pdfTeX error (pageref): invalid page number"
                        );
                        assert!(invalid.pdf_pages().is_empty());
                    });
                });
            });
        });
    }

    #[test]
    fn pdf_article_thread_scanner_allocates_and_commits_typed_beads() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1\\pdfthreadmargin=2pt",
                    "\\shipout\\hbox{\\pdfthread depth 3pt width 10pt height 4pt ",
                    "attr{/I << /Title (custom) >>} name{chapter}X}\\end",
                ),
                stores,
            )
            .expect("thread scanner and shipout");
            let threads = stores.pdf_threads();
            let thread = &threads[0];
            assert_eq!(thread.beads().len(), 1);
            let bytes = stores
                .world()
                .read_artifact(stores.world().artifact_commits()[0])
                .expect("artifact read")
                .expect("artifact exists");
            let artifact = tex_out::PageArtifact::from_bytes(&bytes).expect("artifact parses");
            let marker = artifact
                .effects
                .iter()
                .find_map(|effect| match effect {
                    tex_out::PageEffect::PdfThread(marker) => Some(marker),
                    _ => None,
                })
                .expect("typed thread effect");
            assert_eq!(marker.thread_object, thread.object());
            assert_eq!(
                marker.margin,
                tex_state::scaled::Scaled::from_raw(2 * 65_536)
            );
            assert_eq!(marker.attributes, b"/I << /Title (custom) >>");
        });
    }

    #[test]
    fn local_thread_actions_reserve_thread_objects_without_destination_aliases() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1",
                    "\\pdfoutline thread num 17 {Missing thread}",
                    "\\shipout\\hbox{}\\end",
                ),
                stores,
            )
            .expect("thread action scans");
            assert_eq!(stores.pdf_threads().len(), 1);
            assert_eq!(
                stores.pdf_threads()[0].identity(),
                &tex_state::PdfDestinationIdentity::Number(17)
            );
            assert!(stores.pdf_threads()[0].beads().is_empty());
            assert!(
                stores
                    .pdf_destination(&tex_state::PdfDestinationIdentity::Number(17), false)
                    .is_none()
            );
        });
    }

    #[test]
    fn running_thread_lifecycle_fatally_rejects_hlist_and_nesting_errors() {
        // pdftex.web §1637 calls `pdf_error` for all three illegal traversal
        // states. Each failed shipout is atomic, but the ext4 identity remains
        // the public session error after §93's fatal transcript is rendered.
        for (source, expected) in [
            (
                "\\pdfoutput=1\\shipout\\hbox{\\pdfstartthread name{bad}}\\end",
                "pdfTeX error (ext4): \\pdfstartthread ended up in hlist",
            ),
            (
                "\\pdfoutput=1\\shipout\\hbox{\\pdfendthread}\\end",
                "pdfTeX error (ext4): \\pdfendthread ended up in hlist",
            ),
            (
                "\\pdfoutput=1\\shipout\\vbox{\\pdfstartthread name{nested}\\vbox{\\pdfendthread}}\\end",
                "pdfTeX error (ext4): \\pdfendthread ended up in different nesting level than \\pdfstartthread",
            ),
        ] {
            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let error = run_pdf_memory(source, stores)
                    .expect_err("invalid running-thread traversal is fatal");
                assert_eq!(error.to_string(), expected);
                let terminal = String::from_utf8_lossy(
                    stores.world().memory_terminal_output().unwrap_or_default(),
                );
                let log =
                    String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default());
                for output in [&terminal, &log] {
                    assert!(
                        tex_state::print::without_line_breaks(output).contains(expected),
                        "{output}"
                    );
                    assert!(
                        output.contains("Fatal error occurred, no output PDF file produced!"),
                        "{output}"
                    );
                }
                assert!(stores.pdf_threads().is_empty());
                assert!(stores.pdf_pages().is_empty());
                assert!(stores.world().artifact_commits().is_empty());
            });
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                "\\pdfoutput=1\\shipout\\vbox{\\pdfstartthread name{complete}\\pdfendthread}\\end",
                stores,
            )
            .expect("same-level running thread lifecycle completes");
            assert_eq!(stores.pdf_threads().len(), 1);
            assert_eq!(stores.pdf_threads()[0].beads().len(), 1);
        });
    }

    #[test]
    fn thread_identifier_is_required_before_any_ledger_or_artifact_publication() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let error = run_pdf_memory(
                "\\pdfoutput=1\\shipout\\hbox{\\pdfthread width1pt}\\end",
                stores,
            )
            .expect_err("missing thread identifier is fatal");

            assert_eq!(
                error.to_string(),
                "pdfTeX error (ext4): thread identifier type missing"
            );
            assert!(stores.pdf_threads().is_empty());
            assert!(stores.pdf_pages().is_empty());
            assert!(stores.world().artifact_commits().is_empty());
        });
    }

    #[test]
    fn pdf_destination_duplicate_scanned_after_ship_uses_current_suppression() {
        const WARNING: &str = "\npdfTeX warning (ext4): destination with the same identifier (num7) has been already used, duplicate ignored\n";
        for (suppression, warns) in [(-1, true), (0, true), (1, false)] {
            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let output = run_pdf_memory(
                &format!(
                    "\\pdfoutput=1\\shipout\\hbox{{\\pdfdest num 7 fit}}\\pdfsuppresswarningdupdest={suppression}\\setbox0=\\hbox{{\\pdfdest num 7 fit}}\\end"
                ),
                stores,
            )
            .expect("scan-time duplicate is recoverable");
                assert_eq!(
                    tex_state::print::without_line_breaks(&output)
                        .matches(WARNING)
                        .count(),
                    usize::from(warns),
                    "{output}"
                );
                assert_eq!(stores.pdf_destinations(false).len(), 1);
            });
        }
    }

    #[test]
    fn pdf_objects_match_reference_errors_and_useobjnum_recovery() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfoutput=1\\message{retval0=\\the\\pdfretval}",
                    "\\pdfobj useobjnum 99 {fallback}",
                    "\\message{retval1=\\the\\pdfretval,last=\\the\\pdflastobj}",
                    "\\pdfobj reserveobjnum",
                    "\\pdfobj useobjnum \\pdflastobj {valid}",
                    "\\message{retval2=\\the\\pdfretval}\\end",
                ),
                stores,
            )
            .expect("recover invalid useobjnum");
            assert_eq!(
                output,
                concat!(
                    " retval0=0\npdfTeX warning (\\pdfobj): invalid object number being ignored\n",
                    "retval1=-1,last=1 retval2=-1",
                )
            );
            assert_eq!(stores.pdf_last_object(), 2);
            assert_eq!(stores.pdf_return_value(), -1);

            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let error = run_pdf_memory("\\pdfoutput=1\\pdfrefobj 99\\end", stores)
                    .expect_err("invalid reference must be fatal");
                assert_eq!(
                    error.to_string(),
                    "pdfTeX error (ext1): cannot find referenced object."
                );

                with_pdftex_stores(|stores| {
                    prepare_pdftex_run_stores(stores);
                    let error = run_pdf_memory(
                        "\\pdfoutput=1\\immediate\\pdfobj reserveobjnum\\end",
                        stores,
                    )
                    .expect_err("immediate reservation must be fatal");
                    assert_eq!(
                        error.to_string(),
                        "pdfTeX error (ext1): `\\pdfobj reserveobjnum' cannot be used with \\immediate."
                    );
                });
            });
        });
    }

    #[test]
    fn pdfretval_meaning_survives_formats_but_runtime_value_resets() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            assert_eq!(InternalInteger::PdfReturnValue.operand(), 22);
            let symbol = stores.intern("pdfretval");
            assert_eq!(
                stores.meaning(symbol),
                Meaning::InternalInteger(InternalInteger::PdfReturnValue)
            );
            stores.set_pdf_return_value(-1);

            let format = stores.dump_format().expect("runtime result is not dumped");
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                World::default(),
                format,
                |loaded| {
                    let loaded = &mut PdftexTestStores(loaded);
                    assert_eq!(loaded.pdf_return_value(), 0);
                    let loaded_symbol = loaded.intern("pdfretval");
                    assert_eq!(
                        loaded.meaning(loaded_symbol),
                        Meaning::InternalInteger(InternalInteger::PdfReturnValue)
                    );
                },
            )
            .expect("load format");
        });
    }

    #[test]
    fn ximage_enquiry_meanings_survive_formats_with_fresh_runtime_values() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            for (name, integer, operand) in [
                (
                    "pdflastximagepages",
                    InternalInteger::PdfLastXImagePages,
                    23,
                ),
                (
                    "pdflastximagecolordepth",
                    InternalInteger::PdfLastXImageColorDepth,
                    24,
                ),
            ] {
                assert_eq!(integer.operand(), operand);
                let symbol = stores.intern(name);
                assert_eq!(stores.meaning(symbol), Meaning::InternalInteger(integer));
            }

            let format = stores
                .dump_format()
                .expect("runtime image state is not dumped");
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                World::default(),
                format,
                |loaded| {
                    let loaded = &mut PdftexTestStores(loaded);
                    assert_eq!(loaded.pdf_last_ximage_pages(), 0);
                    assert_eq!(loaded.pdf_last_ximage_color_depth(), 0);
                    for (name, integer) in [
                        ("pdflastximagepages", InternalInteger::PdfLastXImagePages),
                        (
                            "pdflastximagecolordepth",
                            InternalInteger::PdfLastXImageColorDepth,
                        ),
                    ] {
                        let symbol = loaded.intern(name);
                        assert_eq!(loaded.meaning(symbol), Meaning::InternalInteger(integer));
                    }
                },
            )
            .expect("load format");
        });
    }

    #[test]
    fn ximage_enquiries_follow_success_reuse_and_checkpoint_rollback() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
            stores.enable_pdf_output();
            let initial = run_pdf_memory(
                "\\message{initial=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth}",
                stores,
            )
            .expect("initial image enquiries");
            assert!(initial.contains("initial=0/0"), "{initial}");

            let raster_metadata = tex_state::PdfRasterImageMetadata {
                format: tex_state::PdfRasterFormat::Png,
                width: 1,
                height: 1,
                bits_per_component: 16,
                color_space: tex_state::PdfRasterColorSpace::Gray,
                alpha: false,
                png_color_type: Some(0),
            };
            let raster = stores
                .allocate_pdf_external_image(
                    tex_state::PdfExternalImageSource {
                        identity: tex_state::ContentHash::from_bytes(b"raster"),
                        metadata: tex_state::PdfExternalImageMetadata::Raster(raster_metadata),
                        natural_width: Scaled::from_raw(Scaled::UNITY),
                        natural_height: Scaled::from_raw(Scaled::UNITY),
                        bytes: Vec::new(),
                    },
                    tex_state::PdfExternalImageDimensions {
                        width: Scaled::from_raw(Scaled::UNITY),
                        height: Scaled::from_raw(Scaled::UNITY),
                        depth: Scaled::from_raw(0),
                    },
                    0,
                )
                .expect("allocate raster image");
            let raster_snapshot = stores.snapshot();
            let raster_output = run_pdf_memory(
                concat!(
                    "\\message{raster=\\the\\pdflastximagepages/",
                    "\\the\\pdflastximagecolordepth}",
                    "\\pdfrefximage1",
                    "\\message{reuse=\\the\\pdflastximagepages/",
                    "\\the\\pdflastximagecolordepth}",
                ),
                stores,
            )
            .expect("raster enquiries");
            assert!(raster_output.contains("raster=1/16"), "{raster_output}");
            assert!(raster_output.contains("reuse=1/16"), "{raster_output}");

            let page_box = tex_state::PdfPageBox {
                left: Scaled::from_raw(0),
                bottom: Scaled::from_raw(0),
                right: Scaled::from_raw(Scaled::UNITY),
                top: Scaled::from_raw(Scaled::UNITY),
            };
            stores
                .allocate_pdf_external_image(
                    tex_state::PdfExternalImageSource {
                        identity: tex_state::ContentHash::from_bytes(b"pdf"),
                        metadata: tex_state::PdfExternalImageMetadata::PdfPage {
                            page_box,
                            rotation: tex_state::PdfPageRotation::None,
                            page: 2,
                            total_pages: 3,
                            has_page_group: false,
                            pdf_version: (1, 5),
                        },
                        natural_width: page_box.right,
                        natural_height: page_box.top,
                        bytes: Vec::new(),
                    },
                    tex_state::PdfExternalImageDimensions {
                        width: page_box.right,
                        height: page_box.top,
                        depth: Scaled::from_raw(0),
                    },
                    0,
                )
                .expect("allocate PDF image");
            let pdf_output = run_pdf_memory(
                "\\message{pdf=\\the\\pdflastximagepages/\\the\\pdflastximagecolordepth}",
                stores,
            )
            .expect("PDF enquiries");
            assert!(pdf_output.contains("pdf=3/0"), "{pdf_output}");

            stores.rollback(&raster_snapshot);
            assert_eq!(stores.pdf_last_ximage(), raster.id().raw());
            assert_eq!(stores.pdf_last_ximage_pages(), 1);
            assert_eq!(stores.pdf_last_ximage_color_depth(), 16);
        });
    }

    #[test]
    fn pdfrefobj_is_applied_only_when_its_owning_list_ships() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                "\\pdfoutput=1\\pdfobj{x}\\setbox0=\\hbox{\\pdfrefobj 1}\\end",
                stores,
            )
            .expect("discarded reference box executes");
            assert!(
                !stores
                    .pdf_raw_object(1)
                    .expect("raw object 1")
                    .is_referenced()
            );

            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                run_pdf_memory(
                    concat!(
                        "\\pdfoutput=1\\pdfobj{x}",
                        "\\setbox0=\\hbox{\\pdfrefobj 1}\\shipout\\box0\\end",
                    ),
                    stores,
                )
                .expect("shipped reference box executes");
                assert!(
                    stores
                        .pdf_raw_object(1)
                        .expect("raw object 1")
                        .is_referenced()
                );
            });
        });
    }

    #[test]
    fn pdf_document_fragments_expand_and_preserve_source_order() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            stores.set_int_param_global(IntParam::PDF_OUTPUT, 1);
            run_pdf_memory(
                concat!(
                    "\\def\\value{one}",
                    "\\pdfinfo{/First (\\value)}",
                    "\\def\\value{two}",
                    "\\pdfcatalog{/Catalog (\\value)}",
                    "\\pdfinfo{/Second (\\value)}",
                    "\\pdfnames{/Names (\\value)}",
                    "\\pdftrailer{/Trailer (\\value)}",
                    "\\pdftrailerid{<0123><4567>}",
                    "\\end",
                ),
                stores,
            )
            .expect("execute document dictionary actions");

            let mut fragments = |kind| {
                String::from_utf8(stores.pdf_document_fragment(kind))
                    .expect("PDF document fragments are test ASCII")
            };
            assert_eq!(
                fragments(PdfDocumentFragmentKind::Info),
                "/First (one)/Second (two)"
            );
            assert_eq!(
                fragments(PdfDocumentFragmentKind::Catalog),
                "/Catalog (two)"
            );
            assert_eq!(fragments(PdfDocumentFragmentKind::Names), "/Names (two)");
            assert_eq!(
                fragments(PdfDocumentFragmentKind::Trailer),
                "/Trailer (two)"
            );
            assert_eq!(
                fragments(PdfDocumentFragmentKind::TrailerId),
                "<0123><4567>"
            );
        });
    }

    #[test]
    fn pdf_document_fragments_match_dvi_mode_consumption() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output =
                run_pdf_memory("\\pdfinfo{/Ignored true}\\message{continued}\\end", stores)
                    .expect("warning form scans and ignores its argument");
            let output = complete_memory_terminal(&output, stores);
            assert!(output.contains("pdfTeX warning (\\pdfinfo)"));
            assert!(output.contains("continued"));
            assert_eq!(stores.pdf_next_object_id(), 1);
            assert!(
                stores
                    .pdf_document_fragment(PdfDocumentFragmentKind::Info)
                    .is_empty()
            );

            with_pdftex_stores(|stores| {
                prepare_pdftex_run_stores(stores);
                let error = run_pdf_memory("\\pdfnames{/Forbidden true}\\end", stores)
                    .expect_err("pdfnames must fail before scanning in DVI mode");
                assert_eq!(
                    error.to_string(),
                    "pdfTeX error (\\pdfnames): not allowed in DVI mode (\\pdfoutput <= 0)."
                );

                for (source, name) in [
                    ("\\pdfobj{x}\\end", "pdfobj"),
                    ("\\pdfrefobj 3\\end", "pdfrefobj"),
                ] {
                    with_pdftex_stores(|stores| {
                        prepare_pdftex_run_stores(stores);
                        let error = run_pdf_memory(source, stores)
                            .expect_err("object actions are forbidden in DVI mode");
                        assert_eq!(
                            error.to_string(),
                            format!(
                                "pdfTeX error (\\{name}): not allowed in DVI mode (\\pdfoutput <= 0)."
                            )
                        );
                    });
                }
            });
        });
    }

    #[test]
    fn pdfcatalog_openaction_scans_expanded_actions_and_rejects_duplicates() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\pdfoutput=1\\def\\view{/FitH 10}",
                    "\\pdfcatalog{/PageMode /UseNone} openaction goto page 1 {\\view}",
                    "\\end",
                ),
                stores,
            )
            .expect("open action scans");
            let action = stores.pdf_catalog_open_action().expect("catalog action");
            assert_eq!(action.id(), 1);
            let tex_state::PdfActionSpec::GoTo(destination) = action.spec() else {
                panic!("expected GoTo action");
            };
            let tex_state::PdfActionTarget::Page { number, view } = destination.target else {
                panic!("expected page target");
            };
            assert_eq!(number, 1);
            assert_eq!(token_list_text(stores, view), "/FitH 10");

            let error = run_pdf_memory(
                "\\pdfcatalog{} openaction user{<< /S /Named >>}\\end",
                stores,
            )
            .expect_err("duplicate open action is fatal before rescanning");
            assert_eq!(
                error.to_string(),
                "pdfTeX error (ext1): duplicate of openaction"
            );
        });
    }

    #[test]
    fn pdfcatalog_openaction_is_consumed_without_allocation_in_dvi_mode() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let baseline = stores.snapshot();
            let source = concat!(
                "\\pdfcatalog{} openaction goto file{other.pdf} page 2 {/Fit} newwindow",
                "\\pdfcatalog{} openaction user{<< /S /Named /N /Print >>}",
                "\\message{continued}\\end",
            );
            let output = run_pdf_memory(source, stores)
                .expect("DVI mode consumes repeated ignored open actions");
            let output = complete_memory_terminal(&output, stores);
            assert!(output.contains("pdfTeX warning (\\pdfcatalog)"));
            assert!(output.contains("continued"));
            assert!(stores.pdf_catalog_open_action().is_none());
            assert_eq!(stores.pdf_next_object_id(), 1);
            assert!(
                stores
                    .pdf_document_fragment(PdfDocumentFragmentKind::Catalog)
                    .is_empty()
            );
            stores.rollback(&baseline);
            let replay = run_pdf_memory(source, stores)
                .expect("checkpoint replay consumes the same ignored actions");
            assert_eq!(complete_memory_terminal(&replay, stores), output);
            assert_eq!(stores.pdf_next_object_id(), 1);
        });
    }

    #[test]
    fn saved_position_and_snapping_names_have_exact_pdftex_identity() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            for (name, expected) in [
                ("pdfsavepos", UnexpandablePrimitive::PdfSavePos),
                ("pdfsnaprefpoint", UnexpandablePrimitive::PdfSnapRefPoint),
                ("pdfsnapy", UnexpandablePrimitive::PdfSnapY),
                ("pdfsnapycomp", UnexpandablePrimitive::PdfSnapYComp),
            ] {
                let symbol = stores.intern(name);
                assert_eq!(
                    stores.meaning(symbol),
                    Meaning::UnexpandablePrimitive(expected)
                );
            }
            for (name, expected) in [
                ("pdflastxpos", InternalInteger::PdfLastXPos),
                ("pdflastypos", InternalInteger::PdfLastYPos),
            ] {
                let symbol = stores.intern(name);
                assert_eq!(stores.meaning(symbol), Meaning::InternalInteger(expected));
            }
            let nonexistent_alias = stores.intern("pdfsnaptorefpoint");
            assert_eq!(stores.meaning(nonexistent_alias), Meaning::Undefined);
        });
    }

    #[test]
    fn pdftex_layer_is_visible_only_in_pdftex_mode() {
        for (prepare, intentional_overlaps) in [
            (StorePreparation::Tex, &[][..]),
            (StorePreparation::Etex, &[][..]),
            (StorePreparation::Latex, &["expanded", "ifincsname"][..]),
        ] {
            with_pdftex_stores(|stores| {
                prepare.apply(stores);
                for name in pdftex_primitive_names() {
                    if intentional_overlaps.contains(&name) {
                        continue;
                    }
                    let symbol = stores.intern(name);
                    assert_eq!(stores.meaning(symbol), Meaning::Undefined, "{name}");
                }
            });
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            for name in pdftex_primitive_names() {
                let symbol = stores.intern(name);
                assert_ne!(stores.meaning(symbol), Meaning::Undefined, "{name}");
            }
            let revision = stores.intern("pdftexrevision");
            assert_eq!(
                stores.meaning(revision),
                Meaning::ExpandablePrimitive(ExpandablePrimitive::PdfTeXRevision),
            );
        });
    }

    #[test]
    fn late_expansion_primitives_have_source_derived_profile_ownership() {
        for (prepare, expected) in [
            (
                StorePreparation::Tex,
                [Meaning::Undefined, Meaning::Undefined],
            ),
            (
                StorePreparation::Etex,
                [Meaning::Undefined, Meaning::Undefined],
            ),
            (
                StorePreparation::Pdftex,
                [
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInCsName),
                ],
            ),
            (
                StorePreparation::Latex,
                [
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::Expanded),
                    Meaning::ExpandablePrimitive(ExpandablePrimitive::IfInCsName),
                ],
            ),
        ] {
            with_pdftex_stores(|stores| {
                prepare.apply(stores);
                for ((name, primitive), expected) in [
                    ("expanded", ExpandablePrimitive::Expanded),
                    ("ifincsname", ExpandablePrimitive::IfInCsName),
                ]
                .into_iter()
                .zip(expected)
                {
                    let symbol = stores.intern(name);
                    assert_eq!(stores.meaning(symbol), expected, "{name}");
                    assert_eq!(
                        stores.primitive_meaning(name),
                        (expected != Meaning::Undefined)
                            .then_some(Meaning::ExpandablePrimitive(primitive,)),
                        "{name}",
                    );
                }
            });
        }
    }

    #[test]
    fn pdfmatch_options_capture_limits_and_nul_boundary_match_pdftex() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\catcode0=12 ",
                    "\\message{a=\\pdfmatch icase subcount 2{(a)(b+)}{xABBy}/",
                    "\\pdflastmatch0/\\pdflastmatch1/\\pdflastmatch2} ",
                    "\\message{n=\\pdfmatch{ab}{xxab^^@ab}/",
                    "\\pdflastmatch0} ",
                    "\\message{z=\\pdfmatch{z}{abc}/\\pdflastmatch0}\\end",
                ),
                stores,
            )
            .expect("canonical pdfTeX regex controls");
            assert!(output.contains("a=1/1->ABB/1->A/-1->"), "{output}");
            assert!(output.contains("n=1/2->ab"), "{output}");
            assert!(output.contains("z=0/-1->"), "{output}");
        });
    }

    #[test]
    fn pdftex_random_primitives_match_seeded_reference_sequence() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfsetrandomseed 1 ",
                    "\\message{seed=\\the\\pdfrandomseed}",
                    "\\message{u0=\\pdfuniformdeviate0}",
                    "\\message{u1=\\pdfuniformdeviate1}",
                    "\\message{u2=\\pdfuniformdeviate2}",
                    "\\message{u10a=\\pdfuniformdeviate10}",
                    "\\message{u10b=\\pdfuniformdeviate10}",
                    "\\message{uneg=\\pdfuniformdeviate-10}",
                    "\\message{n1=\\pdfnormaldeviate}",
                    "\\message{n2=\\pdfnormaldeviate}",
                    "\\pdfsetrandomseed -1 ",
                    "\\message{negative-seed=\\the\\pdfrandomseed}",
                    "\\message{repeat=\\pdfuniformdeviate10}\\end",
                ),
                stores,
            )
            .expect("seeded pdfTeX random sequence");
            for expected in [
                "seed=1",
                "u0=0",
                "u1=0",
                "u2=1",
                "u10a=6",
                "u10b=5",
                "uneg=-4",
                "n1=44619",
                "n2=31254",
                "negative-seed=1",
                "repeat=7",
            ] {
                assert!(output.contains(expected), "{expected}: {output}");
            }
        });
    }

    #[test]
    fn pdftex_timer_reset_and_shell_status_use_world_inputs() {
        with_pdftex_stores(|stores| {
            stores.world_mut().set_pdf_time_micros(1_250_000);
            stores
                .world_mut()
                .set_shell_escape_policy(ShellEscapePolicy::Restricted);
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\message{elapsed=\\the\\pdfelapsedtime}",
                    "\\message{shell=\\the\\pdfshellescape}",
                    "\\pdfresettimer",
                    "\\message{reset=\\the\\pdfelapsedtime}\\end",
                ),
                stores,
            )
            .expect("pdfTeX timer and shell enquiries");
            assert!(output.contains("elapsed=81920"), "{output}");
            assert!(output.contains("shell=2"), "{output}");
            assert!(output.contains("reset=0"), "{output}");
        });
    }

    #[test]
    fn pdftex_utility_format_load_uses_the_new_world_session_inputs() {
        with_pdftex_stores(|source| {
            prepare_pdftex_run_stores(source);
            source.world_mut().set_pdf_random_seed(1);
            source.world_mut().set_pdf_time_micros(1_000_000);
            source.world_mut().reset_pdf_timer();
            let format = source.dump_format().expect("utility-free format image");

            let mut world = World::memory();
            world.set_pdf_random_seed(9);
            world.set_pdf_time_micros(2_000_000);
            world.set_shell_escape_policy(ShellEscapePolicy::Enabled);
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                world,
                format,
                |loaded| {
                    let loaded = &mut PdftexTestStores(loaded);
                    crate::install_pdftex_format_primitives(loaded);
                    let output = run_pdf_memory(
                        concat!(
                            "\\message{seed=\\the\\pdfrandomseed}",
                            "\\message{elapsed=\\the\\pdfelapsedtime}",
                            "\\message{shell=\\the\\pdfshellescape}\\end",
                        ),
                        loaded,
                    )
                    .expect("fresh World utility inputs");
                    assert!(output.contains("seed=9"), "{output}");
                    assert!(output.contains("elapsed=131072"), "{output}");
                    assert!(output.contains("shell=1"), "{output}");
                },
            )
            .expect("load with fresh World");
        });
    }

    #[test]
    fn pdftex_random_scanners_report_and_recover_bounds() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\pdfsetrandomseed 999999999999 ",
                    "\\message{seed=\\the\\pdfrandomseed}",
                    "\\pdfsetrandomseed 1 ",
                    "\\message{positive=\\pdfuniformdeviate999999999999}",
                    "\\pdfsetrandomseed 1 ",
                    "\\message{negative=\\pdfuniformdeviate-999999999999}",
                    "\\message{missing=\\pdfuniformdeviate\\relax}\\end",
                ),
                stores,
            )
            .expect("recover random scanner diagnostics");
            assert!(output.contains("Number too big"), "{output}");
            assert!(output.contains("seed=2147483647"), "{output}");
            assert!(output.contains("positive=1516446631"), "{output}");
            assert!(output.contains("negative=-1516446631"), "{output}");
            assert!(output.contains("Missing number"), "{output}");
            assert!(output.contains("missing=0"), "{output}");
        });
    }

    #[test]
    fn pdftex_oracle_runner_recovery_and_interaction_are_checkpointed() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let baseline = stores.snapshot();
            let output =
                run_pdf_memory("\\message{missing=\\pdfuniformdeviate\\relax}\\end", stores)
                    .expect("nonstop oracle runner recovers missing number");
            assert!(
                output.contains("Missing number, treated as zero"),
                "{output}"
            );
            assert!(output.contains("missing=0"), "{output}");
            assert_eq!(
                stores.interaction_mode(),
                tex_state::InteractionMode::Nonstop
            );
            stores.rollback(&baseline);
            assert_eq!(
                stores.interaction_mode(),
                tex_state::InteractionMode::ErrorStop
            );
        });
    }

    #[test]
    fn pdftex_random_primitives_are_available_only_in_the_pdftex_profile() {
        for prepare in [StorePreparation::Tex, StorePreparation::Etex] {
            with_pdftex_stores(|stores| {
                prepare.apply(stores);
                for name in [
                    "pdfrandomseed",
                    "pdfsetrandomseed",
                    "pdfuniformdeviate",
                    "pdfnormaldeviate",
                ] {
                    let symbol = stores.intern(name);
                    assert_eq!(stores.meaning(symbol), Meaning::Undefined, "{name}");
                }
            });
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            for name in [
                "pdfrandomseed",
                "pdfsetrandomseed",
                "pdfuniformdeviate",
                "pdfnormaldeviate",
            ] {
                let symbol = stores.intern(name);
                assert_ne!(stores.meaning(symbol), Meaning::Undefined, "{name}");
            }
        });
    }

    fn seed_pdftex_file_facts<G>(stores: &mut Universe<G>) {
        stores
            .world_mut()
            .set_memory_file("asset.bin", vec![0x00, 0x41, 0x7f, 0x80, 0xff, 0x0a])
            .expect("seed virtual file");
        stores
            .world_mut()
            .set_memory_file_modification_date(
                "asset.bin",
                FileModificationDate::with_offset(
                    JobClock {
                        time: 23 * 60 + 5,
                        second: 6,
                        day: 2,
                        month: 2,
                        year: 2024,
                    },
                    -5 * 60,
                ),
            )
            .expect("seed virtual modification date");
    }

    #[test]
    fn pdffiledump_retries_world_inputs_and_obeys_ranges() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            seed_pdftex_file_facts(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\def\\dumpname{asset.bin}",
                    "\\message{A=[\\pdffiledump length 3 {\\dumpname}]} ",
                    "\\message{B=[\\pdffiledump offset 2 length 99 {asset.bin}]} ",
                    "\\message{C=[\\pdffiledump offset 99 length 2 {asset.bin}]} ",
                    "\\message{M=[\\pdffiledump length 2 {missing}]}\\end",
                ),
                stores,
            )
            .expect("file dump resource retries complete");
            assert!(output.contains("A=[00417F]"), "{output}");
            assert!(output.contains("B=[7F80FF0A]"), "{output}");
            assert!(output.contains("C=[]"), "{output}");
            assert!(output.contains("M=[]"), "{output}");
            let asset_records = stores
                .world()
                .input_records()
                .iter()
                .filter(|record| record.path().to_string_lossy() == "asset.bin")
                .count();
            assert_eq!(asset_records, 1, "retries retain one immutable World read");
        });
    }

    #[test]
    fn primitive_identity_and_absolute_conditionals_match_pdftex() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
            concat!(
                "\\ifpdfprimitive\\count\\message{count-original}\\else\\message{count-bad}\\fi ",
                "\\let\\countalias=\\count ",
                "\\ifpdfprimitive\\countalias\\message{alias-bad}\\else\\message{alias-false}\\fi ",
                "\\ifpdfprimitive\\undefinedname\\message{undefined-bad}\\else\\message{undefined-false}\\fi ",
                "{\\def\\count{shadow}",
                "\\ifpdfprimitive\\count\\message{shadow-bad}\\else\\message{shadow-false}\\fi ",
                "\\pdfprimitive\\count0=12\\message{local-count=\\the\\pdfprimitive\\count0}}",
                "\\pdfprimitive\\count0=37 ",
                "\\ifpdfprimitive\\count\\message{restored}\\else\\message{restore-bad}\\fi ",
                "\\def\\pdftexrevision{shadow-revision}",
                "\\edef\\result{A\\pdfprimitive\\pdftexrevision B\\pdfprimitive\\undefinedname C}",
                "\\message{result=\\result/count=\\the\\count0} ",
                "\\ifpdfabsnum -3>2\\message{num-gt}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsnum 2<-3\\message{num-lt}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsnum -3=3\\message{num-eq}\\else\\message{num-bad}\\fi ",
                "\\ifpdfabsdim -3pt>2pt\\message{dim-gt}\\else\\message{dim-bad}\\fi ",
                "\\ifpdfabsdim 2pt<-3pt\\message{dim-lt}\\else\\message{dim-bad}\\fi ",
                "\\ifpdfabsdim -3pt=3pt\\message{dim-eq}\\else\\message{dim-bad}\\fi ",
                "\\end",
            ),
            stores,
        )
        .expect("pdfTeX primitive utility execution");

            for marker in [
                "count-original",
                "alias-false",
                "undefined-false",
                "shadow-false",
                "restored",
                "local-count=12",
                "result=A27BC/count=37",
                "num-gt",
                "num-lt",
                "num-eq",
                "dim-gt",
                "dim-lt",
                "dim-eq",
            ] {
                assert!(output.contains(marker), "missing {marker}: {output}");
            }
            assert!(!output.contains("-bad"), "{output}");
        });
    }

    #[test]
    fn primitive_registry_reconstructs_after_format_load_without_unshadowing() {
        with_pdftex_stores(|source| {
            prepare_pdftex_run_stores(source);
            let count = source.intern("count");
            let revision = source.intern("pdftexrevision");
            source.set_meaning(count, Meaning::Relax);
            source.set_meaning(revision, Meaning::Relax);
            let format = source.dump_format().expect("dump shadowed format");
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                World::default(),
                format,
                |loaded| {
                    let loaded = &mut PdftexTestStores(loaded);
                    crate::install_pdftex_format_primitives(loaded);

                    let output = run_pdf_memory(
                        concat!(
                            "\\ifpdfprimitive\\count\\message{count-bad}\\else\\message{count-shadowed}\\fi ",
                            "\\pdfprimitive\\count0=41 ",
                            "\\edef\\x{\\pdfprimitive\\pdftexrevision}",
                            "\\message{x=\\x/count=\\the\\pdfprimitive\\count0}\\end",
                        ),
                        loaded,
                    )
                    .expect("run restored primitive registry");
                    assert!(output.contains("count-shadowed"), "{output}");
                    assert!(output.contains("x=27/count=41"), "{output}");
                    assert!(!output.contains("count-bad"), "{output}");
                },
            )
            .expect("load format");
        });
    }

    #[test]
    fn pdftex_parameter_defaults_match_the_pinned_initex_engine() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);

            let parameters = pdftex_parameters();
            assert_eq!(parameters.len(), 57);
            for row in parameters {
                match row.default {
                    tex_command::ParameterDefault::Integer(expected) => assert_eq!(
                        stores.int_param(IntParam::new(row.cell.index)),
                        expected,
                        "{}",
                        row.name
                    ),
                    tex_command::ParameterDefault::Scaled(expected) => assert_eq!(
                        stores.dimen_param(DimenParam::new(row.cell.index)).raw(),
                        expected,
                        "{}",
                        row.name
                    ),
                    tex_command::ParameterDefault::EmptyTokens => assert_eq!(
                        stores.tok_param(TokParam::new(row.cell.index)),
                        None,
                        "{}",
                        row.name
                    ),
                    default => panic!("unexpected pdfTeX default: {default:?}"),
                }
            }

            let alias = stores.intern("pdfoptionpdfminorversion");
            let canonical = stores.intern("pdfminorversion");
            assert_eq!(stores.meaning(alias), stores.meaning(canonical));
            for (obsolete, current) in [
                ("pdfoptionalwaysusepdfpagebox", "pdfforcepagebox"),
                ("pdfoptionpdfinclusionerrorlevel", "pdfinclusionerrorlevel"),
            ] {
                let obsolete = stores.intern(obsolete);
                let current = stores.intern(current);
                assert_ne!(stores.meaning(obsolete), stores.meaning(current));
            }
        });
    }

    #[test]
    fn pdftex_parameter_defaults_are_not_installed_in_other_modes() {
        for prepare in [
            StorePreparation::Tex,
            StorePreparation::Etex,
            StorePreparation::Latex,
        ] {
            with_pdftex_stores(|stores| {
                prepare.apply(stores);
                for row in pdftex_parameters() {
                    match row.meaning {
                        Meaning::IntParam(index) => {
                            assert_eq!(stores.int_param(IntParam::new(index)), 0, "{}", row.name);
                        }
                        Meaning::DimenParam(index) => assert_eq!(
                            stores.dimen_param(DimenParam::new(index)),
                            Scaled::from_raw(0),
                            "{}",
                            row.name
                        ),
                        Meaning::TokParam(index) => {
                            assert_eq!(stores.tok_param(TokParam::new(index)), None, "{}", row.name)
                        }
                        meaning => panic!("unexpected pdfTeX parameter meaning: {meaning:?}"),
                    }
                }
            });
        }
    }

    #[test]
    fn pdftex_parameters_obey_groups_globaldefs_and_legacy_aliases() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
            concat!(
                "\\pdfcompresslevel=7 ",
                "\\pdfhorigin=10pt ",
                "\\pdfpagesattr{outer} ",
                "{\\pdfcompresslevel=3 ",
                "\\pdfhorigin=20pt ",
                "\\pdfpagesattr{inner} ",
                "\\message{local=\\the\\pdfcompresslevel/\\the\\pdfhorigin/\\the\\pdfpagesattr}} ",
                "\\message{restored=\\the\\pdfcompresslevel/\\the\\pdfhorigin/\\the\\pdfpagesattr} ",
                "{\\globaldefs=1 ",
                "\\pdfcompresslevel=4 ",
                "\\pdfhorigin=30pt ",
                "\\pdfpagesattr{global}} ",
                "\\pdfoptionpdfminorversion=7 ",
                "\\pdfoptionalwaysusepdfpagebox=2 ",
                "\\pdfoptionpdfinclusionerrorlevel=1 ",
                "{\\pdfoptionpdfminorversion=6 ",
                "\\pdfoptionalwaysusepdfpagebox=4 ",
                "\\pdfoptionpdfinclusionerrorlevel=3 ",
                "\\message{compat-local=\\the\\pdfminorversion/\\the\\pdfoptionalwaysusepdfpagebox/\\the\\pdfforcepagebox/\\the\\pdfoptionpdfinclusionerrorlevel/\\the\\pdfinclusionerrorlevel}} ",
                "\\message{compat-restored=\\the\\pdfminorversion/\\the\\pdfoptionalwaysusepdfpagebox/\\the\\pdfforcepagebox/\\the\\pdfoptionpdfinclusionerrorlevel/\\the\\pdfinclusionerrorlevel} ",
                "\\end",
            ),
            stores,
        )
        .expect("pdfTeX parameter assignments");

            assert!(output.contains("local=3/20.0pt/inner"), "{output}");
            assert!(output.contains("restored=7/10.0pt/outer"), "{output}");
            assert!(output.contains("compat-local=6/4/0/3/0"), "{output}");
            assert!(output.contains("compat-restored=7/2/0/1/0"), "{output}");
            assert_eq!(stores.int_param(IntParam::PDF_COMPRESS_LEVEL), 4);
            assert_eq!(stores.int_param(IntParam::PDF_MINOR_VERSION), 7);
            assert_eq!(
                stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
                2
            );
            assert_eq!(stores.int_param(IntParam::PDF_FORCE_PAGE_BOX), 0);
            assert_eq!(
                stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
                1
            );
            assert_eq!(stores.int_param(IntParam::PDF_INCLUSION_ERROR_LEVEL), 0);
            assert_eq!(
                stores.dimen_param(DimenParam::PDF_H_ORIGIN),
                Scaled::from_raw(30 * 65_536)
            );
            assert_eq!(
                token_parameter_text(stores, TokParam::PDF_PAGES_ATTR),
                "global"
            );
        });
    }

    #[test]
    fn pdf_image_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_image_config", "ref");
        for expected in [
            "defaults=72/0/0/0/0/0/0/0/1000/2200/1/0/0/0",
            "local=-1/9000/-2/5/1/2/3/4/-3/1000001/2/-1/2/-1",
            "restored=96/300/1/2/3/4/1/2/900/1800/0/1/0/1",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!(
                    "../../../tests/corpus/tex_exec/pdf_image_config/pdf_image_config.tex"
                ),
                stores,
            )
            .expect("pdfTeX image configuration assignments");
            for expected in [
                "defaults=72/0/0/0/0/0/0/0/1000/2200/1/0/0/0",
                "local=-1/9000/-2/5/1/2/3/4/-3/1000001/2/-1/2/-1",
                "restored=96/300/1/2/3/4/1/2/900/1800/0/1/0/1",
            ] {
                assert!(output.contains(expected), "missing {expected:?}: {output}");
            }
        });
    }

    #[test]
    fn pdf_font_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_font_config", "ref");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/300/9",
            ".\\a A",
            ".\\b B",
            ".\\a (cmr10) A",
            ".\\b (cmr10@12.0pt) B",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        with_pdftex_oracle_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed cmr10");
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!("../../../tests/corpus/tex_exec/pdf_font_config/pdf_font_config.tex"),
                stores,
            )
            .expect("pdfTeX font configuration assignments and diagnostics");
            for expected in [
                "defaults=0/0/0/0/0/0/0/0/0",
                "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
                "restored=1/2/3/4/5/6/7/300/9",
                ".\\a A",
                ".\\b B",
                ".\\a (cmr10) A",
                ".\\b (cmr10@12.0pt) B",
            ] {
                assert!(output.contains(expected), "missing {expected:?}: {output}");
            }
            let configuration = stores.pdf_font_configuration();
            assert_eq!(configuration.resolved_pk_resolution(600), 300);
            assert!(configuration.traces_fonts());
            assert!(configuration.omits_charset());
        });
    }

    #[test]
    fn pdf_microtype_effects_match_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_microtype_effects", "ref");
        for expected in [
            "\\kern 1.0 (for \\pdfprependkern/\\pdfappendkern)",
            "\\kern 5.0 (for \\pdfprependkern/\\pdfappendkern)",
            "\\glue 4.33333 plus 3.66666 minus 4.11111",
            "\\kern-1.0 (left margin)",
            "\\kern-2.0 (right margin)",
            "\\f (-50) A",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        with_pdftex_oracle_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed cmr10");
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!(
                    "../../../tests/corpus/tex_exec/pdf_microtype_effects/pdf_microtype_effects.tex"
                ),
                stores,
            )
            .expect("pdfTeX microtype effect fixture");
            for expected in [
                "> \\box0=\n\\hbox(6.83331+0.0)x14.58337\n.\\f A\n.\\f B",
                "> \\box3=\n\\hbox(6.83331+0.0)x24.58337",
                ".\\kern 5.0 (for \\pdfprependkern/\\pdfappendkern)",
                "> \\box4=\n\\hbox(6.83331+0.0)x14.58337",
                "> \\box6=\n\\hbox(6.83331+0.0)x18.9167",
                ".\\glue 4.33333 plus 3.66666 minus 4.11111",
                "> \\box7=\n\\hbox(6.83331+0.0)x17.9167",
                "..\\kern-1.0 (left margin)",
                "..\\kern-2.0 (right margin)",
                "> \\box10=\n\\vbox(6.83331+0.0)x20.0",
                "> \\box12=\n\\vbox(6.83331+0.0)x15.0",
                "..\\f (-50) A",
                "> \\box13=\n\\vbox(6.83331+0.0)x15.0",
            ] {
                assert!(output.contains(expected), "missing {expected:?}: {output}");
            }
        });
    }

    #[test]
    fn pdf_margin_kern_queries_skip_finalized_line_skips() {
        // pdftex.web's `left_margin_kern_code`/`right_margin_kern_code`
        // loops skip `cp_skipable` nodes plus the corresponding finalized
        // left/right skip before inspecting the edge margin-kern node.
        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        with_pdftex_oracle_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed cmr10");
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                r"\catcode`\{=1 \catcode`\}=2
                   \font\f=cmr10 \lpcode\f`A=100 \rpcode\f`.=200
                   \pdfprotrudechars=1
                   \setbox0=\vbox{\hsize=20pt
                     \leftskip=0pt plus1fil \rightskip=0pt plus1fil
                     \noindent\f A.\par}
                   \setbox1=\vbox{\unvbox0\global\setbox2=\lastbox}
                   \immediate\write16{margins=\leftmarginkern2/\rightmarginkern2}
                   \end",
                stores,
            )
            .expect("pdfTeX finalized-skip margin-kern query fixture");
            assert!(output.contains("margins=-1.0pt/-2.0pt"), "{output}");
        });
    }

    #[test]
    fn pdf_font_codes_size_and_ligature_suppression_match_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_font_codes", "ref");
        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        with_pdftex_oracle_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed cmr10");
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!("../../../tests/corpus/tex_exec/pdf_font_codes/pdf_font_codes.tex"),
                stores,
            )
            .expect("pdfTeX font-code fixture");
            for expected in [
                "defaults=0/0/1000/0/0/0/0/0/12.0pt",
                "assigned=7/-1000/800/1000/-1000/321/-432/543",
                "tag-before=1",
                "tag-after=0",
                ".\\a f",
                ".\\a i",
            ] {
                assert!(
                    reference.contains(expected),
                    "oracle missing {expected:?}: {reference}"
                );
                assert!(
                    output.contains(expected),
                    "Umber missing {expected:?}: {output}"
                );
            }
            assert!(
                !output.contains("ligature fi"),
                "ligature survived: {output}"
            );
        });
    }

    #[test]
    fn pdf_output_policy_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_output_policy", "ref");
        for expected in [
            "defaults=0/1.4/9/0/3",
            "local=3/6 restored=7/5",
            "pdfTeX error (invalid pdfmajorversion)",
            "pdfTeX error (invalid pdfminorversion)",
            "Object streams disabled now",
            "recovered=1.4",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        with_pdftex_oracle_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!(
                    "../../../tests/corpus/tex_exec/pdf_output_policy/pdf_output_policy.tex"
                ),
                stores,
            )
            .expect("Umber recovers from the pinned range cases");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
            for expected in [
                "defaults=0/1.4/9/0/3",
                "local=3/6",
                "restored=7/5",
                "pdfTeX error (invalid pdfmajorversion)",
                "pdfTeX error (invalid pdfminorversion)",
                "Object streams disabled now",
                "recovered=1.4",
            ] {
                assert!(
                    observed.contains(expected),
                    "missing {expected:?}: {observed}"
                );
            }
            assert_eq!(
                stores.fixed_pdf_output_parameters(),
                Some(tex_state::PdfOutputParameters {
                    output: 1,
                    major_version: 1,
                    minor_version: 4,
                    compress_level: 7,
                    object_compress_level: 0,
                    decimal_digits: 4,
                    gamma: 1_000,
                    image_gamma: 2_200,
                    image_hicolor: 1,
                    image_apply_gamma: 0,
                    draft_mode: 0,
                    inclusion_copy_fonts: 0,
                    pk_resolution: 0,
                    unique_resource_names: 0,
                })
            );
        });
    }

    #[test]
    fn pdf_insert_height_reads_live_page_insertion_accounting() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\vsize=100pt ",
                    "\\count254=1000 \\dimen254=100pt \\skip254=0pt ",
                    "\\message{before=\\pdfinsertht254/absent=\\pdfinsertht253} ",
                    "{\\insert254{\\hbox{\\vrule height10pt depth2pt width0pt}}} ",
                    "\\message{first=\\pdfinsertht254} ",
                    "\\insert254{\\hbox{\\vrule height3pt depth1pt width0pt}} ",
                    "\\message{second=\\pdfinsertht254/absent=\\pdfinsertht253}",
                ),
                stores,
            )
            .expect("pdfTeX insertion-height enquiry");

            for expected in [
                "before=0pt/absent=0pt",
                "first=12.0pt",
                "second=16.0pt/absent=0pt",
            ] {
                assert!(output.contains(expected), "missing {expected:?}: {output}");
            }
            assert_eq!(
                stores.page_insertion_height(254),
                Some(Scaled::from_raw(16 * Scaled::UNITY))
            );

            run_pdf_memory("\\end", stores).expect("finish the page containing insertions");
            assert_eq!(stores.page_insertion_height(254), None);

            with_pdftex_stores(|split_stores| {
                prepare_pdftex_run_stores(split_stores);
                let split_output = run_pdf_memory(
                    concat!(
                        "\\vsize=100pt ",
                        "\\count254=1000 \\dimen254=5pt \\skip254=0pt ",
                        "\\splittopskip=0pt \\splitmaxdepth=0pt ",
                        "\\insert254{",
                        "\\hbox{\\vrule height4pt depth0pt width0pt}",
                        "\\vskip1pt",
                        "\\hbox{\\vrule height4pt depth0pt width0pt}} ",
                        "\\message{split=\\pdfinsertht254}",
                    ),
                    split_stores,
                )
                .expect("split pdfTeX insertion-height enquiry");
                assert!(
                    split_output.contains("split=4.0pt"),
                    "split oracle mismatch: {split_output}"
                );
                assert_eq!(
                    split_stores.page_insertion_height(254),
                    Some(Scaled::from_raw(4 * Scaled::UNITY))
                );
            });
        });
    }

    #[test]
    fn pdf_insert_height_scans_an_expanded_register() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                concat!(
                    "\\vsize=100pt ",
                    "\\count254=1000 \\dimen254=100pt \\skip254=0pt ",
                    "\\def\\insertclass{254}",
                    "\\insert254{\\hbox{\\vrule height6pt depth1pt width0pt}} ",
                    "\\message{expanded=\\pdfinsertht\\insertclass}",
                ),
                stores,
            )
            .expect("expanded insertion register enquiry");
            assert!(output.contains("expanded=7.0pt"), "{output}");
        });
    }

    #[test]
    fn pdf_ximage_bbox_rejects_missing_objects_and_bad_indices() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let missing = run_pdf_memory("\\message{\\pdfximagebbox99 1}", stores)
                .expect_err("missing external image must be fatal");
            assert_eq!(
                missing.to_string(),
                "pdfTeX error (ext1): cannot find referenced object."
            );

            stores
                .allocate_pdf_external_image(
                    tex_state::PdfExternalImageSource {
                        identity: tex_state::ContentHash::from_bytes(b"bbox-invalid-index"),
                        metadata: tex_state::PdfExternalImageMetadata::Raster(
                            tex_state::PdfRasterImageMetadata::placeholder(),
                        ),
                        natural_width: Scaled::from_raw(Scaled::UNITY),
                        natural_height: Scaled::from_raw(Scaled::UNITY),
                        bytes: Vec::new(),
                    },
                    tex_state::PdfExternalImageDimensions {
                        width: Scaled::from_raw(Scaled::UNITY),
                        height: Scaled::from_raw(Scaled::UNITY),
                        depth: Scaled::from_raw(0),
                    },
                    0,
                )
                .expect("register image metadata");
            for index in [0, 5, -1] {
                let error =
                    run_pdf_memory(&format!("\\message{{\\pdfximagebbox1 {index}}}"), stores)
                        .expect_err("bad bbox index must be fatal");
                assert_eq!(
                    error.to_string(),
                    "pdfTeX error (pdfximagebbox): invalid parameter."
                );
            }
        });
    }

    #[test]
    fn pdf_metadata_configuration_matches_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_metadata_config", "ref");
        for expected in [
            "defaults=0/0/0/0/0/0/0/0/0",
            "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
            "restored=1/2/3/4/5/6/7/8/9",
        ] {
            assert!(
                reference.contains(expected),
                "missing {expected:?}: {reference}"
            );
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
                include_str!(
                    "../../../tests/corpus/tex_exec/pdf_metadata_config/pdf_metadata_config.tex"
                ),
                stores,
            )
            .expect("pdfTeX metadata configuration assignments");
            for expected in [
                "defaults=0/0/0/0/0/0/0/0/0",
                "local=-1/-2/-3/-4/-5/-6/-7/-8/-9",
                "restored=1/2/3/4/5/6/7/8/9",
            ] {
                assert!(output.contains(expected), "missing {expected:?}: {output}");
            }
        });
    }

    #[test]
    fn all_page_token_and_dimension_parameters_scan_group_and_display() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let mut source = String::new();
            let parameters = pdftex_parameters();
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Dimension)
            {
                source.push_str(&format!("\\{}=1pt ", row.name));
            }
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Tokens)
            {
                source.push_str(&format!("\\{}{{outer-{}}} ", row.name, row.name));
            }
            source.push('{');
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Dimension)
            {
                source.push_str(&format!(
                    "\\{}=2pt \\message{{L{}=\\the\\{}}} ",
                    row.name, row.name, row.name
                ));
            }
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Tokens)
            {
                source.push_str(&format!(
                    "\\{}{{inner-{}}} \\message{{L{}=\\the\\{}}} ",
                    row.name, row.name, row.name, row.name
                ));
            }
            source.push_str("} \\end");

            let output =
                run_pdf_memory(&source, stores).expect("all pdfTeX page parameters assign");
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Dimension)
            {
                assert!(
                    output.contains(&format!("L{}=2.0pt", row.name)),
                    "{}: {output}",
                    row.name
                );
                assert_eq!(
                    stores.dimen_param(DimenParam::new(row.cell.index)),
                    Scaled::from_raw(Scaled::UNITY)
                );
            }
            for row in parameters
                .iter()
                .filter(|row| row.cell.class == tex_command::ParameterBankClass::Tokens)
            {
                assert!(
                    output.contains(&format!("L{}=inner-{}", row.name, row.name)),
                    "{}: {output}",
                    row.name
                );
                assert_eq!(
                    token_parameter_text(stores, TokParam::new(row.cell.index)),
                    format!("outer-{}", row.name)
                );
            }
        });
    }

    #[test]
    fn pdftex_line_dimension_overrides_follow_ignore_and_precedence_rules() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            run_pdf_memory(
                concat!(
                    "\\setbox0=\\vbox{\\hsize=10pt ",
                    "\\pdfeachlineheight=11pt \\pdfeachlinedepth=12pt ",
                    "\\pdffirstlineheight=13pt \\pdflastlinedepth=14pt ",
                    "\\noindent\\hbox to10pt{}\\penalty-10000\\hbox to10pt{}\\par} ",
                    "\\end",
                ),
                stores,
            )
            .expect("pdfTeX line dimensions");

            let lines = stores.box_line_dimensions(0);
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].0, Scaled::from_raw(13 * Scaled::UNITY));
            assert_eq!(lines[0].1, Scaled::from_raw(12 * Scaled::UNITY));
            assert_eq!(lines[1].0, Scaled::from_raw(11 * Scaled::UNITY));
            assert_eq!(lines[1].1, Scaled::from_raw(14 * Scaled::UNITY));
        });
    }

    #[test]
    fn pdftex_parameters_survive_snapshots_hashes_and_formats() {
        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            stores.set_int_param(IntParam::PDF_COMPRESS_LEVEL, 5);
            stores.set_int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX, 2);
            stores.set_int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL, 3);
            stores.set_int_param(IntParam::IGNORE_PRIMITIVE_ERROR, 1);
            stores.set_dimen_param(DimenParam::PDF_PAGE_WIDTH, Scaled::from_raw(12_345));
            let first_tokens = stores.intern_token_list(&[
                Token::Char {
                    ch: 'f',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'i',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 'r',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 's',
                    cat: Catcode::Other,
                },
                Token::Char {
                    ch: 't',
                    cat: Catcode::Other,
                },
            ]);
            stores.set_tok_param(TokParam::PDF_PAGE_ATTR, first_tokens);
            let first = stores.snapshot();

            stores.set_int_param(IntParam::PDF_COMPRESS_LEVEL, 2);
            stores.set_int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX, 4);
            stores.set_int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL, 5);
            stores.set_int_param(IntParam::IGNORE_PRIMITIVE_ERROR, 2);
            stores.set_dimen_param(DimenParam::PDF_PAGE_WIDTH, Scaled::from_raw(54_321));
            let second_tokens = stores.intern_token_list(&[Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            }]);
            stores.set_tok_param(TokParam::PDF_PAGE_ATTR, second_tokens);

            stores.rollback(&first);
            assert_eq!(stores.int_param(IntParam::PDF_COMPRESS_LEVEL), 5);
            assert_eq!(
                stores.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
                2
            );
            assert_eq!(
                stores.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
                3
            );
            assert_eq!(stores.int_param(IntParam::IGNORE_PRIMITIVE_ERROR), 1);
            assert_eq!(
                stores.dimen_param(DimenParam::PDF_PAGE_WIDTH),
                Scaled::from_raw(12_345)
            );
            assert_eq!(
                token_parameter_text(stores, TokParam::PDF_PAGE_ATTR),
                "first"
            );

            let format = stores.dump_format().expect("pdfTeX parameter format");
            let format_bytes = format.as_bytes().to_vec();
            tex_state::with_materialized_format(
                crate::engine_interner_budget(),
                World::default(),
                format,
                |loaded| {
                    let loaded = &mut PdftexTestStores(loaded);
                    assert_eq!(
                        loaded
                            .capture_format_image()
                            .expect("recapture format")
                            .as_bytes(),
                        format_bytes
                    );
                    assert_eq!(loaded.int_param(IntParam::PDF_COMPRESS_LEVEL), 5);
                    assert_eq!(
                        loaded.int_param(IntParam::PDF_OPTION_ALWAYS_USE_PDF_PAGE_BOX),
                        2
                    );
                    assert_eq!(
                        loaded.int_param(IntParam::PDF_OPTION_INCLUSION_ERROR_LEVEL),
                        3
                    );
                    assert_eq!(loaded.int_param(IntParam::IGNORE_PRIMITIVE_ERROR), 1);
                    assert_eq!(
                        loaded.dimen_param(DimenParam::PDF_PAGE_WIDTH),
                        Scaled::from_raw(12_345)
                    );
                    assert_eq!(
                        token_parameter_text(loaded, TokParam::PDF_PAGE_ATTR),
                        "first"
                    );
                },
            )
            .expect("load format");
        });
    }

    #[test]
    fn mode_and_primitive_error_controls_match_the_pinned_initex_oracle() {
        let reference = test_support::read_fixture("tex_exec", "pdf_compatibility_controls", "ref");
        let expected = [
            "default-ignore=0 local-ignore=3 restored-ignore=0",
            "hmode-quit=no-op",
            "mmode-quit=no-op",
            "vmode-quit=horizontal",
            "ignored error: Infinite glue shrinkage found in box being split",
        ];
        for line in expected {
            assert!(
                reference.contains(line),
                "oracle missing {line:?}: {reference}"
            );
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
            include_str!("../../../tests/corpus/tex_exec/pdf_compatibility_controls/pdf_compatibility_controls.tex"),
            stores,
        )
        .expect("execute mode/error compatibility fixture");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
            for line in expected {
                assert!(
                    observed.contains(line),
                    "Umber missing {line:?}: {observed}"
                );
            }
            assert_eq!(stores.int_param(IntParam::IGNORE_PRIMITIVE_ERROR), 1);
            assert_eq!(UnexpandablePrimitive::QuitVMode.operand(), 265);
        });
    }

    #[test]
    fn pdfmovechars_warns_and_resets_only_when_a_new_pdf_font_is_used() {
        let reference = test_support::read_fixture("tex_exec", "pdf_move_chars_warning", "ref");
        for expected in [
            "pdfTeX warning: Primitive \\pdfmovechars is obsolete.",
            "after-first=0",
            "after-second=1",
        ] {
            assert!(
                reference.contains(expected),
                "oracle missing {expected:?}: {reference}"
            );
        }

        const CMR10: &[u8] = include_bytes!("../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        with_pdftex_oracle_stores(|stores| {
            stores
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("seed cmr10");
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
            include_str!(
                "../../../tests/corpus/tex_exec/pdf_move_chars_warning/pdf_move_chars_warning.tex"
            ),
            stores,
        )
        .expect("execute obsolete pdfmovechars fixture");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
            for expected in [
                "pdfTeX warning: Primitive \\pdfmovechars is obsolete.",
                "after-first=0",
                "after-second=1",
            ] {
                assert!(
                    observed.contains(expected),
                    "Umber missing {expected:?}: {observed}"
                );
            }
            assert_eq!(stores.int_param(IntParam::PDF_MOVE_CHARS), 1);
        });
    }

    #[test]
    fn pdfignoreddimen_is_the_live_prevdepth_and_line_override_sentinel() {
        let reference = test_support::read_fixture("tex_exec", "pdf_ignored_dimen_effects", "ref");
        let expected = [
            "initial-prevdepth=12.0pt",
            "matching-line=0.0pt/0.0pt",
            "active-line=12.0pt/12.0pt",
        ];
        for line in expected {
            assert!(
                reference.contains(line),
                "oracle missing {line:?}: {reference}"
            );
        }

        with_pdftex_stores(|stores| {
            prepare_pdftex_run_stores(stores);
            let output = run_pdf_memory(
            include_str!("../../../tests/corpus/tex_exec/pdf_ignored_dimen_effects/pdf_ignored_dimen_effects.tex"),
            stores,
        )
        .expect("execute live ignored-dimension fixture");
            let terminal = stores.world().memory_terminal_output().unwrap_or_default();
            let observed = format!("{}{}", String::from_utf8_lossy(terminal), output);
            for line in expected {
                assert!(
                    observed.contains(line),
                    "Umber missing {line:?}: {observed}"
                );
            }
        });
    }

    fn token_list_text<G>(stores: &mut Universe<G>, id: TokenListId<G>) -> String {
        stores
            .command_context()
            .expect("admit test token-list reader")
            .token_list(id)
            .iter()
            .filter_map(|word| match word.token() {
                Some(Token::Char { ch, .. }) => Some(ch),
                _ => None,
            })
            .collect()
    }

    fn token_parameter_text<G>(stores: &mut Universe<G>, parameter: TokParam) -> String {
        let context = stores
            .command_context()
            .expect("admit test token-parameter reader");
        context
            .token_parameter(parameter)
            .expect("read test token parameter")
            .map_or_else(String::new, |id| {
                context
                    .token_list(id)
                    .iter()
                    .filter_map(|word| match word.token() {
                        Some(Token::Char { ch, .. }) => Some(ch),
                        _ => None,
                    })
                    .collect()
            })
    }

    mod retained_fixture_properties;
}

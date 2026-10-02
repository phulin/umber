//! Native scan scalars in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_unary_scalar_operation(
        &mut self,
        meaning: Meaning,
        global: bool,
        origin: tex_state::token::OriginId,
    ) -> Result<ReplayStep, ExecError> {
        let boxes = &self.control.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.modes,
            pdf_ignore_depth: self.control.pdf_ignore_depth,
            telemetry: &mut self.control.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                provider,
                &mut self.control.declined_resource_attempt,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        } else {
            command_processor(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor: &mut CommandProcessor<'_, '_, G> = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, host_facts, $call)
            };
        }

        let scan_origin = self.current_origin;

        let has_optional_equals = matches!(
            meaning,
            Meaning::UnexpandablePrimitive(
                UnexpandablePrimitive::PrevDepth
                    | UnexpandablePrimitive::InteractionMode
                    | UnexpandablePrimitive::SpaceFactor
                    | UnexpandablePrimitive::PrevGraf
            ) | Meaning::IntParam(_)
                | Meaning::DimenParam(_)
                | Meaning::PageDimension(_)
                | Meaning::PageInteger(_)
        );
        if has_optional_equals {
            let _ = take_native_scalar!(processor.scan_optional_equals_retained(), scan_origin);
        }
        match meaning {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
                let value =
                    take_native_scalar!(processor.scan_glue_retained(false), scan_origin).value;
                // pdftex.web §§454, 485, 1003, 1236: an internal zero glue
                // quantity retains the shared `zero_glue` pointer when appended.
                // Scanned literal zero glue and negated internal glue are fresh.
                let shared_zero_glue = processor.scanned_glue_shared_zero();
                finish_scan!(self.execute_horizontal_skip(value, shared_zero_glue))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip) => {
                let value =
                    take_native_scalar!(processor.scan_glue_retained(false), scan_origin).value;
                let shared_zero_glue = processor.scanned_glue_shared_zero();
                finish_scan!(self.execute_vertical_skip(value, shared_zero_glue))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
                let amount =
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin).value;
                finish_scan!(self.execute_kern(amount))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
                let value =
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin).value;
                finish_scan!(self.execute_prev_depth(value))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
                let amount =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_penalty(amount))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage) => {
                let object =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_pdf_ref_x_image(object))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
                let seed =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_pdf_set_random_seed(seed.saturating_abs()))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
                let language =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_set_language(language))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                {
                    let context = processor.diagnostic_context_coordinate();
                    finish_scan!(self.execute_set_interaction_mode_value(value, context))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_space_factor(value))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_prev_graf(value))
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_character_code(value, origin, false))
            }
            Meaning::IntParam(index) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_int_param(index, value, global))
            }
            Meaning::DimenParam(index) => {
                let value =
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin).value;
                finish_scan!(self.execute_dimen_param(index, value, global))
            }
            Meaning::PageDimension(dimension) => {
                let value =
                    take_native_scalar!(processor.scan_dimension_retained(), scan_origin).value;
                finish_scan!(self.execute_page_dimension(dimension, value))
            }
            Meaning::PageInteger(integer) => {
                let value =
                    take_native_scalar!(processor.scan_integer_retained(), scan_origin).value;
                finish_scan!(self.execute_page_integer(integer, value))
            }
            _ => unreachable!("unary scalar descriptor restricts command meanings"),
        }
    }
}

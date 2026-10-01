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

        let mut scalar_storage = tex_command::ScalarScanFrame::default();
        let scalar = &mut scalar_storage;

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
            let status = processor.scan_optional_equals_into(scalar);
            let _ = take_control_scalar!(scalar, status, take_boolean, scan_origin);
        }
        match meaning {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
                let status = processor.scan_glue_into(false, scalar);
                let value = take_control_scalar!(scalar, status, take_glue, scan_origin).value;
                // pdftex.web §§454, 485, 1003, 1236: an internal zero glue
                // quantity retains the shared `zero_glue` pointer when appended.
                // Scanned literal zero glue and negated internal glue are fresh.
                let shared_zero_glue = processor.scanned_glue_shared_zero();
                {
                    let (direct_arg0, direct_arg1) = (value, shared_zero_glue);
                    finish_scan!(self.execute_horizontal_skip(direct_arg0, direct_arg1))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip) => {
                let status = processor.scan_glue_into(false, scalar);
                let value = take_control_scalar!(scalar, status, take_glue, scan_origin).value;
                let shared_zero_glue = processor.scanned_glue_shared_zero();
                {
                    let (direct_arg0, direct_arg1) = (value, shared_zero_glue);
                    finish_scan!(self.execute_vertical_skip(direct_arg0, direct_arg1))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
                let status = processor.scan_dimension_into(scalar);
                let amount =
                    take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
                {
                    let (direct_arg0,) = (amount,);
                    finish_scan!(self.execute_kern(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
                let status = processor.scan_dimension_into(scalar);
                let value = take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
                {
                    let (direct_arg0,) = (value,);
                    finish_scan!(self.execute_prev_depth(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
                let status = processor.scan_integer_into(scalar);
                let amount = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (amount,);
                    finish_scan!(self.execute_penalty(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRefXImage) => {
                let status = processor.scan_integer_into(scalar);
                let object = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (object,);
                    finish_scan!(self.execute_pdf_ref_x_image(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
                let status = processor.scan_integer_into(scalar);
                let seed = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (seed.saturating_abs(),);
                    finish_scan!(self.execute_pdf_set_random_seed(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
                let status = processor.scan_integer_into(scalar);
                let language =
                    take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (language,);
                    finish_scan!(self.execute_set_language(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1) =
                        (value, processor.diagnostic_context_coordinate());
                    finish_scan!(self.execute_set_interaction_mode_value(direct_arg0, direct_arg1))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (value,);
                    finish_scan!(self.execute_space_factor(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0,) = (value,);
                    finish_scan!(self.execute_prev_graf(direct_arg0))
                }
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1, direct_arg2) = (value, origin, false);
                    finish_scan!(self.execute_character_code(direct_arg0, direct_arg1, direct_arg2))
                }
            }
            Meaning::IntParam(index) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1, direct_arg2) = (index, value, global);
                    finish_scan!(self.execute_int_param(direct_arg0, direct_arg1, direct_arg2))
                }
            }
            Meaning::DimenParam(index) => {
                let status = processor.scan_dimension_into(scalar);
                let value = take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1, direct_arg2) = (index, value, global);
                    finish_scan!(self.execute_dimen_param(direct_arg0, direct_arg1, direct_arg2))
                }
            }
            Meaning::PageDimension(dimension) => {
                let status = processor.scan_dimension_into(scalar);
                let value = take_control_scalar!(scalar, status, take_dimension, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1) = (dimension, value);
                    finish_scan!(self.execute_page_dimension(direct_arg0, direct_arg1))
                }
            }
            Meaning::PageInteger(integer) => {
                let status = processor.scan_integer_into(scalar);
                let value = take_control_scalar!(scalar, status, take_integer, scan_origin).value;
                {
                    let (direct_arg0, direct_arg1) = (integer, value);
                    finish_scan!(self.execute_page_integer(direct_arg0, direct_arg1))
                }
            }
            _ => unreachable!("unary scalar descriptor restricts command meanings"),
        }
    }
}

//! Native scan primitive recovery in the sole main-control engine.
use super::*;
impl<G> DirectDispatch<'_, '_, G> {
    pub(super) fn scan_unclassified_primitive(
        &mut self,
        command: tex_command::CurrentCommand<G>,
        primitive: UnexpandablePrimitive,
    ) -> Result<ReplayStep, ExecError> {
        let mode = self.control.execution.modes.current_mode();
        let boxes = &self.control.execution.boxes;

        let mut admitted_context = self
            .stores
            .command_context()
            .expect("direct scanner admission");

        let mut host_facts = ExecutorHostFacts {
            modes: &self.control.execution.modes,
            pdf_ignore_depth: self.control.execution.pdf_ignore_depth,
            telemetry: &mut self.control.execution.episode_telemetry,
        };
        let mut owned_processor = if let Some(provider) = self.provider.as_deref_mut() {
            command_processor_with_resource_provider(
                &mut self.control.command,
                self.control.fuel.fuel_mut(),
                &mut self.control.capabilities,
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
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
                &mut host_facts as &mut dyn tex_command::CommandHostFacts<G>,
                &mut self.control.operation_observations,
                self.effects,
                &mut admitted_context,
            )
        };
        owned_processor.set_output_routine_active(boxes.output_routine_active);
        if let Some(cursor) = self.cursor {
            owned_processor.resume_delivery_cursor(cursor);
        }
        let processor = &mut owned_processor;
        // Finish this actual processor/context loan before semantic application.
        macro_rules! finish_scan {
            ($call:expr) => {
                direct_finish_scan!(self, owned_processor, admitted_context, $call)
            };
        }

        let scan_origin = self.current_origin;

        use UnexpandablePrimitive as P;
        match primitive {
            P::Accent
            | P::Advance
            | P::AfterAssignment
            | P::AfterGroup
            | P::BeginGroup
            | P::Box
            | P::CLeaders
            | P::CatCode
            | P::Char
            | P::CharDef
            | P::CloseIn
            | P::CloseOut
            | P::ControlSpace
            | P::Copy
            | P::Count
            | P::CountDef
            | P::Def
            | P::DelCode
            | P::Dimen
            | P::DimenDef
            | P::Discretionary
            | P::Divide
            | P::Dump
            | P::Edef
            | P::End
            | P::EndGroup
            | P::ErrMessage
            | P::Font
            | P::FontDimen
            | P::FutureLet
            | P::Gdef
            | P::HAlign
            | P::HBox
            | P::HFil
            | P::HFilNeg
            | P::HFill
            | P::HRule
            | P::HSkip
            | P::HSs
            | P::HyphenChar
            | P::Hyphenation
            | P::Immediate
            | P::Indent
            | P::Insert
            | P::InteractionMode
            | P::Kern
            | P::LastBox
            | P::LcCode
            | P::Leaders
            | P::Let
            | P::LetterspaceFont
            | P::Lower
            | P::Lowercase
            | P::Marks
            | P::MathCharDef
            | P::MathCode
            | P::Message
            | P::MoveLeft
            | P::MoveRight
            | P::Multiply
            | P::Muskip
            | P::MuskipDef
            | P::NoIndent
            | P::OpenIn
            | P::OpenOut
            | P::Par
            | P::ParShape
            | P::Patterns
            | P::PageDiscards
            | P::PdfAnnot
            | P::PdfCatalog
            | P::PdfColorStack
            | P::PdfDest
            | P::PdfEndLink
            | P::PdfEndThread
            | P::PdfFontAttr
            | P::PdfFontExpand
            | P::PdfGlyphToUnicode
            | P::PdfIncludeChars
            | P::PdfInfo
            | P::PdfLiteral
            | P::PdfCopyFont
            | P::PdfMapFile
            | P::PdfMapLine
            | P::PdfNames
            | P::PdfNoBuiltinToUnicode
            | P::PdfObject
            | P::PdfOutline
            | P::PdfInterwordSpaceOff
            | P::PdfInterwordSpaceOn
            | P::PdfFakeSpace
            | P::PdfRunningLinkOff
            | P::PdfRunningLinkOn
            | P::PdfSpaceFont
            | P::PdfRefXForm
            | P::PdfRefXImage
            | P::PdfReferenceObject
            | P::PdfRestore
            | P::PdfSave
            | P::PdfSavePos
            | P::PdfSnapRefPoint
            | P::PdfSnapY
            | P::PdfSnapYComp
            | P::PdfResetTimer
            | P::PdfSetRandomSeed
            | P::PdfSetMatrix
            | P::PdfStartLink
            | P::PdfStartThread
            | P::PdfThread
            | P::PdfTrailer
            | P::PdfTrailerId
            | P::PdfXForm
            | P::PdfXImage
            | P::ItalicCorrection
            | P::NoBoundary
            | P::NonScript
            | P::Penalty
            | P::PrevDepth
            | P::PrevGraf
            | P::Raise
            | P::Read
            | P::ReadLine
            | P::ScriptFont
            | P::ScriptScriptFont
            | P::SetBox
            | P::SfCode
            | P::Shipout
            | P::Show
            | P::ShowBox
            | P::ShowGroups
            | P::ShowLists
            | P::ShowThe
            | P::ShowTokens
            | P::ShowIfs
            | P::SkewChar
            | P::Skip
            | P::SkipDef
            | P::Special
            | P::TextFont
            | P::Toks
            | P::ToksDef
            | P::UcCode
            | P::UnHBox
            | P::UnHCopy
            | P::UnKern
            | P::UnPenalty
            | P::UnSkip
            | P::UnVBox
            | P::UnVCopy
            | P::SplitDiscards
            | P::Uppercase
            | P::VAlign
            | P::VBox
            | P::VRule
            | P::VSplit
            | P::VTop
            | P::Wd
            | P::Ht
            | P::Dp
            | P::Write
            | P::XLeaders
            | P::Xdef
            | P::Mark
            | P::VAdjust
            | P::SetLanguage
            | P::BatchMode
            | P::ClubPenalties
            | P::DisplayWidowPenalties
            | P::InterLinePenalties
            | P::WidowPenalties
            | P::NonstopMode
            | P::ScrollMode
            | P::QuitVMode
            | P::ErrorStopMode => unreachable!(
                "UnexpandablePrimitive::{primitive:?} has an explicit, mode-complete \
             scan_command dispatch arm and must never reach the exhaustive fallback"
            ),
            // Consumed by `dispatch_main_control_command` *before* the big case,
            // exactly as tex.web consumes them: §1211 `prefixed_command`'s
            // `while cur_cmd=prefix` loop absorbs `\global`/`\long`/`\outer` (and
            // e-TeX's `\protected`) into the accumulator `a` that the assignment
            // cases then read, and §1045's `any_mode(ignore_spaces)` re-enters
            // §1030's `reswitch:` with the next non-blank non-call token. None of
            // them is a mode-dispatched primitive -- §1210 files the prefixes
            // under `any_mode` -- so `scan_command` has, and must have, no arm for
            // them. Reaching this arm means some caller dispatched a command
            // without going through `dispatch_main_control_command`, which is the
            // narrowed-main-control defect of `umber2-johp.208`.
            P::Global | P::Long | P::Outer | P::Protected | P::IgnoreSpaces => unreachable!(
                "UnexpandablePrimitive::{primitive:?} is consumed by \
             dispatch_main_control_command before scan_command; reaching \
             scan_command means a caller bypassed the shared main-control step"
            ),
            // TeX82 §1046's `non_math(...)` table: each of these primitives is a
            // math-noad, math-style, or math-delimiter command whose *only*
            // canonical dispatch is its arm in the shared mode/packed-command
            // switch under `Mode::Math`/`DisplayMath`.
            // Reaching this arm therefore proves `mode` is not math, which is
            // exactly tex.web's non-math table; §1047's `insert_dollar_sign`
            // recovers uniformly for the whole family via the same
            // `recover_missing_math_shift` helper the `mmode+hrule`/`mmode+vskip`/
            // `non_math(non_script)` arms above already use.
            P::Above
            | P::AboveWithDelims
            | P::Atop
            | P::AtopWithDelims
            | P::Delimiter
            | P::DisplayLimits
            | P::DisplayStyle
            | P::Left
            | P::Limits
            | P::MKern
            | P::MSkip
            | P::MathAccent
            | P::MathBin
            | P::MathChar
            | P::MathChoice
            | P::MathClose
            | P::MathInner
            | P::MathOp
            | P::MathOpen
            | P::MathOrd
            | P::MathPunct
            | P::MathRel
            | P::Middle
            | P::NoLimits
            | P::Over
            | P::OverWithDelims
            | P::Overline
            | P::Radical
            | P::Right
            | P::ScriptScriptStyle
            | P::ScriptStyle
            | P::TextStyle
            | P::Underline
            | P::VCenter => {
                (processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error))
                .map_err(|error| scan_error(error, scan_origin))?;
                finish_scan!(self.execute_missing_math_shift())
            }
            // TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)`: unlike the
            // math-noad family immediately above, `\eqno`/`\leqno` outside math
            // mode take `report_illegal_case` ("You can't use `\eqno' in ...
            // mode"), not `insert_dollar_sign` -- tex.web lists them under the
            // separate Forbidden-cases module even though they share the same
            // `eq_no` command code as the math primitive vocabulary. Reaching this
            // arm proves `mode` is not `Math`/`DisplayMath`: the shared
            // mode/packed-command switch handles those cases directly.
            // `mmode+eq_no` itself (gated by `privileged`/`cur_group`) is
            // unaffected.
            P::EqNo | P::LeftEqNo => {
                let (direct_arg0,) = (command.spelling().semantic_token(),);
                finish_scan!(self.execute_illegal_eq_no(direct_arg0))
            }
            // TeX82 §1048's `any_mode(last_item)` Forbidden case: see
            // the illegal-last-item handler. These internal-only quantities
            // reach this function
            // only when delivered standalone (not mid-scan, where
            // `internal_value_from_command` already consumes them), exactly
            // like `\eqno`/`\leqno` above.
            P::LastKern
            | P::LastPenalty
            | P::LastSkip
            | P::FontCharWd
            | P::FontCharHt
            | P::FontCharDp
            | P::FontCharIc
            | P::ParShapeLength
            | P::ParShapeIndent
            | P::ParShapeDimen
            | P::NumExpr
            | P::DimExpr
            | P::GlueExpr
            | P::MuExpr
            | P::GlueStretch
            | P::GlueShrink
            | P::GlueStretchOrder
            | P::GlueShrinkOrder
            | P::GlueToMu
            | P::MuToGlue => {
                let (direct_arg0, direct_arg1, direct_arg2) = (
                    command.spelling().semantic_token(),
                    processor.error_context(),
                    processor.current_diagnostic_site(Some(&command)),
                );
                finish_scan!(self.execute_illegal_last_item(direct_arg0, direct_arg1, direct_arg2))
            }
            // TeX82 §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`.
            // `\cr` and `\crcr` carry the `car_ret` command code (chr `cr_code`
            // and `cr_cr_code`); `\span` carries `tab_mark` with chr `span_code`.
            // `get_next` (§342) only diverts them into a v-template when
            // `align_state=0`, so every other occurrence -- inside an alignment
            // cell whose braces are unbalanced, or outside any alignment at all --
            // is main control's to recover through §1127.
            P::Cr | P::CrCr | P::Span => {
                let (direct_arg0,) = (command,);
                finish_scan!(self.scan_align_error(direct_arg0))
            }
            // TeX82 §1126's `any_mode(no_align): no_align_error` and
            // `any_mode(omit): omit_error` (§1129). Both routines are a
            // `print_err`/`help2`/`error` triple and nothing else: report the
            // command-specific legal position, then discard the primitive.
            P::NoAlign | P::Omit => {
                let (direct_arg0,) = (primitive == P::Omit,);
                finish_scan!(self.execute_misplaced_alignment_command(direct_arg0))
            }
            // e-TeX 2.6 `etex.ch` [17.3822--3880] adds four nonzero modifiers to
            // TeX82's `valign` command code. In horizontal mode `eTeX_enabled`
            // first checks `TeXXeT_state>0`; only the enabled branch appends the
            // corresponding zero-width math node. The ordinary zero modifier
            // remains `\valign` and is dispatched above as an alignment.
            P::BeginL | P::BeginR | P::EndL | P::EndR => {
                let direction = match primitive {
                    P::BeginL => tex_state::node::Direction::BeginL,
                    P::BeginR => tex_state::node::Direction::BeginR,
                    P::EndL => tex_state::node::Direction::EndL,
                    P::EndR => tex_state::node::Direction::EndR,
                    _ => unreachable!("text-direction primitive matched above"),
                };
                {
                    let (direct_arg0, direct_arg1) =
                        (direction, processor.int_param(IntParam::TEX_XET_STATE) > 0);
                    finish_scan!(self.execute_text_direction(direct_arg0, direct_arg1))
                }
            }
            P::DiscretionaryHyphen
            | P::GlobalDefs
            | P::PdfEfCode
            | P::PdfKnacCode
            | P::PdfKnbcCode
            | P::PdfKnbsCode
            | P::PdfLpCode
            | P::PdfNoLigatures
            | P::PdfRpCode
            | P::PdfShbsCode
            | P::PdfStbsCode
            | P::PdfTagCode
            | P::SpaceFactor
            | P::VFil
            | P::VFilNeg
            | P::VFill
            | P::VSkip
            | P::VSs => Err(ExecError::UnimplementedPrimitive {
                primitive,
                mode,
                origin: command.origin(),
            }),
        }
    }
}

use super::*;
use smallvec::SmallVec;

pub(super) struct PageOverlay {
    pub(super) pending_effect_count: usize,
    pub(super) effects: Vec<PageEffect>,
    pub(super) open_out_occurrences: Vec<(usize, tex_state::EffectPos)>,
    pub(super) math: Vec<MathSubstitution>,
    pub(super) directions: Vec<DirectionPermutation>,
    pub(super) omitted_whatsits: Vec<(tex_state::node_arena::NodeListRef, usize)>,
    pub(super) diagnostics: Vec<(PrintSink, String)>,
    #[cfg(test)]
    pub(super) base_whatsit_visits: Vec<BaseWhatsitVisit>,
    color_target: tex_state::PdfColorStackTarget,
    running_thread_depth: Option<usize>,
    output_open_context: String,
    announce_openout: bool,
}

pub(super) struct MathSubstitution {
    pub(super) list: tex_state::node_arena::NodeListRef,
    pub(super) index: usize,
    pub(super) replacement: tex_state::node_arena::NodeListRef,
}

pub(super) struct DirectionPermutation {
    pub(super) list: tex_state::node_arena::NodeListRef,
    pub(super) order: Vec<usize>,
}

struct NormalizeExpansion<'a> {
    write_expander: &'a mut super::WriteExpander<'a>,
    replay_expander: &'a mut super::ReplayTextExpander<'a>,
}

#[allow(clippy::too_many_arguments)] // Output traversal keeps independent immutable/replay inputs.
pub(super) fn normalize_page(
    root: tex_state::node_arena::NodeListRef,
    root_box: (bool, tex_state::node::BoxLr),
    effects_and_context: (PendingPageEffects, String, bool),
    stores: &mut Universe,
    write_expander: &mut super::WriteExpander<'_>,
    replay_expander: &mut super::ReplayTextExpander<'_>,
    color_target: tex_state::PdfColorStackTarget,
) -> Result<PageOverlay, ExecError> {
    let (root_vertical, root_box_lr) = root_box;
    let (pending, output_open_context, announce_openout) = effects_and_context;
    let PendingPageEffects {
        effects,
        open_out_occurrences,
    } = pending;
    let mut effects = effects;
    let snap_reference = if color_target == tex_state::PdfColorStackTarget::Page {
        stores.pdf_snap_reference()
    } else {
        (
            tex_state::scaled::Scaled::from_raw(0),
            tex_state::scaled::Scaled::from_raw(0),
        )
    };
    if snap_reference
        != (
            tex_state::scaled::Scaled::from_raw(0),
            tex_state::scaled::Scaled::from_raw(0),
        )
    {
        effects.push(PageEffect::PdfSnapState {
            x: snap_reference.0,
            y: snap_reference.1,
        });
    }
    if color_target == tex_state::PdfColorStackTarget::Page {
        for restoration in stores.pdf_page_color_stack_restorations() {
            effects.push(PageEffect::PdfColorStack {
                mode: lower_color_stack_mode(restoration.mode),
                payload: restoration.payload,
                page_start: true,
            });
        }
    }
    let pending_effect_count = effects.len();
    let mut overlay = PageOverlay {
        pending_effect_count,
        effects,
        open_out_occurrences,
        math: Vec::new(),
        directions: Vec::new(),
        omitted_whatsits: Vec::new(),
        diagnostics: Vec::new(),
        #[cfg(test)]
        base_whatsit_visits: Vec::new(),
        color_target,
        running_thread_depth: None,
        output_open_context,
        announce_openout,
    };
    let mut expansion = NormalizeExpansion {
        write_expander,
        replay_expander,
    };
    normalize_list(
        stores,
        &mut expansion,
        root,
        NormalizeListContext {
            suppress_deferred_streams: false,
            in_hlist: !root_vertical,
            box_lr: root_box_lr,
            depth: 1,
        },
        &mut overlay,
    )?;
    Ok(overlay)
}

enum NormalizeNode {
    Leaf,
    List(
        tex_state::node_arena::NodeListRef,
        bool,
        bool,
        tex_state::node::BoxLr,
    ),
    Lists([tex_state::node_arena::NodeListRef; 3]),
    Whatsit(Whatsit),
    Math(tex_state::math::MathListNode),
    Unsupported(&'static str),
}

#[derive(Clone, Copy)]
struct NormalizeLocation {
    in_hlist: bool,
    depth: usize,
}

#[derive(Clone, Copy)]
struct NormalizeListContext {
    suppress_deferred_streams: bool,
    in_hlist: bool,
    box_lr: tex_state::node::BoxLr,
    depth: usize,
}

fn normalize_list(
    stores: &mut Universe,
    expansion: &mut NormalizeExpansion<'_>,
    list: tex_state::node_arena::NodeListRef,
    context: NormalizeListContext,
    overlay: &mut PageOverlay,
) -> Result<(), ExecError> {
    let NormalizeListContext {
        suppress_deferred_streams,
        in_hlist,
        box_lr,
        depth,
    } = context;
    check_depth(depth)?;
    let (active_indices, permutation) = {
        let nodes = list.nodes();
        if !nodes.requires_shipout_normalization() {
            return Ok(());
        }
        let permutation = direction_permutation_for_box(nodes, box_lr);
        let mut active_indices = SmallVec::<[usize; 32]>::new();
        if let Some(order) = permutation.as_deref() {
            active_indices.extend(order.iter().copied().filter(|&index| {
                nodes
                    .node_requires_shipout_normalization(index)
                    .expect("direction permutation index belongs to the frozen list")
            }));
        } else {
            active_indices.extend((0..nodes.len()).filter(|&index| {
                nodes
                    .node_requires_shipout_normalization(index)
                    .expect("normalization index belongs to the frozen list")
            }));
        }
        (active_indices, permutation)
    };
    if let Some(order) = permutation {
        overlay.directions.push(DirectionPermutation {
            list: list.clone(),
            order,
        });
    }
    for index in active_indices {
        normalize_index(
            stores,
            expansion,
            list.clone(),
            index,
            suppress_deferred_streams,
            NormalizeLocation { in_hlist, depth },
            overlay,
        )?;
    }
    Ok(())
}

fn normalize_index(
    stores: &mut Universe,
    expansion: &mut NormalizeExpansion<'_>,
    list: tex_state::node_arena::NodeListRef,
    index: usize,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
    overlay: &mut PageOverlay,
) -> Result<(), ExecError> {
    let NormalizeLocation { in_hlist, depth } = location;
    let action = {
        let node = list
            .get(index)
            .expect("normalization index belongs to the frozen list");
        match node {
            Node::HList(box_node) => NormalizeNode::List(
                box_node.children,
                suppress_deferred_streams,
                true,
                box_node.box_lr,
            ),
            Node::VList(box_node) => NormalizeNode::List(
                box_node.children,
                suppress_deferred_streams,
                false,
                box_node.box_lr,
            ),
            Node::Glue {
                leader: Some(StateLeaderPayload::HList(box_node)),
                ..
            } => NormalizeNode::List(box_node.children, true, true, box_node.box_lr),
            Node::Glue {
                leader: Some(StateLeaderPayload::VList(box_node)),
                ..
            } => NormalizeNode::List(box_node.children, true, false, box_node.box_lr),
            Node::Disc {
                pre, post, replace, ..
            } => NormalizeNode::Lists([pre, post, replace]),
            Node::Ins { content, .. } => NormalizeNode::List(
                content,
                suppress_deferred_streams,
                in_hlist,
                tex_state::node::BoxLr::Normal,
            ),
            Node::Adjust(adjust) => NormalizeNode::List(
                adjust.content,
                suppress_deferred_streams,
                in_hlist,
                tex_state::node::BoxLr::Normal,
            ),
            Node::Whatsit(whatsit) => NormalizeNode::Whatsit(whatsit),
            Node::MathList(math) => NormalizeNode::Math(math),
            Node::Unset(_) => NormalizeNode::Unsupported("unset alignment"),
            Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::Nonscript => NormalizeNode::Unsupported("math"),
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Kern { .. }
            | Node::MarginKern { .. }
            | Node::Glue { .. }
            | Node::Penalty(_)
            | Node::Rule { .. }
            | Node::Mark { .. }
            | Node::MathOn(_)
            | Node::MathOff(_)
            | Node::Direction(_) => NormalizeNode::Leaf,
        }
    };
    match action {
        NormalizeNode::Leaf => {}
        NormalizeNode::List(child, suppress, child_in_hlist, child_box_lr) => {
            normalize_list(
                stores,
                expansion,
                child,
                NormalizeListContext {
                    suppress_deferred_streams: suppress,
                    in_hlist: child_in_hlist,
                    box_lr: child_box_lr,
                    depth: depth + 1,
                },
                overlay,
            )?;
        }
        NormalizeNode::Lists(children) => {
            for child in children {
                normalize_list(
                    stores,
                    expansion,
                    child,
                    NormalizeListContext {
                        suppress_deferred_streams,
                        in_hlist,
                        box_lr: tex_state::node::BoxLr::Normal,
                        depth: depth + 1,
                    },
                    overlay,
                )?;
            }
        }
        NormalizeNode::Whatsit(whatsit) => {
            #[cfg(test)]
            if let Some(kind) = base_whatsit_visit_kind(&whatsit) {
                overlay.base_whatsit_visits.push(BaseWhatsitVisit {
                    in_hlist,
                    position: index,
                    kind,
                });
            }
            let anchored = whatsit_is_anchored(&whatsit, suppress_deferred_streams);
            let effect_count = overlay.effects.len();
            append_whatsit_effect(
                stores,
                expansion,
                overlay,
                whatsit,
                suppress_deferred_streams,
                location,
            )?;
            if anchored && overlay.effects.len() == effect_count {
                overlay.omitted_whatsits.push((list.clone(), index));
            }
        }
        NormalizeNode::Math(math) => {
            let mut nodes = crate::math::finish_math_list_node(stores, math, false);
            let replacement = stores.freeze_node_list_owned(&mut nodes);
            overlay.math.push(MathSubstitution {
                list: list.clone(),
                index,
                replacement: replacement.clone(),
            });
            normalize_list(
                stores,
                expansion,
                replacement,
                NormalizeListContext {
                    suppress_deferred_streams,
                    in_hlist,
                    box_lr: tex_state::node::BoxLr::Normal,
                    depth: depth + 1,
                },
                overlay,
            )?;
        }
        NormalizeNode::Unsupported(node) => {
            return Err(ExecError::UnsupportedShipoutNode { node });
        }
    }
    Ok(())
}

#[cfg(test)]
fn base_whatsit_visit_kind(whatsit: &Whatsit) -> Option<BaseWhatsitVisitKind> {
    match whatsit {
        Whatsit::OpenOut { .. } => Some(BaseWhatsitVisitKind::OpenOut),
        Whatsit::DeferredWrite { .. } => Some(BaseWhatsitVisitKind::DeferredWrite),
        Whatsit::CloseOut { slot: Some(_) } => Some(BaseWhatsitVisitKind::NumberedCloseOut),
        Whatsit::CloseOut { slot: None } => Some(BaseWhatsitVisitKind::FallbackCloseOut),
        Whatsit::Special { .. } => Some(BaseWhatsitVisitKind::Special),
        Whatsit::Language { .. } => Some(BaseWhatsitVisitKind::Language),
        _ => None,
    }
}

fn append_whatsit_effect(
    stores: &mut Universe,
    expansion: &mut NormalizeExpansion<'_>,
    overlay: &mut PageOverlay,
    whatsit: Whatsit,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
) -> Result<(), ExecError> {
    let NormalizeLocation { in_hlist, depth } = location;
    let color_target = overlay.color_target;
    let announce_openout = overlay.announce_openout;
    let output_open_context = overlay.output_open_context.clone();
    let effects = &mut overlay.effects;
    let open_out_occurrences = &mut overlay.open_out_occurrences;
    let diagnostics = &mut overlay.diagnostics;
    let running_thread_depth = &mut overlay.running_thread_depth;
    match whatsit {
        Whatsit::OpenOut { slot, path } if !suppress_deferred_streams => {
            // TeX82 §1374 closes the old stream before it attempts the
            // replacement, even when every subsequent open attempt fails.
            stores.close_output_stream(slot);
            let path = retry_openout_target(stores, path, &output_open_context)?;
            stores.open_output_stream(slot, path.clone());
            stores
                .world_mut()
                .set_last_stream_open_context(output_open_context);
            open_out_occurrences.push((effects.len(), stores.world().effect_pos()));
            // web2c's `[53.1374]` log notice, which follows `write_open[j]:=
            // true`. It has to come after the context attach above, which
            // requires the `StreamOpen` record to still be the last effect.
            if announce_openout {
                crate::diagnostics::report_openout(stores, slot.raw(), &path);
            }
            effects.push(PageEffect::OpenOut {
                stream: slot.raw(),
                path,
            });
        }
        Whatsit::CloseOut { slot } if !suppress_deferred_streams => {
            if let Some(slot) = slot {
                stores.close_output_stream(slot);
                effects.push(PageEffect::CloseOut { stream: slot.raw() });
            }
        }
        Whatsit::DeferredWrite { sink, tokens } if !suppress_deferred_streams => {
            let expanded = (expansion.write_expander)(stores, sink, tokens.id())?;
            let text = expanded.text;
            if let Some(sink) = deferred_write_sink(stores, sink) {
                // TeX82 §1370's `write_out` frames the expansion as
                // `print_nl(""); token_show(def_ref); print_ln` when the
                // stream is not an open file. The trailing `print_ln` is part
                // of the write's own text; the leading `print_nl` is not.
                if expanded.publication == crate::shipout::WritePublication::Transactional {
                    if write_line_is_open(stores, sink) {
                        stores.world_mut().write_text(sink, "\n");
                    }
                    stores.world_mut().write_text(sink, &text);
                }
                effects.push(PageEffect::Write {
                    sink: lower_sink(sink),
                    text,
                });
            }
        }
        Whatsit::Special { class, payload } => {
            effects.push(PageEffect::Special { class, payload });
        }
        Whatsit::DeferredSpecial { class, tokens } => {
            let crate::shipout::ExpandedReplayText(payload) =
                (expansion.replay_expander)(stores, super::ReplayTextKind::Special, tokens.id())?;
            effects.push(PageEffect::Special { class, payload });
        }
        Whatsit::PdfReferenceObject { object } => {
            stores
                .reference_pdf_raw_object(object)
                .map_err(|_| ExecError::PdfReferencedObjectNotFound)?;
        }
        Whatsit::PdfAccessibility(control) => {
            effects.push(PageEffect::PdfAccessibility(match control {
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn => {
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOn
                }
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff => {
                    tex_out::PdfAccessibilityEffect::InterwordSpaceOff
                }
                tex_state::node::PdfAccessibilityControl::FakeSpace => {
                    tex_out::PdfAccessibilityEffect::FakeSpace
                }
            }));
        }
        Whatsit::PdfAnnotation { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::Annotation { object },
            ));
        }
        Whatsit::PdfLinkStart { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::LinkStart { object },
            ));
        }
        Whatsit::PdfLinkEnd { object } => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::LinkEnd { object },
            ));
        }
        Whatsit::PdfRunningLink(enabled) => {
            effects.push(PageEffect::PdfAnnotation(
                tex_out::PdfAnnotationEffect::RunningLink(enabled),
            ));
        }
        Whatsit::PdfLiteral { mode, payload } => effects.push(PageEffect::PdfLiteral {
            mode: lower_pdf_literal_mode(mode),
            payload,
        }),
        Whatsit::DeferredPdfLiteral { mode, tokens } => {
            let crate::shipout::ExpandedReplayText(payload) = (expansion.replay_expander)(
                stores,
                super::ReplayTextKind::PdfLiteral,
                tokens.id(),
            )?;
            effects.push(PageEffect::PdfLiteral {
                mode: lower_pdf_literal_mode(mode),
                payload,
            });
        }
        Whatsit::PdfSetMatrix { payload } => {
            validate_pdf_matrix(&payload)?;
            effects.push(PageEffect::PdfSetMatrix { payload });
        }
        Whatsit::PdfSave => effects.push(PageEffect::PdfSave),
        Whatsit::PdfRestore => effects.push(PageEffect::PdfRestore),
        Whatsit::PdfColorStack { id, action } => {
            match stores.apply_pdf_color_stack(id, color_target, &action) {
                Ok(emission) => effects.push(PageEffect::PdfColorStack {
                    mode: lower_color_stack_mode(emission.mode),
                    payload: emission.payload,
                    page_start: false,
                }),
                Err(tex_state::PdfColorStackApplyError::Underflow) => {
                    let target = match color_target {
                        tex_state::PdfColorStackTarget::Page => "page",
                        tex_state::PdfColorStackTarget::Form => "form",
                    };
                    stores.world_mut().write_text(
                        tex_state::PrintSink::TerminalAndLog,
                        &format!("pop empty color {target} stack {id}\n"),
                    );
                }
                Err(tex_state::PdfColorStackApplyError::Unknown) => {
                    unreachable!("validated color stack id")
                }
            }
        }
        Whatsit::PdfSavePos => effects.push(PageEffect::PdfSavePosition),
        Whatsit::PdfSnapRefPoint => effects.push(PageEffect::PdfSnapRefPoint),
        Whatsit::PdfSnapY { glue } => effects.push(PageEffect::PdfSnapY {
            spec: super::lower_glue(stores.glue_spec(glue)),
        }),
        Whatsit::PdfSnapYComp { ratio } => effects.push(PageEffect::PdfSnapYComp { ratio }),
        Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        } => {
            if stores.pdf_form_artifact(object).is_none() {
                let form = stores
                    .pdf_form(object)
                    .ok_or(ExecError::PdfReferencedObjectNotFound)?;
                let artifact = super::stage_form(
                    form,
                    stores,
                    expansion.write_expander,
                    expansion.replay_expander,
                )?;
                stores.publish_pdf_traversal_positions(
                    artifact.last_position(),
                    artifact.snap_reference(),
                );
                stores.set_pdf_form_artifact(object, artifact);
            }
            effects.push(PageEffect::PdfRefXForm {
                object,
                width,
                height,
                depth,
            });
        }
        Whatsit::PdfRefXImage {
            object,
            width,
            height,
            depth,
        } => effects.push(PageEffect::PdfRefXImage {
            object,
            width,
            height,
            depth,
        }),
        Whatsit::PdfDestination(destination) => {
            let tex_state::node::PdfDestinationNode {
                identifier,
                structure,
                kind,
            } = *destination;
            if suppress_deferred_streams {
                return Ok(());
            }
            if color_target == tex_state::PdfColorStackTarget::Form {
                return Err(ExecError::PdfDestinationInForm);
            }
            let identity = match identifier {
                tex_state::PdfActionIdentifier::Name(tokens) => {
                    let mut text = String::new();
                    for &token in stores.tokens(tokens.id()).iter() {
                        tex_state::token_show::append_token_string_text(stores, token, &mut text);
                    }
                    tex_state::PdfDestinationIdentity::Name(text.into_bytes())
                }
                tex_state::PdfActionIdentifier::Number(number) => {
                    tex_state::PdfDestinationIdentity::Number(number)
                }
                tex_state::PdfActionIdentifier::Raw(_) => {
                    unreachable!("destination scanner uses typed identifiers")
                }
            };
            let definition = stores
                .define_pdf_destination(identity.clone(), structure)
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            if definition.duplicate {
                if stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_DEST) <= 0 {
                    diagnostics.push((
                        PrintSink::TerminalAndLog,
                        pdf_destination_duplicate_warning(&identity),
                    ));
                }
                return Ok(());
            }
            let identifier = match identity {
                tex_state::PdfDestinationIdentity::Name(name) => {
                    tex_out::PdfDestinationIdentifier::Name(name)
                }
                tex_state::PdfDestinationIdentity::Number(number) => {
                    tex_out::PdfDestinationIdentifier::Number(number)
                }
            };
            let kind = match kind {
                tex_state::node::PdfDestinationKind::Xyz { zoom } => {
                    tex_out::PdfDestinationKind::Xyz { zoom }
                }
                tex_state::node::PdfDestinationKind::FitBoundingBoxHorizontal => {
                    tex_out::PdfDestinationKind::FitBoundingBoxHorizontal
                }
                tex_state::node::PdfDestinationKind::FitBoundingBoxVertical => {
                    tex_out::PdfDestinationKind::FitBoundingBoxVertical
                }
                tex_state::node::PdfDestinationKind::FitBoundingBox => {
                    tex_out::PdfDestinationKind::FitBoundingBox
                }
                tex_state::node::PdfDestinationKind::FitHorizontal => {
                    tex_out::PdfDestinationKind::FitHorizontal
                }
                tex_state::node::PdfDestinationKind::FitVertical => {
                    tex_out::PdfDestinationKind::FitVertical
                }
                tex_state::node::PdfDestinationKind::FitRectangle(dimensions) => {
                    tex_out::PdfDestinationKind::FitRectangle {
                        width: dimensions.width,
                        height: dimensions.height,
                        depth: dimensions.depth,
                    }
                }
                tex_state::node::PdfDestinationKind::Fit => tex_out::PdfDestinationKind::Fit,
            };
            effects.push(PageEffect::PdfDestination(tex_out::PdfDestinationEffect {
                object: definition.record.object(),
                identifier,
                structure,
                kind,
                margin: stores.dimen_param(DimenParam::PDF_DEST_MARGIN),
            }));
        }
        Whatsit::PdfThread(thread) => {
            let tex_state::node::PdfThreadNode {
                identifier,
                dimensions,
                attributes,
                running,
            } = *thread;
            if suppress_deferred_streams {
                return Ok(());
            }
            if running && in_hlist {
                return Err(ExecError::PdfNavigation(
                    "pdfTeX error (ext4): \\pdfstartthread ended up in hlist",
                ));
            }
            if color_target == tex_state::PdfColorStackTarget::Form {
                return Err(ExecError::PdfThreadInForm);
            }
            let identity = match identifier {
                tex_state::PdfActionIdentifier::Name(tokens) => {
                    let mut text = String::new();
                    for &token in stores.tokens(tokens.id()).iter() {
                        tex_state::token_show::append_token_string_text(stores, token, &mut text);
                    }
                    tex_state::PdfDestinationIdentity::Name(text.into_bytes())
                }
                tex_state::PdfActionIdentifier::Number(number) => {
                    tex_state::PdfDestinationIdentity::Number(number)
                }
                tex_state::PdfActionIdentifier::Raw(_) => {
                    unreachable!("thread scanner uses typed identifiers")
                }
            };
            let (thread, bead) = stores
                .append_pdf_thread_bead(identity.clone())
                .map_err(|_| ExecError::PdfObjectCapacity)?;
            let identifier = match identity {
                tex_state::PdfDestinationIdentity::Name(name) => {
                    tex_out::PdfDestinationIdentifier::Name(name)
                }
                tex_state::PdfDestinationIdentity::Number(number) => {
                    tex_out::PdfDestinationIdentifier::Number(number)
                }
            };
            let mut attribute_bytes = String::new();
            for &token in stores.tokens(attributes.id()).iter() {
                tex_state::token_show::append_token_string_text(
                    stores,
                    token,
                    &mut attribute_bytes,
                );
            }
            let marker = tex_out::PdfThreadEffect {
                thread_object: thread.object(),
                bead_object: bead.bead_object(),
                rectangle_object: bead.rectangle_object(),
                identifier,
                width: dimensions.width,
                height: dimensions.height,
                depth: dimensions.depth,
                attributes: attribute_bytes.into_bytes(),
                margin: stores.dimen_param(DimenParam::PDF_THREAD_MARGIN),
            };
            if running {
                *running_thread_depth = Some(depth);
            }
            effects.push(if running {
                PageEffect::PdfStartThread(marker)
            } else {
                PageEffect::PdfThread(marker)
            });
        }
        Whatsit::PdfEndThread if in_hlist => {
            return Err(ExecError::PdfNavigation(
                "pdfTeX error (ext4): \\pdfendthread ended up in hlist",
            ));
        }
        Whatsit::PdfEndThread => match running_thread_depth.take() {
            Some(start_depth) if start_depth != depth => {
                return Err(ExecError::PdfNavigation(
                    "pdfTeX error (ext4): \\pdfendthread ended up in different nesting level than \\pdfstartthread",
                ));
            }
            _ => effects.push(PageEffect::PdfEndThread),
        },
        Whatsit::OpenOut { .. }
        | Whatsit::CloseOut { .. }
        | Whatsit::DeferredWrite { .. }
        | Whatsit::Language { .. } => {}
    }
    Ok(())
}

/// Resolves TeX82 §1370's live selector when a deferred write reaches
/// shipout. `Stream`, `TerminalAndLog`, and `Log` retain §1342's normalized
/// numbered, above-range, and negative stream identities until this point.
pub(super) fn deferred_write_sink(
    stores: &Universe,
    sink: tex_state::PrintSink,
) -> Option<tex_state::PrintSink> {
    let selector = tex_state::print::Selector::for_interaction(stores.interaction_mode());
    match sink {
        tex_state::PrintSink::Stream(slot) if stores.output_stream_is_open(slot) => Some(sink),
        tex_state::PrintSink::Stream(_) | tex_state::PrintSink::TerminalAndLog => selector.sink(),
        tex_state::PrintSink::Log if selector == tex_state::print::Selector::TermAndLog => {
            Some(tex_state::PrintSink::Log)
        }
        tex_state::PrintSink::Log => selector.sink(),
        tex_state::PrintSink::Terminal => Some(tex_state::PrintSink::Terminal),
    }
}

/// TeX82 §§1373--1374's `out_what` open loop.
fn retry_openout_target(
    stores: &mut Universe,
    name: String,
    context: &str,
) -> Result<String, ExecError> {
    let mut path = openout_target(name);
    while stores.world().retained_output_open_outcome(&path)
        == tex_state::RetainedOutputOpenOutcome::Unavailable
    {
        let interaction = stores.interaction_mode();
        if matches!(
            interaction,
            tex_state::InteractionMode::Batch | tex_state::InteractionMode::Nonstop
        ) {
            stores
                .print_err("I can't write on file `")
                .print(&path)
                .print("'.");
            return Err(ExecError::Fatal(tex_command::FatalError::emergency_stop(
                "job aborted, file error in nonstop mode",
            )));
        }
        let mut report = stores.print_err("I can't write on file `");
        report
            .print(&path)
            .print("'.")
            .print_rendered(context)
            .print_rendered("\n")
            .print("Please type another output file name");
        drop(report);
        let replacement = stores
            .command_context()
            .input_ln(tex_state::CommandLineSource::Terminal { prompt: ": " })
            .ok_or(ExecError::Fatal(tex_command::FatalError::emergency_stop(
                "End of file on the terminal!",
            )))?;
        path = scan_terminal_output_name(&replacement);
    }
    Ok(path)
}

/// TeX82 §§530 and 1374 scan a replacement from the terminal buffer, not
/// through `scan_file_name`'s expanded-token path.
pub(super) fn scan_terminal_output_name(line: &str) -> String {
    let mut name = String::new();
    let mut quoted = false;
    for character in line.trim_start_matches(' ').chars() {
        match character {
            '"' => quoted = !quoted,
            ' ' if !quoted => break,
            _ => name.push(character),
        }
    }
    if name.is_empty() {
        return ".tex".to_owned();
    }
    openout_target(name)
}

fn openout_target(name: String) -> String {
    let mut path = std::path::PathBuf::from(name);
    if path.extension().is_none() {
        path.set_extension("tex");
    }
    path.to_string_lossy().into_owned()
}

fn pdf_destination_duplicate_warning(identity: &tex_state::PdfDestinationIdentity) -> String {
    let identity = match identity {
        tex_state::PdfDestinationIdentity::Name(name) => {
            format!("name{{{}}}", String::from_utf8_lossy(name))
        }
        tex_state::PdfDestinationIdentity::Number(number) => format!("num{number}"),
    };
    format!(
        "\npdfTeX warning (ext4): destination with the same identifier ({identity}) has been already used, duplicate ignored\n"
    )
}

fn validate_pdf_matrix(payload: &[u8]) -> Result<(), ExecError> {
    let valid = std::str::from_utf8(payload).ok().is_some_and(|text| {
        let mut fields = text.split_ascii_whitespace();
        let four_finite = (0..4).all(|_| {
            fields
                .next()
                .and_then(|field| field.parse::<f64>().ok())
                .is_some_and(f64::is_finite)
        });
        four_finite && fields.next().is_none()
    });
    if valid {
        Ok(())
    } else {
        Err(ExecError::InvalidShipoutArtifact(
            "pdfTeX error (\\pdfsetmatrix): Unrecognized format.".to_owned(),
        ))
    }
}

fn lower_pdf_literal_mode(mode: tex_state::node::PdfLiteralMode) -> tex_out::PdfLiteralMode {
    match mode {
        tex_state::node::PdfLiteralMode::Origin => tex_out::PdfLiteralMode::Origin,
        tex_state::node::PdfLiteralMode::Page => tex_out::PdfLiteralMode::Page,
        tex_state::node::PdfLiteralMode::Direct => tex_out::PdfLiteralMode::Direct,
    }
}

fn lower_color_stack_mode(mode: tex_state::PdfColorStackMode) -> tex_out::PdfLiteralMode {
    match mode {
        tex_state::PdfColorStackMode::Origin => tex_out::PdfLiteralMode::Origin,
        tex_state::PdfColorStackMode::Page => tex_out::PdfLiteralMode::Page,
        tex_state::PdfColorStackMode::Direct => tex_out::PdfLiteralMode::Direct,
    }
}

pub(super) fn direction_permutation_for_box(
    nodes: NodeList<'_>,
    box_lr: tex_state::node::BoxLr,
) -> Option<Vec<usize>> {
    if box_lr == tex_state::node::BoxLr::Reversed {
        return None;
    }
    direction_permutation(nodes)
}

fn direction_permutation(nodes: NodeList<'_>) -> Option<Vec<usize>> {
    struct Segment {
        begin: Direction,
        chunks: Vec<Vec<usize>>,
    }
    fn append(target: &mut Vec<usize>, stack: &mut [Segment], index: usize) {
        if let Some(segment) = stack.last_mut() {
            segment.chunks.push(vec![index]);
        } else {
            target.push(index);
        }
    }
    fn finish(target: &mut Vec<usize>, stack: &mut Vec<Segment>) {
        let Some(mut segment) = stack.pop() else {
            return;
        };
        if segment.begin == Direction::BeginR {
            segment.chunks.reverse();
        }
        let nodes = segment.chunks.into_iter().flatten().collect::<Vec<_>>();
        if let Some(parent) = stack.last_mut() {
            parent.chunks.push(nodes);
        } else {
            target.extend(nodes);
        }
    }

    if !nodes.contains_direction() {
        return None;
    }
    let mut reordered = Vec::with_capacity(nodes.len());
    let mut stack = Vec::<Segment>::new();
    for (index, node) in nodes.into_iter().enumerate() {
        match node {
            NodeRef::Direction(
                begin @ (Direction::BeginM | Direction::BeginL | Direction::BeginR),
            ) => stack.push(Segment {
                begin,
                chunks: Vec::new(),
            }),
            NodeRef::Direction(Direction::EndL)
                if stack
                    .last()
                    .is_some_and(|segment| segment.begin == Direction::BeginL) =>
            {
                finish(&mut reordered, &mut stack);
            }
            NodeRef::Direction(Direction::EndR)
                if stack
                    .last()
                    .is_some_and(|segment| segment.begin == Direction::BeginR) =>
            {
                finish(&mut reordered, &mut stack);
            }
            NodeRef::Direction(Direction::EndM)
                if stack
                    .last()
                    .is_some_and(|segment| segment.begin == Direction::BeginM) =>
            {
                finish(&mut reordered, &mut stack);
            }
            NodeRef::Direction(_) => {}
            _ => append(&mut reordered, &mut stack, index),
        }
    }
    while !stack.is_empty() {
        finish(&mut reordered, &mut stack);
    }
    Some(reordered)
}

/// TeX82 §62's `print_nl` test, applied to a `\write`'s own sink.
///
/// §1370 writes an unopened stream through `print_nl("")`, whose guard is
/// `((term_offset>0)and(odd(selector)))or((file_offset>0)and(selector>=
/// log_only))`. A `\write` to a real file has no column to break.
pub(super) fn write_line_is_open(stores: &Universe, sink: tex_state::PrintSink) -> bool {
    let bufs = stores.world().stream_bufs();
    let terminal = !bufs.terminal_partial_line().is_empty();
    let log = !bufs.log_partial_line().is_empty();
    match sink {
        tex_state::PrintSink::Terminal => terminal,
        tex_state::PrintSink::Log => log,
        tex_state::PrintSink::TerminalAndLog => terminal || log,
        tex_state::PrintSink::Stream(_) => false,
    }
}

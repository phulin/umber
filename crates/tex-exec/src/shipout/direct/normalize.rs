use super::*;
use smallvec::SmallVec;
use tex_state::world::ArtifactEffectOrdinal;

pub(super) struct PageOverlay<G> {
    pub(super) pending_effect_count: usize,
    pub(super) effects: Vec<PageEffect>,
    pub(super) open_out_occurrences: Vec<(usize, ArtifactEffectOrdinal)>,
    pub(super) math: Vec<MathSubstitution<G>>,
    pub(super) directions: Vec<DirectionPermutation<G>>,
    pub(super) omitted_whatsits: Vec<(tex_state::ShipoutListId<G>, usize)>,
    pub(super) diagnostics: Vec<(PrintSink, String)>,
    #[cfg(test)]
    pub(super) base_whatsit_visits: Vec<BaseWhatsitVisit>,
    color_target: tex_state::PdfColorStackTarget,
    running_thread_depth: Option<usize>,
    output_open_context: String,
    announce_openout: bool,
}

pub(super) struct MathSubstitution<G> {
    pub(super) list: tex_state::ShipoutListId<G>,
    pub(super) index: usize,
    pub(super) replacement: tex_state::ShipoutListId<G>,
}

pub(super) struct DirectionPermutation<G> {
    pub(super) list: tex_state::ShipoutListId<G>,
    pub(super) order: Vec<usize>,
}

struct NormalizeExpansion<'a, G> {
    write_expander: &'a mut super::WriteExpander<'a, G>,
    replay_expander: &'a mut super::ReplayTextExpander<'a, G>,
}

#[allow(clippy::too_many_arguments)] // Output traversal keeps independent immutable/replay inputs.
pub(super) fn normalize_page<G>(
    root: tex_state::ShipoutListId<G>,
    root_box: (bool, tex_state::node::BoxLr),
    effects_and_context: (PendingPageEffects, String, bool),
    stores: &mut Universe<G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    write_expander: &mut super::WriteExpander<'_, G>,
    replay_expander: &mut super::ReplayTextExpander<'_, G>,
    color_target: tex_state::PdfColorStackTarget,
) -> Result<PageOverlay<G>, ExecError> {
    let (root_vertical, root_box_lr) = root_box;
    let (pending, output_open_context, announce_openout) = effects_and_context;
    let PendingPageEffects {
        effects,
        open_out_occurrences,
    } = pending;
    let mut effects = effects;
    let snap_reference = if color_target == tex_state::PdfColorStackTarget::Page {
        stores
            .command_context()
            .expect("shipout normalization runs inside an admitted command episode")
            .pdf_snap_reference()
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
        let restorations = stores
            .command_context()
            .expect("shipout normalization runs inside an admitted command episode")
            .pdf_page_color_stack_restorations();
        for restoration in restorations {
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
        diagnostic_effects,
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

enum NormalizeNode<G> {
    Leaf,
    List(
        tex_state::ShipoutListId<G>,
        bool,
        bool,
        tex_state::node::BoxLr,
    ),
    Lists([tex_state::ShipoutListId<G>; 3]),
    Whatsit(PreparedWhatsit<G>),
    Math(tex_state::math::MathListNode),
    Unsupported(&'static str),
}

enum PreparedWhatsit<G> {
    DeferredWrite {
        sink: PrintSink,
        tokens: tex_state::ShipoutTokenSource<G>,
    },
    DeferredSpecial {
        class: String,
        tokens: tex_state::ShipoutTokenSource<G>,
    },
    DeferredPdfLiteral {
        mode: tex_state::node::PdfLiteralMode,
        tokens: tex_state::ShipoutTokenSource<G>,
    },
    PdfThread {
        identifier: PreparedPdfIdentifier<G>,
        dimensions: tex_state::PdfAnnotationDimensions,
        attributes: tex_state::ShipoutTokenSource<G>,
        running: bool,
    },
    PdfDestination {
        identifier: PreparedPdfIdentifier<G>,
        structure: Option<u32>,
        kind: tex_state::node::PdfDestinationKind,
    },
    PdfColorStack {
        id: u32,
        source: tex_state::ShipoutNodeSource<G>,
    },
    Other(Whatsit),
}

enum PreparedPdfIdentifier<G> {
    Tokens(tex_state::ShipoutTokenSource<G>),
    Number(u32),
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

fn normalize_list<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    expansion: &mut NormalizeExpansion<'_, G>,
    list: tex_state::ShipoutListId<G>,
    context: NormalizeListContext,
    overlay: &mut PageOverlay<G>,
) -> Result<(), ExecError> {
    let NormalizeListContext {
        suppress_deferred_streams,
        in_hlist,
        box_lr,
        depth,
    } = context;
    check_depth(depth)?;
    let (active_indices, permutation) = match list {
        tex_state::ShipoutListId::Page(id) => normalization_work(
            stores
                .page_node_list(id)
                .expect("shipout root belongs to the live page arena")
                .nodes(),
            box_lr,
        ),
        tex_state::ShipoutListId::Scratch(id) => normalization_work(
            stores
                .shipout_scratch_nodes(id)
                .expect("shipout scratch root belongs to the active transaction"),
            box_lr,
        ),
        tex_state::ShipoutListId::Durable(id) => normalization_work(
            stores
                .node_list(id)
                .expect("shipout root belongs to the live durable generation")
                .nodes(),
            box_lr,
        ),
    };
    if let Some(order) = permutation {
        overlay
            .directions
            .push(DirectionPermutation { list, order });
    }
    for index in active_indices {
        normalize_index(
            stores,
            diagnostic_effects,
            expansion,
            list,
            index,
            suppress_deferred_streams,
            NormalizeLocation { in_hlist, depth },
            overlay,
        )?;
    }
    Ok(())
}

fn normalization_work<List: Copy, Glue: Copy, Tokens>(
    nodes: &[Node<List, Glue, Tokens>],
    box_lr: tex_state::node::BoxLr,
) -> (SmallVec<[usize; 32]>, Option<Vec<usize>>) {
    let permutation = direction_permutation_for_box(nodes, box_lr);
    let mut active_indices = SmallVec::<[usize; 32]>::new();
    if let Some(order) = permutation.as_deref() {
        active_indices.extend(
            order
                .iter()
                .copied()
                .filter(|&index| node_requires_normalization(&nodes[index])),
        );
    } else {
        active_indices.extend(
            nodes
                .iter()
                .enumerate()
                .filter_map(|(index, node)| node_requires_normalization(node).then_some(index)),
        );
    }
    (active_indices, permutation)
}

fn node_requires_normalization<List, Glue, Tokens>(node: &Node<List, Glue, Tokens>) -> bool {
    matches!(
        node,
        Node::HList(_)
            | Node::VList(_)
            | Node::Unset(_)
            | Node::Disc { .. }
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::Direction(_)
            | Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript
            | Node::Adjust(_)
            | Node::Glue {
                leader: Some(_),
                ..
            }
    )
}

#[allow(clippy::too_many_arguments)] // Recursive normalization carries explicit replay and overlay state.
fn normalize_index<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    expansion: &mut NormalizeExpansion<'_, G>,
    list: tex_state::ShipoutListId<G>,
    index: usize,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
    overlay: &mut PageOverlay<G>,
) -> Result<(), ExecError> {
    let NormalizeLocation { in_hlist, depth } = location;
    let action = match list {
        tex_state::ShipoutListId::Page(id) => {
            let node = stores
                .page_node_list(id)
                .expect("shipout root belongs to the live page arena")
                .nodes()
                .get(index)
                .expect("normalization index belongs to the frozen list");
            classify_page_node(node, list, index, suppress_deferred_streams, in_hlist)
        }
        tex_state::ShipoutListId::Scratch(id) => {
            let node = stores
                .shipout_scratch_nodes(id)
                .expect("shipout scratch root belongs to the active transaction")
                .get(index)
                .expect("normalization index belongs to the frozen list");
            classify_scratch_node(node, list, index, suppress_deferred_streams, in_hlist)
        }
        tex_state::ShipoutListId::Durable(id) => {
            let node = stores
                .node_list(id)
                .expect("shipout root belongs to the live durable generation")
                .nodes()
                .get(index)
                .expect("normalization index belongs to the frozen list");
            classify_durable_node(node, list, index, suppress_deferred_streams, in_hlist)
        }
    };
    match action {
        NormalizeNode::Leaf => {}
        NormalizeNode::List(child, suppress, child_in_hlist, child_box_lr) => {
            normalize_list(
                stores,
                diagnostic_effects,
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
                    diagnostic_effects,
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
            if let Some(kind) = prepared_whatsit_visit_kind(&whatsit) {
                overlay.base_whatsit_visits.push(BaseWhatsitVisit {
                    in_hlist,
                    position: index,
                    kind,
                });
            }
            let anchored = prepared_whatsit_is_anchored(&whatsit, suppress_deferred_streams);
            let effect_count = overlay.effects.len();
            append_whatsit_effect(
                stores,
                diagnostic_effects,
                expansion,
                overlay,
                whatsit,
                suppress_deferred_streams,
                location,
            )?;
            if anchored && overlay.effects.len() == effect_count {
                overlay.omitted_whatsits.push((list, index));
            }
        }
        NormalizeNode::Math(math) => {
            let replacement = {
                let mut command = stores.command_context().expect("live generation");
                // A math list surviving to direct shipout is normalization
                // scratch outside command delivery. Its box dimensions are
                // not a canonical command-operation geometry transition.
                let mut geometry = crate::geometry::IgnorePackGeometry;
                crate::math::finish_math_list_node_to_shipout_scratch(
                    &mut command,
                    diagnostic_effects,
                    &mut geometry,
                    math,
                    false,
                )
            };
            let replacement = tex_state::ShipoutListId::Scratch(replacement);
            overlay.math.push(MathSubstitution {
                list,
                index,
                replacement,
            });
            normalize_list(
                stores,
                diagnostic_effects,
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

fn classify_page_node<G>(
    node: &Node,
    source: tex_state::ShipoutListId<G>,
    index: usize,
    suppress_deferred_streams: bool,
    in_hlist: bool,
) -> NormalizeNode<G> {
    if let Node::MathList(math) = node {
        return NormalizeNode::Math(*math);
    }
    if let Node::Whatsit(whatsit) = node {
        return NormalizeNode::Whatsit(prepare_whatsit(whatsit, source, index, |glue| glue));
    }
    classify_transient_node(
        node,
        tex_state::ShipoutListId::Page,
        suppress_deferred_streams,
        in_hlist,
    )
}

fn classify_scratch_node<G>(
    node: &tex_state::ShipoutScratchNode<G>,
    source: tex_state::ShipoutListId<G>,
    index: usize,
    suppress_deferred_streams: bool,
    in_hlist: bool,
) -> NormalizeNode<G> {
    if let Node::Whatsit(whatsit) = node {
        return NormalizeNode::Whatsit(prepare_whatsit(whatsit, source, index, |glue| glue));
    }
    classify_transient_node(node, |list| list, suppress_deferred_streams, in_hlist)
}

fn prepare_whatsit<G, Glue: Copy, Tokens: Clone>(
    whatsit: &Whatsit<Glue, Tokens>,
    source: tex_state::ShipoutListId<G>,
    index: usize,
    resolve_glue: impl Fn(Glue) -> tex_state::glue::GlueSpec,
) -> PreparedWhatsit<G> {
    let identifier = |identifier: &tex_state::node::NodePdfActionIdentifier,
                      field: tex_state::ShipoutTokenField| {
        match identifier {
            tex_state::node::NodePdfActionIdentifier::Name(_)
            | tex_state::node::NodePdfActionIdentifier::Raw(_) => PreparedPdfIdentifier::Tokens(
                tex_state::ShipoutTokenSource::new(source, index, field),
            ),
            tex_state::node::NodePdfActionIdentifier::Number(number) => {
                PreparedPdfIdentifier::Number(*number)
            }
        }
    };
    match whatsit {
        Whatsit::DeferredWrite { sink, .. } => PreparedWhatsit::DeferredWrite {
            sink: *sink,
            tokens: tex_state::ShipoutTokenSource::new(
                source,
                index,
                tex_state::ShipoutTokenField::DeferredWrite,
            ),
        },
        Whatsit::DeferredSpecial { class, .. } => PreparedWhatsit::DeferredSpecial {
            class: class.clone(),
            tokens: tex_state::ShipoutTokenSource::new(
                source,
                index,
                tex_state::ShipoutTokenField::DeferredSpecial,
            ),
        },
        Whatsit::DeferredPdfLiteral { mode, .. } => PreparedWhatsit::DeferredPdfLiteral {
            mode: *mode,
            tokens: tex_state::ShipoutTokenSource::new(
                source,
                index,
                tex_state::ShipoutTokenField::DeferredPdfLiteral,
            ),
        },
        Whatsit::PdfThread(thread) => PreparedWhatsit::PdfThread {
            identifier: identifier(
                &thread.identifier,
                tex_state::ShipoutTokenField::PdfThreadIdentifier,
            ),
            dimensions: thread.dimensions,
            attributes: tex_state::ShipoutTokenSource::new(
                source,
                index,
                tex_state::ShipoutTokenField::PdfThreadAttributes,
            ),
            running: thread.running,
        },
        Whatsit::PdfDestination(destination) => PreparedWhatsit::PdfDestination {
            identifier: identifier(
                &destination.identifier,
                tex_state::ShipoutTokenField::PdfDestinationIdentifier,
            ),
            structure: destination.structure,
            kind: destination.kind,
        },
        Whatsit::PdfColorStack { id, .. } => PreparedWhatsit::PdfColorStack {
            id: *id,
            source: tex_state::ShipoutNodeSource::new(source, index),
        },
        _ => {
            // Every remaining owned field (open path, immediate special/PDF
            // bytes, and navigation-free scalar metadata) is moved from this
            // one clone into its final detached effect. Token-bearing and
            // mutable-state payloads above retain typed source handles.
            let node: Node<PageListId, Glue, Tokens> = Node::Whatsit(whatsit.clone());
            let node = node.map_payloads(resolve_glue, |_| {
                unreachable!("token-owning whatsits use a typed source handle")
            });
            let Node::Whatsit(whatsit) = node else {
                unreachable!()
            };
            PreparedWhatsit::Other(whatsit)
        }
    }
}

fn classify_transient_node<G, List: Copy, Glue, Tokens>(
    node: &Node<List, Glue, Tokens>,
    map: impl Fn(List) -> tex_state::ShipoutListId<G>,
    suppress_deferred_streams: bool,
    in_hlist: bool,
) -> NormalizeNode<G> {
    match node {
        Node::HList(box_node) => NormalizeNode::List(
            map(box_node.children),
            suppress_deferred_streams,
            true,
            box_node.box_lr,
        ),
        Node::VList(box_node) => NormalizeNode::List(
            map(box_node.children),
            suppress_deferred_streams,
            false,
            box_node.box_lr,
        ),
        Node::Glue {
            leader: Some(StateLeaderPayload::HList(box_node)),
            ..
        } => NormalizeNode::List(map(box_node.children), true, true, box_node.box_lr),
        Node::Glue {
            leader: Some(StateLeaderPayload::VList(box_node)),
            ..
        } => NormalizeNode::List(map(box_node.children), true, false, box_node.box_lr),
        Node::Disc {
            pre, post, replace, ..
        } => NormalizeNode::Lists([map(*pre), map(*post), map(*replace)]),
        Node::Ins { content, .. } => NormalizeNode::List(
            map(*content),
            suppress_deferred_streams,
            in_hlist,
            tex_state::node::BoxLr::Normal,
        ),
        Node::Adjust(adjust) => NormalizeNode::List(
            map(adjust.content),
            suppress_deferred_streams,
            in_hlist,
            tex_state::node::BoxLr::Normal,
        ),
        Node::Whatsit(_) => unreachable!("whatsits retain their typed source handle"),
        Node::MathList(_) => NormalizeNode::Unsupported("math"),
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
}

fn classify_durable_node<G>(
    node: &Node,
    source: tex_state::ShipoutListId<G>,
    index: usize,
    suppress_deferred_streams: bool,
    in_hlist: bool,
) -> NormalizeNode<G> {
    if let Node::Whatsit(whatsit) = node {
        return NormalizeNode::Whatsit(prepare_whatsit(whatsit, source, index, |glue| glue));
    }
    classify_transient_node(
        node,
        tex_state::ShipoutListId::durable_child,
        suppress_deferred_streams,
        in_hlist,
    )
}

#[cfg(test)]
fn prepared_whatsit_visit_kind<G>(whatsit: &PreparedWhatsit<G>) -> Option<BaseWhatsitVisitKind> {
    match whatsit {
        PreparedWhatsit::DeferredWrite { .. } => Some(BaseWhatsitVisitKind::DeferredWrite),
        PreparedWhatsit::Other(whatsit) => base_whatsit_visit_kind(whatsit),
        PreparedWhatsit::DeferredSpecial { .. }
        | PreparedWhatsit::DeferredPdfLiteral { .. }
        | PreparedWhatsit::PdfThread { .. }
        | PreparedWhatsit::PdfDestination { .. }
        | PreparedWhatsit::PdfColorStack { .. } => None,
    }
}

#[cfg(test)]
fn base_whatsit_visit_kind(whatsit: &Whatsit) -> Option<BaseWhatsitVisitKind> {
    match whatsit {
        Whatsit::OpenOut { .. } => Some(BaseWhatsitVisitKind::OpenOut),
        Whatsit::CloseOut { slot: Some(_) } => Some(BaseWhatsitVisitKind::NumberedCloseOut),
        Whatsit::CloseOut { slot: None } => Some(BaseWhatsitVisitKind::FallbackCloseOut),
        Whatsit::Special { .. } => Some(BaseWhatsitVisitKind::Special),
        Whatsit::Language { .. } => Some(BaseWhatsitVisitKind::Language),
        _ => None,
    }
}

fn prepared_whatsit_is_anchored<G>(
    whatsit: &PreparedWhatsit<G>,
    suppress_deferred_streams: bool,
) -> bool {
    match whatsit {
        PreparedWhatsit::DeferredWrite { .. } => !suppress_deferred_streams,
        PreparedWhatsit::DeferredSpecial { .. }
        | PreparedWhatsit::DeferredPdfLiteral { .. }
        | PreparedWhatsit::PdfThread { .. }
        | PreparedWhatsit::PdfDestination { .. }
        | PreparedWhatsit::PdfColorStack { .. } => true,
        PreparedWhatsit::Other(whatsit) => whatsit_is_anchored(whatsit, suppress_deferred_streams),
    }
}

#[allow(clippy::too_many_arguments)]
fn append_prepared_pdf_thread<G>(
    stores: &mut Universe<G>,
    overlay: &mut PageOverlay<G>,
    identifier: PreparedPdfIdentifier<G>,
    dimensions: tex_state::PdfAnnotationDimensions,
    attributes: tex_state::ShipoutTokenSource<G>,
    running: bool,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
) -> Result<(), ExecError> {
    if suppress_deferred_streams {
        return Ok(());
    }
    if running && location.in_hlist {
        return Err(ExecError::PdfNavigation(
            "pdfTeX error (ext4): \\pdfstartthread ended up in hlist",
        ));
    }
    if overlay.color_target == tex_state::PdfColorStackTarget::Form {
        return Err(ExecError::PdfThreadInForm);
    }
    let identity = render_prepared_pdf_identity(stores, identifier);
    let (thread, bead) = stores
        .command_context()
        .expect("PDF thread execution runs inside an admitted command episode")
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
    let context = stores
        .command_context()
        .expect("PDF thread token traversal runs inside an admitted command episode");
    context
        .visit_shipout_tokens(attributes, |token| {
            tex_state::token_show::append_token_string_text(
                &context,
                token.semantic_token(),
                &mut attribute_bytes,
            );
            Ok::<(), core::convert::Infallible>(())
        })
        .expect("infallible PDF thread token rendering");
    let margin = context.dimen_param(DimenParam::PDF_THREAD_MARGIN);
    drop(context);
    let marker = tex_out::PdfThreadEffect {
        thread_object: thread.object(),
        bead_object: bead.bead_object(),
        rectangle_object: bead.rectangle_object(),
        identifier,
        width: dimensions.width,
        height: dimensions.height,
        depth: dimensions.depth,
        attributes: attribute_bytes.into_bytes(),
        margin,
    };
    if running {
        overlay.running_thread_depth = Some(location.depth);
    }
    overlay.effects.push(if running {
        PageEffect::PdfStartThread(marker)
    } else {
        PageEffect::PdfThread(marker)
    });
    Ok(())
}

fn render_prepared_pdf_identity<G>(
    stores: &mut Universe<G>,
    identifier: PreparedPdfIdentifier<G>,
) -> tex_state::PdfDestinationIdentity {
    match identifier {
        PreparedPdfIdentifier::Number(number) => tex_state::PdfDestinationIdentity::Number(number),
        PreparedPdfIdentifier::Tokens(tokens) => {
            let mut text = String::new();
            let context = stores
                .command_context()
                .expect("PDF identifier traversal runs inside an admitted command episode");
            context
                .visit_shipout_tokens(tokens, |token| {
                    tex_state::token_show::append_token_string_text(
                        &context,
                        token.semantic_token(),
                        &mut text,
                    );
                    Ok::<(), core::convert::Infallible>(())
                })
                .expect("infallible PDF identifier rendering");
            tex_state::PdfDestinationIdentity::Name(text.into_bytes())
        }
    }
}

fn append_prepared_pdf_destination<G>(
    stores: &mut Universe<G>,
    overlay: &mut PageOverlay<G>,
    identifier: PreparedPdfIdentifier<G>,
    structure: Option<u32>,
    kind: tex_state::node::PdfDestinationKind,
    suppress_deferred_streams: bool,
) -> Result<(), ExecError> {
    if suppress_deferred_streams {
        return Ok(());
    }
    if overlay.color_target == tex_state::PdfColorStackTarget::Form {
        return Err(ExecError::PdfDestinationInForm);
    }
    let identity = render_prepared_pdf_identity(stores, identifier);
    let definition = stores
        .command_context()
        .expect("PDF destination execution runs inside an admitted command episode")
        .define_pdf_destination(identity.clone(), structure)
        .map_err(|_| ExecError::PdfObjectCapacity)?;
    if definition.duplicate {
        if stores.int_param(IntParam::PDF_SUPPRESS_WARNING_DUP_DEST) <= 0 {
            overlay.diagnostics.push((
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
    overlay
        .effects
        .push(PageEffect::PdfDestination(tex_out::PdfDestinationEffect {
            object: definition.record.object(),
            identifier,
            structure,
            kind,
            margin: stores
                .dimen_param(DimenParam::PDF_DEST_MARGIN)
                .expect("shipout reads admitted pdfdestmargin"),
        }));
    Ok(())
}

fn append_whatsit_effect<G>(
    stores: &mut Universe<G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    expansion: &mut NormalizeExpansion<'_, G>,
    overlay: &mut PageOverlay<G>,
    whatsit: PreparedWhatsit<G>,
    suppress_deferred_streams: bool,
    location: NormalizeLocation,
) -> Result<(), ExecError> {
    let whatsit = match whatsit {
        PreparedWhatsit::PdfThread {
            identifier,
            dimensions,
            attributes,
            running,
        } => {
            return append_prepared_pdf_thread(
                stores,
                overlay,
                identifier,
                dimensions,
                attributes,
                running,
                suppress_deferred_streams,
                location,
            );
        }
        PreparedWhatsit::PdfDestination {
            identifier,
            structure,
            kind,
        } => {
            return append_prepared_pdf_destination(
                stores,
                overlay,
                identifier,
                structure,
                kind,
                suppress_deferred_streams,
            );
        }
        PreparedWhatsit::PdfColorStack { id, source } => {
            let emission = stores
                .command_context()
                .expect("PDF color execution runs inside an admitted command episode")
                .apply_shipout_pdf_color_stack(source, id, overlay.color_target);
            match emission {
                Ok(emission) => overlay.effects.push(PageEffect::PdfColorStack {
                    mode: lower_color_stack_mode(emission.mode),
                    payload: emission.payload,
                    page_start: false,
                }),
                Err(tex_state::PdfColorStackApplyError::Underflow) => {
                    let target = match overlay.color_target {
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
            return Ok(());
        }
        other => other,
    };
    let (whatsit, token_source) = match whatsit {
        PreparedWhatsit::DeferredWrite { sink, tokens } => (
            Whatsit::DeferredWrite {
                sink,
                tokens: tex_state::node::NodeTokenList::default(),
            },
            Some(tokens),
        ),
        PreparedWhatsit::DeferredSpecial { class, tokens } => (
            Whatsit::DeferredSpecial {
                class,
                tokens: tex_state::node::NodeTokenList::default(),
            },
            Some(tokens),
        ),
        PreparedWhatsit::DeferredPdfLiteral { mode, tokens } => (
            Whatsit::DeferredPdfLiteral {
                mode,
                tokens: tex_state::node::NodeTokenList::default(),
            },
            Some(tokens),
        ),
        PreparedWhatsit::PdfThread { .. }
        | PreparedWhatsit::PdfDestination { .. }
        | PreparedWhatsit::PdfColorStack { .. } => {
            unreachable!("typed shipout source handled before owned lowering")
        }
        PreparedWhatsit::Other(whatsit) => (whatsit, None),
    };
    let NormalizeLocation { in_hlist, depth } = location;
    let announce_openout = overlay.announce_openout;
    let output_open_context = overlay.output_open_context.clone();
    let effects = &mut overlay.effects;
    let open_out_occurrences = &mut overlay.open_out_occurrences;
    let running_thread_depth = &mut overlay.running_thread_depth;
    match whatsit {
        Whatsit::OpenOut { slot, path } if !suppress_deferred_streams => {
            // TeX82 §1374 closes the old stream before it attempts the
            // replacement, even when every subsequent open attempt fails.
            stores
                .command_context()
                .expect("deferred stream execution runs inside an admitted command episode")
                .close_output_stream(slot);
            let path = retry_openout_target(stores, path, &output_open_context)?;
            stores
                .command_context()
                .expect("deferred stream execution runs inside an admitted command episode")
                .open_output_stream(slot, path.clone().into());
            stores
                .command_context()
                .expect("deferred stream execution runs inside an admitted command episode")
                .set_last_stream_open_context(output_open_context);
            let effect_ordinal = stores
                .world()
                .page_effect_prefix_len()
                .checked_add(stores.world().effect_records().len())
                .and_then(|ordinal| u32::try_from(ordinal).ok())
                .ok_or_else(|| {
                    ExecError::InvalidShipoutArtifact(
                        "detached artifact effect ordinal overflow".to_owned(),
                    )
                })?;
            open_out_occurrences.push((effects.len(), ArtifactEffectOrdinal::new(effect_ordinal)));
            // web2c's `[53.1374]` log notice, which follows `write_open[j]:=
            // true`. It has to come after the context attach above, which
            // requires the `StreamOpen` record to still be the last effect.
            if announce_openout {
                let tracing_online = stores
                    .command_context()
                    .expect("live generation")
                    .int_param(tex_state::env::banks::IntParam::TRACING_ONLINE);
                let (terminal_line_is_open, log_line_is_open) =
                    stores.world().printable_lines_are_open();
                overlay.diagnostics.push(crate::diagnostics::report_openout(
                    tracing_online,
                    terminal_line_is_open,
                    log_line_is_open,
                    slot.raw(),
                    &path,
                ));
            }
            effects.push(PageEffect::OpenOut {
                stream: slot.raw(),
                path,
            });
        }
        Whatsit::CloseOut { slot } if !suppress_deferred_streams => {
            if let Some(slot) = slot {
                stores
                    .command_context()
                    .expect("deferred stream execution runs inside an admitted command episode")
                    .close_output_stream(slot);
                effects.push(PageEffect::CloseOut { stream: slot.raw() });
            }
        }
        Whatsit::DeferredWrite { sink, tokens } if !suppress_deferred_streams => {
            let expanded = (expansion.write_expander)(
                stores,
                diagnostic_effects,
                sink,
                token_source.expect("deferred write retains its typed token source"),
            )?;
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
        Whatsit::DeferredSpecial { class, tokens: _ } => {
            let crate::shipout::ExpandedReplayText(payload) = (expansion.replay_expander)(
                stores,
                diagnostic_effects,
                super::ReplayTextKind::Special,
                token_source.expect("deferred special retains its typed token source"),
            )?;
            effects.push(PageEffect::Special { class, payload });
        }
        Whatsit::PdfReferenceObject { object } => {
            stores
                .command_context()
                .expect("PDF object publication runs inside an admitted command episode")
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
        Whatsit::DeferredPdfLiteral { mode, tokens: _ } => {
            let crate::shipout::ExpandedReplayText(payload) = (expansion.replay_expander)(
                stores,
                diagnostic_effects,
                super::ReplayTextKind::PdfLiteral,
                token_source.expect("deferred literal retains its typed token source"),
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
        Whatsit::PdfColorStack { .. } => {
            unreachable!("color-stack whatsits retain their typed source handle")
        }
        Whatsit::PdfSavePos => effects.push(PageEffect::PdfSavePosition),
        Whatsit::PdfSnapRefPoint => effects.push(PageEffect::PdfSnapRefPoint),
        Whatsit::PdfSnapY { glue } => effects.push(PageEffect::PdfSnapY {
            spec: super::lower_glue(glue),
        }),
        Whatsit::PdfSnapYComp { ratio } => effects.push(PageEffect::PdfSnapYComp { ratio }),
        Whatsit::PdfRefXForm {
            object,
            width,
            height,
            depth,
        } => {
            let artifact = stores
                .command_context()
                .expect("PDF form execution runs inside an admitted command episode")
                .pdf_form_artifact(object);
            if artifact.is_none() {
                let form = stores
                    .command_context()
                    .expect("PDF form execution runs inside an admitted command episode")
                    .pdf_form(object)
                    .ok_or(ExecError::PdfReferencedObjectNotFound)?;
                let artifact = super::stage_form(
                    form,
                    stores,
                    diagnostic_effects,
                    expansion.write_expander,
                    expansion.replay_expander,
                )?;
                let mut command = stores
                    .command_context()
                    .expect("PDF form execution runs inside an admitted command episode");
                command.publish_pdf_traversal_positions(
                    artifact.last_position(),
                    artifact.snap_reference(),
                );
                command.set_pdf_form_artifact(object, artifact);
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
        Whatsit::PdfDestination(_) | Whatsit::PdfThread(_) => {
            unreachable!("navigation whatsits retain typed source handles")
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
pub(super) fn deferred_write_sink<G>(
    stores: &mut Universe<G>,
    sink: tex_state::PrintSink,
) -> Option<tex_state::PrintSink> {
    let selector = tex_state::print::Selector::for_interaction(stores.interaction_mode());
    let mut stream_is_open = |slot| {
        stores
            .command_context()
            .expect("deferred write runs inside an admitted command episode")
            .output_stream_is_open(slot)
    };
    match sink {
        tex_state::PrintSink::Stream(slot) if stream_is_open(slot) => Some(sink),
        tex_state::PrintSink::Stream(_) | tex_state::PrintSink::TerminalAndLog => selector.sink(),
        tex_state::PrintSink::Log if selector == tex_state::print::Selector::TermAndLog => {
            Some(tex_state::PrintSink::Log)
        }
        tex_state::PrintSink::Log => selector.sink(),
        tex_state::PrintSink::Terminal => Some(tex_state::PrintSink::Terminal),
    }
}

/// TeX82 §§1373--1374's `out_what` open loop.
fn retry_openout_target<G>(
    stores: &mut Universe<G>,
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
            .expect("output retry runs inside an admitted command episode")
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

pub(super) fn direction_permutation_for_box<List: Copy, Glue: Copy, Tokens>(
    nodes: &[Node<List, Glue, Tokens>],
    box_lr: tex_state::node::BoxLr,
) -> Option<Vec<usize>> {
    if box_lr == tex_state::node::BoxLr::Reversed {
        return None;
    }
    direction_permutation(nodes)
}

fn direction_permutation<List: Copy, Glue: Copy, Tokens>(
    nodes: &[Node<List, Glue, Tokens>],
) -> Option<Vec<usize>> {
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

    if !nodes.iter().any(|node| matches!(node, Node::Direction(_))) {
        return None;
    }
    let mut reordered = Vec::with_capacity(nodes.len());
    let mut stack = Vec::<Segment>::new();
    for (index, node) in nodes.iter().map(NodeRef::from).enumerate() {
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
pub(super) fn write_line_is_open<G>(stores: &Universe<G>, sink: tex_state::PrintSink) -> bool {
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

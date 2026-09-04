use smallvec::SmallVec;

#[derive(Clone, Debug)]
pub(super) struct MissingHyphenDiagnostic {
    pub(super) node_index: usize,
    pub(super) font: tex_state::ids::FontId,
    pub(super) ch: char,
}

struct HyphenationProjection<'a> {
    physical_post_overrides: &'a mut Vec<(usize, tex_state::node_arena::PageListId)>,
    missing_hyphens: &'a mut Vec<MissingHyphenDiagnostic>,
}

pub(super) struct HyphenatedHlist {
    pub(super) semantic: tex_state::node_arena::PageListId,
    pub(super) physical: tex_state::node_arena::PageListId,
    pub(super) physical_boundaries: Vec<usize>,
    pub(super) missing_hyphens: Vec<MissingHyphenDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HyphenationContext {
    language: u8,
    left: usize,
    right: usize,
}

#[derive(Clone, Copy)]
enum HyphenationScanNode {
    Language {
        language: u8,
        left: usize,
        right: usize,
    },
    MathOn,
    MathOff,
    Glue,
    Other,
}

impl HyphenationScanNode {
    fn from_node(node: tex_state::page_node_arena::PageMaterialNodeRef<'_>) -> Self {
        if let Some((language, left_hyphen_min, right_hyphen_min)) = node.language() {
            Self::Language {
                language,
                left: usize::from(left_hyphen_min.max(1)),
                right: usize::from(right_hyphen_min.max(1)),
            }
        } else if node.is_math_on() {
            Self::MathOn
        } else if node.is_math_off() {
            Self::MathOff
        } else if node.is_glue() {
            Self::Glue
        } else {
            Self::Other
        }
    }
}

struct HyphenationWalk<'walk, 'projection> {
    out: &'walk mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &'walk mut Vec<tex_state::node_arena::PageListId>,
    tfm_work: &'walk mut crate::box_runtime::hmode::LigatureWorkList,
    fuel: &'walk mut tex_command::CommandFuel,
    projection: &'walk mut HyphenationProjection<'projection>,
    output_len: usize,
    retained_start: usize,
    skip_until: usize,
    auto_breaking: bool,
    language: u8,
    left: usize,
    right: usize,
}

impl HyphenationWalk<'_, '_> {
    #[inline(never)]
    fn visit_chunk_prefix<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::AdmittedPageList,
        chunk: tex_state::page_node_arena::PageListChunkCursor,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Result<(), ExecError> {
        if let Some(previous) = stores
            .page_node_span_previous_chunk(&chunk)
            .expect("hyphenation source chunk remains live")
        {
            self.visit_chunk_prefix(stores, source, previous, diagnostic_effects)?;
        }
        self.visit_chunk(stores, source, chunk, diagnostic_effects)
    }

    #[inline(never)]
    fn visit_chunk<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::AdmittedPageList,
        mut chunk: tex_state::page_node_arena::PageListChunkCursor,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Result<(), ExecError> {
        while let Some((index, node)) = stores.page_node_span_next_chunk_node(&mut chunk) {
            if index < self.skip_until {
                continue;
            }
            let observed = { HyphenationScanNode::from_node(node) };
            match observed {
                HyphenationScanNode::Language {
                    language,
                    left,
                    right,
                } => {
                    self.language = language;
                    self.left = left;
                    self.right = right;
                }
                HyphenationScanNode::MathOn => self.auto_breaking = false,
                HyphenationScanNode::MathOff => self.auto_breaking = true,
                HyphenationScanNode::Glue if self.auto_breaking => {
                    let start = index + 1;
                    stores.append_page_active_span_range(
                        self.out,
                        source.span(),
                        self.retained_start..start,
                    );
                    self.output_len += start - self.retained_start;
                    self.retained_start = start;
                    if let Some(candidate) = find_hyphenation_candidate(
                        stores,
                        source,
                        start,
                        (self.language, self.left, self.right),
                    ) {
                        let next = hyphenate_candidate_after_glue(
                            stores,
                            diagnostic_effects,
                            source,
                            start,
                            candidate,
                            self.out,
                            self.out_segments,
                            self.tfm_work,
                            &mut self.output_len,
                            self.fuel,
                            self.projection,
                        )?;
                        self.skip_until = next;
                        self.retained_start = next;
                    }
                }
                HyphenationScanNode::Glue | HyphenationScanNode::Other => {}
            }
        }
        Ok(())
    }
}

/// Installs patterns whose §963 duplicate diagnostics were already reported
/// by the canonical live scanner.
pub(crate) fn apply_scanned_patterns<G>(
    stores: &mut CommandContext<'_, G>,
    patterns: Vec<PatternSpec>,
) -> Result<(), ExecError> {
    let language = current_language(stores);
    for pattern in patterns {
        stores
            .add_hyphenation_pattern_for_language(language, pattern)
            .map_err(pattern_capacity_error)?;
    }
    if stores.int_param(IntParam::SAVING_HYPH_CODES) > 0 {
        let codes = (0u8..=u8::MAX).filter_map(|code| {
            let ch = char::from(code);
            char::from_u32(stores.lccode(ch))
                .filter(|&mapped| mapped != '\0')
                .map(|mapped| (ch, mapped))
        });
        stores.save_hyphenation_codes(language, codes.collect::<Vec<_>>());
    }
    Ok(())
}

pub(crate) fn pattern_capacity_error(
    error: tex_state::hyphenation::HyphenationCapacityError,
) -> ExecError {
    ExecError::Fatal(tex_command::FatalError::overflow(
        "pattern memory",
        i32::try_from(error.capacity).unwrap_or(i32::MAX),
    ))
}

/// Installs exception words the canonical live scanner already normalized, so
/// §935's `Not a letter` was reported where §82 could still show the offending
/// character (`tex_command::ScannedHyphenationData`).
pub(crate) fn apply_scanned_hyphenation_exceptions<G>(
    stores: &mut CommandContext<'_, G>,
    words: Vec<Vec<char>>,
) {
    let language = current_language(stores);
    for word in words {
        let (exception, not_letters) = parse_exception_word(stores, language, &word, false);
        debug_assert_eq!(not_letters, 0);
        if let Some(exception) = exception {
            stores.add_hyphenation_exception_for_language(language, exception);
        }
    }
}

fn hyphenated_hlist_with_projections<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: tex_state::node_arena::PageListId,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<(tex_state::node_arena::PageListId, HyphenationContext), ExecError> {
    // TeX82 §919 initializes the trie on entry to the first hyphenation pass,
    // even when this particular paragraph ultimately supplies no candidate.
    stores.close_hyphenation_patterns();
    let source = stores
        .admit_page_node_list(source)
        .expect("hyphenation source crosses one live page-region boundary");
    let operation = stores.page_node_cursor();
    let mut out = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut out);
    let mut out_segments = Vec::new();
    let left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize;
    let right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize;
    let (retained_start, language, left, right) = {
        let mut walk = HyphenationWalk {
            out: &mut out,
            out_segments: &mut out_segments,
            tfm_work,
            fuel,
            projection,
            output_len: 0,
            retained_start: 0,
            skip_until: 0,
            auto_breaking: true,
            language: 0,
            left,
            right,
        };
        if let Some(tail) = stores
            .admitted_page_tail_chunk(source)
            .expect("hyphenation source remains admitted")
            && let Err(error) = walk.visit_chunk_prefix(stores, source, tail, diagnostic_effects)
        {
            if out.is_open() {
                stores.rollback_page_active_list(&mut out);
            }
            stores
                .truncate_page_nodes(operation)
                .expect("hyphenation rollback restores its page suffix");
            return Err(error);
        }
        (walk.retained_start, walk.language, walk.left, walk.right)
    };
    if retained_start < source.len() {
        stores.append_page_active_span_range(&mut out, source.span(), retained_start..source.len());
    }
    let tail = stores.finalize_page_active_list(&mut out);
    if !tail.is_empty() {
        out_segments.push(tail);
    }
    let output = match out_segments.as_slice() {
        [] => tex_state::node_arena::PageListId::empty(),
        [only] => *only,
        segments => stores.compose_page_node_sequences(segments),
    };
    Ok((
        output,
        HyphenationContext {
            language,
            left,
            right,
        },
    ))
}

/// Builds the semantic hyphenated list together with TeX82's physical
/// diagnostic projection. A boundary discretionary keeps the compact
/// semantic pre-break list used by line breaking, while TeX's linked-list
/// representation exposes the preceding reconstituted character span in the
/// discretionary's diagnostic pre-break branch.
pub(crate) fn hyphenated_hlist_with_fuel<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: tex_state::node_arena::PageListId,
    scratch: &mut crate::mode::HorizontalModeScratch,
    fuel: &mut tex_command::CommandFuel,
) -> Result<HyphenatedHlist, ExecError> {
    let operation = stores.page_node_cursor();
    let mut physical_post_overrides = Vec::new();
    let mut missing_hyphens = Vec::new();
    let mut projection = HyphenationProjection {
        physical_post_overrides: &mut physical_post_overrides,
        missing_hyphens: &mut missing_hyphens,
    };
    let (semantic, _) = match hyphenated_hlist_with_projections(
        stores,
        diagnostic_effects,
        source,
        scratch.tfm_work_mut(),
        fuel,
        &mut projection,
    ) {
        Ok(result) => result,
        Err(error) => {
            stores
                .truncate_page_nodes(operation)
                .expect("hyphenation semantic rollback restores its page suffix");
            return Err(error);
        }
    };
    let physical = match project_physical_hlist(
        stores,
        diagnostic_effects,
        semantic,
        &physical_post_overrides,
        scratch.tfm_work_mut(),
        fuel,
    ) {
        Ok(physical) => physical,
        Err(error) => {
            stores
                .truncate_page_nodes(operation)
                .expect("hyphenation physical rollback restores its page suffix");
            return Err(error);
        }
    };
    let semantic = scratch.reshape_open_type_runs_list(stores, semantic);
    let physical_boundaries = compacted_physical_boundaries(stores, semantic, physical.len());
    Ok(HyphenatedHlist {
        semantic,
        physical,
        physical_boundaries,
        missing_hyphens,
    })
}

fn project_physical_hlist<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    semantic: tex_state::node_arena::PageListId,
    post_overrides: &[(usize, tex_state::node_arena::PageListId)],
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    fuel: &mut tex_command::CommandFuel,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let operation = stores.page_node_cursor();
    let semantic = stores
        .admit_page_node_list(semantic)
        .expect("hyphenated paragraph crosses one live page-region boundary");
    let mut physical = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut physical);
    let mut physical_segments = Vec::new();
    let mut override_index = 0usize;
    let mut retained_start = 0usize;
    let result = if let Some(tail) = stores
        .admitted_page_tail_chunk(semantic)
        .expect("hyphenated paragraph remains admitted")
    {
        project_physical_chunk_prefix(
            stores,
            diagnostic_effects,
            semantic,
            tail,
            post_overrides,
            &mut override_index,
            &mut physical,
            &mut physical_segments,
            &mut retained_start,
            tfm_work,
            fuel,
        )
        .map(|_| ())
    } else {
        Ok(())
    };
    if let Err(error) = result {
        if physical.is_open() {
            stores.rollback_page_active_list(&mut physical);
        }
        stores
            .truncate_page_nodes(operation)
            .expect("physical projection rollback restores its page suffix");
        return Err(error);
    }
    if retained_start < semantic.len() {
        stores.append_page_active_span_range(
            &mut physical,
            semantic.span(),
            retained_start..semantic.len(),
        );
    }
    let tail = stores.finalize_page_active_list(&mut physical);
    if !tail.is_empty() {
        physical_segments.push(tail);
    }
    Ok(match physical_segments.as_slice() {
        [] => tex_state::node_arena::PageListId::empty(),
        [only] => *only,
        segments => stores.compose_page_node_sequences(segments),
    })
}

#[allow(clippy::too_many_arguments)]
fn project_physical_chunk_prefix<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    semantic: tex_state::page_node_arena::AdmittedPageList,
    chunk: tex_state::page_node_arena::PageListChunkCursor,
    post_overrides: &[(usize, tex_state::node_arena::PageListId)],
    override_index: &mut usize,
    physical: &mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    physical_segments: &mut Vec<tex_state::node_arena::PageListId>,
    retained_start: &mut usize,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    fuel: &mut tex_command::CommandFuel,
) -> Result<tex_state::page_node_arena::PageListChunkCursor, ExecError> {
    let previous = stores
        .page_node_span_previous_chunk(&chunk)
        .expect("hyphenated paragraph source chunk remains live");
    let previous = if let Some(previous) = previous {
        Some(project_physical_chunk_prefix(
            stores,
            diagnostic_effects,
            semantic,
            previous,
            post_overrides,
            override_index,
            physical,
            physical_segments,
            retained_start,
            tfm_work,
            fuel,
        )?)
    } else {
        None
    };
    for offset in 0..chunk.len() {
        let index = chunk.logical_start() + offset;
        let post_override = (post_overrides.get(*override_index).map(|entry| entry.0)
            == Some(index))
        .then(|| post_overrides[*override_index].1);
        let pre_pending = physical_pre_break_pending(stores, previous.as_ref(), &chunk, offset);
        if (post_override.is_some() || pre_pending.is_some()) && *retained_start < index {
            stores.append_page_active_span_range(physical, semantic.span(), *retained_start..index);
        }
        let pre_projection = if let Some(pending) = pre_pending {
            // TeX82 §§914--918 builds a discretionary's child closures before
            // linking the discretionary into the main list. Close this
            // output suffix while the child list is published so the page
            // arena likewise has one active construction owner at a time.
            let segment = stores.finalize_page_active_list(physical);
            if !segment.is_empty() {
                physical_segments.push(segment);
            }
            let pre = reconstitute_branch(
                stores,
                diagnostic_effects,
                &pending,
                true,
                crate::box_runtime::hmode::LigatureRightBoundary::Font,
                fuel,
                tfm_work,
            )?;
            stores.open_page_active_list(physical);
            Some(pre)
        } else {
            None
        };
        if post_override.is_some() || pre_projection.is_some() {
            let (kind, mut pre, mut post, replace, physical_replace_count) = {
                let (resolved, node) = stores.page_node_span_chunk_node(&chunk, offset);
                debug_assert_eq!(resolved, index);
                node.discretionary()
                    .expect("physical projection targets a discretionary")
            };
            if let Some(projected) = post_override {
                post = projected;
                *override_index += 1;
            }
            if let Some(projected) = pre_projection {
                pre = projected;
            }
            stores.construct_page_active_list(physical, |destination| {
                destination.discretionary(kind, pre, post, replace, physical_replace_count);
            });
            *retained_start = index + 1;
        }
    }
    Ok(chunk)
}

fn physical_pre_break_pending<G>(
    stores: &CommandContext<'_, G>,
    previous_chunk: Option<&tex_state::page_node_arena::PageListChunkCursor>,
    chunk: &tex_state::page_node_arena::PageListChunkCursor,
    offset: usize,
) -> Option<SmallVec<[PendingHChar; 64]>> {
    let index = chunk.logical_start() + offset;
    if index == 0 {
        return None;
    }
    let qualifies = {
        let (resolved, node) = stores.page_node_span_chunk_node(chunk, offset);
        debug_assert_eq!(resolved, index);
        let Some((DiscKind::AutomaticHyphen, _, _, replace, 2)) = node.discretionary() else {
            return None;
        };
        let replacement = stores
            .page_node_list(replace)
            .expect("discretionary replacement belongs to the live page arena");
        replacement.len() == 1
            && matches!(
                replacement.first(),
                Some(tex_state::node_arena::NodeView::Kern {
                    kind: KernKind::Font,
                    ..
                })
            )
    };
    if !qualifies {
        return None;
    }
    let (font, mut pending) = {
        let (previous, previous_offset) = if offset == 0 {
            let previous = previous_chunk?;
            (previous, previous.len().checked_sub(1)?)
        } else {
            (chunk, offset - 1)
        };
        let (resolved, node) = stores.page_node_span_chunk_node(previous, previous_offset);
        debug_assert_eq!(resolved, index - 1);
        if let Some((font, ch, origin)) = node.character() {
            {
                let mut pending: SmallVec<[PendingHChar; 64]> = SmallVec::new();
                pending.push(PendingHChar { font, ch, origin });
                (font, pending)
            }
        } else {
            let mut pending: SmallVec<[PendingHChar; 64]> = SmallVec::new();
            let font = node.visit_ligature_source(|ch, origin| {
                pending.push(PendingHChar {
                    font: tex_state::font::NULL_FONT,
                    ch,
                    origin,
                });
            })?;
            pending.iter_mut().for_each(|entry| entry.font = font);
            (font, pending)
        }
    };
    let hyphen = usable_hyphen_char(stores, font)?;
    let last_origin = pending
        .last()
        .map_or(OriginId::UNKNOWN, |entry| entry.origin);
    pending.push(PendingHChar {
        font,
        ch: hyphen,
        origin: last_origin,
    });
    Some(pending)
}

fn compacted_physical_boundaries<G>(
    stores: &CommandContext<'_, G>,
    semantic: tex_state::node_arena::PageListId,
    physical_len: usize,
) -> Vec<usize> {
    let semantic = stores
        .admit_page_node_list(semantic)
        .expect("shaped paragraph crosses one live page-region boundary");
    let nodes = stores
        .admitted_page_nodes(semantic)
        .expect("shaped paragraph belongs to the live page arena");
    let mut boundary = 0usize;
    let mut boundaries = Vec::with_capacity(nodes.len() + 1);
    boundaries.push(0);
    nodes.nodes().for_each(|node| {
        boundary = boundary.saturating_add(match node {
            tex_state::NodeView::Lig { orig, .. } => orig.len().max(1),
            _ => 1,
        });
        boundaries.push(boundary.min(physical_len));
    });
    if let Some(last) = boundaries.last_mut() {
        *last = physical_len;
    }
    boundaries
}

fn update_hyphenation_context(
    node: tex_state::NodeView<'_>,
    language: &mut u8,
    left: &mut usize,
    right: &mut usize,
) {
    if let tex_state::NodeView::Whatsit(tex_state::node::Whatsit::Language {
        language: new_language,
        left_hyphen_min,
        right_hyphen_min,
    }) = node
    {
        *language = new_language;
        *left = usize::from(left_hyphen_min.max(1));
        *right = usize::from(right_hyphen_min.max(1));
    }
}

#[allow(clippy::too_many_arguments)] // Hyphenation traversal keeps cursor, fuel, and projection state independent.
fn hyphenate_candidate_after_glue<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: tex_state::page_node_arena::AdmittedPageList,
    start: usize,
    candidate: HyphenationCandidate,
    out: &mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &mut Vec<tex_state::node_arena::PageListId>,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    output_len: &mut usize,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<usize, ExecError> {
    let HyphenationCandidate {
        word_start,
        end: index,
        right_boundary,
        language,
        left,
        right,
        word,
    } = candidate;

    let lowercase: String = word.iter().map(|ch| ch.lower).collect();
    let positions = stores.hyphen_positions_for_language(language, &lowercase, left, right);
    if positions.is_empty() {
        stores.append_page_active_span_range(out, source.span(), start..index);
        *output_len += index - start;
        return Ok(index);
    }

    stores.append_page_active_span_range(out, source.span(), start..word_start);
    *output_len += word_start - start;
    let trailing_font_kern = stores
        .admitted_page_nodes(source)
        .expect("hyphenation source belongs to the live page arena")
        .get(index - 1)
        .is_some_and(|node| {
            matches!(
                node,
                tex_state::node_arena::NodeView::Kern {
                    kind: KernKind::Font,
                    ..
                }
            )
        });
    let no_left_boundary = word_start != 0
        && stores
            .admitted_page_nodes(source)
            .expect("hyphenation source belongs to the live page arena")
            .get(word_start - 1)
            .is_some_and(|node| {
                matches!(
                    node,
                    tex_state::node_arena::NodeView::Kern {
                        kind: KernKind::Font,
                        ..
                    }
                )
            });
    // TeX82 §§914--918 constructs each discretionary's three child lists
    // before it links the discretionary into the reconstituted main list.
    // Seal the retained source segment before that nested construction, then
    // resume a fresh main-list segment for the generated word.
    let segment = stores.finalize_page_active_list(out);
    if !segment.is_empty() {
        out_segments.push(segment);
    }
    stores.open_page_active_list(out);
    append_hyphenated_word(
        stores,
        diagnostic_effects,
        &word,
        &positions,
        no_left_boundary,
        right_boundary,
        out,
        out_segments,
        output_len,
        fuel,
        projection,
        tfm_work,
    )?;
    if trailing_font_kern && !matches!(right_boundary, HyphenationRightBoundary::Character(_)) {
        stores.append_page_active_span_range(out, source.span(), index - 1..index);
        *output_len += 1;
    }
    Ok(index)
}

struct HyphenationCandidate {
    word_start: usize,
    end: usize,
    right_boundary: HyphenationRightBoundary,
    language: u8,
    left: usize,
    right: usize,
    word: smallvec::SmallVec<[WordChar; 64]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HyphenationRightBoundary {
    None,
    Font,
    Character(u8),
}

/// Implements TeX82 §§891--895's bounded scan from the glue after which a
/// potentially hyphenatable part begins through the same-font letter span.
fn find_hyphenation_candidate<G>(
    stores: &CommandContext<'_, G>,
    source: tex_state::page_node_arena::AdmittedPageList,
    start: usize,
    context: (u8, usize, usize),
) -> Option<HyphenationCandidate> {
    let nodes = stores.admitted_page_nodes(source).ok()?;
    if start > nodes.len() {
        return None;
    }
    let (mut language, mut left, mut right) = context;
    let mut word_start = None;
    let mut font = None;
    let mut minima = 0;
    let mut word = smallvec::SmallVec::<[WordChar; 64]>::new();
    let mut word_end = None;
    let mut right_boundary = HyphenationRightBoundary::None;
    // TeX describes seeking the first letter, collecting the bounded word,
    // and checking its terminator as adjacent loops. Keep those phases in one
    // forward arena traversal so a packed predecessor chain is descended
    // once, while preserving the word-end coordinate from the middle phase.
    let permitted = nodes.try_for_each_range(start..nodes.len(), |index, node| {
        if word_start.is_none() {
            match first_word_char(stores, language, node.clone()) {
                Some((candidate_font, ch, lower)) => {
                    if lower != ch && stores.int_param(IntParam::UC_HYPH) <= 0 {
                        return core::ops::ControlFlow::Break(false);
                    }
                    let Some(candidate_minima) = left.checked_add(right) else {
                        return core::ops::ControlFlow::Break(false);
                    };
                    if candidate_minima > 63
                        || !(0..=255).contains(&stores.font_hyphen_char(candidate_font))
                    {
                        return core::ops::ControlFlow::Break(false);
                    }
                    word_start = Some(index);
                    font = Some(candidate_font);
                    minima = candidate_minima;
                }
                None if is_pre_word_skip(node.clone()) => {
                    update_hyphenation_context(node, &mut language, &mut left, &mut right);
                    return core::ops::ControlFlow::Continue(());
                }
                None => return core::ops::ControlFlow::Break(false),
            }
        }

        if word_end.is_none() {
            let candidate_font = font.expect("word start establishes its font");
            let continues_word = match node {
                tex_state::NodeView::Char {
                    font: node_font,
                    ch,
                    origin,
                } if node_font == candidate_font => {
                    if word.len() < 63
                        && let Some(lower) = normalized_hyphen_code(stores, language, ch)
                    {
                        word.push(WordChar {
                            font: candidate_font,
                            ch,
                            lower,
                            origin,
                        });
                        right_boundary = HyphenationRightBoundary::None;
                        true
                    } else {
                        right_boundary = u8::try_from(ch as u32).map_or(
                            HyphenationRightBoundary::None,
                            HyphenationRightBoundary::Character,
                        );
                        false
                    }
                }
                tex_state::NodeView::Lig {
                    font: node_font,
                    ref orig,
                    ref origins,
                    right_hit,
                    ..
                } if node_font == candidate_font => {
                    let original_len = word.len();
                    let capacity_ok = word
                        .len()
                        .checked_add(orig.len())
                        .is_some_and(|len| len <= 63);
                    let mut valid = capacity_ok;
                    if capacity_ok {
                        for (offset, &ch) in orig.iter().enumerate() {
                            let Some(lower) = normalized_hyphen_code(stores, language, ch) else {
                                valid = false;
                                break;
                            };
                            if let Some(&origin) = origins.get(offset) {
                                word.push(WordChar {
                                    font: candidate_font,
                                    ch,
                                    lower,
                                    origin,
                                });
                            }
                        }
                    }
                    if !valid {
                        word.truncate(original_len);
                        right_boundary = orig
                            .first()
                            .and_then(|ch| u8::try_from(*ch as u32).ok())
                            .map_or(
                                HyphenationRightBoundary::None,
                                HyphenationRightBoundary::Character,
                            );
                    } else if right_hit {
                        right_boundary = HyphenationRightBoundary::Font;
                    } else {
                        right_boundary = HyphenationRightBoundary::None;
                    }
                    valid
                }
                tex_state::NodeView::Kern {
                    kind: KernKind::Font,
                    ..
                } => {
                    right_boundary = HyphenationRightBoundary::Font;
                    true
                }
                _ => false,
            };
            if continues_word {
                return core::ops::ControlFlow::Continue(());
            }
            word_end = Some(index);
        }

        match node {
            tex_state::NodeView::Char { .. }
            | tex_state::NodeView::Lig { .. }
            | tex_state::NodeView::Kern {
                kind: KernKind::Font,
                ..
            } => core::ops::ControlFlow::Continue(()),
            tex_state::NodeView::Glue { .. }
            | tex_state::NodeView::Penalty(_)
            | tex_state::NodeView::Ins { .. }
            | tex_state::NodeView::Adjust(_)
            | tex_state::NodeView::Mark { .. }
            | tex_state::NodeView::Whatsit(_)
            | tex_state::NodeView::Direction(
                tex_state::node::Direction::BeginL
                | tex_state::node::Direction::EndL
                | tex_state::node::Direction::BeginR
                | tex_state::node::Direction::EndR,
            )
            | tex_state::NodeView::Kern { .. } => core::ops::ControlFlow::Break(true),
            _ => core::ops::ControlFlow::Break(false),
        }
    });
    let permitted = match permitted {
        core::ops::ControlFlow::Continue(()) => word_start.is_some(),
        core::ops::ControlFlow::Break(permitted) => permitted,
    };
    if !permitted || word.len() < minima {
        return None;
    }
    Some(HyphenationCandidate {
        word_start: word_start.expect("permitted candidate has a word start"),
        end: word_end.unwrap_or(nodes.len()),
        right_boundary,
        language,
        left,
        right,
        word,
    })
}

fn first_word_char<G>(
    stores: &CommandContext<'_, G>,
    language: u8,
    node: tex_state::NodeView<'_>,
) -> Option<(tex_state::ids::FontId, char, char)> {
    match node {
        tex_state::NodeView::Char { font, ch, .. } => {
            normalized_hyphen_code(stores, language, ch).map(|lower| (font, ch, lower))
        }
        tex_state::NodeView::Lig { font, orig, .. } => orig.first().and_then(|&first| {
            normalized_hyphen_code(stores, language, first).map(|lower| (font, first, lower))
        }),
        _ => None,
    }
}

fn is_pre_word_skip(node: tex_state::NodeView<'_>) -> bool {
    matches!(
        node,
        tex_state::NodeView::Kern {
            kind: KernKind::Font,
            ..
        } | tex_state::NodeView::Whatsit(_)
            | tex_state::NodeView::Direction(
                tex_state::node::Direction::BeginL
                    | tex_state::node::Direction::EndL
                    | tex_state::node::Direction::BeginR
                    | tex_state::node::Direction::EndR
            )
    ) || matches!(
        node,
        tex_state::NodeView::Char { .. } | tex_state::NodeView::Lig { .. }
    )
}

fn parse_exception_word<G>(
    stores: &CommandContext<'_, G>,
    language: u8,
    word: &[char],
    normalize: bool,
) -> (Option<ExceptionSpec>, usize) {
    let mut normalized = String::new();
    let mut positions = Vec::new();
    let mut not_letters = 0;
    let mut letters_seen = 0usize;
    for &ch in word {
        if ch == '-' {
            // pdfTeX §§26030--27481 retain at most 63 exception letters.
            // A marker after a discarded letter is discarded with it rather
            // than being folded onto the last retained interletter boundary.
            if letters_seen <= 63 {
                positions.push(normalized.chars().count());
            }
            continue;
        }
        // A word the live scanner produced has already had §935's `lc_code`
        // test applied to every character, so every character left in it is a
        // letter and nothing here can diagnose one.
        let mapped = if normalize {
            normalized_hyphen_code(stores, language, ch)
        } else {
            Some(ch)
        };
        if let Some(ch) = mapped {
            letters_seen += 1;
            if normalized.chars().count() < 63 {
                normalized.push(ch);
            }
        } else {
            not_letters += 1;
        }
    }
    (
        (!normalized.is_empty()).then_some(ExceptionSpec {
            word: normalized,
            positions,
        }),
        not_letters,
    )
}

fn normalized_lccode<G>(stores: &CommandContext<'_, G>, ch: char) -> Option<char> {
    char::from_u32(stores.lccode(ch)).filter(|&mapped| mapped != '\0')
}

fn normalized_hyphen_code<G>(
    stores: &CommandContext<'_, G>,
    language: u8,
    ch: char,
) -> Option<char> {
    stores
        .saved_hyphenation_code(language, ch)
        .unwrap_or_else(|| normalized_lccode(stores, ch))
}

fn current_language<G>(stores: &CommandContext<'_, G>) -> u8 {
    u8::try_from(stores.int_param(IntParam::LANGUAGE)).unwrap_or(0)
}

fn right_boundary_cell(boundary: HyphenationRightBoundary) -> LigatureRightBoundary {
    match boundary {
        HyphenationRightBoundary::None => LigatureRightBoundary::Suppressed,
        HyphenationRightBoundary::Font => LigatureRightBoundary::Font,
        HyphenationRightBoundary::Character(ch) => LigatureRightBoundary::Character(ch),
    }
}

/// Runs one branch directly into its final page-list builder. Nested replays
/// borrow the parent cursor's tape window, so neither a node list nor an event
/// list is needed between TFM and the page arena.
fn reconstitute_branch<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: &[PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    fuel: &mut tex_command::CommandFuel,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    if source.is_empty() {
        return Ok(tex_state::node_arena::PageListId::empty());
    }
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let result = {
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: &mut output,
        };
        crate::box_runtime::hmode::run_tfm_ligature_machine_nested_with_work(
            stores,
            diagnostic_effects,
            source,
            no_left_boundary,
            right_boundary,
            false,
            fuel,
            tfm_work,
            &mut sink,
        )
    };
    match result {
        Ok(()) => Ok(stores.finalize_page_active_list(&mut output)),
        Err(error) => {
            stores.rollback_page_active_list(&mut output);
            Err(ExecError::Command(error))
        }
    }
}

#[derive(Clone, Copy)]
struct PreviousOutput {
    font: tex_state::ids::FontId,
    ch: char,
    origin: OriginId,
    ligature_present: bool,
}

struct PreBreakSink<'a> {
    output: &'a mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    previous: Option<PreviousOutput>,
    fallback: Option<PendingHRunChar>,
    saw_output: bool,
    keep_reconstituted_tail: bool,
}

impl PreBreakSink<'_> {
    fn emit_fallback<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        let glyph = self
            .fallback
            .take()
            .expect("pre-break fallback hyphen remains available");
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph(stores, glyph);
    }
}

impl FinalHNodeSink for PreBreakSink<'_> {
    fn glyph<G>(&mut self, stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar) {
        if !self.saw_output {
            self.saw_output = true;
            if !glyph.ligature_present
                && glyph.orig.len() == 1
                && self.previous.is_some_and(|previous| {
                    previous.font == glyph.font
                        && previous.ch == glyph.ch
                        && !previous.ligature_present
                        && previous.origin
                            == glyph.origins.first().cloned().unwrap_or(OriginId::UNKNOWN)
                })
            {
                self.keep_reconstituted_tail = true;
                return;
            }
            self.emit_fallback(stores);
            return;
        }
        if !self.keep_reconstituted_tail {
            return;
        }
        self.saw_output = true;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph(stores, glyph);
    }

    fn glyph_cell<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        source: &[crate::box_runtime::hmode::LigatureSourceEntry],
    ) {
        if !self.saw_output {
            self.saw_output = true;
            if !glyph.ligature_present
                && source.len() == 1
                && self.previous.is_some_and(|previous| {
                    previous.font == glyph.font
                        && previous.ch == glyph.ch
                        && !previous.ligature_present
                        && previous.origin == source[0].origin
                })
            {
                self.keep_reconstituted_tail = true;
                return;
            }
            self.emit_fallback(stores);
            return;
        }
        if !self.keep_reconstituted_tail {
            return;
        }
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph_cell(stores, glyph, source);
    }

    fn kern<G>(&mut self, stores: &mut CommandContext<'_, G>, amount: Scaled, kind: KernKind) {
        if !self.saw_output {
            self.saw_output = true;
            self.emit_fallback(stores);
            return;
        }
        if !self.keep_reconstituted_tail {
            return;
        }
        self.saw_output = true;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.kern(stores, amount, kind);
    }

    fn explicit_hyphen_disc<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        if !self.saw_output {
            self.saw_output = true;
            self.emit_fallback(stores);
            return;
        }
        if !self.keep_reconstituted_tail {
            return;
        }
        self.saw_output = true;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.explicit_hyphen_disc(stores);
    }

    fn discretionary<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        kind: DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    ) {
        if !self.saw_output {
            self.saw_output = true;
            self.emit_fallback(stores);
            return;
        }
        if !self.keep_reconstituted_tail {
            return;
        }
        self.saw_output = true;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.discretionary(stores, kind, pre, post, replace, physical_replace_count);
    }
}

fn pending_word_range(
    word: &[WordChar],
    range: std::ops::Range<usize>,
) -> SmallVec<[PendingHChar; 64]> {
    word[range]
        .iter()
        .map(WordChar::pending)
        .collect::<SmallVec<_>>()
}

fn pre_break_branch<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    previous: Option<PreviousOutput>,
    fallback: PendingHRunChar,
    source: &[PendingHChar],
    fuel: &mut tex_command::CommandFuel,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    if source.is_empty() {
        return Ok(tex_state::node_arena::PageListId::empty());
    }
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let result = {
        let mut sink = PreBreakSink {
            output: &mut output,
            previous,
            fallback: Some(fallback),
            saw_output: false,
            keep_reconstituted_tail: false,
        };
        crate::box_runtime::hmode::run_tfm_ligature_machine_nested_with_work(
            stores,
            diagnostic_effects,
            source,
            true,
            LigatureRightBoundary::Font,
            false,
            fuel,
            tfm_work,
            &mut sink,
        )
    };
    match result {
        Ok(()) => Ok(stores.finalize_page_active_list(&mut output)),
        Err(error) => {
            stores.rollback_page_active_list(&mut output);
            Err(ExecError::Command(error))
        }
    }
}

fn singleton_glyph_branch<G>(
    stores: &mut CommandContext<'_, G>,
    glyph: crate::box_runtime::hmode::LigatureGlyphCell,
    tfm_work: &crate::box_runtime::hmode::LigatureWorkList,
) -> tex_state::node_arena::PageListId {
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let source = tfm_work.source(glyph.provenance);
    let mut sink = crate::box_runtime::hmode::PageNodeSink {
        output: &mut output,
    };
    sink.glyph_cell(stores, glyph, source);
    stores.finalize_page_active_list(&mut output)
}

fn singleton_kern_branch<G>(
    stores: &mut CommandContext<'_, G>,
    amount: Scaled,
    kind: KernKind,
) -> tex_state::node_arena::PageListId {
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let mut sink = crate::box_runtime::hmode::PageNodeSink {
        output: &mut output,
    };
    sink.kern(stores, amount, kind);
    stores.finalize_page_active_list(&mut output)
}

struct PhysicalMeasureSink {
    current: usize,
    current_start: usize,
    current_len: usize,
    initial_end: usize,
    position: usize,
    nodes: usize,
    synchronization: Option<usize>,
    nodes_at_synchronization: usize,
}

impl PhysicalMeasureSink {
    fn observe(&mut self, work: &crate::box_runtime::hmode::LigatureWorkList) {
        if self.synchronization.is_none()
            && self.position >= self.initial_end
            && work.physical_boundary_present(
                self.current,
                self.current_start,
                self.current_len,
                self.position,
            )
        {
            self.synchronization = Some(self.position);
            self.nodes_at_synchronization = self.nodes;
        } else if self
            .synchronization
            .is_some_and(|synchronization| self.position <= synchronization)
        {
            self.nodes_at_synchronization = self.nodes;
        }
    }
}

impl FinalHNodeSink for PhysicalMeasureSink {
    fn glyph<G>(&mut self, _stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar) {
        self.nodes += 1;
        self.position = self.position.saturating_add(glyph.orig.len());
    }

    fn glyph_cell<G>(
        &mut self,
        _stores: &mut CommandContext<'_, G>,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        source: &[crate::box_runtime::hmode::LigatureSourceEntry],
    ) {
        self.nodes += 1;
        self.position = self.position.saturating_add(source.len());
        let _ = glyph;
    }

    fn glyph_cell_with_work<G>(
        &mut self,
        _stores: &mut CommandContext<'_, G>,
        _current: usize,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        work: &mut crate::box_runtime::hmode::LigatureWorkList,
        _diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        self.nodes += 1;
        self.position = self.position.saturating_add(glyph.provenance.len);
        self.observe(work);
        Ok(())
    }

    fn kern<G>(&mut self, _stores: &mut CommandContext<'_, G>, _amount: Scaled, _kind: KernKind) {
        self.nodes += 1;
    }

    fn kern_with_work<G>(
        &mut self,
        _stores: &mut CommandContext<'_, G>,
        _current: usize,
        _amount: Scaled,
        _kind: KernKind,
        work: &mut crate::box_runtime::hmode::LigatureWorkList,
        _diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        _fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        self.nodes += 1;
        self.observe(work);
        Ok(())
    }

    fn explicit_hyphen_disc<G>(&mut self, _stores: &mut CommandContext<'_, G>) {
        self.nodes += 1;
    }

    fn discretionary<G>(
        &mut self,
        _stores: &mut CommandContext<'_, G>,
        _kind: DiscKind,
        _pre: tex_state::node_arena::PageListId,
        _post: tex_state::node_arena::PageListId,
        _replace: tex_state::node_arena::PageListId,
        _physical_replace_count: u8,
    ) {
        self.nodes += 1;
    }
}

struct PhysicalPrefixSink<'a> {
    output: &'a mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    remaining: usize,
}

impl FinalHNodeSink for PhysicalPrefixSink<'_> {
    fn glyph<G>(&mut self, stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph(stores, glyph);
    }

    fn glyph_cell<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        source: &[crate::box_runtime::hmode::LigatureSourceEntry],
    ) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph_cell(stores, glyph, source);
    }

    fn kern<G>(&mut self, stores: &mut CommandContext<'_, G>, amount: Scaled, kind: KernKind) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.kern(stores, amount, kind);
    }

    fn explicit_hyphen_disc<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.explicit_hyphen_disc(stores);
    }

    fn discretionary<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        kind: DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    ) {
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.discretionary(stores, kind, pre, post, replace, physical_replace_count);
    }
}

fn physical_post_branch<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: &[PendingHChar],
    right_boundary: LigatureRightBoundary,
    nodes: usize,
    fuel: &mut tex_command::CommandFuel,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    if source.is_empty() || nodes == 0 {
        return Ok(tex_state::node_arena::PageListId::empty());
    }
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut output);
    let result = {
        let mut sink = PhysicalPrefixSink {
            output: &mut output,
            remaining: nodes,
        };
        crate::box_runtime::hmode::run_tfm_ligature_machine_nested_with_work(
            stores,
            diagnostic_effects,
            source,
            false,
            right_boundary,
            false,
            fuel,
            tfm_work,
            &mut sink,
        )
    };
    match result {
        Ok(()) => Ok(stores.finalize_page_active_list(&mut output)),
        Err(error) => {
            stores.rollback_page_active_list(&mut output);
            Err(ExecError::Command(error))
        }
    }
}

#[allow(clippy::too_many_arguments)] // TeX's projection keeps source coordinates, branches, tape, diagnostics, and fuel explicit.
fn physical_projection_for_glyph<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    position: usize,
    current_start: usize,
    current_len: usize,
    current: usize,
    right_boundary: LigatureRightBoundary,
    fuel: &mut tex_command::CommandFuel,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
) -> Result<(u8, tex_state::node_arena::PageListId), ExecError> {
    let minor_pending = pending_word_range(word, position..word.len());
    let mut probe = PhysicalMeasureSink {
        current,
        current_start,
        current_len,
        initial_end: current_start.saturating_add(current_len),
        position,
        nodes: 0,
        synchronization: None,
        nodes_at_synchronization: 0,
    };
    // The first replay measures the full physical branch and owns its
    // diagnostics. The direct child replay below is output-only and keeps a
    // detached collector so a missing glyph is not reported twice.
    crate::box_runtime::hmode::run_tfm_ligature_machine_nested_with_work(
        stores,
        diagnostic_effects,
        &minor_pending,
        false,
        right_boundary,
        false,
        fuel,
        tfm_work,
        &mut probe,
    )
    .map_err(ExecError::Command)?;
    let synchronization = probe.synchronization.unwrap_or(word.len());
    let major_len = tfm_work.physical_nodes_through_boundary(
        current,
        current_start,
        current_len,
        synchronization,
    );
    let minor_len = if probe.synchronization.is_some() {
        probe.nodes_at_synchronization
    } else {
        probe.nodes
    };
    let mut replay_diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
    let physical_post = physical_post_branch(
        stores,
        &mut replay_diagnostic_effects,
        &minor_pending,
        right_boundary,
        minor_len,
        fuel,
        tfm_work,
    )?;
    let physical_replace_count = u8::try_from(major_len)
        .ok()
        .filter(|&count| count <= 127)
        .expect("a TeX word has at most 127 physical replacement nodes");
    Ok((physical_replace_count, physical_post))
}

struct HyphenationReconstitutionCursor<'output, 'word, 'projection, 'vectors> {
    output: &'output mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &'output mut Vec<tex_state::node_arena::PageListId>,
    word: &'word [WordChar],
    positions: &'word [usize],
    right_boundary: LigatureRightBoundary,
    position_index: usize,
    char_start: usize,
    output_len: &'output mut usize,
    projection: &'projection mut HyphenationProjection<'vectors>,
    previous: Option<PreviousOutput>,
}

fn command_error(error: ExecError) -> tex_command::CommandError {
    match error {
        ExecError::Command(error) => error,
        _ => unreachable!("hyphenation branch construction only reports command errors"),
    }
}

impl<'output, 'word, 'projection, 'vectors>
    HyphenationReconstitutionCursor<'output, 'word, 'projection, 'vectors>
{
    fn suspend_main<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        let segment = stores.finalize_page_active_list(self.output);
        if !segment.is_empty() {
            self.out_segments.push(segment);
        }
    }

    fn resume_main<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        stores.open_page_active_list(self.output);
    }

    fn automatic_pre<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        position: usize,
        full_prefix: bool,
        fuel: &mut tex_command::CommandFuel,
        tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    ) -> Result<tex_state::node_arena::PageListId, ExecError> {
        let previous = self.word[position - 1];
        let mut source = if full_prefix {
            pending_word_range(self.word, 0..position)
        } else {
            pending_word_range(self.word, position - 1..position)
        };
        let Some(ch) = automatic_hyphen_char(
            stores,
            previous.font,
            *self.output_len,
            self.projection.missing_hyphens,
        ) else {
            return Ok(tex_state::node_arena::PageListId::empty());
        };
        let fallback = PendingHRunChar::new(previous.font, ch, OriginId::UNKNOWN);
        source.push(PendingHChar {
            font: previous.font,
            ch,
            origin: previous.origin,
        });
        if full_prefix {
            reconstitute_branch(
                stores,
                diagnostic_effects,
                &source,
                true,
                LigatureRightBoundary::Font,
                fuel,
                tfm_work,
            )
        } else {
            pre_break_branch(
                stores,
                diagnostic_effects,
                self.previous,
                fallback,
                &source,
                fuel,
                tfm_work,
            )
        }
    }

    fn append_boundary_disc<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        replacement: Option<(Scaled, KernKind)>,
        fuel: &mut tex_command::CommandFuel,
        tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    ) -> Result<(), ExecError> {
        let position = self
            .positions
            .get(self.position_index)
            .copied()
            .expect("boundary discretionary has a pending source position");
        if position == 0 {
            self.position_index += 1;
            return Ok(());
        }
        self.suspend_main(stores);
        let pre =
            match self.automatic_pre(stores, diagnostic_effects, position, false, fuel, tfm_work) {
                Ok(pre) => pre,
                Err(error) => {
                    self.resume_main(stores);
                    return Err(error);
                }
            };
        let (replace, physical_replace_count) = replacement.map_or(
            (tex_state::node_arena::PageListId::empty(), 0),
            |(amount, kind)| (singleton_kern_branch(stores, amount, kind), 2),
        );
        let empty = tex_state::node_arena::PageListId::empty();
        self.resume_main(stores);
        stores.construct_page_active_list(self.output, |destination| {
            destination.discretionary(
                DiscKind::AutomaticHyphen,
                pre,
                empty,
                replace,
                physical_replace_count,
            );
        });
        self.previous = None;
        *self.output_len += 1;
        self.position_index += 1;
        Ok(())
    }

    fn append_through_disc<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        current: usize,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        fuel: &mut tex_command::CommandFuel,
        tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
    ) -> Result<usize, ExecError> {
        self.suspend_main(stores);
        let position = self.positions[self.position_index];
        let end = self.char_start.saturating_add(glyph.provenance.len);
        let pre =
            match self.automatic_pre(stores, diagnostic_effects, position, true, fuel, tfm_work) {
                Ok(pre) => pre,
                Err(error) => {
                    self.resume_main(stores);
                    return Err(error);
                }
            };
        let post_source = pending_word_range(self.word, position..end);
        let post = match reconstitute_branch(
            stores,
            diagnostic_effects,
            &post_source,
            false,
            self.right_boundary,
            fuel,
            tfm_work,
        ) {
            Ok(post) => post,
            Err(error) => {
                self.resume_main(stores);
                return Err(error);
            }
        };
        let (physical_replace_count, physical_post) = match physical_projection_for_glyph(
            stores,
            diagnostic_effects,
            self.word,
            position,
            self.char_start,
            glyph.provenance.len,
            current,
            self.right_boundary,
            fuel,
            tfm_work,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                self.resume_main(stores);
                return Err(error);
            }
        };
        let replace = singleton_glyph_branch(stores, glyph, tfm_work);
        self.resume_main(stores);
        stores.construct_page_active_list(self.output, |destination| {
            destination.discretionary(
                DiscKind::AutomaticHyphen,
                pre,
                post,
                replace,
                physical_replace_count,
            );
        });
        self.previous = None;
        self.projection
            .physical_post_overrides
            .push((*self.output_len, physical_post));
        *self.output_len += 1;
        self.position_index += 1;
        Ok(end)
    }
}

impl FinalHNodeSink for HyphenationReconstitutionCursor<'_, '_, '_, '_> {
    fn glyph<G>(&mut self, stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar) {
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph(stores, glyph);
        *self.output_len += 1;
    }

    fn glyph_cell_with_work<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        current: usize,
        glyph: crate::box_runtime::hmode::LigatureGlyphCell,
        work: &mut crate::box_runtime::hmode::LigatureWorkList,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        let source_len = glyph.provenance.len;
        let char_end = self.char_start.saturating_add(source_len);
        let boundary = self.positions.get(self.position_index) == Some(&self.char_start);
        if boundary {
            while self.positions.get(self.position_index) == Some(&self.char_start) {
                self.append_boundary_disc(stores, diagnostic_effects, None, fuel, work)
                    .map_err(command_error)?;
            }
        }
        if let Some(&position) = self
            .positions
            .get(self.position_index)
            .filter(|&&position| self.char_start < position && position < char_end)
        {
            let end = self
                .append_through_disc(stores, diagnostic_effects, current, glyph, fuel, work)
                .map_err(command_error)?;
            while self
                .positions
                .get(self.position_index)
                .is_some_and(|&next| next < end)
            {
                self.position_index += 1;
            }
            self.char_start = end;
            let _ = position;
            return Ok(());
        }
        let glyph_font = glyph.font;
        let glyph_is_ligature = glyph.ligature_present;
        let source = work.source(glyph.provenance);
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.glyph_cell(stores, glyph, source);
        *self.output_len += 1;
        let previous = source.first().map(|entry| PreviousOutput {
            font: glyph_font,
            ch: entry.ch,
            origin: entry.origin,
            ligature_present: glyph_is_ligature,
        });
        self.previous = previous;
        self.char_start = char_end;
        Ok(())
    }

    fn kern<G>(&mut self, stores: &mut CommandContext<'_, G>, amount: Scaled, kind: KernKind) {
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.kern(stores, amount, kind);
        *self.output_len += 1;
        self.previous = None;
    }

    fn kern_with_work<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        _current: usize,
        amount: Scaled,
        kind: KernKind,
        work: &mut crate::box_runtime::hmode::LigatureWorkList,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
        fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        if kind == KernKind::Font
            && self.positions.get(self.position_index) == Some(&self.char_start)
        {
            while self.positions.get(self.position_index) == Some(&self.char_start) {
                self.append_boundary_disc(
                    stores,
                    diagnostic_effects,
                    Some((amount, kind)),
                    fuel,
                    work,
                )
                .map_err(command_error)?;
            }
            return Ok(());
        }
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.kern(stores, amount, kind);
        *self.output_len += 1;
        self.previous = None;
        Ok(())
    }

    fn explicit_hyphen_disc<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.explicit_hyphen_disc(stores);
        self.previous = None;
        *self.output_len += 1;
    }

    fn discretionary<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        kind: DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    ) {
        let mut sink = crate::box_runtime::hmode::PageNodeSink {
            output: self.output,
        };
        sink.discretionary(stores, kind, pre, post, replace, physical_replace_count);
        self.previous = None;
        *self.output_len += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn append_hyphenated_word<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    positions: &[usize],
    no_left_boundary: bool,
    right_boundary: HyphenationRightBoundary,
    output: &mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &mut Vec<tex_state::node_arena::PageListId>,
    output_len: &mut usize,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
    tfm_work: &mut crate::box_runtime::hmode::LigatureWorkList,
) -> Result<(), ExecError> {
    let pending = pending_word_range(word, 0..word.len());
    let mut cursor = HyphenationReconstitutionCursor {
        output,
        out_segments,
        word,
        positions,
        right_boundary: right_boundary_cell(right_boundary),
        position_index: 0,
        char_start: 0,
        output_len,
        projection,
        previous: None,
    };
    crate::box_runtime::hmode::run_tfm_ligature_machine_with_work(
        stores,
        diagnostic_effects,
        &pending,
        no_left_boundary,
        right_boundary_cell(right_boundary),
        false,
        fuel,
        tfm_work,
        &mut cursor,
    )
    .map_err(ExecError::Command)?;
    while let Some(&position) = cursor.positions.get(cursor.position_index) {
        debug_assert_eq!(position, cursor.char_start);
        cursor.append_boundary_disc(stores, diagnostic_effects, None, fuel, tfm_work)?;
    }
    Ok(())
}

fn usable_hyphen_char<G>(
    stores: &CommandContext<'_, G>,
    font: tex_state::ids::FontId,
) -> Option<char> {
    let code = u8::try_from(stores.font_hyphen_char(font)).ok()?;
    stores
        .font_character_exists(font, char::from(code))
        .then(|| char::from(code))
}

/// TeX82 §929's `new_character(hf, hyf_char)` at automatic-discretionary
/// construction. An out-of-range hyphen character disables the attempt in
/// §923, while an in-range missing glyph warns under §581 and yields no node.
fn automatic_hyphen_char<G>(
    stores: &CommandContext<'_, G>,
    font: tex_state::ids::FontId,
    node_index: usize,
    missing_hyphens: &mut Vec<MissingHyphenDiagnostic>,
) -> Option<char> {
    let code = u8::try_from(stores.font_hyphen_char(font)).ok()?;
    if stores.font_character_exists(font, char::from(code)) {
        return Some(char::from(code));
    }
    missing_hyphens.push(MissingHyphenDiagnostic {
        node_index,
        font,
        ch: char::from(code),
    });
    None
}

#[derive(Clone, Copy)]
struct WordChar {
    font: tex_state::ids::FontId,
    ch: char,
    lower: char,
    origin: OriginId,
}

impl WordChar {
    fn pending(&self) -> PendingHChar {
        PendingHChar {
            font: self.font,
            ch: self.ch,
            origin: self.origin,
        }
    }
}
use tex_state::CommandContext;
use tex_state::env::banks::IntParam;
use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::node::{DiscKind, KernKind};
use tex_state::token::OriginId;

use crate::ExecError;
use crate::box_runtime::hmode::{FinalHNodeSink, LigatureRightBoundary};
use crate::mode::{PendingHChar, PendingHRunChar};
use tex_state::scaled::Scaled;

#[cfg(test)]
mod tests {
    use super::*;
    use tex_state::node::Node;

    fn character(font: tex_state::ids::FontId, ch: char) -> Node {
        Node::Char {
            font,
            ch,
            origin: OriginId::UNKNOWN,
        }
    }

    fn candidate<G>(
        stores: &mut CommandContext<'_, G>,
        nodes: &[Node],
    ) -> Option<HyphenationCandidate> {
        let source = stores.publish_page_nodes(nodes.to_vec());
        let source = stores
            .admit_page_node_list(source)
            .expect("test paragraph source remains live");
        find_hyphenation_candidate(stores, source, 0, (0, 1, 1))
    }

    fn second_font<G>(stores: &mut CommandContext<'_, G>) -> tex_state::ids::FontId {
        stores.intern_font(tex_state::font::LoadedFont::new(
            "second",
            "second.tfm",
            tex_fonts::font_content_hash(b"second"),
            0,
            tex_state::scaled::Scaled::from_raw(10 * tex_state::scaled::Scaled::UNITY),
            tex_state::scaled::Scaled::from_raw(10 * tex_state::scaled::Scaled::UNITY),
            vec![tex_state::scaled::Scaled::from_raw(0); 7],
            tex_state::font::FontMetrics::default(),
        ))
    }

    fn hyphenation_font<G>(stores: &mut CommandContext<'_, G>) -> tex_state::ids::FontId {
        let mut characters = vec![None; 256];
        for code in [b'-', b'.', b'a', b'b', b'c', b'd'] {
            characters[usize::from(code)] = Some(tex_state::font::CharMetrics {
                width: tex_state::scaled::Scaled::from_raw(tex_state::scaled::Scaled::UNITY / 2),
                height: tex_state::scaled::Scaled::from_raw(0),
                depth: tex_state::scaled::Scaled::from_raw(0),
                italic_correction: tex_state::scaled::Scaled::from_raw(0),
                tag: tex_state::font::CharTag::None,
            });
        }
        characters[usize::from(b'c')]
            .as_mut()
            .expect("test letter exists")
            .tag = tex_state::font::CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        };
        characters[usize::from(b'd')]
            .as_mut()
            .expect("test letter exists")
            .tag = tex_state::font::CharTag::LigKern {
            program_index: 1,
            start_index: 1,
        };
        let program = vec![
            tex_fonts::LigKernInstruction {
                skip_byte: 128,
                next_char: b'-',
                command: Some(tex_fonts::LigKernCommand::Kern(
                    tex_state::scaled::Scaled::from_raw(-2_345),
                )),
            },
            tex_fonts::LigKernInstruction {
                skip_byte: 128,
                next_char: b'.',
                command: Some(tex_fonts::LigKernCommand::Kern(
                    tex_state::scaled::Scaled::from_raw(-1_234),
                )),
            },
        ];
        let size = tex_state::scaled::Scaled::from_raw(10 * tex_state::scaled::Scaled::UNITY);
        stores.intern_font(tex_state::font::LoadedFont::new(
            "hyphenation-test",
            "hyphenation-test.tfm",
            tex_fonts::font_content_hash(b"hyphenation-test"),
            0,
            size,
            size,
            vec![tex_state::scaled::Scaled::from_raw(0); 7],
            tex_state::font::FontMetrics::new(characters, program, None, None, Vec::new()),
        ))
    }

    fn hyphenation_source<G>(
        stores: &mut CommandContext<'_, G>,
        font: tex_state::ids::FontId,
    ) -> tex_state::node_arena::PageListId {
        let mut nodes = vec![Node::Glue {
            spec: tex_state::glue::GlueSpec::ZERO,
            kind: tex_state::node::GlueKind::Normal,
            leader: None,
        }];
        nodes.extend("abcd".chars().map(|ch| character(font, ch)));
        nodes.push(Node::Penalty(0));
        stores.publish_page_nodes(nodes)
    }

    fn diagnostic_text<G>(stores: &tex_state::Universe<G>) -> String {
        stores
            .world()
            .effect_records()
            .iter()
            .filter_map(|record| match record {
                tex_state::world::EffectRecord::StreamWrite { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn automatic_missing_hyphen_warns_but_disabled_hyphen_does_not() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = stores.current_font();
            stores
                .assign_int_param(
                    IntParam::TRACING_LOST_CHARS,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("parameter");
            stores.set_font_hyphen_char(font, -1);
            let mut missing = Vec::new();
            assert_eq!(automatic_hyphen_char(&stores, font, 7, &mut missing), None);
            assert!(missing.is_empty());

            stores.set_font_hyphen_char(font, i32::from(b'?'));
            assert_eq!(automatic_hyphen_char(&stores, font, 7, &mut missing), None);
            assert_eq!(missing.len(), 1);
            assert_eq!(missing[0].node_index, 7);
            assert_eq!((missing[0].font, missing[0].ch), (font, '?'));
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            crate::diagnostics::report_missing_character_warning(
                &mut stores,
                &mut diagnostic_effects,
                font,
                '?',
                false,
            );
            drop(stores);
            universe
                .world_mut()
                .publish_diagnostic_effects(diagnostic_effects);
            assert!(
                diagnostic_text(universe)
                    .contains("Missing character: There is no ? in font nullfont!")
            );
        });
    }

    #[test]
    fn automatic_discretionary_children_publish_between_main_list_segments() {
        // TeX82 §§914--918 constructs pre-break, post-break, and replacement
        // lists before linking the discretionary into the main list.
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            stores.add_hyphenation_exception_for_language(
                0,
                ExceptionSpec {
                    word: "abcd".to_owned(),
                    positions: vec![2],
                },
            );
            stores
                .assign_int_param(
                    IntParam::LEFT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("left minimum");
            stores
                .assign_int_param(
                    IntParam::RIGHT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("right minimum");
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");

            let hyphenated = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut effects,
                source,
                &mut scratch,
                fuel.fuel_mut(),
            )
            .expect("automatic discretionary construction succeeds");
            let semantic = stores
                .page_nodes(hyphenated.semantic)
                .expect("semantic list");
            let disc = semantic
                .iter()
                .find(|node| matches!(node, tex_state::NodeView::Disc { .. }))
                .expect("exception inserts a discretionary");
            let tex_state::NodeView::Disc { pre, .. } = disc else {
                unreachable!()
            };
            assert!(matches!(
                stores
                    .page_node_list(pre)
                    .expect("pre-break child remains live")
                    .first(),
                Some(tex_state::NodeView::Char { ch: '-', .. })
            ));

            let afterward = stores.publish_page_nodes(vec![Node::Penalty(17)]);
            assert_eq!(
                afterward.len(),
                1,
                "no active builder escapes paragraph end"
            );
        });
    }

    #[test]
    fn automatic_hyphen_retains_preceding_font_kern_in_pre_break_branch() {
        // TeX82 §§903--918 reconstitutes the preceding character together
        // with the optional hyphen because the pair can introduce a font
        // ligature or kern. The compact semantic list already retains the
        // preceding character, so its pre-break branch retains the suffix.
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            stores.add_hyphenation_exception_for_language(
                0,
                ExceptionSpec {
                    word: "abcd".to_owned(),
                    positions: vec![3],
                },
            );
            stores
                .assign_int_param(
                    IntParam::LEFT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("left minimum");
            stores
                .assign_int_param(
                    IntParam::RIGHT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("right minimum");
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");

            let hyphenated = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut effects,
                source,
                &mut scratch,
                fuel.fuel_mut(),
            )
            .expect("automatic discretionary construction succeeds");
            let semantic = stores
                .page_nodes(hyphenated.semantic)
                .expect("semantic list");
            let disc = semantic
                .iter()
                .find(|node| matches!(node, tex_state::NodeView::Disc { .. }))
                .expect("exception inserts a discretionary");
            let tex_state::NodeView::Disc { pre, .. } = disc else {
                unreachable!()
            };
            let pre = stores
                .page_node_list(pre)
                .expect("pre-break branch")
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            assert!(matches!(
                pre.as_slice(),
                [
                    Node::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                    Node::Char { ch: '-', .. },
                ] if amount.raw() == -2_345
            ));
        });
    }

    #[test]
    fn streaming_reconstitution_links_multiple_discretionaries_in_order() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            stores.add_hyphenation_exception_for_language(
                0,
                ExceptionSpec {
                    word: "abcd".to_owned(),
                    positions: vec![1, 3],
                },
            );
            stores
                .assign_int_param(
                    IntParam::LEFT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("left minimum");
            stores
                .assign_int_param(
                    IntParam::RIGHT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("right minimum");
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");
            let hyphenated = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut effects,
                source,
                &mut scratch,
                fuel.fuel_mut(),
            )
            .expect("multiple discretionary construction succeeds");
            let discs = stores
                .page_nodes(hyphenated.semantic)
                .expect("semantic list")
                .iter()
                .filter(|node| matches!(node, tex_state::NodeView::Disc { .. }))
                .count();
            assert_eq!(discs, 2);
        });
    }

    #[test]
    fn streaming_reconstitution_rolls_back_and_retries_after_fuel_abort() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            stores.add_hyphenation_exception_for_language(
                0,
                ExceptionSpec {
                    word: "abcd".to_owned(),
                    positions: vec![2],
                },
            );
            stores
                .assign_int_param(
                    IntParam::LEFT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("left minimum");
            stores
                .assign_int_param(
                    IntParam::RIGHT_HYPHEN_MIN,
                    1,
                    tex_state::AssignmentScope::Global,
                )
                .expect("right minimum");
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let mut exhausted = tex_command::CommandFuelLedger::new(1).expect("bounded fuel");
            assert!(
                hyphenated_hlist_with_fuel(
                    &mut stores,
                    &mut effects,
                    source,
                    &mut scratch,
                    exhausted.fuel_mut(),
                )
                .is_err()
            );
            assert_eq!(
                stores
                    .page_node_list(source)
                    .expect("source remains live")
                    .len(),
                6,
                "fuel abort leaves the pending semantic source untouched"
            );
            let mut retry = tex_command::CommandFuelLedger::default();
            let retried = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut effects,
                source,
                &mut scratch,
                retry.fuel_mut(),
            )
            .expect("retry after rollback succeeds");
            assert!(
                stores
                    .page_nodes(retried.semantic)
                    .expect("semantic list")
                    .iter()
                    .any(|node| matches!(node, tex_state::NodeView::Disc { .. }))
            );
        });
    }

    #[test]
    fn unhyphenated_word_keeps_the_single_main_list_owner() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");

            let unchanged = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut effects,
                source,
                &mut scratch,
                fuel.fuel_mut(),
            )
            .expect("word without an allowed position succeeds");
            assert!(
                stores
                    .page_nodes(unchanged.semantic)
                    .expect("semantic list")
                    .iter()
                    .all(|node| !matches!(node, tex_state::NodeView::Disc { .. })),
                "the same word without hyphenation history is the negative control"
            );
        });
    }

    #[test]
    fn pre_hyphenation_candidate_uses_language_minima_and_canonical_delimiters() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = stores.current_font();
            stores
                .assign_int_param(
                    IntParam::DEFAULT_HYPHEN_CHAR,
                    i32::from(b'-'),
                    tex_state::AssignmentScope::Global,
                )
                .expect("parameter");
            let nodes = vec![
                character(font, '.'),
                Node::Whatsit(tex_state::node::Whatsit::Language {
                    language: 7,
                    left_hyphen_min: 2,
                    right_hyphen_min: 2,
                }),
                Node::Kern {
                    amount: tex_state::scaled::Scaled::from_raw(0),
                    kind: KernKind::Font,
                },
                character(font, 'a'),
                character(font, 'b'),
                character(font, 'c'),
                character(font, 'd'),
                character(font, '.'),
                Node::Penalty(0),
            ];

            let found = candidate(&mut stores, &nodes).expect("four letters meet the 2+2 minima");
            assert_eq!((found.language, found.left, found.right), (7, 2, 2));
            assert_eq!((found.word_start, found.end), (3, 7));
            assert_eq!(
                found.right_boundary,
                HyphenationRightBoundary::Character(b'.')
            );
            assert!(found.word.iter().all(|letter| letter.font == font));
            assert_eq!(
                found
                    .word
                    .iter()
                    .map(|letter| letter.lower)
                    .collect::<String>(),
                "abcd"
            );

            let mut too_short = nodes;
            too_short[1] = Node::Whatsit(tex_state::node::Whatsit::Language {
                language: 7,
                left_hyphen_min: 3,
                right_hyphen_min: 2,
            });
            assert!(candidate(&mut stores, &too_short).is_none());
        });
    }

    #[test]
    fn hyphenation_reconstitution_uses_same_font_nonletter_as_right_boundary() {
        // TeX82 §897 saves the first same-font nonletter as `hyf_bchar`;
        // §§903--918 reconstitute the word against it while retaining the
        // original punctuation node after the replaced word.
        fn run(with_position: bool) -> Vec<Node> {
            let mut result = Vec::new();
            crate::test_harness::with_nonstop_plain_universe(|universe| {
                let mut stores = universe.command_context().expect("test state is admitted");
                let font = hyphenation_font(&mut stores);
                stores.set_font_hyphen_char(font, i32::from(b'-'));
                if with_position {
                    stores.add_hyphenation_exception_for_language(
                        0,
                        ExceptionSpec {
                            word: "abcd".to_owned(),
                            positions: vec![2],
                        },
                    );
                }
                stores
                    .assign_int_param(
                        IntParam::LEFT_HYPHEN_MIN,
                        1,
                        tex_state::AssignmentScope::Global,
                    )
                    .expect("left minimum");
                stores
                    .assign_int_param(
                        IntParam::RIGHT_HYPHEN_MIN,
                        1,
                        tex_state::AssignmentScope::Global,
                    )
                    .expect("right minimum");
                let mut nodes = vec![Node::Glue {
                    spec: tex_state::glue::GlueSpec::ZERO,
                    kind: tex_state::node::GlueKind::Normal,
                    leader: None,
                }];
                nodes.extend("abcd.".chars().map(|ch| character(font, ch)));
                nodes.push(Node::Penalty(0));
                let source = stores.publish_page_nodes(nodes);
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut scratch = crate::mode::HorizontalModeScratch::default();
                let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");
                let hyphenated = hyphenated_hlist_with_fuel(
                    &mut stores,
                    &mut effects,
                    source,
                    &mut scratch,
                    fuel.fuel_mut(),
                )
                .expect("hyphenation succeeds");
                result = stores
                    .page_nodes(hyphenated.semantic)
                    .expect("semantic list")
                    .iter()
                    .cloned()
                    .collect();
            });
            result
        }

        let unchanged = run(false);
        assert!(unchanged.windows(2).any(|nodes| matches!(
            nodes,
            [Node::Char { ch: 'd', .. }, Node::Char { ch: '.', .. }]
        )));

        let reconstituted = run(true);
        assert!(reconstituted.windows(3).any(|nodes| matches!(
            nodes,
            [
                Node::Char { ch: 'd', .. },
                Node::Kern {
                    amount,
                    kind: KernKind::Font,
                },
                Node::Char { ch: '.', .. },
            ] if amount.raw() == -1_234
        )));
    }

    #[test]
    fn pre_hyphenation_visits_every_base_whatsit_without_transferring_ownership() {
        // TeX82 §§1362--1363: pre-hyphenation recognizes only the language
        // subtype as state; every base subtype remains in exact list order
        // with its immutable token payload and stream fields still owned.
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
            let tokens = stores
                .allocate_node_token_list(&[tex_state::token::TokenWord::pack(
                    tex_state::token::Token::Char {
                        ch: 'w',
                        cat: tex_state::token::Catcode::Letter,
                    },
                )])
                .expect("test node token list");
            let nodes = vec![
                Node::Whatsit(tex_state::node::Whatsit::OpenOut {
                    slot: tex_state::StreamSlot::new(15),
                    path: "visit.tex".into(),
                }),
                Node::Whatsit(tex_state::node::Whatsit::DeferredWrite {
                    sink: tex_state::PrintSink::Log,
                    tokens,
                }),
                Node::Whatsit(tex_state::node::Whatsit::CloseOut {
                    slot: Some(tex_state::StreamSlot::new(0)),
                }),
                Node::Whatsit(tex_state::node::Whatsit::CloseOut { slot: None }),
                Node::Whatsit(tex_state::node::Whatsit::Special {
                    class: "dvi".into(),
                    payload: b"visit".to_vec(),
                }),
                Node::Whatsit(tex_state::node::Whatsit::Language {
                    language: 7,
                    left_hyphen_min: 2,
                    right_hyphen_min: 3,
                }),
            ];
            let mut fuel = tex_command::CommandFuelLedger::new(1_000).expect("bounded fuel");

            let source = stores.publish_page_nodes(nodes.clone());
            let mut scratch = crate::mode::HorizontalModeScratch::default();
            let visited = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut diagnostic_effects,
                source,
                &mut scratch,
                fuel.fuel_mut(),
            )
            .expect("base-whatsit visit succeeds");

            let semantic = stores
                .page_nodes(visited.semantic)
                .expect("semantic root")
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let physical = stores
                .page_nodes(visited.physical)
                .expect("physical root")
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(semantic, nodes);
            assert_eq!(physical, nodes);
            assert!(visited.missing_hyphens.is_empty());
            let mut physical_post_overrides = Vec::new();
            let mut missing_hyphens = Vec::new();
            let mut projection = HyphenationProjection {
                physical_post_overrides: &mut physical_post_overrides,
                missing_hyphens: &mut missing_hyphens,
            };
            let (_, final_context) = hyphenated_hlist_with_projections(
                &mut stores,
                &mut diagnostic_effects,
                source,
                scratch.tfm_work_mut(),
                fuel.fuel_mut(),
                &mut projection,
            )
            .expect("the traced pre-hyphenation visit succeeds");
            assert_eq!(
                final_context,
                HyphenationContext {
                    language: 7,
                    left: 2,
                    right: 3,
                },
                "the actual pre-hyphenation traversal applies the language node's state"
            );
            assert_eq!(
                stores
                    .node_token_words(tokens)
                    .expect("live node token key"),
                [tex_state::token::TokenWord::pack(
                    tex_state::token::Token::Char {
                        ch: 'w',
                        cat: tex_state::token::Catcode::Letter,
                    }
                )]
            );
            drop(stores);
            assert!(universe.world().effect_records().is_empty());
        });
    }

    #[test]
    fn pre_hyphenation_candidate_applies_uppercase_and_same_font_eligibility() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = stores.current_font();
            let other_font = second_font(&mut stores);
            stores
                .assign_int_param(
                    IntParam::DEFAULT_HYPHEN_CHAR,
                    i32::from(b'-'),
                    tex_state::AssignmentScope::Global,
                )
                .expect("parameter");
            let nodes = vec![
                character(font, 'A'),
                character(font, 'b'),
                character(font, 'c'),
                character(other_font, 'd'),
                Node::Penalty(0),
            ];

            assert!(
                candidate(&mut stores, &nodes).is_none(),
                "uppercase starts need uchyph"
            );
            stores
                .assign_int_param(IntParam::UC_HYPH, 1, tex_state::AssignmentScope::Global)
                .expect("parameter");
            let found = candidate(&mut stores, &nodes).expect("enabled uppercase candidate");
            assert_eq!((found.word_start, found.end), (0, 3));
            assert!(found.word.iter().all(|letter| letter.font == font));
            assert_eq!(
                found
                    .word
                    .iter()
                    .map(|letter| letter.lower)
                    .collect::<String>(),
                "abc",
                "the other-font character delimits the same-font lowercase projection"
            );
        });
    }

    #[test]
    fn pre_hyphenation_candidate_retains_the_63_letter_prefix_at_the_64_boundary() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = stores.current_font();
            stores
                .assign_int_param(
                    IntParam::DEFAULT_HYPHEN_CHAR,
                    i32::from(b'-'),
                    tex_state::AssignmentScope::Global,
                )
                .expect("parameter");

            let sixty_three = vec![character(font, 'a'); 63];
            let found = candidate(&mut stores, &sixty_three).expect("63-letter candidate");
            assert_eq!((found.word.len(), found.word_start, found.end), (63, 0, 63));

            let sixty_four = vec![character(font, 'a'); 64];
            let found = candidate(&mut stores, &sixty_four).expect("63-letter prefix at c64");
            assert_eq!((found.word.len(), found.word_start, found.end), (63, 0, 63));
        });
    }

    #[test]
    fn candidate_scan_is_linear_while_positional_probe_remains_explicit() {
        fn delta(
            after: tex_state::node_arena::NodeTraversalCounters,
            before: tex_state::node_arena::NodeTraversalCounters,
        ) -> tex_state::node_arena::NodeTraversalCounters {
            tex_state::node_arena::NodeTraversalCounters {
                index_resolutions: after
                    .index_resolutions
                    .saturating_sub(before.index_resolutions),
                index_predecessor_steps: after
                    .index_predecessor_steps
                    .saturating_sub(before.index_predecessor_steps),
                forward_chunk_crossings: after
                    .forward_chunk_crossings
                    .saturating_sub(before.forward_chunk_crossings),
            }
        }

        for values in [1_usize, 4_096] {
            crate::test_harness::with_nonstop_plain_universe(|universe| {
                let mut stores = universe.command_context().expect("test state is admitted");
                let font = stores.current_font();
                stores
                    .assign_int_param(
                        IntParam::DEFAULT_HYPHEN_CHAR,
                        i32::from(b'-'),
                        tex_state::AssignmentScope::Global,
                    )
                    .expect("parameter");
                let source = stores.publish_page_nodes(vec![character(font, 'a'); values]);
                let source = stores
                    .admit_page_node_list(source)
                    .expect("test paragraph source remains live");
                let nodes = stores
                    .admitted_page_nodes(source)
                    .expect("test paragraph span remains admitted");

                let before = nodes.testing_traversal_counters();
                let found = find_hyphenation_candidate(&stores, source, 0, (0, 0, 0))
                    .expect("lowercase source is a candidate");
                let candidate = delta(nodes.testing_traversal_counters(), before);
                assert_eq!(candidate.index_resolutions, 0);
                assert_eq!(candidate.index_predecessor_steps, 0);
                assert!(
                    !found.word.spilled(),
                    "TeX's 63-letter word bound is inline"
                );

                let reference_before = nodes.testing_traversal_counters();
                let mut visited = 0;
                nodes.for_each_range(0..nodes.len(), |_, _| visited += 1);
                let reference = delta(nodes.testing_traversal_counters(), reference_before);
                assert_eq!(visited, values);
                assert_eq!(
                    candidate.forward_chunk_crossings,
                    reference.forward_chunk_crossings
                );

                let probe_before = nodes.testing_traversal_counters();
                assert!(nodes.get(0).is_some());
                let positional = delta(nodes.testing_traversal_counters(), probe_before);
                assert_eq!(positional.index_resolutions, 1);
                if values == 4_096 {
                    assert!(positional.index_predecessor_steps > 0);
                }
                eprintln!(
                    "HYPHENATION_TRAVERSAL_SCALE values={values} sequential_index_resolutions={} sequential_predecessor_steps={} sequential_block_crossings={} positional_index_resolutions={} positional_predecessor_steps={}",
                    candidate.index_resolutions,
                    candidate.index_predecessor_steps,
                    candidate.forward_chunk_crossings,
                    positional.index_resolutions,
                    positional.index_predecessor_steps,
                );
            });
        }
    }

    #[test]
    fn outer_hyphenation_walk_reduces_indexed_reads_to_output_block_work() {
        fn delta(
            after: tex_state::node_arena::NodeTraversalCounters,
            before: tex_state::node_arena::NodeTraversalCounters,
        ) -> tex_state::node_arena::NodeTraversalCounters {
            tex_state::node_arena::NodeTraversalCounters {
                index_resolutions: after
                    .index_resolutions
                    .saturating_sub(before.index_resolutions),
                index_predecessor_steps: after
                    .index_predecessor_steps
                    .saturating_sub(before.index_predecessor_steps),
                forward_chunk_crossings: after
                    .forward_chunk_crossings
                    .saturating_sub(before.forward_chunk_crossings),
            }
        }

        for values in [1_usize, 4_096] {
            crate::test_harness::with_nonstop_plain_universe(|universe| {
                let mut stores = universe.command_context().expect("test state is admitted");
                let font = stores.current_font();
                let source = stores.publish_page_nodes(vec![character(font, 'a'); values]);
                let span = stores
                    .admit_page_node_span(source)
                    .expect("test paragraph source remains live");
                let nodes = stores
                    .page_node_span(span)
                    .expect("test paragraph span remains admitted");

                let reference_before = nodes.testing_traversal_counters();
                let mut reference_visits = 0;
                nodes.for_each_range(0..nodes.len(), |_, _| reference_visits += 1);
                let reference = delta(nodes.testing_traversal_counters(), reference_before);
                assert_eq!(reference_visits, values);

                let before = nodes.testing_traversal_counters();
                let _ = nodes;
                let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
                let mut post_overrides = Vec::new();
                let mut missing_hyphens = Vec::new();
                let mut projection = HyphenationProjection {
                    physical_post_overrides: &mut post_overrides,
                    missing_hyphens: &mut missing_hyphens,
                };
                let mut ledger =
                    tex_command::CommandFuelLedger::new(100_000).expect("bounded fuel");
                let mut scratch = crate::mode::HorizontalModeScratch::default();
                let (output, context) = hyphenated_hlist_with_projections(
                    &mut stores,
                    &mut effects,
                    source,
                    scratch.tfm_work_mut(),
                    ledger.fuel_mut(),
                    &mut projection,
                )
                .expect("direct outer hyphenation walk");
                let direct = delta(
                    stores
                        .page_node_span(span)
                        .expect("source remains live after traversal")
                        .testing_traversal_counters(),
                    before,
                );

                assert_eq!(output.len(), values);
                assert_eq!(context.language, 0);
                assert_eq!(
                    direct.index_resolutions, 0,
                    "direct shared copying performs no indexed source resolution"
                );
                assert_eq!(
                    direct.index_predecessor_steps, 0,
                    "direct shared copying follows stable chunk coordinates"
                );
                assert_eq!(
                    direct.forward_chunk_crossings,
                    reference.forward_chunk_crossings
                );
                assert!(post_overrides.is_empty());
                assert!(missing_hyphens.is_empty());
                eprintln!(
                    "HYPHENATION_OUTER_CHUNK_SCALE values={values} index_resolutions={} index_predecessor_steps={} forward_block_crossings={}",
                    direct.index_resolutions,
                    direct.index_predecessor_steps,
                    direct.forward_chunk_crossings,
                );
            });
        }
    }
}

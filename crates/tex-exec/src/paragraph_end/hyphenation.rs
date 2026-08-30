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
    fn from_node(node: &Node) -> Self {
        match node {
            Node::Whatsit(tex_state::node::Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Self::Language {
                language: *language,
                left: usize::from((*left_hyphen_min).max(1)),
                right: usize::from((*right_hyphen_min).max(1)),
            },
            Node::MathOn(_) => Self::MathOn,
            Node::MathOff(_) => Self::MathOff,
            Node::Glue { .. } => Self::Glue,
            _ => Self::Other,
        }
    }
}

struct HyphenationWalk<'walk, 'projection> {
    out: &'walk mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &'walk mut Vec<tex_state::node_arena::PageListId>,
    generated_word: &'walk mut Vec<Node>,
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
        source: tex_state::page_node_arena::PageListSpan,
        chunk: tex_state::page_node_arena::PageListChunkCursor,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Result<(), ExecError> {
        if let Some(previous) = stores
            .page_node_span_previous_chunk(source, &chunk)
            .expect("hyphenation source chunk remains live")
        {
            self.visit_chunk_prefix(stores, source, previous, diagnostic_effects)?;
        }
        self.visit_chunk(stores, source, &chunk, diagnostic_effects)
    }

    #[inline(never)]
    fn visit_chunk<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::PageListSpan,
        chunk: &tex_state::page_node_arena::PageListChunkCursor,
        diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    ) -> Result<(), ExecError> {
        for offset in 0..chunk.len() {
            let index = chunk.logical_start() + offset;
            if index < self.skip_until {
                continue;
            }
            let observed = {
                let (resolved, node) = stores
                    .page_node_span_chunk_node(source, chunk, offset)
                    .expect("hyphenation source chunk remains live");
                debug_assert_eq!(resolved, index);
                HyphenationScanNode::from_node(node)
            };
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
                        source,
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
                            self.generated_word,
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
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<(tex_state::node_arena::PageListId, HyphenationContext), ExecError> {
    // TeX82 §919 initializes the trie on entry to the first hyphenation pass,
    // even when this particular paragraph ultimately supplies no candidate.
    stores.close_hyphenation_patterns();
    let source = stores
        .admit_page_node_span(source)
        .expect("hyphenation source crosses one live page-region boundary");
    let mut out = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut out);
    let mut out_segments = Vec::new();
    let mut generated_word = Vec::new();
    let left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize;
    let right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize;
    let (retained_start, language, left, right) = {
        let mut walk = HyphenationWalk {
            out: &mut out,
            out_segments: &mut out_segments,
            generated_word: &mut generated_word,
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
            .page_node_span_tail_chunk(source)
            .expect("hyphenation source remains admitted")
        {
            walk.visit_chunk_prefix(stores, source, tail, diagnostic_effects)?;
        }
        (walk.retained_start, walk.language, walk.left, walk.right)
    };
    if retained_start < source.len() {
        stores.append_page_active_span_range(&mut out, source, retained_start..source.len());
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
    fuel: &mut tex_command::CommandFuel,
) -> Result<HyphenatedHlist, ExecError> {
    let mut physical_post_overrides = Vec::new();
    let mut missing_hyphens = Vec::new();
    let mut projection = HyphenationProjection {
        physical_post_overrides: &mut physical_post_overrides,
        missing_hyphens: &mut missing_hyphens,
    };
    let (semantic, _) = hyphenated_hlist_with_projections(
        stores,
        diagnostic_effects,
        source,
        fuel,
        &mut projection,
    )?;
    let physical = project_physical_hlist(
        stores,
        diagnostic_effects,
        semantic,
        &physical_post_overrides,
        fuel,
    )?;
    let mut shaping_chars = Vec::new();
    let mut shaping_scratch = crate::box_runtime::hmode::OpenTypeShapingScratch::default();
    let semantic = crate::box_runtime::hmode::reshape_open_type_runs_list(
        stores,
        semantic,
        &mut shaping_chars,
        &mut shaping_scratch,
    );
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
    fuel: &mut tex_command::CommandFuel,
) -> Result<tex_state::node_arena::PageListId, ExecError> {
    let semantic = stores
        .admit_page_node_span(semantic)
        .expect("hyphenated paragraph crosses one live page-region boundary");
    let mut physical = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    stores.open_page_active_list(&mut physical);
    let mut physical_segments = Vec::new();
    let mut override_index = 0usize;
    let mut retained_start = 0usize;
    if let Some(tail) = stores
        .page_node_span_tail_chunk(semantic)
        .expect("hyphenated paragraph remains admitted")
    {
        let _ = project_physical_chunk_prefix(
            stores,
            diagnostic_effects,
            semantic,
            tail,
            post_overrides,
            &mut override_index,
            &mut physical,
            &mut physical_segments,
            &mut retained_start,
            fuel,
        )?;
    }
    if retained_start < semantic.len() {
        stores.append_page_active_span_range(
            &mut physical,
            semantic,
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
    semantic: tex_state::page_node_arena::PageListSpan,
    chunk: tex_state::page_node_arena::PageListChunkCursor,
    post_overrides: &[(usize, tex_state::node_arena::PageListId)],
    override_index: &mut usize,
    physical: &mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    physical_segments: &mut Vec<tex_state::node_arena::PageListId>,
    retained_start: &mut usize,
    fuel: &mut tex_command::CommandFuel,
) -> Result<tex_state::page_node_arena::PageListChunkCursor, ExecError> {
    let previous = stores
        .page_node_span_previous_chunk(semantic, &chunk)
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
        let pre_pending =
            physical_pre_break_pending(stores, semantic, previous.as_ref(), &chunk, offset);
        if (post_override.is_some() || pre_pending.is_some()) && *retained_start < index {
            stores.append_page_active_span_range(physical, semantic, *retained_start..index);
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
            let pre = crate::box_runtime::hmode::reconstitute_with_fuel(
                stores,
                diagnostic_effects,
                &pending,
                true,
                false,
                fuel,
            )
            .map_err(ExecError::Command)?;
            let pre = stores.publish_page_nodes(pre);
            stores.open_page_active_list(physical);
            Some(pre)
        } else {
            None
        };
        if post_override.is_some() || pre_projection.is_some() {
            let mut replacement = {
                let (resolved, node) = stores
                    .page_node_span_chunk_node(semantic, &chunk, offset)
                    .expect("hyphenated paragraph source chunk remains live");
                debug_assert_eq!(resolved, index);
                node.clone()
            };
            let Node::Disc { pre, post, .. } = &mut replacement else {
                unreachable!("physical projection targets a discretionary")
            };
            if let Some(projected) = post_override {
                *post = projected;
                *override_index += 1;
            }
            if let Some(projected) = pre_projection {
                *pre = projected;
            }
            stores.push_page_active_list(physical, replacement);
            *retained_start = index + 1;
        }
    }
    Ok(chunk)
}

fn physical_pre_break_pending<G>(
    stores: &CommandContext<'_, G>,
    semantic: tex_state::page_node_arena::PageListSpan,
    previous_chunk: Option<&tex_state::page_node_arena::PageListChunkCursor>,
    chunk: &tex_state::page_node_arena::PageListChunkCursor,
    offset: usize,
) -> Option<Vec<PendingHChar>> {
    let index = chunk.logical_start() + offset;
    if index == 0 {
        return None;
    }
    let qualifies = {
        let (resolved, node) = stores
            .page_node_span_chunk_node(semantic, chunk, offset)
            .ok()?;
        debug_assert_eq!(resolved, index);
        let Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            replace,
            physical_replace_count: 2,
            ..
        } = node
        else {
            return None;
        };
        let replacement = stores
            .page_node_list(*replace)
            .expect("discretionary replacement belongs to the live page arena");
        replacement.len() == 1
            && matches!(
                replacement.first(),
                Some(Node::Kern {
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
        let (resolved, node) = stores
            .page_node_span_chunk_node(semantic, previous, previous_offset)
            .ok()?;
        debug_assert_eq!(resolved, index - 1);
        match node {
            Node::Char { font, ch, origin } => (
                *font,
                vec![PendingHChar {
                    font: *font,
                    ch: *ch,
                    origin: *origin,
                }],
            ),
            Node::Lig {
                font,
                orig,
                origins,
                ..
            } => (
                *font,
                orig.iter()
                    .copied()
                    .zip(origins.iter().copied())
                    .map(|(ch, origin)| PendingHChar {
                        font: *font,
                        ch,
                        origin,
                    })
                    .collect(),
            ),
            _ => return None,
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
        .admit_page_node_span(semantic)
        .expect("shaped paragraph crosses one live page-region boundary");
    let nodes = stores
        .page_node_span(semantic)
        .expect("shaped paragraph belongs to the live page arena");
    let mut boundary = 0usize;
    let mut boundaries = Vec::with_capacity(nodes.len() + 1);
    boundaries.push(0);
    nodes.nodes().for_each(|node| {
        boundary = boundary.saturating_add(match node {
            Node::Lig { orig, .. } => orig.len().max(1),
            _ => 1,
        });
        boundaries.push(boundary.min(physical_len));
    });
    if let Some(last) = boundaries.last_mut() {
        *last = physical_len;
    }
    boundaries
}

fn update_hyphenation_context(node: &Node, language: &mut u8, left: &mut usize, right: &mut usize) {
    if let Node::Whatsit(tex_state::node::Whatsit::Language {
        language: new_language,
        left_hyphen_min,
        right_hyphen_min,
    }) = node
    {
        *language = *new_language;
        *left = usize::from((*left_hyphen_min).max(1));
        *right = usize::from((*right_hyphen_min).max(1));
    }
}

#[allow(clippy::too_many_arguments)] // Hyphenation traversal keeps cursor, fuel, and projection state independent.
fn hyphenate_candidate_after_glue<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    source: tex_state::page_node_arena::PageListSpan,
    start: usize,
    candidate: HyphenationCandidate,
    out: &mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    out_segments: &mut Vec<tex_state::node_arena::PageListId>,
    generated_word: &mut Vec<Node>,
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
        stores.append_page_active_span_range(out, source, start..index);
        *output_len += index - start;
        return Ok(index);
    }

    stores.append_page_active_span_range(out, source, start..word_start);
    *output_len += word_start - start;
    let trailing_font_kern = stores
        .page_node_span(source)
        .expect("hyphenation source belongs to the live page arena")
        .owned_node(index - 1)
        .is_some_and(|node| {
            matches!(
                node,
                Node::Kern {
                    kind: KernKind::Font,
                    ..
                }
            )
        });
    let no_left_boundary = word_start != 0
        && stores
            .page_node_span(source)
            .expect("hyphenation source belongs to the live page arena")
            .owned_node(word_start - 1)
            .is_some_and(|node| {
                matches!(
                    node,
                    Node::Kern {
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
    generated_word.clear();
    append_hyphenated_word(
        stores,
        diagnostic_effects,
        &word,
        &positions,
        no_left_boundary,
        right_boundary,
        generated_word,
        output_len,
        fuel,
        projection,
    )?;
    stores.open_page_active_list(out);
    for node in generated_word.drain(..) {
        stores.push_page_active_list(out, node);
    }
    if trailing_font_kern && !matches!(right_boundary, HyphenationRightBoundary::Character(_)) {
        stores.append_page_active_span_range(out, source, index - 1..index);
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
    source: tex_state::page_node_arena::PageListSpan,
    start: usize,
    context: (u8, usize, usize),
) -> Option<HyphenationCandidate> {
    let nodes = stores.page_node_span(source).ok()?;
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
            match first_word_char(stores, language, node) {
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
                None if is_pre_word_skip(node) => {
                    update_hyphenation_context(node, &mut language, &mut left, &mut right);
                    return core::ops::ControlFlow::Continue(());
                }
                None => return core::ops::ControlFlow::Break(false),
            }
        }

        if word_end.is_none() {
            let candidate_font = font.expect("word start establishes its font");
            let continues_word = match node {
                Node::Char {
                    font: node_font,
                    ch,
                    origin,
                } if *node_font == candidate_font => {
                    if word.len() < 63
                        && let Some(lower) = normalized_hyphen_code(stores, language, *ch)
                    {
                        word.push(WordChar {
                            font: candidate_font,
                            ch: *ch,
                            lower,
                            origin: *origin,
                        });
                        right_boundary = HyphenationRightBoundary::None;
                        true
                    } else {
                        right_boundary = u8::try_from(*ch as u32).map_or(
                            HyphenationRightBoundary::None,
                            HyphenationRightBoundary::Character,
                        );
                        false
                    }
                }
                Node::Lig {
                    font: node_font,
                    orig,
                    origins,
                    right_hit,
                    ..
                } if *node_font == candidate_font => {
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
                    } else if *right_hit {
                        right_boundary = HyphenationRightBoundary::Font;
                    } else {
                        right_boundary = HyphenationRightBoundary::None;
                    }
                    valid
                }
                Node::Kern {
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
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Kern {
                kind: KernKind::Font,
                ..
            } => core::ops::ControlFlow::Continue(()),
            Node::Glue { .. }
            | Node::Penalty(_)
            | Node::Ins { .. }
            | Node::Adjust(_)
            | Node::Mark { .. }
            | Node::Whatsit(_)
            | Node::Direction(
                tex_state::node::Direction::BeginL
                | tex_state::node::Direction::EndL
                | tex_state::node::Direction::BeginR
                | tex_state::node::Direction::EndR,
            )
            | Node::Kern { .. } => core::ops::ControlFlow::Break(true),
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
    node: &Node,
) -> Option<(tex_state::ids::FontId, char, char)> {
    match node {
        Node::Char { font, ch, .. } => {
            normalized_hyphen_code(stores, language, *ch).map(|lower| (*font, *ch, lower))
        }
        Node::Lig { font, orig, .. } => orig.first().and_then(|&first| {
            normalized_hyphen_code(stores, language, first).map(|lower| (*font, first, lower))
        }),
        _ => None,
    }
}

fn is_pre_word_skip(node: &Node) -> bool {
    matches!(
        node,
        Node::Kern {
            kind: KernKind::Font,
            ..
        } | Node::Whatsit(_)
            | Node::Direction(
                tex_state::node::Direction::BeginL
                    | tex_state::node::Direction::EndL
                    | tex_state::node::Direction::BeginR
                    | tex_state::node::Direction::EndR
            )
    ) || matches!(node, Node::Char { .. } | Node::Lig { .. })
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

fn reconstitute_hyphenated_span<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    pending: &[PendingHChar],
    no_left_boundary: bool,
    right_boundary: HyphenationRightBoundary,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, ExecError> {
    let result = match right_boundary {
        HyphenationRightBoundary::None => {
            crate::box_runtime::hmode::reconstitute_with_right_character(
                stores,
                diagnostic_effects,
                pending,
                no_left_boundary,
                None,
                fuel,
            )
        }
        HyphenationRightBoundary::Font => crate::box_runtime::hmode::reconstitute_with_fuel(
            stores,
            diagnostic_effects,
            pending,
            no_left_boundary,
            false,
            fuel,
        ),
        HyphenationRightBoundary::Character(ch) => {
            crate::box_runtime::hmode::reconstitute_with_right_character(
                stores,
                diagnostic_effects,
                pending,
                no_left_boundary,
                Some(ch),
                fuel,
            )
        }
    };
    result.map_err(ExecError::Command)
}

#[allow(clippy::too_many_arguments)] // Word reconstruction carries independent boundary and projection state.
fn append_hyphenated_word<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    positions: &[usize],
    no_left_boundary: bool,
    right_boundary: HyphenationRightBoundary,
    out: &mut Vec<Node>,
    output_len: &mut usize,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<(), ExecError> {
    let pending: Vec<_> = word.iter().map(WordChar::pending).collect();
    let nodes = reconstitute_hyphenated_span(
        stores,
        diagnostic_effects,
        &pending,
        no_left_boundary,
        right_boundary,
        fuel,
    )?;
    let mut position_index = 0;
    let mut char_start = 0;

    for (node_index, node) in nodes.iter().cloned().enumerate() {
        let boundary_kern = matches!(
            node,
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        ) && positions.get(position_index) == Some(&char_start);
        while positions.get(position_index) == Some(&char_start) {
            let replacement = boundary_kern.then_some(node.clone());
            let disc = discretionary_hyphen(
                stores,
                word[char_start - 1].font,
                replacement,
                *output_len,
                projection.missing_hyphens,
            );
            out.push(disc);
            *output_len += 1;
            position_index += 1;
        }
        if boundary_kern {
            continue;
        }

        let char_end = char_start + node_original_len(&node);
        if let Some(&position) = positions
            .get(position_index)
            .filter(|&&position| char_start < position && position < char_end)
        {
            let (disc, physical_post) = discretionary_through_node(
                stores,
                diagnostic_effects,
                word,
                ((char_start, position, char_end), *output_len),
                node,
                &nodes[node_index + 1..],
                right_boundary,
                fuel,
                projection.missing_hyphens,
            )?;
            projection
                .physical_post_overrides
                .push((*output_len, physical_post));
            out.push(disc);
            *output_len += 1;
            position_index += 1;
            // TeX82 likewise suppresses another hyphenation point whose
            // branches have not synchronized before this node ends.
            while positions
                .get(position_index)
                .is_some_and(|&next| next < char_end)
            {
                position_index += 1;
            }
        } else {
            out.push(node);
            *output_len += 1;
        }
        char_start = char_end;
    }

    while let Some(&position) = positions.get(position_index) {
        debug_assert_eq!(position, char_start);
        let disc = discretionary_hyphen(
            stores,
            word[position - 1].font,
            None,
            *output_len,
            projection.missing_hyphens,
        );
        out.push(disc);
        *output_len += 1;
        position_index += 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Discretionary reconstruction mirrors TeX's source and replacement coordinates.
fn discretionary_through_node<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    location: ((usize, usize, usize), usize),
    replacement: Node,
    following: &[Node],
    right_boundary: HyphenationRightBoundary,
    fuel: &mut tex_command::CommandFuel,
    missing_hyphens: &mut Vec<MissingHyphenDiagnostic>,
) -> Result<(Node, tex_state::node_arena::PageListId), ExecError> {
    let (span, node_index) = location;
    let (start, position, end) = span;
    let font = word[position - 1].font;
    let mut pre_pending: Vec<_> = word[start..position]
        .iter()
        .map(WordChar::pending)
        .collect();
    if let Some(ch) = automatic_hyphen_char(stores, font, node_index, missing_hyphens) {
        pre_pending.push(PendingHChar {
            font,
            ch,
            origin: word[position - 1].origin,
        });
    }
    let pre = crate::box_runtime::hmode::reconstitute_with_fuel(
        stores,
        diagnostic_effects,
        &pre_pending,
        true,
        false,
        fuel,
    )
    .map_err(ExecError::Command)?;
    let post_pending: Vec<_> = word[position..end].iter().map(WordChar::pending).collect();
    let post = reconstitute_hyphenated_span(
        stores,
        diagnostic_effects,
        &post_pending,
        false,
        right_boundary,
        fuel,
    )?;

    let (physical_replace_count, physical_post) = physical_discretionary_projection(
        stores,
        diagnostic_effects,
        word,
        span,
        PhysicalDiscretionarySource {
            replacement: &replacement,
            following,
            right_boundary,
        },
        fuel,
    )?;
    let disc = automatic_discretionary_with_count(
        stores,
        &pre,
        &post,
        &[replacement],
        physical_replace_count,
    )
    .expect("a reconstituted word has a bounded physical replacement span");
    Ok((disc, physical_post))
}

/// Replays TeX82 §§914--918's synchronization rule in character space.
///
/// Umber's semantic channel stores a ligature together with its source
/// characters, whereas TeX counts the linked nodes produced by successive
/// `reconstitute` calls.  The two branches synchronize at the first source
/// character boundary represented by both reconstitutions.  Projecting that
/// boundary here retains TeX's exact replacement count and post-break list
/// without flattening the semantic ligature used by packing and shipout.
struct PhysicalDiscretionarySource<'a> {
    replacement: &'a Node,
    following: &'a [Node],
    right_boundary: HyphenationRightBoundary,
}

fn physical_discretionary_projection<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    span: (usize, usize, usize),
    source: PhysicalDiscretionarySource<'_>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(u8, tex_state::node_arena::PageListId), ExecError> {
    let PhysicalDiscretionarySource {
        replacement,
        following,
        right_boundary,
    } = source;
    let (start, position, end) = span;
    let mut major = Vec::with_capacity(following.len() + 1);
    major.push(replacement.clone());
    major.extend_from_slice(following);
    let minor_pending = word[position..]
        .iter()
        .map(WordChar::pending)
        .collect::<Vec<_>>();
    let minor = reconstitute_hyphenated_span(
        stores,
        diagnostic_effects,
        &minor_pending,
        false,
        right_boundary,
        fuel,
    )?;

    let (major_len, minor_len) =
        synchronized_physical_branch_lengths(&major, start, &minor, position, end, word.len());
    let physical_replace_count = u8::try_from(major_len)
        .ok()
        .filter(|&count| count <= 127)
        .expect("a TeX word has at most 127 physical replacement nodes");
    Ok((
        physical_replace_count,
        stores.publish_page_nodes(minor[..minor_len].to_vec()),
    ))
}

fn physical_character_boundaries(nodes: &[Node], start: usize) -> Vec<usize> {
    let mut boundary = start;
    nodes
        .iter()
        .map(|node| {
            boundary = boundary.saturating_add(node_original_len(node));
            boundary
        })
        .collect()
}

fn nodes_through_character_boundary(boundaries: &[usize], synchronization: usize) -> usize {
    boundaries
        .iter()
        .take_while(|&&boundary| boundary <= synchronization)
        .count()
}

fn synchronized_physical_branch_lengths(
    major: &[Node],
    major_start: usize,
    minor: &[Node],
    minor_start: usize,
    initial_end: usize,
    word_len: usize,
) -> (usize, usize) {
    let major_boundaries = physical_character_boundaries(major, major_start);
    let minor_boundaries = physical_character_boundaries(minor, minor_start);
    let synchronization = major_boundaries
        .iter()
        .copied()
        .filter(|&boundary| boundary >= initial_end)
        .find(|boundary| minor_boundaries.contains(boundary))
        .unwrap_or(word_len);
    (
        nodes_through_character_boundary(&major_boundaries, synchronization),
        nodes_through_character_boundary(&minor_boundaries, synchronization),
    )
}

fn automatic_discretionary_with_count<G>(
    stores: &mut CommandContext<'_, G>,
    pre: &[Node],
    post: &[Node],
    replace: &[Node],
    physical_replace_count: u8,
) -> Option<Node> {
    (physical_replace_count <= 127).then(|| {
        let pre = stores.publish_page_nodes(pre.to_vec());
        let post = stores.publish_page_nodes(post.to_vec());
        let replace = stores.publish_page_nodes(replace.to_vec());
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre,
            post,
            replace,
            physical_replace_count,
        }
    })
}

/// TeX82 §§904/914/918 counts the physical nodes between an automatic
/// discretionary and the reconstitution synchronization point. Umber keeps
/// a ligature and its boundary kern structured, so recover that pre-collapse
/// physical span at the construction boundary where the distinction is known.
fn automatic_physical_replace_count(replace: &[Node]) -> Option<u8> {
    let count = match replace {
        [
            Node::Kern {
                kind: KernKind::Font,
                ..
            },
        ] => 2,
        [Node::Lig { .. }] => 1,
        _ => replace.len(),
    };
    u8::try_from(count).ok().filter(|&count| count <= 127)
}

fn node_original_len(node: &Node) -> usize {
    match node {
        Node::Char { .. } => 1,
        Node::Lig { orig, .. } => orig.len(),
        Node::Kern { .. } => 0,
        _ => 0,
    }
}

fn discretionary_hyphen<G>(
    stores: &mut CommandContext<'_, G>,
    font: tex_state::ids::FontId,
    replacement: Option<Node>,
    node_index: usize,
    missing_hyphens: &mut Vec<MissingHyphenDiagnostic>,
) -> Node {
    let empty = tex_state::node_arena::PageListId::empty();
    let pre = automatic_hyphen_char(stores, font, node_index, missing_hyphens).map_or_else(
        || empty,
        |ch| {
            stores.publish_page_nodes(vec![Node::Char {
                font,
                ch,
                origin: OriginId::UNKNOWN,
            }])
        },
    );
    let replace = replacement.as_ref().map_or_else(
        || empty,
        |node| stores.publish_page_nodes(vec![node.clone()]),
    );
    Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre,
        post: empty,
        replace,
        physical_replace_count: replacement.as_ref().map_or(0, |node| {
            automatic_physical_replace_count(std::slice::from_ref(node))
                .expect("one reconstituted node fits TeX82's replacement count")
        }),
    }
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

#[derive(Clone)]
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
use tex_state::node::{DiscKind, KernKind, Node};
use tex_state::token::OriginId;

use crate::ExecError;
use crate::mode::PendingHChar;

#[cfg(test)]
mod tests {
    use super::*;

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
            .admit_page_node_span(source)
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
        characters[usize::from(b'd')]
            .as_mut()
            .expect("test letter exists")
            .tag = tex_state::font::CharTag::LigKern {
            program_index: 0,
            start_index: 0,
        };
        let program = vec![tex_fonts::LigKernInstruction {
            skip_byte: 128,
            next_char: b'.',
            command: Some(tex_fonts::LigKernCommand::Kern(
                tex_state::scaled::Scaled::from_raw(-1_234),
            )),
        }];
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
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");

            let hyphenated =
                hyphenated_hlist_with_fuel(&mut stores, &mut effects, source, fuel.fuel_mut())
                    .expect("automatic discretionary construction succeeds");
            let semantic = stores
                .page_nodes(hyphenated.semantic)
                .expect("semantic list");
            let disc = semantic
                .iter()
                .find(|node| matches!(node, Node::Disc { .. }))
                .expect("exception inserts a discretionary");
            let Node::Disc { pre, .. } = disc else {
                unreachable!()
            };
            assert!(matches!(
                stores
                    .page_node_list(*pre)
                    .expect("pre-break child remains live")
                    .first(),
                Some(Node::Char { ch: '-', .. })
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
    fn unhyphenated_word_keeps_the_single_main_list_owner() {
        crate::test_harness::with_nonstop_plain_universe(|universe| {
            let mut stores = universe.command_context().expect("test state is admitted");
            let font = hyphenation_font(&mut stores);
            stores.set_font_hyphen_char(font, i32::from(b'-'));
            let source = hyphenation_source(&mut stores, font);
            let mut effects = tex_state::diagnostic::DiagnosticEffects::new();
            let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");

            let unchanged =
                hyphenated_hlist_with_fuel(&mut stores, &mut effects, source, fuel.fuel_mut())
                    .expect("word without an allowed position succeeds");
            assert!(
                stores
                    .page_nodes(unchanged.semantic)
                    .expect("semantic list")
                    .iter()
                    .all(|node| !matches!(node, Node::Disc { .. })),
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
                let mut fuel = tex_command::CommandFuelLedger::new(10_000).expect("bounded fuel");
                let hyphenated =
                    hyphenated_hlist_with_fuel(&mut stores, &mut effects, source, fuel.fuel_mut())
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
            let tokens =
                tex_state::node::NodeTokenList::new(vec![tex_state::token::TokenWord::pack(
                    tex_state::token::Token::Char {
                        ch: 'w',
                        cat: tex_state::token::Catcode::Letter,
                    },
                )]);
            let nodes = vec![
                Node::Whatsit(tex_state::node::Whatsit::OpenOut {
                    slot: tex_state::StreamSlot::new(15),
                    path: "visit.tex".into(),
                }),
                Node::Whatsit(tex_state::node::Whatsit::DeferredWrite {
                    sink: tex_state::PrintSink::Log,
                    tokens: tokens.clone(),
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
            let visited = hyphenated_hlist_with_fuel(
                &mut stores,
                &mut diagnostic_effects,
                source,
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
                tokens.words(),
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
                    .admit_page_node_span(source)
                    .expect("test paragraph source remains live");
                let nodes = stores
                    .page_node_span(source)
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
                assert!(nodes.owned_node(0).is_some());
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
                let (output, context) = hyphenated_hlist_with_projections(
                    &mut stores,
                    &mut effects,
                    source,
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
                    direct.index_resolutions, 1,
                    "only the existing output-range traversal resolves an index"
                );
                assert_eq!(
                    direct.index_predecessor_steps,
                    reference.forward_chunk_crossings
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

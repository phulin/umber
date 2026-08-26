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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HyphenationContext {
    language: u8,
    left: usize,
    right: usize,
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
    nodes: Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<(Vec<Node>, HyphenationContext), ExecError> {
    // TeX82 §919 initializes the trie on entry to the first hyphenation pass,
    // even when this particular paragraph ultimately supplies no candidate.
    stores.close_hyphenation_patterns();
    let mut out: Option<Vec<Node>> = None;
    let mut index = 0;
    let mut auto_breaking = true;
    let mut language = 0;
    let mut left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize;
    let mut right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize;

    while index < nodes.len() {
        let node = &nodes[index];
        update_hyphenation_context(node, &mut language, &mut left, &mut right);
        match node {
            Node::MathOn(_) => auto_breaking = false,
            Node::MathOff(_) => auto_breaking = true,
            _ => {}
        }
        if let Some(out) = &mut out {
            out.push(node.clone());
        }
        index += 1;

        if auto_breaking
            && matches!(node, Node::Glue { .. })
            && let Some(next) = hyphenate_after_glue(
                stores,
                diagnostic_effects,
                &nodes,
                index,
                (language, left, right),
                &mut out,
                fuel,
                projection,
            )?
        {
            index = next;
        }
    }
    Ok((
        out.unwrap_or(nodes),
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
pub(crate) fn hyphenated_hlist_sequence_with_fuel<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    nodes: Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<
    (
        tex_state::node_sequence::NodeSequence,
        Vec<MissingHyphenDiagnostic>,
    ),
    ExecError,
> {
    let mut physical_post_overrides = Vec::new();
    let mut missing_hyphens = Vec::new();
    let mut projection = HyphenationProjection {
        physical_post_overrides: &mut physical_post_overrides,
        missing_hyphens: &mut missing_hyphens,
    };
    let (semantic, _) = hyphenated_hlist_with_projections(
        stores,
        diagnostic_effects,
        nodes,
        fuel,
        &mut projection,
    )?;
    let mut physical = semantic.clone();
    for (index, physical_post) in physical_post_overrides {
        if let Some(Node::Disc { post, .. }) = physical.get_mut(index) {
            *post = physical_post;
        }
    }
    project_physical_pre_break_spans(stores, diagnostic_effects, &mut physical, fuel)?;
    Ok((
        tex_state::node_sequence::NodeSequence::from_channels(semantic, physical),
        missing_hyphens,
    ))
}

fn project_physical_pre_break_spans<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    nodes: &mut [Node],
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    for index in 1..nodes.len() {
        let Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            replace,
            physical_replace_count: 2,
            ..
        } = &nodes[index]
        else {
            continue;
        };
        let replacement = stores
            .page_node_list(*replace)
            .expect("discretionary replacement belongs to the live page arena");
        if replacement.len() != 1
            || !matches!(
                replacement.first(),
                Some(tex_state::node_arena::NodeRef::Kern {
                    kind: KernKind::Font,
                    ..
                })
            )
        {
            continue;
        }

        let (font, chars, origins) = match &nodes[index - 1] {
            Node::Char { font, ch, origin } => (*font, vec![*ch], vec![*origin]),
            Node::Lig {
                font,
                orig,
                origins,
                ..
            } => (*font, orig.clone(), origins.clone()),
            _ => continue,
        };
        let Some(hyphen) = usable_hyphen_char(stores, font) else {
            continue;
        };
        let last_origin = origins.last().copied().unwrap_or(OriginId::UNKNOWN);
        let mut pending = chars
            .into_iter()
            .zip(origins)
            .map(|(ch, origin)| PendingHChar { font, ch, origin })
            .collect::<Vec<_>>();
        pending.push(PendingHChar {
            font,
            ch: hyphen,
            origin: last_origin,
        });
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
        let Node::Disc {
            pre: physical_pre, ..
        } = &mut nodes[index]
        else {
            unreachable!()
        };
        *physical_pre = pre;
    }
    Ok(())
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
fn hyphenate_after_glue<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    nodes: &[Node],
    start: usize,
    context: (u8, usize, usize),
    out: &mut Option<Vec<Node>>,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<Option<usize>, ExecError> {
    let Some(candidate) = find_hyphenation_candidate(stores, nodes, start, context) else {
        return Ok(None);
    };
    let HyphenationCandidate {
        word_start,
        end: index,
        language,
        left,
        right,
        word,
    } = candidate;

    let lowercase: String = word.iter().map(|ch| ch.lower).collect();
    let positions = stores.hyphen_positions_for_language(language, &lowercase, left, right);
    if positions.is_empty() {
        if let Some(out) = out {
            out.extend_from_slice(&nodes[start..index]);
        }
        return Ok(Some(index));
    }

    let out = out.get_or_insert_with(|| {
        let mut out = Vec::with_capacity(nodes.len());
        out.extend_from_slice(&nodes[..start]);
        out
    });
    out.extend_from_slice(&nodes[start..word_start]);
    let trailing_font_kern = nodes[word_start..index].last().and_then(|node| match node {
        Node::Kern {
            amount,
            kind: KernKind::Font,
        } => Some(Node::Kern {
            amount: *amount,
            kind: KernKind::Font,
        }),
        _ => None,
    });
    let no_left_boundary = matches!(
        out.last(),
        Some(Node::Kern {
            kind: KernKind::Font,
            ..
        })
    );
    append_hyphenated_word(
        stores,
        diagnostic_effects,
        &word,
        &positions,
        no_left_boundary,
        out,
        fuel,
        projection,
    )?;
    if let Some(kern) = trailing_font_kern {
        out.push(kern);
    }
    Ok(Some(index))
}

struct HyphenationCandidate {
    word_start: usize,
    end: usize,
    language: u8,
    left: usize,
    right: usize,
    word: Vec<WordChar>,
}

/// Implements TeX82 §§891--895's bounded scan from the glue after which a
/// potentially hyphenatable part begins through the same-font letter span.
fn find_hyphenation_candidate<G>(
    stores: &CommandContext<'_, G>,
    nodes: &[Node],
    start: usize,
    context: (u8, usize, usize),
) -> Option<HyphenationCandidate> {
    let (mut language, mut left, mut right) = context;
    let mut index = start;
    let (word_start, font) = loop {
        let node = nodes.get(index)?;
        match first_word_char(stores, language, node) {
            Some((font, ch, lower)) => {
                if lower != ch && stores.int_param(IntParam::UC_HYPH) <= 0 {
                    return None;
                }
                break (index, font);
            }
            None if is_pre_word_skip(node) => {
                update_hyphenation_context(node, &mut language, &mut left, &mut right);
                index += 1;
            }
            None => return None,
        }
    };

    let minima = left.checked_add(right)?;
    if minima > 63 {
        return None;
    }
    let hyphen = stores.font_hyphen_char(font);
    if !(0..=255).contains(&hyphen) {
        return None;
    }

    let mut word = Vec::new();
    index = word_start;
    while let Some(node) = nodes.get(index) {
        match node {
            Node::Char {
                font: node_font,
                ch,
                origin,
            } if *node_font == font && word.len() < 63 => {
                let Some(lower) = normalized_hyphen_code(stores, language, *ch) else {
                    break;
                };
                word.push(WordChar {
                    font,
                    ch: *ch,
                    lower,
                    origin: *origin,
                });
                index += 1;
            }
            Node::Lig {
                font: node_font,
                ch,
                orig,
                origins,
                ..
            } if *node_font == font => {
                let chars = orig.clone();
                if word
                    .len()
                    .checked_add(chars.len())
                    .is_none_or(|len| len > 63)
                {
                    break;
                }
                let Some(normalized) = chars
                    .into_iter()
                    .map(|ch| normalized_hyphen_code(stores, language, ch).map(|lower| (ch, lower)))
                    .collect::<Option<Vec<_>>>()
                else {
                    break;
                };
                for ((ch, lower), origin) in normalized.into_iter().zip(origins.iter().cloned()) {
                    word.push(WordChar {
                        font,
                        ch,
                        lower,
                        origin,
                    });
                }
                index += 1;
            }
            Node::Kern {
                kind: KernKind::Font,
                ..
            } => {
                index += 1;
            }
            _ => break,
        }
    }

    if word.len() < minima || !permitted_word_terminator(nodes, index) {
        return None;
    }
    Some(HyphenationCandidate {
        word_start,
        end: index,
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

fn permitted_word_terminator(nodes: &[Node], mut index: usize) -> bool {
    while let Some(node) = nodes.get(index) {
        match node {
            Node::Char { .. }
            | Node::Lig { .. }
            | Node::Kern {
                kind: KernKind::Font,
                ..
            } => index += 1,
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
            | Node::Kern { .. } => return true,
            _ => return false,
        }
    }
    true
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

#[allow(clippy::too_many_arguments)] // Word reconstruction carries independent boundary and projection state.
fn append_hyphenated_word<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    positions: &[usize],
    no_left_boundary: bool,
    out: &mut Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
    projection: &mut HyphenationProjection<'_>,
) -> Result<(), ExecError> {
    let pending: Vec<_> = word.iter().map(WordChar::pending).collect();
    let nodes = crate::box_runtime::hmode::reconstitute_with_fuel(
        stores,
        diagnostic_effects,
        &pending,
        no_left_boundary,
        false,
        fuel,
    )
    .map_err(ExecError::Command)?;
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
            out.push(discretionary_hyphen(
                stores,
                word[char_start - 1].font,
                replacement,
                out.len(),
                projection.missing_hyphens,
            ));
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
                ((char_start, position, char_end), out.len()),
                node,
                &nodes[node_index + 1..],
                fuel,
                projection.missing_hyphens,
            )?;
            projection
                .physical_post_overrides
                .push((out.len(), physical_post));
            out.push(disc);
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
        }
        char_start = char_end;
    }

    while let Some(&position) = positions.get(position_index) {
        debug_assert_eq!(position, char_start);
        out.push(discretionary_hyphen(
            stores,
            word[position - 1].font,
            None,
            out.len(),
            projection.missing_hyphens,
        ));
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
    let post = crate::box_runtime::hmode::reconstitute_with_fuel(
        stores,
        diagnostic_effects,
        &post_pending,
        false,
        false,
        fuel,
    )
    .map_err(ExecError::Command)?;

    let (physical_replace_count, physical_post) = physical_discretionary_projection(
        stores,
        diagnostic_effects,
        word,
        span,
        &replacement,
        following,
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
fn physical_discretionary_projection<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut tex_state::diagnostic::DiagnosticEffects,
    word: &[WordChar],
    span: (usize, usize, usize),
    replacement: &Node,
    following: &[Node],
    fuel: &mut tex_command::CommandFuel,
) -> Result<(u8, tex_state::node_arena::PageListId), ExecError> {
    let (start, position, end) = span;
    let mut major = Vec::with_capacity(following.len() + 1);
    major.push(replacement.clone());
    major.extend_from_slice(following);
    let minor_pending = word[position..]
        .iter()
        .map(WordChar::pending)
        .collect::<Vec<_>>();
    let minor = crate::box_runtime::hmode::reconstitute_with_fuel(
        stores,
        diagnostic_effects,
        &minor_pending,
        false,
        false,
        fuel,
    )
    .map_err(ExecError::Command)?;

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
        stores: &CommandContext<'_, G>,
        nodes: &[Node],
    ) -> Option<HyphenationCandidate> {
        find_hyphenation_candidate(stores, nodes, 0, (0, 1, 1))
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

            let found = candidate(&stores, &nodes).expect("four letters meet the 2+2 minima");
            assert_eq!((found.language, found.left, found.right), (7, 2, 2));
            assert_eq!((found.word_start, found.end), (3, 7));
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
            assert!(candidate(&stores, &too_short).is_none());
        });
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

            let (visited, diagnostics) = hyphenated_hlist_sequence_with_fuel(
                &mut stores,
                &mut diagnostic_effects,
                nodes.clone(),
                fuel.fuel_mut(),
            )
            .expect("base-whatsit visit succeeds");

            assert_eq!(visited.semantic(), nodes);
            assert_eq!(visited.physical(), nodes);
            assert!(diagnostics.is_empty());
            let mut physical_post_overrides = Vec::new();
            let mut missing_hyphens = Vec::new();
            let mut projection = HyphenationProjection {
                physical_post_overrides: &mut physical_post_overrides,
                missing_hyphens: &mut missing_hyphens,
            };
            let (_, final_context) = hyphenated_hlist_with_projections(
                &mut stores,
                &mut diagnostic_effects,
                nodes.clone(),
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
                candidate(&stores, &nodes).is_none(),
                "uppercase starts need uchyph"
            );
            stores
                .assign_int_param(IntParam::UC_HYPH, 1, tex_state::AssignmentScope::Global)
                .expect("parameter");
            let found = candidate(&stores, &nodes).expect("enabled uppercase candidate");
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
            let found = candidate(&stores, &sixty_three).expect("63-letter candidate");
            assert_eq!((found.word.len(), found.word_start, found.end), (63, 0, 63));

            let sixty_four = vec![character(font, 'a'); 64];
            let found = candidate(&stores, &sixty_four).expect("63-letter prefix at c64");
            assert_eq!((found.word.len(), found.word_start, found.end), (63, 0, 63));
        });
    }
}

use tex_lex::InputStack;
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::node::{DiscKind, KernKind, Node};
use tex_state::token::{Catcode, OriginId, Token};

use super::*;
use crate::ExecError;
use crate::mode::PendingHChar;

pub(super) fn execute_patterns(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let words = scan_hyphenation_words(input, stores, execution, "\\patterns")?;
    let patterns = words
        .iter()
        .map(|word| parse_pattern_word(stores, word).0)
        .collect();
    let diagnostics = apply_patterns(stores, patterns).map_err(pattern_capacity_error)?;
    report_apply_diagnostics(stores, diagnostics)?;
    Ok(())
}

pub(super) fn execute_hyphenation(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    let words = scan_hyphenation_words(input, stores, execution, "\\hyphenation")?;
    let diagnostics = apply_hyphenation_exceptions(stores, words);
    report_apply_diagnostics(stores, diagnostics)?;
    Ok(())
}

/// TeX82 §961's `k>0` branch ("Insert a new pattern into the linked trie")
/// applied to §962's already-validated pattern representation. Canonical main
/// control receives that representation directly from its live scanner; the
/// retired `InputStack` compatibility path above converts its raw words before
/// crossing this apply boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HyphenationApplyDiagnostic {
    NotALetter,
    DuplicatePattern,
}

pub(crate) fn report_apply_diagnostics(
    stores: &mut Universe,
    diagnostics: Vec<HyphenationApplyDiagnostic>,
) -> Result<(), ExecError> {
    for diagnostic in diagnostics {
        let (message, help): (&str, &[&str]) = match diagnostic {
            HyphenationApplyDiagnostic::NotALetter => (
                "Not a letter",
                &[
                    "Letters in \\hyphenation words must have \\lccode>0.",
                    "Proceed; I'll ignore the character I just read.",
                ],
            ),
            HyphenationApplyDiagnostic::DuplicatePattern => {
                ("Duplicate pattern", &["(See Appendix H.)"])
            }
        };
        let mut report = stores.print_err(message);
        report.help(help);
        report.error().jump_out()?;
    }
    Ok(())
}

pub(crate) fn apply_patterns(
    stores: &mut Universe,
    patterns: Vec<PatternSpec>,
) -> Result<Vec<HyphenationApplyDiagnostic>, tex_state::hyphenation::HyphenationCapacityError> {
    install_patterns(stores, patterns, true)
}

/// Installs patterns whose §963 duplicate diagnostics were already reported
/// by the canonical live scanner.
pub(crate) fn apply_scanned_patterns(
    stores: &mut Universe,
    patterns: Vec<PatternSpec>,
) -> Result<(), ExecError> {
    let diagnostics = install_patterns(stores, patterns, false).map_err(pattern_capacity_error)?;
    debug_assert!(diagnostics.is_empty());
    Ok(())
}

fn install_patterns(
    stores: &mut Universe,
    patterns: Vec<PatternSpec>,
    collect_duplicate_diagnostics: bool,
) -> Result<Vec<HyphenationApplyDiagnostic>, tex_state::hyphenation::HyphenationCapacityError> {
    let language = current_language(stores);
    let mut diagnostics = Vec::new();
    for pattern in patterns {
        if stores.add_hyphenation_pattern_for_language(language, pattern)?
            && collect_duplicate_diagnostics
        {
            diagnostics.push(HyphenationApplyDiagnostic::DuplicatePattern);
        }
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
    Ok(diagnostics)
}

fn pattern_capacity_error(error: tex_state::hyphenation::HyphenationCapacityError) -> ExecError {
    ExecError::Fatal(tex_command::FatalError::overflow(
        "pattern memory",
        i32::try_from(error.capacity).unwrap_or(i32::MAX),
    ))
}

/// TeX82's analogous `new_hyph_exceptions` word application for
/// `\hyphenation`, shared with canonical main control; see [`apply_patterns`].
pub(crate) fn apply_hyphenation_exceptions(
    stores: &mut Universe,
    words: Vec<Vec<char>>,
) -> Vec<HyphenationApplyDiagnostic> {
    install_hyphenation_exceptions(stores, words, true)
}

/// Installs exception words the canonical live scanner already normalized, so
/// §935's `Not a letter` was reported where §82 could still show the offending
/// character (`tex_command::ScannedHyphenationData`).
pub(crate) fn apply_scanned_hyphenation_exceptions(stores: &mut Universe, words: Vec<Vec<char>>) {
    let diagnostics = install_hyphenation_exceptions(stores, words, false);
    debug_assert!(diagnostics.is_empty());
}

fn install_hyphenation_exceptions(
    stores: &mut Universe,
    words: Vec<Vec<char>>,
    normalize: bool,
) -> Vec<HyphenationApplyDiagnostic> {
    let language = current_language(stores);
    let mut diagnostics = Vec::new();
    for word in words {
        let (exception, not_letters) = parse_exception_word(stores, language, &word, normalize);
        diagnostics.extend(std::iter::repeat_n(
            HyphenationApplyDiagnostic::NotALetter,
            not_letters,
        ));
        if let Some(exception) = exception {
            stores.add_hyphenation_exception_for_language(language, exception);
        }
    }
    diagnostics
}

#[cfg(test)]
pub(crate) fn hyphenated_hlist_with_fuel(
    stores: &mut Universe,
    nodes: Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, ExecError> {
    hyphenated_hlist_with_projections(stores, nodes, fuel, &mut Vec::new())
}

fn hyphenated_hlist_with_projections(
    stores: &mut Universe,
    nodes: Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
    physical_post_overrides: &mut Vec<(usize, tex_state::ids::NodeListId)>,
) -> Result<Vec<Node>, ExecError> {
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
                &nodes,
                index,
                (language, left, right),
                &mut out,
                fuel,
                physical_post_overrides,
            )?
        {
            index = next;
        }
    }
    Ok(out.unwrap_or(nodes))
}

/// Builds the semantic hyphenated list together with TeX82's physical
/// diagnostic projection. A boundary discretionary keeps the compact
/// semantic pre-break list used by line breaking, while TeX's linked-list
/// representation exposes the preceding reconstituted character span in the
/// discretionary's diagnostic pre-break branch.
pub(crate) fn hyphenated_hlist_sequence_with_fuel(
    stores: &mut Universe,
    nodes: Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
) -> Result<tex_state::node_sequence::NodeSequence, ExecError> {
    let mut physical_post_overrides = Vec::new();
    let semantic =
        hyphenated_hlist_with_projections(stores, nodes, fuel, &mut physical_post_overrides)?;
    let mut physical = semantic.clone();
    for (index, physical_post) in physical_post_overrides {
        if let Some(Node::Disc { post, .. }) = physical.get_mut(index) {
            *post = physical_post;
        }
    }
    project_physical_pre_break_spans(stores, &mut physical, fuel)?;
    Ok(tex_state::node_sequence::NodeSequence::from_channels(
        semantic, physical,
    ))
}

fn project_physical_pre_break_spans(
    stores: &mut Universe,
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
        let replacement = stores.nodes(*replace);
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
        let pre = super::hmode::reconstitute_with_fuel(stores, &pending, true, false, fuel)
            .map_err(ExecError::Command)?;
        let pre = stores.freeze_node_list(&pre);
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

#[cfg(test)]
pub(crate) fn test_physical_pre_break_projection(
    stores: &mut Universe,
    nodes: &[Node],
) -> Vec<Node> {
    let mut projected = nodes.to_vec();
    let mut fuel = tex_command::CommandFuelLedger::default();
    project_physical_pre_break_spans(stores, &mut projected, fuel.fuel_mut())
        .expect("test diagnostic projection fuel");
    projected
}

#[cfg(test)]
pub(crate) fn hyphenated_hlist(stores: &mut Universe, nodes: Vec<Node>) -> Vec<Node> {
    let mut fuel = tex_command::CommandFuelLedger::default();
    hyphenated_hlist_with_fuel(stores, nodes, fuel.fuel_mut()).expect("test hyphenation fuel")
}

/// Returns legal character boundaries for pass-1 OpenType shaping.
pub(super) fn candidate_positions_for_chars(
    stores: &Universe,
    language: u8,
    chars: &[PendingHChar],
    left: usize,
    right: usize,
) -> Vec<usize> {
    if chars.len() > 63 || chars.len() < left.saturating_add(right) {
        return Vec::new();
    }
    let Some(first) = chars.first() else {
        return Vec::new();
    };
    if !(0..=255).contains(&stores.font_hyphen_char(first.font))
        || chars.iter().any(|entry| entry.font != first.font)
    {
        return Vec::new();
    }
    let Some(normalized) = chars
        .iter()
        .map(|entry| normalized_hyphen_code(stores, language, entry.ch))
        .collect::<Option<String>>()
    else {
        return Vec::new();
    };
    if !normalized.starts_with(first.ch) && stores.int_param(IntParam::UC_HYPH) <= 0 {
        return Vec::new();
    }
    stores.hyphen_positions_for_language(language, &normalized, left, right)
}

/// Renders the discretionary breaks TeX82 §923's `hyphenate` would find in a
/// single word, using the current `\language`, `\lefthyphenmin`, and
/// `\righthyphenmin`.
///
/// This is the query plain.tex's `\showhyphens` macro answers indirectly, by
/// packing the word into an over-wide `\vbox` and reading the underfull-box
/// report. Tests that only need the pattern/exception/hyphen-code decision
/// ask for it directly instead of asserting on box-display text.
#[cfg(test)]
pub(crate) fn test_hyphenated_word_text(stores: &Universe, word: &str) -> String {
    let language = current_language(stores);
    let Some(normalized) = word
        .chars()
        .map(|ch| normalized_hyphen_code(stores, language, ch))
        .collect::<Option<String>>()
    else {
        return word.to_owned();
    };
    let left = usize::try_from(stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(0)).unwrap_or(0);
    let right = usize::try_from(stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(0)).unwrap_or(0);
    let positions = stores.hyphen_positions_for_language(language, &normalized, left, right);
    let mut text = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if positions.contains(&index) {
            text.push('-');
        }
        text.push(ch);
    }
    text
}

#[cfg(test)]
pub(crate) fn test_hyphenated_word(stores: &mut Universe, nodes: &[Node]) -> Vec<Node> {
    let glue = stores.glue_param(tex_state::env::banks::GlueParam::PAR_SKIP);
    let boundary = Node::Glue {
        spec: glue,
        kind: tex_state::node::GlueKind::Normal,
        leader: None,
    };
    let mut paragraph = Vec::with_capacity(nodes.len() + 2);
    paragraph.push(boundary.clone());
    paragraph.extend_from_slice(nodes);
    paragraph.push(boundary);
    let mut fuel = tex_command::CommandFuelLedger::default();
    let mut hyphenated = hyphenated_hlist_with_fuel(stores, paragraph, fuel.fuel_mut())
        .expect("test hyphenation fuel");
    hyphenated.remove(0);
    hyphenated.pop();
    hyphenated
}

#[cfg(test)]
pub(crate) fn test_language_context(nodes: &[Node]) -> (u8, usize, usize) {
    let mut language = 0;
    let mut left = 1;
    let mut right = 1;
    for node in nodes {
        update_hyphenation_context(node, &mut language, &mut left, &mut right);
    }
    (language, left, right)
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

fn hyphenate_after_glue(
    stores: &mut Universe,
    nodes: &[Node],
    start: usize,
    context: (u8, usize, usize),
    out: &mut Option<Vec<Node>>,
    fuel: &mut tex_command::CommandFuel,
    physical_post_overrides: &mut Vec<(usize, tex_state::ids::NodeListId)>,
) -> Result<Option<usize>, ExecError> {
    let (mut language, mut left, mut right) = context;
    let mut index = start;
    let (word_start, font) = loop {
        let Some(node) = nodes.get(index) else {
            return Ok(None);
        };
        match first_word_char(stores, language, node) {
            Some((font, ch, lower)) => {
                if lower != ch && stores.int_param(IntParam::UC_HYPH) <= 0 {
                    return Ok(None);
                }
                break (index, font);
            }
            None if is_pre_word_skip(node) => {
                update_hyphenation_context(node, &mut language, &mut left, &mut right);
                index += 1;
            }
            None => return Ok(None),
        }
    };

    let Some(minima) = left.checked_add(right) else {
        return Ok(None);
    };
    if minima > 63 {
        return Ok(None);
    }
    let hyphen = stores.font_hyphen_char(font);
    if !(0..=255).contains(&hyphen) {
        return Ok(None);
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
                for ((ch, lower), origin) in normalized.into_iter().zip(origins.iter().copied()) {
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
        return Ok(None);
    }

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
        &word,
        &positions,
        no_left_boundary,
        out,
        fuel,
        physical_post_overrides,
    )?;
    if let Some(kern) = trailing_font_kern {
        out.push(kern);
    }
    Ok(Some(index))
}

fn first_word_char(
    stores: &Universe,
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

fn scan_hyphenation_words(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<Vec<Vec<char>>, ExecError> {
    let open = loop {
        let traced = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        .ok_or(ExecError::MissingToken { context })?;
        let token = tex_expand::semantic_token(traced);
        if is_space(token) {
            continue;
        }
        if let Token::Cs(symbol) = token
            && stores.meaning(symbol) == Meaning::Relax
        {
            continue;
        }
        break token;
    };
    if !is_begin_group(open) {
        return Err(ExecError::MissingToken { context });
    }
    let mut words = Vec::new();
    let mut current = Vec::new();
    let mut depth = 1usize;
    while let Some(traced) = get_x_token_with_context(
        input,
        &mut tex_state::ExpansionContext::new(stores),
        execution,
    )? {
        let token = tex_expand::semantic_token(traced);
        if is_begin_group(token) {
            depth += 1;
            continue;
        }
        if is_end_group(token) {
            depth -= 1;
            if depth == 0 {
                if !current.is_empty() {
                    words.push(current);
                }
                return Ok(words);
            }
            continue;
        }
        match token {
            Token::Char {
                cat: Catcode::Space,
                ..
            } => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            Token::Char { ch, .. } => current.push(ch),
            Token::Cs(_) | Token::Param(_) | Token::Frozen(_) => {}
        }
    }
    Err(ExecError::MissingToken { context })
}

fn parse_pattern_word(stores: &Universe, word: &[char]) -> (PatternSpec, usize) {
    let mut letters = Vec::new();
    let mut values = vec![0u8];
    let mut digit_sensed = false;
    let mut nonletters = 0;
    for &ch in word {
        if !digit_sensed && ch.is_ascii_digit() {
            let digit = ch.to_digit(10).expect("ASCII digit has a value");
            *values.last_mut().expect("values is non-empty") = digit as u8;
            digit_sensed = true;
        } else {
            let normalized = if ch == '.' {
                '.'
            } else {
                normalized_lccode(stores, ch).unwrap_or_else(|| {
                    nonletters += 1;
                    '.'
                })
            };
            if letters.len() < 63 {
                letters.push(normalized);
                values.push(0);
                digit_sensed = false;
            }
        }
    }
    (PatternSpec { letters, values }, nonletters)
}

fn parse_exception_word(
    stores: &Universe,
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

fn normalized_lccode(stores: &Universe, ch: char) -> Option<char> {
    char::from_u32(stores.lccode(ch)).filter(|&mapped| mapped != '\0')
}

fn normalized_hyphen_code(stores: &Universe, language: u8, ch: char) -> Option<char> {
    stores
        .saved_hyphenation_code(language, ch)
        .unwrap_or_else(|| normalized_lccode(stores, ch))
}

fn current_language(stores: &Universe) -> u8 {
    u8::try_from(stores.int_param(IntParam::LANGUAGE)).unwrap_or(0)
}

fn append_hyphenated_word(
    stores: &mut Universe,
    word: &[WordChar],
    positions: &[usize],
    no_left_boundary: bool,
    out: &mut Vec<Node>,
    fuel: &mut tex_command::CommandFuel,
    physical_post_overrides: &mut Vec<(usize, tex_state::ids::NodeListId)>,
) -> Result<(), ExecError> {
    let pending: Vec<_> = word.iter().map(WordChar::pending).collect();
    let nodes =
        super::hmode::reconstitute_with_fuel(stores, &pending, no_left_boundary, false, fuel)
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
                word,
                (char_start, position, char_end),
                node,
                &nodes[node_index + 1..],
                fuel,
            )?;
            physical_post_overrides.push((out.len(), physical_post));
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
        out.push(discretionary_hyphen(stores, word[position - 1].font, None));
        position_index += 1;
    }
    Ok(())
}

fn discretionary_through_node(
    stores: &mut Universe,
    word: &[WordChar],
    span: (usize, usize, usize),
    replacement: Node,
    following: &[Node],
    fuel: &mut tex_command::CommandFuel,
) -> Result<(Node, tex_state::ids::NodeListId), ExecError> {
    let (start, position, end) = span;
    let font = word[position - 1].font;
    let mut pre_pending: Vec<_> = word[start..position]
        .iter()
        .map(WordChar::pending)
        .collect();
    if let Some(ch) = usable_hyphen_char(stores, font) {
        pre_pending.push(PendingHChar {
            font,
            ch,
            origin: word[position - 1].origin,
        });
    }
    let pre = super::hmode::reconstitute_with_fuel(stores, &pre_pending, true, false, fuel)
        .map_err(ExecError::Command)?;
    let post_pending: Vec<_> = word[position..end].iter().map(WordChar::pending).collect();
    let post = super::hmode::reconstitute_with_fuel(stores, &post_pending, false, false, fuel)
        .map_err(ExecError::Command)?;

    let (physical_replace_count, physical_post) = physical_discretionary_projection(
        stores,
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
fn physical_discretionary_projection(
    stores: &mut Universe,
    word: &[WordChar],
    span: (usize, usize, usize),
    replacement: &Node,
    following: &[Node],
    fuel: &mut tex_command::CommandFuel,
) -> Result<(u8, tex_state::ids::NodeListId), ExecError> {
    let (start, position, end) = span;
    let mut major = Vec::with_capacity(following.len() + 1);
    major.push(replacement.clone());
    major.extend_from_slice(following);
    let minor_pending = word[position..]
        .iter()
        .map(WordChar::pending)
        .collect::<Vec<_>>();
    let minor = super::hmode::reconstitute_with_fuel(stores, &minor_pending, false, false, fuel)
        .map_err(ExecError::Command)?;

    let (major_len, minor_len) = synchronized_physical_branch_lengths(
        &major,
        start,
        &minor,
        position,
        end,
        word.len(),
    );
    let physical_replace_count = u8::try_from(major_len)
        .ok()
        .filter(|&count| count <= 127)
        .expect("a TeX word has at most 127 physical replacement nodes");
    Ok((
        physical_replace_count,
        stores.freeze_node_list(&minor[..minor_len]),
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

#[cfg(test)]
pub(crate) fn test_physical_post_break_span(
    word_len: usize,
    span: (usize, usize, usize),
    replacement: &Node,
    following: &[Node],
    minor: &[Node],
) -> (u8, Vec<Node>) {
    let (start, position, end) = span;
    let mut major = vec![replacement.clone()];
    major.extend_from_slice(following);
    let (major_len, minor_len) = synchronized_physical_branch_lengths(
        &major, start, minor, position, end, word_len,
    );
    (
        u8::try_from(major_len).expect("bounded test projection"),
        minor[..minor_len].to_vec(),
    )
}

/// Freezes a §914 automatic discretionary when §918's replacement count fits.
#[cfg(test)]
fn automatic_discretionary(
    stores: &mut Universe,
    pre: &[Node],
    post: &[Node],
    replace: &[Node],
) -> Option<Node> {
    let physical_replace_count = automatic_physical_replace_count(replace)?;
    automatic_discretionary_with_count(stores, pre, post, replace, physical_replace_count)
}

fn automatic_discretionary_with_count(
    stores: &mut Universe,
    pre: &[Node],
    post: &[Node],
    replace: &[Node],
    physical_replace_count: u8,
) -> Option<Node> {
    (physical_replace_count <= 127).then(|| Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre: stores.freeze_node_list(pre),
        post: stores.freeze_node_list(post),
        replace: stores.freeze_node_list(replace),
        physical_replace_count,
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

#[cfg(test)]
pub(crate) fn test_automatic_discretionary(
    stores: &mut Universe,
    replace: &[Node],
) -> Option<Node> {
    automatic_discretionary(stores, &[], &[], replace)
}

fn node_original_len(node: &Node) -> usize {
    match node {
        Node::Char { .. } => 1,
        Node::Lig { orig, .. } => orig.len(),
        Node::Kern { .. } => 0,
        _ => 0,
    }
}

fn discretionary_hyphen(
    stores: &mut Universe,
    font: tex_state::ids::FontId,
    replacement: Option<Node>,
) -> Node {
    let empty = stores.freeze_node_list(&[]);
    let pre = usable_hyphen_char(stores, font).map_or(empty, |ch| {
        stores.freeze_node_list(&[Node::Char {
            font,
            ch,
            origin: OriginId::UNKNOWN,
        }])
    });
    let replace = replacement.as_ref().map_or(empty, |node| {
        stores.freeze_node_list(std::slice::from_ref(node))
    });
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

fn usable_hyphen_char(stores: &Universe, font: tex_state::ids::FontId) -> Option<char> {
    let code = u8::try_from(stores.font_hyphen_char(font)).ok()?;
    stores
        .font_char_exists(font, code)
        .then(|| char::from(code))
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

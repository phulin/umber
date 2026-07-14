use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::node::{DiscKind, KernKind, Node};
use tex_state::token::{Catcode, Token};

use super::*;
use crate::ExecError;
use crate::mode::PendingHChar;

pub(super) fn execute_patterns<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let language = current_language(stores);
    for word in scan_hyphenation_words(input, stores, execution, "\\patterns")? {
        if let Some(pattern) = parse_pattern_word(stores, &word) {
            stores.add_hyphenation_pattern_for_language(language, pattern);
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
    Ok(())
}

pub(super) fn execute_hyphenation<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
) -> Result<(), ExecError>
where
    S: InputSource,
{
    let language = current_language(stores);
    for word in scan_hyphenation_words(input, stores, execution, "\\hyphenation")? {
        if let Some(exception) = parse_exception_word(stores, language, &word) {
            stores.add_hyphenation_exception_for_language(language, exception);
        }
    }
    Ok(())
}

pub(crate) fn hyphenated_hlist(stores: &mut Universe, nodes: &[Node]) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
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
        out.push(node.clone());
        index += 1;

        if auto_breaking
            && matches!(node, Node::Glue { .. })
            && let Some(next) =
                hyphenate_after_glue(stores, nodes, index, language, left, right, &mut out)
        {
            index = next;
        }
    }
    out
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
    let mut hyphenated = hyphenated_hlist(stores, &paragraph);
    hyphenated.remove(0);
    hyphenated.pop();
    hyphenated
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
    mut language: u8,
    mut left: usize,
    mut right: usize,
    out: &mut Vec<Node>,
) -> Option<usize> {
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

    if left.saturating_add(right) > 63 {
        return None;
    }
    let hyphen = stores.font_hyphen_char(font);
    if !(0..=255).contains(&hyphen) {
        return None;
    }

    let mut word = Vec::new();
    let mut word_nodes = Vec::new();
    index = word_start;
    while let Some(node) = nodes.get(index) {
        match node {
            Node::Char {
                font: node_font,
                ch,
            } if *node_font == font && word.len() < 63 => {
                let Some(lower) = normalized_hyphen_code(stores, language, *ch) else {
                    break;
                };
                word.push(WordChar {
                    font,
                    ch: *ch,
                    lower,
                });
                word_nodes.push(node.clone());
                index += 1;
            }
            Node::Lig {
                font: node_font,
                ch,
                orig,
            } if *node_font == font => {
                let chars = ligature_original_chars(*ch, *orig);
                if word.len().saturating_add(chars.len()) > 63 {
                    break;
                }
                let Some(normalized) = chars
                    .into_iter()
                    .map(|ch| normalized_hyphen_code(stores, language, ch).map(|lower| (ch, lower)))
                    .collect::<Option<Vec<_>>>()
                else {
                    break;
                };
                for (ch, lower) in normalized {
                    word.push(WordChar { font, ch, lower });
                }
                word_nodes.push(node.clone());
                index += 1;
            }
            Node::Kern {
                kind: KernKind::Font,
                ..
            } => {
                word_nodes.push(node.clone());
                index += 1;
            }
            _ => break,
        }
    }

    if word.len() < left.saturating_add(right) || !permitted_word_terminator(nodes, index) {
        return None;
    }

    let lowercase: String = word.iter().map(|ch| ch.lower).collect();
    let positions = stores.hyphen_positions_for_language(language, &lowercase, left, right);
    out.extend_from_slice(&nodes[start..word_start]);
    if positions.is_empty() {
        out.extend(word_nodes);
    } else {
        let trailing_font_kern = word_nodes.last().and_then(|node| match node {
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
        append_hyphenated_word(stores, &word, &positions, no_left_boundary, out);
        if let Some(kern) = trailing_font_kern {
            out.push(kern);
        }
    }
    Some(index)
}

fn first_word_char(
    stores: &Universe,
    language: u8,
    node: &Node,
) -> Option<(tex_state::ids::FontId, char, char)> {
    match node {
        Node::Char { font, ch } => {
            normalized_hyphen_code(stores, language, *ch).map(|lower| (*font, *ch, lower))
        }
        Node::Lig { font, ch, orig } => {
            ligature_original_chars(*ch, *orig)
                .first()
                .and_then(|&first| {
                    normalized_hyphen_code(stores, language, first)
                        .map(|lower| (*font, first, lower))
                })
        }
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
            | Node::Kern { .. } => return true,
            _ => return false,
        }
    }
    true
}

fn scan_hyphenation_words<S>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_, S>,
    context: &'static str,
) -> Result<Vec<Vec<char>>, ExecError>
where
    S: InputSource,
{
    let mut recorder = NoopRecorder;
    let open = loop {
        let traced =
            get_x_token_with_recorder_and_context(input, stores, &mut recorder, execution)?
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
    while let Some(traced) =
        get_x_token_with_recorder_and_context(input, stores, &mut recorder, execution)?
    {
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

fn parse_pattern_word(stores: &Universe, word: &[char]) -> Option<PatternSpec> {
    let mut letters = Vec::new();
    let mut values = vec![0u8];
    for &ch in word {
        if let Some(digit) = ch.to_digit(10) {
            *values.last_mut().expect("values is non-empty") = digit as u8;
        } else {
            let normalized = if ch == '.' {
                '.'
            } else {
                normalized_lccode(stores, ch)?
            };
            letters.push(normalized);
            values.push(0);
        }
    }
    Some(PatternSpec { letters, values })
}

fn parse_exception_word(stores: &Universe, language: u8, word: &[char]) -> Option<ExceptionSpec> {
    let mut normalized = String::new();
    let mut positions = Vec::new();
    for &ch in word {
        if ch == '-' {
            positions.push(normalized.chars().count());
        } else {
            normalized.push(normalized_hyphen_code(stores, language, ch)?);
        }
    }
    Some(ExceptionSpec {
        word: normalized,
        positions,
    })
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
) {
    let pending: Vec<_> = word.iter().map(WordChar::pending).collect();
    let nodes = super::hmode::reconstitute(stores, &pending, no_left_boundary, false);
    let mut position_index = 0;
    let mut char_start = 0;

    for node in nodes {
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
            out.push(discretionary_through_node(
                stores, word, char_start, position, char_end, node,
            ));
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
}

fn discretionary_through_node(
    stores: &mut Universe,
    word: &[WordChar],
    start: usize,
    position: usize,
    end: usize,
    replacement: Node,
) -> Node {
    let font = word[position - 1].font;
    let mut pre_pending: Vec<_> = word[start..position]
        .iter()
        .map(WordChar::pending)
        .collect();
    if let Some(ch) = usable_hyphen_char(stores, font) {
        pre_pending.push(PendingHChar { font, ch });
    }
    let pre = super::hmode::reconstitute(stores, &pre_pending, true, false);
    let post_pending: Vec<_> = word[position..end].iter().map(WordChar::pending).collect();
    let post = super::hmode::reconstitute(stores, &post_pending, false, false);

    let pre = stores.freeze_node_list(&pre);
    let post = stores.freeze_node_list(&post);
    let replace = stores.freeze_node_list(&[replacement]);
    Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre,
        post,
        replace,
    }
}

fn node_original_len(node: &Node) -> usize {
    match node {
        Node::Char { .. } => 1,
        Node::Lig { ch, orig, .. } => ligature_original_chars(*ch, *orig).len(),
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
        stores.freeze_node_list(&[Node::Char { font, ch }])
    });
    let replace = replacement.as_ref().map_or(empty, |node| {
        stores.freeze_node_list(std::slice::from_ref(node))
    });
    Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre,
        post: empty,
        replace,
    }
}

fn usable_hyphen_char(stores: &Universe, font: tex_state::ids::FontId) -> Option<char> {
    let code = u8::try_from(stores.font_hyphen_char(font)).ok()?;
    stores
        .font_char_exists(font, code)
        .then(|| char::from(code))
}

pub(super) fn ligature_original_chars(ch: char, orig: (char, char)) -> Vec<char> {
    match ch as u32 {
        0o13 => vec!['f', 'f'],
        0o14 => vec!['f', 'i'],
        0o15 => vec!['f', 'l'],
        0o16 => vec!['f', 'f', 'i'],
        0o17 => vec!['f', 'f', 'l'],
        _ => vec![orig.0, orig.1],
    }
}

#[derive(Clone, Copy)]
struct WordChar {
    font: tex_state::ids::FontId,
    ch: char,
    lower: char,
}

impl WordChar {
    fn pending(&self) -> PendingHChar {
        PendingHChar {
            font: self.font,
            ch: self.ch,
        }
    }
}

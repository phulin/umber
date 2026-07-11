use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::IntParam;
use tex_state::hyphenation::{ExceptionSpec, PatternSpec};
use tex_state::node::{DiscKind, KernKind, Node};
use tex_state::token::{Catcode, Token};

use super::*;
use crate::ExecError;
use crate::mode::PendingHChar;

pub(super) fn execute_patterns<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    for word in scan_hyphenation_words(input, stores, hooks, "\\patterns")? {
        if let Some(pattern) = parse_pattern_word(stores, &word) {
            stores.add_hyphenation_pattern(pattern);
        }
    }
    Ok(())
}

pub(super) fn execute_hyphenation<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
) -> Result<(), ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    for word in scan_hyphenation_words(input, stores, hooks, "\\hyphenation")? {
        if let Some(exception) = parse_exception_word(stores, &word) {
            stores.add_hyphenation_exception(exception);
        }
    }
    Ok(())
}

pub(crate) fn hyphenated_hlist(stores: &mut Universe, nodes: &[Node]) -> Vec<Node> {
    let mut out = Vec::new();
    let mut word = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if push_word_node(stores, node, nodes.get(index + 1), &mut word) {
            continue;
        }
        flush_word(stores, &mut word, &mut out);
        out.push(node.clone());
    }
    flush_word(stores, &mut word, &mut out);
    out
}

fn scan_hyphenation_words<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: &'static str,
) -> Result<Vec<Vec<char>>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    let mut recorder = NoopRecorder;
    let open = loop {
        let traced = get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
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
        get_x_token_with_recorder_and_hooks(input, stores, &mut recorder, hooks)?
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

fn parse_exception_word(stores: &Universe, word: &[char]) -> Option<ExceptionSpec> {
    let mut normalized = String::new();
    let mut positions = Vec::new();
    for &ch in word {
        if ch == '-' {
            positions.push(normalized.chars().count());
        } else {
            normalized.push(normalized_lccode(stores, ch)?);
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

fn push_word_node(
    stores: &Universe,
    node: &Node,
    next: Option<&Node>,
    word: &mut Vec<WordChar>,
) -> bool {
    match node {
        Node::Char { font, ch } => {
            let Some(lower) = normalized_lccode(stores, *ch) else {
                return false;
            };
            word.push(WordChar {
                font: *font,
                ch: *ch,
                lower,
                uppercase: lower != *ch,
            });
            true
        }
        Node::Lig { font, ch, orig } => {
            let chars = ligature_original_chars(*ch, *orig);
            let normalized: Option<Vec<_>> = chars
                .into_iter()
                .map(|ch| normalized_lccode(stores, ch).map(|lower| (ch, lower)))
                .collect();
            let Some(normalized) = normalized else {
                return false;
            };
            for (ch, lower) in normalized {
                word.push(WordChar {
                    font: *font,
                    ch,
                    lower,
                    uppercase: lower != ch,
                });
            }
            true
        }
        Node::Kern {
            kind: KernKind::Font,
            ..
        } if !word.is_empty() && next.is_some_and(|node| is_word_node(stores, node)) => true,
        _ => false,
    }
}

fn is_word_node(stores: &Universe, node: &Node) -> bool {
    match node {
        Node::Char { ch, .. } => normalized_lccode(stores, *ch).is_some(),
        Node::Lig { ch, orig, .. } => ligature_original_chars(*ch, *orig)
            .into_iter()
            .all(|ch| normalized_lccode(stores, ch).is_some()),
        _ => false,
    }
}

fn flush_word(stores: &mut Universe, word: &mut Vec<WordChar>, out: &mut Vec<Node>) {
    if word.is_empty() {
        return;
    }
    let lowercase: String = word.iter().map(|ch| ch.lower).collect();
    let left = stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(0) as usize;
    let right = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(0) as usize;
    let positions = if word
        .first()
        .is_none_or(|ch| u8::try_from(stores.font_hyphen_char(ch.font)).is_err())
        || word.first().is_some_and(|ch| ch.uppercase) && stores.int_param(IntParam::UC_HYPH) <= 0
    {
        Vec::new()
    } else {
        stores.hyphen_positions(&lowercase, left, right)
    };
    if positions.is_empty() {
        let pending: Vec<_> = word.iter().map(|ch| ch.pending()).collect();
        out.extend(super::hmode::reconstitute(stores, &pending, false, false));
        word.clear();
        return;
    }

    append_hyphenated_word(stores, word, &positions, out);
    word.clear();
}

fn append_hyphenated_word(
    stores: &mut Universe,
    word: &[WordChar],
    positions: &[usize],
    out: &mut Vec<Node>,
) {
    let pending: Vec<_> = word.iter().map(WordChar::pending).collect();
    let nodes = super::hmode::reconstitute(stores, &pending, false, false);
    let mut position_index = 0;
    let mut char_start = 0;

    for node in nodes {
        while positions.get(position_index) == Some(&char_start) {
            out.push(discretionary_hyphen(stores, word[char_start - 1].font));
            position_index += 1;
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
        out.push(discretionary_hyphen(stores, word[position - 1].font));
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
    let hyphen = u8::try_from(stores.font_hyphen_char(font))
        .ok()
        .map(char::from)
        .unwrap_or('-');

    let mut pre_pending: Vec<_> = word[start..position]
        .iter()
        .map(WordChar::pending)
        .collect();
    pre_pending.push(PendingHChar { font, ch: hyphen });
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

fn discretionary_hyphen(stores: &mut Universe, font: tex_state::ids::FontId) -> Node {
    let hyphen = u8::try_from(stores.font_hyphen_char(font))
        .ok()
        .map(char::from)
        .unwrap_or('-');
    let pre = stores.freeze_node_list(&[Node::Char { font, ch: hyphen }]);
    let empty = stores.freeze_node_list(&[]);
    Node::Disc {
        kind: DiscKind::AutomaticHyphen,
        pre,
        post: empty,
        replace: empty,
    }
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
    uppercase: bool,
}

impl WordChar {
    fn pending(&self) -> PendingHChar {
        PendingHChar {
            font: self.font,
            ch: self.ch,
        }
    }
}

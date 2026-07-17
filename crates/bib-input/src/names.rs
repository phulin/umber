use bib_model::{Literal, Name, NameBuilder, NameList, NamePartValue};
use bib_unicode::{compatibility_hash, normalise_nfc, normalise_string_hash, remove_outer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClassicNameLimits {
    pub max_names: usize,
    pub max_name_bytes: usize,
    pub max_nesting: usize,
    pub max_work: usize,
    pub max_diagnostics: usize,
}

impl Default for ClassicNameLimits {
    fn default() -> Self {
        Self {
            max_names: 10_000,
            max_name_bytes: 1024 * 1024,
            max_nesting: 256,
            max_work: 8 * 1024 * 1024,
            max_diagnostics: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClassicNameOptions<'a> {
    pub separators: &'a [&'a str],
    pub others: &'a [&'a str],
    pub limits: ClassicNameLimits,
}

impl Default for ClassicNameOptions<'static> {
    fn default() -> Self {
        Self {
            separators: &["and"],
            others: &["others"],
            limits: ClassicNameLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassicNameDiagnosticKind {
    EmptyName,
    TooManyCommas,
    UnbalancedBraces,
    Limit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicNameDiagnostic {
    pub kind: ClassicNameDiagnosticKind,
    pub offset: usize,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClassicNameParse {
    pub names: NameList,
    pub diagnostics: Vec<ClassicNameDiagnostic>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameHashScope {
    Full,
    Initials,
}

pub fn parse_classic_name_list(value: &str, options: ClassicNameOptions<'_>) -> ClassicNameParse {
    let mut parser = ListParser::new(value, options);
    parser.parse()
}

pub fn parse_classic_name(
    value: &str,
    limits: ClassicNameLimits,
) -> Result<Name, ClassicNameDiagnostic> {
    if value.len() > limits.max_name_bytes {
        return Err(diagnostic(
            ClassicNameDiagnosticKind::Limit,
            0,
            "name exceeds byte limit",
        ));
    }
    validate_braces(value, limits.max_nesting)?;
    parse_one(value).ok_or_else(|| {
        diagnostic(
            ClassicNameDiagnosticKind::EmptyName,
            0,
            "name contains no parts",
        )
    })
}

pub fn classic_name_hash(names: &NameList, scope: NameHashScope) -> String {
    let mut key = String::new();
    for name in names.iter() {
        for part in [name.family(), name.given(), name.prefix(), name.suffix()]
            .into_iter()
            .flatten()
        {
            match scope {
                NameHashScope::Full => key.push_str(part.value().as_str()),
                NameHashScope::Initials => {
                    for initial in part.initials() {
                        key.push_str(initial);
                    }
                }
            }
        }
    }
    if names.has_others() {
        key.push('+');
    }
    compatibility_hash(&normalise_nfc(&normalise_string_hash(&key)))
}

struct ListParser<'a> {
    value: &'a str,
    options: ClassicNameOptions<'a>,
    diagnostics: Vec<ClassicNameDiagnostic>,
    work: usize,
}

impl<'a> ListParser<'a> {
    fn new(value: &'a str, options: ClassicNameOptions<'a>) -> Self {
        Self {
            value,
            options,
            diagnostics: Vec::new(),
            work: 0,
        }
    }

    fn parse(&mut self) -> ClassicNameParse {
        if self.value.len() > self.options.limits.max_work {
            self.push(diagnostic(
                ClassicNameDiagnosticKind::Limit,
                0,
                "name-list work limit exceeded",
            ));
            return self.finish(Vec::new(), false);
        }
        if let Err(error) = validate_braces(self.value, self.options.limits.max_nesting) {
            self.push(error);
            return self.finish(Vec::new(), false);
        }

        let segments = split_list(self.value, self.options.separators);
        let mut names = Vec::new();
        let mut has_others = false;
        for (offset, segment) in segments {
            self.work = self.work.saturating_add(segment.len());
            if self.work > self.options.limits.max_work {
                self.push(diagnostic(
                    ClassicNameDiagnosticKind::Limit,
                    offset,
                    "name-list work limit exceeded",
                ));
                break;
            }
            let segment = segment.trim();
            if segment.is_empty() {
                self.push(diagnostic(
                    ClassicNameDiagnosticKind::EmptyName,
                    offset,
                    "empty name between separators",
                ));
                continue;
            }
            if self
                .options
                .others
                .iter()
                .any(|alias| segment.eq_ignore_ascii_case(alias))
            {
                has_others = true;
                continue;
            }
            if names.len() == self.options.limits.max_names {
                self.push(diagnostic(
                    ClassicNameDiagnosticKind::Limit,
                    offset,
                    "name count limit exceeded",
                ));
                break;
            }
            if segment.len() > self.options.limits.max_name_bytes {
                self.push(diagnostic(
                    ClassicNameDiagnosticKind::Limit,
                    offset,
                    "name exceeds byte limit",
                ));
                continue;
            }
            let comma_count = top_level_delimiters(segment, ',').len();
            if comma_count > 2 {
                self.push(diagnostic(
                    ClassicNameDiagnosticKind::TooManyCommas,
                    offset,
                    "unprotected name has more than two commas",
                ));
                continue;
            }
            if let Some(name) = parse_one(segment) {
                names.push(name);
            }
        }
        self.finish(names, has_others)
    }

    fn finish(&mut self, names: Vec<Name>, has_others: bool) -> ClassicNameParse {
        ClassicNameParse {
            names: NameList::new(names, has_others),
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    fn push(&mut self, error: ClassicNameDiagnostic) {
        if self.diagnostics.len() < self.options.limits.max_diagnostics {
            self.diagnostics.push(error);
        }
    }
}

fn parse_one(source: &str) -> Option<Name> {
    let sanitized = sanitize(source);
    let (whole_protected, protected) = collapse_outer(&sanitized);
    let mut builder = NameBuilder::new();
    builder.source(source.trim().to_owned());
    if whole_protected {
        builder.family_part(make_part(protected, true));
        return builder.freeze().ok();
    }

    let comma_positions = top_level_delimiters(&sanitized, ',');
    let pieces = split_at_positions(&sanitized, &comma_positions);
    match pieces.as_slice() {
        [only] => parse_first_von_last(only, &mut builder),
        [von_last, given] => {
            parse_von_last(von_last, &mut builder);
            set_tokens(&mut builder, Part::Given, &split_words(given));
        }
        [von_last, suffix, given] => {
            parse_von_last(von_last, &mut builder);
            set_part(&mut builder, Part::Suffix, suffix);
            set_tokens(&mut builder, Part::Given, &split_words(given));
        }
        _ => return None,
    }
    builder.freeze().ok()
}

fn parse_first_von_last(value: &str, builder: &mut NameBuilder) {
    let tokens = split_words(value);
    if tokens.is_empty() {
        return;
    }
    let family_start = tokens
        .iter()
        .position(|token| starts_lowercase(token))
        .map_or(tokens.len() - 1, |prefix_start| {
            let after_prefix = tokens[prefix_start..]
                .iter()
                .position(|token| !starts_lowercase(token))
                .map(|relative| prefix_start + relative);
            after_prefix.unwrap_or(tokens.len() - 1)
        });
    let prefix_start = tokens[..family_start]
        .iter()
        .position(|token| starts_lowercase(token));
    if let Some(prefix_start) = prefix_start {
        set_tokens(builder, Part::Given, &tokens[..prefix_start]);
        set_tokens(builder, Part::Prefix, &tokens[prefix_start..family_start]);
    } else {
        set_tokens(builder, Part::Given, &tokens[..family_start]);
    }
    set_tokens(builder, Part::Family, &tokens[family_start..]);
}

fn parse_von_last(value: &str, builder: &mut NameBuilder) {
    let tokens = split_words(value);
    if tokens.len() > 1 && starts_lowercase(tokens[0]) {
        let family_start = tokens
            .iter()
            .position(|token| !starts_lowercase(token))
            .unwrap_or(tokens.len() - 1);
        set_tokens(builder, Part::Prefix, &tokens[..family_start]);
        set_tokens(builder, Part::Family, &tokens[family_start..]);
    } else {
        set_tokens(builder, Part::Family, &tokens);
    }
}

#[derive(Clone, Copy)]
enum Part {
    Family,
    Given,
    Prefix,
    Suffix,
}

fn set_tokens(builder: &mut NameBuilder, part: Part, tokens: &[&str]) {
    if !tokens.is_empty() {
        set_part(builder, part, &join_name_parts(tokens));
    }
}

fn set_part(builder: &mut NameBuilder, part: Part, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let (stripped, value) = remove_outer(value);
    let part_value = make_part(&value, stripped);
    match part {
        Part::Family => builder.family_part(part_value),
        Part::Given => builder.given_part(part_value),
        Part::Prefix => builder.prefix_part(part_value),
        Part::Suffix => builder.suffix_part(part_value),
    };
}

fn make_part(value: &str, stripped: bool) -> NamePartValue {
    let normalized = normalise_nfc(value);
    NamePartValue::new(
        Literal::new(&normalized),
        initials(&normalized, stripped),
        stripped,
    )
}

fn initials(value: &str, protected: bool) -> Vec<String> {
    let words = if protected {
        vec![value]
    } else {
        split_words(value)
    };
    words
        .into_iter()
        .filter_map(|word| {
            let mut parts = split_top_level(word, '-');
            if parts.len() > 1
                && parts.last().is_some_and(|part| !part.is_empty())
                && parts[0]
                    .chars()
                    .filter(|character| character.is_alphabetic())
                    .all(char::is_lowercase)
            {
                parts.remove(0);
            }
            let initial = parts
                .into_iter()
                .filter_map(initial_segment)
                .collect::<Vec<_>>()
                .join("-");
            (!initial.is_empty()).then_some(initial)
        })
        .collect()
}

fn initial_segment(value: &str) -> Option<String> {
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if matches!(character, '\u{2bb}' | '\u{2bc}' | '\u{2be}' | '\u{2bf}') {
            continue;
        }
        if character == '\\' {
            while chars.peek().is_some_and(|next| next.is_ascii_alphabetic()) {
                chars.next();
            }
            continue;
        }
        if character.is_alphabetic() {
            let mut initial = character.to_string();
            while chars
                .peek()
                .is_some_and(|next| unicode_normalization::char::is_combining_mark(*next))
            {
                initial.push(chars.next().expect("peeked character exists"));
            }
            return Some(initial);
        }
    }
    None
}

fn join_name_parts(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [part] => (*part).to_owned(),
        [first, second] => format!("{first}~{second}"),
        _ => {
            let mut joined = parts[0].to_owned();
            joined.push(if visible_len(parts[0]) < 3 { '~' } else { ' ' });
            joined.push_str(&parts[1..parts.len() - 1].join(" "));
            joined.push('~');
            joined.push_str(parts[parts.len() - 1]);
            joined
        }
    }
}

fn visible_len(value: &str) -> usize {
    value
        .chars()
        .filter(|character| !matches!(character, '{' | '}'))
        .count()
}

fn starts_lowercase(value: &str) -> bool {
    if value.starts_with('{') {
        return false;
    }
    value
        .chars()
        .find(|character| character.is_alphabetic())
        .is_some_and(char::is_lowercase)
}

fn sanitize(value: &str) -> String {
    let mut output = String::new();
    let mut whitespace = false;
    for character in value.trim().chars() {
        if character.is_whitespace() {
            whitespace = true;
        } else {
            if whitespace && !output.is_empty() {
                output.push(' ');
            }
            whitespace = false;
            output.push(character);
        }
    }
    output
}

fn collapse_outer(value: &str) -> (bool, &str) {
    let mut current = value;
    let mut stripped = false;
    while current.starts_with('{') && current.ends_with('}') && outer_encloses(current) {
        stripped = true;
        current = &current[1..current.len() - 1];
    }
    (stripped, current)
}

fn outer_encloses(value: &str) -> bool {
    let mut depth = 0usize;
    for (offset, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && offset + 1 != value.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn validate_braces(value: &str, max_nesting: usize) -> Result<(), ClassicNameDiagnostic> {
    let mut depth = 0usize;
    for (offset, character) in value.char_indices() {
        match character {
            '{' => {
                depth += 1;
                if depth > max_nesting {
                    return Err(diagnostic(
                        ClassicNameDiagnosticKind::Limit,
                        offset,
                        "name brace nesting limit exceeded",
                    ));
                }
            }
            '}' if depth == 0 => {
                return Err(diagnostic(
                    ClassicNameDiagnosticKind::UnbalancedBraces,
                    offset,
                    "unmatched closing brace in name list",
                ));
            }
            '}' => depth -= 1,
            _ => {}
        }
    }
    if depth == 0 {
        Ok(())
    } else {
        Err(diagnostic(
            ClassicNameDiagnosticKind::UnbalancedBraces,
            value.len(),
            "unclosed brace in name list",
        ))
    }
}

fn split_list<'a>(value: &'a str, separators: &[&str]) -> Vec<(usize, &'a str)> {
    let mut result = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut offset = 0usize;
    while offset < value.len() {
        let character = value[offset..]
            .chars()
            .next()
            .expect("valid character boundary");
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => {
                if let Some(separator) = separators.iter().find(|separator| {
                    let end = offset + separator.len();
                    end <= value.len()
                        && value[offset..end].eq_ignore_ascii_case(separator)
                        && boundary_before(value, offset)
                        && boundary_after(value, end)
                }) {
                    result.push((start, &value[start..offset]));
                    offset += separator.len();
                    start = offset;
                    continue;
                }
            }
            _ => {}
        }
        offset += character.len_utf8();
    }
    result.push((start, &value[start..]));
    result
}

fn boundary_before(value: &str, offset: usize) -> bool {
    offset == 0
        || value[..offset]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
}

fn boundary_after(value: &str, offset: usize) -> bool {
    offset == value.len()
        || value[offset..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

fn split_words(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    for (offset, character) in value.char_indices() {
        match character {
            '{' => {
                start.get_or_insert(offset);
                depth += 1;
            }
            '}' => {
                start.get_or_insert(offset);
                depth = depth.saturating_sub(1);
            }
            '~' if depth == 0 => {
                if let Some(begin) = start.take() {
                    result.push(&value[begin..offset]);
                }
            }
            _ if character.is_whitespace() && depth == 0 => {
                if let Some(begin) = start.take() {
                    result.push(&value[begin..offset]);
                }
            }
            _ => {
                start.get_or_insert(offset);
            }
        };
    }
    if let Some(begin) = start {
        result.push(&value[begin..]);
    }
    result
}

fn top_level_delimiters(value: &str, delimiter: char) -> Vec<usize> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    for (offset, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if character == delimiter && depth == 0 => result.push(offset),
            _ => {}
        }
    }
    result
}

fn split_at_positions<'a>(value: &'a str, positions: &[usize]) -> Vec<&'a str> {
    let mut result = Vec::new();
    let mut start = 0usize;
    for &position in positions {
        result.push(value[start..position].trim());
        start = position + 1;
    }
    result.push(value[start..].trim());
    result
}

fn split_top_level(value: &str, delimiter: char) -> Vec<&str> {
    split_at_positions(value, &top_level_delimiters(value, delimiter))
}

fn diagnostic(
    kind: ClassicNameDiagnosticKind,
    offset: usize,
    message: impl Into<String>,
) -> ClassicNameDiagnostic {
    ClassicNameDiagnostic {
        kind,
        offset,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;

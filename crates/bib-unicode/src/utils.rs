use md5::{Digest, Md5};
use unicode_normalization::UnicodeNormalization;

pub fn normalise_nfc(value: &str) -> String {
    value.nfc().collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RangeEnd {
    Number(i64),
    Open,
    Last,
}

pub fn compatibility_hash(value: &str) -> String {
    format!("{:x}", Md5::digest(value.as_bytes()))
}

pub fn normalise_string(value: &str, strip_outer: bool) -> String {
    let value = if strip_outer {
        remove_outer(value.trim()).1
    } else {
        value.trim().to_owned()
    };
    value
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | ',' | ':' | '.' | '–' | '-' | '{' | '}'))
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn normalise_string_underscore(value: &str, strip_outer: bool) -> String {
    let value = if strip_outer {
        remove_outer(value.trim()).1
    } else {
        value.trim().to_owned()
    };
    let mut out = String::new();
    let mut separator = false;
    for c in value.chars() {
        if c.is_alphanumeric() || unicode_normalization::char::is_combining_mark(c) {
            if separator && !out.is_empty() {
                out.push('_');
            }
            separator = false;
            out.push(c);
        } else if c.is_whitespace() || matches!(c, ',' | '-') {
            separator = true;
        }
    }
    out
}

pub fn normalise_string_hash(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' {
            if chars.peek().is_some_and(|next| next.is_alphabetic()) {
                while chars.peek().is_some_and(|next| next.is_alphabetic()) {
                    output.push(chars.next().expect("peeked character exists"));
                }
            } else if let Some(escaped) = chars.next() {
                output.push_str(&(escaped as u32).to_string());
            }
            output.push(':');
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                chars.next();
            }
        } else if !matches!(character, '{' | '}' | '~' | '.') && !character.is_whitespace() {
            output.push(character);
        }
    }
    output
}

pub fn reduce_array<T: Eq + Clone>(values: &[T], removed: &[T]) -> Vec<T> {
    let mut out = Vec::new();
    for value in values {
        if !removed.contains(value) && !out.contains(value) {
            out.push(value.clone());
        }
    }
    out
}

pub fn remove_outer(value: &str) -> (bool, String) {
    if value.starts_with('{') && value.ends_with('}') && balanced_outer(value) {
        (true, value[1..value.len() - 1].to_owned())
    } else {
        (false, value.to_owned())
    }
}

fn balanced_outer(value: &str) -> bool {
    let mut depth = 0;
    for (i, c) in value.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 && i + 1 != value.len() {
                    return false;
                }
            }
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

pub fn range_len(ranges: &[(Option<&str>, Option<&str>)]) -> i64 {
    let mut total = 0;
    for (start, end) in ranges {
        if start.is_none_or(|value| value.is_empty()) {
            return -1;
        }
        let (Some(start), Some(end)) = (
            start.filter(|v| !v.is_empty()),
            end.filter(|v| !v.is_empty()),
        ) else {
            return if end.is_none() && start.is_some() {
                total + 1
            } else {
                -1
            };
        };
        let Some(a) = ordinal(start) else {
            return -1;
        };
        let Some(b) = ordinal(end) else {
            return -1;
        };
        let b = abbreviated_end(start, end).unwrap_or(b);
        total += (b - a).abs() + 1;
    }
    total
}

fn abbreviated_end(start: &str, end: &str) -> Option<i64> {
    let start_number: i64 = start.parse().ok()?;
    let end_number: i64 = end.parse().ok()?;
    if end_number >= start_number {
        return Some(end_number);
    }
    let factor = 10_i64.pow(end.len().try_into().ok()?);
    Some((start_number / factor) * factor + end_number)
}

fn ordinal(value: &str) -> Option<i64> {
    if let Ok(number) = value.parse() {
        return Some(number);
    }
    let normalized: String = value.nfkd().collect();
    let mut total = 0;
    let mut last = 0;
    for c in normalized.to_ascii_uppercase().chars() {
        let n = match c {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            'D' => 500,
            'M' => 1000,
            _ => return None,
        };
        total += if n > last { n - 2 * last } else { n };
        last = n;
    }
    Some(total)
}

pub fn parse_range(value: &str) -> Option<(i64, RangeEnd)> {
    if value.is_empty() {
        return None;
    }
    if let Some(start) = value.strip_suffix("--+") {
        return Some((start.parse().ok()?, RangeEnd::Last));
    }
    if let Some((a, b)) = value.split_once("--") {
        return Some((a.parse().ok()?, RangeEnd::Number(b.parse().ok()?)));
    }
    if let Some(start) = value.strip_suffix('-') {
        return Some((start.parse().ok()?, RangeEnd::Open));
    }
    if let Some(end) = value.strip_prefix('-') {
        return Some((1, RangeEnd::Number(end.parse().ok()?)));
    }
    Some((1, RangeEnd::Number(value.parse().ok()?)))
}

pub fn split_xsv(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quoted = false;
    for (i, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '{' if !quoted => depth += 1,
            '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                out.push(value[start..i].trim().trim_matches('"').to_owned());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(value[start..].trim().trim_matches('"').to_owned());
    out
}

pub fn strip_noinit(value: &str) -> String {
    value
        .replace("\\texttt{", "{")
        .replace("\\texttt ", "")
        .replace("\\bibtexspatium ", "")
        .replace("\\bibtexspatium", "")
        .replace("{}", "")
}

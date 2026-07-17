use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecodeSet {
    Null,
    Base,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TexRecoder {
    decode: RecodeSet,
    encode: RecodeSet,
}

impl TexRecoder {
    pub const fn new(decode: RecodeSet, encode: RecodeSet) -> Self {
        Self { decode, encode }
    }

    pub fn decode(self, input: &str) -> String {
        if self.decode == RecodeSet::Null {
            return input.replace("\\hbox ", "\\hbox");
        }
        let mut out = input.to_owned();
        for (tex, unicode) in base_decode() {
            out = out.replace(tex, unicode);
        }
        if self.decode == RecodeSet::Full {
            for (tex, unicode) in full_decode() {
                out = out.replace(tex, unicode);
            }
        }
        decode_accents(&out).nfc().collect()
    }

    pub fn encode(self, input: &str) -> String {
        let mut out: String = input.nfc().collect();
        for (unicode, tex) in base_encode() {
            out = out.replace(unicode, tex);
        }
        if self.encode == RecodeSet::Full {
            for (unicode, tex) in full_encode() {
                out = out.replace(unicode, tex);
            }
        }
        out
    }
}

fn base_decode() -> &'static [(&'static str, &'static str)] {
    &[
        ("\\textless", "<"),
        ("\\textampersand", "&"),
        ("\\DH{}", "Ð"),
        ("\\dj{}", "đ"),
        ("\\i{}", "ı"),
        ("\\i", "ı"),
        ("\\j{}", "ȷ"),
        ("\\j", "ȷ"),
        ("\\textdegree{}", "°"),
        ("\\textdegree ", "°"),
    ]
}
fn full_decode() -> &'static [(&'static str, &'static str)] {
    &[("\\alpha", "α"), ("\\textdiv", "÷")]
}
fn base_encode() -> &'static [(&'static str, &'static str)] {
    &[
        ("Ð", "\\DH{}"),
        ("đ", "\\dj{}"),
        ("Þ", "\\TH{}"),
        ("Å", "\\r{A}"),
        ("å", "\\r{a}"),
        ("®", "\\textregistered{}"),
        ("°", "\\textdegree{}"),
        ("–", "--"),
        ("ı", "\\i{}"),
    ]
}
fn full_encode() -> &'static [(&'static str, &'static str)] {
    &[
        ("α", "{$\\alpha$}"),
        ("µ", "{$\\mu$}"),
        ("≄", "{$\\not\\simeq$}"),
        ("©", "{$\\copyright$}"),
        ("÷", "{$\\div$}"),
    ]
}

fn decode_accents(input: &str) -> String {
    const ACCENTS: &[(&str, char)] = &[
        ("'", '\u{301}'),
        ("`", '\u{300}'),
        ("^", '\u{302}'),
        ("\"", '\u{308}'),
        ("~", '\u{303}'),
        ("=", '\u{304}'),
        (".", '\u{307}'),
        ("d", '\u{323}'),
        ("c", '\u{327}'),
        ("r", '\u{30a}'),
        ("v", '\u{30c}'),
        ("u", '\u{306}'),
        ("|", '\u{30d}'),
    ];
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\'
            && i + 2 < chars.len()
            && let Some((_, mark)) = ACCENTS.iter().find(|(a, _)| a.starts_with(chars[i + 1]))
        {
            let mut j = i + 2;
            let braced = chars[j] == '{';
            if braced {
                j += 1;
            }
            if chars.get(j) == Some(&'\\') && matches!(chars.get(j + 1), Some('i' | 'j')) {
                j += 1;
            }
            if let Some(&base) = chars.get(j) {
                let base = if base == 'i' {
                    'ı'
                } else if base == 'j' {
                    'ȷ'
                } else {
                    base
                };
                out.push(base);
                out.push(*mark);
                j += 1;
                if braced && chars.get(j) == Some(&'}') {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

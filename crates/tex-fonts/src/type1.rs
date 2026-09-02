//! Detached Type-1 PFB decoding for PDF embedding.

use md5::Digest as _;
use std::collections::BTreeSet;
use umber_hash::{AHash64, HashDomain};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfType1ProgramIdentity([u8; 8]);

impl PdfType1ProgramIdentity {
    #[must_use]
    pub const fn bytes(self) -> [u8; 8] {
        self.0
    }
}

/// PDF-ready Type-1 bytes with the PFB segment framing removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfType1Program {
    identity: PdfType1ProgramIdentity,
    bytes: Vec<u8>,
    length1: u32,
    length2: u32,
    length3: u32,
}

impl PdfType1Program {
    pub fn from_pfb(bytes: &[u8]) -> Result<Self, PdfType1ProgramError> {
        let mut cursor = 0usize;
        let mut decoded = Vec::new();
        let mut lengths = [0u32; 3];
        let mut segment = 0usize;
        while cursor < bytes.len() {
            if bytes.get(cursor..cursor + 2) == Some(&[0x80, 0x03]) {
                cursor += 2;
                break;
            }
            if bytes.get(cursor) != Some(&0x80) {
                return Err(PdfType1ProgramError::BadSegmentMarker);
            }
            let kind = *bytes
                .get(cursor + 1)
                .ok_or(PdfType1ProgramError::TruncatedSegmentHeader)?;
            if segment >= 3 || kind != if segment == 1 { 2 } else { 1 } {
                return Err(PdfType1ProgramError::UnexpectedSegmentKind(kind));
            }
            let length_bytes: [u8; 4] = bytes
                .get(cursor + 2..cursor + 6)
                .ok_or(PdfType1ProgramError::TruncatedSegmentHeader)?
                .try_into()
                .expect("four-byte slice");
            let length = u32::from_le_bytes(length_bytes);
            let end = (cursor + 6)
                .checked_add(length as usize)
                .ok_or(PdfType1ProgramError::SegmentTooLarge)?;
            let data = bytes
                .get(cursor + 6..end)
                .ok_or(PdfType1ProgramError::TruncatedSegmentData)?;
            decoded.extend_from_slice(data);
            lengths[segment] = length;
            segment += 1;
            cursor = end;
        }
        if segment < 2 || cursor != bytes.len() {
            return Err(PdfType1ProgramError::MissingEndMarker);
        }
        let identity = PdfType1ProgramIdentity(
            AHash64::for_bytes(HashDomain::Type1Program, &decoded).to_le_bytes(),
        );
        Ok(Self {
            identity,
            bytes: decoded,
            length1: lengths[0],
            length2: lengths[1],
            length3: lengths[2],
        })
    }

    /// Builds a deterministic PDF-ready subset containing only the named
    /// CharStrings (plus `.notdef`) and the transitive closure of the
    /// subroutines they call.
    ///
    /// Like pdfTeX's `writet1.c::t1_subset_ascii_part` and `t1_read_subrs`,
    /// the cleartext and encrypted private-dictionary prelude are
    /// line-normalized, their subset-invalid `/UniqueID` entries are omitted,
    /// and the encoding is rebuilt from the requested glyphs. PDF does not
    /// need the PFB zero trailer, so subset streams end with the eexec segment.
    pub fn subset(
        &self,
        glyph_names: &BTreeSet<Vec<u8>>,
        subset_font_name: &[u8],
    ) -> Result<Self, PdfType1SubsetError> {
        let clear_end = usize::try_from(self.length1).map_err(|_| PdfType1SubsetError::Overflow)?;
        let encrypted_end = clear_end
            .checked_add(usize::try_from(self.length2).map_err(|_| PdfType1SubsetError::Overflow)?)
            .ok_or(PdfType1SubsetError::Overflow)?;
        let clear = self
            .bytes
            .get(..clear_end)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;
        let encrypted = self
            .bytes
            .get(clear_end..encrypted_end)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;
        let clear = subset_ascii_part(self, clear, glyph_names, subset_font_name)?;
        let decrypted = eexec_crypt(encrypted, false);
        let decrypted = subset_encrypted_prelude(&decrypted)?;
        let subset_plaintext = subset_charstrings_and_subrs(&decrypted, glyph_names)?;
        let encrypted = eexec_crypt(&subset_plaintext, true);
        let mut bytes = Vec::with_capacity(clear.len() + encrypted.len());
        bytes.extend_from_slice(&clear);
        bytes.extend_from_slice(&encrypted);
        let length1 = u32::try_from(clear.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        let length2 = u32::try_from(encrypted.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        Ok(Self {
            identity: PdfType1ProgramIdentity(
                AHash64::for_bytes(HashDomain::Type1Program, &bytes).to_le_bytes(),
            ),
            bytes,
            length1,
            length2,
            length3: 0,
        })
    }

    /// Resolves a code through a cleartext built-in Type-1 encoding array.
    #[must_use]
    pub fn builtin_glyph_name(&self, code: u8) -> Option<Vec<u8>> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let mut tokens = PostScriptTokens::new(cleartext);
        while let Some(token) = tokens.next() {
            if token != PostScriptToken::Word(b"dup") {
                continue;
            }
            let Some(PostScriptToken::Word(encoded_code)) = tokens.next() else {
                continue;
            };
            let Some(PostScriptToken::LiteralName(glyph_name)) = tokens.next() else {
                continue;
            };
            let Some(PostScriptToken::Word(b"put")) = tokens.next() else {
                continue;
            };
            if encoded_code == code.to_string().as_bytes() && !glyph_name.is_empty() {
                return Some(glyph_name.to_vec());
            }
        }
        None
    }

    #[must_use]
    pub const fn identity(&self) -> PdfType1ProgramIdentity {
        self.identity
    }
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    #[must_use]
    pub const fn lengths(&self) -> [u32; 3] {
        [self.length1, self.length2, self.length3]
    }

    /// Reads the cleartext `/FontBBox` without interpreting PostScript.
    #[must_use]
    pub fn font_bbox(&self) -> Option<[i32; 4]> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let marker = b"/FontBBox";
        let start = cleartext
            .windows(marker.len())
            .position(|window| window == marker)?
            + marker.len();
        let mut values = [0; 4];
        let mut count = 0usize;
        for token in cleartext[start..]
            .split(|byte| byte.is_ascii_whitespace() || matches!(byte, b'{' | b'}' | b'[' | b']'))
            .filter(|token| !token.is_empty())
        {
            if count == 4 {
                break;
            }
            let text = std::str::from_utf8(token).ok()?;
            match text.parse::<i32>() {
                Ok(value) => {
                    values[count] = value;
                    count += 1;
                }
                Err(_) if count == 0 => continue,
                Err(_) => return None,
            }
        }
        (count == 4).then_some(values)
    }

    /// Reads `/StdVW`, the Type-1 vertical stem width used by PDF descriptors.
    #[must_use]
    pub fn stem_v(&self) -> Option<i32> {
        self.cleartext_integer(b"/StdVW")
    }

    #[must_use]
    pub fn italic_angle(&self) -> Option<i32> {
        self.cleartext_integer(b"/ItalicAngle")
    }

    #[must_use]
    pub fn is_fixed_pitch(&self) -> bool {
        self.cleartext_value(b"/isFixedPitch")
            .is_some_and(|value| value == b"true")
    }

    fn cleartext_integer(&self, marker: &[u8]) -> Option<i32> {
        let value = self.cleartext_value(marker)?;
        std::str::from_utf8(value).ok()?.parse().ok()
    }

    fn cleartext_value(&self, marker: &[u8]) -> Option<&[u8]> {
        let cleartext = self.bytes.get(..self.length1 as usize)?;
        let start = cleartext
            .windows(marker.len())
            .position(|window| window == marker)?
            + marker.len();
        cleartext[start..]
            .split(|byte| byte.is_ascii_whitespace() || matches!(byte, b'[' | b']' | b'{' | b'}'))
            .find(|token| !token.is_empty())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostScriptToken<'a> {
    Word(&'a [u8]),
    LiteralName(&'a [u8]),
    Delimiter,
}

struct PostScriptTokens<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> PostScriptTokens<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }
}

impl<'a> Iterator for PostScriptTokens<'a> {
    type Item = PostScriptToken<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.cursor += 1;
            }
            if self.bytes.get(self.cursor) != Some(&b'%') {
                break;
            }
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
            {
                self.cursor += 1;
            }
        }

        let first = *self.bytes.get(self.cursor)?;
        if first == b'/' {
            self.cursor += 1;
            let start = self.cursor;
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(|byte| !is_postscript_separator(*byte))
            {
                self.cursor += 1;
            }
            return Some(PostScriptToken::LiteralName(
                &self.bytes[start..self.cursor],
            ));
        }
        if is_postscript_delimiter(first) {
            self.cursor += 1;
            return Some(PostScriptToken::Delimiter);
        }

        let start = self.cursor;
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| !is_postscript_separator(*byte))
        {
            self.cursor += 1;
        }
        Some(PostScriptToken::Word(&self.bytes[start..self.cursor]))
    }
}

const fn is_postscript_separator(byte: u8) -> bool {
    byte.is_ascii_whitespace() || is_postscript_delimiter(byte)
}

const fn is_postscript_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// Reproduces pdfTeX's deterministic six-letter subset tag for a sorted glyph
/// set and PostScript font name. Collision handling is performed by the PDF
/// document assembler, so this function represents round zero.
#[must_use]
pub fn pdftex_subset_tag(glyph_names: &BTreeSet<Vec<u8>>, font_name: &[u8]) -> [u8; 6] {
    let mut digest = md5::Md5::new();
    for glyph in glyph_names {
        digest.update(glyph);
        digest.update(b" ");
    }
    digest.update(font_name);
    digest.update(0i32.to_ne_bytes());
    let digest = digest.finalize();
    let mut rolling = digest[..13]
        .iter()
        .map(|value| i32::from(*value))
        .sum::<i32>();
    let mut tag = [0; 6];
    for index in 0..6 {
        if index > 0 {
            rolling = rolling - i32::from(digest[index - 1]) + i32::from(digest[(index + 12) % 16]);
        }
        tag[index] = (rolling % 26) as u8 + b'A';
    }
    tag
}

fn replace_font_name(cleartext: &[u8], name: &[u8]) -> Result<Vec<u8>, PdfType1SubsetError> {
    let marker = b"/FontName";
    let marker_start = cleartext
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let slash = cleartext[marker_start + marker.len()..]
        .iter()
        .position(|byte| *byte == b'/')
        .map(|offset| marker_start + marker.len() + offset)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let end = cleartext[slash + 1..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .map(|offset| slash + 1 + offset)
        .ok_or(PdfType1SubsetError::MissingFontName)?;
    let mut replaced = Vec::with_capacity(cleartext.len() + name.len());
    replaced.extend_from_slice(&cleartext[..slash + 1]);
    replaced.extend_from_slice(name);
    replaced.extend_from_slice(&cleartext[end..]);
    Ok(replaced)
}

fn subset_ascii_part(
    program: &PdfType1Program,
    cleartext: &[u8],
    glyph_names: &BTreeSet<Vec<u8>>,
    font_name: &[u8],
) -> Result<Vec<u8>, PdfType1SubsetError> {
    let normalized = normalize_type1_ascii(cleartext);
    let normalized = replace_font_name(&normalized, font_name)?;
    let mut encoding_start = None;
    let mut encoding_end = None;
    let mut cursor = 0usize;
    for line in normalized.split_inclusive(|byte| *byte == b'\n') {
        if encoding_start.is_none() && line.starts_with(b"/Encoding") {
            encoding_start = Some(cursor);
        }
        if encoding_start.is_some() && type1_line_ends_with_def(line) {
            encoding_end = Some(cursor + line.len());
            break;
        }
        cursor += line.len();
    }
    let encoding_start = encoding_start.ok_or(PdfType1SubsetError::MissingEncoding)?;
    let encoding_end = encoding_end.ok_or(PdfType1SubsetError::MissingEncoding)?;

    let mut subset = Vec::with_capacity(normalized.len());
    append_without_unique_id(&mut subset, &normalized[..encoding_start], true);
    if &normalized[encoding_start..encoding_end] == b"/Encoding StandardEncoding def\n" {
        subset.extend_from_slice(b"/Encoding StandardEncoding def\n");
    } else {
        subset.extend_from_slice(b"/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n");
        let mut encoded = 0usize;
        for glyph_name in glyph_names {
            let Some(code) = (0u8..=u8::MAX).find(|code| {
                program
                    .builtin_glyph_name(*code)
                    .is_some_and(|name| name == *glyph_name)
            }) else {
                continue;
            };
            subset.extend_from_slice(b"dup ");
            subset.extend_from_slice(code.to_string().as_bytes());
            subset.extend_from_slice(b" /");
            subset.extend_from_slice(glyph_name);
            subset.extend_from_slice(b" put\n");
            encoded += 1;
        }
        if encoded == 0 {
            subset.extend_from_slice(b"dup 0 /.notdef put\n");
        }
        subset.extend_from_slice(b"readonly def\n");
    }
    append_without_unique_id(&mut subset, &normalized[encoding_end..], false);
    Ok(subset)
}

fn normalize_type1_ascii(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut line = Vec::new();
    let mut pending_space = false;
    for byte in bytes.iter().copied().chain(std::iter::once(b'\n')) {
        match byte {
            b'\r' | b'\n' => {
                if !line.is_empty() {
                    line.push(b'\n');
                    normalized.extend_from_slice(&line);
                    line.clear();
                }
                pending_space = false;
            }
            b' ' | b'\t' => {
                if !line.is_empty() {
                    pending_space = true;
                }
            }
            _ => {
                if pending_space {
                    line.push(b' ');
                    pending_space = false;
                }
                line.push(byte);
            }
        }
    }
    normalized
}

fn append_without_unique_id(output: &mut Vec<u8>, bytes: &[u8], only_definitions: bool) {
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let unique_id =
            line.starts_with(b"/UniqueID") && (!only_definitions || type1_line_ends_with_def(line));
        if !unique_id {
            output.extend_from_slice(line);
        }
    }
}

fn type1_line_ends_with_def(line: &[u8]) -> bool {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line == b"def" || line.ends_with(b" def")
}

fn eexec_crypt(bytes: &[u8], encrypt: bool) -> Vec<u8> {
    let mut state = 55_665u16;
    bytes
        .iter()
        .map(|byte| {
            let output = byte ^ (state >> 8) as u8;
            let cipher = if encrypt { output } else { *byte };
            state = (u32::from(cipher) + u32::from(state))
                .wrapping_mul(52_845)
                .wrapping_add(22_719) as u16;
            output
        })
        .collect()
}

/// Applies the line-copying rule from `writet1.c::t1_read_subrs` before the
/// binary Subrs/CharStrings records begin. `t1_start_eexec` consumes the
/// source's first four eexec seed bytes and regenerates them as zeros.
fn subset_encrypted_prelude(plaintext: &[u8]) -> Result<Vec<u8>, PdfType1SubsetError> {
    plaintext
        .get(..4)
        .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
    let source = plaintext
        .get(4..)
        .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
    let boundary =
        encrypted_program_boundary(source).ok_or(PdfType1SubsetError::MissingCharStrings)?;

    let normalized = normalize_type1_ascii(&source[..boundary]);
    let mut subset = Vec::with_capacity(plaintext.len());
    subset.extend_from_slice(&[0; 4]);
    append_without_unique_id(&mut subset, &normalized, false);
    subset.extend_from_slice(&source[boundary..]);
    Ok(subset)
}

fn encrypted_program_boundary(source: &[u8]) -> Option<usize> {
    let mut line_start = 0usize;
    while line_start < source.len() {
        let line_end = source[line_start..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(source.len(), |offset| line_start + offset);
        let line = &source[line_start..line_end];
        if line.starts_with(b"/Subrs")
            || line
                .windows(b"/CharStrings".len())
                .any(|window| window == b"/CharStrings")
        {
            return Some(line_start);
        }
        line_start = line_end;
        while source
            .get(line_start)
            .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
        {
            line_start += 1;
        }
    }
    None
}

#[derive(Clone, Copy)]
struct Type1StringEntry<'a> {
    start: usize,
    end: usize,
    charstring: &'a [u8],
    serialized_tail: &'a [u8],
}

struct Type1Subrs<'a> {
    count_start: usize,
    count_end: usize,
    entries_end: usize,
    entries: Vec<Option<Type1StringEntry<'a>>>,
    token_pair: (&'static [u8], &'static [u8]),
}

fn subset_charstrings_and_subrs(
    plaintext: &[u8],
    glyph_names: &BTreeSet<Vec<u8>>,
) -> Result<Vec<u8>, PdfType1SubsetError> {
    let marker = b"/CharStrings";
    let marker_start = plaintext
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(PdfType1SubsetError::MissingCharStrings)?;
    let len_iv = type1_len_iv(
        plaintext
            .get(4..marker_start)
            .ok_or(PdfType1SubsetError::MalformedCharStrings)?,
    )?;
    let subrs = parse_type1_subrs(plaintext, marker_start)?;
    let mut cursor = marker_start + marker.len();
    skip_space(plaintext, &mut cursor);
    let count_start = cursor;
    let _declared = parse_decimal(plaintext, &mut cursor)?;
    let count_end = cursor;
    let begin = plaintext[cursor..]
        .windows(b"begin".len())
        .position(|window| window == b"begin")
        .map(|offset| cursor + offset + b"begin".len())
        .ok_or(PdfType1SubsetError::MissingCharStrings)?;
    cursor = begin;
    let mut entries = Vec::new();
    loop {
        let entry_start = cursor;
        skip_space(plaintext, &mut cursor);
        if plaintext.get(cursor) != Some(&b'/') {
            cursor = entry_start;
            break;
        }
        cursor += 1;
        let name_start = cursor;
        while plaintext
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        let name = plaintext
            .get(name_start..cursor)
            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
        let entry = parse_type1_string_entry(plaintext, entry_start, &mut cursor)?;
        entries.push((name.to_vec(), entry));
    }
    if entries.is_empty() {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    let mut selected_names = glyph_names.clone();
    selected_names.insert(b".notdef".to_vec());
    let mut used_subrs = vec![false; subrs.as_ref().map_or(0, |value| value.entries.len())];
    for used in used_subrs.iter_mut().take(4) {
        *used = true;
    }
    let mut scanned_names = BTreeSet::new();
    let mut last_other_subr_3 = 3;
    loop {
        // pdfTeX marks `.notdef` before walking the requested glyph tree. This
        // matters for the stateful OtherSubr 3 hint-replacement convention.
        let Some(name) = (!scanned_names.contains(b".notdef".as_slice()))
            .then(|| b".notdef".to_vec())
            .or_else(|| {
                selected_names
                    .iter()
                    .find(|name| !scanned_names.contains(*name))
                    .cloned()
            })
        else {
            break;
        };
        scanned_names.insert(name.clone());
        if let (Some(subrs), Some((_, entry))) = (
            subrs.as_ref(),
            entries.iter().find(|(entry_name, _)| *entry_name == name),
        ) {
            let mut stack = Vec::new();
            Type1CharstringClosure {
                len_iv,
                subrs,
                used_subrs: &mut used_subrs,
                selected_names: &mut selected_names,
                last_other_subr_3: &mut last_other_subr_3,
            }
            .scan(entry.charstring, &mut stack, 0)?;
        }
    }
    let kept = entries
        .iter()
        .filter(|(name, _)| selected_names.contains(name))
        .collect::<Vec<_>>();
    if kept.len() == 1 && glyph_names.iter().any(|name| name.as_slice() != b".notdef") {
        return Err(PdfType1SubsetError::MissingRequestedGlyphs);
    }
    let mut subset = Vec::with_capacity(plaintext.len());
    if let Some(subrs) = subrs {
        subset.extend_from_slice(&plaintext[..subrs.count_start]);
        let highest_used = used_subrs.iter().rposition(|used| *used);
        let emitted_count = highest_used.map_or(0, |index| index + 1);
        subset.extend_from_slice(emitted_count.to_string().as_bytes());
        let first_entry = subrs
            .entries
            .iter()
            .flatten()
            .map(|entry| entry.start)
            .min()
            .unwrap_or(subrs.entries_end);
        subset.extend_from_slice(&plaintext[subrs.count_end..first_entry]);
        let return_charstring = encrypted_return_charstring(len_iv);
        for (index, used) in used_subrs.iter().copied().enumerate().take(emitted_count) {
            subset.extend_from_slice(b"dup ");
            subset.extend_from_slice(index.to_string().as_bytes());
            if used {
                let entry = subrs.entries[index]
                    .as_ref()
                    .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                subset.extend_from_slice(b" ");
                subset.extend_from_slice(entry.charstring.len().to_string().as_bytes());
                subset.extend_from_slice(entry.serialized_tail);
            } else {
                subset.extend_from_slice(b" ");
                subset.extend_from_slice(return_charstring.len().to_string().as_bytes());
                subset.extend_from_slice(subrs.token_pair.0);
                subset.extend_from_slice(b" ");
                subset.extend_from_slice(&return_charstring);
                subset.extend_from_slice(b" ");
                subset.extend_from_slice(subrs.token_pair.1);
                subset.push(b'\n');
            }
        }
        subset.extend_from_slice(&plaintext[subrs.entries_end..count_start]);
    } else {
        subset.extend_from_slice(&plaintext[..count_start]);
    }
    subset.extend_from_slice(kept.len().to_string().as_bytes());
    subset.extend_from_slice(&plaintext[count_end..begin]);
    for (_, entry) in kept {
        subset.extend_from_slice(&plaintext[entry.start..entry.end]);
    }
    subset.extend_from_slice(&subset_type1_trailer(&plaintext[cursor..]));
    Ok(subset)
}

/// Mirrors `writet1.c::t1_subset_end`: its `t1_getline` path normalizes the
/// textual suffix through the eexec close marker, while bytes beyond that
/// marker belong to the next input boundary and are not line-normalized.
fn subset_type1_trailer(bytes: &[u8]) -> Vec<u8> {
    const EEXEC_END: &[u8] = b"mark currentfile closefile";

    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let line_end = bytes[cursor..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(bytes.len(), |offset| cursor + offset);
        let mut next_line = line_end;
        skip_line_ending(bytes, &mut next_line);
        let normalized_line = normalize_type1_ascii(&bytes[cursor..line_end]);
        if normalized_line
            .strip_suffix(b"\n")
            .unwrap_or(&normalized_line)
            .ends_with(EEXEC_END)
        {
            let mut normalized = normalize_type1_ascii(&bytes[..next_line]);
            normalized.extend_from_slice(&bytes[next_line..]);
            return normalized;
        }
        if next_line == cursor {
            break;
        }
        cursor = next_line;
    }
    bytes.to_vec()
}

fn parse_type1_subrs<'a>(
    plaintext: &'a [u8],
    charstrings_start: usize,
) -> Result<Option<Type1Subrs<'a>>, PdfType1SubsetError> {
    let marker = b"/Subrs";
    let Some(marker_start) = plaintext[..charstrings_start]
        .windows(marker.len())
        .rposition(|window| window == marker)
    else {
        return Ok(None);
    };
    let mut cursor = marker_start + marker.len();
    skip_space(plaintext, &mut cursor);
    let count_start = cursor;
    let declared = parse_decimal(plaintext, &mut cursor)?;
    let count_end = cursor;
    while plaintext
        .get(cursor)
        .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
    {
        cursor += 1;
    }
    skip_line_ending(plaintext, &mut cursor);

    let mut entries = vec![None; declared];
    let mut entries_end = cursor;
    let mut token_pair = None;
    loop {
        let entry_start = cursor;
        skip_space(plaintext, &mut cursor);
        if plaintext.get(cursor..cursor + 3) != Some(b"dup")
            || !plaintext
                .get(cursor + 3)
                .is_some_and(u8::is_ascii_whitespace)
        {
            break;
        }
        cursor += 3;
        skip_space(plaintext, &mut cursor);
        let index = parse_decimal(plaintext, &mut cursor)?;
        if index >= declared {
            return Err(PdfType1SubsetError::MalformedCharStrings);
        }
        let entry = parse_type1_string_entry(plaintext, entry_start, &mut cursor)?;
        token_pair.get_or_insert(detect_type1_token_pair(entry.serialized_tail)?);
        entries[index] = Some(entry);
        entries_end = cursor;
    }
    if declared != 0 && token_pair.is_none() {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    Ok(Some(Type1Subrs {
        count_start,
        count_end,
        entries_end,
        entries,
        token_pair: token_pair.unwrap_or((b" RD", b"NP")),
    }))
}

fn parse_type1_string_entry<'a>(
    plaintext: &'a [u8],
    entry_start: usize,
    cursor: &mut usize,
) -> Result<Type1StringEntry<'a>, PdfType1SubsetError> {
    skip_space(plaintext, cursor);
    let length = parse_decimal(plaintext, cursor)?;
    skip_space(plaintext, cursor);
    let token_start = *cursor;
    while plaintext
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        *cursor += 1;
    }
    if token_start == *cursor || !plaintext.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    let serialized_tail_start = token_start
        .checked_sub(1)
        .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
    *cursor += 1;
    let charstring_start = *cursor;
    *cursor = cursor
        .checked_add(length)
        .ok_or(PdfType1SubsetError::Overflow)?;
    let charstring_end = *cursor;
    if charstring_end > plaintext.len() {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    while plaintext
        .get(*cursor)
        .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
    {
        *cursor += 1;
    }
    skip_line_ending(plaintext, cursor);
    Ok(Type1StringEntry {
        start: entry_start,
        end: *cursor,
        charstring: &plaintext[charstring_start..charstring_end],
        serialized_tail: &plaintext[serialized_tail_start..*cursor],
    })
}

fn skip_line_ending(bytes: &[u8], cursor: &mut usize) {
    if bytes.get(*cursor) == Some(&b'\r') {
        *cursor += 1;
    }
    if bytes.get(*cursor) == Some(&b'\n') {
        *cursor += 1;
    }
}

fn detect_type1_token_pair(
    tail: &[u8],
) -> Result<(&'static [u8], &'static [u8]), PdfType1SubsetError> {
    for pair in [
        (b" RD".as_slice(), b"NP".as_slice()),
        (b" -|".as_slice(), b"|".as_slice()),
        (b" RD".as_slice(), b"noaccess put".as_slice()),
        (b" -|".as_slice(), b"noaccess put".as_slice()),
    ] {
        let without_lf = tail.strip_suffix(b"\n").unwrap_or(tail);
        let without_eol = without_lf.strip_suffix(b"\r").unwrap_or(without_lf);
        if tail.starts_with(pair.0) && without_eol.ends_with(pair.1) {
            return Ok(pair);
        }
    }
    Err(PdfType1SubsetError::MalformedCharStrings)
}

fn type1_len_iv(bytes: &[u8]) -> Result<usize, PdfType1SubsetError> {
    let mut len_iv = 4usize;
    for line in bytes.split(|byte| matches!(byte, b'\r' | b'\n')) {
        let Some(rest) = line.strip_prefix(b"/lenIV") else {
            continue;
        };
        let value = std::str::from_utf8(rest)
            .ok()
            .and_then(|rest| rest.split_ascii_whitespace().next())
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
        len_iv = usize::try_from(value).map_err(|_| PdfType1SubsetError::MalformedCharStrings)?;
    }
    Ok(len_iv)
}

fn encrypted_return_charstring(len_iv: usize) -> Vec<u8> {
    let mut plaintext = vec![0; len_iv];
    plaintext.push(11);
    charstring_crypt(&plaintext, true)
}

fn charstring_crypt(bytes: &[u8], encrypt: bool) -> Vec<u8> {
    let mut state = 4_330u16;
    bytes
        .iter()
        .map(|byte| {
            let output = byte ^ (state >> 8) as u8;
            let cipher = if encrypt { output } else { *byte };
            state = (u32::from(cipher) + u32::from(state))
                .wrapping_mul(52_845)
                .wrapping_add(22_719) as u16;
            output
        })
        .collect()
}

struct Type1CharstringClosure<'a, 'b> {
    len_iv: usize,
    subrs: &'b Type1Subrs<'a>,
    used_subrs: &'b mut [bool],
    selected_names: &'b mut BTreeSet<Vec<u8>>,
    last_other_subr_3: &'b mut i32,
}

impl Type1CharstringClosure<'_, '_> {
    fn scan(
        &mut self,
        encrypted: &[u8],
        stack: &mut Vec<i32>,
        depth: usize,
    ) -> Result<(), PdfType1SubsetError> {
        let recursion_limit = self.subrs.entries.len().saturating_add(1);
        if depth > recursion_limit || encrypted.len() < self.len_iv {
            return Err(PdfType1SubsetError::MalformedCharStrings);
        }
        let decrypted = charstring_crypt(encrypted, false);
        let mut cursor = self.len_iv;
        while cursor < decrypted.len() {
            let byte = decrypted[cursor];
            cursor += 1;
            if byte >= 32 {
                let value = match byte {
                    32..=246 => i32::from(byte) - 139,
                    247..=250 => {
                        let next = *decrypted
                            .get(cursor)
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                        cursor += 1;
                        (i32::from(byte) - 247) * 256 + 108 + i32::from(next)
                    }
                    251..=254 => {
                        let next = *decrypted
                            .get(cursor)
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                        cursor += 1;
                        -((i32::from(byte) - 251) * 256) - 108 - i32::from(next)
                    }
                    255 => {
                        let bytes: [u8; 4] = decrypted
                            .get(cursor..cursor + 4)
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?
                            .try_into()
                            .expect("four-byte slice");
                        cursor += 4;
                        i32::from_be_bytes(bytes)
                    }
                    _ => unreachable!(),
                };
                if stack.len() == 24 {
                    return Err(PdfType1SubsetError::MalformedCharStrings);
                }
                stack.push(value);
                continue;
            }

            if byte == 12 {
                let escaped = *decrypted
                    .get(cursor)
                    .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                cursor += 1;
                match escaped {
                    0 => clear_type1_stack(stack, 0)?,
                    1 | 2 => clear_type1_stack(stack, 6)?,
                    6 => {
                        require_type1_stack(stack, 5)?;
                        let base = usize::try_from(stack[3])
                            .ok()
                            .and_then(standard_encoding_glyph_name)
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                        let accent = usize::try_from(stack[4])
                            .ok()
                            .and_then(standard_encoding_glyph_name)
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                        self.selected_names.insert(base.to_vec());
                        self.selected_names.insert(accent.to_vec());
                        stack.clear();
                    }
                    7 => clear_type1_stack(stack, 4)?,
                    12 => {
                        require_type1_stack_at_least(stack, 2)?;
                        stack.truncate(stack.len() - 2);
                        stack.push(0);
                    }
                    16 => {
                        require_type1_stack_at_least(stack, 2)?;
                        let other_subr = stack[stack.len() - 1];
                        let argument_count = usize::try_from(stack[stack.len() - 2])
                            .map_err(|_| PdfType1SubsetError::MalformedCharStrings)?;
                        if other_subr == 3 {
                            require_type1_stack_at_least(stack, 3)?;
                            *self.last_other_subr_3 = stack[stack.len() - 3];
                        }
                        let consumed = argument_count
                            .checked_add(2)
                            .ok_or(PdfType1SubsetError::Overflow)?;
                        require_type1_stack_at_least(stack, consumed)?;
                        stack.truncate(stack.len() - consumed);
                    }
                    17 => {
                        if stack.len() == 24 {
                            return Err(PdfType1SubsetError::MalformedCharStrings);
                        }
                        stack.push(*self.last_other_subr_3);
                    }
                    33 => clear_type1_stack(stack, 2)?,
                    _ => return Err(PdfType1SubsetError::MalformedCharStrings),
                }
                continue;
            }

            match byte {
                1 | 3 | 5 | 21 => clear_type1_stack(stack, 2)?,
                4 | 6 | 7 | 22 => clear_type1_stack(stack, 1)?,
                8 => clear_type1_stack(stack, 6)?,
                9 | 14 => stack.clear(),
                10 => {
                    let index = usize::try_from(
                        stack
                            .pop()
                            .ok_or(PdfType1SubsetError::MalformedCharStrings)?,
                    )
                    .map_err(|_| PdfType1SubsetError::MalformedCharStrings)?;
                    let entry = self
                        .subrs
                        .entries
                        .get(index)
                        .and_then(Option::as_ref)
                        .ok_or(PdfType1SubsetError::MalformedCharStrings)?;
                    self.used_subrs[index] = true;
                    self.scan(entry.charstring, stack, depth + 1)?;
                }
                11 => {}
                13 => clear_type1_stack(stack, 2)?,
                30 | 31 => clear_type1_stack(stack, 4)?,
                _ => return Err(PdfType1SubsetError::MalformedCharStrings),
            }
        }
        Ok(())
    }
}

fn require_type1_stack(stack: &[i32], expected: usize) -> Result<(), PdfType1SubsetError> {
    if stack.len() == expected {
        Ok(())
    } else {
        Err(PdfType1SubsetError::MalformedCharStrings)
    }
}

fn require_type1_stack_at_least(stack: &[i32], expected: usize) -> Result<(), PdfType1SubsetError> {
    if stack.len() >= expected {
        Ok(())
    } else {
        Err(PdfType1SubsetError::MalformedCharStrings)
    }
}

fn clear_type1_stack(stack: &mut Vec<i32>, expected: usize) -> Result<(), PdfType1SubsetError> {
    require_type1_stack(stack, expected)?;
    stack.clear();
    Ok(())
}

fn standard_encoding_glyph_name(code: usize) -> Option<&'static [u8]> {
    const ASCII: [&[u8]; 95] = [
        b"space",
        b"exclam",
        b"quotedbl",
        b"numbersign",
        b"dollar",
        b"percent",
        b"ampersand",
        b"quoteright",
        b"parenleft",
        b"parenright",
        b"asterisk",
        b"plus",
        b"comma",
        b"hyphen",
        b"period",
        b"slash",
        b"zero",
        b"one",
        b"two",
        b"three",
        b"four",
        b"five",
        b"six",
        b"seven",
        b"eight",
        b"nine",
        b"colon",
        b"semicolon",
        b"less",
        b"equal",
        b"greater",
        b"question",
        b"at",
        b"A",
        b"B",
        b"C",
        b"D",
        b"E",
        b"F",
        b"G",
        b"H",
        b"I",
        b"J",
        b"K",
        b"L",
        b"M",
        b"N",
        b"O",
        b"P",
        b"Q",
        b"R",
        b"S",
        b"T",
        b"U",
        b"V",
        b"W",
        b"X",
        b"Y",
        b"Z",
        b"bracketleft",
        b"backslash",
        b"bracketright",
        b"asciicircum",
        b"underscore",
        b"quoteleft",
        b"a",
        b"b",
        b"c",
        b"d",
        b"e",
        b"f",
        b"g",
        b"h",
        b"i",
        b"j",
        b"k",
        b"l",
        b"m",
        b"n",
        b"o",
        b"p",
        b"q",
        b"r",
        b"s",
        b"t",
        b"u",
        b"v",
        b"w",
        b"x",
        b"y",
        b"z",
        b"braceleft",
        b"bar",
        b"braceright",
        b"asciitilde",
    ];
    if let Some(index) = code.checked_sub(0x20).filter(|index| *index < ASCII.len()) {
        return Some(ASCII[index]);
    }
    if code > u8::MAX as usize {
        return None;
    }
    Some(match code {
        0xa1 => b"exclamdown",
        0xa2 => b"cent",
        0xa3 => b"sterling",
        0xa4 => b"fraction",
        0xa5 => b"yen",
        0xa6 => b"florin",
        0xa7 => b"section",
        0xa8 => b"currency",
        0xa9 => b"quotesingle",
        0xaa => b"quotedblleft",
        0xab => b"guillemotleft",
        0xac => b"guilsinglleft",
        0xad => b"guilsinglright",
        0xae => b"fi",
        0xaf => b"fl",
        0xb1 => b"endash",
        0xb2 => b"dagger",
        0xb3 => b"daggerdbl",
        0xb4 => b"periodcentered",
        0xb6 => b"paragraph",
        0xb7 => b"bullet",
        0xb8 => b"quotesinglbase",
        0xb9 => b"quotedblbase",
        0xba => b"quotedblright",
        0xbb => b"guillemotright",
        0xbc => b"ellipsis",
        0xbd => b"perthousand",
        0xbf => b"questiondown",
        0xc1 => b"grave",
        0xc2 => b"acute",
        0xc3 => b"circumflex",
        0xc4 => b"tilde",
        0xc5 => b"macron",
        0xc6 => b"breve",
        0xc7 => b"dotaccent",
        0xc8 => b"dieresis",
        0xca => b"ring",
        0xcb => b"cedilla",
        0xcd => b"hungarumlaut",
        0xce => b"ogonek",
        0xcf => b"caron",
        0xd0 => b"emdash",
        0xe1 => b"AE",
        0xe3 => b"ordfeminine",
        0xe8 => b"Lslash",
        0xe9 => b"Oslash",
        0xea => b"OE",
        0xeb => b"ordmasculine",
        0xf1 => b"ae",
        0xf5 => b"dotlessi",
        0xf8 => b"lslash",
        0xf9 => b"oslash",
        0xfa => b"oe",
        0xfb => b"germandbls",
        _ => b".notdef",
    })
}

fn skip_space(bytes: &[u8], cursor: &mut usize) {
    while bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

fn parse_decimal(bytes: &[u8], cursor: &mut usize) -> Result<usize, PdfType1SubsetError> {
    let start = *cursor;
    while bytes.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if *cursor == start {
        return Err(PdfType1SubsetError::MalformedCharStrings);
    }
    std::str::from_utf8(&bytes[start..*cursor])
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or(PdfType1SubsetError::MalformedCharStrings)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfType1SubsetError {
    InvalidSegments,
    MissingFontName,
    MissingEncoding,
    MissingCharStrings,
    MalformedCharStrings,
    MissingRequestedGlyphs,
    Overflow,
}

impl std::fmt::Display for PdfType1SubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot subset Type-1 font program: {self:?}")
    }
}

impl std::error::Error for PdfType1SubsetError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfType1ProgramError {
    BadSegmentMarker,
    TruncatedSegmentHeader,
    UnexpectedSegmentKind(u8),
    SegmentTooLarge,
    TruncatedSegmentData,
    MissingEndMarker,
}

impl std::fmt::Display for PdfType1ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid Type-1 PFB program: {self:?}")
    }
}
impl std::error::Error for PdfType1ProgramError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_pfb_framing_and_records_pdf_segment_lengths() {
        let parts: &[&[u8]] = &[
            &[0x80, 1, 3, 0, 0, 0],
            b"abc",
            &[0x80, 2, 2, 0, 0, 0],
            b"de",
            &[0x80, 1, 1, 0, 0, 0],
            b"f",
            &[0x80, 3],
        ];
        let pfb = parts.concat();
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");
        assert_eq!(program.bytes(), b"abcdef");
        assert_eq!(program.lengths(), [3, 2, 1]);
        assert_ne!(program.identity().bytes(), [0; 8]);
    }

    #[test]
    fn reads_cleartext_font_bbox_without_postscript_execution() {
        let header = b"%!PS /FontBBox {-40 -250 1009 750 }readonly def /StdVW [69] def /ItalicAngle 0 def /isFixedPitch true def\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
        pfb.extend_from_slice(header);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");
        assert_eq!(program.font_bbox(), Some([-40, -250, 1009, 750]));
        assert_eq!(program.stem_v(), Some(69));
        assert_eq!(program.italic_angle(), Some(0));
        assert!(program.is_fixed_pitch());
    }

    #[test]
    fn resolves_compact_builtin_entries_used_by_corpus_fonts() {
        let header = b"%!PS\n/Encoding 256 array\n\
            dup 10/uni03A9 put\n\
            dup 1/acute put\n\
            dup % encoding comments may separate tokens\n\
              15/d15 put\n\
            dup 0/minus put\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
        pfb.extend_from_slice(header);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");

        assert_eq!(
            program.builtin_glyph_name(10).as_deref(),
            Some(b"uni03A9".as_slice())
        );
        assert_eq!(
            program.builtin_glyph_name(1).as_deref(),
            Some(b"acute".as_slice())
        );
        assert_eq!(
            program.builtin_glyph_name(15).as_deref(),
            Some(b"d15".as_slice())
        );
        assert_eq!(
            program.builtin_glyph_name(0).as_deref(),
            Some(b"minus".as_slice())
        );
        assert_eq!(program.builtin_glyph_name(2), None);
    }

    #[test]
    fn permits_a_subset_whose_only_requested_glyph_is_notdef() {
        let pfb = include_bytes!("../../../tests/corpus/pdf/embedded_type1/cmr10.pfb");
        let program = PdfType1Program::from_pfb(pfb).expect("committed PFB");
        let glyphs = [b".notdef".to_vec()].into_iter().collect::<BTreeSet<_>>();

        let subset = program
            .subset(&glyphs, b"AAAAAA+CMR10")
            .expect("a blank encoded slot still forms a valid subset");
        let decrypted = eexec_crypt(
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize],
            false,
        );
        assert!(decrypted.windows(9).any(|window| window == b"/.notdef "));
        assert!(!decrypted.windows(3).any(|window| window == b"/A "));
    }

    #[test]
    fn subsets_charstrings_separated_by_postscript_carriage_returns() {
        let clear =
            b"%!PS /FontName /Fixture def\r/Encoding StandardEncoding def\rcurrentfile eexec\r";
        let plaintext = [
            b"\0\0\0\0".as_slice(),
            b"/CharStrings 3 dict dup begin\r\
              /.notdef 1 RD x ND\r\
              /space 1 RD y ND\r\
              /A 1 RD z ND\r\
              end\r",
        ]
        .concat();
        let encrypted = eexec_crypt(&plaintext, true);
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(clear.len() as u32).to_le_bytes());
        pfb.extend_from_slice(clear);
        pfb.extend_from_slice(&[0x80, 2]);
        pfb.extend_from_slice(&(encrypted.len() as u32).to_le_bytes());
        pfb.extend_from_slice(&encrypted);
        pfb.extend_from_slice(&[0x80, 3]);
        let program = PdfType1Program::from_pfb(&pfb).expect("valid synthetic PFB");
        let glyphs = [b"space".to_vec()].into_iter().collect::<BTreeSet<_>>();

        let subset = program
            .subset(&glyphs, b"AAAAAA+Fixture")
            .expect("CR-only CharStrings subset");
        let decrypted = eexec_crypt(
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize],
            false,
        );
        assert!(decrypted.windows(7).any(|window| window == b"/space "));
        assert!(!decrypted.windows(3).any(|window| window == b"/A "));
    }

    #[test]
    fn encrypted_prelude_is_normalized_without_touching_subrs() {
        let mut plaintext = vec![1, 2, 3, 4];
        plaintext.extend_from_slice(
            b"dup\r/Private  8 dict dup begin \r/UniqueID 42 def\r/lenIV 4 def\r/Subrs 1 array\r\0binary",
        );

        let subset = subset_encrypted_prelude(&plaintext).expect("synthetic encrypted prelude");
        assert_eq!(
            subset,
            b"\0\0\0\0dup\n/Private 8 dict dup begin\n/lenIV 4 def\n/Subrs 1 array\r\0binary",
        );
    }

    #[test]
    fn subset_trailer_normalizes_only_through_the_eexec_close_line() {
        let trailer = b"  readonly\t put \r\tmark  currentfile closefile \r\x20\x09\x00binary";

        let subset = subset_type1_trailer(trailer);

        assert_eq!(
            subset,
            b"readonly put\nmark currentfile closefile\n\x20\x09\x00binary",
        );
        assert!(
            !subset
                .windows(b"\n mark currentfile closefile".len())
                .any(|window| window == b"\n mark currentfile closefile")
        );
    }

    #[test]
    fn subsets_transitive_subrs_as_a_sparse_unremapped_prefix() {
        let clear =
            b"%!PS\n/FontName /Fixture def\n/Encoding StandardEncoding def\ncurrentfile eexec\n";
        let mut plaintext =
            b"seed/Private 8 dict dup begin\n/lenIV 0 def\n/Subrs 9 array\n".to_vec();
        let subr_plaintexts: [&[u8]; 9] = [
            &[11],
            &[11],
            &[11],
            &[139, 22, 11],
            &[140, 22, 11],
            &[11],
            &[141, 22, 11],
            &[144, 10, 11],
            &[11],
        ];
        let mut encrypted_subrs = Vec::new();
        for (index, charstring) in subr_plaintexts.iter().enumerate() {
            let encrypted = charstring_crypt(charstring, true);
            encrypted_subrs.push(encrypted.clone());
            plaintext.extend_from_slice(format!("dup {index} {} RD ", encrypted.len()).as_bytes());
            plaintext.extend_from_slice(&encrypted);
            plaintext.extend_from_slice(b" NP\n");
        }
        plaintext.extend_from_slice(b"ND\n2 index /CharStrings 3 dict dup begin\n");
        let charstrings = [
            (b".notdef".as_slice(), vec![14]),
            (b"A".as_slice(), vec![146, 10, 14]),
            (b"B".as_slice(), vec![147, 10, 14]),
        ];
        let mut encrypted_charstrings = Vec::new();
        for (name, charstring) in charstrings {
            let encrypted = charstring_crypt(&charstring, true);
            encrypted_charstrings.push((name, encrypted.clone()));
            plaintext.extend_from_slice(
                format!("/{} {} RD ", String::from_utf8_lossy(name), encrypted.len()).as_bytes(),
            );
            plaintext.extend_from_slice(&encrypted);
            plaintext.extend_from_slice(b" ND\n");
        }
        plaintext.extend_from_slice(b"end\nend\n");
        let encrypted = eexec_crypt(&plaintext, true);
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(clear.len() as u32).to_le_bytes());
        pfb.extend_from_slice(clear);
        pfb.extend_from_slice(&[0x80, 2]);
        pfb.extend_from_slice(&(encrypted.len() as u32).to_le_bytes());
        pfb.extend_from_slice(&encrypted);
        pfb.extend_from_slice(&[0x80, 3]);

        let program = PdfType1Program::from_pfb(&pfb).expect("synthetic PFB");
        let glyphs = [b"A".to_vec()].into_iter().collect::<BTreeSet<_>>();
        let subset = program
            .subset(&glyphs, b"AAAAAA+Fixture")
            .expect("transitive sparse Subrs subset");
        let decrypted = eexec_crypt(
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize],
            false,
        );

        assert!(
            decrypted
                .windows(14)
                .any(|window| window == b"/Subrs 8 array")
        );
        assert!(
            !decrypted
                .windows(14)
                .any(|window| window == b"/Subrs 9 array")
        );
        for index in [0, 1, 2, 3, 5, 7] {
            let expected = [
                format!("dup {index} {} RD ", encrypted_subrs[index].len()).into_bytes(),
                encrypted_subrs[index].clone(),
                b" NP\n".to_vec(),
            ]
            .concat();
            assert!(
                decrypted
                    .windows(expected.len())
                    .any(|window| window == expected)
            );
        }
        let encrypted_return = encrypted_return_charstring(0);
        for index in [4, 6] {
            let expected = [
                format!("dup {index} {} RD ", encrypted_return.len()).into_bytes(),
                encrypted_return.clone(),
                b" NP\n".to_vec(),
            ]
            .concat();
            assert!(
                decrypted
                    .windows(expected.len())
                    .any(|window| window == expected)
            );
        }
        let unchanged_a = [
            format!("/A {} RD ", encrypted_charstrings[1].1.len()).into_bytes(),
            encrypted_charstrings[1].1.clone(),
            b" ND\n".to_vec(),
        ]
        .concat();
        assert!(
            decrypted
                .windows(unchanged_a.len())
                .any(|window| window == unchanged_a)
        );
        assert!(!decrypted.windows(3).any(|window| window == b"/B "));
        assert!(!decrypted.windows(6).any(|window| window == b"dup 8 "));
    }

    #[test]
    fn subsets_committed_cmr_charstrings_and_matches_pdftex_tag() {
        let pfb = include_bytes!("../../../tests/corpus/pdf/embedded_type1/cmr10.pfb");
        let program = PdfType1Program::from_pfb(pfb).expect("committed PFB");
        let glyphs = [b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let tag = pdftex_subset_tag(&glyphs, b"CMR10");
        assert_eq!(&tag, b"QBBONQ");
        let subset_name = [tag.as_slice(), b"+CMR10"].concat();
        let subset = program
            .subset(&glyphs, &subset_name)
            .expect("committed CMR subsets");
        assert!(subset.bytes().len() < program.bytes().len());
        assert_eq!(subset.lengths(), [1391, 7337, 0]);
        assert_eq!(
            format!("{:x}", md5::Md5::digest(subset.bytes())),
            "fce9d1c28cd155a89a22e437b3c33f91",
            "the complete program must stay byte-exact with pinned pdfTeX",
        );
        assert!(
            subset
                .bytes()
                .windows(b"/FontName /QBBONQ+CMR10".len())
                .any(|window| window == b"/FontName /QBBONQ+CMR10")
        );
        let decrypted = eexec_crypt(
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize],
            false,
        );
        assert!(decrypted.ends_with(b"\nmark currentfile closefile\n"));
        let charstrings = decrypted
            .windows(b"/CharStrings".len())
            .position(|window| window == b"/CharStrings")
            .expect("CMR10 has CharStrings");
        assert_eq!(
            format!("{:x}", md5::Md5::digest(&decrypted[..charstrings])),
            "3fded621e7054c5969ec9f611a072a31",
            "the Subrs prefix must stay byte-exact with pinned pdfTeX",
        );
        for glyph in [b"/.notdef ".as_slice(), b"/A ", b"/B ", b"/C "] {
            assert!(
                decrypted.windows(glyph.len()).any(|window| window == glyph),
                "missing {}",
                String::from_utf8_lossy(glyph)
            );
        }
        assert!(!decrypted.windows(3).any(|window| window == b"/D "));
    }

    #[test]
    fn subset_preludes_match_pdftex_and_reject_the_unfiltered_control() {
        let pfb = include_bytes!("../../../tests/corpus/pdf/embedded_type1/cmr10.pfb");
        let program = PdfType1Program::from_pfb(pfb).expect("committed PFB");
        let glyphs = [b"A".to_vec(), b"B".to_vec(), b"C".to_vec()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let original_clear = &program.bytes()[..program.length1 as usize];

        let subset = program
            .subset(&glyphs, b"QBBONQ+CMR10")
            .expect("committed CMR subset");
        let clear = &subset.bytes()[..subset.length1 as usize];
        let unique_id_definition = b"/UniqueID 5000793 def";
        assert_eq!(
            format!("{:x}", md5::Md5::digest(clear)),
            "1d2f7f176577933a65bb76a39a86b955",
            "cleartext must stay byte-exact with the pinned pdfTeX stream",
        );
        assert!(
            original_clear
                .windows(unique_id_definition.len())
                .any(|window| window == unique_id_definition)
        );
        assert!(
            original_clear
                .windows(12)
                .any(|window| window == b"dup 0 /Gamma")
        );
        assert!(
            !clear
                .windows(unique_id_definition.len())
                .any(|window| window == unique_id_definition)
        );
        assert!(!clear.windows(12).any(|window| window == b"dup 0 /Gamma"));
        assert!(clear.windows(13).any(|window| window == b"dup 65 /A put"));
        assert!(clear.ends_with(b"currentfile eexec\n"));

        let original_encrypted = &program.bytes()
            [program.length1 as usize..(program.length1 + program.length2) as usize];
        let original_plaintext = eexec_crypt(original_encrypted, false);
        let private_unique_id = b"/UniqueID 5000793 def";
        assert!(
            original_plaintext
                .windows(private_unique_id.len())
                .any(|window| window == private_unique_id),
            "the unfiltered encrypted prelude is the negative control",
        );

        let encrypted =
            &subset.bytes()[subset.length1 as usize..(subset.length1 + subset.length2) as usize];
        let plaintext = eexec_crypt(encrypted, false);
        let subrs = plaintext
            .windows(b"/Subrs".len())
            .position(|window| window == b"/Subrs")
            .expect("CMR10 has a Subrs array");
        let encrypted_prelude = &plaintext[..subrs];
        assert_eq!(
            format!("{:x}", md5::Md5::digest(encrypted_prelude)),
            "d14befc268a732887ef2b44c876b5abc",
            "encrypted prelude must stay byte-exact with pinned pdfTeX",
        );
        assert!(
            !encrypted_prelude
                .windows(private_unique_id.len())
                .any(|window| window == private_unique_id),
            "pdfTeX suppresses private-dictionary UniqueID while subsetting",
        );
    }
}

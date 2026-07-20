use super::{FormatGlue, FormatMacro, FormatName, FormatToken, StoreFormat, StoreFormatError};
use crate::ContentHash;
use crate::glue::{GlueSpec, GlueStore, Order};
use crate::interner::{ControlSequenceKind, Interner, semantic_atom};
use crate::macro_store::{MacroMeaning, MacroParameterPattern, MacroStore};
use crate::scaled::Scaled;
use crate::token::{Catcode, FrozenToken, Token};
use crate::token_store::{TokenSemanticIdBuilder, TokenStore};

pub(crate) const NAMES_SECTION: u32 = 256;
pub(crate) const NAMES_LOOKUP_SECTION: u32 = 257;
pub(crate) const TOKEN_LISTS_SECTION: u32 = 272;
pub(crate) const MACROS_SECTION: u32 = 288;
pub(crate) const GLUE_SECTION: u32 = 304;

const SECTION_VERSION: u32 = 1;
const NAMES_HEADER: usize = 24;
const NAME_RECORD: usize = 24;
const TOKENS_HEADER: usize = 24;
const TOKEN_SECTION_VERSION: u32 = 2;
const TOKEN_RECORD: usize = 16;
const LEGACY_TOKEN_RECORD: usize = 24;
const MACROS_HEADER: usize = 16;
const MACRO_RECORD: usize = 16;
const GLUE_HEADER: usize = 16;
const GLUE_RECORD: usize = 24;

pub(crate) struct EncodedFrozenCore {
    pub names: Vec<u8>,
    pub names_lookup: Vec<u8>,
    pub token_lists: Vec<u8>,
    pub macros: Vec<u8>,
    pub glue: Vec<u8>,
}

pub(crate) struct FrozenCoreSections<'a> {
    pub names: &'a [u8],
    pub names_lookup: &'a [u8],
    pub token_lists: &'a [u8],
    pub macros: &'a [u8],
    pub glue: &'a [u8],
    pub checksum: u64,
}

pub(crate) struct DecodedFrozenCore {
    pub interner: Interner,
    pub tokens: TokenStore,
    pub macros: MacroStore,
    pub glue: GlueStore,
    pub names: Vec<FormatName>,
    pub token_lists: Vec<Vec<FormatToken>>,
    pub macro_rows: Vec<FormatMacro>,
    pub glue_rows: Vec<FormatGlue>,
}

pub(crate) fn encode(format: &StoreFormat) -> Result<EncodedFrozenCore, StoreFormatError> {
    Ok(EncodedFrozenCore {
        names: encode_names(&format.names)?,
        names_lookup: encode_name_lookup(&format.names)?,
        token_lists: encode_tokens(&format.names, &format.token_lists)?,
        macros: encode_macros(&format.macros)?,
        glue: encode_glue(&format.glue)?,
    })
}

pub(crate) fn decode(
    sections: FrozenCoreSections<'_>,
) -> Result<DecodedFrozenCore, StoreFormatError> {
    let name_lookup =
        crate::frozen_lookup::decode(sections.names_lookup, name_count(sections.names)?)
            .map_err(StoreFormatError::Invalid)?;
    let (interner, names) = decode_names(sections.names, name_lookup, sections.checksum)?;
    let (tokens, token_lists) = decode_tokens(sections.token_lists, &interner, sections.checksum)?;
    let (macros, macro_rows) = decode_macros(sections.macros, &tokens)?;
    let (glue, glue_rows) = decode_glue(sections.glue, sections.checksum)?;
    Ok(DecodedFrozenCore {
        interner,
        tokens,
        macros,
        glue,
        names,
        token_lists,
        macro_rows,
        glue_rows,
    })
}

fn name_count(bytes: &[u8]) -> Result<usize, StoreFormatError> {
    if bytes.len() < NAMES_HEADER {
        return Err(StoreFormatError::Invalid("frozen names"));
    }
    Ok(read_u32(bytes, 4) as usize)
}

fn name_key(name: &FormatName) -> Vec<u8> {
    let mut key = Vec::with_capacity(name.text.len() + 1);
    key.push(u8::from(name.active));
    key.extend_from_slice(name.text.as_bytes());
    key
}

fn encode_name_lookup(names: &[FormatName]) -> Result<Vec<u8>, StoreFormatError> {
    crate::frozen_lookup::encode(
        names
            .iter()
            .enumerate()
            .map(|(index, name)| Ok((name_key(name), u32_count(index, "frozen name index")?)))
            .collect::<Result<Vec<_>, StoreFormatError>>()?,
    )
    .map_err(StoreFormatError::Invalid)
}

fn encode_names(names: &[FormatName]) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32_count(names.len(), "frozen name count")?;
    let records_len = checked_len(names.len(), NAME_RECORD, "frozen name records")?;
    let strings_offset = NAMES_HEADER
        .checked_add(records_len)
        .ok_or(StoreFormatError::Invalid("frozen name section overflow"))?;
    let strings_len: usize = names.iter().try_fold(0_usize, |total, name| {
        total
            .checked_add(name.text.len())
            .ok_or(StoreFormatError::Invalid("frozen name bytes overflow"))
    })?;
    let total = strings_offset
        .checked_add(strings_len)
        .ok_or(StoreFormatError::Invalid("frozen name section overflow"))?;
    let mut out = vec![0; total];
    write_header(&mut out, NAMES_HEADER, count, strings_offset, strings_len)?;
    let mut string_cursor = 0_usize;
    for (index, name) in names.iter().enumerate() {
        let record = NAMES_HEADER + index * NAME_RECORD;
        out[record] = u8::from(name.active);
        put_u32(
            &mut out,
            record + 4,
            u32_count(string_cursor, "frozen name offset")?,
        );
        put_u32(
            &mut out,
            record + 8,
            u32_count(name.text.len(), "frozen name length")?,
        );
        let kind = if name.active {
            ControlSequenceKind::ActiveCharacter
        } else {
            ControlSequenceKind::Named
        };
        put_u64(&mut out, record + 16, semantic_atom(kind, &name.text));
        let start = strings_offset + string_cursor;
        out[start..start + name.text.len()].copy_from_slice(name.text.as_bytes());
        string_cursor += name.text.len();
    }
    Ok(out)
}

fn decode_names(
    bytes: &[u8],
    lookup: crate::frozen_lookup::FrozenLookup,
    checksum: u64,
) -> Result<(Interner, Vec<FormatName>), StoreFormatError> {
    let (count, strings_offset, strings_len) =
        read_header(bytes, NAMES_HEADER, NAME_RECORD, "frozen names")?;
    if strings_offset
        .checked_add(strings_len)
        .is_none_or(|end| end != bytes.len())
    {
        return Err(StoreFormatError::Invalid("frozen name byte range"));
    }
    let arena = String::from_utf8(bytes[strings_offset..].to_vec())
        .map_err(|_| StoreFormatError::Invalid("frozen name bytes are not UTF-8"))?;
    let mut rows = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(count);
    let mut kinds = Vec::with_capacity(count);
    let mut atoms = Vec::with_capacity(count);
    let mut cursor = 0_u32;
    for index in 0..count {
        let record = NAMES_HEADER + index * NAME_RECORD;
        if bytes[record] > 1
            || bytes[record + 1..record + 4].iter().any(|byte| *byte != 0)
            || read_u32(bytes, record + 12) != 0
        {
            return Err(StoreFormatError::Invalid("invalid frozen name record"));
        }
        let start = read_u32(bytes, record + 4);
        let len = read_u32(bytes, record + 8);
        if start != cursor {
            return Err(StoreFormatError::Invalid("non-canonical frozen name span"));
        }
        cursor = start
            .checked_add(len)
            .ok_or(StoreFormatError::Invalid("frozen name span overflow"))?;
        if cursor as usize > arena.len() {
            return Err(StoreFormatError::Invalid("frozen name span out of bounds"));
        }
        let text = arena
            .get(start as usize..cursor as usize)
            .ok_or(StoreFormatError::Invalid(
                "frozen name span is not UTF-8 aligned",
            ))?;
        let active = bytes[record] == 1;
        let kind = if active {
            ControlSequenceKind::ActiveCharacter
        } else {
            ControlSequenceKind::Named
        };
        let atom = read_u64(bytes, record + 16);
        if atom != semantic_atom(kind, text) {
            return Err(StoreFormatError::Invalid("frozen name semantic atom"));
        }
        rows.push(FormatName {
            active,
            text: text.to_owned(),
        });
        spans.push((start, len));
        kinds.push(kind);
        atoms.push(atom);
    }
    if cursor as usize != arena.len() {
        return Err(StoreFormatError::Invalid("unused frozen name bytes"));
    }
    lookup
        .validate_targets(&rows.iter().map(name_key).collect::<Vec<_>>())
        .and_then(|()| lookup.spot_check(checksum))
        .map_err(StoreFormatError::Invalid)?;
    let interner = Interner::from_frozen(arena, spans, kinds, atoms, lookup)
        .map_err(StoreFormatError::Invalid)?;
    Ok((interner, rows))
}

fn encode_tokens(
    names: &[FormatName],
    lists: &[Vec<FormatToken>],
) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32_count(lists.len(), "frozen token-list count")?;
    let records_len = checked_len(lists.len(), TOKEN_RECORD, "frozen token-list records")?;
    let words_offset = TOKENS_HEADER
        .checked_add(records_len)
        .ok_or(StoreFormatError::Invalid("frozen token section overflow"))?;
    let word_count: usize = lists.iter().try_fold(0_usize, |total, list| {
        total
            .checked_add(list.len())
            .ok_or(StoreFormatError::Invalid("frozen token count overflow"))
    })?;
    let words_end = words_offset
        .checked_add(checked_len(word_count, 4, "frozen token words")?)
        .ok_or(StoreFormatError::Invalid("frozen token section overflow"))?;
    let mut keys = Vec::with_capacity(lists.len());
    let mut out = vec![0; words_end];
    write_header(&mut out, TOKENS_HEADER, count, words_offset, word_count)?;
    put_u32(&mut out, 0, TOKEN_SECTION_VERSION);
    let mut word_cursor = 0_usize;
    for (index, list) in lists.iter().enumerate() {
        let record = TOKENS_HEADER + index * TOKEN_RECORD;
        put_u32(
            &mut out,
            record,
            u32_count(word_cursor, "frozen token start")?,
        );
        put_u32(
            &mut out,
            record + 4,
            u32_count(list.len(), "frozen token length")?,
        );
        let mut semantic = TokenSemanticIdBuilder::new();
        let mut hash = crate::frozen_lookup::FrozenWordHasher::new();
        for token in list {
            let (word, runtime, atom) = encode_token_v2(token, names)?;
            let offset = words_offset + word_cursor * 4;
            put_u32(&mut out, offset, word);
            hash.push_u32(word);
            semantic.push(runtime, atom);
            word_cursor += 1;
        }
        let semantic_id = semantic.finish().value();
        put_u64(&mut out, record + 8, semantic_id);
        let _ = index;
        keys.push(hash.finish());
    }
    out.extend_from_slice(
        &crate::frozen_lookup::encode_direct(&keys).map_err(StoreFormatError::Invalid)?,
    );
    Ok(out)
}

fn decode_tokens(
    bytes: &[u8],
    interner: &Interner,
    checksum: u64,
) -> Result<(TokenStore, Vec<Vec<FormatToken>>), StoreFormatError> {
    if bytes.len() < TOKENS_HEADER {
        return Err(StoreFormatError::Invalid("frozen token lists"));
    }
    if read_u32(bytes, 0) == SECTION_VERSION {
        return decode_tokens_v1(bytes, interner, checksum);
    }
    if read_u32(bytes, 0) != TOKEN_SECTION_VERSION
        || read_u32(bytes, 8) != TOKENS_HEADER as u32
        || read_u32(bytes, 20) != 0
    {
        return Err(StoreFormatError::Invalid("frozen token lists"));
    }
    let count = read_u32(bytes, 4) as usize;
    let words_offset = TOKENS_HEADER
        .checked_add(checked_len(count, TOKEN_RECORD, "frozen token lists")?)
        .ok_or(StoreFormatError::Invalid("frozen token lists"))?;
    if read_u32(bytes, 12) as usize != words_offset {
        return Err(StoreFormatError::Invalid("frozen token lists"));
    }
    let word_count = read_u32(bytes, 16) as usize;
    let words_end = words_offset
        .checked_add(checked_len(word_count, 4, "frozen token words")?)
        .ok_or(StoreFormatError::Invalid("frozen token word range"))?;
    if words_end > bytes.len() {
        return Err(StoreFormatError::Invalid("frozen token word range"));
    }
    let mut arena = Vec::with_capacity(word_count);
    let mut rows = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(count);
    let mut semantic_ids = Vec::with_capacity(count);
    let mut hashes = Vec::with_capacity(count);
    let mut cursor = 0_u32;
    for index in 0..count {
        let record = TOKENS_HEADER + index * TOKEN_RECORD;
        let start = read_u32(bytes, record);
        let len = read_u32(bytes, record + 4);
        if start != cursor {
            return Err(StoreFormatError::Invalid(
                "invalid frozen token-list record",
            ));
        }
        cursor = start
            .checked_add(len)
            .ok_or(StoreFormatError::Invalid("frozen token span overflow"))?;
        if cursor as usize > word_count {
            return Err(StoreFormatError::Invalid("frozen token span out of bounds"));
        }
        let mut row = Vec::with_capacity(len as usize);
        let mut semantic = TokenSemanticIdBuilder::new();
        let mut hash = crate::frozen_lookup::FrozenWordHasher::new();
        for word_index in start..cursor {
            let word = read_u32(bytes, words_offset + word_index as usize * 4);
            hash.push_u32(word);
            let (format, runtime, atom) = decode_token_v2(word, interner)?;
            row.push(format);
            arena.push(runtime);
            semantic.push(runtime, atom);
        }
        let expected = read_u64(bytes, record + 8);
        let semantic_id = semantic.finish();
        if semantic_id.value() != expected {
            return Err(StoreFormatError::Invalid("frozen token semantic identity"));
        }
        rows.push(row);
        spans.push((start, len));
        semantic_ids.push(semantic_id);
        hashes.push(hash.finish());
    }
    if cursor as usize != word_count {
        return Err(StoreFormatError::Invalid("unused frozen token words"));
    }
    let lookup = crate::frozen_lookup::decode_direct(&bytes[words_end..], &hashes)
        .map_err(StoreFormatError::Invalid)?;
    let tokens = TokenStore::from_frozen(
        arena,
        spans,
        semantic_ids,
        crate::token_store::FrozenTokenLookup::Direct(lookup),
    )
    .map_err(StoreFormatError::Invalid)?;
    Ok((tokens, rows))
}

fn decode_tokens_v1(
    bytes: &[u8],
    interner: &Interner,
    checksum: u64,
) -> Result<(TokenStore, Vec<Vec<FormatToken>>), StoreFormatError> {
    let (count, words_offset, word_count) = read_header(
        bytes,
        TOKENS_HEADER,
        LEGACY_TOKEN_RECORD,
        "frozen token lists",
    )?;
    let words_len = checked_len(word_count, 8, "frozen token words")?;
    let words_end = words_offset
        .checked_add(words_len)
        .ok_or(StoreFormatError::Invalid("frozen token word range"))?;
    if words_end > bytes.len() {
        return Err(StoreFormatError::Invalid("frozen token word range"));
    }
    let mut arena = Vec::with_capacity(word_count);
    let mut rows = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(count);
    let mut semantic_ids = Vec::with_capacity(count);
    let mut keys = Vec::with_capacity(count);
    let mut cursor = 0_u32;
    for index in 0..count {
        let record = TOKENS_HEADER + index * LEGACY_TOKEN_RECORD;
        let start = read_u32(bytes, record);
        let len = read_u32(bytes, record + 4);
        if start != cursor || read_u64(bytes, record + 16) != 0 {
            return Err(StoreFormatError::Invalid(
                "invalid frozen token-list record",
            ));
        }
        cursor = start
            .checked_add(len)
            .ok_or(StoreFormatError::Invalid("frozen token span overflow"))?;
        if cursor as usize > word_count {
            return Err(StoreFormatError::Invalid("frozen token span out of bounds"));
        }
        let mut row = Vec::with_capacity(len as usize);
        let mut semantic = TokenSemanticIdBuilder::new();
        let mut key = Vec::with_capacity(len as usize * 8);
        for word_index in start..cursor {
            let word = read_u64(bytes, words_offset + word_index as usize * 8);
            key.extend_from_slice(&word.to_le_bytes());
            let (format, runtime, atom) = decode_token(word, interner)?;
            row.push(format);
            arena.push(runtime);
            semantic.push(runtime, atom);
        }
        let expected = read_u64(bytes, record + 8);
        let semantic_id = semantic.finish();
        if semantic_id.value() != expected {
            return Err(StoreFormatError::Invalid("frozen token semantic identity"));
        }
        rows.push(row);
        spans.push((start, len));
        semantic_ids.push(semantic_id);
        keys.push(key);
    }
    if cursor as usize != word_count {
        return Err(StoreFormatError::Invalid("unused frozen token words"));
    }
    let lookup = crate::frozen_lookup::decode(&bytes[words_end..], count)
        .and_then(|lookup| {
            lookup.validate_targets(&keys)?;
            lookup.spot_check(checksum)?;
            Ok(lookup)
        })
        .map_err(StoreFormatError::Invalid)?;
    let tokens = TokenStore::from_frozen(
        arena,
        spans,
        semantic_ids,
        crate::token_store::FrozenTokenLookup::Legacy(lookup),
    )
    .map_err(StoreFormatError::Invalid)?;
    Ok((tokens, rows))
}

type SemanticAtom = (u64, ContentHash);
type DecodedToken = (FormatToken, Token, Option<SemanticAtom>);

fn encode_token_v2(
    token: &FormatToken,
    names: &[FormatName],
) -> Result<(u32, Token, Option<SemanticAtom>), StoreFormatError> {
    const CS_TAG: u32 = 1 << 30;
    const PARAM_TAG: u32 = 2 << 30;
    const FROZEN_TAG: u32 = 3 << 30;
    Ok(match *token {
        FormatToken::Char { ch, cat } => {
            let catcode = catcode(cat)?;
            (
                u32::from(cat) << 21 | ch as u32,
                Token::Char { ch, cat: catcode },
                None,
            )
        }
        FormatToken::Cs(raw) => {
            if raw >= CS_TAG {
                return Err(StoreFormatError::Invalid(
                    "frozen token symbol exceeds 30 bits",
                ));
            }
            let name = names
                .get(raw as usize)
                .ok_or(StoreFormatError::Invalid("frozen token symbol reference"))?;
            let kind = if name.active {
                ControlSequenceKind::ActiveCharacter
            } else {
                ControlSequenceKind::Named
            };
            (
                CS_TAG | raw,
                Token::Cs(crate::interner::Symbol::new(raw)),
                Some(strong_semantic_atom(kind, &name.text)),
            )
        }
        FormatToken::Param(slot) => (PARAM_TAG | u32::from(slot), Token::Param(slot), None),
        FormatToken::Frozen(kind @ 0..=1) => (
            FROZEN_TAG | u32::from(kind),
            if kind == 0 {
                Token::Frozen(FrozenToken::END_TEMPLATE)
            } else {
                Token::Frozen(FrozenToken::END_V)
            },
            None,
        ),
        FormatToken::Frozen(_) => return Err(StoreFormatError::Invalid("frozen token kind")),
    })
}

fn decode_token_v2(word: u32, interner: &Interner) -> Result<DecodedToken, StoreFormatError> {
    let tag = word >> 30;
    let payload = word & 0x3fff_ffff;
    match tag {
        0 => {
            let code = payload & 0x001f_ffff;
            let cat = (payload >> 21) as u8;
            if cat > 15 {
                return Err(StoreFormatError::Invalid("frozen token catcode"));
            }
            let ch =
                char::from_u32(code).ok_or(StoreFormatError::Invalid("frozen token character"))?;
            let catcode = catcode(cat)?;
            Ok((
                FormatToken::Char { ch, cat },
                Token::Char { ch, cat: catcode },
                None,
            ))
        }
        1 => {
            let symbol = interner
                .symbol_at_slot(payload)
                .ok_or(StoreFormatError::Invalid("frozen token symbol reference"))?;
            Ok((
                FormatToken::Cs(payload),
                Token::Cs(symbol),
                interner.semantic_atom(symbol).map(|fingerprint| {
                    let symbol = interner
                        .resolve_stored(symbol)
                        .expect("decoded frozen symbol should resolve to its stored identity");
                    let (_, identity) =
                        strong_semantic_atom(interner.kind_id(symbol), interner.resolve_id(symbol));
                    (fingerprint, identity)
                }),
            ))
        }
        2 if payload <= u32::from(u8::MAX) => Ok((
            FormatToken::Param(payload as u8),
            Token::Param(payload as u8),
            None,
        )),
        3 if payload <= 1 => Ok((
            FormatToken::Frozen(payload as u8),
            if payload == 0 {
                Token::Frozen(FrozenToken::END_TEMPLATE)
            } else {
                Token::Frozen(FrozenToken::END_V)
            },
            None,
        )),
        _ => Err(StoreFormatError::Invalid("invalid frozen token word")),
    }
}

fn decode_token(word: u64, interner: &Interner) -> Result<DecodedToken, StoreFormatError> {
    let tag = word >> 56;
    let payload = word & 0x00ff_ffff_ffff_ffff;
    match tag {
        0 if payload >> 40 == 0 => {
            let code = payload as u32;
            let cat = (payload >> 32) as u8;
            let ch =
                char::from_u32(code).ok_or(StoreFormatError::Invalid("frozen token character"))?;
            let catcode = catcode(cat)?;
            Ok((
                FormatToken::Char { ch, cat },
                Token::Char { ch, cat: catcode },
                None,
            ))
        }
        1 if payload <= u64::from(u32::MAX) => {
            let raw = payload as u32;
            let symbol = interner
                .symbol_at_slot(raw)
                .ok_or(StoreFormatError::Invalid("frozen token symbol reference"))?;
            Ok((
                FormatToken::Cs(raw),
                Token::Cs(symbol),
                interner.semantic_atom(symbol).map(|fingerprint| {
                    let symbol = interner
                        .resolve_stored(symbol)
                        .expect("decoded frozen symbol should resolve to its stored identity");
                    let kind = interner.kind_id(symbol);
                    let name = interner.resolve_id(symbol);
                    let (_, identity) = strong_semantic_atom(kind, name);
                    (fingerprint, identity)
                }),
            ))
        }
        2 if payload <= u64::from(u8::MAX) => Ok((
            FormatToken::Param(payload as u8),
            Token::Param(payload as u8),
            None,
        )),
        3 if payload <= 1 => Ok((
            FormatToken::Frozen(payload as u8),
            if payload == 0 {
                Token::Frozen(FrozenToken::END_TEMPLATE)
            } else {
                Token::Frozen(FrozenToken::END_V)
            },
            None,
        )),
        _ => Err(StoreFormatError::Invalid("invalid frozen token word")),
    }
}

fn strong_semantic_atom(kind: ControlSequenceKind, name: &str) -> (u64, ContentHash) {
    let mut bytes = Vec::with_capacity(name.len() + 1);
    bytes.push(match kind {
        ControlSequenceKind::Named => 0,
        ControlSequenceKind::ActiveCharacter => 1,
    });
    bytes.extend_from_slice(name.as_bytes());
    (
        semantic_atom(kind, name),
        crate::state_hash::semantic_identity_bytes(b"umber-control-sequence-v1", &bytes),
    )
}

fn encode_macros(macros: &[FormatMacro]) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32_count(macros.len(), "frozen macro count")?;
    let records_len = checked_len(macros.len(), MACRO_RECORD, "frozen macro records")?;
    let mut out = vec![0; MACROS_HEADER + records_len];
    put_u32(&mut out, 0, SECTION_VERSION);
    put_u32(&mut out, 4, count);
    put_u32(&mut out, 8, MACROS_HEADER as u32);
    for (index, row) in macros.iter().enumerate() {
        if row.flags & !0x0f != 0 {
            return Err(StoreFormatError::Invalid("frozen macro flags"));
        }
        let record = MACROS_HEADER + index * MACRO_RECORD;
        out[record] = row.flags;
        put_u32(&mut out, record + 4, row.parameter_text);
        put_u32(&mut out, record + 8, row.replacement_text);
    }
    Ok(out)
}

fn decode_macros(
    bytes: &[u8],
    tokens: &TokenStore,
) -> Result<(MacroStore, Vec<FormatMacro>), StoreFormatError> {
    if bytes.len() < MACROS_HEADER
        || read_u32(bytes, 0) != SECTION_VERSION
        || read_u32(bytes, 8) != MACROS_HEADER as u32
        || read_u32(bytes, 12) != 0
    {
        return Err(StoreFormatError::Invalid("invalid frozen macro header"));
    }
    let count = read_u32(bytes, 4) as usize;
    if MACROS_HEADER
        .checked_add(checked_len(count, MACRO_RECORD, "frozen macro records")?)
        .is_none_or(|end| end != bytes.len())
    {
        return Err(StoreFormatError::Invalid("frozen macro section length"));
    }
    let mut rows = Vec::with_capacity(count);
    let mut definitions = Vec::with_capacity(count);
    let mut patterns = Vec::with_capacity(count);
    for index in 0..count {
        let record = MACROS_HEADER + index * MACRO_RECORD;
        if bytes[record] & !0x0f != 0
            || bytes[record + 1..record + 4].iter().any(|byte| *byte != 0)
            || read_u32(bytes, record + 12) != 0
        {
            return Err(StoreFormatError::Invalid("invalid frozen macro record"));
        }
        let row = FormatMacro {
            flags: bytes[record],
            parameter_text: read_u32(bytes, record + 4),
            replacement_text: read_u32(bytes, record + 8),
        };
        let parameter_text = tokens
            .resolve_stored(crate::ids::TokenListId::new(row.parameter_text))
            .ok_or(StoreFormatError::Invalid(
                "frozen macro parameter reference",
            ))?;
        let replacement_text = tokens
            .resolve_stored(crate::ids::TokenListId::new(row.replacement_text))
            .ok_or(StoreFormatError::Invalid(
                "frozen macro replacement reference",
            ))?;
        definitions.push(MacroMeaning::new(
            crate::meaning::MeaningFlags::from_bits(row.flags),
            parameter_text,
            replacement_text,
        ));
        patterns.push(MacroParameterPattern::from_tokens(
            tokens.get(parameter_text),
        ));
        rows.push(row);
    }
    let macros =
        MacroStore::from_frozen(definitions, patterns).map_err(StoreFormatError::Invalid)?;
    Ok((macros, rows))
}

fn encode_glue(glue: &[FormatGlue]) -> Result<Vec<u8>, StoreFormatError> {
    let count = u32_count(glue.len(), "frozen glue count")?;
    let records_len = checked_len(glue.len(), GLUE_RECORD, "frozen glue records")?;
    let mut out = vec![0; GLUE_HEADER + records_len];
    put_u32(&mut out, 0, SECTION_VERSION);
    put_u32(&mut out, 4, count);
    put_u32(&mut out, 8, GLUE_HEADER as u32);
    for (index, row) in glue.iter().enumerate() {
        order(row.stretch_order)?;
        order(row.shrink_order)?;
        let record = GLUE_HEADER + index * GLUE_RECORD;
        put_i32(&mut out, record, row.width);
        put_i32(&mut out, record + 4, row.stretch);
        put_i32(&mut out, record + 8, row.shrink);
        out[record + 12] = row.stretch_order;
        out[record + 13] = row.shrink_order;
    }
    let lookup = crate::frozen_lookup::encode((0..glue.len()).map(|index| {
        let start = GLUE_HEADER + index * GLUE_RECORD;
        (out[start..start + GLUE_RECORD].to_vec(), index as u32)
    }))
    .map_err(StoreFormatError::Invalid)?;
    out.extend_from_slice(&lookup);
    Ok(out)
}

fn decode_glue(
    bytes: &[u8],
    checksum: u64,
) -> Result<(GlueStore, Vec<FormatGlue>), StoreFormatError> {
    if bytes.len() < GLUE_HEADER
        || read_u32(bytes, 0) != SECTION_VERSION
        || read_u32(bytes, 8) != GLUE_HEADER as u32
        || read_u32(bytes, 12) != 0
    {
        return Err(StoreFormatError::Invalid("invalid frozen glue header"));
    }
    let count = read_u32(bytes, 4) as usize;
    let records_end = GLUE_HEADER
        .checked_add(checked_len(count, GLUE_RECORD, "frozen glue records")?)
        .ok_or(StoreFormatError::Invalid("frozen glue section length"))?;
    if records_end > bytes.len() {
        return Err(StoreFormatError::Invalid("frozen glue section length"));
    }
    let mut rows = Vec::with_capacity(count);
    let mut specs = Vec::with_capacity(count);
    let mut keys = Vec::with_capacity(count);
    for index in 0..count {
        let record = GLUE_HEADER + index * GLUE_RECORD;
        if bytes[record + 14..record + GLUE_RECORD]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(StoreFormatError::Invalid(
                "nonzero frozen glue reserved bytes",
            ));
        }
        let row = FormatGlue {
            width: read_i32(bytes, record),
            stretch: read_i32(bytes, record + 4),
            shrink: read_i32(bytes, record + 8),
            stretch_order: bytes[record + 12],
            shrink_order: bytes[record + 13],
        };
        specs.push(GlueSpec {
            width: Scaled::from_raw(row.width),
            stretch: Scaled::from_raw(row.stretch),
            stretch_order: order(row.stretch_order)?,
            shrink: Scaled::from_raw(row.shrink),
            shrink_order: order(row.shrink_order)?,
        });
        rows.push(row);
        keys.push(bytes[record..record + GLUE_RECORD].to_vec());
    }
    let lookup = crate::frozen_lookup::decode(&bytes[records_end..], count)
        .and_then(|lookup| {
            lookup.validate_targets(&keys)?;
            lookup.spot_check(checksum)?;
            Ok(lookup)
        })
        .map_err(StoreFormatError::Invalid)?;
    let glue = GlueStore::from_frozen(specs, lookup).map_err(StoreFormatError::Invalid)?;
    Ok((glue, rows))
}

fn write_header(
    out: &mut [u8],
    header: usize,
    count: u32,
    data_offset: usize,
    data_len: usize,
) -> Result<(), StoreFormatError> {
    put_u32(out, 0, SECTION_VERSION);
    put_u32(out, 4, count);
    put_u32(out, 8, header as u32);
    put_u32(
        out,
        12,
        u32_count(data_offset, "frozen section data offset")?,
    );
    put_u32(out, 16, u32_count(data_len, "frozen section data length")?);
    Ok(())
}

fn read_header(
    bytes: &[u8],
    header: usize,
    record: usize,
    label: &'static str,
) -> Result<(usize, usize, usize), StoreFormatError> {
    if bytes.len() < header
        || read_u32(bytes, 0) != SECTION_VERSION
        || read_u32(bytes, 8) != header as u32
        || read_u32(bytes, 20) != 0
    {
        return Err(StoreFormatError::Invalid(label));
    }
    let count = read_u32(bytes, 4) as usize;
    let expected_data = header
        .checked_add(checked_len(count, record, label)?)
        .ok_or(StoreFormatError::Invalid(label))?;
    let data_offset = read_u32(bytes, 12) as usize;
    if data_offset != expected_data {
        return Err(StoreFormatError::Invalid(label));
    }
    Ok((count, data_offset, read_u32(bytes, 16) as usize))
}

fn checked_len(
    count: usize,
    width: usize,
    message: &'static str,
) -> Result<usize, StoreFormatError> {
    count
        .checked_mul(width)
        .ok_or(StoreFormatError::Invalid(message))
}

fn u32_count(value: usize, message: &'static str) -> Result<u32, StoreFormatError> {
    u32::try_from(value).map_err(|_| StoreFormatError::Invalid(message))
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed field"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed field"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("fixed field"))
}

fn catcode(value: u8) -> Result<Catcode, StoreFormatError> {
    super::catcode(value)
}

fn order(value: u8) -> Result<Order, StoreFormatError> {
    super::order(value)
}

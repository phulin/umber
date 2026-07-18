use super::{FormatCodeTables, FormatFont, StoreFormat, StoreFormatError, catcode};
use crate::code_tables::{CodeTableValues, CodeTables};
use crate::font::FontStore;
use crate::hyphenation::HyphenationTable;
use crate::ids::FontId;
use crate::interner::Interner;

pub(crate) const FONTS_SECTION: u32 = 320;
pub(crate) const CODE_TABLES_SECTION: u32 = 336;
pub(crate) const HYPHENATION_SECTION: u32 = 352;

const VERSION: u32 = 1;
const FONTS_HEADER: usize = 32;
const CODE_TABLES_HEADER: usize = 16;
const CODE_TABLE_RECORD: usize = 32;
const HYPHENATION_HEADER: usize = 16;

pub(crate) struct EncodedFrozenNonNode {
    pub fonts: Vec<u8>,
    pub code_tables: Vec<u8>,
    pub hyphenation: Vec<u8>,
}

pub(crate) struct FrozenNonNodeSections<'a> {
    pub fonts: &'a [u8],
    pub code_tables: &'a [u8],
    pub hyphenation: &'a [u8],
}

pub(crate) struct DecodedFrozenNonNode {
    pub fonts: FontStore,
    pub code_tables: CodeTables,
    pub hyphenation: HyphenationTable,
    pub prepared_mag: Option<i32>,
    pub last_loaded_font: FontId,
    pub font_rows: Vec<FormatFont>,
    pub code_rows: Vec<FormatCodeTables>,
}

pub(crate) fn encode(format: &StoreFormat) -> Result<EncodedFrozenNonNode, StoreFormatError> {
    Ok(EncodedFrozenNonNode {
        fonts: encode_fonts(format)?,
        code_tables: encode_code_tables(&format.code_tables)?,
        hyphenation: encode_hyphenation(&format.hyphenation)?,
    })
}

pub(crate) fn decode(
    sections: FrozenNonNodeSections<'_>,
    interner: &Interner,
) -> Result<DecodedFrozenNonNode, StoreFormatError> {
    let (fonts, font_rows, prepared_mag, last_loaded_font) =
        decode_fonts(sections.fonts, interner)?;
    let (code_tables, code_rows) = decode_code_tables(sections.code_tables)?;
    let hyphenation = decode_hyphenation(sections.hyphenation)?;
    Ok(DecodedFrozenNonNode {
        fonts,
        code_tables,
        hyphenation,
        prepared_mag,
        last_loaded_font,
        font_rows,
        code_rows,
    })
}

fn encode_fonts(format: &StoreFormat) -> Result<Vec<u8>, StoreFormatError> {
    let payload = bincode::serialize(&format.fonts)
        .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    let mut out = vec![0; FONTS_HEADER + payload.len()];
    put_u32(&mut out, 0, VERSION);
    put_u32(
        &mut out,
        4,
        u32_len(format.fonts.len(), "frozen font count")?,
    );
    put_u32(&mut out, 8, FONTS_HEADER as u32);
    put_u32(&mut out, 12, u32_len(payload.len(), "frozen font payload")?);
    if let Some(value) = format.prepared_mag {
        put_u32(&mut out, 16, 1);
        put_i32(&mut out, 20, value);
    }
    put_u32(&mut out, 24, format.last_loaded_font);
    out[FONTS_HEADER..].copy_from_slice(&payload);
    Ok(out)
}

fn decode_fonts(
    bytes: &[u8],
    interner: &Interner,
) -> Result<(FontStore, Vec<FormatFont>, Option<i32>, FontId), StoreFormatError> {
    if bytes.len() < FONTS_HEADER
        || read_u32(bytes, 0) != VERSION
        || read_u32(bytes, 8) as usize != FONTS_HEADER
        || read_u32(bytes, 28) != 0
    {
        return Err(StoreFormatError::Invalid("invalid frozen font header"));
    }
    let payload_len = read_u32(bytes, 12) as usize;
    if FONTS_HEADER.checked_add(payload_len) != Some(bytes.len()) {
        return Err(StoreFormatError::Invalid("frozen font payload range"));
    }
    let prepared_mag = match read_u32(bytes, 16) {
        0 if read_i32(bytes, 20) == 0 => None,
        1 => Some(read_i32(bytes, 20)),
        _ => return Err(StoreFormatError::Invalid("invalid frozen prepared-mag tag")),
    };
    let rows: Vec<FormatFont> = bincode::deserialize(&bytes[FONTS_HEADER..])
        .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    if rows.len() != read_u32(bytes, 4) as usize
        || bincode::serialize(&rows).map_err(|error| StoreFormatError::Codec(error.to_string()))?
            != bytes[FONTS_HEADER..]
    {
        return Err(StoreFormatError::Invalid(
            "non-canonical frozen font payload",
        ));
    }
    let mut runtime = Vec::with_capacity(rows.len());
    for row in &rows {
        let identifier = row
            .identifier
            .map(|raw| {
                interner
                    .symbol_at_slot(raw)
                    .and_then(|symbol| interner.resolve_stored(symbol))
                    .ok_or(StoreFormatError::Invalid(
                        "frozen font identifier is not live",
                    ))
            })
            .transpose()?;
        let expansion = row.expansion;
        runtime.push((row.clone().restore(), identifier, expansion));
    }
    let fonts = FontStore::from_frozen(runtime, interner).map_err(StoreFormatError::Invalid)?;
    let last_raw = read_u32(bytes, 24);
    let last = fonts
        .resolve_stored(FontId::new(last_raw))
        .ok_or(StoreFormatError::Invalid(
            "frozen last loaded font is not live",
        ))?;
    Ok((fonts, rows, prepared_mag, last))
}

fn encode_code_tables(rows: &[FormatCodeTables]) -> Result<Vec<u8>, StoreFormatError> {
    let records_len =
        rows.len()
            .checked_mul(CODE_TABLE_RECORD)
            .ok_or(StoreFormatError::Invalid(
                "frozen code-table section overflow",
            ))?;
    let mut out = vec![0; CODE_TABLES_HEADER + records_len];
    put_u32(&mut out, 0, VERSION);
    put_u32(&mut out, 4, u32_len(rows.len(), "frozen code-table count")?);
    put_u32(&mut out, 8, CODE_TABLES_HEADER as u32);
    for (index, row) in rows.iter().enumerate() {
        let at = CODE_TABLES_HEADER + index * CODE_TABLE_RECORD;
        put_u32(&mut out, at, row.code);
        out[at + 4] = row.catcode;
        put_u32(&mut out, at + 8, row.lccode);
        put_u32(&mut out, at + 12, row.uccode);
        put_u16(&mut out, at + 16, row.sfcode);
        put_u32(&mut out, at + 20, row.mathcode);
        put_i32(&mut out, at + 24, row.delcode);
    }
    Ok(out)
}

fn decode_code_tables(
    bytes: &[u8],
) -> Result<(CodeTables, Vec<FormatCodeTables>), StoreFormatError> {
    if bytes.len() < CODE_TABLES_HEADER
        || read_u32(bytes, 0) != VERSION
        || read_u32(bytes, 8) as usize != CODE_TABLES_HEADER
        || read_u32(bytes, 12) != 0
    {
        return Err(StoreFormatError::Invalid(
            "invalid frozen code-table header",
        ));
    }
    let count = read_u32(bytes, 4) as usize;
    if CODE_TABLES_HEADER.checked_add(count.checked_mul(CODE_TABLE_RECORD).ok_or(
        StoreFormatError::Invalid("frozen code-table section overflow"),
    )?) != Some(bytes.len())
    {
        return Err(StoreFormatError::Invalid("frozen code-table record range"));
    }
    let mut runtime_rows = Vec::with_capacity(count);
    let mut format_rows = Vec::with_capacity(count);
    for index in 0..count {
        let at = CODE_TABLES_HEADER + index * CODE_TABLE_RECORD;
        if bytes[at + 5..at + 8]
            .iter()
            .chain(&bytes[at + 18..at + 20])
            .chain(&bytes[at + 28..at + 32])
            .any(|byte| *byte != 0)
        {
            return Err(StoreFormatError::Invalid(
                "frozen code-table reserved bytes",
            ));
        }
        let ch = char::from_u32(read_u32(bytes, at))
            .ok_or(StoreFormatError::Invalid("frozen code-table codepoint"))?;
        let catcode = catcode(bytes[at + 4])?;
        let values = CodeTableValues {
            catcode,
            lccode: read_u32(bytes, at + 8),
            uccode: read_u32(bytes, at + 12),
            sfcode: read_u16(bytes, at + 16),
            mathcode: read_u32(bytes, at + 20),
            delcode: read_i32(bytes, at + 24),
        };
        runtime_rows.push((ch, values));
        format_rows.push(FormatCodeTables {
            code: ch as u32,
            catcode: catcode as u8,
            lccode: values.lccode,
            uccode: values.uccode,
            sfcode: values.sfcode,
            mathcode: values.mathcode,
            delcode: values.delcode,
        });
    }
    let tables = CodeTables::from_frozen(&runtime_rows).map_err(StoreFormatError::Invalid)?;
    Ok((tables, format_rows))
}

fn encode_hyphenation(table: &HyphenationTable) -> Result<Vec<u8>, StoreFormatError> {
    let payload =
        bincode::serialize(table).map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    let mut out = vec![0; HYPHENATION_HEADER + payload.len()];
    put_u32(&mut out, 0, VERSION);
    put_u32(&mut out, 4, HYPHENATION_HEADER as u32);
    put_u32(
        &mut out,
        8,
        u32_len(payload.len(), "frozen hyphenation payload")?,
    );
    out[HYPHENATION_HEADER..].copy_from_slice(&payload);
    Ok(out)
}

fn decode_hyphenation(bytes: &[u8]) -> Result<HyphenationTable, StoreFormatError> {
    if bytes.len() < HYPHENATION_HEADER
        || read_u32(bytes, 0) != VERSION
        || read_u32(bytes, 4) as usize != HYPHENATION_HEADER
        || read_u32(bytes, 12) != 0
        || HYPHENATION_HEADER.checked_add(read_u32(bytes, 8) as usize) != Some(bytes.len())
    {
        return Err(StoreFormatError::Invalid(
            "invalid frozen hyphenation header",
        ));
    }
    let table: HyphenationTable = bincode::deserialize(&bytes[HYPHENATION_HEADER..])
        .map_err(|error| StoreFormatError::Codec(error.to_string()))?;
    if bincode::serialize(&table).map_err(|error| StoreFormatError::Codec(error.to_string()))?
        != bytes[HYPHENATION_HEADER..]
    {
        return Err(StoreFormatError::Invalid(
            "non-canonical frozen hyphenation payload",
        ));
    }
    table.validate_frozen().map_err(StoreFormatError::Invalid)?;
    Ok(table)
}

fn u32_len(value: usize, message: &'static str) -> Result<u32, StoreFormatError> {
    u32::try_from(value).map_err(|_| StoreFormatError::Invalid(message))
}
fn put_u16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut [u8], at: usize, value: i32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().expect("validated range"))
}
fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated range"))
}
fn read_i32(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().expect("validated range"))
}

//! Official e-TeX V2 e-TRIP master-artifact comparison.
//!
//! The canonical e-TeX 2.6 oracle remains the exact semantic/text/DVI
//! authority. This module additionally binds that exact run to the CTAN V2
//! masters without turning implementation- or platform-owned text into a
//! semantic oracle.

use std::fmt::Write as _;
use std::path::Path;

use tex_out::dvi::disasm::DviFile;

pub struct OfficialEtripRun<'a> {
    pub initex_log: &'a [u8],
    pub terminal: &'a [u8],
    pub log: &'a [u8],
    pub dvi: &'a [u8],
    pub output: &'a [u8],
}

pub fn compare(root: &Path, run: OfficialEtripRun<'_>) -> Result<(), String> {
    let masters = root.join("third_party/trip");
    let official_initex = read(&masters.join("etripin.log"))?;
    let official_terminal = read(&masters.join("etrip.fot"))?;
    let official_log = read(&masters.join("etrip.log"))?;
    let official_typ = read(&masters.join("etrip.typ"))?;
    let official_output = read(&masters.join("etrip.out"))?;

    compare_bytes(
        "official INITEX log",
        &normalize_text(
            &official_initex,
            TextOrigin::OfficialV2,
            TextChannel::InitexLog,
        )?,
        &normalize_text(
            run.initex_log,
            TextOrigin::AdaptedEtex26,
            TextChannel::InitexLog,
        )?,
    )?;
    compare_bytes(
        "official terminal photo",
        &normalize_text(
            &official_terminal,
            TextOrigin::OfficialV2,
            TextChannel::LoadedTerminal,
        )?,
        &normalize_text(
            run.terminal,
            TextOrigin::AdaptedEtex26,
            TextChannel::LoadedTerminal,
        )?,
    )?;
    compare_bytes(
        "official loaded log",
        &normalize_text(
            &official_log,
            TextOrigin::OfficialV2,
            TextChannel::LoadedLog,
        )?,
        &normalize_text(run.log, TextOrigin::AdaptedEtex26, TextChannel::LoadedLog)?,
    )?;
    compare_bytes("official output file", &official_output, run.output)?;

    let expected_typ = project_official_dvitype(&official_typ)?;
    let actual_typ = project_dvi(run.dvi)?;
    compare_bytes(
        "official DVItype projection",
        expected_typ.as_bytes(),
        actual_typ.as_bytes(),
    )
}

#[allow(clippy::disallowed_methods)] // Host-side comparison reads pinned official artifacts.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path)
        .map_err(|error| format!("read official e-TRIP master {}: {error}", path.display()))
}

fn compare_bytes(channel: &str, expected: &[u8], actual: &[u8]) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    Err(format!(
        "{channel} mismatch at byte {offset}: expected {}, actual {}; expected context {:?}, actual context {:?}",
        byte_at(expected, offset),
        byte_at(actual, offset),
        context(expected, offset),
        context(actual, offset),
    ))
}

fn context(bytes: &[u8], offset: usize) -> String {
    let start = offset.saturating_sub(24);
    let end = (offset + 24).min(bytes.len());
    bytes[start..end].escape_ascii().to_string()
}

fn byte_at(bytes: &[u8], offset: usize) -> String {
    bytes
        .get(offset)
        .map_or_else(|| "EOF".to_owned(), |byte| format!("0x{byte:02x}"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextOrigin {
    OfficialV2,
    AdaptedEtex26,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TextChannel {
    InitexLog,
    LoadedTerminal,
    LoadedLog,
}

fn normalize_text(
    input: &[u8],
    origin: TextOrigin,
    channel: TextChannel,
) -> Result<Vec<u8>, String> {
    let mut text = String::from_utf8(input.to_vec())
        .map_err(|_| "official e-TRIP text artifact is not UTF-8".to_owned())?
        .replace("\r\n", "\n");
    if text.contains('\r') {
        return Err("official e-TRIP text artifact contains a bare carriage return".into());
    }

    text = text
        .replace("(./etrip.tex", "(etrip.tex")
        .replace("(./etrip.out", "(etrip.out")
        .replace("3.141592653-2.6", "3.14159-2.0")
        .replace("3.14159-2.6", "3.14159-2.0")
        .replace("version/revision 2.6", "version/revision 2.0");
    text = strip_startup_framing(&text);

    text = normalize_source_lines(&text, origin == TextOrigin::AdaptedEtex26)?;

    let mut normalized = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let replacement = normalize_line(line, channel);
        normalized.push_str(&replacement);
    }

    if channel == TextChannel::LoadedLog {
        normalized = normalize_etex_version_diagnostics(normalized)?;
    }
    Ok(normalized.into_bytes())
}

fn strip_startup_framing(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with("This is e-TeX, Version ")
            || line.starts_with("**&etrip")
            || line.starts_with("***etrip")
        {
            continue;
        }
        stripped.push_str(line);
    }
    stripped
}

fn normalize_line(line: &str, channel: TextChannel) -> String {
    if line.starts_with(" (preloaded format=etrip ") {
        return " (preloaded format=etrip <date>)\n".into();
    }
    if line_has_two_decimal_fields(line, " strings of total length ") {
        return "<string pool statistics>\n".into();
    }
    if line.ends_with(" multiletter control sequences\n")
        && line
            .split_whitespace()
            .next()
            .is_some_and(|field| field.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return "<multiletter control sequences>\n".into();
    }
    if line.starts_with("Hyphenation trie of length ") {
        return replace_after(line, " ops out of ", "<capacity>");
    }
    if line.starts_with("Memory usage before: ") {
        return "<memory usage>\n".into();
    }
    if channel == TextChannel::LoadedLog && is_final_usage_statistic(line) {
        return final_usage_label(line);
    }
    replace_glue_set_rounding(line)
}

fn line_has_two_decimal_fields(line: &str, separator: &str) -> bool {
    line.strip_suffix('\n')
        .and_then(|line| line.split_once(separator))
        .is_some_and(|(left, right)| {
            left.bytes().all(|byte| byte.is_ascii_digit())
                && right.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn replace_after(line: &str, marker: &str, replacement: &str) -> String {
    let Some(index) = line.find(marker) else {
        return line.to_owned();
    };
    let value = index + marker.len();
    let end = line[value..]
        .find(|character: char| !character.is_ascii_digit())
        .map_or(line.len(), |offset| value + offset);
    format!("{}{replacement}{}", &line[..value], &line[end..])
}

fn replace_glue_set_rounding(line: &str) -> String {
    let Some(marker) = line.find(", glue set ") else {
        return line.to_owned();
    };
    if !(line.starts_with("\\hbox(") || line.starts_with("\\vbox(")) {
        return line.to_owned();
    }
    let value = marker + ", glue set ".len();
    let end = line[value..]
        .find(|character: char| {
            !(character.is_ascii_digit() || matches!(character, '-' | '+' | '.'))
        })
        .map_or(line.len(), |offset| value + offset);
    format!("{}<rounding>{}", &line[..value], &line[end..])
}

fn is_final_usage_statistic(line: &str) -> bool {
    let trimmed = line.trim_start();
    [
        "strings out of ",
        "string characters out of ",
        "words of memory out of ",
        "multiletter control sequences out of ",
        "words of font info for ",
        "hyphenation exceptions out of ",
        "stack positions out of ",
    ]
    .iter()
    .any(|marker| trimmed.contains(marker))
}

fn final_usage_label(line: &str) -> String {
    let trimmed = line.trim_start();
    for (marker, label) in [
        ("strings out of ", "<strings usage>\n"),
        ("string characters out of ", "<string characters usage>\n"),
        ("words of memory out of ", "<memory words usage>\n"),
        (
            "multiletter control sequences out of ",
            "<control sequence usage>\n",
        ),
        ("words of font info for ", "<font info usage>\n"),
        ("hyphenation exceptions out of ", "<hyphenation usage>\n"),
        ("stack positions out of ", "<stack usage>\n"),
    ] {
        if trimmed.contains(marker) {
            return label.into();
        }
    }
    line.to_owned()
}

/// Projects only e-TRIP's terminal engine-usage block across physical storage
/// implementations. Every surrounding loaded-log byte remains exact.
pub(super) fn normalize_loaded_log_engine_usage(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "e-TRIP loaded log is not UTF-8".to_owned())?;
    if text.contains('\r') {
        return Err("e-TRIP loaded log contains a bare carriage return".into());
    }
    let mut normalized = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if is_final_usage_statistic(line) {
            normalized.push_str(&final_usage_label(line));
        } else {
            normalized.push_str(line);
        }
    }
    Ok(normalized.into_bytes())
}

fn normalize_etex_version_diagnostics(mut text: String) -> Result<String, String> {
    text = text.replace(
        "this will be denominator of:",
        "this will begin denominator of:",
    );
    let v2 = concat!(
        "{reassigning \\toks3000=}\n",
        "{reassigning \\toks3000=}\n",
        "{changing \\toks3000=}\n",
        "{into \\toks3000=a b c}\n",
    );
    let v26 = concat!(
        "{changing \\toks3000=}\n",
        "{into \\toks3000=a b c}\n",
        "{changing \\toks3000=a b c}\n",
        "{into \\toks3000=}\n",
        "{changing \\toks3000=}\n",
        "{into \\toks3000=a b c}\n",
    );
    let count_v2 = text.matches(v2).count();
    let count_v26 = text.matches(v26).count();
    if count_v2 + count_v26 != 1 {
        let context = text.find("toks3000").map_or("<absent>", |offset| {
            &text[offset.saturating_sub(80)..(offset + 300).min(text.len())]
        });
        return Err(format!(
            "e-TRIP sparse-register trace bridge expected one V2 or V2.6 block, found {count_v2} and {count_v26}; context: {context:?}"
        ));
    }
    Ok(text
        .replace(v2, "<e-TeX sparse token reassignment trace>\n")
        .replace(v26, "<e-TeX sparse token reassignment trace>\n"))
}

fn normalize_source_lines(text: &str, shift_adapted_root: bool) -> Result<String, String> {
    let mut output = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let marker = [b"lines ".as_slice(), b"line ".as_slice(), b"l.".as_slice()]
            .into_iter()
            .filter_map(|marker| {
                bytes[cursor..]
                    .windows(marker.len())
                    .enumerate()
                    .find(|(offset, window)| {
                        *window == marker
                            && bytes
                                .get(cursor + offset + marker.len())
                                .is_some_and(u8::is_ascii_digit)
                    })
                    .map(|(offset, _)| offset)
                    .map(|offset| (cursor + offset, marker))
            })
            .min_by_key(|(offset, _)| *offset);
        let Some((start, marker)) = marker else {
            output.push_str(&text[cursor..]);
            break;
        };
        output.push_str(&text[cursor..start + marker.len()]);
        let number_start = start + marker.len();
        let (first, mut end) = parse_decimal(bytes, number_start)?;
        write!(
            output,
            "{}",
            normalized_source_line(first, shift_adapted_root)
        )
        .expect("write to String");
        if marker == b"lines " && bytes.get(end..end + 2) == Some(b"--") {
            output.push_str("--");
            end += 2;
            let (second, second_end) = parse_wrapped_decimal(bytes, end)?;
            write!(
                output,
                "{}",
                normalized_source_line(second, shift_adapted_root)
            )
            .expect("write to String");
            end = second_end;
        }
        cursor = end;
    }
    Ok(output)
}

fn parse_decimal(bytes: &[u8], start: usize) -> Result<(u32, usize), String> {
    let mut end = start;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if end == start {
        return Err("e-TRIP line-reference marker is not followed by a number".into());
    }
    let number = std::str::from_utf8(&bytes[start..end])
        .expect("decimal bytes are UTF-8")
        .parse()
        .map_err(|_| "e-TRIP line reference does not fit u32".to_owned())?;
    Ok((number, end))
}

fn parse_wrapped_decimal(bytes: &[u8], start: usize) -> Result<(u32, usize), String> {
    let (prefix, mut end) = parse_decimal(bytes, start)?;
    if bytes.get(end) != Some(&b'\n') || !bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
        return Ok((prefix, end));
    }
    end += 1;
    let (suffix, suffix_end) = parse_decimal(bytes, end)?;
    let digits = suffix_end - end;
    let scale = 10_u32
        .checked_pow(u32::try_from(digits).map_err(|_| "wrapped line number is too long")?)
        .ok_or_else(|| "wrapped line number is too long".to_owned())?;
    Ok((prefix * scale + suffix, suffix_end))
}

const fn normalized_source_line(line: u32, shift_adapted_root: bool) -> u32 {
    if shift_adapted_root && line >= 90 {
        line - 2
    } else {
        line
    }
}

fn project_official_dvitype(bytes: &[u8]) -> Result<String, String> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| "official e-TRIP DVItype master is not UTF-8".to_owned())?
        .replace("\r\n", "\n");
    let mut projection = String::new();
    for line in text.lines() {
        if line.starts_with("numerator/denominator=") {
            projection.push_str(line);
            projection.push('\n');
        } else if let Some(magnification) = line.strip_prefix("magnification=") {
            let value = magnification
                .split_once(';')
                .map_or(magnification, |(value, _)| value);
            writeln!(projection, "magnification={value}").expect("write to String");
        } else if line.contains(": beginning of page ") {
            let (offset, counts) = line
                .split_once(": beginning of page ")
                .ok_or_else(|| "malformed official DVItype page line".to_owned())?;
            let counts = counts.trim().replace('.', ",");
            writeln!(projection, "page:{offset}:{counts}").expect("write to String");
        } else if let Some(offset) = line.trim().strip_suffix(": eop") {
            writeln!(projection, "eop:{offset}").expect("write to String");
        } else if let Some(offset) = line
            .strip_prefix("Postamble starts at byte ")
            .and_then(|line| line.strip_suffix('.'))
        {
            writeln!(projection, "post:{offset}").expect("write to String");
        } else if line.starts_with("maxv=") {
            projection.push_str(line);
            projection.push('\n');
        }
    }
    if projection.lines().count() != 10 {
        return Err(format!(
            "official e-TRIP DVItype projection has {} fields, expected 10",
            projection.lines().count()
        ));
    }
    Ok(projection)
}

fn project_dvi(bytes: &[u8]) -> Result<String, String> {
    let file = DviFile::parse(bytes).map_err(|error| format!("parse e-TRIP DVI: {error}"))?;
    if bytes.first() != Some(&247) || bytes.get(1) != Some(&2) {
        return Err("e-TRIP DVI is missing the version-2 preamble".into());
    }
    let numerator = read_i32(bytes, 2)?;
    let denominator = read_i32(bytes, 6)?;
    let magnification = read_i32(bytes, 10)?;
    let mut projection =
        format!("numerator/denominator={numerator}/{denominator}\nmagnification={magnification}\n");
    for page in &file.pages {
        let counts = page
            .counts
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        writeln!(projection, "page:{}:{counts}", page.bop_offset).expect("write to String");
        let eop = page
            .eop_end
            .and_then(|end| end.checked_sub(1))
            .ok_or_else(|| format!("e-TRIP DVI page {} has no eop", page.index + 1))?;
        writeln!(projection, "eop:{eop}").expect("write to String");
    }
    writeln!(projection, "post:{}", file.post_offset).expect("write to String");
    let maxv = read_i32(bytes, file.post_offset + 17)?;
    let maxh = read_i32(bytes, file.post_offset + 21)?;
    let maxstack = read_u16(bytes, file.post_offset + 25)?;
    let pages = read_u16(bytes, file.post_offset + 27)?;
    writeln!(
        projection,
        "maxv={maxv}, maxh={maxh}, maxstackdepth={maxstack}, totalpages={pages}"
    )
    .expect("write to String");
    Ok(projection)
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let field = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("e-TRIP DVI is truncated at byte {offset}"))?;
    Ok(i32::from_be_bytes(
        field.try_into().expect("four-byte field"),
    ))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let field = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("e-TRIP DVI is truncated at byte {offset}"))?;
    Ok(u16::from_be_bytes(
        field.try_into().expect("two-byte field"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_usage_normalization_abstracts_storage_specific_font_info_words() {
        assert!(is_final_usage_statistic(
            " 2286 words of font info for 3 fonts"
        ));
        assert_eq!(
            final_usage_label(" 60 words of font info for 3 fonts"),
            "<font info usage>\n"
        );
        let expected = b"before\n 18 strings out of 13506\n 2286 words of font info for 3 fonts, out of 20000 for 75\nafter\n";
        let actual = b"before\n 4 strings out of 13973\n 60 words of font info for 3 fonts, out of 20000 for 75\nafter\n";
        assert_eq!(
            normalize_loaded_log_engine_usage(expected).expect("expected projection"),
            normalize_loaded_log_engine_usage(actual).expect("actual projection")
        );
        let changed = b"before changed\n 4 strings out of 13973\n 60 words of font info for 3 fonts, out of 20000 for 75\nafter\n";
        assert_ne!(
            normalize_loaded_log_engine_usage(expected).expect("expected projection"),
            normalize_loaded_log_engine_usage(changed).expect("changed projection")
        );
    }

    #[test]
    fn deliberate_official_artifact_perturbation_fails_actionably() {
        let error = compare_bytes("official output file", b"\\endgroup \n", b"\\endgraup \n")
            .expect_err("perturbed official artifact must fail");
        assert!(
            error.contains("official output file mismatch at byte 6"),
            "{error}"
        );
        assert!(error.contains("expected 0x6f, actual 0x61"), "{error}");
    }

    #[test]
    fn dvitype_projection_rejects_a_perturbed_page_offset() {
        let official = concat!(
            "numerator/denominator=25400000/473628672\n",
            "magnification=1000; ignored float\n",
            "42: beginning of page 1.0.0.0.0.0.0.0.0.0 \n",
            "87: eop \n",
            "88: beginning of page 1.0.0.0.0.0.0.0.0.0 \n",
            "133: eop \n",
            "135: beginning of page 1.0.0.0.0.0.0.0.0.0 \n",
            "179: eop \n",
            "Postamble starts at byte 180.\n",
            "maxv=0, maxh=0, maxstackdepth=0, totalpages=3\n",
        );
        let projected = project_official_dvitype(official.as_bytes()).expect("projection");
        let error = compare_bytes(
            "official DVItype projection",
            projected.as_bytes(),
            projected.replace("page:135", "page:134").as_bytes(),
        )
        .expect_err("perturbed DVItype page offset must fail");
        assert!(
            error.contains("official DVItype projection mismatch"),
            "{error}"
        );
    }
}

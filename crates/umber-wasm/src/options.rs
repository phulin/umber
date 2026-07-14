use js_sys::{Array, Reflect, Uint8Array};
use umber::{FileKind, FileRequestKey, SessionLimits, SessionOptions, SessionWebFont};
use wasm_bindgen::{JsCast, JsValue};

use crate::js_error;

pub(crate) fn parse_options(value: &JsValue) -> Result<SessionOptions, JsValue> {
    require_object(value, "session options")?;
    let mut options = SessionOptions {
        main_path: required_string(value, "mainPath")?,
        ..SessionOptions::default()
    };
    options.job_name = optional_string(value, "jobName")?;
    options.format = optional_bytes(value, "format")?;
    options.html = !field(value, "html")?.is_undefined() && !field(value, "html")?.is_null();
    if let Some(clock) = optional_object(value, "clock")? {
        options.clock.year = integer::<i32>(&clock, "year")?;
        options.clock.month = integer::<i32>(&clock, "month")?;
        options.clock.day = integer::<i32>(&clock, "day")?;
        options.clock.time = integer::<i32>(&clock, "minutes")?;
    }
    if let Some(limits) = optional_object(value, "limits")? {
        options.limits = parse_limits(&limits)?;
    }
    Ok(options)
}

pub(crate) fn parse_html_font(value: &JsValue) -> Result<SessionWebFont, JsValue> {
    require_object(value, "HTML font")?;
    let woff2 = required_bytes(value, "woff2")?;
    let digest = parse_digest(&required_string(value, "sha256")?)?;
    let encoding_value = field(value, "encoding")?;
    if !Array::is_array(&encoding_value) {
        return Err(js_error("HTML font encoding must be an array"));
    }
    let array = Array::from(&encoding_value);
    if array.length() != 256 {
        return Err(js_error("HTML font encoding must contain 256 entries"));
    }
    let mut encoding = Vec::with_capacity(256);
    for value in array.iter() {
        if value.is_null() || value.is_undefined() {
            encoding.push(None);
        } else {
            encoding.push(Some(value.as_string().ok_or_else(|| {
                js_error("HTML font encoding entries must be strings or null")
            })?));
        }
    }
    Ok(SessionWebFont {
        name: required_string(value, "name")?,
        tfm_content_hash_hex: required_string(value, "tfmContentHash")?,
        woff2,
        sha256: digest,
        encoding,
        provenance: required_string(value, "provenance")?,
        embeddable: required_bool(value, "embeddable")?,
    })
}

pub(crate) fn parse_request_key(value: &JsValue) -> Result<FileRequestKey, JsValue> {
    require_object(value, "file request key")?;
    let kind = match required_string(value, "kind")?.as_str() {
        "tex" => FileKind::TexInput,
        "tfm" => FileKind::Tfm,
        _ => return Err(js_error("file request kind must be 'tex' or 'tfm'")),
    };
    FileRequestKey::new(kind, &required_string(value, "name")?).map_err(crate::boundary_error)
}

fn parse_limits(value: &JsValue) -> Result<SessionLimits, JsValue> {
    let mut limits = SessionLimits::default();
    if has_value(value, "attempts")? {
        limits.attempts = integer::<u32>(value, "attempts")?;
    }
    if has_value(value, "userFiles")? {
        limits.user_files = integer::<usize>(value, "userFiles")?;
    }
    if has_value(value, "resolvedFiles")? {
        limits.resolved_files = integer::<usize>(value, "resolvedFiles")?;
    }
    if has_value(value, "oneFileBytes")? {
        limits.one_file_bytes = integer::<usize>(value, "oneFileBytes")?;
    }
    if has_value(value, "cachedFileBytes")? {
        limits.cached_file_bytes = integer::<usize>(value, "cachedFileBytes")?;
    }
    if has_value(value, "userSourceBytes")? {
        limits.user_source_bytes = integer::<usize>(value, "userSourceBytes")?;
    }
    if has_value(value, "outputBytes")? {
        limits.output_bytes = integer::<usize>(value, "outputBytes")?;
    }
    Ok(limits)
}

fn required_string(object: &JsValue, name: &str) -> Result<String, JsValue> {
    field(object, name)?
        .as_string()
        .ok_or_else(|| js_error(&format!("{name} must be a string")))
}

fn optional_string(object: &JsValue, name: &str) -> Result<Option<String>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    value
        .as_string()
        .map(Some)
        .ok_or_else(|| js_error(&format!("{name} must be a string")))
}

fn optional_bytes(object: &JsValue, name: &str) -> Result<Option<Vec<u8>>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    if !value.is_instance_of::<Uint8Array>() {
        return Err(js_error(&format!("{name} must be a Uint8Array")));
    }
    Ok(Some(Uint8Array::new(&value).to_vec()))
}

fn required_bytes(object: &JsValue, name: &str) -> Result<Vec<u8>, JsValue> {
    let value = field(object, name)?;
    if !value.is_instance_of::<Uint8Array>() {
        return Err(js_error(&format!("{name} must be a Uint8Array")));
    }
    Ok(Uint8Array::new(&value).to_vec())
}

fn required_bool(object: &JsValue, name: &str) -> Result<bool, JsValue> {
    field(object, name)?
        .as_bool()
        .ok_or_else(|| js_error(&format!("{name} must be a boolean")))
}

fn parse_digest(value: &str) -> Result<[u8; 32], JsValue> {
    if value.len() != 64 {
        return Err(js_error("sha256 must contain 64 lowercase hex digits"));
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte| match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(js_error("sha256 must use lowercase hex")),
        };
        digest[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(digest)
}

fn optional_object(object: &JsValue, name: &str) -> Result<Option<JsValue>, JsValue> {
    let value = field(object, name)?;
    if absent(&value) {
        return Ok(None);
    }
    require_object(&value, name)?;
    Ok(Some(value))
}

fn integer<T>(object: &JsValue, name: &str) -> Result<T, JsValue>
where
    T: TryFrom<u64>,
{
    let number = field(object, name)?
        .as_f64()
        .filter(|number| number.is_finite() && number.fract() == 0.0 && *number >= 0.0)
        .ok_or_else(|| js_error(&format!("{name} must be a non-negative integer")))?;
    if number > u64::MAX as f64 {
        return Err(js_error(&format!("{name} is out of range")));
    }
    T::try_from(number as u64).map_err(|_| js_error(&format!("{name} is out of range")))
}

fn has_value(object: &JsValue, name: &str) -> Result<bool, JsValue> {
    Ok(!absent(&field(object, name)?))
}

fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    Reflect::get(object, &JsValue::from_str(name))
}

fn require_object(value: &JsValue, name: &str) -> Result<(), JsValue> {
    if !value.is_object() || value.is_null() {
        return Err(js_error(&format!("{name} must be an object")));
    }
    Ok(())
}

fn absent(value: &JsValue) -> bool {
    value.is_undefined() || value.is_null()
}

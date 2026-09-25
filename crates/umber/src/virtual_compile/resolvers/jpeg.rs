pub(super) fn dimensions(bytes: &[u8]) -> Result<(u32, u32, u8, u8), String> {
    let mut cursor = 2;
    while cursor + 4 <= bytes.len() {
        if bytes[cursor] != 0xff {
            cursor += 1;
            continue;
        }
        let marker = bytes[cursor + 1];
        cursor += 2;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if cursor + 2 > bytes.len() {
            break;
        }
        let length = usize::from(u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]));
        if length < 2 || cursor + length > bytes.len() {
            return Err("invalid JPEG marker length".to_owned());
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if length < 8 {
                return Err("invalid JPEG frame header".to_owned());
            }
            return Ok((
                u32::from(u16::from_be_bytes([bytes[cursor + 5], bytes[cursor + 6]])),
                u32::from(u16::from_be_bytes([bytes[cursor + 3], bytes[cursor + 4]])),
                bytes[cursor + 2],
                bytes[cursor + 7],
            ));
        }
        cursor += length;
    }
    Err("JPEG has no supported frame header".to_owned())
}

/// pdfTeX's writejpg.c reads density only from the first APP0/APP1 marker.
/// JFIF centimetre densities truncate after conversion; a missing JFIF axis
/// inherits the other axis before the live image-resolution fallback applies.
pub(super) fn resolution(bytes: &[u8]) -> Option<(u32, u32)> {
    let marker = bytes.get(2..4)?;
    let length = usize::from(u16::from_be_bytes(bytes.get(4..6)?.try_into().ok()?));
    let data = bytes.get(6..4_usize.checked_add(length)?)?;
    if marker == [0xff, 0xe0] && data.get(..5)? == b"JFIF\0" {
        let x = u32::from(u16::from_be_bytes(data.get(8..10)?.try_into().ok()?));
        let y = u32::from(u16::from_be_bytes(data.get(10..12)?.try_into().ok()?));
        let (x, y) = match data.get(7)? {
            1 => (x, y),
            2 => ((f64::from(x) * 2.54) as u32, (f64::from(y) * 2.54) as u32),
            _ => (0, 0),
        };
        Some((if x == 0 { y } else { x }, if y == 0 { x } else { y }))
    } else if marker == [0xff, 0xe1] && data.get(..5)? == b"Exif\0" {
        exif_resolution(data.get(5..)?)
    } else {
        None
    }
}

fn exif_resolution(data: &[u8]) -> Option<(u32, u32)> {
    let start = data.iter().position(|byte| *byte != 0)?;
    let tiff = data.get(start..)?;
    let big_endian = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let word = |offset: usize| -> Option<u16> {
        let bytes = tiff.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
        Some(if big_endian {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let long = |offset: usize| -> Option<u32> {
        let bytes = tiff.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    if word(2)? != 42 {
        return None;
    }
    let directory = usize::try_from(long(4)?).ok()?;
    let count = usize::from(word(directory)?);
    let mut x = 72;
    let mut y = 72;
    let mut unit = 1.0;
    for index in 0..count {
        let field = directory
            .checked_add(2)?
            .checked_add(index.checked_mul(12)?)?;
        let tag = word(field)?;
        let kind = word(field.checked_add(2)?)?;
        let value = field.checked_add(8)?;
        match (tag, kind) {
            (282 | 283, 5 | 10) => {
                let offset = usize::try_from(long(value)?).ok()?;
                let numerator = long(offset)?;
                let denominator = long(offset.checked_add(4)?)?;
                if let Some(density) = numerator.checked_div(denominator) {
                    // writejpg.c divides integer numerator/denominator before
                    // applying the unit conversion, even for fractional DPI.
                    if tag == 282 {
                        x = density;
                    } else {
                        y = density;
                    }
                }
            }
            (296, 1 | 3 | 4 | 9) => {
                let value = match kind {
                    1 => u32::from(*tiff.get(value)?),
                    3 => u32::from(word(value)?),
                    _ => long(value)?,
                };
                if value == 3 {
                    unit = 2.54;
                } else if value == 2 {
                    unit = 1.0;
                }
            }
            _ => {}
        }
    }
    Some(((f64::from(x) * unit) as u32, (f64::from(y) * unit) as u32))
}

#[cfg(test)]
mod tests;

//! pdfTeX Type-1 map transforms applied before font subsetting.

use super::*;

impl PdfType1Program {
    /// Applies pdfTeX's Type-1 map transform to the cleartext program before
    /// subsetting. See `writet1.c::t1_modify_fm` and `t1_modify_italic`.
    pub fn with_transform(
        &self,
        transform: crate::PdfType1Transform,
        font_name: &[u8],
    ) -> Result<Self, PdfType1SubsetError> {
        if transform == crate::PdfType1Transform::default() {
            return Ok(self.clone());
        }
        let clear_end = self.length1 as usize;
        let clear = self
            .bytes
            .get(..clear_end)
            .ok_or(PdfType1SubsetError::InvalidSegments)?;
        let mut rewritten = Vec::with_capacity(self.bytes.len() + 64);
        let mut saw_matrix = false;
        for line in clear.split_inclusive(|byte| *byte == b'\n') {
            if line.starts_with(b"/FontMatrix") {
                let open = line
                    .iter()
                    .position(|byte| matches!(byte, b'[' | b'{'))
                    .ok_or(PdfType1SubsetError::InvalidFontMatrix)?;
                let close_char = if line[open] == b'[' { b']' } else { b'}' };
                let close = line
                    .iter()
                    .position(|byte| *byte == close_char)
                    .ok_or(PdfType1SubsetError::InvalidFontMatrix)?;
                let values = line[open + 1..close]
                    .split(u8::is_ascii_whitespace)
                    .filter(|part| !part.is_empty())
                    .map(|bytes| {
                        std::str::from_utf8(bytes)
                            .ok()
                            .and_then(|s| s.parse::<f32>().ok())
                    })
                    .collect::<Option<Vec<_>>>()
                    .ok_or(PdfType1SubsetError::InvalidFontMatrix)?;
                let [mut a, b, mut c, d, mut e, f] = <[f32; 6]>::try_from(values)
                    .map_err(|_| PdfType1SubsetError::InvalidFontMatrix)?;
                if ![a, b, c, d, e, f].into_iter().all(f32::is_finite) {
                    return Err(PdfType1SubsetError::InvalidFontMatrix);
                }
                // In writet1.c, the matrix slots are float, while `1E-3`
                // promotes each operation to double before assignment.
                let slant = f64::from(transform.slant) * 1e-3;
                a = (f64::from(a) + f64::from(b) * slant) as f32;
                c = (f64::from(c) + f64::from(d) * slant) as f32;
                e = (f64::from(e) + f64::from(f) * slant) as f32;
                if transform.extend != 0 {
                    let extend = f64::from(transform.extend) * 1e-3;
                    a = (f64::from(a) * extend) as f32;
                    c = (f64::from(c) * extend) as f32;
                    e = (f64::from(e) * extend) as f32;
                }
                if ![a, c, e].into_iter().all(f32::is_finite) {
                    return Err(PdfType1SubsetError::InvalidFontMatrix);
                }
                rewritten.extend_from_slice(&line[..open + 1]);
                for (index, value) in [a, b, c, d, e, f].into_iter().enumerate() {
                    if index != 0 {
                        rewritten.push(b' ');
                    }
                    rewritten.extend_from_slice(pdftex_g(value).as_bytes());
                }
                rewritten.extend_from_slice(&line[close..]);
                saw_matrix = true;
            } else if transform.slant != 0 && line.starts_with(b"/ItalicAngle") {
                let start = b"/ItalicAngle".len();
                let rest = &line[start..];
                let leading = rest
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .count();
                let value_end = rest[leading..]
                    .iter()
                    .position(u8::is_ascii_whitespace)
                    .ok_or(PdfType1SubsetError::InvalidItalicAngle)?
                    + leading;
                let value: f32 = std::str::from_utf8(&rest[leading..value_end])
                    .ok()
                    .and_then(|text| text.parse().ok())
                    .ok_or(PdfType1SubsetError::InvalidItalicAngle)?;
                if !value.is_finite() {
                    return Err(PdfType1SubsetError::InvalidItalicAngle);
                }
                let angle = (f64::from(value) - slant_angle(transform.slant)) as f32;
                if !angle.is_finite() {
                    return Err(PdfType1SubsetError::InvalidItalicAngle);
                }
                rewritten.extend_from_slice(&line[..start + leading]);
                rewritten.extend_from_slice(pdftex_g(angle).as_bytes());
                rewritten.extend_from_slice(&rest[value_end..]);
            } else {
                rewritten.extend_from_slice(line);
            }
        }
        if !saw_matrix {
            return Err(PdfType1SubsetError::InvalidFontMatrix);
        }
        let mut rewritten = replace_font_name(&rewritten, font_name)?;
        let length1 = u32::try_from(rewritten.len()).map_err(|_| PdfType1SubsetError::Overflow)?;
        rewritten.extend_from_slice(&self.bytes[clear_end..]);
        Ok(Self {
            identity: PdfType1ProgramIdentity(
                AHash64::for_bytes(HashDomain::Type1Program, &rewritten).to_le_bytes(),
            ),
            length1,
            length2: self.length2,
            length3: self.length3,
            bytes: rewritten,
        })
    }
}

fn slant_angle(slant: i32) -> f64 {
    (f64::from(slant) * 1e-3).atan().to_degrees()
}

/// Match C's default `%g`: six significant digits, with scientific notation
/// below 1e-4 or at 1e6 after rounding.
fn pdftex_g(value: f32) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific format");
    let exponent: i32 = exponent.parse().expect("scientific exponent");
    if !(-4..6).contains(&exponent) {
        let mantissa = trim_decimal(mantissa.to_owned());
        return format!("{mantissa}e{exponent:+03}");
    }
    let decimals = (5 - exponent) as usize;
    let mut text = format!("{value:.decimals$}");
    text = trim_decimal(text);
    text
}

fn trim_decimal(mut text: String) -> String {
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{PdfType1Program, PdfType1SubsetError, pdftex_g};

    #[test]
    fn type1_numeric_format_uses_six_significant_digits() {
        assert_eq!(pdftex_g(0.000167), "0.000167");
        assert_eq!(pdftex_g(0.0000167), "1.67e-05");
        assert_eq!(pdftex_g(1_000_000.0), "1e+06");
        assert_eq!(pdftex_g(999_999.5), "1e+06");
    }

    #[test]
    fn malformed_nonfinite_matrix_is_rejected_before_formatting() {
        let clear = b"%!PS\n/FontName /Fixture def\n/FontMatrix [NaN 0 0 0.001 0 0] def\n";
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(clear.len() as u32).to_le_bytes());
        pfb.extend_from_slice(clear);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        let program = PdfType1Program::from_pfb(&pfb).expect("syntactic PFB framing");
        assert_eq!(
            program.with_transform(
                crate::PdfType1Transform {
                    slant: 167,
                    extend: 0
                },
                b"Fixture-Slant_167"
            ),
            Err(PdfType1SubsetError::InvalidFontMatrix)
        );
    }
}

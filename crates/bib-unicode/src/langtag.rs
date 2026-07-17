#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LanguageTag {
    pub grandfathered: Option<String>,
    pub language: Option<String>,
    pub extlang: Vec<String>,
    pub script: Option<String>,
    pub region: Option<String>,
    pub variants: Vec<String>,
    pub extensions: Vec<String>,
    pub private_use: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LanguageTagError {
    Empty,
    InvalidSubtag,
    DuplicateRegion,
    MissingExtensionValue,
    TooLong,
}

impl LanguageTag {
    pub fn parse(input: &str) -> Result<Self, LanguageTagError> {
        if input.is_empty() {
            return Err(LanguageTagError::Empty);
        }
        if input.len() > 255 {
            return Err(LanguageTagError::TooLong);
        }
        if matches!(
            input.to_ascii_lowercase().as_str(),
            "i-enochian" | "i-klingon" | "en-gb-oed"
        ) {
            return Ok(Self {
                grandfathered: Some(input.to_owned()),
                ..Self::default()
            });
        }
        let parts: Vec<_> = input.split('-').collect();
        if parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(LanguageTagError::InvalidSubtag);
        }
        let language = parts[0];
        if !(2..=8).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic())
        {
            return Err(LanguageTagError::InvalidSubtag);
        }
        let mut out = Self {
            language: Some(language.to_ascii_lowercase()),
            ..Self::default()
        };
        let mut i = 1;
        while i < parts.len()
            && language.len() <= 3
            && out.extlang.len() < 3
            && parts[i].len() == 3
            && parts[i].bytes().all(|b| b.is_ascii_alphabetic())
        {
            out.extlang.push(parts[i].to_ascii_lowercase());
            i += 1;
        }
        if i < parts.len()
            && parts[i].len() == 4
            && parts[i].bytes().all(|b| b.is_ascii_alphabetic())
        {
            let lower = parts[i].to_ascii_lowercase();
            out.script = Some(format!(
                "{}{}",
                lower[..1].to_ascii_uppercase(),
                &lower[1..]
            ));
            i += 1;
        }
        if i < parts.len()
            && ((parts[i].len() == 2 && parts[i].bytes().all(|b| b.is_ascii_alphabetic()))
                || (parts[i].len() == 3 && parts[i].bytes().all(|b| b.is_ascii_digit())))
        {
            out.region = Some(parts[i].to_ascii_uppercase());
            i += 1;
        }
        while i < parts.len() {
            let part = parts[i];
            if part.eq_ignore_ascii_case("x") {
                i += 1;
                if i == parts.len() {
                    return Err(LanguageTagError::MissingExtensionValue);
                }
                out.private_use
                    .extend(parts[i..].iter().map(|s| (*s).to_owned()));
                return Ok(out);
            }
            if part.len() == 1 {
                i += 1;
                let start = i;
                while i < parts.len() && parts[i].len() >= 2 {
                    out.extensions.push(parts[i].to_ascii_lowercase());
                    i += 1;
                }
                if i == start {
                    return Err(LanguageTagError::MissingExtensionValue);
                }
            } else if (5..=8).contains(&part.len())
                || (part.len() == 4 && part.as_bytes()[0].is_ascii_digit())
            {
                out.variants.push(part.to_ascii_lowercase());
                i += 1;
            } else if ((part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
                || (part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit())))
                && out.region.is_some()
            {
                return Err(LanguageTagError::DuplicateRegion);
            } else {
                return Err(LanguageTagError::InvalidSubtag);
            }
        }
        Ok(out)
    }
}

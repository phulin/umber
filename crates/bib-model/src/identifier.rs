use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentifierError {
    kind: &'static str,
    value: String,
    message: &'static str,
}

impl IdentifierError {
    fn new(kind: &'static str, value: &str, message: &'static str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
            message,
        }
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} `{}`: {}",
            self.kind, self.value, self.message
        )
    }
}

impl std::error::Error for IdentifierError {}

macro_rules! text_identifier {
    ($name:ident, $kind:literal, $validator:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                $validator($kind, &value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

text_identifier!(EntryId, "entry identifier", validate_visible);
text_identifier!(DataListId, "data-list identifier", validate_visible);
text_identifier!(EntryType, "entry type", validate_symbol);
text_identifier!(FieldId, "field identifier", validate_symbol);
text_identifier!(OptionId, "option identifier", validate_symbol);
text_identifier!(
    TransformationId,
    "transformation identifier",
    validate_symbol
);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SectionId(u32);

impl SectionId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn validate_visible(kind: &'static str, value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::new(kind, value, "value is empty"));
    }
    if value.trim() != value {
        return Err(IdentifierError::new(
            kind,
            value,
            "leading or trailing whitespace is not allowed",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(IdentifierError::new(
            kind,
            value,
            "control characters are not allowed",
        ));
    }
    Ok(())
}

fn validate_symbol(kind: &'static str, value: &str) -> Result<(), IdentifierError> {
    validate_visible(kind, value)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
    {
        return Err(IdentifierError::new(
            kind,
            value,
            "only ASCII letters, digits, underscore, hyphen, and colon are allowed",
        ));
    }
    Ok(())
}

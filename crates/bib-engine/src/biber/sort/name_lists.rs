use std::fmt;

use bib_model::{Name, NameList};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NameListLimits {
    maximum: usize,
    minimum: usize,
}

impl NameListLimits {
    pub fn new(maximum: usize, minimum: usize) -> Result<Self, NameListLimitError> {
        if maximum == 0 {
            return Err(NameListLimitError::ZeroMaximum);
        }
        if minimum == 0 {
            return Err(NameListLimitError::ZeroMinimum);
        }
        if minimum > maximum {
            return Err(NameListLimitError::MinimumExceedsMaximum { minimum, maximum });
        }
        Ok(Self { maximum, minimum })
    }

    #[must_use]
    pub const fn maximum(self) -> usize {
        self.maximum
    }

    #[must_use]
    pub const fn minimum(self) -> usize {
        self.minimum
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameListLimitError {
    ZeroMaximum,
    ZeroMinimum,
    MinimumExceedsMaximum { minimum: usize, maximum: usize },
}

impl fmt::Display for NameListLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroMaximum => formatter.write_str("maximum visible names must be at least one"),
            Self::ZeroMinimum => formatter.write_str("minimum visible names must be at least one"),
            Self::MinimumExceedsMaximum { minimum, maximum } => write!(
                formatter,
                "minimum visible names ({minimum}) exceeds maximum ({maximum})"
            ),
        }
    }
}

impl std::error::Error for NameListLimitError {}

/// A non-owning decision about the visible prefix of a complete name list.
///
/// An explicit `others` marker always means that more names exist. It also
/// requests the configured minimum, capped by the number of concrete names.
/// Otherwise lists at or below the maximum remain intact and longer lists are
/// reduced to the minimum. Source order is never changed.
#[derive(Clone, Copy, Debug)]
pub struct NameListVisibility<'a> {
    names: &'a NameList,
    visible: usize,
    more_names: bool,
}

impl<'a> NameListVisibility<'a> {
    #[must_use]
    pub fn resolve(names: &'a NameList, limits: NameListLimits) -> Self {
        let must_truncate = names.has_others() || names.len() > limits.maximum;
        let visible = if must_truncate {
            names.len().min(limits.minimum)
        } else {
            names.len()
        };
        Self {
            names,
            visible,
            more_names: names.has_others() || visible < names.len(),
        }
    }

    #[must_use]
    pub const fn visible_count(self) -> usize {
        self.visible
    }

    #[must_use]
    pub const fn more_names(self) -> bool {
        self.more_names
    }

    #[must_use]
    pub fn is_truncated(self) -> bool {
        self.visible < self.names.len()
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = &'a Name> {
        self.names.iter().take(self.visible)
    }

    /// Materializes the visible prefix for consumers that need an owned list.
    /// `preserve_more_names` controls whether a truncation/explicit-others
    /// marker contributes to downstream hashing and serialization.
    #[must_use]
    pub fn to_name_list(self, preserve_more_names: bool) -> NameList {
        NameList::new(self.iter().cloned(), preserve_more_names && self.more_names)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NameVisibilityOptions {
    pub cite: NameListLimits,
    pub bibliography: NameListLimits,
    pub alpha: NameListLimits,
}

/// The three independently resolved visible-name decisions recorded by an
/// output list. Callers resolve global/type/entry precedence before creating
/// this value, so policy is deterministic and independent of mutable config.
#[derive(Clone, Copy, Debug)]
pub struct NameVisibility<'a> {
    cite: NameListVisibility<'a>,
    bibliography: NameListVisibility<'a>,
    alpha: NameListVisibility<'a>,
}

impl<'a> NameVisibility<'a> {
    #[must_use]
    pub fn resolve(names: &'a NameList, options: NameVisibilityOptions) -> Self {
        Self {
            cite: NameListVisibility::resolve(names, options.cite),
            bibliography: NameListVisibility::resolve(names, options.bibliography),
            alpha: NameListVisibility::resolve(names, options.alpha),
        }
    }

    #[must_use]
    pub const fn cite(self) -> NameListVisibility<'a> {
        self.cite
    }

    #[must_use]
    pub const fn bibliography(self) -> NameListVisibility<'a> {
        self.bibliography
    }

    #[must_use]
    pub const fn alpha(self) -> NameListVisibility<'a> {
        self.alpha
    }
}

#[cfg(test)]
mod tests;

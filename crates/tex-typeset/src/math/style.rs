use tex_state::math::MathFontSize;

/// TeX's four math style families, without the cramped bit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StyleFamily {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// Full Appendix G math style: D/T/S/SS plus TeX's cramped variants.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Style {
    family: StyleFamily,
    cramped: bool,
}

impl Style {
    pub const DISPLAY: Self = Self::new(StyleFamily::Display, false);
    pub const TEXT: Self = Self::new(StyleFamily::Text, false);
    pub const SCRIPT: Self = Self::new(StyleFamily::Script, false);
    pub const SCRIPT_SCRIPT: Self = Self::new(StyleFamily::ScriptScript, false);

    #[must_use]
    pub const fn new(family: StyleFamily, cramped: bool) -> Self {
        Self { family, cramped }
    }

    #[must_use]
    pub const fn family(self) -> StyleFamily {
        self.family
    }

    #[must_use]
    pub const fn cramped(self) -> bool {
        self.cramped
    }

    #[must_use]
    pub const fn is_display(self) -> bool {
        matches!(self.family, StyleFamily::Display)
    }

    #[must_use]
    pub const fn is_script_or_smaller(self) -> bool {
        matches!(self.family, StyleFamily::Script | StyleFamily::ScriptScript)
    }

    #[must_use]
    pub const fn size(self) -> MathFontSize {
        match self.family {
            StyleFamily::Display | StyleFamily::Text => MathFontSize::Text,
            StyleFamily::Script => MathFontSize::Script,
            StyleFamily::ScriptScript => MathFontSize::ScriptScript,
        }
    }

    #[must_use]
    pub const fn script_level(self) -> u8 {
        match self.family {
            StyleFamily::Display | StyleFamily::Text => 0,
            StyleFamily::Script => 1,
            StyleFamily::ScriptScript => 2,
        }
    }

    #[must_use]
    pub const fn cramped_style(self) -> Self {
        // AppG rule 17
        Self::new(self.family, true)
    }

    #[must_use]
    pub const fn sub_style(self) -> Self {
        // AppG rule 18
        Self::new(self.smaller_script_family(), true)
    }

    #[must_use]
    pub const fn sup_style(self) -> Self {
        // AppG rule 18
        Self::new(self.smaller_script_family(), self.cramped)
    }

    #[must_use]
    pub const fn num_style(self) -> Self {
        // AppG rule 15
        let code = self.code() + 2 - 2 * (self.code() / 6);
        Self::from_code(code)
    }

    #[must_use]
    pub const fn denom_style(self) -> Self {
        // AppG rule 15
        let code = 2 * (self.code() / 2) + 1 + 2 - 2 * (self.code() / 6);
        Self::from_code(code)
    }

    #[must_use]
    pub const fn from_math_style(style: tex_state::math::MathStyle) -> Self {
        match style {
            tex_state::math::MathStyle::Display => Self::DISPLAY,
            tex_state::math::MathStyle::Text => Self::TEXT,
            tex_state::math::MathStyle::Script => Self::SCRIPT,
            tex_state::math::MathStyle::ScriptScript => Self::SCRIPT_SCRIPT,
        }
    }

    const fn smaller_script_family(self) -> StyleFamily {
        match self.family {
            StyleFamily::Display | StyleFamily::Text => StyleFamily::Script,
            StyleFamily::Script | StyleFamily::ScriptScript => StyleFamily::ScriptScript,
        }
    }

    const fn code(self) -> u8 {
        let base = match self.family {
            StyleFamily::Display => 0,
            StyleFamily::Text => 2,
            StyleFamily::Script => 4,
            StyleFamily::ScriptScript => 6,
        };
        base + self.cramped as u8
    }

    const fn from_code(code: u8) -> Self {
        let family = match code / 2 {
            0 => StyleFamily::Display,
            1 => StyleFamily::Text,
            2 => StyleFamily::Script,
            _ => StyleFamily::ScriptScript,
        };
        Self::new(family, code % 2 == 1)
    }
}

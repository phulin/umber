use super::FusedError;

pub(super) const TAG_CHAR: u32 = 0;
const TAG_CONTROL: u32 = 1;
pub(super) const TAG_PARAMETER: u32 = 2;
pub(super) const TAG_BEGIN_GROUP: u32 = 3;
pub(super) const TAG_END_GROUP: u32 = 4;
const TAG_SHIFT: u32 = 24;
const VALUE_MASK: u32 = (1 << TAG_SHIFT) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Token(u32);

impl Token {
    const fn character(value: u8) -> Self {
        Self((TAG_CHAR << TAG_SHIFT) | value as u32)
    }

    const fn control(value: Control) -> Self {
        Self((TAG_CONTROL << TAG_SHIFT) | value as u32)
    }

    pub(super) const fn parameter(index: u8) -> Self {
        Self((TAG_PARAMETER << TAG_SHIFT) | index as u32)
    }

    const fn begin_group() -> Self {
        Self(TAG_BEGIN_GROUP << TAG_SHIFT)
    }

    const fn end_group() -> Self {
        Self(TAG_END_GROUP << TAG_SHIFT)
    }

    pub(super) const fn tag(self) -> u32 {
        self.0 >> TAG_SHIFT
    }

    const fn value(self) -> u32 {
        self.0 & VALUE_MASK
    }

    pub(super) fn as_char(self) -> Option<u8> {
        (self.tag() == TAG_CHAR).then(|| self.value() as u8)
    }

    pub(super) fn as_control(self) -> Option<Control> {
        (self.tag() == TAG_CONTROL).then(|| Control::from_raw(self.value() as u8))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Control {
    Count,
    Def,
    EmitE,
    EmitF,
    Advance,
    Global,
    IfNum,
    Else,
    Fi,
    Shipout,
    Hbox,
    Kern,
    Relax,
    End,
}

impl Control {
    fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Count,
            1 => Self::Def,
            2 => Self::EmitE,
            3 => Self::EmitF,
            4 => Self::Advance,
            5 => Self::Global,
            6 => Self::IfNum,
            7 => Self::Else,
            8 => Self::Fi,
            9 => Self::Shipout,
            10 => Self::Hbox,
            11 => Self::Kern,
            12 => Self::Relax,
            13 => Self::End,
            _ => unreachable!("validated packed control id"),
        }
    }

    pub(super) const fn macro_slot(self) -> Option<usize> {
        match self {
            Self::EmitE => Some(0),
            Self::EmitF => Some(1),
            _ => None,
        }
    }
}

pub(super) enum Frame<'a> {
    Source {
        bytes: &'a [u8],
        cursor: PackedCursor,
    },
    Packed {
        tokens: &'a [Token],
        cursor: PackedCursor,
        argument: Option<&'a [Token]>,
    },
}

const TOKEN_CURSOR_TAG: u32 = 1 << 31;
const CURSOR_POSITION_MASK: u32 = TOKEN_CURSOR_TAG - 1;

#[derive(Clone, Copy)]
pub(super) struct PackedCursor(u32);

impl PackedCursor {
    pub(super) const fn source() -> Self {
        Self(0)
    }

    pub(super) const fn tokens() -> Self {
        Self(TOKEN_CURSOR_TAG)
    }

    pub(super) const fn position(self) -> usize {
        (self.0 & CURSOR_POSITION_MASK) as usize
    }

    pub(super) fn advance(&mut self) {
        let position = self
            .position()
            .checked_add(1)
            .expect("cursor position must fit usize");
        assert!(position <= CURSOR_POSITION_MASK as usize, "cursor overflow");
        self.0 = (self.0 & TOKEN_CURSOR_TAG) | position as u32;
    }

    pub(super) const fn is_token_cursor(self) -> bool {
        self.0 & TOKEN_CURSOR_TAG != 0
    }
}

pub(super) fn lex_source(
    bytes: &[u8],
    cursor: &mut PackedCursor,
) -> Result<Option<Token>, FusedError> {
    while bytes
        .get(cursor.position())
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor.advance();
    }
    let Some(&byte) = bytes.get(cursor.position()) else {
        return Ok(None);
    };
    cursor.advance();
    match byte {
        b'{' => Ok(Some(Token::begin_group())),
        b'}' => Ok(Some(Token::end_group())),
        b'\\' => {
            let start = cursor.position();
            if bytes
                .get(cursor.position())
                .is_some_and(u8::is_ascii_alphabetic)
            {
                while bytes
                    .get(cursor.position())
                    .is_some_and(u8::is_ascii_alphabetic)
                {
                    cursor.advance();
                }
                let name = &bytes[start..cursor.position()];
                if bytes
                    .get(cursor.position())
                    .is_some_and(u8::is_ascii_whitespace)
                {
                    cursor.advance();
                }
                Ok(Some(Token::control(lookup_control(name)?)))
            } else {
                let Some(&single) = bytes.get(cursor.position()) else {
                    return Err(FusedError::UnexpectedEof("control symbol"));
                };
                cursor.advance();
                Ok(Some(Token::character(single)))
            }
        }
        value => Ok(Some(Token::character(value))),
    }
}

fn lookup_control(name: &[u8]) -> Result<Control, FusedError> {
    let control = match name {
        b"count" => Control::Count,
        b"def" => Control::Def,
        b"e" => Control::EmitE,
        b"f" => Control::EmitF,
        b"advance" => Control::Advance,
        b"global" => Control::Global,
        b"ifnum" => Control::IfNum,
        b"else" => Control::Else,
        b"fi" => Control::Fi,
        b"shipout" => Control::Shipout,
        b"hbox" => Control::Hbox,
        b"kern" => Control::Kern,
        b"relax" => Control::Relax,
        b"end" => Control::End,
        _ => {
            return Err(FusedError::UnknownControlSequence(
                String::from_utf8_lossy(name).into_owned(),
            ));
        }
    };
    Ok(control)
}

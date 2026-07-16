use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(u64),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Error {
    offset: usize,
    message: String,
}

impl Error {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON byte {}: {}", self.offset, self.message)
    }
}

pub(crate) fn parse(text: &str) -> Result<Value, Error> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        cursor: 0,
    };
    let value = parser.value()?;
    parser.whitespace();
    if parser.cursor != parser.bytes.len() {
        return Err(parser.error("trailing content"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Value, Error> {
        self.whitespace();
        match self.peek() {
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'0'..=b'9') => self.number().map(Value::Number),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, expected: &[u8], value: Value) -> Result<Value, Error> {
        if self.bytes.get(self.cursor..self.cursor + expected.len()) == Some(expected) {
            self.cursor += expected.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn number(&mut self) -> Result<u64, Error> {
        let start = self.cursor;
        if self.take() == Some(b'0') {
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("numbers may not have leading zeroes"));
            }
        } else {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(self.error("manifest numbers must be unsigned integers"));
        }
        let value = std::str::from_utf8(&self.bytes[start..self.cursor])
            .map_err(|_| self.error("invalid number"))?;
        value.parse().map_err(|_| self.error("integer exceeds u64"))
    }

    fn string(&mut self) -> Result<String, Error> {
        self.expect(b'"')?;
        let mut output = String::new();
        let mut segment = self.cursor;
        loop {
            let Some(byte) = self.take() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => {
                    self.push_utf8(&mut output, segment, self.cursor - 1)?;
                    return Ok(output);
                }
                b'\\' => {
                    self.push_utf8(&mut output, segment, self.cursor - 1)?;
                    self.escape(&mut output)?;
                    segment = self.cursor;
                }
                0..=0x1f => return Err(self.error("unescaped control character in string")),
                _ => {}
            }
        }
    }

    fn push_utf8(&self, output: &mut String, start: usize, end: usize) -> Result<(), Error> {
        output.push_str(
            std::str::from_utf8(&self.bytes[start..end])
                .map_err(|_| Error::new(start, "string is not valid UTF-8"))?,
        );
        Ok(())
    }

    fn escape(&mut self, output: &mut String) -> Result<(), Error> {
        match self.take() {
            Some(b'"') => output.push('"'),
            Some(b'\\') => output.push('\\'),
            Some(b'/') => output.push('/'),
            Some(b'b') => output.push('\u{0008}'),
            Some(b'f') => output.push('\u{000c}'),
            Some(b'n') => output.push('\n'),
            Some(b'r') => output.push('\r'),
            Some(b't') => output.push('\t'),
            Some(b'u') => {
                let first = self.hex_quad()?;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if self.take() != Some(b'\\') || self.take() != Some(b'u') {
                        return Err(
                            self.error("high surrogate must be followed by a low surrogate")
                        );
                    }
                    let second = self.hex_quad()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.error("invalid low surrogate"));
                    }
                    0x1_0000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(self.error("unexpected low surrogate"));
                } else {
                    u32::from(first)
                };
                output.push(
                    char::from_u32(scalar).ok_or_else(|| self.error("invalid Unicode escape"))?,
                );
            }
            _ => return Err(self.error("invalid string escape")),
        }
        Ok(())
    }

    fn hex_quad(&mut self) -> Result<u16, Error> {
        let mut value = 0_u16;
        for _ in 0..4 {
            value = value
                .checked_mul(16)
                .and_then(|value| self.take().and_then(hex).map(|digit| value + digit))
                .ok_or_else(|| self.error("invalid Unicode escape"))?;
        }
        Ok(value)
    }

    fn array(&mut self) -> Result<Value, Error> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.cursor += 1;
            return Ok(Value::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.whitespace();
            match self.take() {
                Some(b',') => {}
                Some(b']') => return Ok(Value::Array(values)),
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self) -> Result<Value, Error> {
        self.expect(b'{')?;
        let mut fields = BTreeMap::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.cursor += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(b':')?;
            let value = self.value()?;
            if fields.insert(key.clone(), value).is_some() {
                return Err(self.error(format!("duplicate object field {key:?}")));
            }
            self.whitespace();
            match self.take() {
                Some(b',') => {}
                Some(b'}') => return Ok(Value::Object(fields)),
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.cursor += 1;
        }
    }

    fn expect(&mut self, expected: u8) -> Result<(), Error> {
        if self.take() == Some(expected) {
            Ok(())
        } else {
            Err(self.error(format!("expected {:?}", char::from(expected))))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.cursor).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let value = self.peek()?;
        self.cursor += 1;
        Some(value)
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error::new(self.cursor, message)
    }
}

fn hex(byte: u8) -> Option<u16> {
    match byte {
        b'0'..=b'9' => Some(u16::from(byte - b'0')),
        b'a'..=b'f' => Some(u16::from(byte - b'a') + 10),
        b'A'..=b'F' => Some(u16::from(byte - b'A') + 10),
        _ => None,
    }
}

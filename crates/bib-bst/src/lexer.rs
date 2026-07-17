//! Byte-aware classic BST tokenization.

use crate::{CompileLimits, Diagnostic, DiagnosticKind, SourceLocation};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Identifier(String),
    Integer(i64),
    String(String),
    Quote,
    OpenBrace,
    CloseBrace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) location: SourceLocation,
}

pub(crate) struct Lexed {
    pub(crate) tokens: Vec<Token>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) nesting: usize,
    pub(crate) work: usize,
}

pub(crate) fn lex(bytes: &[u8], limits: CompileLimits) -> Lexed {
    let mut lexer = Lexer {
        bytes,
        limits,
        at: 0,
        line: 1,
        column: 1,
        tokens: Vec::new(),
        diagnostics: Vec::new(),
        nesting: 0,
        maximum_nesting: 0,
        work: 0,
    };
    while lexer.at < bytes.len() && lexer.charge() {
        lexer.skip_space_and_comments();
        if lexer.at >= bytes.len() {
            break;
        }
        let location = lexer.location();
        match lexer.peek() {
            b'{' => {
                lexer.bump();
                lexer.nesting += 1;
                lexer.maximum_nesting = lexer.maximum_nesting.max(lexer.nesting);
                lexer.push(TokenKind::OpenBrace, location);
            }
            b'}' => {
                lexer.bump();
                if lexer.nesting == 0 {
                    lexer.error(DiagnosticKind::Syntax, location, "unexpected closing brace");
                } else {
                    lexer.nesting -= 1;
                }
                lexer.push(TokenKind::CloseBrace, location);
            }
            b'\'' => {
                lexer.bump();
                lexer.push(TokenKind::Quote, location);
            }
            b'"' => lexer.string(location),
            b'#' => lexer.integer(location),
            byte if is_word(byte) => lexer.word(location),
            _ => {
                lexer.bump();
                lexer.error(DiagnosticKind::Syntax, location, "invalid BST byte");
            }
        }
        if lexer.tokens.len() >= limits.tokens {
            lexer.error(DiagnosticKind::Limit, location, "BST token limit exceeded");
            break;
        }
        if lexer.maximum_nesting > limits.nesting {
            lexer.error(
                DiagnosticKind::Limit,
                location,
                "BST nesting limit exceeded",
            );
            break;
        }
    }
    if lexer.nesting != 0 {
        lexer.error(
            DiagnosticKind::Syntax,
            lexer.location(),
            "unterminated brace group",
        );
    }
    Lexed {
        tokens: lexer.tokens,
        diagnostics: lexer.diagnostics,
        nesting: lexer.maximum_nesting,
        work: lexer.work,
    }
}

struct Lexer<'a> {
    bytes: &'a [u8],
    limits: CompileLimits,
    at: usize,
    line: usize,
    column: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    nesting: usize,
    maximum_nesting: usize,
    work: usize,
}

impl Lexer<'_> {
    fn charge(&mut self) -> bool {
        self.work += 1;
        if self.work > self.limits.work {
            self.error(
                DiagnosticKind::Limit,
                self.location(),
                "BST lexer work limit exceeded",
            );
            false
        } else {
            true
        }
    }
    fn peek(&self) -> u8 {
        self.bytes[self.at]
    }
    fn location(&self) -> SourceLocation {
        SourceLocation::new(self.at, self.line, self.column)
    }
    fn bump(&mut self) {
        let byte = self.bytes[self.at];
        self.at += 1;
        self.work += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }
    fn push(&mut self, kind: TokenKind, location: SourceLocation) {
        self.tokens.push(Token { kind, location });
    }
    fn error(&mut self, kind: DiagnosticKind, location: SourceLocation, message: &str) {
        if self.diagnostics.len() < self.limits.diagnostics {
            self.diagnostics
                .push(Diagnostic::new(kind, location, message));
        }
    }
    fn skip_space_and_comments(&mut self) {
        while self.at < self.bytes.len() && self.work <= self.limits.work {
            match self.peek() {
                b' ' | b'\t' | b'\r' | b'\n' => self.bump(),
                b'%' => {
                    while self.at < self.bytes.len()
                        && self.peek() != b'\n'
                        && self.work <= self.limits.work
                    {
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }
    fn word(&mut self, location: SourceLocation) {
        let start = self.at;
        while self.at < self.bytes.len() && is_word(self.peek()) && self.work <= self.limits.work {
            self.bump();
        }
        self.push(
            TokenKind::Identifier(compatibility_string(&self.bytes[start..self.at])),
            location,
        );
    }
    fn integer(&mut self, location: SourceLocation) {
        self.bump();
        let start = self.at;
        if self.at < self.bytes.len() && self.peek() == b'-' {
            self.bump();
        }
        let digits = self.at;
        while self.at < self.bytes.len()
            && self.peek().is_ascii_digit()
            && self.work <= self.limits.work
        {
            self.bump();
        }
        if digits == self.at {
            self.error(DiagnosticKind::Syntax, location, "expected digits after #");
            return;
        }
        let value = std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|value| value.parse().ok());
        match value {
            Some(value) => self.push(TokenKind::Integer(value), location),
            None => self.error(
                DiagnosticKind::Syntax,
                location,
                "BST integer is out of range",
            ),
        }
    }
    fn string(&mut self, location: SourceLocation) {
        self.bump();
        let start = self.at;
        while self.at < self.bytes.len() && self.peek() != b'"' && self.work <= self.limits.work {
            self.bump();
        }
        if self.at == self.bytes.len() {
            self.error(DiagnosticKind::Syntax, location, "unterminated BST string");
            return;
        }
        let text = compatibility_string(&self.bytes[start..self.at]);
        self.bump();
        self.push(TokenKind::String(text), location);
    }
}

fn is_word(byte: u8) -> bool {
    byte >= 0x80
        || byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'$' | b'.' | b':' | b'=' | b'<' | b'>' | b'+' | b'-' | b'*' | b'_'
        )
}

fn compatibility_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

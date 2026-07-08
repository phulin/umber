//! Expanded integer scanning shared by conditionals and future stomach code.

use std::fmt;

use tex_lex::{InputSource, InputStack, LexError, TokenListReplayKind};
use tex_state::ExpansionState;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, Token};

use crate::{
    ExpandError, ExpandNext, ExpansionHooks, NoInputExpandNext, NoopExpansionHooks, NoopRecorder,
    ReadRecorder,
};

const INT_MAX: i64 = i32::MAX as i64;
const MAX_REGISTER: i32 = 32_767;

/// A successfully scanned TeX integer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedInt {
    value: i32,
    diagnostic: Option<IntegerDiagnostic>,
}

impl ScannedInt {
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self {
            value,
            diagnostic: None,
        }
    }

    #[must_use]
    pub const fn with_diagnostic(value: i32, diagnostic: IntegerDiagnostic) -> Self {
        Self {
            value,
            diagnostic: Some(diagnostic),
        }
    }

    #[must_use]
    pub const fn value(self) -> i32 {
        self.value
    }

    #[must_use]
    pub const fn diagnostic(self) -> Option<IntegerDiagnostic> {
        self.diagnostic
    }
}

/// Recoverable diagnostics emitted while still producing TeX's capped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerDiagnostic {
    NumberTooBig,
}

impl fmt::Display for IntegerDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NumberTooBig => f.write_str("Number too big"),
        }
    }
}

/// Errors that prevent integer scanning from producing a value.
#[derive(Debug)]
pub enum ScanIntError {
    Expand(ExpandError),
    Lex(LexError),
    MissingNumber,
    RegisterNumberOutOfRange(i32),
    UnsupportedInternalInteger(Token),
}

impl fmt::Display for ScanIntError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expand(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::MissingNumber => f.write_str("Missing number"),
            Self::RegisterNumberOutOfRange(value) => {
                write!(f, "register number {value} is out of range")
            }
            Self::UnsupportedInternalInteger(token) => {
                write!(f, "unsupported internal integer token {token:?}")
            }
        }
    }
}

impl std::error::Error for ScanIntError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expand(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::MissingNumber
            | Self::RegisterNumberOutOfRange(_)
            | Self::UnsupportedInternalInteger(_) => None,
        }
    }
}

impl From<ExpandError> for ScanIntError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

impl From<LexError> for ScanIntError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

/// Scans a TeX `<number>` using expanded tokens.
///
/// Supported internal integers are the state surfaces implemented so far:
/// `\count<number>`, `\dimen<number>` coerced to scaled points, `\endlinechar`,
/// and chardef-like meanings represented by [`Meaning::CharGiven`].
pub fn scan_int<S>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
{
    scan_int_with_recorder_and_hooks(input, stores, &mut NoopRecorder, &mut NoopExpansionHooks)
}

/// Scans a TeX `<number>` while preserving caller-supplied expansion hooks.
pub fn scan_int_with_recorder_and_hooks<S, R, H>(
    input: &mut InputStack<S>,
    stores: &mut impl ExpansionState,
    recorder: &mut R,
    hooks: &mut H,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
{
    scan_int_with_expander_and_hooks(input, stores, recorder, hooks, &mut NoInputExpandNext)
}

pub fn scan_int_with_expander_and_hooks<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let (negative, token) = scan_signs(input, stores, recorder, hooks, expander)?;
    let Some(token) = token else {
        return Err(ScanIntError::MissingNumber);
    };

    let scanned = scan_unsigned_after_first_token(input, stores, recorder, hooks, expander, token)?;
    Ok(apply_sign(scanned, negative))
}

fn next_x<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<Option<Token>, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    Ok(expander.next_expanded_token(input, stores, recorder, hooks)?)
}

fn scan_signs<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(bool, Option<Token>), ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let mut negative = false;
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
            return Ok((negative, None));
        };
        if is_space(token) {
            continue;
        }
        if is_char(token, '+') {
            continue;
        }
        if is_char(token, '-') {
            negative = !negative;
            continue;
        }
        return Ok((negative, Some(token)));
    }
}

fn scan_unsigned_after_first_token<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: Token,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    match token {
        Token::Char {
            ch,
            cat: Catcode::Other,
        } if ch.is_ascii_digit() => scan_radix_digits(
            input,
            stores,
            recorder,
            hooks,
            expander,
            digit_value(ch).expect("matched digit"),
            10,
        ),
        Token::Char {
            ch: '\'',
            cat: Catcode::Other,
        } => scan_prefixed_digits(input, stores, recorder, hooks, expander, 8),
        Token::Char {
            ch: '"',
            cat: Catcode::Other,
        } => scan_prefixed_digits(input, stores, recorder, hooks, expander, 16),
        Token::Char {
            ch: '`',
            cat: Catcode::Other,
        } => scan_backtick_constant(input, stores, recorder, hooks, expander),
        Token::Cs(symbol) => {
            scan_internal_integer(input, stores, recorder, hooks, expander, token, symbol)
        }
        _ => Err(ScanIntError::MissingNumber),
    }
}

fn scan_prefixed_digits<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    radix: i64,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return Err(ScanIntError::MissingNumber);
    };
    let Some(digit) = token_digit_for_radix(token, radix) else {
        unread_token(input, stores, token);
        return Err(ScanIntError::MissingNumber);
    };
    scan_radix_digits(input, stores, recorder, hooks, expander, digit, radix)
}

fn scan_radix_digits<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    first_digit: i64,
    radix: i64,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let mut value = first_digit;
    let mut overflow = value > INT_MAX;
    loop {
        let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
            break;
        };
        let Some(digit) = token_digit_for_radix(token, radix) else {
            if !is_space(token) {
                unread_token(input, stores, token);
            }
            break;
        };
        match value
            .checked_mul(radix)
            .and_then(|value| value.checked_add(digit))
        {
            Some(next) if next <= INT_MAX => value = next,
            _ => {
                overflow = true;
                value = INT_MAX;
            }
        }
    }

    Ok(scanned_unsigned(value, overflow))
}

fn scan_backtick_constant<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return Err(ScanIntError::MissingNumber);
    };
    let value = match token {
        Token::Char { ch, .. } => ch as i32,
        Token::Cs(symbol) => stores
            .resolve(symbol)
            .chars()
            .next()
            .ok_or(ScanIntError::MissingNumber)? as i32,
        Token::Param(_) => return Err(ScanIntError::MissingNumber),
    };
    consume_optional_space(input, stores, recorder, hooks, expander)?;
    Ok(ScannedInt::new(value))
}

fn scan_internal_integer<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: Token,
    symbol: Symbol,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let meaning = stores.meaning(symbol);
    recorder.record_meaning(symbol, meaning);
    match meaning {
        Meaning::CharGiven(ch) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(ch as i32))
        }
        Meaning::MathCharGiven(value) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(i32::from(value)))
        }
        Meaning::CountRegister(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(stores.count(index)))
        }
        Meaning::DimenRegister(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(stores.dimen(index).raw()))
        }
        Meaning::IntParam(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(stores.int_param(IntParam::new(index))))
        }
        Meaning::DimenParam(index) => {
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(
                stores.dimen_param(DimenParam::new(index)).raw(),
            ))
        }
        Meaning::UnexpandablePrimitive(primitive) => scan_internal_integer_primitive(
            input, stores, recorder, hooks, expander, token, primitive,
        ),
        _ => {
            let name = stores.resolve(symbol);
            match name {
                "count" => {
                    let index = scan_register_index(input, stores, recorder, hooks, expander)?;
                    let value = stores.count(index);
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(value))
                }
                "dimen" => {
                    let index = scan_register_index(input, stores, recorder, hooks, expander)?;
                    let value = stores.dimen(index).raw();
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(value))
                }
                "endlinechar" => {
                    consume_optional_space(input, stores, recorder, hooks, expander)?;
                    Ok(ScannedInt::new(stores.int_param(IntParam::END_LINE_CHAR)))
                }
                _ => Err(ScanIntError::UnsupportedInternalInteger(token)),
            }
        }
    }
}

fn scan_internal_integer_primitive<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
    token: Token,
    primitive: tex_state::meaning::UnexpandablePrimitive,
) -> Result<ScannedInt, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    match primitive {
        tex_state::meaning::UnexpandablePrimitive::Count => {
            let index = scan_register_index(input, stores, recorder, hooks, expander)?;
            let value = stores.count(index);
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(value))
        }
        tex_state::meaning::UnexpandablePrimitive::Dimen => {
            let index = scan_register_index(input, stores, recorder, hooks, expander)?;
            let value = stores.dimen(index).raw();
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(value))
        }
        tex_state::meaning::UnexpandablePrimitive::CatCode
        | tex_state::meaning::UnexpandablePrimitive::LcCode
        | tex_state::meaning::UnexpandablePrimitive::UcCode
        | tex_state::meaning::UnexpandablePrimitive::SfCode
        | tex_state::meaning::UnexpandablePrimitive::MathCode
        | tex_state::meaning::UnexpandablePrimitive::DelCode => {
            let code =
                scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?.value();
            let ch = u32::try_from(code)
                .ok()
                .and_then(char::from_u32)
                .ok_or(ScanIntError::RegisterNumberOutOfRange(code))?;
            let value = match primitive {
                tex_state::meaning::UnexpandablePrimitive::CatCode => stores.catcode(ch) as i32,
                tex_state::meaning::UnexpandablePrimitive::LcCode => stores.lccode(ch) as i32,
                tex_state::meaning::UnexpandablePrimitive::UcCode => stores.uccode(ch) as i32,
                tex_state::meaning::UnexpandablePrimitive::SfCode => stores.sfcode(ch) as i32,
                tex_state::meaning::UnexpandablePrimitive::MathCode => stores.mathcode(ch) as i32,
                tex_state::meaning::UnexpandablePrimitive::DelCode => stores.delcode(ch),
                _ => unreachable!("outer match restricts primitive"),
            };
            consume_optional_space(input, stores, recorder, hooks, expander)?;
            Ok(ScannedInt::new(value))
        }
        _ => Err(ScanIntError::UnsupportedInternalInteger(token)),
    }
}

fn scan_register_index<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<u16, ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let value = scan_int_with_expander_and_hooks(input, stores, recorder, hooks, expander)?.value();
    if !(0..=MAX_REGISTER).contains(&value) {
        return Err(ScanIntError::RegisterNumberOutOfRange(value));
    }
    Ok(value as u16)
}

fn consume_optional_space<S, St, R, H, E>(
    input: &mut InputStack<S>,
    stores: &mut St,
    recorder: &mut R,
    hooks: &mut H,
    expander: &mut E,
) -> Result<(), ScanIntError>
where
    S: InputSource,
    St: ExpansionState,
    R: ReadRecorder,
    H: ExpansionHooks<S>,
    E: ExpandNext<S, St, R, H>,
{
    let Some(token) = next_x(input, stores, recorder, hooks, expander)? else {
        return Ok(());
    };
    if !is_space(token) {
        unread_token(input, stores, token);
    }
    Ok(())
}

fn unread_token<S>(input: &mut InputStack<S>, stores: &mut impl ExpansionState, token: Token) {
    let token_list = stores.intern_token_list(&[token]);
    input.push_token_list(token_list, TokenListReplayKind::Inserted);
}

fn apply_sign(scanned: ScannedInt, negative: bool) -> ScannedInt {
    let value = if negative {
        -scanned.value()
    } else {
        scanned.value()
    };
    ScannedInt {
        value,
        diagnostic: scanned.diagnostic(),
    }
}

fn scanned_unsigned(value: i64, overflow: bool) -> ScannedInt {
    if overflow {
        ScannedInt::with_diagnostic(i32::MAX, IntegerDiagnostic::NumberTooBig)
    } else {
        ScannedInt::new(value as i32)
    }
}

fn token_digit_for_radix(token: Token, radix: i64) -> Option<i64> {
    let Token::Char { ch, .. } = token else {
        return None;
    };
    let digit = digit_value(ch)?;
    (digit < radix).then_some(digit)
}

fn digit_value(ch: char) -> Option<i64> {
    match ch {
        '0'..='9' => Some(i64::from(ch as u8 - b'0')),
        'a'..='f' => Some(i64::from(ch as u8 - b'a' + 10)),
        'A'..='F' => Some(i64::from(ch as u8 - b'A' + 10)),
        _ => None,
    }
}

fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

fn is_char(token: Token, expected: char) -> bool {
    matches!(
        token,
        Token::Char {
            ch,
            cat: Catcode::Other
        } if ch == expected
    )
}

#[cfg(test)]
mod tests {
    use tex_lex::{InputStack, MemoryInput};
    use tex_state::env::banks::IntParam;
    use tex_state::macro_store::MacroMeaning;
    use tex_state::meaning::{Meaning, MeaningFlags};
    use tex_state::scaled::Scaled;
    use tex_state::token::{Catcode, Token};
    use tex_state::{ExpansionState, Universe};

    use crate::scan_int::{IntegerDiagnostic, ScanIntError, scan_int};

    fn scan(input: &str) -> (i32, Option<IntegerDiagnostic>, Option<Token>) {
        let mut stores = Universe::new();
        let mut input = InputStack::new(MemoryInput::new(input));
        let scanned = scan_int(&mut input, &mut stores).expect("integer scan should succeed");
        let next = input
            .next_token(&mut stores)
            .expect("remaining token should lex");
        (scanned.value(), scanned.diagnostic(), next)
    }

    fn scan_with_stores(
        input_text: &str,
        stores: &mut impl ExpansionState,
    ) -> (i32, Option<Token>) {
        let mut input = InputStack::new(MemoryInput::new(input_text));
        let scanned = scan_int(&mut input, stores).expect("integer scan should succeed");
        let next = input
            .next_token(stores)
            .expect("remaining token should lex");
        (scanned.value(), next)
    }

    fn char_token(ch: char, cat: Catcode) -> Token {
        Token::Char { ch, cat }
    }

    #[test]
    fn scans_repeated_signs_with_intervening_spaces() {
        let (value, diagnostic, next) = scan(" - + - 123x");

        assert_eq!(value, 123);
        assert_eq!(diagnostic, None);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn scans_decimal_octal_and_hex_constants() {
        assert_eq!(scan("123x").0, 123);
        assert_eq!(scan("'177x").0, 127);
        assert_eq!(scan("\"7F x").0, 127);
    }

    #[test]
    fn scans_backtick_character_and_control_sequence_constants() {
        let (value, _diagnostic, next) = scan("`A x");
        assert_eq!(value, 65);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));

        let mut stores = Universe::new();
        stores.intern("alpha");
        let (value, next) = scan_with_stores("`\\alpha x", &mut stores);
        assert_eq!(value, i32::from(b'a'));
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn consumes_at_most_one_trailing_space() {
        let (value, _diagnostic, next) = scan("12  x");

        assert_eq!(value, 12);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn leaves_non_space_terminator_available() {
        let (value, _diagnostic, next) = scan("12+x");

        assert_eq!(value, 12);
        assert_eq!(next, Some(char_token('+', Catcode::Other)));
    }

    #[test]
    fn scans_supported_internal_integers() {
        let mut stores = Universe::new();
        stores.intern("count");
        stores.intern("dimen");
        stores.intern("endlinechar");
        stores.set_count(12, -34);
        stores.set_dimen(3, Scaled::from_raw(65_536));
        stores.set_int_param(IntParam::END_LINE_CHAR, 13);

        assert_eq!(scan_with_stores("\\count12 x", &mut stores).0, -34);
        assert_eq!(scan_with_stores("\\dimen3 x", &mut stores).0, 65_536);
        assert_eq!(scan_with_stores("\\endlinechar x", &mut stores).0, 13);
    }

    #[test]
    fn scans_chardef_like_meanings() {
        let mut stores = Universe::new();
        let letter_a = stores.intern("a");
        stores.set_meaning(letter_a, Meaning::CharGiven('A'));

        let (value, next) = scan_with_stores("\\a x", &mut stores);

        assert_eq!(value, 65);
        assert_eq!(next, Some(char_token('x', Catcode::Letter)));
    }

    #[test]
    fn scans_values_through_macro_expansion() {
        let mut stores = Universe::new();
        let number = stores.intern("number");
        let replacement = stores.intern_token_list(&[
            char_token('4', Catcode::Other),
            char_token('2', Catcode::Other),
        ]);
        let params = stores.intern_token_list(&[]);
        stores.set_macro_meaning(
            number,
            MacroMeaning::new(MeaningFlags::EMPTY, params, replacement),
        );

        assert_eq!(scan_with_stores("\\number x", &mut stores).0, 42);
    }

    #[test]
    fn reports_number_too_big_and_caps_value() {
        let mut stores = Universe::new();
        let mut input = InputStack::new(MemoryInput::new("2147483648 x"));
        let scanned = scan_int(&mut input, &mut stores).expect("scan should cap overflow");

        assert_eq!(scanned.value(), i32::MAX);
        assert_eq!(scanned.diagnostic(), Some(IntegerDiagnostic::NumberTooBig));
        let diagnostic = scanned
            .diagnostic()
            .expect("overflow should emit diagnostic");
        assert_eq!(format!("{diagnostic}"), "Number too big");
    }

    #[test]
    fn rejects_out_of_range_register_numbers() {
        let mut stores = Universe::new();
        stores.intern("count");
        let mut input = InputStack::new(MemoryInput::new("\\count32768"));
        let err = scan_int(&mut input, &mut stores).expect_err("register should be rejected");

        assert!(matches!(err, ScanIntError::RegisterNumberOutOfRange(32768)));
    }
}

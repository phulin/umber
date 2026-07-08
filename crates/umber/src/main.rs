use std::env;
use std::fs::File;
use std::io;
use std::process::ExitCode;

use tex_lex::{FileInput, Lexer};
use tex_state::env::banks::IntParam;
use tex_state::stores::Stores;
use tex_state::token::Token;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("umber: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), CliError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("lex-dump") => {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing input path for lex-dump"));
            };
            if args.next().is_some() {
                return Err(CliError::Usage("lex-dump accepts exactly one input path"));
            }
            lex_dump(&path)
        }
        None => {
            println!("umber {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(_) => Err(CliError::Usage("expected: umber lex-dump <file.tex>")),
    }
}

#[allow(clippy::disallowed_methods)] // CLI entry point opens the user-requested file.
fn lex_dump(path: &str) -> Result<(), CliError> {
    let file = File::open(path)?;
    let mut stores = Stores::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(FileInput::from_file(file));

    while let Some(token) = lexer.next_token(&mut stores)? {
        println!("{}", format_token(token, &stores));
    }

    Ok(())
}

fn format_token(token: Token, stores: &Stores) -> String {
    match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
    }
}

#[derive(Debug)]
enum CliError {
    Usage(&'static str),
    Io(io::Error),
    Lex(tex_lex::LexError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<io::Error> for CliError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tex_lex::LexError> for CliError {
    fn from(value: tex_lex::LexError) -> Self {
        Self::Lex(value)
    }
}

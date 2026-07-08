use std::env;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_expand::ExpansionHooks;
use tex_lex::{FileInput, InputStack, Lexer};
use tex_state::env::banks::IntParam;
use tex_state::stores::Stores;
use tex_state::token::Token;

mod expand_dump;

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
        Some("expand-dump") => {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing input path for expand-dump"));
            };
            if args.next().is_some() {
                return Err(CliError::Usage(
                    "expand-dump accepts exactly one input path",
                ));
            }
            expand_dump::expand_dump(&path).map_err(CliError::ExpandDump)
        }
        Some("run") => {
            let Some(path) = args.next() else {
                return Err(CliError::Usage("missing input path for run"));
            };
            if args.next().is_some() {
                return Err(CliError::Usage("run accepts exactly one input path"));
            }
            run_tex(&path)
        }
        None => {
            println!("umber {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(_) => Err(CliError::Usage(
            "expected: umber <lex-dump|expand-dump|run> <file.tex>",
        )),
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

#[allow(clippy::disallowed_methods)] // CLI entry point opens the user-requested file.
fn run_tex(path: &str) -> Result<(), CliError> {
    let path = Path::new(path);
    let file = File::open(path)?;
    let mut stores = Stores::new();
    umber::prepare_run_stores(&mut stores);

    let mut input = InputStack::new(FileInput::from_file(file));
    let mut hooks = RunHooks::new(path);
    let log = umber::run_input_with_hooks(&mut input, &mut stores, &mut hooks)?;
    print!("{log}");
    Ok(())
}

#[allow(clippy::disallowed_methods)] // CLI driver opens files requested by \input.
struct RunHooks {
    base_dir: PathBuf,
    job_name: String,
}

impl RunHooks {
    fn new(path: &Path) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new(".")).to_owned();
        let job_name = path
            .file_stem()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("texput")
            .to_owned();
        Self { base_dir, job_name }
    }
}

impl ExpansionHooks<FileInput> for RunHooks {
    #[allow(clippy::disallowed_methods)] // CLI driver opens files requested by \input.
    fn open_input(&mut self, name: &str) -> Result<FileInput, String> {
        let mut path = self.base_dir.join(name);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        File::open(&path)
            .map(FileInput::from_file)
            .map_err(|err| format!("{} ({err})", path.display()))
    }

    fn job_name(&self) -> &str {
        &self.job_name
    }
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
    ExpandDump(expand_dump::ExpandDumpError),
    Exec(tex_exec::ExecError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::Io(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ExpandDump(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
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

impl From<tex_exec::ExecError> for CliError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Exec(value)
    }
}

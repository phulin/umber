use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_expand::ExpansionHooks;
use tex_lex::{InputStack, Lexer, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{Universe, World, WorldError};

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
            let mut show_fixtures = false;
            let mut rest: Vec<String> = args.collect();
            if rest.first().is_some_and(|arg| arg == "--show-fixtures") {
                show_fixtures = true;
                rest.remove(0);
            }
            if rest.last().is_some_and(|arg| arg == "--show-fixtures") {
                show_fixtures = true;
                rest.pop();
            }
            let [path] = rest.as_slice() else {
                if rest.is_empty() {
                    return Err(CliError::Usage("missing input path for run"));
                }
                return Err(CliError::Usage(
                    "run accepts one input path and optional --show-fixtures",
                ));
            };
            if path == "--show-fixtures" {
                return Err(CliError::Usage("missing input path for run"));
            }
            run_tex(path, show_fixtures)
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

fn lex_dump(path: &str) -> Result<(), CliError> {
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(WorldInput::from_content(content));

    while let Some(token) = lexer.next_token(&mut stores)? {
        println!("{}", format_token(token, &stores));
    }

    Ok(())
}

fn run_tex(path: &str, _show_fixtures: bool) -> Result<(), CliError> {
    let path = Path::new(path);
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    umber::prepare_run_stores(&mut stores);

    let mut input = InputStack::new(WorldInput::from_content(content));
    let mut hooks = RunHooks::new(path);
    let _run = umber::run_input_collecting_artifacts(&mut input, &mut stores, &mut hooks)?;
    let effect_pos = stores.world().effect_pos();
    stores.commit_effects(effect_pos)?;
    Ok(())
}

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

impl ExpansionHooks<WorldInput> for RunHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<WorldInput, String> {
        let mut path = self.base_dir.join(name);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        input
            .read_input_file(&path)
            .map(WorldInput::from_content)
            .map_err(|err| format!("{} ({err})", path.display()))
    }

    fn job_name(&self) -> &str {
        &self.job_name
    }
}

fn format_token(token: Token, stores: &Universe) -> String {
    match token {
        Token::Char { ch, cat } => format!("char:{}:{}", ch as u32, cat as u8),
        Token::Cs(symbol) => format!("cs:{}", stores.resolve(symbol)),
        Token::Param(slot) => format!("param:{slot}"),
    }
}

#[derive(Debug)]
enum CliError {
    Usage(&'static str),
    World(WorldError),
    Lex(tex_lex::LexError),
    ExpandDump(expand_dump::ExpandDumpError),
    Exec(tex_exec::ExecError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::World(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ExpandDump(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<WorldError> for CliError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
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

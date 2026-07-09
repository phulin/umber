use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack, Lexer, MemoryInput, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::token::Token;
use tex_state::{Universe, World, WorldError};

mod expand_dump;

const PLAIN_CORPUS_BOOTSTRAP: &str = r##"
% Minimal plain-format prelude for external corpus parity.
% This is not plain.tex; umber2-sfc.1 owns full plain.tex bring-up.
\catcode`\@=11
\font\tenrm=cmr10
\font\tenbf=cmbx10
\font\tensl=cmsl10
\font\tentt=cmtt10
\font\tenit=cmti10
\let\rm=\tenrm \let\bf=\tenbf \let\sl=\tensl \let\tt=\tentt \let\it=\tenit
\tenrm
\countdef\pageno=0 \pageno=1
\toksdef\headline=10 \toksdef\footline=11 \headline={} \footline={}
\countdef\footnotenum=20 \countdef\exno=21 \countdef\secnum=22
\countdef\subsecnum=23 \countdef\hour=24 \countdef\minute=25
\dimendef\theight=20 \dimendef\squaredimen=21
\chardef\contents=0 \chardef\index=1
\def\newcount#1{} \def\newdimen#1{} \def\newwrite#1{}
\def\fmtname{plain}
\def\fmtversion{3.141592653}
\hsize=6.5in \vsize=8.9in \maxdepth=4pt
\topskip=10pt \baselineskip=12pt \lineskip=1pt \lineskiplimit=0pt
\parindent=20pt \parskip=0pt plus 1pt \parfillskip=0pt plus 1fil
\def\line#1{\hbox to\hsize{#1}}
\def\leftline#1{\line{#1\hss}}
\def\rightline#1{\line{\hss#1}}
\def\centerline#1{\line{\hss#1\hss}}
\def\"#1{{\accent127 #1}}
\def\c#1{{\accent24 #1}}
\def\ae{\char26 }
\def\break{\penalty-10000 }
\def\eject{\par\break}
\def\bye{\par\vfill\eject\end}
\def\folio{\ifnum\pageno<0 \romannumeral-\pageno \else\number\pageno \fi}
\def\nopagenumbers{\headline={}\footline={}}
\def\raggedbottom{}
\def\smallskip{\vskip 3pt plus 1pt minus 1pt}
\def\medskip{\vskip 6pt plus 2pt minus 2pt}
\def\bigskip{\vskip 12pt plus 4pt minus 4pt}
\def\loop#1\repeat{\def\body{#1}\iterate}
\def\iterate{\body \let\next=\iterate \else\let\next=\relax\fi \next}
\let\repeat=\fi
\def\magstep#1{\ifcase#1 1000\or 1200\or 1440\or 1728\or 2074\or 2488\fi}
\def\magstephalf{1095}
\def\newif#1{}
\let\ifamrfonts=\iffalse
\def\amrfontstrue{\let\ifamrfonts=\iftrue}
\def\amrfontsfalse{\let\ifamrfonts=\iffalse}
\let\ifcanspell=\iffalse
\def\canspelltrue{\let\ifcanspell=\iftrue}
\def\canspellfalse{\let\ifcanspell=\iffalse}
\let\iftitlepage=\iffalse
\def\titlepagetrue{\let\iftitlepage=\iftrue}
\def\titlepagefalse{\let\iftitlepage=\iffalse}
\let\ifwritingcontents=\iffalse
\def\writingcontentstrue{\let\ifwritingcontents=\iftrue}
\def\writingcontentsfalse{\let\ifwritingcontents=\iffalse}
\let\ifwritingindex=\iffalse
\def\writingindextrue{\let\ifwritingindex=\iftrue}
\def\writingindexfalse{\let\ifwritingindex=\iffalse}
\let\ifwritinganswers=\iffalse
\def\writinganswerstrue{\let\ifwritinganswers=\iftrue}
\def\writinganswersfalse{\let\ifwritinganswers=\iffalse}
\catcode`\@=12
"##;

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
            let opts = RunCliOptions::parse(args)?;
            run_tex(&opts)
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

fn run_tex(opts: &RunCliOptions) -> Result<(), CliError> {
    let path = opts.input.as_path();
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    umber::prepare_run_stores(&mut stores);

    let mut input = InputStack::new(RunInputSource::World(WorldInput::from_content(content)));
    if opts.plain_format {
        input.push_source(RunInputSource::Memory(MemoryInput::new(
            PLAIN_CORPUS_BOOTSTRAP,
        )));
    }
    let mut hooks = RunHooks::new(path);
    let run = umber::run_input_collecting_artifacts(&mut input, &mut stores, &mut hooks)?;
    if let Some(output) = &opts.dvi {
        let dvi = umber::dvi_from_artifacts(&stores, &run.artifacts)?;
        stores.world_mut().write_file(output, dvi)?;
    }
    if opts.show_fixtures {
        print!("{}", run.terminal_text);
        return Ok(());
    }
    let effect_pos = stores.world().effect_pos();
    stores.commit_effects(effect_pos)?;
    Ok(())
}

struct RunCliOptions {
    input: PathBuf,
    show_fixtures: bool,
    dvi: Option<PathBuf>,
    plain_format: bool,
}

impl RunCliOptions {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, CliError> {
        let mut input = None;
        let mut show_fixtures = false;
        let mut dvi = None;
        let mut plain_format = false;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--show-fixtures" => {
                    show_fixtures = true;
                }
                "--plain-format" => {
                    plain_format = true;
                }
                "--dvi" => {
                    if dvi.is_some() {
                        return Err(CliError::Usage("run accepts at most one --dvi output path"));
                    }
                    let Some(path) = args.next() else {
                        return Err(CliError::Usage("missing output path for --dvi"));
                    };
                    dvi = Some(PathBuf::from(path));
                }
                flag if flag.starts_with('-') => {
                    return Err(CliError::Usage(
                        "run accepts one input path with optional --show-fixtures, --plain-format, and --dvi <path>",
                    ));
                }
                path => {
                    if input.is_some() {
                        return Err(CliError::Usage(
                            "run accepts one input path with optional --show-fixtures, --plain-format, and --dvi <path>",
                        ));
                    }
                    input = Some(PathBuf::from(path));
                }
            }
        }
        let input = input.ok_or(CliError::Usage("missing input path for run"))?;
        Ok(Self {
            input,
            show_fixtures,
            dvi,
            plain_format,
        })
    }
}

enum RunInputSource {
    Memory(MemoryInput),
    World(WorldInput),
}

impl InputSource for RunInputSource {
    fn read_line(&mut self) -> Result<Option<String>, WorldError> {
        match self {
            Self::Memory(input) => input.read_line(),
            Self::World(input) => input.read_line(),
        }
    }
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

impl ExpansionHooks<RunInputSource> for RunHooks {
    fn open_input<C: tex_state::InputReadState>(
        &mut self,
        input: &mut C,
        name: &str,
    ) -> Result<RunInputSource, String> {
        let mut path = self.base_dir.join(name);
        if path.extension().is_none() {
            path.set_extension("tex");
        }
        input
            .read_input_file(&path)
            .map(WorldInput::from_content)
            .map(RunInputSource::World)
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
    Dvi(umber::DviBuildError),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(message) => f.write_str(message),
            Self::World(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::ExpandDump(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::Dvi(err) => write!(f, "{err}"),
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

impl From<umber::DviBuildError> for CliError {
    fn from(value: umber::DviBuildError) -> Self {
        Self::Dvi(value)
    }
}

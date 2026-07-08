use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use tex_exec::try_execute_assignment;
use tex_expand::{ExpandError, ExpansionHooks, get_x_token_with_hooks};
use tex_lex::{FileInput, InputStack, LexError};
use tex_state::env::banks::IntParam;
use tex_state::meaning::Meaning;
use tex_state::stores::Stores;
use tex_state::token::Token;

use crate::format_token;

#[allow(clippy::disallowed_methods)] // CLI entry point opens the user-requested file.
pub fn expand_dump(path: &str) -> Result<(), ExpandDumpError> {
    let path = Path::new(path);
    let file = File::open(path)?;
    let mut stores = Stores::new();
    install_dump_primitives(&mut stores);

    let input = InputStack::new(FileInput::from_file(file));
    let mut driver = DumpDriver {
        input,
        stores,
        hooks: FileHooks::new(path),
    };
    driver.dump()
}

struct DumpDriver {
    input: InputStack<FileInput>,
    stores: Stores,
    hooks: FileHooks,
}

impl DumpDriver {
    fn dump(&mut self) -> Result<(), ExpandDumpError> {
        while let Some(token) = self.next_delivered()? {
            if try_execute_assignment(token, &mut self.input, &mut self.stores, &mut self.hooks)? {
                continue;
            }
            println!("{}", format_token(token, &self.stores));
        }
        Ok(())
    }

    fn next_delivered(&mut self) -> Result<Option<Token>, ExpandDumpError> {
        Ok(get_x_token_with_hooks(
            &mut self.input,
            &mut self.stores,
            &mut self.hooks,
        )?)
    }
}

#[allow(clippy::disallowed_methods)] // CLI driver opens user-requested TeX inputs.
struct FileHooks {
    base_dir: PathBuf,
    job_name: String,
}

impl FileHooks {
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

impl ExpansionHooks<FileInput> for FileHooks {
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

fn install_dump_primitives(stores: &mut Stores) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    stores.intern("par");
    tex_exec::install_unexpandable_primitives(stores);

    tex_expand::install_expandable_primitives(stores);

    for name in [
        "def",
        "edef",
        "gdef",
        "xdef",
        "long",
        "outer",
        "protected",
        "global",
        "globaldefs",
        "let",
        "futurelet",
        "chardef",
        "catcode",
        "count",
        "dimen",
        "toks",
        "endlinechar",
        "escapechar",
    ] {
        stores.intern(name);
    }
}

#[derive(Debug)]
pub enum ExpandDumpError {
    Io(io::Error),
    Exec(tex_exec::ExecError),
    Lex(LexError),
    Expand(ExpandError),
}

impl std::fmt::Display for ExpandDumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ExpandDumpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Exec(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
        }
    }
}

impl From<io::Error> for ExpandDumpError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<tex_exec::ExecError> for ExpandDumpError {
    fn from(value: tex_exec::ExecError) -> Self {
        Self::Exec(value)
    }
}

impl From<LexError> for ExpandDumpError {
    fn from(value: LexError) -> Self {
        Self::Lex(value)
    }
}

impl From<ExpandError> for ExpandDumpError {
    fn from(value: ExpandError) -> Self {
        Self::Expand(value)
    }
}

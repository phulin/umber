use std::path::{Path, PathBuf};

use tex_exec::try_execute_assignment;
use tex_expand::{ExpandError, ExpansionHooks, get_x_token_with_hooks, semantic_token};
use tex_lex::{InputStack, LexError, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::meaning::Meaning;
use tex_state::token::Token;
use tex_state::{ExpansionContext, Universe, World, WorldError};

use crate::format_token;

pub fn expand_dump(path: &str) -> Result<(), ExpandDumpError> {
    let path = Path::new(path);
    let mut stores = Universe::with_world(World::real());
    let content = stores.world_mut().read_file(path)?;
    install_dump_primitives(&mut stores);

    let input = InputStack::new(WorldInput::from_content(content));
    let mut driver = DumpDriver {
        input,
        stores,
        hooks: FileHooks::new(path),
    };
    driver.dump()
}

struct DumpDriver {
    input: InputStack<WorldInput>,
    stores: Universe,
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
        let mut expansion = ExpansionContext::new(&mut self.stores);
        Ok(
            get_x_token_with_hooks(&mut self.input, &mut expansion, &mut self.hooks)?
                .map(semantic_token),
        )
    }
}

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

impl ExpansionHooks<WorldInput> for FileHooks {
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

fn install_dump_primitives(stores: &mut Universe) {
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
    World(WorldError),
    Exec(tex_exec::ExecError),
    Lex(LexError),
    Expand(ExpandError),
}

impl std::fmt::Display for ExpandDumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(err) => write!(f, "{err}"),
            Self::Exec(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ExpandDumpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::World(err) => Some(err),
            Self::Exec(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
        }
    }
}

impl From<WorldError> for ExpandDumpError {
    fn from(value: WorldError) -> Self {
        Self::World(value)
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

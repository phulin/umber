use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use tex_expand::scan::{ScanToksError, scan_toks};
use tex_expand::scan_int;
use tex_expand::{
    Dispatch, ExpandError, ExpansionHooks, ExpansionReplayKind, NoopRecorder, dispatch_with_hooks,
    get_x_token_with_hooks,
};
use tex_lex::{FileInput, InputStack, LexError, MemoryInput, TokenListReplayKind};
use tex_state::env::banks::IntParam;
use tex_state::macro_store::MacroMeaning;
use tex_state::meaning::{ExpandablePrimitive, Meaning, MeaningFlags};
use tex_state::stores::Stores;
use tex_state::token::{Catcode, Token};

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
        pending: VecDeque::new(),
    };
    driver.dump()
}

struct DumpDriver {
    input: InputStack<FileInput>,
    stores: Stores,
    hooks: FileHooks,
    pending: VecDeque<Token>,
}

impl DumpDriver {
    fn dump(&mut self) -> Result<(), ExpandDumpError> {
        while let Some(token) = self.next_delivered()? {
            if self.try_consume_driver_form(token)? {
                continue;
            }
            println!("{}", format_token(token, &self.stores));
        }
        Ok(())
    }

    fn next_delivered(&mut self) -> Result<Option<Token>, ExpandDumpError> {
        if let Some(token) = self.pending.pop_front() {
            return Ok(Some(token));
        }
        Ok(get_x_token_with_hooks(
            &mut self.input,
            &mut self.stores,
            &mut self.hooks,
        )?)
    }

    fn next_raw(&mut self) -> Result<Option<Token>, ExpandDumpError> {
        Ok(self.input.next_token(&mut self.stores)?)
    }

    fn next_non_space_raw(&mut self) -> Result<Option<Token>, ExpandDumpError> {
        loop {
            let Some(token) = self.next_raw()? else {
                return Ok(None);
            };
            if !is_space(token) {
                return Ok(Some(token));
            }
        }
    }

    fn next_non_space_x(&mut self) -> Result<Option<Token>, ExpandDumpError> {
        loop {
            let Some(token) =
                get_x_token_with_hooks(&mut self.input, &mut self.stores, &mut self.hooks)?
            else {
                return Ok(None);
            };
            if !is_space(token) {
                return Ok(Some(token));
            }
        }
    }

    fn try_consume_driver_form(&mut self, first: Token) -> Result<bool, ExpandDumpError> {
        let Token::Cs(symbol) = first else {
            return Ok(false);
        };
        match self.stores.resolve(symbol) {
            "def" => self.consume_macro_definition(MeaningFlags::EMPTY, false, false),
            "edef" => self.consume_macro_definition(MeaningFlags::EMPTY, false, true),
            "gdef" => self.consume_macro_definition(MeaningFlags::EMPTY, true, false),
            "xdef" => self.consume_macro_definition(MeaningFlags::EMPTY, true, true),
            "let" => self.consume_let(false),
            "chardef" => self.consume_chardef(false),
            "catcode" => self.consume_catcode(),
            "long" | "outer" | "global" => self.consume_prefixed(first),
            _ => Ok(false),
        }
    }

    fn consume_prefixed(&mut self, first: Token) -> Result<bool, ExpandDumpError> {
        let mut global = false;
        let mut flags = MeaningFlags::EMPTY;
        let mut consumed = vec![first];

        loop {
            let Some(token) = self.next_delivered()? else {
                self.pending.extend(consumed);
                return Ok(false);
            };
            consumed.push(token);
            let Token::Cs(symbol) = token else {
                self.replay_unconsumed(consumed);
                return Ok(false);
            };

            match self.stores.resolve(symbol) {
                "long" => flags = flags | MeaningFlags::LONG,
                "outer" => flags = flags | MeaningFlags::OUTER,
                "global" => global = true,
                "def" => return self.consume_macro_definition(flags, global, false),
                "edef" => return self.consume_macro_definition(flags, global, true),
                "gdef" => return self.consume_macro_definition(flags, true, false),
                "xdef" => return self.consume_macro_definition(flags, true, true),
                "let" => return self.consume_let(global),
                "chardef" => return self.consume_chardef(global),
                _ => {
                    self.replay_unconsumed(consumed);
                    return Ok(false);
                }
            }
        }
    }

    fn replay_unconsumed(&mut self, tokens: Vec<Token>) {
        self.pending.extend(tokens);
    }

    fn consume_macro_definition(
        &mut self,
        flags: MeaningFlags,
        global: bool,
        expanded: bool,
    ) -> Result<bool, ExpandDumpError> {
        let Some(target) = self.next_non_space_raw()? else {
            return Err(ExpandDumpError::Definition(
                "missing control sequence after macro definition",
            ));
        };
        let Token::Cs(target) = target else {
            return Err(ExpandDumpError::Definition(
                "macro definition target must be a control sequence",
            ));
        };

        let scanned = scan_toks(&mut self.input, &mut self.stores, flags)?;
        let mut meaning = scanned.meaning();
        if expanded {
            let expanded_body =
                expand_replacement_text(&mut self.stores, meaning.replacement_text())?;
            meaning = MacroMeaning::new(flags, meaning.parameter_text(), expanded_body);
        }

        if global {
            self.stores.set_macro_meaning_global(target, meaning);
        } else {
            self.stores.set_macro_meaning(target, meaning);
        }
        Ok(true)
    }

    fn consume_let(&mut self, global: bool) -> Result<bool, ExpandDumpError> {
        let Some(target) = self.next_non_space_raw()? else {
            return Err(ExpandDumpError::Definition(
                "missing control sequence after \\let",
            ));
        };
        let Token::Cs(target) = target else {
            return Err(ExpandDumpError::Definition(
                "\\let target must be a control sequence",
            ));
        };

        let rhs = self.next_optional_equals_raw()?;
        let meaning = match rhs {
            Token::Cs(symbol) => self.stores.meaning(symbol),
            Token::Char { ch, .. } => Meaning::CharGiven(ch),
            Token::Param(_) => {
                return Err(ExpandDumpError::Definition(
                    "\\let cannot assign a macro parameter token in expand-dump",
                ));
            }
        };
        if global {
            self.stores.set_meaning_global(target, meaning);
        } else {
            self.stores.set_meaning(target, meaning);
        }
        Ok(true)
    }

    fn consume_chardef(&mut self, global: bool) -> Result<bool, ExpandDumpError> {
        let Some(target) = self.next_non_space_raw()? else {
            return Err(ExpandDumpError::Definition(
                "missing control sequence after \\chardef",
            ));
        };
        let Token::Cs(target) = target else {
            return Err(ExpandDumpError::Definition(
                "\\chardef target must be a control sequence",
            ));
        };
        self.skip_optional_equals_x()?;
        let value = scan_int::scan_int(&mut self.input, &mut self.stores)?.value();
        let Some(ch) = u32::try_from(value).ok().and_then(char::from_u32) else {
            return Err(ExpandDumpError::Definition(
                "\\chardef value is not a valid character",
            ));
        };
        if global {
            self.stores
                .set_meaning_global(target, Meaning::CharGiven(ch));
        } else {
            self.stores.set_meaning(target, Meaning::CharGiven(ch));
        }
        Ok(true)
    }

    fn consume_catcode(&mut self) -> Result<bool, ExpandDumpError> {
        let code = scan_int::scan_int(&mut self.input, &mut self.stores)?.value();
        self.skip_optional_equals_x()?;
        let catcode = scan_int::scan_int(&mut self.input, &mut self.stores)?.value();
        let Some(ch) = u32::try_from(code).ok().and_then(char::from_u32) else {
            return Err(ExpandDumpError::Definition(
                "\\catcode character code is invalid",
            ));
        };
        let cat = catcode_from_i32(catcode)?;
        self.stores.set_catcode(ch, cat);
        Ok(true)
    }

    fn next_optional_equals_raw(&mut self) -> Result<Token, ExpandDumpError> {
        let Some(token) = self.next_non_space_raw()? else {
            return Err(ExpandDumpError::Definition(
                "missing token after optional equals",
            ));
        };
        if is_other_equals(token) {
            self.next_non_space_raw()?
                .ok_or(ExpandDumpError::Definition("missing token after equals"))
        } else {
            Ok(token)
        }
    }

    fn skip_optional_equals_x(&mut self) -> Result<(), ExpandDumpError> {
        let Some(token) = self.next_non_space_x()? else {
            return Err(ExpandDumpError::Definition("missing assignment value"));
        };
        if !is_other_equals(token) {
            self.pending.push_front(token);
        }
        Ok(())
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

fn expand_replacement_text(
    stores: &mut Stores,
    replacement_text: tex_state::ids::TokenListId,
) -> Result<tex_state::ids::TokenListId, ExpandDumpError> {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(replacement_text, TokenListReplayKind::Inserted);
    let mut builder = stores.token_list_builder();
    let mut hooks = EdefHooks;
    let mut recorder = NoopRecorder;

    loop {
        let Some(read) = input.next_expansion_token(stores)? else {
            break;
        };
        let token = read.token();
        if read.suppress_expansion() {
            builder.push(token);
            continue;
        }

        let Token::Cs(symbol) = token else {
            builder.push(token);
            continue;
        };
        let meaning = stores.meaning(symbol);
        if meaning == Meaning::ExpandablePrimitive(ExpandablePrimitive::NoExpand) {
            builder.push(token);
            let Some(suppressed) = input.next_token(stores)? else {
                return Err(ExpandError::MissingTokenAfterPrimitive(
                    tex_expand::ExpandableOpcode::NoExpand,
                )
                .into());
            };
            builder.push(suppressed);
            continue;
        }

        match dispatch_with_hooks(
            token,
            &mut input,
            stores,
            &mut recorder,
            &mut hooks,
            meaning,
        )? {
            Dispatch::Continue => {}
            Dispatch::Deliver(token) | Dispatch::DeliverNoExpand(token) => builder.push(token),
            push @ Dispatch::Push { .. } => apply_edef_push(&mut input, push),
        }
    }
    Ok(stores.finish_token_list(&mut builder))
}

fn apply_edef_push(input: &mut InputStack<MemoryInput>, dispatch: Dispatch) {
    let Dispatch::Push {
        replay_kind,
        token_list,
        macro_arguments,
    } = dispatch
    else {
        return;
    };
    if replay_kind == ExpansionReplayKind::MacroBody {
        input.push_macro_body(token_list, macro_arguments);
    } else {
        input.push_token_list(token_list, replay_kind.as_lex_kind());
    }
}

struct EdefHooks;

impl ExpansionHooks<MemoryInput> for EdefHooks {
    fn open_input(&mut self, name: &str) -> Result<MemoryInput, String> {
        Err(format!(
            "\\input {name} is not supported while expanding \\edef in expand-dump"
        ))
    }
}

fn install_dump_primitives(stores: &mut Stores) {
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let relax = stores.intern("relax");
    stores.set_meaning(relax, Meaning::Relax);
    stores.intern("par");

    for (name, primitive) in [
        ("expandafter", ExpandablePrimitive::ExpandAfter),
        ("noexpand", ExpandablePrimitive::NoExpand),
        ("csname", ExpandablePrimitive::CsName),
        ("endcsname", ExpandablePrimitive::EndCsName),
        ("string", ExpandablePrimitive::String),
        ("number", ExpandablePrimitive::Number),
        ("romannumeral", ExpandablePrimitive::RomanNumeral),
        ("meaning", ExpandablePrimitive::Meaning),
        ("the", ExpandablePrimitive::The),
        ("input", ExpandablePrimitive::Input),
        ("endinput", ExpandablePrimitive::EndInput),
        ("jobname", ExpandablePrimitive::JobName),
        ("fontname", ExpandablePrimitive::FontName),
        ("topmark", ExpandablePrimitive::TopMark),
        ("firstmark", ExpandablePrimitive::FirstMark),
        ("botmark", ExpandablePrimitive::BotMark),
        ("splitfirstmark", ExpandablePrimitive::SplitFirstMark),
        ("splitbotmark", ExpandablePrimitive::SplitBotMark),
        ("iftrue", ExpandablePrimitive::IfTrue),
        ("iffalse", ExpandablePrimitive::IfFalse),
        ("if", ExpandablePrimitive::If),
        ("ifcat", ExpandablePrimitive::IfCat),
        ("ifx", ExpandablePrimitive::IfX),
        ("ifnum", ExpandablePrimitive::IfNum),
        ("ifdim", ExpandablePrimitive::IfDim),
        ("ifodd", ExpandablePrimitive::IfOdd),
        ("ifcase", ExpandablePrimitive::IfCase),
        ("ifvmode", ExpandablePrimitive::IfVMode),
        ("ifhmode", ExpandablePrimitive::IfHMode),
        ("ifmmode", ExpandablePrimitive::IfMMode),
        ("ifinner", ExpandablePrimitive::IfInner),
        ("ifvoid", ExpandablePrimitive::IfVoid),
        ("ifhbox", ExpandablePrimitive::IfHBox),
        ("ifvbox", ExpandablePrimitive::IfVBox),
        ("ifeof", ExpandablePrimitive::IfEof),
        ("else", ExpandablePrimitive::Else),
        ("or", ExpandablePrimitive::Or),
        ("fi", ExpandablePrimitive::Fi),
    ] {
        let symbol = stores.intern(name);
        stores.set_meaning(symbol, Meaning::ExpandablePrimitive(primitive));
    }

    for name in [
        "def",
        "edef",
        "gdef",
        "xdef",
        "long",
        "outer",
        "global",
        "let",
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

fn is_space(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            cat: Catcode::Space,
            ..
        }
    )
}

fn is_other_equals(token: Token) -> bool {
    matches!(
        token,
        Token::Char {
            ch: '=',
            cat: Catcode::Other
        }
    )
}

fn catcode_from_i32(value: i32) -> Result<Catcode, ExpandDumpError> {
    match value {
        0 => Ok(Catcode::Escape),
        1 => Ok(Catcode::BeginGroup),
        2 => Ok(Catcode::EndGroup),
        3 => Ok(Catcode::MathShift),
        4 => Ok(Catcode::AlignmentTab),
        5 => Ok(Catcode::EndLine),
        6 => Ok(Catcode::Parameter),
        7 => Ok(Catcode::Superscript),
        8 => Ok(Catcode::Subscript),
        9 => Ok(Catcode::Ignored),
        10 => Ok(Catcode::Space),
        11 => Ok(Catcode::Letter),
        12 => Ok(Catcode::Other),
        13 => Ok(Catcode::Active),
        14 => Ok(Catcode::Comment),
        15 => Ok(Catcode::Invalid),
        _ => Err(ExpandDumpError::Definition(
            "\\catcode value must be in 0..=15",
        )),
    }
}

#[derive(Debug)]
pub enum ExpandDumpError {
    Io(io::Error),
    Lex(LexError),
    Expand(ExpandError),
    ScanToks(ScanToksError),
    ScanInt(scan_int::ScanIntError),
    Definition(&'static str),
}

impl std::fmt::Display for ExpandDumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Lex(err) => write!(f, "{err}"),
            Self::Expand(err) => write!(f, "{err}"),
            Self::ScanToks(err) => write!(f, "{err}"),
            Self::ScanInt(err) => write!(f, "{err}"),
            Self::Definition(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ExpandDumpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Lex(err) => Some(err),
            Self::Expand(err) => Some(err),
            Self::ScanToks(err) => Some(err),
            Self::ScanInt(err) => Some(err),
            Self::Definition(_) => None,
        }
    }
}

impl From<io::Error> for ExpandDumpError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
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

impl From<ScanToksError> for ExpandDumpError {
    fn from(value: ScanToksError) -> Self {
        Self::ScanToks(value)
    }
}

impl From<scan_int::ScanIntError> for ExpandDumpError {
    fn from(value: scan_int::ScanIntError) -> Self {
        Self::ScanInt(value)
    }
}

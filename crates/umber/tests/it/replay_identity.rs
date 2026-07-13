use std::env;

use proptest::prelude::*;
use proptest::test_runner::Config;
use tex_state::Universe;

const REPLAY_SHARDS: u32 = 8;

const PRELUDE: &str = concat!(
    r"\def\A{\count0=1} ",
    r"\gdef\B{\global\count0=2} ",
    r"\def\C{\advance\count0 by 1} ",
    r"\def\D{\dimen0=1pt} ",
    r"\edef\E{\count0=\the\count0} ",
    r"\xdef\X{\dimen0=\the\dimen0} ",
    r"\let\L=\A ",
    r"\countdef\countalias=300 ",
    r"\dimendef\dimenalias=301 ",
    r"\skipdef\skipalias=302 ",
    r"\muskipdef\muskipalias=303 ",
    r"\toksdef\toksalias=304 ",
    r"\chardef\charalias=65 ",
    r"\mathchardef\mathalias=123 ",
);

#[derive(Clone, Debug)]
enum Fragment {
    MacroDef {
        kind: DefKind,
        target: MacroTarget,
        body: MacroBody,
    },
    Let {
        target: MacroTarget,
        source: MacroTarget,
    },
    FutureLet,
    RegisterAssign {
        global: bool,
        register: RegisterKind,
        index: RegisterIndex,
        value: ValueSeed,
    },
    AliasAssign {
        global: bool,
        alias: AliasKind,
        value: ValueSeed,
    },
    ParameterAssign {
        global: bool,
        parameter: ParameterKind,
        value: ValueSeed,
    },
    CodeAssign {
        table: CodeTable,
        value: ValueSeed,
    },
    Arithmetic {
        op: ArithmeticOp,
        register: NumericRegister,
        index: RegisterIndex,
        value: ValueSeed,
    },
    AfterAssignment(MacroTarget),
    AfterGroup(MacroTarget),
    Conditional(ConditionalKind),
    MacroCall(MacroTarget),
    CaseChange(CaseKind),
    Diagnostic(DiagnosticKind),
    IgnoreSpaces,
    Relax,
    EnterGroup,
    LeaveGroup,
}

#[derive(Clone, Copy, Debug)]
enum DefKind {
    Def,
    Edef,
    Gdef,
    Xdef,
}

#[derive(Clone, Copy, Debug)]
enum MacroTarget {
    A,
    B,
    C,
    D,
    E,
    X,
    L,
}

#[derive(Clone, Copy, Debug)]
enum MacroBody {
    CountSet,
    GlobalCountSet,
    AdvanceCount,
    EdefCount,
    EdefDimen,
}

#[derive(Clone, Copy, Debug)]
enum RegisterKind {
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
}

#[derive(Clone, Copy, Debug)]
enum AliasKind {
    Count,
    Dimen,
    Skip,
    Muskip,
    Toks,
}

#[derive(Clone, Copy, Debug)]
enum NumericRegister {
    Count,
    Dimen,
    Skip,
    Muskip,
}

#[derive(Clone, Copy, Debug)]
enum RegisterIndex {
    Dense(u16),
    Sparse(u16),
    High(u16),
}

#[derive(Clone, Copy, Debug)]
enum ParameterKind {
    GlobalDefs,
    EndLineChar,
    Mag,
    ParIndent,
    ParSkip,
    EveryPar,
}

#[derive(Clone, Copy, Debug)]
enum CodeTable {
    Cat,
    Lc,
    Uc,
    Sf,
    Math,
    Del,
}

#[derive(Clone, Copy, Debug)]
enum ArithmeticOp {
    Advance,
    Multiply,
    Divide,
}

#[derive(Clone, Copy, Debug)]
enum ConditionalKind {
    True,
    False,
    Num,
    Dim,
    Odd,
    Case,
}

#[derive(Clone, Copy, Debug)]
enum CaseKind {
    Upper,
    Lower,
}

#[derive(Clone, Copy, Debug)]
enum DiagnosticKind {
    Message,
    ShowThe,
    ShowLists,
    ShowHyphens,
}

#[derive(Clone, Copy, Debug)]
struct ValueSeed(i32);

macro_rules! replay_identity_shard {
    ($name:ident, $shard:expr) => {
        proptest! {
            #![proptest_config(Config {
                cases: prop_cases_for_shard($shard),
                failure_persistence: None,
                ..Config::default()
            })]

            #[test]
            fn $name(program in program_strategy()) {
                assert_replay_identity(&program);
            }
        }
    };
}

replay_identity_shard!(replay_identity_through_real_primitives_0, 0);
replay_identity_shard!(replay_identity_through_real_primitives_1, 1);
replay_identity_shard!(replay_identity_through_real_primitives_2, 2);
replay_identity_shard!(replay_identity_through_real_primitives_3, 3);
replay_identity_shard!(replay_identity_through_real_primitives_4, 4);
replay_identity_shard!(replay_identity_through_real_primitives_5, 5);
replay_identity_shard!(replay_identity_through_real_primitives_6, 6);
replay_identity_shard!(replay_identity_through_real_primitives_7, 7);

#[test]
fn stale_epoch_global_compaction_regression_replays_cleanly() {
    assert_replay_identity(
        r"{\global\count11=1 \count11=2} \count11=3 {\count11=4} \count11=3 \end",
    );
}

fn assert_replay_identity(source: &str) {
    let mut stores = Universe::new();
    umber::prepare_run_stores(&mut stores);
    let before = stores.testing_state_hash();
    let checkpoint = stores.snapshot();

    let log = match umber::run_memory_with_stores(source, &mut stores) {
        Ok(log) => log,
        Err(err) => panic!("generated program failed: {err}\n{source}"),
    };
    verify_shadow(&stores);

    stores.rollback(&checkpoint);
    assert_eq!(
        stores.testing_state_hash(),
        before,
        "rollback hash diverged after program:\n{source}\nlog:\n{log}"
    );
    verify_shadow(&stores);
}

fn program_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(fragment_seed(), 1..36).prop_map(|fragments| {
        let mut program = String::from(PRELUDE);
        let mut depth = 0_u8;
        for fragment in fragments {
            match fragment {
                Fragment::EnterGroup => {
                    program.push_str("{ ");
                    depth = depth.saturating_add(1);
                }
                Fragment::LeaveGroup if depth > 0 => {
                    program.push_str("} ");
                    depth -= 1;
                }
                Fragment::LeaveGroup => {
                    program.push_str("{ ");
                    depth = depth.saturating_add(1);
                }
                Fragment::AfterGroup(target) if depth == 0 => {
                    program.push_str("{ ");
                    program.push_str(&render_aftergroup(target));
                    program.push_str("} ");
                }
                other => {
                    program.push_str(&render_fragment(other));
                    program.push(' ');
                }
            }
        }
        for _ in 0..depth {
            program.push_str("} ");
        }
        program.push_str("\\end");
        program
    })
}

fn fragment_seed() -> impl Strategy<Value = Fragment> {
    prop_oneof![
        4 => (def_kind(), macro_target(), macro_body()).prop_map(|(kind, target, body)| {
            Fragment::MacroDef { kind, target, body }
        }),
        2 => (macro_target(), macro_target()).prop_map(|(target, source)| {
            Fragment::Let { target, source }
        }),
        1 => Just(Fragment::FutureLet),
        9 => (any::<bool>(), register_kind(), register_index(), value_seed()).prop_map(
            |(global, register, index, value)| Fragment::RegisterAssign {
                global,
                register,
                index,
                value,
            },
        ),
        3 => (any::<bool>(), alias_kind(), value_seed()).prop_map(|(global, alias, value)| {
            Fragment::AliasAssign { global, alias, value }
        }),
        4 => (any::<bool>(), parameter_kind(), value_seed()).prop_map(
            |(global, parameter, value)| Fragment::ParameterAssign {
                global,
                parameter,
                value,
            },
        ),
        3 => (code_table(), value_seed()).prop_map(|(table, value)| {
            Fragment::CodeAssign { table, value }
        }),
        4 => (arithmetic_op(), numeric_register(), register_index(), value_seed()).prop_map(
            |(op, register, index, value)| Fragment::Arithmetic {
                op,
                register,
                index,
                value,
            },
        ),
        3 => macro_target().prop_map(Fragment::AfterAssignment),
        3 => macro_target().prop_map(Fragment::AfterGroup),
        4 => conditional_kind().prop_map(Fragment::Conditional),
        4 => macro_target().prop_map(Fragment::MacroCall),
        2 => case_kind().prop_map(Fragment::CaseChange),
        2 => diagnostic_kind().prop_map(Fragment::Diagnostic),
        1 => Just(Fragment::IgnoreSpaces),
        1 => Just(Fragment::Relax),
        3 => Just(Fragment::EnterGroup),
        3 => Just(Fragment::LeaveGroup),
    ]
}

fn render_fragment(fragment: Fragment) -> String {
    match fragment {
        Fragment::MacroDef { kind, target, body } => {
            format!(r"\{}\{}{{{}}}", kind.name(), target.name(), body.render())
        }
        Fragment::Let { target, source } => format!(r"\let\{}=\{}", target.name(), source.name()),
        Fragment::FutureLet => r"\futurelet\F\relax".to_owned(),
        Fragment::RegisterAssign {
            global,
            register,
            index,
            value,
        } => format!(
            r"{}\{}{}={}",
            global_prefix(global),
            register.name(),
            index.raw(),
            render_register_value(register, value),
        ),
        Fragment::AliasAssign {
            global,
            alias,
            value,
        } => format!(
            r"{}\{}={}",
            global_prefix(global),
            alias.name(),
            render_alias_value(alias, value),
        ),
        Fragment::ParameterAssign {
            global,
            parameter,
            value,
        } => format!(
            r"{}\{}={}",
            global_prefix(global),
            parameter.name(),
            render_parameter_value(parameter, value),
        ),
        Fragment::CodeAssign { table, value } => {
            format!(r"\{}`@={}", table.name(), render_code_value(table, value))
        }
        Fragment::Arithmetic {
            op,
            register,
            index,
            value,
        } => format!(
            r"\{} \{}{} by {}",
            op.name(),
            register.name(),
            index.raw(),
            render_arithmetic_value(op, register, value),
        ),
        Fragment::AfterAssignment(target) => {
            format!(r"\afterassignment\{}\count{}=7", target.name(), 1)
        }
        Fragment::AfterGroup(target) => render_aftergroup(target),
        Fragment::Conditional(kind) => kind.render(),
        Fragment::MacroCall(target) => format!(r"\{}", target.name()),
        Fragment::CaseChange(kind) => match kind {
            CaseKind::Upper => r"\uppercase{\count2=3}".to_owned(),
            CaseKind::Lower => r"\lowercase{\count2=3}".to_owned(),
        },
        Fragment::Diagnostic(kind) => kind.render(),
        Fragment::IgnoreSpaces => r"\ignorespaces   \relax".to_owned(),
        Fragment::Relax => r"\relax".to_owned(),
        Fragment::EnterGroup | Fragment::LeaveGroup => String::new(),
    }
}

fn render_aftergroup(target: MacroTarget) -> String {
    format!(r"\aftergroup\{}", target.name())
}

impl DefKind {
    fn name(self) -> &'static str {
        match self {
            Self::Def => "def",
            Self::Edef => "edef",
            Self::Gdef => "gdef",
            Self::Xdef => "xdef",
        }
    }
}

impl MacroTarget {
    fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::X => "X",
            Self::L => "L",
        }
    }
}

impl MacroBody {
    fn render(self) -> &'static str {
        match self {
            Self::CountSet => r"\count0=1",
            Self::GlobalCountSet => r"\global\count0=2",
            Self::AdvanceCount => r"\advance\count0 by 1",
            Self::EdefCount => r"\count0=\the\count0",
            Self::EdefDimen => r"\dimen0=\the\dimen0",
        }
    }
}

impl RegisterKind {
    fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Dimen => "dimen",
            Self::Skip => "skip",
            Self::Muskip => "muskip",
            Self::Toks => "toks",
        }
    }
}

impl AliasKind {
    fn name(self) -> &'static str {
        match self {
            Self::Count => "countalias",
            Self::Dimen => "dimenalias",
            Self::Skip => "skipalias",
            Self::Muskip => "muskipalias",
            Self::Toks => "toksalias",
        }
    }
}

impl NumericRegister {
    fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Dimen => "dimen",
            Self::Skip => "skip",
            Self::Muskip => "muskip",
        }
    }
}

impl RegisterIndex {
    fn raw(self) -> u16 {
        match self {
            Self::Dense(value) | Self::Sparse(value) | Self::High(value) => value,
        }
    }
}

impl ParameterKind {
    fn name(self) -> &'static str {
        match self {
            Self::GlobalDefs => "globaldefs",
            Self::EndLineChar => "endlinechar",
            Self::Mag => "mag",
            Self::ParIndent => "parindent",
            Self::ParSkip => "parskip",
            Self::EveryPar => "everypar",
        }
    }
}

impl CodeTable {
    fn name(self) -> &'static str {
        match self {
            Self::Cat => "catcode",
            Self::Lc => "lccode",
            Self::Uc => "uccode",
            Self::Sf => "sfcode",
            Self::Math => "mathcode",
            Self::Del => "delcode",
        }
    }
}

impl ArithmeticOp {
    fn name(self) -> &'static str {
        match self {
            Self::Advance => "advance",
            Self::Multiply => "multiply",
            Self::Divide => "divide",
        }
    }
}

impl ConditionalKind {
    fn render(self) -> String {
        match self {
            Self::True => r"\iftrue \count4=1\else \count4=2\fi".to_owned(),
            Self::False => r"\iffalse \count4=1\else \count4=2\fi".to_owned(),
            Self::Num => r"\ifnum\count0<5 \count5=1\else \count5=2\fi".to_owned(),
            Self::Dim => r"\ifdim\dimen0<2pt \dimen5=1pt\else \dimen5=2pt\fi".to_owned(),
            Self::Odd => r"\ifodd\count0 \count6=1\else \count6=2\fi".to_owned(),
            Self::Case => r"\ifcase1 \count7=0\or \count7=1\else \count7=2\fi".to_owned(),
        }
    }
}

impl DiagnosticKind {
    fn render(self) -> String {
        match self {
            Self::Message => r"\message{m:\the\count0}".to_owned(),
            Self::ShowThe => r"\showthe\count0".to_owned(),
            Self::ShowLists => r"\showlists".to_owned(),
            Self::ShowHyphens => r"\patterns{a1ba}\showhyphens{aba}".to_owned(),
        }
    }
}

fn render_register_value(register: RegisterKind, value: ValueSeed) -> String {
    match register {
        RegisterKind::Count => int_value(value).to_string(),
        RegisterKind::Dimen => dimen_value(value),
        RegisterKind::Skip => glue_value(value, "pt"),
        RegisterKind::Muskip => glue_value(value, "mu"),
        RegisterKind::Toks => r"{\A\the\count0}".to_owned(),
    }
}

fn render_alias_value(alias: AliasKind, value: ValueSeed) -> String {
    match alias {
        AliasKind::Count => int_value(value).to_string(),
        AliasKind::Dimen => dimen_value(value),
        AliasKind::Skip => glue_value(value, "pt"),
        AliasKind::Muskip => glue_value(value, "mu"),
        AliasKind::Toks => r"{\B\the\dimen0}".to_owned(),
    }
}

fn render_parameter_value(parameter: ParameterKind, value: ValueSeed) -> String {
    match parameter {
        ParameterKind::GlobalDefs => match value.0.rem_euclid(3) {
            0 => "-1".to_owned(),
            1 => "0".to_owned(),
            _ => "1".to_owned(),
        },
        ParameterKind::EndLineChar => match value.0.rem_euclid(3) {
            0 => "-1".to_owned(),
            1 => "13".to_owned(),
            _ => "32".to_owned(),
        },
        ParameterKind::Mag => "1000".to_owned(),
        ParameterKind::ParIndent => dimen_value(value),
        ParameterKind::ParSkip => glue_value(value, "pt"),
        ParameterKind::EveryPar => r"{\C}".to_owned(),
    }
}

fn render_numeric_value(register: NumericRegister, value: ValueSeed) -> String {
    match register {
        NumericRegister::Count => {
            let value = match value.0.rem_euclid(5) {
                0 => 1,
                1 => 2,
                2 => -1,
                3 => 0,
                _ => 3,
            };
            value.to_string()
        }
        NumericRegister::Dimen => dimen_value(value),
        NumericRegister::Skip => glue_value(value, "pt"),
        NumericRegister::Muskip => glue_value(value, "mu"),
    }
}

fn render_arithmetic_value(
    op: ArithmeticOp,
    register: NumericRegister,
    value: ValueSeed,
) -> String {
    match op {
        ArithmeticOp::Advance => render_numeric_value(register, value),
        ArithmeticOp::Multiply | ArithmeticOp::Divide => {
            let value = match value.0.rem_euclid(4) {
                0 => 1,
                1 => 2,
                2 => -1,
                _ => 3,
            };
            value.to_string()
        }
    }
}

fn render_code_value(table: CodeTable, value: ValueSeed) -> String {
    match table {
        CodeTable::Cat => match value.0.rem_euclid(3) {
            0 => "11".to_owned(),
            1 => "12".to_owned(),
            _ => "12".to_owned(),
        },
        CodeTable::Lc | CodeTable::Uc => char_code(value).to_string(),
        CodeTable::Sf => (1000 + value.0.rem_euclid(100)).to_string(),
        CodeTable::Math => value.0.rem_euclid(32_768).to_string(),
        CodeTable::Del => value.0.rem_euclid(16_777_216).to_string(),
    }
}

fn dimen_value(value: ValueSeed) -> String {
    match value.0.rem_euclid(5) {
        0 => "0pt".to_owned(),
        1 => "1pt".to_owned(),
        2 => "-2pt".to_owned(),
        3 => ".5pt".to_owned(),
        _ => "65536sp".to_owned(),
    }
}

fn glue_value(value: ValueSeed, unit: &str) -> String {
    match value.0.rem_euclid(4) {
        0 => format!("0{unit}"),
        1 => format!("1{unit} plus 2{unit} minus 1{unit}"),
        2 => format!("2{unit} plus 1fil"),
        _ => format!("3{unit} minus 1fill"),
    }
}

fn int_value(value: ValueSeed) -> i32 {
    match value.0.rem_euclid(7) {
        0 => 0,
        1 => 1,
        2 => -1,
        3 => 17,
        4 => 255,
        5 => 256,
        _ => 1024,
    }
}

fn char_code(value: ValueSeed) -> i32 {
    i32::from(b'a') + value.0.rem_euclid(26)
}

fn global_prefix(global: bool) -> &'static str {
    if global { r"\global" } else { "" }
}

fn def_kind() -> impl Strategy<Value = DefKind> {
    prop_oneof![
        Just(DefKind::Def),
        Just(DefKind::Edef),
        Just(DefKind::Gdef),
        Just(DefKind::Xdef),
    ]
}

fn macro_target() -> impl Strategy<Value = MacroTarget> {
    prop_oneof![
        Just(MacroTarget::A),
        Just(MacroTarget::B),
        Just(MacroTarget::C),
        Just(MacroTarget::D),
        Just(MacroTarget::E),
        Just(MacroTarget::X),
        Just(MacroTarget::L),
    ]
}

fn macro_body() -> impl Strategy<Value = MacroBody> {
    prop_oneof![
        Just(MacroBody::CountSet),
        Just(MacroBody::GlobalCountSet),
        Just(MacroBody::AdvanceCount),
        Just(MacroBody::EdefCount),
        Just(MacroBody::EdefDimen),
    ]
}

fn register_kind() -> impl Strategy<Value = RegisterKind> {
    prop_oneof![
        Just(RegisterKind::Count),
        Just(RegisterKind::Dimen),
        Just(RegisterKind::Skip),
        Just(RegisterKind::Muskip),
        Just(RegisterKind::Toks),
    ]
}

fn alias_kind() -> impl Strategy<Value = AliasKind> {
    prop_oneof![
        Just(AliasKind::Count),
        Just(AliasKind::Dimen),
        Just(AliasKind::Skip),
        Just(AliasKind::Muskip),
        Just(AliasKind::Toks),
    ]
}

fn numeric_register() -> impl Strategy<Value = NumericRegister> {
    prop_oneof![
        Just(NumericRegister::Count),
        Just(NumericRegister::Dimen),
        Just(NumericRegister::Skip),
        Just(NumericRegister::Muskip),
    ]
}

fn register_index() -> impl Strategy<Value = RegisterIndex> {
    prop_oneof![
        3 => (0_u16..16).prop_map(RegisterIndex::Dense),
        2 => (256_u16..320).prop_map(RegisterIndex::Sparse),
        1 => (32_704_u16..32_768).prop_map(RegisterIndex::High),
    ]
}

fn parameter_kind() -> impl Strategy<Value = ParameterKind> {
    prop_oneof![
        Just(ParameterKind::GlobalDefs),
        Just(ParameterKind::EndLineChar),
        Just(ParameterKind::Mag),
        Just(ParameterKind::ParIndent),
        Just(ParameterKind::ParSkip),
        Just(ParameterKind::EveryPar),
    ]
}

fn code_table() -> impl Strategy<Value = CodeTable> {
    prop_oneof![
        Just(CodeTable::Cat),
        Just(CodeTable::Lc),
        Just(CodeTable::Uc),
        Just(CodeTable::Sf),
        Just(CodeTable::Math),
        Just(CodeTable::Del),
    ]
}

fn arithmetic_op() -> impl Strategy<Value = ArithmeticOp> {
    prop_oneof![
        Just(ArithmeticOp::Advance),
        Just(ArithmeticOp::Multiply),
        Just(ArithmeticOp::Divide),
    ]
}

fn conditional_kind() -> impl Strategy<Value = ConditionalKind> {
    prop_oneof![
        Just(ConditionalKind::True),
        Just(ConditionalKind::False),
        Just(ConditionalKind::Num),
        Just(ConditionalKind::Dim),
        Just(ConditionalKind::Odd),
        Just(ConditionalKind::Case),
    ]
}

fn case_kind() -> impl Strategy<Value = CaseKind> {
    prop_oneof![Just(CaseKind::Upper), Just(CaseKind::Lower)]
}

fn diagnostic_kind() -> impl Strategy<Value = DiagnosticKind> {
    prop_oneof![
        Just(DiagnosticKind::Message),
        Just(DiagnosticKind::ShowThe),
        Just(DiagnosticKind::ShowLists),
        Just(DiagnosticKind::ShowHyphens),
    ]
}

fn value_seed() -> impl Strategy<Value = ValueSeed> {
    (-4_i32..12).prop_map(ValueSeed)
}

#[allow(clippy::disallowed_methods)]
fn prop_cases() -> u32 {
    env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(64)
}

fn prop_cases_for_shard(shard: u32) -> u32 {
    let cases = prop_cases();
    let base = cases / REPLAY_SHARDS;
    let remainder = cases % REPLAY_SHARDS;
    base + u32::from(shard < remainder)
}

#[cfg(feature = "shadow")]
fn verify_shadow(stores: &Universe) {
    stores.verify_shadow();
}

#[cfg(not(feature = "shadow"))]
fn verify_shadow(_: &Universe) {}

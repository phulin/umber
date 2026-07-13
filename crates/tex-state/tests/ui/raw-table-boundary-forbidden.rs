use tex_state::code_tables::CodeTables;
use tex_state::interner::Interner;
use tex_state::token::Catcode;

fn main() {
    let mut interner = Interner::new();
    let _symbol = interner.intern("rogue");

    let mut tables = CodeTables::new();
    tables.set_catcode('@', Catcode::Letter);
    tables.set_lccode('@', u32::from('a'));
    tables.set_uccode('@', u32::from('A'));
    tables.set_sfcode('@', 1000);
    tables.set_mathcode('@', u32::from('@'));
    tables.set_delcode('@', -1);
}

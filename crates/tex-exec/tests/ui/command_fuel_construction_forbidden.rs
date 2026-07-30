use tex_command::CommandFuel;

type FuelAlias = CommandFuel;

fn main() {
    let _ = CommandFuel::new(1);
    let _ = <CommandFuel as Default>::default();
    let _: FuelAlias = Default::default();
    let _ = FuelAlias::default();
}

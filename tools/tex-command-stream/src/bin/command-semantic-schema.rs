//! Prints the structural schema generated from the command-semantic V2 type.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&tex_command_stream::semantic::manifest_schema())?
    );
    Ok(())
}

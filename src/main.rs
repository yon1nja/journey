use anyhow::Result;
use clap::Parser;

use journey::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let output = journey::run(cli)?;
    if !output.is_empty() {
        println!("{output}");
    }
    Ok(())
}

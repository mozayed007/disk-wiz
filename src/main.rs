use clap::Parser;
use disk_wiz::cli::Cli;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match disk_wiz::run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dw: {e:#}");
            ExitCode::from(1)
        }
    }
}

//! The `sa-forge-signer-serve` command line: the long-running signer and nothing else.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::config::{self, Config};
use crate::{serve, trace};

/// Run the long-running signer: HTTP at /v1/check, /v1/sign and /v1/halt, MCP at /mcp (see [serve] in the config)
#[derive(Parser)]
#[command(name = "sa-forge-signer-serve", version, long_about = None)]
struct Cli {
    /// Config file [default: `$SA_FORGE_SIGNER_CONFIG`, else `~/.config/sa-forge-signer/config.toml`]
    #[arg(long)]
    config: Option<PathBuf>,
}

/// Serves until SIGTERM or SIGINT and returns the exit code.
#[must_use]
pub fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    trace::init()?;
    let path = match cli.config {
        Some(p) => p,
        None => config::default_path()?,
    };
    serve::run(Config::load(&path, true)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["sa-forge-signer-serve"]).is_ok());
        assert!(Cli::try_parse_from(["sa-forge-signer-serve", "--config", "x.toml"]).is_ok());
        assert!(Cli::try_parse_from(["sa-forge-signer-serve", "serve"]).is_err());
    }
}

//! `sa-forge-signer-mcp`: MCP on standard input and output, for a running `sa-forge-signer-serve`.

fn main() -> std::process::ExitCode {
    sa_forge_signer::mcp_cli::main()
}

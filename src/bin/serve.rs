//! `sa-forge-signer-serve`: the long-running signer over HTTP and MCP.

fn main() -> std::process::ExitCode {
    sa_forge_signer::serve_cli::main()
}

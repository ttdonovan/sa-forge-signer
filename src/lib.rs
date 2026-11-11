//! sa-forge-signer: a thin, self-hosted signer for agents playing SAGE C4 on Z.ink. Unofficial.
//!
//! This is not a public API: it exists so the binaries share one implementation. The interface
//! callers may depend on is the command line, the routes and the file formats (`CONTRIBUTING.md`).

mod audit;
#[cfg(test)]
mod bench;
mod checks;
pub mod cli;
mod cluster;
mod config;
mod engine;
mod facts;
mod halt;
mod instruction_table;
mod keystore;
pub mod mcp_cli;
mod payload;
mod policy;
mod policy_approval;
mod rpc;
mod serve;
pub mod serve_cli;
#[cfg(test)]
mod testrpc;
mod text;
mod trace;
mod transfer;
mod url;

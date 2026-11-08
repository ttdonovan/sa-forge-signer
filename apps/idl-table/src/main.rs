//! sa-forge-signer-idl-table: writes the signer's instruction table from Codama IDL files.
//! It never copies an IDL: the table holds only per-instruction facts and each IDL's sha256.

mod table;

use std::io::Write;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};

const USAGE: &str = "usage: sa-forge-signer-idl-table --cluster <name> --from <where the IDLs came from> [--out <file>] <idl.json>...";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let (mut cluster, mut from, mut out, mut files) = (None, None, None, Vec::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cluster" => cluster = it.next(),
            "--from" => from = it.next(),
            "--out" => out = it.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            f if f.starts_with('-') => bail!("unknown flag {f}"),
            _ => files.push(a),
        }
    }
    let cluster = cluster.context("--cluster is required")?;
    let from = from.context("--from is required, for example \"sage-forge <commit>\"")?;
    if files.is_empty() {
        bail!("no IDL files given");
    }
    let idls = files
        .iter()
        .map(|f| {
            std::fs::read_to_string(f)
                .with_context(|| format!("reading {f}"))
                .map(|t| (t, f.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    let table = table::build(&cluster, &from, &idls)?;
    let mut text = serde_json::to_string_pretty(&table)?;
    text.push('\n');
    match out {
        Some(path) => std::fs::write(&path, text).with_context(|| format!("writing {path}"))?,
        None => std::io::stdout().write_all(text.as_bytes())?,
    }
    eprintln!(
        "{} instructions from {} programs",
        table.instructions.len(),
        table.programs.len()
    );
    Ok(())
}

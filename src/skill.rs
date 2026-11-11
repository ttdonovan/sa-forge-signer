//! The skill that teaches an agent to use the signer's MCP tools: its text, the resources the
//! service serves from it, and installing it.
//!
//! The service serves the same files `sa-forge-signer-mcp skill install` writes, so the two
//! cannot differ.

use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const SKILL: &str = include_str!("../skills/sa-forge-signer/SKILL.md");
const TOOLS: &str = include_str!("../skills/sa-forge-signer/references/tools.md");
const NAME: &str = "sa-forge-signer";
/// A file holding this was written by this binary, so installing may replace it.
const MARK: &str = "installed by sa-forge-signer-mcp";

pub const INDEX_URI: &str = "sa-forge-signer://llms.txt";
pub const FULL_URI: &str = "sa-forge-signer://llms-full.txt";
/// The skill and its supporting file, at the URIs the MCP Skills extension lays out.
pub const SKILL_URI: &str = "skill://sa-forge-signer/SKILL.md";
pub const TOOLS_URI: &str = "skill://sa-forge-signer/references/tools.md";

/// A resource the service serves: its URI, name, MIME type and a line on what it is.
pub const RESOURCES: [(&str, &str, &str, &str); 4] = [
    (
        INDEX_URI,
        "llms.txt",
        "text/plain",
        "Start here: what the signer is, and a line on each tool and resource.",
    ),
    (
        FULL_URI,
        "llms-full.txt",
        "text/plain",
        "The guide and the tool reference in one file.",
    ),
    (
        SKILL_URI,
        "SKILL.md",
        "text/markdown",
        "The guide: sending an action, acting on each outcome, reconciling an unknown one, and when to halt.",
    ),
    (
        TOOLS_URI,
        "tools.md",
        "text/markdown",
        "Each tool with its arguments, and every field of the report.",
    ),
];

/// A tool as the index describes it: name and what it does.
const TOOL_LINES: [(&str, &str); 4] = [
    (
        "check",
        "Run every check and a simulation on a payload. Never signs.",
    ),
    (
        "sign",
        "Check, simulate, sign, send and confirm a payload. Returns the JSON report.",
    ),
    (
        "key_show",
        "A key's pubkey, class, approval, balance, whether it is halted, and its policy digest.",
    ),
    (
        "halt",
        "Stop all signing with a key until a person resumes it. There is no resume tool.",
    ),
];

/// The whole skill file.
pub const fn text() -> &'static str {
    SKILL
}

/// The text of the resource at `uri`, or `None` for a URI the service does not serve.
pub fn resource(uri: &str) -> Option<String> {
    match uri {
        INDEX_URI => Some(index()),
        FULL_URI => Some(full()),
        SKILL_URI => Some(SKILL.to_owned()),
        TOOLS_URI => Some(TOOLS.to_owned()),
        _ => None,
    }
}

/// `llms.txt`: what the signer is, then a line for everything the service serves.
fn index() -> String {
    let mut out = String::from(
        "# sa-forge-signer\n\n> Signs SAGE C4 payloads built by forge-mcp, for agents that must never hold a key. Pass a payload unchanged to `check`, then `sign`, and act on the report's `outcome`.\n\n## Resources\n\n",
    );
    for (uri, name, _, about) in RESOURCES {
        let _ = writeln!(out, "- [{name}]({uri}): {about}");
    }
    out.push_str("\n## Tools\n\n");
    for (name, about) in TOOL_LINES {
        let _ = writeln!(out, "- `{name}`: {about}");
    }
    out
}

/// `llms-full.txt`: the guide, then the tool reference.
fn full() -> String {
    format!("{}\n{TOOLS}", body())
}

/// The skill without its frontmatter, for the one-file form.
fn body() -> String {
    let mut lines = SKILL.lines();
    if lines.next() != Some("---") {
        return SKILL.to_owned();
    }
    for line in lines.by_ref() {
        if line == "---" {
            break;
        }
    }
    let body: Vec<&str> = lines.collect();
    let mut out = body.join("\n").trim_start_matches('\n').to_owned();
    out.push('\n');
    out
}

/// What installing did.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    Created(PathBuf),
    Replaced(PathBuf),
    Unchanged(PathBuf),
}

/// The skills folder of a project, or of the user when no project is given.
pub fn skills_dir(project: Option<&Path>, home: Option<&Path>) -> Result<PathBuf> {
    let base = match project {
        Some(dir) => dir,
        None => home.context("no home folder found; pass --project <dir>")?,
    };
    Ok(base.join(".claude").join("skills"))
}

/// Writes the skill as `<skills>/sa-forge-signer/SKILL.md`, with its tool reference.
///
/// A file of that name this binary did not write is left alone, and that is an error.
pub fn install(skills: &Path) -> Result<Installed> {
    let dir = skills.join(NAME);
    let path = dir.join("SKILL.md");
    let existing = match fs::read_to_string(&path) {
        Ok(existing) => Some(existing),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let outcome = match existing {
        Some(existing) if existing == SKILL && reference_is_current(&dir) => {
            return Ok(Installed::Unchanged(path));
        }
        Some(existing) if existing == SKILL || existing.contains(MARK) => {
            Installed::Replaced(path.clone())
        }
        Some(_) => bail!(
            "{} exists and was not written by sa-forge-signer-mcp; left alone",
            path.display()
        ),
        None => Installed::Created(path.clone()),
    };
    let references = dir.join("references");
    fs::create_dir_all(&references)
        .with_context(|| format!("creating {}", references.display()))?;
    fs::write(&path, SKILL).with_context(|| format!("writing {}", path.display()))?;
    let reference = references.join("tools.md");
    fs::write(&reference, TOOLS).with_context(|| format!("writing {}", reference.display()))?;
    Ok(outcome)
}

/// Whether the installed supporting file is the one this binary carries.
fn reference_is_current(dir: &Path) -> bool {
    fs::read_to_string(dir.join("references").join("tools.md")).is_ok_and(|text| text == TOOLS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_is_the_skill_without_its_frontmatter() {
        assert!(text().starts_with("---\nname: sa-forge-signer\n"));
        assert!(text().contains(MARK));
        let guide = body();
        assert!(guide.starts_with("# Using sa-forge-signer\n"));
        assert!(!guide.contains("\nname: sa-forge-signer"));
        assert!(text().ends_with(&guide));
    }

    #[test]
    fn every_listed_resource_can_be_read_and_nothing_else() {
        for (uri, _, _, _) in RESOURCES {
            assert!(resource(uri).is_some_and(|t| !t.is_empty()), "{uri}");
        }
        assert!(resource("sa-forge-signer://config.toml").is_none());
        assert!(resource("llms.txt").is_none());
        let index = resource(INDEX_URI).unwrap();
        for (uri, _, _, _) in RESOURCES {
            assert!(index.contains(uri), "{uri}");
        }
        let full = resource(FULL_URI).unwrap();
        assert!(full.starts_with("# Using sa-forge-signer\n"));
        assert!(full.ends_with(TOOLS));
    }

    #[test]
    fn the_index_and_the_reference_name_the_tools_the_service_lists() {
        let index = resource(INDEX_URI).unwrap();
        for (name, _) in TOOL_LINES {
            assert!(index.contains(&format!("- `{name}`: ")), "{name}");
            assert!(TOOLS.contains(&format!("- `{name}(")), "{name}");
        }
    }

    #[test]
    fn installing_creates_then_keeps_then_replaces_only_its_own_file() {
        let skills =
            std::env::temp_dir().join(format!("sa-forge-signer-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&skills);
        let path = skills.join(NAME).join("SKILL.md");
        assert_eq!(install(&skills).unwrap(), Installed::Created(path.clone()));
        assert_eq!(
            install(&skills).unwrap(),
            Installed::Unchanged(path.clone())
        );
        let reference = skills.join(NAME).join("references/tools.md");
        assert_eq!(fs::read_to_string(&reference).unwrap(), TOOLS);
        fs::write(&reference, "stale").unwrap();
        assert_eq!(install(&skills).unwrap(), Installed::Replaced(path.clone()));
        assert_eq!(fs::read_to_string(&reference).unwrap(), TOOLS);
        fs::write(&path, format!("older text, {MARK}")).unwrap();
        assert_eq!(install(&skills).unwrap(), Installed::Replaced(path.clone()));
        assert_eq!(fs::read_to_string(&path).unwrap(), SKILL);
        fs::write(&path, "someone else's skill").unwrap();
        assert!(install(&skills).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "someone else's skill");
        fs::remove_dir_all(&skills).unwrap();
    }

    #[test]
    fn the_skills_folder_is_the_projects_or_the_users() {
        let project = Path::new("/work/repo");
        let home = Path::new("/home/me");
        assert_eq!(
            skills_dir(Some(project), Some(home)).unwrap(),
            Path::new("/work/repo/.claude/skills")
        );
        assert_eq!(
            skills_dir(None, Some(home)).unwrap(),
            Path::new("/home/me/.claude/skills")
        );
        assert!(skills_dir(None, None).is_err());
    }
}

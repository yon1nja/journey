//! Agent skills that teach coding agents to use Journey, embedded in the binary.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::storage;

/// Skill name and SKILL.md content. Every skill name starts with `journey-`.
const SKILLS: &[(&str, &str)] = &[
    (
        "journey-start",
        include_str!("skills/journey-start/SKILL.md"),
    ),
    (
        "journey-side-quest",
        include_str!("skills/journey-side-quest/SKILL.md"),
    ),
    (
        "journey-resume",
        include_str!("skills/journey-resume/SKILL.md"),
    ),
];

/// Claude Code's global skills folder, honoring `CLAUDE_CONFIG_DIR`.
fn default_dir() -> Result<PathBuf> {
    if let Some(config) = env::var_os("CLAUDE_CONFIG_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(config).join("skills"));
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))?;
    Ok(home.join(".claude").join("skills"))
}

fn skills_dir(dir: Option<&Path>) -> Result<PathBuf> {
    match dir {
        Some(dir) => Ok(dir.to_path_buf()),
        None => default_dir(),
    }
}

pub fn install(dir: Option<&Path>) -> Result<String> {
    let dir = skills_dir(dir)?;
    let mut out = Vec::new();
    for (name, content) in SKILLS {
        let path = dir.join(name).join("SKILL.md");
        let state = match fs::read_to_string(&path) {
            Ok(existing) if existing == *content => "up to date",
            Ok(_) => "updated",
            Err(_) => "installed",
        };
        if state != "up to date" {
            storage::write_string_atomic(&path, content)
                .with_context(|| format!("failed to install skill {}", path.display()))?;
        }
        out.push(format!("{state}: {}", path.display()));
    }
    Ok(out.join("\n"))
}

pub fn remove(dir: Option<&Path>) -> Result<String> {
    let dir = skills_dir(dir)?;
    let mut out = Vec::new();
    for (name, _) in SKILLS {
        let skill_dir = dir.join(name);
        let path = skill_dir.join("SKILL.md");
        let Ok(existing) = fs::read_to_string(&path) else {
            continue;
        };
        // Only remove folders that still hold a Journey skill of the same name.
        if !existing.starts_with(&format!("---\nname: {name}\n")) {
            out.push(format!(
                "skipped (not a Journey skill): {}",
                skill_dir.display()
            ));
            continue;
        }
        fs::remove_dir_all(&skill_dir)
            .with_context(|| format!("failed to remove {}", skill_dir.display()))?;
        out.push(format!("removed: {}", skill_dir.display()));
    }
    if out.is_empty() {
        return Ok(format!("no Journey skills installed in {}", dir.display()));
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_have_matching_frontmatter() {
        for (name, content) in SKILLS {
            assert!(name.starts_with("journey-"));
            assert!(content.starts_with(&format!("---\nname: {name}\ndescription: ")));
            assert!(content.contains("journey guide"));
        }
    }
}

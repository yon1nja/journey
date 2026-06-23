use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone)]
pub struct DiscoveredRepo {
    pub root: PathBuf,
    pub branch: String,
}

pub fn discover_repo(path: &Path) -> Result<DiscoveredRepo> {
    let root = run_git(path, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root.trim());
    let branch = current_branch(&root)?;
    Ok(DiscoveredRepo { root, branch })
}

pub fn create_worktree(
    root: &Path,
    worktree: &Path,
    branch: &str,
    create_branch: bool,
) -> Result<()> {
    let worktree_arg = worktree.to_str().ok_or_else(|| {
        anyhow::anyhow!("worktree path is not valid UTF-8: {}", worktree.display())
    })?;
    if create_branch {
        run_git(root, &["worktree", "add", "-b", branch, worktree_arg]).map(|_| ())
    } else {
        run_git(root, &["worktree", "add", worktree_arg, branch]).map(|_| ())
    }
}

pub fn run_git(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", path.display()))?;

    if output.status.success() {
        String::from_utf8(output.stdout).context("git stdout was not UTF-8")
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git -C {} {} failed: {}",
            path.display(),
            args.join(" "),
            stderr.trim()
        );
    }
}

fn current_branch(worktree: &Path) -> Result<String> {
    match run_git(worktree, &["symbolic-ref", "--short", "HEAD"]) {
        Ok(branch) => Ok(branch.trim().to_string()),
        Err(_) => Ok("HEAD".to_string()),
    }
}

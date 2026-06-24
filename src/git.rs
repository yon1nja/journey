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

pub fn list_branches(root: &Path) -> Result<Vec<String>> {
    let output = run_git(root, &["branch", "--format=%(refname:short)"])?;
    let mut branches = output
        .lines()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    branches.sort();
    branches.dedup();
    if branches.is_empty() {
        bail!("no local branches found");
    }
    Ok(branches)
}

pub fn worktree_for_branch(root: &Path, branch: &str) -> Result<Option<PathBuf>> {
    let output = run_git(root, &["worktree", "list", "--porcelain"])?;
    let mut current_worktree = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current_worktree = Some(PathBuf::from(path));
        } else if let Some(name) = line.strip_prefix("branch ") {
            let name = name.strip_prefix("refs/heads/").unwrap_or(name);
            if name == branch {
                return Ok(current_worktree);
            }
        } else if line.is_empty() {
            current_worktree = None;
        }
    }
    Ok(None)
}

pub fn ensure_worktree_removable(root: &Path, worktree: &Path) -> Result<()> {
    let canonical_worktree = worktree.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize worktree {} before removal",
            worktree.display()
        )
    })?;
    if is_main_worktree(root, &canonical_worktree)? {
        bail!(
            "{} is the main working tree; Journey will not delete it",
            canonical_worktree.display()
        );
    }

    let status = run_git(&canonical_worktree, &["status", "--porcelain"])?;
    if !status.trim().is_empty() {
        bail!(
            "{} has uncommitted changes; commit, stash, or remove them before deleting the worktree",
            canonical_worktree.display()
        );
    }
    Ok(())
}

pub fn remove_worktree(root: &Path, worktree: &Path) -> Result<()> {
    ensure_worktree_removable(root, worktree)?;
    let worktree_arg = worktree.to_str().ok_or_else(|| {
        anyhow::anyhow!("worktree path is not valid UTF-8: {}", worktree.display())
    })?;
    run_git(root, &["worktree", "remove", worktree_arg]).map(|_| ())
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

fn is_main_worktree(root: &Path, worktree: &Path) -> Result<bool> {
    let output = run_git(root, &["worktree", "list", "--porcelain"])?;
    let mut worktree_paths = output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from);
    let Some(main_worktree) = worktree_paths.next() else {
        bail!("git worktree list did not report any worktrees");
    };
    Ok(main_worktree
        .canonicalize()
        .map(|path| path == worktree)
        .unwrap_or(false))
}

fn current_branch(worktree: &Path) -> Result<String> {
    match run_git(worktree, &["symbolic-ref", "--short", "HEAD"]) {
        Ok(branch) => Ok(branch.trim().to_string()),
        Err(_) => Ok("HEAD".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn git_command(repo: &Path, args: &[&str]) {
        run_git(repo, args).unwrap();
    }

    fn init_repo(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        git_command(root, &["init"]);
        git_command(root, &["config", "user.email", "journey@example.com"]);
        git_command(root, &["config", "user.name", "Journey Test"]);
        std::fs::write(root.join("README.md"), "initial\n").unwrap();
        git_command(root, &["add", "README.md"]);
        git_command(root, &["commit", "-m", "initial"]);
    }

    #[test]
    fn finds_existing_worktree_for_branch() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("feature-worktree");
        init_repo(&repo);
        git_command(&repo, &["branch", "feature"]);
        create_worktree(&repo, &worktree, "feature", false).unwrap();

        let existing = worktree_for_branch(&repo, "feature").unwrap();

        assert_eq!(
            existing.unwrap().canonicalize().unwrap(),
            worktree.canonicalize().unwrap()
        );
        assert!(worktree_for_branch(&repo, "missing").unwrap().is_none());
    }
}

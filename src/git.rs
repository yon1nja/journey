use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::models::GitState;

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

pub fn collect_state(worktree: &Path) -> Result<GitState> {
    let head = run_git(worktree, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();
    let branch = current_branch(worktree)?;
    let upstream = try_git(
        worktree,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
    let (ahead, behind) = ahead_behind(worktree, upstream.as_deref())?;
    let tracked_dirty = !run_git(worktree, &["status", "--porcelain", "--untracked-files=no"])?
        .trim()
        .is_empty();
    let untracked_files = run_git(worktree, &["ls-files", "--others", "--exclude-standard"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    Ok(GitState {
        head,
        branch,
        upstream,
        ahead,
        behind,
        tracked_dirty,
        untracked_files,
    })
}

pub fn create_dirty_snapshot(
    worktree: &Path,
    journey_id: &str,
    seq: u64,
    repo_name: &str,
) -> Result<Option<String>> {
    let snap = run_git(worktree, &["stash", "create"])?.trim().to_string();
    if snap.is_empty() {
        return Ok(None);
    }

    let ref_name = format!(
        "refs/journey/{}/{}-{}",
        sanitize_ref_component(journey_id),
        seq,
        sanitize_ref_component(repo_name)
    );
    run_git(worktree, &["update-ref", &ref_name, &snap])?;
    Ok(Some(ref_name))
}

pub fn diff_stat(worktree: &Path, head: &str, snapshot_ref: &str) -> Result<String> {
    let stat = run_git(worktree, &["diff", "--stat", head, snapshot_ref])?;
    let stat = stat.trim();
    if stat.is_empty() {
        Ok("no diff".to_string())
    } else {
        Ok(stat.to_string())
    }
}

pub fn apply_snapshot(worktree: &Path, snapshot_ref: &str) -> Result<()> {
    run_git(worktree, &["stash", "apply", snapshot_ref]).map(|_| ())
}

pub fn ensure_worktree(root: &Path, worktree: &Path, branch: &str) -> Result<()> {
    if worktree.exists() {
        return Ok(());
    }
    if !root.exists() {
        bail!(
            "cannot create missing worktree {}; root repo {} does not exist",
            worktree.display(),
            root.display()
        );
    }
    if let Some(parent) = worktree.parent() {
        fs::create_dir_all(parent)?;
    }
    let worktree_arg = worktree
        .to_str()
        .ok_or_else(|| anyhow!("worktree path is not valid UTF-8: {}", worktree.display()))?;
    run_git(root, &["worktree", "add", worktree_arg, branch]).map(|_| ())
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

fn try_git(path: &Path, args: &[&str]) -> Option<String> {
    run_git(path, args).ok()
}

fn current_branch(worktree: &Path) -> Result<String> {
    match run_git(worktree, &["symbolic-ref", "--short", "HEAD"]) {
        Ok(branch) => Ok(branch.trim().to_string()),
        Err(_) => Ok("HEAD".to_string()),
    }
}

fn ahead_behind(worktree: &Path, upstream: Option<&str>) -> Result<(u32, u32)> {
    let Some(upstream) = upstream else {
        return Ok((0, 0));
    };
    let range = format!("HEAD...{upstream}");
    let output = run_git(worktree, &["rev-list", "--left-right", "--count", &range])?;
    let mut parts = output.split_whitespace();
    let ahead = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let behind = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    Ok((ahead, behind))
}

fn sanitize_ref_component(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "value".to_string()
    } else {
        out
    }
}

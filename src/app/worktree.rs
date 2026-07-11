use super::*;

pub(crate) fn link_repo(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_path: &Path,
    name: Option<String>,
) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let discovered = git::discover_repo(repo_path)?;
    let repo_name = match name {
        Some(name) => validate_name(&name)?,
        None => discovered
            .root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "could not derive repo name from {}",
                    discovered.root.display()
                )
            })?,
    };

    if ctx.journey.repos.iter().any(|repo| repo.name == repo_name) {
        bail!("repo name `{repo_name}` is already linked; pass --name to choose another name");
    }

    let now = events::now_rfc3339()?;
    let canonical_root = storage::canonicalize_existing(&discovered.root)?;
    let canonical_worktree =
        storage::attach_worktree(home, &ctx.journey.id, &repo_name, &canonical_root, &now)?;
    let repo = RepoRef {
        name: repo_name.clone(),
        root: canonical_root,
        worktree: canonical_worktree,
        branch: discovered.branch.clone(),
    };
    ctx.journey.repos.push(repo.clone());
    if let Err(err) = storage::save_journey(&ctx.path, &ctx.journey) {
        let _ = storage::detach_worktree(home, &ctx.journey.id, &repo.name);
        return Err(err);
    }
    if let Err(err) = storage::sync_worktree_link(&ctx.path, &repo) {
        let _ = storage::detach_worktree(home, &ctx.journey.id, &repo.name);
        return Err(err);
    }

    let record = events::append_event(
        &ctx.path,
        EventKind::LinkRepo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            worktree: repo.worktree.clone(),
            branch: repo.branch.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts.clone()))?;

    Ok(format!(
        "linked `{}` ({}) to Journey `{}`",
        repo.name,
        repo.worktree.display(),
        ctx.journey.id
    ))
}

pub(crate) fn unlink_repo(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_name: &str,
) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let Some(position) = ctx
        .journey
        .repos
        .iter()
        .position(|repo| repo.name == repo_name)
    else {
        bail!(
            "repo `{repo_name}` is not linked to Journey `{}`",
            ctx.journey.id
        );
    };

    let repo = ctx.journey.repos.remove(position);
    storage::save_journey(&ctx.path, &ctx.journey)?;
    storage::detach_worktree(home, &ctx.journey.id, &repo.name)?;
    storage::remove_worktree_link(&ctx.path, &repo.name)?;

    let record = events::append_event(
        &ctx.path,
        EventKind::UnlinkRepo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            worktree: repo.worktree.clone(),
            branch: repo.branch.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;

    Ok(format!(
        "unlinked `{}` from Journey `{}`",
        repo.name, ctx.journey.id
    ))
}

pub(crate) fn delete_worktree(
    home: &Path,
    cwd: &Path,
    id: Option<&str>,
    repo_name: &str,
) -> Result<String> {
    let mut ctx = storage::resolve_context(home, id, cwd)?;
    let Some(position) = ctx
        .journey
        .repos
        .iter()
        .position(|repo| repo.name == repo_name)
    else {
        bail!(
            "repo `{repo_name}` is not linked to Journey `{}`",
            ctx.journey.id
        );
    };

    let repo = ctx.journey.repos[position].clone();
    git::remove_worktree(&repo.root, &repo.worktree)?;

    ctx.journey.repos.remove(position);
    storage::save_journey(&ctx.path, &ctx.journey)?;
    storage::detach_worktree(home, &ctx.journey.id, &repo.name)?;
    storage::remove_worktree_link(&ctx.path, &repo.name)?;

    let record = events::append_event(
        &ctx.path,
        EventKind::UnlinkRepo {
            name: repo.name.clone(),
            root: repo.root.clone(),
            worktree: repo.worktree.clone(),
            branch: repo.branch.clone(),
        },
    )?;
    finish_mutation(&ctx, Some(record.ts))?;

    Ok(format!(
        "deleted worktree {} and unlinked `{}` from Journey `{}`",
        repo.worktree.display(),
        repo.name,
        ctx.journey.id
    ))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git_command(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("failed to run git");

        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("git stdout was not UTF-8")
    }

    fn init_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        git_command(root, &["init"]);
        git_command(root, &["config", "user.email", "journey@example.com"]);
        git_command(root, &["config", "user.name", "Journey Test"]);
        fs::write(root.join("README.md"), "initial\n").unwrap();
        git_command(root, &["add", "README.md"]);
        git_command(root, &["commit", "-m", "initial"]);
    }

    #[test]
    fn delete_worktree_removes_git_worktree_and_unlinks_repo() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("journey-home");
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("feature-worktree");
        init_repo(&repo);
        git_command(&repo, &["branch", "feature"]);
        git::create_worktree(&repo, &worktree, "feature", false).unwrap();

        new_journey(&home, "Delete Worktree", None).unwrap();
        link_repo(
            &home,
            &repo,
            Some("delete-worktree"),
            &worktree,
            Some("feature".to_string()),
        )
        .unwrap();

        let deleted = delete_worktree(&home, &repo, Some("delete-worktree"), "feature").unwrap();

        assert!(deleted.contains("deleted worktree"));
        assert!(!worktree.exists());
        let journey =
            storage::load_journey(&storage::journey_dir(&home, "delete-worktree")).unwrap();
        assert!(journey.repos.is_empty());
        assert!(storage::load_worktree_index(&home)
            .unwrap()
            .attachments
            .is_empty());
        let journal = fs::read_to_string(
            storage::journey_dir(&home, "delete-worktree").join(storage::JOURNAL_FILE),
        )
        .unwrap();
        assert!(journal.contains("\"type\":\"unlink_repo\""));
    }

    #[test]
    fn done_archives_and_removes_worktrees_while_keeping_context() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("journey-home");
        let repo = temp.path().join("repo");
        let first = temp.path().join("first-worktree");
        let second = temp.path().join("second-worktree");
        init_repo(&repo);
        git_command(&repo, &["branch", "first"]);
        git_command(&repo, &["branch", "second"]);
        git::create_worktree(&repo, &first, "first", false).unwrap();
        git::create_worktree(&repo, &second, "second", false).unwrap();

        new_journey(&home, "Done Journey", None).unwrap();
        link_repo(
            &home,
            &repo,
            Some("done-journey"),
            &first,
            Some("first".to_string()),
        )
        .unwrap();
        link_repo(
            &home,
            &repo,
            Some("done-journey"),
            &second,
            Some("second".to_string()),
        )
        .unwrap();

        let result = done(&home, &repo, Some("done-journey")).unwrap();

        assert!(result.contains("archived and removed 2 worktrees"));
        assert!(!first.exists());
        assert!(!second.exists());
        let journey_dir = storage::journey_dir(&home, "done-journey");
        let journey = storage::load_journey(&journey_dir).unwrap();
        assert_eq!(journey.status, JourneyStatus::Archived);
        assert_eq!(journey.repos.len(), 2);
        assert!(journey_dir.join(storage::JOURNAL_FILE).exists());
        assert!(storage::load_worktree_index(&home)
            .unwrap()
            .attachments
            .is_empty());
        assert!(!journey_dir
            .join(storage::WORKTREES_DIR)
            .join("first")
            .exists());
        assert!(!journey_dir
            .join(storage::WORKTREES_DIR)
            .join("second")
            .exists());
    }
}

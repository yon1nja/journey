use super::*;

pub(super) fn branch_filter_indices(branches: &[String], query: &str) -> Vec<usize> {
    let terms = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return (0..branches.len()).collect();
    }

    branches
        .iter()
        .enumerate()
        .filter_map(|(idx, branch)| {
            let branch = branch.to_lowercase();
            terms
                .iter()
                .all(|term| branch.contains(term))
                .then_some(idx)
        })
        .collect()
}

pub(super) fn handle_text_input(
    input: &mut String,
    key: KeyEvent,
    target: DialogKeyTarget,
) -> DialogAction {
    match key.code {
        KeyCode::Esc => DialogAction::Cancel,
        KeyCode::Enter => target.submit_action(),
        KeyCode::Backspace => {
            input.pop();
            DialogAction::None
        }
        KeyCode::Char(ch) if editable_modifiers(key.modifiers) => {
            input.push(ch);
            DialogAction::None
        }
        _ => DialogAction::None,
    }
}

pub(super) fn editable_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

pub(super) fn sync_list_state(state: &mut ListState, selected: usize, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(selected.min(len - 1)));
    }
}

pub(super) fn create_and_link_worktree(
    home: &Path,
    cwd: &Path,
    journey_id: &str,
    branch: &str,
    path: &Path,
    create_branch: bool,
) -> Result<String> {
    let discovered = git::discover_repo(cwd)?;
    git::create_worktree(&discovered.root, path, branch, create_branch)?;
    let repo_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "worktree".to_string());
    let linked = app::link_repo(home, cwd, Some(journey_id), path, Some(repo_name))?;
    let action = if create_branch {
        "created worktree"
    } else {
        "checked out worktree"
    };
    Ok(format!(
        "{action} {} on branch `{}`\n{}",
        path.display(),
        branch,
        linked
    ))
}

pub(super) fn cd_command(path: &Path) -> String {
    format!("cd -- {}", shell_quote(path))
}

pub(super) fn shell_quote(path: &Path) -> String {
    let value = path.display().to_string();
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut last_error = None;
    for mut command in clipboard_commands() {
        match run_clipboard_command(&mut command, text) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no clipboard command is available")))
}

pub(super) fn clipboard_commands() -> Vec<Command> {
    let mut commands = Vec::new();

    #[cfg(target_os = "macos")]
    {
        commands.push(Command::new("pbcopy"));
    }

    #[cfg(target_os = "windows")]
    {
        commands.push(Command::new("clip"));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        commands.push(Command::new("wl-copy"));

        let mut xclip = Command::new("xclip");
        xclip.args(["-selection", "clipboard"]);
        commands.push(xclip);

        let mut xsel = Command::new("xsel");
        xsel.args(["--clipboard", "--input"]);
        commands.push(xsel);
    }

    commands
}

pub(super) fn run_clipboard_command(command: &mut Command, text: &str) -> Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to start clipboard command")?;
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("clipboard command did not open stdin");
    };
    stdin
        .write_all(text.as_bytes())
        .context("failed to write clipboard text")?;
    drop(stdin);

    let status = child
        .wait()
        .context("failed to wait for clipboard command")?;
    if !status.success() {
        anyhow::bail!("clipboard command exited with {status}");
    }
    Ok(())
}

pub(super) fn run_command_in_journey(command: &str, journey_path: &Path) -> Result<()> {
    let status = Command::new(command)
        .current_dir(journey_path)
        .status()
        .with_context(|| {
            format!(
                "failed to start {command} in Journey folder {}",
                journey_path.display()
            )
        })?;
    if !status.success() {
        anyhow::bail!("{command} exited with {status}");
    }
    Ok(())
}

pub(super) enum Dialog {
    None,
    NewTitle {
        input: String,
    },
    NewDescription {
        title: String,
        input: String,
    },
    WorktreeBranch {
        journey_id: String,
        input: String,
        default_branch: String,
    },
    WorktreePath {
        journey_id: String,
        branch: String,
        input: String,
        default_path: PathBuf,
    },
    ExistingBranch {
        journey_id: String,
        branches: Vec<String>,
        query: String,
        selected: usize,
        state: ListState,
    },
    ExistingWorktreePath {
        journey_id: String,
        branch: String,
        input: String,
        default_path: PathBuf,
    },
    ExistingWorktreeLinkConfirm {
        journey_id: String,
        branch: String,
        worktree: PathBuf,
    },
    Unlink {
        journey_id: String,
        repos: Vec<RepoRef>,
        selected: usize,
        state: ListState,
    },
    DeleteWorktree {
        journey_id: String,
        repos: Vec<RepoRef>,
        selected: usize,
        state: ListState,
    },
    Capture {
        journey_id: String,
        input: String,
    },
    DoneConfirm {
        journey_id: String,
        repo_count: usize,
    },
}

#[derive(Clone, Copy)]
pub(super) enum DialogKeyTarget {
    NewTitle,
    NewDescription,
    WorktreeBranch,
    WorktreePath,
    Capture,
    ExistingWorktreePath,
}

impl DialogKeyTarget {
    fn submit_action(self) -> DialogAction {
        match self {
            DialogKeyTarget::NewTitle => DialogAction::SubmitNewTitle,
            DialogKeyTarget::NewDescription => DialogAction::SubmitNewDescription,
            DialogKeyTarget::WorktreeBranch => DialogAction::SubmitWorktreeBranch,
            DialogKeyTarget::WorktreePath => DialogAction::SubmitWorktreePath,
            DialogKeyTarget::Capture => DialogAction::SubmitCapture,
            DialogKeyTarget::ExistingWorktreePath => DialogAction::SubmitExistingWorktreePath,
        }
    }
}

pub(super) enum DialogAction {
    None,
    Cancel,
    SubmitNewTitle,
    SubmitNewDescription,
    SubmitWorktreeBranch,
    SubmitWorktreePath,
    SubmitCapture,
    SubmitExistingBranch,
    SubmitExistingWorktreePath,
    SubmitExistingWorktreeLink,
    SubmitUnlink,
    SubmitDeleteWorktree,
    SubmitDone,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cd_command_quotes_journey_paths() {
        let command = cd_command(Path::new("/tmp/my journey/it's here"));

        assert_eq!(command, "cd -- '/tmp/my journey/it'\\''s here'");
    }

    #[test]
    fn branch_filter_matches_all_query_terms() {
        let branches = vec![
            "main".to_string(),
            "feature/editor-esm".to_string(),
            "fix/editor-cjs".to_string(),
        ];

        assert_eq!(branch_filter_indices(&branches, ""), vec![0, 1, 2]);
        assert_eq!(branch_filter_indices(&branches, "editor esm"), vec![1]);
        assert!(branch_filter_indices(&branches, "editor missing").is_empty());
    }
}

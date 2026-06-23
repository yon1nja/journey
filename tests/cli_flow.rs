use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn journey(home: &Path, args: &[&str]) -> String {
    journey_in(home, None, args)
}

fn journey_in(home: &Path, cwd: Option<&Path>, args: &[&str]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_journey"));
    command.env("JOURNEY_HOME", home).args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().expect("failed to run journey");

    assert!(
        output.status.success(),
        "journey {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout was not UTF-8")
}

fn journey_fails(home: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_journey"))
        .env("JOURNEY_HOME", home)
        .args(args)
        .output()
        .expect("failed to run journey");

    assert!(
        !output.status.success(),
        "journey {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stderr).expect("stderr was not UTF-8")
}

fn git(repo: &Path, args: &[&str]) -> String {
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

    String::from_utf8(output.stdout).expect("stdout was not UTF-8")
}

fn init_repo(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init"]);
    git(root, &["config", "user.email", "journey@example.com"]);
    git(root, &["config", "user.name", "Journey Test"]);
    fs::write(root.join("README.md"), "initial\n").unwrap();
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-m", "initial"]);
}

#[test]
fn full_cli_flow_links_docs_and_worktrees_without_generated_state() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    let created = journey(&home, &["new", "Test", "Journey"]);
    assert!(created.contains("`test-journey`"));

    let doc_path = journey(
        &home,
        &["doc", "new", "design", "--journey", "test-journey"],
    );
    assert!(Path::new(doc_path.trim()).exists());

    let linked = journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "test-journey"],
    );
    assert!(linked.contains("linked `repo`"));
    assert!(home
        .join("journeys/test-journey/worktrees/repo")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());

    fs::write(repo.join("README.md"), "initial\nchanged\n").unwrap();
    fs::write(repo.join("scratch.log"), "scratch\n").unwrap();
    assert!(git(&repo, &["stash", "list"]).trim().is_empty());

    let error = journey_fails(&home, &["checkpoint", "--journey", "test-journey"]);
    assert!(error.contains("unrecognized subcommand 'checkpoint'"));
    assert!(git(&repo, &["stash", "list"]).trim().is_empty());

    let journal = fs::read_to_string(home.join("journeys/test-journey/journal.jsonl")).unwrap();
    assert!(journal.contains("\"type\":\"link_repo\""));
    assert!(!journal.contains("dirty_snapshot_ref"));
    assert!(!journal.contains("scratch.log"));

    let resumed = journey(&home, &["resume", "test-journey"]);
    assert!(resumed.contains("Journey `test-journey` is now active"));

    assert!(!home.join("journeys/test-journey/NOW.md").exists());
}

#[test]
fn removed_structured_context_commands_are_not_available() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Context", "Test"]);
    let journey_dir = home.join("journeys/context-test");

    let output = Command::new(env!("CARGO_BIN_EXE_journey"))
        .env("JOURNEY_HOME", &home)
        .current_dir(&journey_dir)
        .args(["note", "inside", "context"])
        .output()
        .expect("failed to run journey");

    assert!(
        !output.status.success(),
        "journey note unexpectedly succeeded\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'note'"));
    assert!(!journey_dir.join("NOW.md").exists());
}

#[test]
fn list_non_interactive_keeps_table_output() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "List", "One"]);
    journey(&home, &["new", "List", "Two"]);

    let listed = journey(&home, &["list", "--non-interactive"]);

    assert!(listed.contains("list-one\tactive\t"));
    assert!(listed.contains("list-two\tactive\t"));
    assert!(listed.contains("no repos"));
}

#[test]
fn bare_journey_shows_non_interactive_list_when_piped() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Piped", "Test"]);

    let output = journey(&home, &[]);
    assert!(output.contains("piped-test"));
}

#[test]
fn journey_description_is_stored_and_rendered() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(
        &home,
        &[
            "new",
            "Described",
            "Journey",
            "--description",
            "Why this effort exists",
        ],
    );

    let status = journey(&home, &["status", "described-journey"]);
    assert!(status.contains("description: Why this effort exists"));

    let preview = journey(&home, &["__fzf-preview", "described-journey"]);
    assert!(preview.contains("description:"));
    assert!(preview.contains("Why this effort exists"));

    let journey_yaml =
        fs::read_to_string(home.join("journeys/described-journey/journey.yaml")).unwrap();
    assert!(journey_yaml.contains("description: Why this effort exists"));
}

#[test]
fn resume_and_pause_are_lifecycle_status_changes() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Lifecycle", "Only"]);

    let paused = journey(&home, &["pause", "lifecycle-only"]);
    assert!(paused.contains("Journey `lifecycle-only` is now paused"));

    let status = journey(&home, &["status", "lifecycle-only"]);
    assert!(status.contains("status: paused"));

    let resumed = journey(&home, &["resume", "lifecycle-only"]);
    assert!(resumed.contains("Journey `lifecycle-only` is now active"));

    let status = journey(&home, &["status", "lifecycle-only"]);
    assert!(status.contains("status: active"));
    assert!(!status.contains("checkpoint"));
}

#[test]
fn fzf_helpers_render_candidates_and_preview() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Fzf", "Helper"]);
    journey(&home, &["new", "Paused", "Helper"]);
    journey(&home, &["pause", "paused-helper"]);
    journey(&home, &["doc", "new", "popup", "--journey", "fzf-helper"]);

    let candidates = journey(&home, &["__fzf-candidates", "--query", "active"]);
    assert!(candidates.contains("fzf-helper\tFzf Helper"));
    assert!(!candidates.contains("paused-helper\tPaused Helper"));

    let all_candidates = journey(&home, &["__fzf-candidates", "--query", ""]);
    assert!(all_candidates.contains("fzf-helper\tFzf Helper"));
    assert!(all_candidates.contains("paused-helper\tPaused Helper"));

    let preview = journey(&home, &["__fzf-preview", "fzf-helper"]);
    assert!(preview.contains("Fzf Helper"));
    assert!(preview.contains("fzf-helper"));
    assert!(preview.contains("docs/popup.md"));
    assert!(!preview.contains("next actions"));
    assert!(!preview.contains("checkpoint"));
}

#[test]
fn shell_init_wraps_journey_for_parent_shell_cd() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");

    let init = journey(&home, &["shell-init"]);

    assert!(init.contains("journey() {"));
    assert!(init.contains("JOURNEY_SHELL_INTEGRATION=1"));
    assert!(init.contains("__journey_cd__\t"));
    assert!(init.contains("builtin cd --"));
}

#[test]
fn fzf_cd_action_exits_with_cd_request_under_shell_integration() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Cd", "Target"]);

    let output = Command::new(env!("CARGO_BIN_EXE_journey"))
        .env("JOURNEY_HOME", &home)
        .env("JOURNEY_SHELL_INTEGRATION", "1")
        .args(["__fzf-transform", "enter", "act:cd-target:shell"])
        .output()
        .expect("failed to run journey");

    assert!(
        output.status.success(),
        "journey __fzf-transform failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout was not UTF-8");
    let expected_dir = home.join("journeys/cd-target").display().to_string();
    assert!(stdout.contains("become(printf '%s\\n'"));
    assert!(stdout.contains("__journey_cd__\t"));
    assert!(stdout.contains(&expected_dir));
    assert!(!stdout.contains("$SHELL"));
    assert!(!stdout.contains("execute(cd"));
}

#[test]
fn context_resolves_from_inside_attached_worktree() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    journey(&home, &["new", "Repo", "Context"]);
    journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "repo-context"],
    );
    fs::create_dir_all(repo.join("src/nested")).unwrap();

    let doc = journey_in(
        &home,
        Some(&repo.join("src/nested")),
        &["doc", "new", "from-repo"],
    );
    assert!(doc.contains("repo-context/docs/from-repo.md"));

    let listed = journey_in(&home, Some(&repo), &["doc", "list"]);
    assert!(listed.contains("from-repo.md"));
}

#[test]
fn linked_worktree_can_only_belong_to_one_active_journey() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    journey(&home, &["new", "First", "Journey"]);
    journey(&home, &["new", "Second", "Journey"]);
    journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "first-journey"],
    );

    let error = journey_fails(
        &home,
        &[
            "link",
            repo.to_str().unwrap(),
            "--journey",
            "second-journey",
        ],
    );
    assert!(error.contains("already attached to Journey `first-journey`"));

    let unlinked = journey(&home, &["unlink", "repo", "--journey", "first-journey"]);
    assert!(unlinked.contains("unlinked `repo`"));

    let linked = journey(
        &home,
        &[
            "link",
            repo.to_str().unwrap(),
            "--journey",
            "second-journey",
        ],
    );
    assert!(linked.contains("Journey `second-journey`"));
}

#[test]
fn archive_detaches_worktree_context() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    journey(&home, &["new", "Archive", "Detach"]);
    journey(
        &home,
        &[
            "link",
            repo.to_str().unwrap(),
            "--journey",
            "archive-detach",
        ],
    );

    let archived = journey(&home, &["archive", "archive-detach"]);
    assert!(archived.contains("detached 1 worktrees"));

    let output = Command::new(env!("CARGO_BIN_EXE_journey"))
        .env("JOURNEY_HOME", &home)
        .current_dir(&repo)
        .args(["doc", "list"])
        .output()
        .expect("failed to run journey");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("attached worktree"));
}

#[test]
fn doctor_repair_rebuilds_worktree_index() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    journey(&home, &["new", "Repair", "Index"]);
    journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "repair-index"],
    );
    fs::remove_file(home.join("worktree-index.yaml")).unwrap();

    let repaired = journey(&home, &["doctor", "--repair"]);
    assert!(repaired.contains("rebuilt worktree index with 1 attachments"));

    let listed = journey_in(&home, Some(&repo), &["doc", "list"]);
    assert_eq!(listed.trim(), "no docs");
}

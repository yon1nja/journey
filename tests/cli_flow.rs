use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn journey_with_stdin(home: &Path, cwd: Option<&Path>, args: &[&str], stdin: &str) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_journey"));
    command
        .env("JOURNEY_HOME", home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    let mut child = command.spawn().expect("failed to run journey");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .expect("failed to write journey stdin");
    let output = child.wait_with_output().expect("failed to run journey");

    assert!(
        output.status.success(),
        "journey {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("stdout was not UTF-8")
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

    let agents = fs::read_to_string(home.join("journeys/test-journey/AGENTS.md")).unwrap();
    assert!(agents.contains(
        home.join("JOURNEY-AGENTS.md")
            .to_str()
            .expect("path should be UTF-8")
    ));
    assert!(agents.contains("journey.yaml"));
    assert!(!agents.contains("journey capture <text>"));

    let journey_agents = fs::read_to_string(home.join("JOURNEY-AGENTS.md")).unwrap();
    assert!(journey_agents.contains("README.md"));
    assert!(journey_agents.contains("journey readme new"));
    assert!(journey_agents.contains("journey capture <text>"));
    assert!(!journey_agents.contains("interactive"));

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
fn bare_journey_prints_overview_and_active_list() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Piped", "Test"]);

    let output = journey(&home, &[]);
    assert!(output.contains("journey --help"));
    assert!(output.contains("piped-test\tactive\t"));
    assert!(!home.join("config.toml").exists());
}

#[test]
fn list_filters_by_status() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Keep", "Active"]);
    journey(&home, &["new", "Now", "Paused"]);
    journey(&home, &["pause", "now-paused"]);

    let active = journey(&home, &["list"]);
    assert!(active.contains("keep-active"));
    assert!(!active.contains("now-paused"));

    let paused = journey(&home, &["list", "--status", "paused"]);
    assert!(paused.contains("now-paused"));
    assert!(!paused.contains("keep-active"));
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

    let journey_yaml =
        fs::read_to_string(home.join("journeys/described-journey/journey.yaml")).unwrap();
    assert!(journey_yaml.contains("description: Why this effort exists"));
}

#[test]
fn readme_command_creates_root_readme_without_overwriting() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Readme", "Journey"]);

    let readme_path = journey(&home, &["readme", "new", "--journey", "readme-journey"]);
    let readme_path = PathBuf::from(readme_path.trim());
    assert_eq!(readme_path, home.join("journeys/readme-journey/README.md"));
    assert_eq!(
        fs::read_to_string(&readme_path).unwrap(),
        "# Readme Journey\n\n"
    );

    let printed_path = journey(&home, &["readme", "path", "--journey", "readme-journey"]);
    assert_eq!(printed_path.trim(), readme_path.display().to_string());

    let error = journey_fails(&home, &["readme", "new", "--journey", "readme-journey"]);
    assert!(error.contains("README already exists"));
}

#[test]
fn capture_appends_timestamped_markdown_without_journal_events() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Capture", "Flow"]);

    let captured = journey(
        &home,
        &[
            "capture",
            "--journey",
            "capture-flow",
            "remember",
            "the",
            "race",
        ],
    );
    assert!(captured.contains("journeys/capture-flow/docs/capture.md"));

    let piped = journey_with_stdin(
        &home,
        None,
        &["capture", "--journey", "capture-flow", "--doc", "scratch"],
        "line one\nline two\n",
    );
    assert!(piped.contains("journeys/capture-flow/docs/scratch.md"));

    let capture = fs::read_to_string(home.join("journeys/capture-flow/docs/capture.md")).unwrap();
    assert!(capture.starts_with("# capture\n\n## "));
    assert!(capture.contains("\n\nremember the race\n"));

    let scratch = fs::read_to_string(home.join("journeys/capture-flow/docs/scratch.md")).unwrap();
    assert!(scratch.starts_with("# scratch\n\n## "));
    assert!(scratch.contains("\n\nline one\nline two\n"));

    let docs = journey(&home, &["doc", "list", "--journey", "capture-flow"]);
    assert!(docs.contains("capture.md"));
    assert!(docs.contains("scratch.md"));

    let status = journey(&home, &["status", "capture-flow"]);
    assert!(status.contains("events: 0"));

    let journal = fs::read_to_string(home.join("journeys/capture-flow/journal.jsonl")).unwrap();
    assert!(journal.trim().is_empty());
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
fn hidden_fzf_helpers_are_not_part_of_the_cli_surface() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Fzf", "Helper"]);

    let error = journey_fails(&home, &["__fzf-candidates", "--query", "active"]);
    assert!(error.contains("unrecognized subcommand '__fzf-candidates'"));
}

#[test]
fn shell_init_is_not_part_of_the_cli_surface() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");

    let error = journey_fails(&home, &["shell-init"]);

    assert!(error.contains("unrecognized subcommand 'shell-init'"));
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

#[test]
fn worktree_new_creates_branch_worktree_and_links_it() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);
    journey(&home, &["new", "Worktree", "Flow"]);

    let created = journey_in(
        &home,
        Some(&repo),
        &[
            "worktree",
            "new",
            "--branch",
            "feat/thing",
            "--journey",
            "worktree-flow",
        ],
    );
    let worktree = repo.join(".worktrees/feat-thing");
    assert!(created.contains("created branch `feat/thing`"));
    assert!(worktree.join("README.md").exists());
    assert_eq!(
        git(&worktree, &["branch", "--show-current"]).trim(),
        "feat/thing"
    );

    let journey_yaml =
        fs::read_to_string(home.join("journeys/worktree-flow/journey.yaml")).unwrap();
    assert!(journey_yaml.contains("name: feat-thing"));
    assert!(journey_yaml.contains("branch: feat/thing"));

    let removed = journey(
        &home,
        &[
            "worktree",
            "remove",
            "feat-thing",
            "--journey",
            "worktree-flow",
        ],
    );
    assert!(removed.contains("deleted worktree"));
    assert!(!worktree.exists());
}

#[test]
fn worktree_new_reuses_existing_branches() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);
    git(&repo, &["branch", "existing"]);
    git(&repo, &["branch", "checked-out"]);
    let elsewhere = temp.path().join("elsewhere");
    git(
        &repo,
        &[
            "worktree",
            "add",
            elsewhere.to_str().unwrap(),
            "checked-out",
        ],
    );
    journey(&home, &["new", "Reuse", "Branches"]);

    let checked_out = journey(
        &home,
        &[
            "worktree",
            "new",
            "--branch",
            "existing",
            "--repo",
            repo.to_str().unwrap(),
            "--journey",
            "reuse-branches",
        ],
    );
    assert!(checked_out.contains("checked out branch `existing`"));
    assert!(repo.join(".worktrees/existing").exists());

    let reused = journey(
        &home,
        &[
            "worktree",
            "new",
            "--branch",
            "checked-out",
            "--repo",
            repo.to_str().unwrap(),
            "--journey",
            "reuse-branches",
        ],
    );
    assert!(reused.contains("already checked out"));
    assert!(reused.contains("linked `elsewhere`"));

    let error = journey_fails(
        &home,
        &[
            "worktree",
            "new",
            "--branch",
            "existing",
            "--from",
            "HEAD",
            "--repo",
            repo.to_str().unwrap(),
            "--journey",
            "reuse-branches",
        ],
    );
    assert!(error.contains("--from only applies to new branches"));
}

#[test]
fn close_requires_outcome_and_marks_done() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);
    journey(&home, &["new", "Close", "Flow"]);
    journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "close-flow"],
    );

    let error = journey_fails(&home, &["close", "close-flow"]);
    assert!(error.contains("--outcome"));

    let closed = journey(
        &home,
        &["close", "close-flow", "--outcome", "Fixed the race"],
    );
    assert!(closed.contains("Journey `close-flow` is done (detached 1 worktrees)"));
    assert!(repo.exists());

    let status = journey(&home, &["status", "close-flow"]);
    assert!(status.contains("status: done"));
    let journey_yaml = fs::read_to_string(home.join("journeys/close-flow/journey.yaml")).unwrap();
    assert!(journey_yaml.contains("outcome: Fixed the race"));
    let journal = fs::read_to_string(home.join("journeys/close-flow/journal.jsonl")).unwrap();
    assert!(journal.contains("\"type\":\"close\""));
    assert!(journal.contains("Fixed the race"));

    let error = journey_fails(&home, &["close", "close-flow", "--outcome", "again"]);
    assert!(error.contains("already done"));

    let resumed = journey(&home, &["resume", "close-flow"]);
    assert!(resumed.contains("attached 1 worktrees"));
    let journey_yaml = fs::read_to_string(home.join("journeys/close-flow/journey.yaml")).unwrap();
    assert!(!journey_yaml.contains("outcome:"));
}

#[test]
fn close_can_remove_worktrees() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);
    journey(&home, &["new", "Close", "Remove"]);
    journey_in(
        &home,
        Some(&repo),
        &[
            "worktree",
            "new",
            "--branch",
            "side",
            "--journey",
            "close-remove",
        ],
    );
    let worktree = repo.join(".worktrees/side");
    fs::write(worktree.join("dirty.txt"), "dirty\n").unwrap();

    let error = journey_fails(
        &home,
        &[
            "close",
            "close-remove",
            "--outcome",
            "Done",
            "--remove-worktrees",
        ],
    );
    assert!(error.contains("uncommitted changes"));
    assert!(worktree.exists());

    fs::remove_file(worktree.join("dirty.txt")).unwrap();
    let closed = journey(
        &home,
        &[
            "close",
            "close-remove",
            "--outcome",
            "Done",
            "--remove-worktrees",
        ],
    );
    assert!(closed.contains("removed 1 worktrees"));
    assert!(!worktree.exists());
    assert!(repo.exists());
}

#[test]
fn guide_is_printed_and_mirrored_to_shared_guidance() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");

    let guide = journey(&home, &["guide"]);
    assert!(guide.starts_with("# Journey: guide for agents"));
    assert!(guide.contains("## When to use it"));
    assert!(guide.contains("journey close <id> --outcome"));

    let help = journey(&home, &["--help"]);
    assert!(help.contains("journey guide"));

    journey(&home, &["new", "Guide", "Sync"]);
    let shared = home.join("JOURNEY-AGENTS.md");
    assert_eq!(
        fs::read_to_string(&shared).unwrap().trim_end(),
        guide.trim_end()
    );

    let per_journey = home.join("journeys/guide-sync/AGENTS.md");
    fs::write(&shared, "# stale guidance from an older version\n").unwrap();
    fs::write(&per_journey, "# my own instructions\n").unwrap();

    journey(&home, &["list"]);
    assert_eq!(
        fs::read_to_string(&shared).unwrap().trim_end(),
        guide.trim_end()
    );
    assert_eq!(
        fs::read_to_string(&per_journey).unwrap(),
        "# my own instructions\n"
    );
}

#[test]
fn json_output_gives_agents_ids_and_paths() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    let repo = temp.path().join("repo");
    init_repo(&repo);

    let created: serde_json::Value = serde_json::from_str(&journey(
        &home,
        &["new", "Json", "Flow", "--description", "Why", "--json"],
    ))
    .unwrap();
    assert_eq!(created["id"], "json-flow");
    assert_eq!(created["status"], "active");
    let path = PathBuf::from(created["path"].as_str().unwrap());
    assert!(path.join("journey.yaml").exists());

    journey(
        &home,
        &["link", repo.to_str().unwrap(), "--journey", "json-flow"],
    );
    journey(&home, &["doc", "new", "notes", "--journey", "json-flow"]);
    journey(&home, &["readme", "new", "--journey", "json-flow"]);

    let listed: serde_json::Value =
        serde_json::from_str(&journey(&home, &["list", "--json"])).unwrap();
    assert_eq!(listed[0]["id"], "json-flow");
    assert_eq!(listed[0]["title"], "Json Flow");
    assert_eq!(listed[0]["repos"][0], "repo");
    assert_eq!(listed[0]["path"], path.to_str().unwrap());

    let status: serde_json::Value =
        serde_json::from_str(&journey(&home, &["status", "json-flow", "--json"])).unwrap();
    assert_eq!(status["description"], "Why");
    assert_eq!(status["docs"][0], "notes.md");
    assert_eq!(status["repos"][0]["name"], "repo");
    assert!(status["repos"][0]["branch"].is_string());
    assert!(status["readme"].as_str().unwrap().ends_with("README.md"));
    assert!(status["updated"].is_string());

    let docs: serde_json::Value = serde_json::from_str(&journey(
        &home,
        &["doc", "list", "--journey", "json-flow", "--json"],
    ))
    .unwrap();
    assert_eq!(docs[0]["name"], "notes.md");
    assert!(docs[0]["path"].as_str().unwrap().ends_with("docs/notes.md"));

    let text = journey(&home, &["status", "json-flow"]);
    assert!(text.contains("docs: 1\n  notes.md"));
    assert!(text.contains("repos: 1\n  repo\t"));
}

#[test]
fn list_search_finds_journeys_across_statuses() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(
        &home,
        &["new", "Auth", "Failures", "--description", "Token refresh"],
    );
    journey(&home, &["new", "Billing", "Export"]);
    journey(&home, &["new", "Old", "Auth", "Work"]);
    journey(&home, &["archive", "old-auth-work"]);

    let found = journey(&home, &["list", "--search", "AUTH"]);
    assert!(found.contains("auth-failures"));
    assert!(found.contains("old-auth-work"));
    assert!(!found.contains("billing-export"));

    let by_description = journey(&home, &["list", "--search", "refresh"]);
    assert!(by_description.contains("auth-failures"));

    let active_only = journey(&home, &["list", "--search", "auth", "--status", "active"]);
    assert!(!active_only.contains("old-auth-work"));

    let none = journey(&home, &["list", "--search", "nothing-here"]);
    assert_eq!(none.trim(), "no Journeys match `nothing-here`");

    let all = journey(&home, &["list", "--all"]);
    assert!(all.contains("billing-export"));
    assert!(all.contains("old-auth-work"));
}

#[test]
fn side_quest_closes_with_outcome_in_linked_readme() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Memory", "Work"]);
    journey(&home, &["readme", "new", "--journey", "memory-work"]);

    let created = journey(
        &home,
        &[
            "new",
            "Fix",
            "Leak",
            "--side-quest",
            "--linked-to",
            "memory-work",
        ],
    );
    assert!(created.contains("created side-quest `fix-leak`"));
    assert!(!home.join("journeys/memory-work/fix-leak").exists());
    let side_quest_yaml = fs::read_to_string(home.join("journeys/fix-leak/journey.yaml")).unwrap();
    assert!(side_quest_yaml.contains("kind: side-quest"));
    assert!(side_quest_yaml.contains("- memory-work"));

    let listed = journey(&home, &["list"]);
    assert!(listed.contains("fix-leak\tactive\t"));
    assert!(listed.contains("\tside-quest"));

    let parent: serde_json::Value =
        serde_json::from_str(&journey(&home, &["status", "memory-work", "--json"])).unwrap();
    assert_eq!(parent["kind"], "journey");
    assert_eq!(parent["linked_from"][0], "fix-leak");

    let closed = journey(
        &home,
        &[
            "close",
            "fix-leak",
            "--outcome",
            "Dropped the stale listener",
        ],
    );
    assert!(closed.contains("recorded outcome in the README of Journey `memory-work`"));

    let readme = fs::read_to_string(home.join("journeys/memory-work/README.md")).unwrap();
    assert!(readme.starts_with("# Memory Work\n"));
    assert!(readme.contains("## Side-quests\n\n- `fix-leak` ("));
    assert!(readme.contains("): Dropped the stale listener\n"));

    let error = journey_fails(
        &home,
        &["new", "Orphan", "--side-quest", "--linked-to", "missing"],
    );
    assert!(error.contains("unknown Journey id: missing"));
}

#[test]
fn side_quest_without_link_and_with_missing_readme() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Parent"]);
    journey(&home, &["new", "Loose", "Quest", "--side-quest"]);
    let closed = journey(
        &home,
        &["close", "loose-quest", "--outcome", "Nothing to fix"],
    );
    assert!(!closed.contains("recorded outcome"));

    journey(&home, &["new", "Linked", "Quest", "--side-quest"]);
    journey(&home, &["relate", "parent", "--journey", "linked-quest"]);
    journey(&home, &["close", "linked-quest", "--outcome", "Fixed"]);
    let readme = fs::read_to_string(home.join("journeys/parent/README.md")).unwrap();
    assert!(readme.starts_with("# Parent\n\n## Side-quests\n\n- `linked-quest` ("));
}

#[test]
fn relate_unrelate_and_promote() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("journey-home");
    journey(&home, &["new", "Main", "Effort"]);
    journey(&home, &["new", "Quick", "Check", "--side-quest"]);
    let quest_dir = home.join("journeys/quick-check");

    let related = journey_in(&home, Some(&quest_dir), &["relate", "main-effort"]);
    assert!(related.contains("related Journey `quick-check` to `main-effort`"));
    let error = journey_fails(
        &home,
        &["relate", "main-effort", "--journey", "quick-check"],
    );
    assert!(error.contains("already related"));
    let error = journey_fails(
        &home,
        &["relate", "quick-check", "--journey", "quick-check"],
    );
    assert!(error.contains("cannot be related to itself"));

    let status = journey(&home, &["status", "quick-check"]);
    assert!(status.contains("kind: side-quest"));
    assert!(status.contains("related to: main-effort"));

    let unrelated = journey(
        &home,
        &["unrelate", "main-effort", "--journey", "quick-check"],
    );
    assert!(unrelated.contains("unrelated"));
    assert!(!journey(&home, &["status", "quick-check"]).contains("related to"));

    let promoted = journey(&home, &["promote", "quick-check"]);
    assert!(promoted.contains("promoted side-quest `quick-check` to a journey"));
    assert!(quest_dir.join("journey.yaml").exists());
    assert!(journey(&home, &["status", "quick-check"]).contains("kind: journey"));
    let error = journey_fails(&home, &["promote", "quick-check"]);
    assert!(error.contains("is not a side-quest"));

    let journal = fs::read_to_string(quest_dir.join("journal.jsonl")).unwrap();
    assert!(journal.contains("\"type\":\"relate\""));
    assert!(journal.contains("\"type\":\"unrelate\""));
    assert!(journal.contains("\"type\":\"promote\""));
}

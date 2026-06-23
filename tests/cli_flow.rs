use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn journey(home: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_journey"))
        .env("JOURNEY_HOME", home)
        .args(args)
        .output()
        .expect("failed to run journey");

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
fn full_cli_flow_records_dirty_snapshot_without_stash_stack() {
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

    let checkpoint = journey(
        &home,
        &[
            "checkpoint",
            "--journey",
            "test-journey",
            "-m",
            "dirty checkpoint",
        ],
    );
    assert!(checkpoint.contains("1 dirty"));
    assert!(checkpoint.contains("1 untracked files"));
    assert!(git(&repo, &["stash", "list"]).trim().is_empty());

    let journal = fs::read_to_string(home.join("journeys/test-journey/journal.jsonl")).unwrap();
    assert!(journal.contains("\"dirty_snapshot_ref\""));
    assert!(journal.contains("scratch.log"));

    let resumed = journey(&home, &["resume", "test-journey"]);
    assert!(resumed.contains("dirty snapshot stat"));
    assert!(resumed.contains("1 untracked files were recorded"));

    let now = fs::read_to_string(home.join("journeys/test-journey/NOW.md")).unwrap();
    assert!(now.contains("GENERATED - do not edit"));
    assert!(now.contains("docs/design.md"));
    assert!(now.contains("scratch.log"));
}

#[test]
fn context_commands_work_inside_journey_folder() {
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
        output.status.success(),
        "journey note failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let now = fs::read_to_string(journey_dir.join("NOW.md")).unwrap();
    assert!(now.contains("Context Test"));
}

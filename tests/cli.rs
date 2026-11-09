use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos() as u64
            ^ COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("worktree-deck-{unique}"));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Repo {
    tmp: TempDir,
    root: PathBuf,
}

impl Repo {
    fn init() -> Self {
        let tmp = TempDir::new();
        let root = tmp.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test"]);
        fs::write(root.join("README"), "seed\n").unwrap();
        git(&root, &["add", "README"]);
        git(&root, &["commit", "-q", "-m", "init"]);
        Self { tmp, root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.tmp.path().join(name)
    }
}

struct Captured {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn git(dir: &Path, args: &[&str]) -> Output {
    let output = git_command(dir, args).output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        })
        .join("worktree-deck")
}

fn run(dir: &Path, args: &[&str]) -> Captured {
    let output = Command::new(binary())
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run worktree-deck");
    Captured {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn code(captured: &Captured) -> i32 {
    captured.status.code().unwrap_or(127)
}

fn assert_ok(captured: &Captured) {
    assert!(
        captured.status.success(),
        "status {}\nstdout:\n{}\nstderr:\n{}",
        code(captured),
        captured.stdout,
        captured.stderr
    );
}

fn assert_plain(text: &str) {
    assert!(!text.contains('\u{1b}'), "unexpected ANSI in {text}");
}

fn json_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("expected object, got {value}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn worktree_by_branch<'a>(document: &'a Value, branch: &str) -> &'a Value {
    document["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["branch"].as_str() == Some(branch))
        .unwrap_or_else(|| panic!("missing branch {branch} in {document}"))
}

#[test]
fn list_shows_main_and_linked_worktrees() {
    let repo = Repo::init();
    let linked = repo.path("linked");
    git(&repo.root, &["branch", "feature"]);
    git(
        &repo.root,
        &["worktree", "add", "--", linked.to_str().unwrap(), "feature"],
    );

    let captured = run(&repo.root, &["list"]);
    assert_ok(&captured);
    assert_plain(&captured.stdout);
    assert_plain(&captured.stderr);
    assert!(captured.stderr.is_empty(), "{}", captured.stderr);
    assert!(captured.stdout.contains("role: main"));
    assert!(captured.stdout.contains("role: linked"));
    assert!(captured.stdout.contains("branch: feature"));
    assert!(captured
        .stdout
        .contains("changes: staged=0 unstaged=0 untracked=0"));
}

#[test]
fn list_and_inspect_json_contract_has_no_ansi() {
    let repo = Repo::init();
    let linked = repo.path("linked");
    git(&repo.root, &["branch", "feature"]);
    git(
        &repo.root,
        &["worktree", "add", "--", linked.to_str().unwrap(), "feature"],
    );

    let listed = run(&repo.root, &["list", "--json"]);
    assert_ok(&listed);
    assert_plain(&listed.stdout);
    assert_plain(&listed.stderr);
    assert!(listed.stderr.is_empty());
    let document: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(document["schema_version"], 1);
    assert_eq!(
        json_keys(&document),
        vec!["schema_version".to_string(), "worktrees".to_string()]
    );
    let main = worktree_by_branch(&document, "main");
    assert_eq!(
        json_keys(main),
        vec![
            "bare",
            "branch",
            "changes",
            "detached",
            "head",
            "is_main",
            "lock_reason",
            "locked",
            "path",
            "prunable",
            "prunable_reason",
            "ref_name",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>()
    );
    assert_eq!(main["is_main"], true);
    assert_eq!(main["detached"], false);
    assert_eq!(main["bare"], false);
    assert_eq!(main["ref_name"], "refs/heads/main");
    assert_eq!(main["locked"], false);
    assert!(main["lock_reason"].is_null());
    assert!(main["head"]
        .as_str()
        .unwrap()
        .chars()
        .all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(
        json_keys(&main["changes"]),
        vec![
            "staged".to_string(),
            "unstaged".to_string(),
            "untracked".to_string()
        ]
    );

    let inspected = run(&linked, &["inspect", "--json", "feature"]);
    assert_ok(&inspected);
    assert_plain(&inspected.stdout);
    let one: Value = serde_json::from_str(&inspected.stdout).unwrap();
    assert_eq!(one["schema_version"], 1);
    assert_eq!(one["worktree"]["branch"], "feature");
    assert_eq!(one["worktree"]["is_main"], false);
    let by_ref = run(&repo.root, &["path", "refs/heads/feature"]);
    assert_ok(&by_ref);
    assert_eq!(
        by_ref.stdout,
        format!("{}\n", one["worktree"]["path"].as_str().unwrap())
    );
    fs::create_dir(repo.root.join("nested-dir")).unwrap();
    let nested = run(&repo.root.join("nested-dir"), &["path", "."]);
    assert_ok(&nested);
    assert_eq!(
        nested.stdout,
        format!("{}\n", main["path"].as_str().unwrap())
    );

    let missing = run(&repo.root, &["inspect", "--json", "missing"]);
    assert_eq!(code(&missing), 5);
    assert_plain(&missing.stdout);
    assert_plain(&missing.stderr);
    let err: Value = serde_json::from_str(&missing.stdout).unwrap();
    assert_eq!(err["schema_version"], 1);
    assert_eq!(err["error"]["code"], "not_found");
    assert_eq!(
        json_keys(&err),
        vec!["error".to_string(), "schema_version".to_string()]
    );
    assert_eq!(
        json_keys(&err["error"]),
        vec!["code".to_string(), "message".to_string()]
    );
    assert!(missing.stderr.starts_with("error:"));
}

#[test]
fn detached_head_is_labeled_in_text_and_json() {
    let repo = Repo::init();
    let detached = repo.path("detached");
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "--detach",
            "--",
            detached.to_str().unwrap(),
            "HEAD",
        ],
    );
    let captured = run(&repo.root, &["list", "--json"]);
    assert_ok(&captured);
    let document: Value = serde_json::from_str(&captured.stdout).unwrap();
    let item = document["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["detached"] == true)
        .expect("detached worktree");
    assert!(item["branch"].is_null());
    assert!(item["ref_name"].is_null());
    assert_eq!(item["is_main"], false);

    let text = run(&repo.root, &["list"]);
    assert_ok(&text);
    assert!(text.stdout.contains("detached: yes"));
}

#[test]
fn outside_a_repository_fails_with_structured_json() {
    let tmp = TempDir::new();
    let captured = run(tmp.path(), &["list", "--json"]);
    assert_eq!(code(&captured), 3);
    assert_plain(&captured.stdout);
    assert_plain(&captured.stderr);
    let document: Value = serde_json::from_str(&captured.stdout).unwrap();
    assert_eq!(document["error"]["code"], "not_a_repository");
    assert!(captured.stderr.starts_with("error:"));

    let text = run(tmp.path(), &["list"]);
    assert_eq!(code(&text), 3);
    assert!(text.stdout.is_empty());
    assert!(text.stderr.contains("not inside a git repository"));
}

#[test]
fn counts_staged_unstaged_and_untracked_changes() {
    let repo = Repo::init();
    let linked = repo.path("linked");
    git(&repo.root, &["branch", "feature"]);
    git(
        &repo.root,
        &["worktree", "add", "--", linked.to_str().unwrap(), "feature"],
    );
    fs::write(linked.join("README"), "changed\n").unwrap();
    fs::write(linked.join("staged.txt"), "staged\n").unwrap();
    git(&linked, &["add", "staged.txt"]);
    fs::create_dir(linked.join("nested")).unwrap();
    fs::write(linked.join("nested").join("keep.txt"), "new\n").unwrap();

    let captured = run(&repo.root, &["list", "--json"]);
    assert_ok(&captured);
    let document: Value = serde_json::from_str(&captured.stdout).unwrap();
    let feature = worktree_by_branch(&document, "feature");
    assert_eq!(feature["changes"]["staged"], 1);
    assert_eq!(feature["changes"]["unstaged"], 1);
    assert_eq!(feature["changes"]["untracked"], 1);
}

#[test]
fn special_paths_round_trip_through_path_and_json() {
    let repo = Repo::init();
    let space = repo.path("my worktree");
    let unicode = repo.path("árvore");
    let space_out = run(
        &repo.root,
        &[
            "create",
            "--path",
            space.to_str().unwrap(),
            "--new-branch",
            "space-b",
            "--start-point",
            "HEAD",
            "--yes",
        ],
    );
    assert_ok(&space_out);
    let unicode_out = run(
        &repo.root,
        &[
            "create",
            "--path",
            unicode.to_str().unwrap(),
            "--new-branch",
            "unicode-b",
            "--start-point",
            "main",
            "--yes",
        ],
    );
    assert_ok(&unicode_out);

    let listed = run(&repo.root, &["list", "--json"]);
    assert_ok(&listed);
    let document: Value = serde_json::from_str(&listed.stdout).unwrap();
    let space_item = worktree_by_branch(&document, "space-b");
    let unicode_item = worktree_by_branch(&document, "unicode-b");
    let space_path = space_item["path"].as_str().unwrap();
    let unicode_path = unicode_item["path"].as_str().unwrap();
    assert!(space_path.contains("my worktree"));
    assert!(Path::new(space_path).join("README").is_file());
    assert!(Path::new(unicode_path).join("README").is_file());
    assert_ne!(space_path, unicode_path);

    let printed = run(&repo.root, &["path", "space-b"]);
    assert_ok(&printed);
    assert_eq!(printed.stderr, "");
    assert_eq!(printed.stdout, format!("{space_path}\n"));
    let current = run(&repo.root, &["path", "."]);
    assert_ok(&current);
    assert_eq!(
        current.stdout,
        format!(
            "{}\n",
            worktree_by_branch(&document, "main")["path"]
                .as_str()
                .unwrap()
        )
    );
}

#[test]
fn create_requires_explicit_source_confirmation_and_keeps_dry_run_side_effect_free() {
    let repo = Repo::init();
    let relative = PathBuf::from("..").join("relative topic");
    let missing_branch = run(
        &repo.root,
        &[
            "create",
            "--path",
            repo.path("missing").to_str().unwrap(),
            "--new-branch",
            "topic",
            "--yes",
        ],
    );
    assert_eq!(code(&missing_branch), 2);
    assert!(!repo.path("missing").exists());

    let neither = run(
        &repo.root,
        &[
            "create",
            "--path",
            repo.path("neither").to_str().unwrap(),
            "--yes",
        ],
    );
    assert_eq!(code(&neither), 2);
    assert!(neither.stderr.contains("--branch") || neither.stderr.contains("start-point"));

    let unconfirmed = run(
        &repo.root,
        &[
            "create",
            "--path",
            repo.path("unconfirmed").to_str().unwrap(),
            "--new-branch",
            "topic",
            "--start-point",
            "HEAD",
        ],
    );
    assert_eq!(code(&unconfirmed), 4);
    assert!(unconfirmed.stderr.contains("--yes"));
    assert!(!repo.path("unconfirmed").exists());
    assert!(git(&repo.root, &["branch", "--list", "topic"])
        .stdout
        .is_empty());

    let dry = run(
        &repo.root,
        &[
            "create",
            "--path",
            &relative.to_string_lossy(),
            "--new-branch",
            "topic",
            "--start-point",
            "HEAD",
            "--dry-run",
        ],
    );
    assert_ok(&dry);
    assert!(dry.stdout.contains("dry-run: yes"));
    assert!(dry.stdout.contains("effects: none"));
    assert!(dry.stdout.contains("worktree add"));
    assert!(dry.stdout.contains("-b"));
    assert!(dry.stdout.contains("topic"));
    assert!(dry.stdout.contains("HEAD"));
    assert!(!dry.stdout.contains("--force"));
    assert!(!repo.path("relative topic").exists());
    assert!(git(&repo.root, &["branch", "--list", "topic"])
        .stdout
        .is_empty());

    let in_use = run(
        &repo.root,
        &[
            "create",
            "--path",
            repo.path("second-main").to_str().unwrap(),
            "--branch",
            "main",
            "--yes",
        ],
    );
    assert_eq!(code(&in_use), 4);
    assert!(in_use.stderr.contains("already checked out"));
    assert!(!repo.path("second-main").exists());

    let unknown = run(
        &repo.root,
        &[
            "create",
            "--path",
            repo.path("bad-start").to_str().unwrap(),
            "--new-branch",
            "topic",
            "--start-point",
            "does-not-exist",
            "--yes",
        ],
    );
    assert_eq!(code(&unknown), 1);
    assert!(unknown.stderr.contains("was not found"));
    assert!(!repo.path("bad-start").exists());

    let created = run(
        &repo.root,
        &[
            "create",
            "--path",
            &relative.to_string_lossy(),
            "--new-branch",
            "topic",
            "--start-point",
            "main",
            "--yes",
        ],
    );
    assert_ok(&created);
    assert!(created.stdout.contains("created"));
    assert!(created.stdout.contains("branch: topic"));
    assert!(repo.path("relative topic").join("README").is_file());
    let from_linked = run(&repo.path("relative topic"), &["path", "main"]);
    assert_ok(&from_linked);
    assert!(from_linked.stdout.contains("repo"));
}

#[test]
fn remove_refuses_main_locked_dirty_and_unconfirmed_without_deleting_files() {
    let repo = Repo::init();
    let main_remove = run(&repo.root, &["remove", "--yes", "."]);
    assert_eq!(code(&main_remove), 4);
    assert!(main_remove.stderr.contains("main worktree"));
    assert_eq!(
        fs::read_to_string(repo.root.join("README")).unwrap(),
        "seed\n"
    );

    let locked = repo.path("locked");
    git(&repo.root, &["branch", "locked-b"]);
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "--",
            locked.to_str().unwrap(),
            "locked-b",
        ],
    );
    git(
        &repo.root,
        &[
            "worktree",
            "lock",
            "--reason",
            "do not touch",
            "--",
            locked.to_str().unwrap(),
        ],
    );
    let locked_remove = run(&repo.root, &["remove", "--yes", "locked-b"]);
    assert_eq!(code(&locked_remove), 4);
    assert!(locked_remove.stderr.contains("locked"));
    assert!(locked.join("README").is_file());
    let locked_json = run(&repo.root, &["inspect", "--json", "locked-b"]);
    assert_ok(&locked_json);
    let locked_doc: Value = serde_json::from_str(&locked_json.stdout).unwrap();
    assert_eq!(locked_doc["worktree"]["locked"], true);
    assert_eq!(locked_doc["worktree"]["lock_reason"], "do not touch");

    for (name, branch, prepare) in [
        ("untracked", "untracked-b", Prepare::Untracked),
        ("staged", "staged-b", Prepare::Staged),
        ("unstaged", "unstaged-b", Prepare::Unstaged),
    ] {
        let path = repo.path(name);
        git(&repo.root, &["branch", branch]);
        git(
            &repo.root,
            &["worktree", "add", "--", path.to_str().unwrap(), branch],
        );
        let marker = path.join("notes.txt");
        match prepare {
            Prepare::Untracked => fs::write(&marker, "keep-me\n").unwrap(),
            Prepare::Staged => {
                fs::write(&marker, "keep-me\n").unwrap();
                git(&path, &["add", "notes.txt"]);
            }
            Prepare::Unstaged => {
                fs::write(path.join("README"), "keep-me\n").unwrap();
            }
        }
        let refused = run(&repo.root, &["remove", "--yes", branch]);
        assert_eq!(code(&refused), 4, "{branch}\n{}", refused.stderr);
        assert!(
            refused.stderr.contains("local changes"),
            "{}",
            refused.stderr
        );
        assert!(path.is_dir());
        if matches!(prepare, Prepare::Unstaged) {
            assert_eq!(
                fs::read_to_string(path.join("README")).unwrap(),
                "keep-me\n"
            );
        } else {
            assert_eq!(fs::read_to_string(&marker).unwrap(), "keep-me\n");
        }
    }

    let clean = repo.path("clean");
    git(&repo.root, &["branch", "clean-b"]);
    git(
        &repo.root,
        &["worktree", "add", "--", clean.to_str().unwrap(), "clean-b"],
    );
    let unconfirmed = run(&repo.root, &["remove", "clean-b"]);
    assert_eq!(code(&unconfirmed), 4);
    assert!(unconfirmed.stderr.contains("--yes"));
    assert!(clean.join("README").is_file());
    assert!(unconfirmed.stdout.is_empty());

    let dry = run(&repo.root, &["remove", "clean-b", "--dry-run"]);
    assert_ok(&dry);
    assert!(dry.stdout.contains("dry-run: yes"));
    assert!(dry.stdout.contains("effects: none"));
    assert!(dry.stdout.contains("worktree remove"));
    assert!(dry.stdout.contains("keeps_branch: clean-b"));
    assert!(!dry.stdout.contains("--force"));
    assert!(clean.join("README").is_file());

    let dirty = repo.path("dirty-dry");
    git(&repo.root, &["branch", "dirty-dry-b"]);
    git(
        &repo.root,
        &[
            "worktree",
            "add",
            "--",
            dirty.to_str().unwrap(),
            "dirty-dry-b",
        ],
    );
    fs::write(dirty.join("notes.txt"), "keep-me\n").unwrap();
    let dry_dirty = run(&repo.root, &["remove", "dirty-dry-b", "--dry-run"]);
    assert_eq!(code(&dry_dirty), 4);
    assert!(dry_dirty.stdout.is_empty(), "{}", dry_dirty.stdout);
    assert!(dry_dirty.stderr.contains("dry-run made no changes"));
    assert_eq!(
        fs::read_to_string(dirty.join("notes.txt")).unwrap(),
        "keep-me\n"
    );
}

#[test]
fn remove_clean_worktree_keeps_the_branch_and_prunable_registration() {
    let repo = Repo::init();
    let clean = repo.path("clean");
    git(&repo.root, &["branch", "clean-b"]);
    git(
        &repo.root,
        &["worktree", "add", "--", clean.to_str().unwrap(), "clean-b"],
    );
    let tip =
        String::from_utf8(git(&repo.root, &["rev-parse", "refs/heads/clean-b"]).stdout).unwrap();
    let removed = run(&repo.root, &["remove", "--yes", "clean-b"]);
    assert_ok(&removed);
    assert!(removed.stdout.contains("removed"));
    assert!(removed.stdout.contains("branch_kept: clean-b"));
    assert!(!clean.exists());
    let after =
        String::from_utf8(git(&repo.root, &["rev-parse", "refs/heads/clean-b"]).stdout).unwrap();
    assert_eq!(tip, after);
    assert_eq!(
        fs::read_to_string(repo.root.join("README")).unwrap(),
        "seed\n"
    );

    let gone = repo.path("gone");
    git(&repo.root, &["branch", "gone-b"]);
    git(
        &repo.root,
        &["worktree", "add", "--", gone.to_str().unwrap(), "gone-b"],
    );
    fs::remove_dir_all(&gone).unwrap();
    let listed = run(&repo.root, &["list", "--json"]);
    assert_ok(&listed);
    let document: Value = serde_json::from_str(&listed.stdout).unwrap();
    let item = worktree_by_branch(&document, "gone-b");
    assert_eq!(item["prunable"], true);
    assert!(item["changes"].is_null());
    let removed_prunable = run(&repo.root, &["remove", "--yes", "gone-b"]);
    assert_ok(&removed_prunable);
    let after_list = String::from_utf8(git(&repo.root, &["worktree", "list"]).stdout).unwrap();
    assert!(!after_list.contains("gone"));
    assert!(
        git(&repo.root, &["rev-parse", "--verify", "refs/heads/gone-b"])
            .status
            .success()
    );
}

#[test]
fn recorded_git_invocations_do_not_force_prune_or_delete_branches() {
    let repo = Repo::init();
    let log = repo.path("git.log");
    let bin = repo.path("bin");
    fs::create_dir(&bin).unwrap();
    let real =
        String::from_utf8(Command::new("which").arg("git").output().unwrap().stdout).unwrap();
    let real = real.trim();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexec {} \"$@\"\n",
        shell_single(&log),
        shell_single(Path::new(real))
    );
    let fake = bin.join("git");
    fs::write(&fake, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
    }

    let path = repo.path("logged");
    let mut command = Command::new(binary());
    command
        .current_dir(&repo.root)
        .args([
            "create",
            "--path",
            path.to_str().unwrap(),
            "--new-branch",
            "logged-b",
            "--start-point",
            "HEAD",
            "--yes",
        ])
        .stdin(Stdio::null())
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    let created = command.output().unwrap();
    assert!(
        created.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let removed = Command::new(binary())
        .current_dir(&repo.root)
        .args(["remove", "--yes", "logged-b"])
        .stdin(Stdio::null())
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    let transcript = fs::read_to_string(&log).unwrap();
    assert!(transcript.contains("worktree add"), "{transcript}");
    assert!(transcript.contains("worktree remove"), "{transcript}");
    assert!(transcript.contains("-b"), "{transcript}");
    for forbidden in ["--force", "prune", "reset", "clean"] {
        assert!(
            !transcript.contains(forbidden),
            "{forbidden} in {transcript}"
        );
    }
    assert!(git(
        &repo.root,
        &["rev-parse", "--verify", "refs/heads/logged-b"]
    )
    .status
    .success());
}

#[test]
fn help_and_missing_subcommand_are_plain_text() {
    let repo = Repo::init();
    let help = run(&repo.root, &["--help"]);
    assert_ok(&help);
    assert_plain(&help.stdout);
    assert!(help.stdout.contains("Examples"));
    assert!(help.stdout.contains("Exit codes"));
    let list_help = run(&repo.root, &["list", "--help"]);
    assert_ok(&list_help);
    assert!(list_help.stdout.contains("--json"));
    let create_help = run(&repo.root, &["create", "--help"]);
    assert_ok(&create_help);
    assert!(create_help.stdout.contains("--start-point"));
    assert!(create_help.stdout.contains("--dry-run"));
    let remove_help = run(&repo.root, &["remove", "--help"]);
    assert_ok(&remove_help);
    assert!(remove_help.stdout.contains("--yes"));

    let bare = run(&repo.root, &[]);
    assert_eq!(code(&bare), 2);
    assert!(bare.stderr.contains("subcommand"));
    assert_plain(&bare.stderr);
}

#[test]
fn shell_integration_cds_without_eval() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/worktree-deck.sh"))
            .unwrap();
    for line in source.lines() {
        let code = line.split('#').next().unwrap_or("");
        assert!(
            !code.split_whitespace().any(|word| word == "eval"),
            "shell helper must not eval: {line}"
        );
    }
    assert!(source.contains("cd --"));
    let syntax = Command::new("bash")
        .args(["-n", "contrib/worktree-deck.sh"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        syntax.status.success(),
        "{}",
        String::from_utf8_lossy(&syntax.stderr)
    );

    let tmp = TempDir::new();
    let dest = tmp.path().join("My Dir");
    fs::create_dir(&dest).unwrap();
    let bin = tmp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let fake = bin.join("worktree-deck");
    fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' {}\n", shell_single(&dest)),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&fake).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake, permissions).unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("contrib/worktree-deck.sh");
    let output = Command::new("bash")
        .arg("-c")
        .arg("source \"$1\"; worktree_deck_cd sample; printf '%s\\n' \"$PWD\"")
        .arg("bash")
        .arg(&script)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        dest.to_str().unwrap()
    );

    fs::write(&fake, "#!/bin/sh\nprintf 'one\\ntwo\\n'\n").unwrap();
    let rejected = Command::new("bash")
        .arg("-c")
        .arg("source \"$1\"; worktree_deck_cd sample")
        .arg("bash")
        .arg(&script)
        .current_dir(tmp.path())
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("newline"));
}

#[test]
fn sources_do_not_call_forbidden_operations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for name in [
        "cli.rs",
        "git.rs",
        "main.rs",
        "ops.rs",
        "output.rs",
        "parse.rs",
        "tui.rs",
        "lib.rs",
    ] {
        let text = fs::read_to_string(root.join(name)).unwrap();
        let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
        for needle in [
            "--force",
            "remove_dir_all",
            "reset --hard",
            "worktree prune",
            "git clean",
        ] {
            assert!(!production.contains(needle), "{name} contains {needle}");
        }
    }
}

#[derive(Clone, Copy)]
enum Prepare {
    Untracked,
    Staged,
    Unstaged,
}

fn shell_single(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

//! A throwaway Git repository for tests, shared by every module that needs one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    pub(crate) fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("repo-lines-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let repo = Self { path };
        repo.run(&["init", "-b", "master"]);
        repo.run(&["config", "user.name", "Repo Lines Test"]);
        repo.run(&["config", "user.email", "repo-lines@example.invalid"]);
        repo
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn write(&self, name: &str, contents: &[u8]) {
        fs::write(self.path.join(name), contents).unwrap();
    }

    pub(crate) fn write_git_info_attributes(&self, contents: &[u8]) {
        fs::write(self.path.join(".git/info/attributes"), contents).unwrap();
    }

    pub(crate) fn commit(&self, message: &str) {
        self.run(&["add", "."]);
        self.run(&["commit", "-m", message]);
    }

    pub(crate) fn commit_at(&self, message: &str, datetime: &str) {
        let output = Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(&self.path)
            .env("GIT_AUTHOR_DATE", datetime)
            .env("GIT_COMMITTER_DATE", datetime)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub(crate) fn head(&self) -> String {
        String::from_utf8(self.output(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_owned()
    }

    pub(crate) fn run(&self, args: &[&str]) {
        let output = self.output(args);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn output(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.path)
            .output()
            .unwrap()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

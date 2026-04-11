//! Test workspace with temporary directory and file helpers.

use std::path::Path;

/// A temporary workspace for E2E tests.
/// Files are created in a temp directory that is cleaned up on drop.
pub struct TestWorkspace {
    dir: tempfile::TempDir,
}

impl TestWorkspace {
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("failed to create temp dir"),
        }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn path_str(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    /// Write a file relative to the workspace root.
    pub fn write_file(&self, relative_path: &str, content: &str) {
        let path = self.dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent dirs");
        }
        std::fs::write(&path, content).expect("failed to write file");
    }

    /// Read a file relative to the workspace root.
    pub fn read_file(&self, relative_path: &str) -> String {
        let path = self.dir.path().join(relative_path);
        std::fs::read_to_string(&path).expect("failed to read file")
    }

    /// Check if a file exists relative to the workspace root.
    pub fn file_exists(&self, relative_path: &str) -> bool {
        self.dir.path().join(relative_path).exists()
    }
}

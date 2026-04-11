//! Auto-memory system with MEMORY.md index.
//!
//! Matching Claude Code's memory system: persistent file-based memories
//! stored in `.cisco-code/memory/` with a MEMORY.md index file.
//! Memory types: user, feedback, project, reference.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Memory entry types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// User role, preferences, knowledge.
    User,
    /// Feedback on approach — corrections and confirmations.
    Feedback,
    /// Project context — goals, decisions, deadlines.
    Project,
    /// Pointers to external resources.
    Reference,
}

/// A single memory entry with frontmatter metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
    pub content: String,
    /// Filename (relative to memory directory).
    pub filename: String,
}

/// Frontmatter parsed from a memory file.
#[derive(Debug, Clone, Deserialize)]
struct MemoryFrontmatter {
    name: String,
    description: String,
    #[serde(rename = "type")]
    memory_type: MemoryType,
}

/// Memory manager for reading and writing memory files.
pub struct MemoryManager {
    /// Root directory for memory files.
    memory_dir: PathBuf,
    /// Path to MEMORY.md index.
    index_path: PathBuf,
    /// Cached entries.
    entries: Vec<MemoryEntry>,
}

impl MemoryManager {
    /// Create a new MemoryManager for the given directory.
    pub fn new(memory_dir: &Path) -> Self {
        let index_path = memory_dir.join("MEMORY.md");
        Self {
            memory_dir: memory_dir.to_path_buf(),
            index_path,
            entries: Vec::new(),
        }
    }

    /// Create from project directory (uses `.cisco-code/memory/`).
    pub fn for_project(project_dir: &str) -> Self {
        let dir = PathBuf::from(project_dir)
            .join(".cisco-code")
            .join("memory");
        Self::new(&dir)
    }

    /// Create from user home directory (uses `~/.cisco-code/memory/`).
    pub fn for_user() -> Option<Self> {
        let home = std::env::var("HOME").ok()?;
        let dir = PathBuf::from(home)
            .join(".cisco-code")
            .join("memory");
        Some(Self::new(&dir))
    }

    /// Load all memory entries from disk.
    pub fn load(&mut self) -> Result<()> {
        self.entries.clear();

        if !self.memory_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.memory_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") && path.file_name() != Some("MEMORY.md".as_ref()) {
                if let Ok(mem) = self.parse_memory_file(&path) {
                    self.entries.push(mem);
                }
            }
        }

        Ok(())
    }

    /// Parse a memory file with frontmatter.
    fn parse_memory_file(&self, path: &Path) -> Result<MemoryEntry> {
        let content = std::fs::read_to_string(path)?;
        let (frontmatter, body) = parse_frontmatter(&content)?;

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown.md")
            .to_string();

        Ok(MemoryEntry {
            name: frontmatter.name,
            description: frontmatter.description,
            memory_type: frontmatter.memory_type,
            content: body,
            filename,
        })
    }

    /// Save a memory entry to disk and update the index.
    pub fn save(&mut self, entry: &MemoryEntry) -> Result<()> {
        std::fs::create_dir_all(&self.memory_dir)?;

        let file_content = format!(
            "---\nname: {}\ndescription: {}\ntype: {}\n---\n\n{}",
            entry.name,
            entry.description,
            serde_json::to_string(&entry.memory_type)?.trim_matches('"'),
            entry.content,
        );

        let path = self.memory_dir.join(&entry.filename);
        std::fs::write(&path, file_content)?;

        // Update cached entries
        self.entries.retain(|e| e.filename != entry.filename);
        self.entries.push(entry.clone());

        // Update index
        self.update_index()?;

        Ok(())
    }

    /// Remove a memory entry.
    pub fn remove(&mut self, filename: &str) -> Result<bool> {
        let path = self.memory_dir.join(filename);
        if path.exists() {
            std::fs::remove_file(&path)?;
            self.entries.retain(|e| e.filename != filename);
            self.update_index()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get all loaded entries.
    pub fn entries(&self) -> &[MemoryEntry] {
        &self.entries
    }

    /// Find entries by type.
    pub fn entries_by_type(&self, memory_type: &MemoryType) -> Vec<&MemoryEntry> {
        self.entries
            .iter()
            .filter(|e| &e.memory_type == memory_type)
            .collect()
    }

    /// Search entries by keyword in name, description, or content.
    pub fn search(&self, keyword: &str) -> Vec<&MemoryEntry> {
        let kw = keyword.to_lowercase();
        self.entries
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&kw)
                    || e.description.to_lowercase().contains(&kw)
                    || e.content.to_lowercase().contains(&kw)
            })
            .collect()
    }

    /// Read the MEMORY.md index contents.
    pub fn read_index(&self) -> Option<String> {
        std::fs::read_to_string(&self.index_path).ok()
    }

    /// Render all memories as context for injection into the system prompt.
    pub fn render_context(&self, max_tokens: usize) -> String {
        let mut context = String::new();
        let mut token_estimate = 0;

        for entry in &self.entries {
            let section = format!(
                "## {} ({})\n{}\n\n",
                entry.name,
                serde_json::to_string(&entry.memory_type)
                    .unwrap_or_default()
                    .trim_matches('"'),
                entry.content,
            );

            let section_tokens = section.len() / 4; // rough estimate
            if token_estimate + section_tokens > max_tokens {
                break;
            }
            context.push_str(&section);
            token_estimate += section_tokens;
        }

        context
    }

    /// Update the MEMORY.md index file.
    fn update_index(&self) -> Result<()> {
        let mut index = String::new();
        for entry in &self.entries {
            index.push_str(&format!(
                "- [{}]({}) — {}\n",
                entry.name, entry.filename, entry.description,
            ));
        }
        std::fs::write(&self.index_path, index)?;
        Ok(())
    }

    /// Directory path.
    pub fn dir(&self) -> &Path {
        &self.memory_dir
    }
}

/// Parse YAML-style frontmatter from markdown content.
fn parse_frontmatter(content: &str) -> Result<(MemoryFrontmatter, String)> {
    let content = content.trim();
    if !content.starts_with("---") {
        anyhow::bail!("no frontmatter found");
    }

    let rest = &content[3..];
    let end = rest
        .find("---")
        .ok_or_else(|| anyhow::anyhow!("unterminated frontmatter"))?;

    let yaml = &rest[..end].trim();
    let body = rest[end + 3..].trim().to_string();

    // Simple key-value parsing (not full YAML)
    let mut name = String::new();
    let mut description = String::new();
    let mut memory_type = MemoryType::User;

    for line in yaml.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("type:") {
            memory_type = match val.trim() {
                "user" => MemoryType::User,
                "feedback" => MemoryType::Feedback,
                "project" => MemoryType::Project,
                "reference" => MemoryType::Reference,
                _ => MemoryType::User,
            };
        }
    }

    Ok((
        MemoryFrontmatter {
            name,
            description,
            memory_type,
        },
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, MemoryManager) {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("memory");
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = MemoryManager::new(&dir);
        (tmp, mgr)
    }

    #[test]
    fn test_save_and_load() {
        let (_tmp, mut mgr) = setup();
        let entry = MemoryEntry {
            name: "User Role".into(),
            description: "User is a Rust developer".into(),
            memory_type: MemoryType::User,
            content: "Zhuoran is a senior engineer at Cisco.".into(),
            filename: "user_role.md".into(),
        };
        mgr.save(&entry).unwrap();

        // Reload
        let mut mgr2 = MemoryManager::new(mgr.dir());
        mgr2.load().unwrap();
        assert_eq!(mgr2.entries().len(), 1);
        assert_eq!(mgr2.entries()[0].name, "User Role");
        assert_eq!(mgr2.entries()[0].memory_type, MemoryType::User);
    }

    #[test]
    fn test_remove() {
        let (_tmp, mut mgr) = setup();
        let entry = MemoryEntry {
            name: "Temp".into(),
            description: "temp".into(),
            memory_type: MemoryType::Feedback,
            content: "Remove me".into(),
            filename: "temp.md".into(),
        };
        mgr.save(&entry).unwrap();
        assert_eq!(mgr.entries().len(), 1);

        mgr.remove("temp.md").unwrap();
        assert!(mgr.entries().is_empty());
    }

    #[test]
    fn test_entries_by_type() {
        let (_tmp, mut mgr) = setup();
        mgr.save(&MemoryEntry {
            name: "A".into(),
            description: "a".into(),
            memory_type: MemoryType::User,
            content: "user".into(),
            filename: "a.md".into(),
        })
        .unwrap();
        mgr.save(&MemoryEntry {
            name: "B".into(),
            description: "b".into(),
            memory_type: MemoryType::Project,
            content: "project".into(),
            filename: "b.md".into(),
        })
        .unwrap();

        assert_eq!(mgr.entries_by_type(&MemoryType::User).len(), 1);
        assert_eq!(mgr.entries_by_type(&MemoryType::Project).len(), 1);
        assert_eq!(mgr.entries_by_type(&MemoryType::Feedback).len(), 0);
    }

    #[test]
    fn test_search() {
        let (_tmp, mut mgr) = setup();
        mgr.save(&MemoryEntry {
            name: "Rust Preferences".into(),
            description: "Pure Rust, no Python".into(),
            memory_type: MemoryType::Feedback,
            content: "User wants pure Rust solutions".into(),
            filename: "rust.md".into(),
        })
        .unwrap();

        assert_eq!(mgr.search("rust").len(), 1);
        assert_eq!(mgr.search("python").len(), 1); // in description
        assert_eq!(mgr.search("java").len(), 0);
    }

    #[test]
    fn test_index_updated() {
        let (_tmp, mut mgr) = setup();
        mgr.save(&MemoryEntry {
            name: "Test".into(),
            description: "A test memory".into(),
            memory_type: MemoryType::User,
            content: "Content".into(),
            filename: "test.md".into(),
        })
        .unwrap();

        let index = mgr.read_index().unwrap();
        assert!(index.contains("[Test](test.md)"));
        assert!(index.contains("A test memory"));
    }

    #[test]
    fn test_render_context() {
        let (_tmp, mut mgr) = setup();
        mgr.save(&MemoryEntry {
            name: "Memory One".into(),
            description: "first".into(),
            memory_type: MemoryType::User,
            content: "Content of memory one".into(),
            filename: "one.md".into(),
        })
        .unwrap();

        let ctx = mgr.render_context(4096);
        assert!(ctx.contains("Memory One"));
        assert!(ctx.contains("Content of memory one"));
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nname: Test\ndescription: A test\ntype: feedback\n---\n\nBody content here.";
        let (fm, body) = parse_frontmatter(content).unwrap();
        assert_eq!(fm.name, "Test");
        assert_eq!(fm.description, "A test");
        assert_eq!(fm.memory_type, MemoryType::Feedback);
        assert_eq!(body, "Body content here.");
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        assert!(parse_frontmatter("No frontmatter here").is_err());
    }

    #[test]
    fn test_for_project() {
        let mgr = MemoryManager::for_project("/tmp/myproject");
        assert!(mgr.dir().to_string_lossy().contains(".cisco-code/memory"));
    }

    #[test]
    fn test_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut mgr = MemoryManager::new(&tmp.path().join("nonexistent"));
        mgr.load().unwrap();
        assert!(mgr.entries().is_empty());
    }
}

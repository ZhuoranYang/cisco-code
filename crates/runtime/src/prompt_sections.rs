//! Dynamic prompt section registry with memoization.
//!
//! Design insight from Claude Code: The system prompt is assembled from ~15
//! named sections. Some are static (core identity, tool guidelines) and can be
//! cached for the entire session. Others are dynamic (git status, date, todos)
//! and must be recomputed each turn. A few are expensive (instruction loading,
//! skill discovery) but only change on specific events.
//!
//! The `PromptSectionRegistry` allows:
//! - Named section registration with a compute function
//! - Per-section memoization (computed once, reused until invalidated)
//! - Targeted invalidation (e.g., invalidate "memory" after a memory write)
//! - Cache boundary split (static sections get prompt cache control)

use std::collections::HashMap;

/// A named section in the system prompt.
#[derive(Debug, Clone)]
pub struct PromptSection {
    /// Unique identifier for this section (e.g., "core", "environment", "memory").
    pub name: String,
    /// The rendered content of this section.
    pub content: String,
    /// Whether this section's content is stable across turns and should go
    /// before the cache boundary (leveraging Anthropic's prompt caching).
    pub cacheable: bool,
}

/// Registry that manages named prompt sections with optional memoization.
///
/// Sections are rendered in registration order. Each section can be:
/// - **Static**: registered once with fixed content (always memoized).
/// - **Dynamic**: recomputed every turn (never memoized).
/// - **Memoized**: computed once, then cached until explicitly invalidated.
///
/// Usage:
/// ```ignore
/// let mut registry = PromptSectionRegistry::new();
/// registry.register_static("core", "You are Cisco Code...", true);
/// registry.register_memoized("instructions", || load_project_instructions("."), true);
/// registry.register_dynamic("environment", || detect_environment());
/// // ... later, after a memory write:
/// registry.invalidate("memory");
/// ```
pub struct PromptSectionRegistry {
    /// Section definitions in registration order.
    sections: Vec<SectionDef>,
    /// Memoized section content, keyed by section name.
    cache: HashMap<String, String>,
}

/// Internal definition of a section.
struct SectionDef {
    name: String,
    kind: SectionKind,
    cacheable: bool,
}

enum SectionKind {
    /// Fixed content that never changes.
    Static(String),
    /// Content computed by a closure, cached until invalidated.
    Memoized(Box<dyn Fn() -> String + Send + Sync>),
    /// Content recomputed every call to `build()`.
    Dynamic(Box<dyn Fn() -> String + Send + Sync>),
}

impl PromptSectionRegistry {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
            cache: HashMap::new(),
        }
    }

    /// Register a section with fixed content (always cached).
    pub fn register_static(&mut self, name: &str, content: impl Into<String>, cacheable: bool) {
        let content = content.into();
        self.cache.insert(name.to_string(), content.clone());
        self.sections.push(SectionDef {
            name: name.to_string(),
            kind: SectionKind::Static(content),
            cacheable,
        });
    }

    /// Register a section computed by a closure, memoized until `invalidate()`.
    pub fn register_memoized<F>(&mut self, name: &str, compute: F, cacheable: bool)
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.sections.push(SectionDef {
            name: name.to_string(),
            kind: SectionKind::Memoized(Box::new(compute)),
            cacheable,
        });
    }

    /// Register a section recomputed every turn (never memoized).
    pub fn register_dynamic<F>(&mut self, name: &str, compute: F)
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.sections.push(SectionDef {
            name: name.to_string(),
            kind: SectionKind::Dynamic(Box::new(compute)),
            cacheable: false,
        });
    }

    /// Invalidate a memoized section, forcing recomputation on next `build()`.
    pub fn invalidate(&mut self, name: &str) {
        self.cache.remove(name);
    }

    /// Invalidate all memoized sections.
    pub fn invalidate_all(&mut self) {
        self.cache.clear();
        // Re-populate static sections (they never change).
        // Collect into a temp vec to avoid borrowing &self.sections and &mut self.cache simultaneously.
        let statics: Vec<(String, String)> = self
            .sections
            .iter()
            .filter_map(|s| {
                if let SectionKind::Static(ref content) = s.kind {
                    Some((s.name.clone(), content.clone()))
                } else {
                    None
                }
            })
            .collect();
        for (name, content) in statics {
            self.cache.insert(name, content);
        }
    }

    /// Check if a section exists.
    pub fn has_section(&self, name: &str) -> bool {
        self.sections.iter().any(|s| s.name == name)
    }

    /// Get the number of registered sections.
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Get names of all registered sections in order.
    pub fn section_names(&self) -> Vec<&str> {
        self.sections.iter().map(|s| s.name.as_str()).collect()
    }

    /// Build the full prompt, respecting memoization.
    ///
    /// Returns `(static_sections, dynamic_sections)` — the caller can
    /// use the split for cache control (static gets `cache_control: ephemeral`).
    pub fn build(&mut self) -> (String, String) {
        let mut static_parts = Vec::new();
        let mut dynamic_parts = Vec::new();
        // Collect newly-computed memoized results to insert into cache after
        // the iteration (avoids borrowing &self.sections and &mut self.cache
        // simultaneously, and ensures each closure is called exactly once).
        let mut to_cache: Vec<(String, String)> = Vec::new();

        for section in &self.sections {
            let content = match &section.kind {
                SectionKind::Static(content) => content.clone(),
                SectionKind::Memoized(compute) => {
                    if let Some(cached) = self.cache.get(&section.name) {
                        cached.clone()
                    } else {
                        let result = compute();
                        if !result.is_empty() {
                            to_cache.push((section.name.clone(), result.clone()));
                        }
                        result
                    }
                }
                SectionKind::Dynamic(compute) => compute(),
            };

            if content.is_empty() {
                continue;
            }

            if section.cacheable {
                static_parts.push(content);
            } else {
                dynamic_parts.push(content);
            }
        }

        // Insert newly-computed memoized results into the cache.
        for (name, value) in to_cache {
            self.cache.insert(name, value);
        }

        (static_parts.join("\n\n"), dynamic_parts.join("\n\n"))
    }

    /// Build the full prompt as a single string (for backward compatibility).
    pub fn build_full(&mut self) -> String {
        let (static_part, dynamic_part) = self.build();
        if dynamic_part.is_empty() {
            static_part
        } else {
            format!("{static_part}\n\n{dynamic_part}")
        }
    }
}

impl Default for PromptSectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_registry() {
        let mut reg = PromptSectionRegistry::new();
        let (s, d) = reg.build();
        assert!(s.is_empty());
        assert!(d.is_empty());
    }

    #[test]
    fn test_static_sections() {
        let mut reg = PromptSectionRegistry::new();
        reg.register_static("core", "You are an AI assistant.", true);
        reg.register_static("tools", "Use tools carefully.", true);

        let (s, d) = reg.build();
        assert!(s.contains("You are an AI assistant."));
        assert!(s.contains("Use tools carefully."));
        assert!(d.is_empty());
    }

    #[test]
    fn test_dynamic_sections() {
        let mut reg = PromptSectionRegistry::new();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        reg.register_dynamic("date", move || {
            let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            format!("Call #{n}")
        });

        let (_, d1) = reg.build();
        assert!(d1.contains("Call #0"));

        let (_, d2) = reg.build();
        assert!(d2.contains("Call #1")); // recomputed each time
    }

    #[test]
    fn test_memoized_section() {
        let mut reg = PromptSectionRegistry::new();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        reg.register_memoized(
            "instructions",
            move || {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                "Project instructions here.".to_string()
            },
            true,
        );

        let (s1, _) = reg.build();
        assert!(s1.contains("Project instructions here."));
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Second build should use cache — counter stays at 1
        let (s2, _) = reg.build();
        assert!(s2.contains("Project instructions here."));
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn test_invalidate_forces_recompute() {
        let mut reg = PromptSectionRegistry::new();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        reg.register_memoized(
            "memory",
            move || {
                let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                format!("Memory v{n}")
            },
            false,
        );

        let (_, d1) = reg.build();
        assert!(d1.contains("Memory v0"));

        // Invalidate and rebuild — should recompute
        reg.invalidate("memory");
        let (_, d2) = reg.build();
        assert!(d2.contains("Memory v1"));

        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn test_static_dynamic_ordering() {
        let mut reg = PromptSectionRegistry::new();
        reg.register_static("core", "# Core", true);
        reg.register_dynamic("env", || "# Environment".to_string());
        reg.register_static("tools", "# Tools", true);
        reg.register_dynamic("date", || "# Date".to_string());

        let (s, d) = reg.build();
        // Static sections grouped together
        assert!(s.contains("# Core"));
        assert!(s.contains("# Tools"));
        // Dynamic sections grouped together
        assert!(d.contains("# Environment"));
        assert!(d.contains("# Date"));
    }

    #[test]
    fn test_empty_sections_skipped() {
        let mut reg = PromptSectionRegistry::new();
        reg.register_static("core", "Content", true);
        reg.register_dynamic("empty", || String::new());

        let (s, d) = reg.build();
        assert!(s.contains("Content"));
        assert!(d.is_empty()); // empty dynamic section skipped
    }

    #[test]
    fn test_section_names() {
        let mut reg = PromptSectionRegistry::new();
        reg.register_static("a", "A", true);
        reg.register_dynamic("b", || "B".to_string());
        reg.register_memoized("c", || "C".to_string(), false);

        assert_eq!(reg.section_names(), vec!["a", "b", "c"]);
        assert_eq!(reg.section_count(), 3);
        assert!(reg.has_section("a"));
        assert!(!reg.has_section("z"));
    }

    #[test]
    fn test_invalidate_all() {
        let mut reg = PromptSectionRegistry::new();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        reg.register_static("core", "static", true);
        reg.register_memoized(
            "memo",
            move || {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                "memoized".to_string()
            },
            false,
        );

        reg.build(); // compute memo once
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        reg.invalidate_all();
        reg.build(); // should recompute memo
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn test_build_full() {
        let mut reg = PromptSectionRegistry::new();
        reg.register_static("core", "Static part", true);
        reg.register_dynamic("env", || "Dynamic part".to_string());

        let full = reg.build_full();
        assert!(full.contains("Static part"));
        assert!(full.contains("Dynamic part"));
    }

    #[test]
    fn test_default() {
        let reg = PromptSectionRegistry::default();
        assert_eq!(reg.section_count(), 0);
    }
}

//! `.litignore` handling (SPEC §1.2).
//!
//! Gitignore pattern semantics; a subdirectory's `.litignore`
//! applies to its own subtree. The `.lit/` directory itself is
//! implicitly ignored by the working-tree scanner, not by rules
//! here. Minimal v1 parity (basic globs, comments, blank lines,
//! negation, directory-only, anchoring, subdirectory files); full
//! edge-case parity is deferred per SPEC §1.2.

use crate::glob::{compile, match_path, Rule};

struct RuleSet {
    base: String,
    rules: Vec<Rule>,
}

#[derive(Default)]
pub struct Ignore {
    sets: Vec<RuleSet>,
}

impl Ignore {
    pub fn new() -> Self {
        Ignore::default()
    }

    /// Registers one `.litignore` file. `base` is the
    /// repo-relative directory holding it (`""` for the root).
    /// Files must be added root-first, ancestors before
    /// descendants: later rules override earlier ones (last match
    /// wins), which realizes gitignore's deeper-file precedence.
    pub fn add_file(&mut self, base: &str, content: &str) {
        let mut rules = Vec::new();
        for line in content.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if let Some(rule) = compile(line) {
                rules.push(rule);
            }
        }
        if !rules.is_empty() {
            self.sets.push(RuleSet {
                base: base.to_string(),
                rules,
            });
        }
    }

    /// Last matching rule across all applicable sets decides;
    /// negation re-includes. The working-tree walker prunes
    /// ignored directories, which realizes gitignore's "cannot
    /// re-include under an excluded directory".
    pub fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for set in &self.sets {
            let sub = if set.base.is_empty() {
                rel
            } else {
                match rel
                    .strip_prefix(set.base.as_str())
                    .and_then(|r| r.strip_prefix('/'))
                {
                    Some(r) if !r.is_empty() => r,
                    _ => continue,
                }
            };
            for rule in &set.rules {
                if match_path(rule, sub, is_dir) {
                    ignored = !rule.negated;
                }
            }
        }
        ignored
    }
}

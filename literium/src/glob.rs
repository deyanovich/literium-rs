//! Gitignore-style pattern matching (SPEC §1.1–§1.2).
//!
//! Compiles `.gitignore`-shaped patterns — comments, blank lines,
//! `*` / `**` / `?` / `[...]` globs, trailing `/` for
//! directory-only, leading or interior `/` for anchoring, `!` for
//! negation, backslash escapes — into matchers over repo-relative
//! slash-separated paths. Shared by the ignore and attributes
//! layers.

use regex::Regex;

/// One compiled pattern. The regex matches a slash-separated path
/// relative to the pattern's base directory (the directory of the
/// file the pattern came from), with no leading or trailing slash.
#[derive(Clone, Debug)]
pub struct Rule {
    pub negated: bool,
    pub dir_only: bool,
    pub re: Regex,
}

/// Compiles one pattern line. `None` for blank/comment lines.
pub fn compile(pattern: &str) -> Option<Rule> {
    // Trailing unescaped spaces are ignored.
    let mut pat: String = pattern.to_string();
    loop {
        let mut chars = pat.chars().rev();
        match (chars.next(), chars.next()) {
            (Some(' '), Some('\\')) => break,
            (Some(' '), _) => {
                pat.pop();
            }
            _ => break,
        }
    }
    if pat.is_empty() || pat.starts_with('#') {
        return None;
    }

    let negated = if let Some(rest) = pat.strip_prefix('!') {
        pat = rest.to_string();
        true
    } else {
        false
    };
    let dir_only = if let Some(rest) = pat.strip_suffix('/') {
        pat = rest.to_string();
        true
    } else {
        false
    };

    // A pattern with a slash anywhere (after the trailing-slash
    // strip) is anchored to the base directory; otherwise it
    // matches at any depth.
    let anchored = pat.contains('/');
    let pat = pat.strip_prefix('/').unwrap_or(&pat);

    let body = translate(pat);
    let full = if anchored {
        format!("^{body}\\z")
    } else {
        format!("(?:^|/){body}\\z")
    };
    let re = Regex::new(&full).ok()?;
    Some(Rule {
        negated,
        dir_only,
        re,
    })
}

fn translate(p: &str) -> String {
    let mut re = String::new();
    let mut rest = p;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("**/") {
            re.push_str("(?:[^/]+/)*");
            rest = r;
        } else if rest == "**" {
            re.push_str(".*");
            rest = "";
        } else if let Some(r) = rest.strip_prefix("**") {
            re.push_str("[^/]*"); // interior ** = *
            rest = r;
        } else if let Some(r) = rest.strip_prefix('*') {
            re.push_str("[^/]*");
            rest = r;
        } else if let Some(r) = rest.strip_prefix('?') {
            re.push_str("[^/]");
            rest = r;
        } else if let Some(r) = rest.strip_prefix('\\') {
            let mut cs = r.chars();
            if let Some(c) = cs.next() {
                re.push_str(&regex::escape(&c.to_string()));
                rest = cs.as_str();
            } else {
                rest = r; // trailing lone backslash: drop, like quotemeta of nothing
            }
        } else if let Some(r) = rest.strip_prefix('[') {
            let mut cls = String::new();
            let mut r = r;
            if let Some(x) = r.strip_prefix('!') {
                cls.push('^');
                r = x;
            }
            if let Some(x) = r.strip_prefix(']') {
                cls.push_str("\\]");
                r = x;
            }
            while !r.is_empty() && !r.starts_with(']') {
                if let Some(x) = r.strip_prefix('\\') {
                    let mut cs = x.chars();
                    if let Some(c) = cs.next() {
                        cls.push_str(&regex::escape(&c.to_string()));
                        r = cs.as_str();
                    } else {
                        r = x;
                    }
                } else {
                    let mut cs = r.chars();
                    let c = cs.next().unwrap();
                    if c.is_alphanumeric() || c == '_' || c == '-' {
                        cls.push(c);
                    } else {
                        cls.push_str(&regex::escape(&c.to_string()));
                    }
                    r = cs.as_str();
                }
            }
            r = r.strip_prefix(']').unwrap_or(r);
            re.push('[');
            re.push_str(&cls);
            re.push(']');
            rest = r;
        } else {
            let mut cs = rest.chars();
            let c = cs.next().unwrap();
            re.push_str(&regex::escape(&c.to_string()));
            rest = cs.as_str();
        }
    }
    re
}

/// Direct match of one compiled rule against one path.
pub fn match_path(rule: &Rule, rel: &str, is_dir: bool) -> bool {
    if rule.dir_only && !is_dir {
        return false;
    }
    rule.re.is_match(rel)
}

/// Gitignore semantics for classifying a *file* without a
/// directory walk: a rule matching any ancestor directory covers
/// the file.
pub fn match_file_or_ancestor(rule: &Rule, rel: &str) -> bool {
    if match_path(rule, rel, false) {
        return true;
    }
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop();
    while !parts.is_empty() {
        if match_path(rule, &parts.join("/"), true) {
            return true;
        }
        parts.pop();
    }
    false
}

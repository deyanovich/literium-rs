// Port of literium-perl t/04-glob-ignore.t — SPEC §1.1–§1.2.

use literium::glob::{compile, match_file_or_ancestor, match_path};
use literium::ignore::Ignore;

#[test]
fn glob_compilation() {
    assert!(compile("").is_none(), "blank line ignored");
    assert!(compile("# c").is_none(), "comment ignored");
    assert!(compile("   ").is_none(), "whitespace-only ignored");

    let txt = compile("*.txt").unwrap();
    assert!(match_path(&txt, "a.txt", false), "*.txt matches at root");
    assert!(
        match_path(&txt, "x/y/a.txt", false),
        "unanchored matches at depth"
    );
    assert!(!match_path(&txt, "a.txt.bak", false), "no suffix overmatch");

    let anch = compile("/draft.md").unwrap();
    assert!(match_path(&anch, "draft.md", false), "leading / anchors");
    assert!(
        !match_path(&anch, "sub/draft.md", false),
        "anchored: no match at depth"
    );

    let mid = compile("docs/notes.md").unwrap();
    assert!(
        match_path(&mid, "docs/notes.md", false),
        "interior slash anchors"
    );
    assert!(
        !match_path(&mid, "x/docs/notes.md", false),
        "interior slash: not at depth"
    );

    let dir = compile("build/").unwrap();
    assert!(
        match_path(&dir, "build", true),
        "trailing / matches directory"
    );
    assert!(
        !match_path(&dir, "build", false),
        "trailing /: not a plain file"
    );
    assert!(
        match_path(&dir, "x/build", true),
        "dir-only unanchored at depth"
    );

    let dstar = compile("**/logs").unwrap();
    assert!(match_path(&dstar, "logs", false), "**/ matches zero dirs");
    assert!(match_path(&dstar, "a/b/logs", false), "**/ matches deep");

    let inner = compile("a/**/b").unwrap();
    assert!(match_path(&inner, "a/b", false), "a/**/b matches a/b");
    assert!(match_path(&inner, "a/x/y/b", false), "a/**/b matches deep");
    assert!(
        !match_path(&inner, "a/xb", false),
        "a/**/b needs whole segments"
    );

    let trail = compile("cache/**").unwrap();
    assert!(match_path(&trail, "cache/x", false), "trailing /** inside");
    assert!(match_path(&trail, "cache/x/y", false), "trailing /** deep");
    assert!(
        !match_path(&trail, "cache", false),
        "trailing /** not the dir"
    );

    let q = compile("ch?.md").unwrap();
    assert!(match_path(&q, "ch1.md", false), "? matches one char");
    assert!(!match_path(&q, "ch10.md", false), "? not two chars");

    let cls = compile("ch[0-9x].md").unwrap();
    assert!(match_path(&cls, "ch7.md", false), "char class range");
    assert!(match_path(&cls, "chx.md", false), "char class literal");
    assert!(!match_path(&cls, "chy.md", false), "char class excludes");

    let negcls = compile("ch[!0-9].md").unwrap();
    assert!(
        match_path(&negcls, "cha.md", false),
        "negated class matches"
    );
    assert!(
        !match_path(&negcls, "ch3.md", false),
        "negated class excludes"
    );

    let esc = compile("with\\ space.txt").unwrap();
    assert!(
        match_path(&esc, "with space.txt", false),
        "escaped space literal"
    );

    let neg = compile("!keep.md").unwrap();
    assert!(neg.negated, "negation flag");

    assert!(
        match_file_or_ancestor(&compile("build").unwrap(), "build/deep/file.txt"),
        "ancestor-dir match covers contained file"
    );
}

#[test]
fn ignore_stacking() {
    // SPEC §1.2.
    let mut ign = Ignore::new();
    ign.add_file(
        "",
        "*.log\n!important.log\nbuild/\n# comment\n\ndrafts/*.tmp\n",
    );

    assert!(ign.ignored("x.log", false), "root pattern ignores");
    assert!(ign.ignored("a/b/x.log", false), "unanchored at depth");
    assert!(!ign.ignored("important.log", false), "negation re-includes");
    assert!(ign.ignored("build", true), "directory-only ignores dir");
    assert!(!ign.ignored("build", false), "directory-only skips file");
    assert!(ign.ignored("drafts/a.tmp", false), "anchored subpath");
    assert!(
        !ign.ignored("deep/drafts/a.tmp", false),
        "anchored not at depth"
    );

    // Subdirectory .litignore applies to its subtree (SPEC §1.2).
    ign.add_file("notes", "scratch*\n!scratch-keep.md\n");
    assert!(
        ign.ignored("notes/scratch1.md", false),
        "subdir pattern applies within"
    );
    assert!(
        !ign.ignored("scratch1.md", false),
        "subdir pattern not outside"
    );
    assert!(
        !ign.ignored("notes/scratch-keep.md", false),
        "subdir negation wins"
    );

    // Deeper file overrides root (added later, last match wins).
    let mut ov = Ignore::new();
    ov.add_file("", "*.md\n");
    ov.add_file("pub", "!*.md\n");
    assert!(ov.ignored("x.md", false), "root ignores .md");
    assert!(
        !ov.ignored("pub/x.md", false),
        "deeper negation overrides root"
    );
}

# RAG Code-File Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the RAG indexer to walk and chunk `.rs` and `.allium` files in addition to `.md`, so `search_docs` is useful on the dispatch repo itself.

**Architecture:** All changes are confined to `src/service/repo_index.rs`. A new private helper `chunk_by_declarations` implements a two-buffer algorithm that keeps doc comments and attributes in the same chunk as the declaration they annotate. `walk_md_files` is replaced by `walk_indexable_files`; `chunk_file` is renamed `chunk_markdown`; a new `chunk_for_extension` dispatcher routes by file extension.

**Tech Stack:** Rust 2021, `ignore` crate (file walking, already in use), `rusqlite` (storage), `tokio` (async), `tempfile` (tests)

---

## File Map

| File | Change |
|---|---|
| `src/service/repo_index.rs` | All changes — new functions, renamed functions, updated callers |

No other files touched. All new test code goes into the `mod tests` block already at the bottom of the file.

---

## Task 1: Add `chunk_by_declarations` private helper

**Files:**
- Modify: `src/service/repo_index.rs`

The two-buffer algorithm: `main_current` holds non-attribute body lines; `attr_buffer` holds pending `#[`/`///` (or `-- ` for Allium) lines that will move into the *next* chunk when a declaration boundary arrives. This ensures doc comments and attributes land with the declaration they annotate, not with the preceding item.

- [ ] **Step 1.1: Write the failing tests**

Add the following block inside the `mod tests` section of `src/service/repo_index.rs`, after the existing `chunk_file` tests:

```rust
// --- chunk_by_declarations ---

#[test]
fn chunk_by_decls_empty_returns_no_chunks() {
    let result = chunk_by_declarations("", &["fn "], |_| false);
    assert!(result.is_empty());
}

#[test]
fn chunk_by_decls_whitespace_only_returns_no_chunks() {
    let result = chunk_by_declarations("   \n\n  ", &["fn "], |_| false);
    assert!(result.is_empty());
}

#[test]
fn chunk_by_decls_no_keyword_match_returns_single_chunk() {
    let content = "use std::fmt;\n\nsome text";
    let result = chunk_by_declarations(content, &["fn "], |_| false);
    assert_eq!(result.len(), 1);
    assert!(result[0].contains("some text"));
}

#[test]
fn chunk_by_decls_two_declarations_produce_two_chunks() {
    let content = "fn foo() {}\n\nfn bar() {}";
    let result = chunk_by_declarations(content, &["fn "], |_| false);
    assert_eq!(result.len(), 2);
    assert!(result[0].contains("fn foo()"), "got: {}", result[0]);
    assert!(result[1].contains("fn bar()"), "got: {}", result[1]);
}

#[test]
fn chunk_by_decls_preamble_merges_into_first_chunk() {
    let content = "use std::fmt;\n\nfn foo() {}";
    let result = chunk_by_declarations(content, &["fn "], |_| false);
    assert_eq!(result.len(), 1);
    assert!(result[0].contains("use std::fmt;"), "got: {}", result[0]);
    assert!(result[0].contains("fn foo()"), "got: {}", result[0]);
}

#[test]
fn chunk_by_decls_attr_before_second_decl_lands_in_new_chunk() {
    let content = "fn foo() {}\n\n#[attr]\nfn bar() {}";
    let result = chunk_by_declarations(content, &["fn "], |line| line.starts_with("#["));
    assert_eq!(result.len(), 2);
    assert!(
        !result[0].contains("#[attr]"),
        "attr should NOT be in foo chunk: {}",
        result[0]
    );
    assert!(
        result[1].contains("#[attr]"),
        "attr should be in bar chunk: {}",
        result[1]
    );
    assert!(result[1].contains("fn bar()"));
}

#[test]
fn chunk_by_decls_non_attr_line_flushes_attr_buffer_into_body() {
    // An attr line followed by a non-attr non-boundary line gets absorbed into the body.
    let content = "fn foo() {}\n#[orphan]\nbody line\nfn bar() {}";
    let result = chunk_by_declarations(content, &["fn "], |line| line.starts_with("#["));
    assert_eq!(result.len(), 2);
    assert!(
        result[0].contains("#[orphan]"),
        "orphaned attr flushed into foo chunk: {}",
        result[0]
    );
    assert!(result[1].contains("fn bar()"));
}

#[test]
fn chunk_by_decls_pub_prefix_triggers_split() {
    let result = chunk_by_declarations(
        "pub fn foo() {}\n\npub fn bar() {}",
        &["fn "],
        |_| false,
    );
    assert_eq!(result.len(), 2);
}

#[test]
fn chunk_by_decls_pub_crate_prefix_triggers_split() {
    let result = chunk_by_declarations(
        "pub(crate) fn foo() {}\n\npub(crate) fn bar() {}",
        &["fn "],
        |_| false,
    );
    assert_eq!(result.len(), 2);
}

#[test]
fn chunk_by_decls_async_prefix_triggers_split() {
    let result = chunk_by_declarations(
        "async fn foo() {}\n\nasync fn bar() {}",
        &["fn "],
        |_| false,
    );
    assert_eq!(result.len(), 2);
}
```

- [ ] **Step 1.2: Run to verify compilation failure**

```bash
cargo test "repo_index::tests::chunk_by_decls" 2>&1 | head -15
```

Expected: compile error — `chunk_by_declarations` not found.

- [ ] **Step 1.3: Implement `strip_vis`, `is_decl_boundary`, `chunk_by_declarations`**

Insert these three private functions in `src/service/repo_index.rs` just before the existing `fn split_frontmatter` (around line 35). They must be placed before `chunk_rust` and `chunk_allium` (added later) since those call `chunk_by_declarations`.

```rust
fn strip_vis(s: &str) -> &str {
    if let Some(r) = s.strip_prefix("pub(crate) ") {
        return r;
    }
    if let Some(r) = s.strip_prefix("pub(super) ") {
        return r;
    }
    if let Some(r) = s.strip_prefix("pub ") {
        return r;
    }
    s
}

fn is_decl_boundary(line: &str, keywords: &[&str]) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') || line.is_empty() {
        return false;
    }
    let s = strip_vis(line);
    let s = s.strip_prefix("async ").unwrap_or(s);
    let s = s.strip_prefix("unsafe ").unwrap_or(s);
    keywords.iter().any(|kw| s.starts_with(kw))
}

fn chunk_by_declarations(
    content: &str,
    decl_keywords: &[&str],
    is_attr_line: impl Fn(&str) -> bool,
) -> Vec<String> {
    if content.trim().is_empty() {
        return vec![];
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut main_current = String::new();
    let mut attr_buffer = String::new();
    let mut has_seen_decl = false;

    for line in content.lines() {
        if is_decl_boundary(line, decl_keywords) {
            if has_seen_decl && !main_current.trim().is_empty() {
                chunks.push(main_current.trim_end().to_string());
                main_current = format!("{attr_buffer}{line}\n");
            } else {
                // First declaration: absorb preamble + pending attrs into this chunk.
                main_current.push_str(&attr_buffer);
                main_current.push_str(line);
                main_current.push('\n');
            }
            attr_buffer.clear();
            has_seen_decl = true;
        } else if is_attr_line(line) {
            attr_buffer.push_str(line);
            attr_buffer.push('\n');
        } else {
            // Non-boundary, non-attr: flush pending attrs into body.
            main_current.push_str(&attr_buffer);
            attr_buffer.clear();
            main_current.push_str(line);
            main_current.push('\n');
        }
    }

    let remaining = format!("{main_current}{attr_buffer}");
    if !remaining.trim().is_empty() {
        chunks.push(remaining.trim_end().to_string());
    }

    if chunks.is_empty() {
        vec![content.trim().to_string()]
    } else {
        chunks
    }
}
```

- [ ] **Step 1.4: Run the new tests**

```bash
cargo test "repo_index::tests::chunk_by_decls" 2>&1 | tail -15
```

Expected: all 9 `chunk_by_decls_*` tests pass. No regressions in other tests.

- [ ] **Step 1.5: Commit**

```bash
git add src/service/repo_index.rs
git commit -m "feat(rag): add chunk_by_declarations two-buffer helper"
```

---

## Task 2: Add `chunk_rust` and `chunk_allium`

**Files:**
- Modify: `src/service/repo_index.rs`

`chunk_rust` splits Rust source at top-level declarations (`fn`, `impl`, `struct`, `enum`, `trait`, `type`, `mod`, `const`, `static`) after stripping visibility/modifier prefixes. `chunk_allium` does the same for Allium blocks (`entity`, `rule`, `surface`, `config`, `enum`, `concept`, `external`, `invariant`). Both preserve doc comments/attributes in the correct chunk via `chunk_by_declarations`.

- [ ] **Step 2.1: Write the failing tests**

Add after the `chunk_by_decls` tests in `mod tests`:

```rust
// --- chunk_rust ---

#[test]
fn chunk_rust_fn_at_col0_splits() {
    let chunks = chunk_rust("fn foo() {}\n\nfn bar() {}");
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].contains("fn foo()"));
    assert!(chunks[1].contains("fn bar()"));
}

#[test]
fn chunk_rust_pub_fn_splits() {
    let chunks = chunk_rust("pub fn foo() {}\n\npub fn bar() {}");
    assert_eq!(chunks.len(), 2);
}

#[test]
fn chunk_rust_async_fn_splits() {
    let chunks = chunk_rust("async fn foo() {}\n\nfn bar() {}");
    assert_eq!(chunks.len(), 2);
}

#[test]
fn chunk_rust_pub_async_fn_splits() {
    let chunks = chunk_rust("pub async fn foo() {}\n\nfn bar() {}");
    assert_eq!(chunks.len(), 2);
}

#[test]
fn chunk_rust_indented_fn_does_not_split() {
    let content = "impl Foo {\n    fn method(&self) {}\n    fn other(&self) {}\n}";
    let chunks = chunk_rust(content);
    assert_eq!(chunks.len(), 1, "indented fn should not split: {:?}", chunks);
}

#[test]
fn chunk_rust_impl_splits() {
    let content = "struct Foo {}\n\nimpl Foo {\n    fn foo(&self) {}\n}";
    let chunks = chunk_rust(content);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].contains("struct Foo"));
    assert!(chunks[1].contains("impl Foo"));
}

#[test]
fn chunk_rust_impl_generic_splits() {
    let content = "struct Foo<T>(T);\n\nimpl<T: std::fmt::Debug> Foo<T> {\n    fn foo(&self) {}\n}";
    let chunks = chunk_rust(content);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[1].contains("impl<T:"), "got: {}", chunks[1]);
}

#[test]
fn chunk_rust_derive_attr_stays_with_item() {
    let content = "struct Foo { x: i32 }\n\n#[derive(Debug)]\npub enum Bar { A }";
    let chunks = chunk_rust(content);
    assert_eq!(chunks.len(), 2);
    assert!(
        !chunks[0].contains("#[derive"),
        "derive must not be in Foo chunk: {}",
        chunks[0]
    );
    assert!(
        chunks[1].contains("#[derive"),
        "derive must be in Bar chunk: {}",
        chunks[1]
    );
    assert!(chunks[1].contains("pub enum Bar"));
}

#[test]
fn chunk_rust_doc_comment_stays_with_fn() {
    let content = "fn foo() {}\n\n/// Does bar.\nfn bar() {}";
    let chunks = chunk_rust(content);
    assert_eq!(chunks.len(), 2);
    assert!(
        !chunks[0].contains("/// Does bar."),
        "doc must not be in foo chunk: {}",
        chunks[0]
    );
    assert!(
        chunks[1].contains("/// Does bar."),
        "doc must be in bar chunk: {}",
        chunks[1]
    );
}

// --- chunk_allium ---

#[test]
fn chunk_allium_entity_and_rule_produce_two_chunks() {
    let content = "entity Task {\n    id: TaskId\n}\n\nrule CreateTask {\n    when: foo\n}";
    let chunks = chunk_allium(content);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].contains("entity Task"));
    assert!(chunks[1].contains("rule CreateTask"));
}

#[test]
fn chunk_allium_section_comment_stays_with_rule() {
    let content = "entity Task {}\n\n-- == Creation ==\n\nrule CreateTask {}";
    let chunks = chunk_allium(content);
    assert_eq!(chunks.len(), 2);
    assert!(
        !chunks[0].contains("-- == Creation =="),
        "comment should not be in entity chunk: {}",
        chunks[0]
    );
    assert!(
        chunks[1].contains("-- == Creation =="),
        "comment should be in rule chunk: {}",
        chunks[1]
    );
}

#[test]
fn chunk_allium_config_block_splits() {
    let content = "entity Task {}\n\nconfig {\n    key: value\n}";
    let chunks = chunk_allium(content);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[1].contains("config {"));
}

#[test]
fn chunk_allium_enum_splits() {
    let content = "entity Task {}\n\nenum Status { active | done }";
    let chunks = chunk_allium(content);
    assert_eq!(chunks.len(), 2);
    assert!(chunks[1].contains("enum Status"));
}
```

- [ ] **Step 2.2: Run to verify compilation failure**

```bash
cargo test "repo_index::tests::chunk_rust\|repo_index::tests::chunk_allium" 2>&1 | head -10
```

Expected: compile error — `chunk_rust` and `chunk_allium` not found.

- [ ] **Step 2.3: Implement `chunk_rust` and `chunk_allium`**

Add these two functions in `src/service/repo_index.rs` immediately after `chunk_by_declarations` and before `split_frontmatter`:

```rust
pub(crate) fn chunk_rust(content: &str) -> Vec<String> {
    const RUST_KEYWORDS: &[&str] = &[
        "fn ", "impl ", "impl<", "struct ", "enum ", "trait ",
        "type ", "mod ", "const ", "static ",
    ];
    chunk_by_declarations(content, RUST_KEYWORDS, |line| {
        line.starts_with("#[") || line.starts_with("///")
    })
}

pub(crate) fn chunk_allium(content: &str) -> Vec<String> {
    const ALLIUM_KEYWORDS: &[&str] = &[
        "entity ", "rule ", "surface ", "config", "enum ",
        "concept ", "external ", "invariant ",
    ];
    chunk_by_declarations(content, ALLIUM_KEYWORDS, |line| line.starts_with("-- "))
}
```

- [ ] **Step 2.4: Run the tests**

```bash
cargo test "repo_index::tests::chunk_rust\|repo_index::tests::chunk_allium" 2>&1 | tail -20
```

Expected: all `chunk_rust_*` and `chunk_allium_*` tests pass. No other regressions.

- [ ] **Step 2.5: Commit**

```bash
git add src/service/repo_index.rs
git commit -m "feat(rag): add chunk_rust and chunk_allium chunkers"
```

---

## Task 3: Add `chunk_for_extension` dispatcher + rename `chunk_file` → `chunk_markdown`

**Files:**
- Modify: `src/service/repo_index.rs`

Rename `chunk_file` to `chunk_markdown` everywhere in the file (the function is only used inside `repo_index.rs`), then add `chunk_for_extension` which dispatches to the right chunker by extension.

- [ ] **Step 3.1: Write the failing tests for `chunk_for_extension`**

Add to `mod tests`:

```rust
// --- chunk_for_extension ---

#[test]
fn chunk_for_extension_md_uses_h2_splitting() {
    let content = "## Section A\n\nText A.\n\n## Section B\n\nText B.";
    let chunks = chunk_for_extension(content, "md");
    assert_eq!(chunks.len(), 2);
    assert!(chunks[0].contains("Section A"));
    assert!(chunks[1].contains("Section B"));
}

#[test]
fn chunk_for_extension_rs_uses_declaration_splitting() {
    let chunks = chunk_for_extension("fn foo() {}\n\nfn bar() {}", "rs");
    assert_eq!(chunks.len(), 2);
}

#[test]
fn chunk_for_extension_allium_uses_allium_splitting() {
    let chunks = chunk_for_extension("entity Task {}\n\nrule CreateTask {}", "allium");
    assert_eq!(chunks.len(), 2);
}

#[test]
fn chunk_for_extension_unknown_ext_returns_single_chunk() {
    let chunks = chunk_for_extension("some content", "txt");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0], "some content");
}

#[test]
fn chunk_for_extension_unknown_ext_empty_returns_no_chunks() {
    let chunks = chunk_for_extension("", "txt");
    assert!(chunks.is_empty());
}
```

- [ ] **Step 3.2: Run to verify compilation failure**

```bash
cargo test "repo_index::tests::chunk_for_extension" 2>&1 | head -10
```

Expected: compile error — `chunk_for_extension` not found.

- [ ] **Step 3.3: Rename `chunk_file` → `chunk_markdown` in the file**

Use the Edit tool with `replace_all: true` on `src/service/repo_index.rs`:
- old_string: `chunk_file`
- new_string: `chunk_markdown`

This renames:
- The function declaration (`pub(crate) fn chunk_markdown`)
- The call in `index_repo` (temporarily still passes full content, fixed in Task 5)
- All test function names (`chunk_markdown_no_h2_returns_single_chunk`, etc.)
- All calls inside those tests (`chunk_markdown(content)`)

- [ ] **Step 3.4: Add `chunk_for_extension` after `chunk_markdown`**

Insert this function immediately after the closing brace of `chunk_markdown`:

```rust
pub(crate) fn chunk_for_extension(content: &str, ext: &str) -> Vec<String> {
    match ext {
        "md" => chunk_markdown(content),
        "rs" => chunk_rust(content),
        "allium" => chunk_allium(content),
        _ => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                vec![]
            } else {
                vec![trimmed.to_string()]
            }
        }
    }
}
```

- [ ] **Step 3.5: Run all tests in the module**

```bash
cargo test repo_index::tests 2>&1 | tail -20
```

Expected: all `chunk_markdown_*`, `chunk_for_extension_*`, `chunk_rust_*`, `chunk_allium_*`, `chunk_by_decls_*` tests pass. Total should be ~35+ tests in the block.

- [ ] **Step 3.6: Commit**

```bash
git add src/service/repo_index.rs
git commit -m "feat(rag): add chunk_for_extension dispatcher, rename chunk_file to chunk_markdown"
```

---

## Task 4: Replace `walk_md_files` with `walk_indexable_files`

**Files:**
- Modify: `src/service/repo_index.rs`

Add a constant `INDEXABLE_EXTENSIONS` and replace the `.md`-only walker with a walker that covers all three types.

- [ ] **Step 4.1: Write the failing tests**

Add to `mod tests`, after the `walk_md_files` tests (which are around line 482):

```rust
// --- walk_indexable_files ---

#[test]
fn walk_indexable_finds_md_rs_and_allium() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.md"), "# Note").unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
    std::fs::write(dir.path().join("spec.allium"), "entity Task {}").unwrap();
    std::fs::write(dir.path().join("config.txt"), "text").unwrap();
    let found = walk_indexable_files(dir.path()).unwrap();
    assert_eq!(found.len(), 3, "should find .md, .rs, .allium but not .txt");
}

#[test]
fn walk_indexable_skips_dispatch_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".dispatch")).unwrap();
    std::fs::write(dir.path().join(".dispatch").join("ignored.rs"), "fn x() {}").unwrap();
    std::fs::write(dir.path().join("real.rs"), "fn y() {}").unwrap();
    let found = walk_indexable_files(dir.path()).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].ends_with("real.rs"));
}

#[test]
fn walk_indexable_ignores_unsupported_extensions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("script.py"), "def foo(): pass").unwrap();
    std::fs::write(dir.path().join("data.json"), "{}").unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn foo() {}").unwrap();
    let found = walk_indexable_files(dir.path()).unwrap();
    assert_eq!(found.len(), 1, "only .rs should be found");
}
```

- [ ] **Step 4.2: Run to verify failure**

```bash
cargo test "repo_index::tests::walk_indexable" 2>&1 | head -10
```

Expected: compile error — `walk_indexable_files` not found.

- [ ] **Step 4.3: Add `INDEXABLE_EXTENSIONS` and `walk_indexable_files`**

In `src/service/repo_index.rs`, add the constant just before the `walk_md_files` function (~line 127), then add the new function after it:

```rust
const INDEXABLE_EXTENSIONS: &[&str] = &["md", "rs", "allium"];

fn walk_indexable_files(repo_path: &Path) -> Result<Vec<std::path::PathBuf>> {
    let dispatch_dir = repo_path.join(DISPATCH_DIR);
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(repo_path).hidden(false).build() {
        let entry = entry?;
        let path = entry.path();
        if path.starts_with(&dispatch_dir) {
            continue;
        }
        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if INDEXABLE_EXTENSIONS.contains(&ext) {
                files.push(path.to_owned());
            }
        }
    }
    Ok(files)
}
```

Keep `walk_md_files` in place for now — it is still referenced by two existing tests (`walk_md_finds_markdown_files` and `walk_md_skips_dispatch_dir`). Those tests will be removed in Task 5.

- [ ] **Step 4.4: Run the new tests**

```bash
cargo test "repo_index::tests::walk_indexable" 2>&1 | tail -15
```

Expected: all three `walk_indexable_*` tests pass.

- [ ] **Step 4.5: Commit**

```bash
git add src/service/repo_index.rs
git commit -m "feat(rag): add walk_indexable_files and INDEXABLE_EXTENSIONS"
```

---

## Task 5: Wire `index_repo` + integration tests

**Files:**
- Modify: `src/service/repo_index.rs`

Replace the two `walk_md_files` / `chunk_markdown` calls in `index_repo` with `walk_indexable_files` / `chunk_for_extension`. Remove the now-dead `walk_md_files` function and its tests.

- [ ] **Step 5.1: Write the failing integration tests**

Add to `mod tests` after the existing `index_repo_*` tests:

```rust
#[tokio::test]
async fn index_repo_indexes_rs_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "/// Adds two numbers.\npub fn add(a: i32, b: i32) -> i32 { a + b }\n\npub fn sub(a: i32, b: i32) -> i32 { a - b }",
    )
    .unwrap();

    let svc = RepoIndexService::new(EmbeddingService::new_test());
    let result = svc.index_repo(dir.path()).await.unwrap();

    assert_eq!(result.files_indexed, 1);
    assert_eq!(result.files_skipped, 0);
    assert!(result.chunks_total >= 1);

    let conn = open_rag_db(dir.path()).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rag_chunks", [], |r| r.get(0))
        .unwrap();
    assert!(count >= 1);
}

#[tokio::test]
async fn index_repo_indexes_allium_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("spec.allium"),
        "entity Task {\n    id: TaskId\n}\n\nrule CreateTask {\n    when: foo\n}",
    )
    .unwrap();

    let svc = RepoIndexService::new(EmbeddingService::new_test());
    let result = svc.index_repo(dir.path()).await.unwrap();

    assert_eq!(result.files_indexed, 1);
    assert!(result.chunks_total >= 1);
}

#[tokio::test]
async fn search_docs_finds_results_in_rs_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "/// Computes the sum of two integers.\npub fn add(a: i32, b: i32) -> i32 { a + b }",
    )
    .unwrap();

    let svc = RepoIndexService::new(EmbeddingService::new_test());
    svc.index_repo(dir.path()).await.unwrap();

    let results = svc.search_docs(dir.path(), "add integers", 5).await.unwrap();
    assert!(!results.is_empty(), "expected at least one result");
    assert!(results[0].file_path.ends_with("lib.rs"));
}
```

- [ ] **Step 5.2: Run to verify the tests fail**

```bash
cargo test "index_repo_indexes_rs\|index_repo_indexes_allium\|search_docs_finds_results_in_rs" 2>&1 | head -20
```

Expected: tests compile but fail (`.rs` and `.allium` files are not yet indexed).

- [ ] **Step 5.3: Update `index_repo` — Phase 1: swap walker**

In `src/service/repo_index.rs`, inside `index_repo`'s Phase 1 `spawn_blocking` closure (~line 172), replace:

```rust
let on_disk = walk_md_files(&repo_path)?;
```

with:

```rust
let on_disk = walk_indexable_files(&repo_path)?;
```

- [ ] **Step 5.4: Update `index_repo` — Phase 2: swap chunker**

In the Phase 2 loop (~line 209), replace:

```rust
let content = tokio::fs::read_to_string(&path).await?;
let chunks = chunk_markdown(&content);
```

with:

```rust
let content = tokio::fs::read_to_string(&path).await?;
let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
let chunks = chunk_for_extension(&content, ext);
```

- [ ] **Step 5.5: Remove `walk_md_files` and its tests**

Delete the `fn walk_md_files` function body from `src/service/repo_index.rs`.

In `mod tests`, delete the two tests `walk_md_finds_markdown_files` and `walk_md_skips_dispatch_dir` — their coverage is now provided by the `walk_indexable_*` tests added in Task 4.

- [ ] **Step 5.6: Run the full test suite**

```bash
cargo test 2>&1 | tail -30
```

Expected: all tests pass, including the three new integration tests. Verify that the `walk_md_*` tests are gone and `walk_indexable_*` tests pass.

- [ ] **Step 5.7: Run clippy**

```bash
cargo clippy --all-targets -- -D warnings 2>&1 | tail -20
```

Expected: zero warnings.

- [ ] **Step 5.8: Commit**

```bash
git add src/service/repo_index.rs
git commit -m "feat(rag): wire index_repo to walk and chunk .rs and .allium files"
```

---

## Verification

After all tasks are complete, run the full suite one final time:

```bash
cargo test
```

All tests must pass. Zero clippy warnings.

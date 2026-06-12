//! Language-aware chunking of source files into embedding-ready text slices.
//!
//! Markdown splits at H2 headers (prepending any frontmatter to every chunk),
//! Rust and Allium split at top-level declaration boundaries (carrying leading
//! attributes/doc-comments with the following declaration), and other
//! indexable files become a single chunk.

/// Strips leading visibility modifiers from a line.
fn strip_vis(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("pub(crate) ") {
        return rest;
    }
    if let Some(rest) = s.strip_prefix("pub(super) ") {
        return rest;
    }
    if let Some(rest) = s.strip_prefix("pub ") {
        return rest;
    }
    s
}

/// Returns true if `line` (at column 0) starts a declaration boundary.
///
/// Visibility (`pub`, `pub(crate)`, `pub(super)`) and async/unsafe modifiers
/// are stripped before checking against `keywords`.
fn is_decl_boundary(line: &str, keywords: &[&str]) -> bool {
    let s = strip_vis(line);
    let s = s.strip_prefix("async ").unwrap_or(s);
    let s = s.strip_prefix("unsafe ").unwrap_or(s);
    // Handle `unsafe async fn` — after stripping `unsafe `, `async ` may still be present.
    let s = s.strip_prefix("async ").unwrap_or(s);
    keywords.iter().any(|kw| s.starts_with(kw))
}

/// Split `content` into chunks at declaration boundaries.
///
/// Uses a two-buffer algorithm:
/// - `main_current`: non-attribute body lines for the current chunk.
/// - `attr_buffer`: pending attribute/doc lines at column 0 that belong to
///   the NEXT chunk when a declaration boundary arrives.
fn chunk_by_declarations(
    content: &str,
    decl_keywords: &[&str],
    is_attr_line: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut main_current = String::new();
    let mut attr_buffer = String::new();
    let mut has_seen_decl = false;

    for line in content.lines() {
        if is_decl_boundary(line, decl_keywords) {
            // main_current is non-empty whenever has_seen_decl is true, but guard is kept for clarity.
            if has_seen_decl && !main_current.is_empty() {
                chunks.push(main_current.trim_end().to_string());
                main_current = format!("{attr_buffer}{line}\n");
            } else {
                // First declaration: absorb preamble + pending attrs
                main_current.push_str(&attr_buffer);
                main_current.push_str(line);
                main_current.push('\n');
            }
            attr_buffer.clear();
            has_seen_decl = true;
        } else if is_attr_line(line) {
            attr_buffer.push_str(line);
            attr_buffer.push('\n');
        } else if line.trim().is_empty() && !attr_buffer.is_empty() {
            // Blank line inside an attr run: keep it in the buffer so it
            // travels with the attribute to the next declaration.
            attr_buffer.push_str(line);
            attr_buffer.push('\n');
        } else {
            // Flush attr_buffer into body (attr was orphaned — not followed by a decl)
            main_current.push_str(&attr_buffer);
            main_current.push_str(line);
            main_current.push('\n');
            attr_buffer.clear();
        }
    }

    // Append any remaining attr_buffer to the last chunk
    let remaining = format!("{main_current}{attr_buffer}");
    if !remaining.trim().is_empty() {
        chunks.push(remaining.trim_end().to_string());
    }

    chunks
}

pub(crate) fn chunk_rust(content: &str) -> Vec<String> {
    const RUST_KEYWORDS: &[&str] = &[
        "fn ", "impl ", "impl<", "struct ", "enum ", "trait ", "type ", "mod ", "const ", "static ",
    ];
    chunk_by_declarations(content, RUST_KEYWORDS, |line| {
        line.starts_with("#[") || line.starts_with("///")
    })
}

pub(crate) fn chunk_allium(content: &str) -> Vec<String> {
    const ALLIUM_KEYWORDS: &[&str] = &[
        "entity ",
        "rule ",
        "surface ",
        "config ",
        "enum ",
        "concept ",
        "external ",
        "invariant ",
        "value ",
    ];
    chunk_by_declarations(content, ALLIUM_KEYWORDS, |line| line.starts_with("-- "))
}

/// Returns `(None, content)` if no valid frontmatter fence is found.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let s = content.trim_start();
    if !s.starts_with("---") {
        return (None, content);
    }
    let Some(after_open) = s.get(3..) else {
        return (None, content);
    };
    let Some(inner) = after_open.strip_prefix('\n') else {
        return (None, content);
    };
    let Some(close) = inner.find("\n---") else {
        return (None, content);
    };
    let fm = inner[..close].trim();
    let body = inner[close + 4..].trim_start_matches('\n');
    (Some(fm).filter(|s| !s.is_empty()), body)
}

/// Split `content` into embedding-ready text chunks.
///
/// Rules:
/// - Each H2 (`## `) header starts a new chunk.
/// - The frontmatter block (if present) is prepended to every chunk.
/// - Files with no H2 headers are one chunk (the whole body).
/// - Files that are empty or contain only frontmatter produce no chunks.
pub(crate) fn chunk_markdown(content: &str) -> Vec<String> {
    let (fm, body) = split_frontmatter(content);

    if body.trim().is_empty() {
        return vec![];
    }

    let prefix: Option<String> = fm.map(|fm| format!("{fm}\n---\n"));

    // Split at H2 boundaries. Content before the first H2 (preamble, H1
    // title, etc.) is merged into the first H2 chunk rather than treated as
    // a standalone chunk — it rarely has standalone retrieval value.
    let mut raw_chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut seen_h2 = false;

    for line in body.lines() {
        if line.starts_with("## ") {
            if seen_h2 && !current.trim().is_empty() {
                raw_chunks.push(current.trim_end().to_string());
                current = String::new();
            }
            seen_h2 = true;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        raw_chunks.push(current.trim_end().to_string());
    }

    if raw_chunks.is_empty() {
        raw_chunks.push(body.trim().to_string());
    }

    match prefix {
        Some(pfx) => raw_chunks.iter().map(|c| format!("{pfx}{c}")).collect(),
        None => raw_chunks,
    }
}

// Dispatches by extension. Keep the recognised arms in sync with
// `scan::INDEXABLE_EXTENSIONS`; unrecognised extensions become a single chunk.
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // --- chunk_markdown ---

    #[test]
    fn chunk_markdown_no_h2_returns_single_chunk() {
        let content = "# Title\n\nSome body text.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Some body text."));
    }

    #[test]
    fn chunk_markdown_two_h2s_returns_two_chunks() {
        let content = "# Title\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Section A"));
        assert!(chunks[0].contains("Text A."));
        assert!(chunks[1].contains("Section B"));
        assert!(chunks[1].contains("Text B."));
    }

    #[test]
    fn chunk_markdown_with_frontmatter_prepends_to_each_chunk() {
        let content =
            "---\ntags: [foo, bar]\n---\n\n## Section A\n\nText A.\n\n## Section B\n\nText B.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 2);
        for chunk in &chunks {
            assert!(
                chunk.contains("tags: [foo, bar]"),
                "missing frontmatter in: {chunk}"
            );
        }
    }

    #[test]
    fn chunk_markdown_no_h2_with_frontmatter_is_one_chunk_with_prefix() {
        let content = "---\ninterviewee: Gustaf\n---\n\n# Title\n\nBody text.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("interviewee: Gustaf"));
        assert!(chunks[0].contains("Body text."));
    }

    #[test]
    fn chunk_markdown_empty_body_returns_no_chunks() {
        let content = "---\ntags: [foo]\n---\n";
        let chunks = chunk_markdown(content);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_markdown_empty_string_returns_no_chunks() {
        let chunks = chunk_markdown("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_markdown_h2_at_start_of_body() {
        let content = "## Only Section\n\nContent here.";
        let chunks = chunk_markdown(content);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Only Section"));
    }

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
        let result =
            chunk_by_declarations("pub fn foo() {}\n\npub fn bar() {}", &["fn "], |_| false);
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
        let result =
            chunk_by_declarations("async fn foo() {}\n\nasync fn bar() {}", &["fn "], |_| {
                false
            });
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_unsafe_async_prefix_triggers_split() {
        let result = chunk_by_declarations(
            "unsafe async fn foo() {}\n\nunsafe async fn bar() {}",
            &["fn "],
            |_| false,
        );
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn chunk_by_decls_attr_before_first_decl_stays_in_first_chunk() {
        // attr_buffer is flushed into the first chunk (not lost) when the first declaration arrives.
        let content = "#[derive(Debug)]\nstruct Foo {}";
        let result = chunk_by_declarations(content, &["struct "], |l| l.starts_with("#["));
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("#[derive(Debug)]"), "got: {}", result[0]);
        assert!(result[0].contains("struct Foo"), "got: {}", result[0]);
    }

    #[test]
    fn chunk_by_decls_trailing_attr_absorbed_into_last_chunk() {
        // An attr at the very end of a file (after the last declaration body) stays in that chunk.
        let content = "fn foo() {}\n#[orphan_at_eof]";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("#["));
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("#[orphan_at_eof]"), "got: {}", result[0]);
    }

    #[test]
    fn chunk_by_decls_adjacent_decls_no_blank_line_produce_two_chunks() {
        let result = chunk_by_declarations("fn foo() {}\nfn bar() {}", &["fn "], |_| false);
        assert_eq!(
            result.len(),
            2,
            "adjacent decls should each be their own chunk: {:?}",
            result
        );
    }

    #[test]
    fn chunk_by_decls_single_fn_returns_one_chunk() {
        let result = chunk_by_declarations("fn foo() {}", &["fn "], |_| false);
        assert_eq!(result.len(), 1);
        assert!(result[0].contains("fn foo()"));
    }

    #[test]
    fn chunk_by_decls_doc_comment_before_decl_stays_with_it() {
        let content = "fn foo() {}\n\n/// Does bar.\nfn bar() {}";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("///"));
        assert_eq!(result.len(), 2);
        assert!(
            !result[0].contains("/// Does bar."),
            "doc comment must not be in foo chunk: {}",
            result[0]
        );
        assert!(
            result[1].contains("/// Does bar."),
            "doc comment must be in bar chunk: {}",
            result[1]
        );
    }

    #[test]
    fn chunk_by_decls_indented_fn_is_not_a_boundary() {
        let content = "fn outer() {\n    fn inner() {}\n}";
        let result = chunk_by_declarations(content, &["fn "], |_| false);
        assert_eq!(result.len(), 1, "indented fn must not split: {:?}", result);
        assert!(result[0].contains("fn inner()"));
    }

    #[test]
    fn chunk_by_decls_only_attr_lines_no_decls_returns_single_chunk() {
        let content = "#[derive(Debug)]\n#[allow(dead_code)]";
        let result = chunk_by_declarations(content, &["fn "], |l| l.starts_with("#["));
        assert_eq!(
            result.len(),
            1,
            "attr-only content must not be empty: {:?}",
            result
        );
        assert!(result[0].contains("#[derive(Debug)]"));
    }

    // --- is_decl_boundary ---

    #[test]
    fn is_decl_boundary_fn_at_col0_returns_true() {
        assert!(is_decl_boundary("fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_async_fn_returns_true() {
        assert!(is_decl_boundary("async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_fn_returns_true() {
        assert!(is_decl_boundary("pub fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_async_fn_returns_true() {
        assert!(is_decl_boundary("pub async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_crate_async_fn_returns_true() {
        assert!(is_decl_boundary("pub(crate) async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_pub_super_fn_returns_true() {
        assert!(is_decl_boundary("pub(super) fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_unsafe_fn_returns_true() {
        assert!(is_decl_boundary("unsafe fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_unsafe_async_fn_returns_true() {
        assert!(is_decl_boundary("unsafe async fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_struct_returns_true() {
        assert!(is_decl_boundary("struct Foo", &["fn ", "struct "]));
    }

    #[test]
    fn is_decl_boundary_enum_returns_true() {
        assert!(is_decl_boundary("enum Bar", &["fn ", "enum "]));
    }

    #[test]
    fn is_decl_boundary_impl_returns_true() {
        assert!(is_decl_boundary("impl Foo", &["fn ", "impl "]));
    }

    #[test]
    fn is_decl_boundary_indented_fn_returns_false() {
        assert!(!is_decl_boundary("    fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_comment_returns_false() {
        assert!(!is_decl_boundary("// fn foo()", &["fn "]));
    }

    #[test]
    fn is_decl_boundary_let_stmt_returns_false() {
        assert!(!is_decl_boundary("let x = fn_call()", &["fn "]));
    }

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
        assert_eq!(
            chunks.len(),
            1,
            "indented fn should not split: {:?}",
            chunks
        );
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
        let content =
            "struct Foo<T>(T);\n\nimpl<T: std::fmt::Debug> Foo<T> {\n    fn foo(&self) {}\n}";
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

    #[test]
    fn chunk_rust_struct_splits() {
        let chunks = chunk_rust("struct Foo { x: i32 }\n\nstruct Bar { y: i32 }");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("struct Foo"));
        assert!(chunks[1].contains("struct Bar"));
    }

    #[test]
    fn chunk_rust_trait_splits() {
        let chunks = chunk_rust("struct Foo {}\n\ntrait Greet {\n    fn hello(&self);\n}");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("trait Greet"));
    }

    #[test]
    fn chunk_rust_const_splits() {
        let chunks = chunk_rust("fn foo() {}\n\nconst MAX: usize = 100;");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("const MAX"));
    }

    #[test]
    fn chunk_rust_mod_splits() {
        let chunks = chunk_rust("fn foo() {}\n\nmod utils {\n    pub fn helper() {}\n}");
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("mod utils"));
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

    #[test]
    fn chunk_allium_surface_splits() {
        let content = "entity Task {}\n\nsurface KanbanBoard {\n    shows: Task\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("surface KanbanBoard"));
    }

    #[test]
    fn chunk_allium_value_splits() {
        let content = "entity Task {}\n\nvalue TrajectoryEntry {\n    id: u64\n}";
        let chunks = chunk_allium(content);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].contains("value TrajectoryEntry"));
    }

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
}

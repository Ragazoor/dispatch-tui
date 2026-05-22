# RAG Code-File Support Design

**Date:** 2026-05-22
**Task:** #1216 — Update RAG to index code files

## Background

The RAG indexer (`src/service/repo_index.rs`) currently only walks `.md` files and uses an
H2-header-based chunker designed for Markdown. This makes it unusable on code-heavy repos like
the dispatch repo itself (primarily Rust `.rs` files and Allium `.allium` spec files).

## Scope

Extend the indexer to walk and chunk `.rs` and `.allium` files in addition to `.md`. No changes
to the embedding model, DB schema, MCP API, or search logic.

---

## File Walking

`walk_md_files` is renamed `walk_indexable_files`. A new constant drives which extensions are
walked:

```rust
const INDEXABLE_EXTENSIONS: &[&str] = &["md", "rs", "allium"];
```

All existing walk behaviour is preserved: `ignore::WalkBuilder` (respects `.gitignore`), skip
`.dispatch/` directory, return `Vec<PathBuf>`. The caller in `index_repo` reads the extension
from each path to select a chunker.

Adding a new file type in future requires only updating this constant and adding a dispatch arm
in `chunk_for_extension`.

---

## Chunking

### Public API

| Function | Input | Description |
|---|---|---|
| `chunk_markdown(content)` | `&str` | Renamed from `chunk_file`. H2-based + frontmatter. |
| `chunk_rust(content)` | `&str` | Top-level Rust declaration boundaries. |
| `chunk_allium(content)` | `&str` | Top-level Allium block boundaries. |
| `chunk_for_extension(content, ext)` | `&str, &str` | Dispatcher → one of the above. |

`chunk_for_extension` maps:
- `"md"` → `chunk_markdown`
- `"rs"` → `chunk_rust`
- `"allium"` → `chunk_allium`
- anything else → whole file as one chunk (safe fallback)

### Private shared helper: `chunk_by_declarations`

Both `chunk_rust` and `chunk_allium` delegate to:

```rust
fn chunk_by_declarations(
    content: &str,
    decl_keywords: &[&str],
    is_attr_line: fn(&str) -> bool,
) -> Vec<String>
```

**Two-buffer algorithm:**

The core insight is that doc comments and attributes immediately preceding a declaration belong
*semantically* to that declaration, not to the item above it. A single-buffer accumulator would
attach `/// doc comment` to the wrong chunk. The two-buffer approach fixes this.

```
main_current: String   # non-attribute content for the current chunk
attr_buffer:  String   # pending attr/doc lines that belong to the NEXT chunk

has_seen_decl: bool = false

for each line:
    if is_decl_boundary(line, decl_keywords):
        if has_seen_decl and main_current.trim() non-empty:
            push main_current.trim_end() to chunks
        main_current = attr_buffer + line + "\n"
        attr_buffer  = ""
        has_seen_decl = true
    elif is_attr_line(line):
        attr_buffer += line + "\n"
    else:
        main_current += attr_buffer + line + "\n"  # flush attr_buffer into body
        attr_buffer = ""

remaining = (main_current + attr_buffer).trim_end()
if non-empty: push remaining to chunks

if chunks is empty: return [content.trim()]
```

**Boundary detection (`is_decl_boundary`):**

Strips visibility (`pub`, `pub(crate)`, `pub(super)`) and modifiers (`async`, `unsafe`) before
checking `decl_keywords`. The line must start at column 0 (no leading whitespace).

**Rust declaration keywords:**
`fn `, `impl `, `impl<`, `struct `, `enum `, `trait `, `type `, `mod `, `const `, `static `

Rust attribute/doc lines (`is_attr_line`):
lines at column 0 starting with `#[` or `///`

**Allium declaration keywords:**
`entity `, `rule `, `surface `, `config`, `enum `, `concept `, `external `, `invariant `

Note: `config` is checked without a trailing space because it appears as `config {` in specs.

Allium comment lines (`is_attr_line`):
lines starting with `-- ` (Allium doc/section comments)

### Behaviour examples

**Rust — doc comment stays with its function:**
```
// BEFORE split_frontmatter chunk:
use std::path::Path; ...

// chunk containing split_frontmatter:
/// Returns `(None, content)` if no valid frontmatter fence is found.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) { ... }
```

**Rust — attribute stays with its item:**
```
// chunk 1:
pub struct Foo { x: i32, }

// chunk 2:
#[derive(Debug, Clone)]
pub enum Bar { A, B }
```

**Allium — section comment stays with its rule:**
```
// chunk 1:
entity Task { ... }

// chunk 2:
-- == Task Creation ==

rule CreateTask { ... }
```

---

## Changes to `index_repo`

Two targeted changes in the Phase 1 blocking closure:

1. Replace `walk_md_files(&repo_path)?` with `walk_indexable_files(&repo_path)?`
2. In Phase 2, extract the extension from the path and call:
   ```rust
   let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
   let chunks = chunk_for_extension(&content, ext);
   ```

No other changes to `index_repo`, `search_docs`, DB schema, or MCP handlers.

---

## Testing

All new logic is unit-tested before implementation (TDD).

### `chunk_by_declarations` unit tests
- Empty content → no chunks
- Content with no matching keywords → one chunk
- Two declarations → two chunks, each containing its own body
- `pub`, `pub(crate)`, `pub(super)` prefixes all trigger correctly
- `async fn` and `unsafe fn` both split
- Attribute/doc lines before a declaration land in the NEW chunk, not the old one
- Non-attr line after an attr line flushes attr into the current chunk body

### `chunk_rust` unit tests
- `fn`, `pub fn`, `async fn`, `pub async fn` all split
- Indented `fn` inside an impl body does NOT split
- `impl Foo` and `impl<T: Trait>` both split
- `#[derive(Debug)]` before `struct` → in same chunk as `struct`
- `/// doc comment` before `fn` → in same chunk as `fn`

### `chunk_allium` unit tests
- `entity`, `rule`, `surface`, `config`, `enum`, `concept`, `external`, `invariant` each start new chunks
- `-- ` comment lines at column 0 before a rule land in the rule's chunk
- `-- comment` mid-body (indented or not) stays in current chunk

### `walk_indexable_files` unit tests
- Finds `.md`, `.rs`, `.allium` files in a tempdir
- Ignores `.txt`, `.json`, `.py` and other unsupported extensions
- Still skips `.dispatch/` directory

### Integration tests (added to `mod tests` in `repo_index.rs`)
- `index_repo_indexes_rs_files` — tempdir with a `.rs` file → `files_indexed: 1`, ≥1 chunk
- `index_repo_indexes_allium_files` — tempdir with a `.allium` file → `files_indexed: 1`
- `search_docs_finds_rust_code` — index a `.rs` file, search a keyword from it, assert a result

---

## Out of Scope

- Other languages (`.toml`, `.py`, `.ts`) — can be added later by extending `INDEXABLE_EXTENSIONS`
  and adding keywords to `chunk_for_extension`
- `#[cfg(test)] mod tests` filtering — included for now; test bodies are rarely the search target
  but filtering would add complexity without clear benefit
- Allium spec update — `index_repo` / `search_docs` are not currently specced in any `.allium`
  file; no spec change needed for this refactor

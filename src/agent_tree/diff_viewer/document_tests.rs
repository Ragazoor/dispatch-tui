use super::*;
use crate::process::MockProcessRunner;

fn patch(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\n@@ -1 +1 @@\n-old\n+new\n")
}

fn binary(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\nBinary files a/{path} and b/{path} differ\n")
}

fn rig(outputs: &[&str]) -> MockProcessRunner {
    MockProcessRunner::new(
        outputs
            .iter()
            .map(|o| MockProcessRunner::ok_with_stdout(o.as_bytes()))
            .collect(),
    )
}

fn texts(lines: &[DiffLine]) -> Vec<String> {
    lines.iter().map(|l| l.text.clone()).collect()
}

/// The path heading that opens each file's section — the document's own
/// account of which files it is showing, and in what order.
fn headings(lines: &[DiffLine]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Heading)
        .map(|l| l.text.clone())
        .collect()
}

/// Tree order, not the order the user opened them in: the tree is the index
/// this pane is read through, so the two must scroll the same way.
#[test]
fn each_file_renders_under_its_own_path_heading() {
    let runner = rig(&[&patch("a.rs"), &patch("src/lib.rs")]);

    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs", "src/lib.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    assert_eq!(headings(&doc), vec!["a.rs", "src/lib.rs"]);
    assert_eq!(texts(&doc)[0], "a.rs");
}

/// The document follows the order the TREE published, and does not sort.
/// Tree order is not path order — a folder's own files sort ahead of its
/// subfolders (`RowsPutAFoldersOwnFilesFirst`), so `z.rs` has a row above
/// `src/lib.rs` while sorting after it lexicographically.
///
/// The input is deliberately an order no sort of these paths produces.
#[test]
fn the_document_follows_the_published_order_rather_than_sorting() {
    let runner = rig(&[&patch("a.rs"), &patch("z.rs"), &patch("src/lib.rs")]);

    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs", "z.rs", "src/lib.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    assert_eq!(headings(&doc), vec!["a.rs", "z.rs", "src/lib.rs"]);
}

/// A refusal renders its reason in place of contents. There are exactly
/// two refusals now — binary and too large — and no "not yet staged"
/// placeholder (DiffRefusal).
#[test]
fn a_refused_file_renders_its_reason_in_place_of_contents() {
    let runner = rig(&[&binary("logo.png")]);

    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["logo.png"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    let lines = texts(&doc);
    assert_eq!(lines, vec!["logo.png", DiffRefusal::Binary.message()]);
    assert_eq!(doc[1].kind, DiffLineKind::Refusal);
}

/// One refused file must not cost the user the diffs either side of it —
/// which is why a refusal rides on the file rather than on the pane.
#[test]
fn a_refusal_does_not_stop_the_files_around_it_rendering() {
    let runner = rig(&[&patch("a.rs"), &binary("logo.png"), &patch("z.rs")]);

    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs", "logo.png", "z.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    assert_eq!(headings(&doc), vec!["a.rs", "logo.png", "z.rs"]);
    let lines = texts(&doc);
    assert!(lines.iter().any(|l| l == DiffRefusal::Binary.message()));
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("@@")).count(),
        2,
        "both neighbours must still have their hunks; got {lines:?}"
    );
}

/// The agent reverted a file the user had open. It renders nothing and
/// stays open — see OpenDiffPathsMaySurviveTheirFiles in the spec.
#[test]
fn a_path_with_nothing_to_show_contributes_no_section() {
    let runner = rig(&["", &patch("b.rs")]);

    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs", "b.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    assert_eq!(headings(&doc), vec!["b.rs"]);
}

#[test]
fn an_empty_open_set_builds_an_empty_document() {
    let runner = rig(&[]);
    let doc = build_document(Path::new("/wt"), None, &[], &BTreeSet::new(), &runner).unwrap();
    assert!(doc.is_empty());
}

#[test]
fn added_and_removed_lines_are_classified_apart_from_the_file_headers() {
    let runner = rig(&[&patch("a.rs")]);
    let doc = build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .unwrap();

    let kinds: Vec<_> = doc.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        vec![
            DiffLineKind::Heading,
            DiffLineKind::Meta,
            DiffLineKind::Meta,
            DiffLineKind::Removed,
            DiffLineKind::Added,
        ]
    );
}

/// `+++` and `---` open git's file headers. Reading them as content would
/// paint two lines of every single diff the wrong colour.
#[test]
fn the_triple_dash_file_headers_are_not_read_as_content() {
    assert_eq!(DiffLineKind::of("--- a/x.rs"), DiffLineKind::Meta);
    assert_eq!(DiffLineKind::of("+++ b/x.rs"), DiffLineKind::Meta);
    assert_eq!(DiffLineKind::of("-old"), DiffLineKind::Removed);
    assert_eq!(DiffLineKind::of("+new"), DiffLineKind::Added);
    assert_eq!(DiffLineKind::of(" same"), DiffLineKind::Context);
}

#[test]
fn a_git_failure_fails_the_whole_build() {
    let runner = MockProcessRunner::new(vec![MockProcessRunner::fail("fatal: bad object")]);
    assert!(build_document(
        Path::new("/wt"),
        None,
        &open_set(&["a.rs"]),
        &BTreeSet::new(),
        &runner,
    )
    .is_err());
}

use super::*;

fn root() -> PathBuf {
    PathBuf::from("/repo")
}

fn changed(path: &str, change: FileChange) -> GitFileChange {
    GitFileChange {
        path: PathBuf::from(path),
        change,
        counts: None,
    }
}

fn modified(path: &str) -> GitFileChange {
    changed(path, FileChange::Modified)
}

fn added(path: &str) -> GitFileChange {
    changed(path, FileChange::Added)
}

fn counted(path: &str, change: FileChange, added: u32, removed: u32) -> GitFileChange {
    GitFileChange {
        path: PathBuf::from(path),
        change,
        counts: Some(LineCounts { added, removed }),
    }
}

/// Build a `--numstat -z` stream. Git emits one NUL-terminated record per
/// path, the three fields inside it separated by tabs.
fn numstat_stream(records: &[&str]) -> String {
    records.iter().map(|r| format!("{r}\0")).collect()
}

fn counts_of(node: &TreeNode, path: &[&str]) -> Option<LineCounts> {
    let segments: Vec<String> = path.iter().map(|s| (*s).to_owned()).collect();
    node.node_at(&segments).expect("node should exist").counts
}

// -- parse_numstat ----------------------------------------------------

#[test]
fn numstat_reads_added_and_removed_for_each_path() {
    let out = numstat_stream(&["12\t3\tsrc/foo.rs", "0\t7\tsrc/bar.rs"]);
    let parsed = parse_numstat(&out);

    assert_eq!(
        parsed.get(Path::new("src/foo.rs")),
        Some(&LineCounts {
            added: 12,
            removed: 3
        })
    );
    assert_eq!(
        parsed.get(Path::new("src/bar.rs")),
        Some(&LineCounts {
            added: 0,
            removed: 7
        })
    );
}

/// Git spells a binary diff `-\t-\t<path>`. That is an absence of counts,
/// not a zero, and it must not become one.
#[test]
fn numstat_reports_no_counts_for_a_binary_file() {
    let out = numstat_stream(&["-\t-\tassets/logo.png"]);
    assert_eq!(parse_numstat(&out).get(Path::new("assets/logo.png")), None);
}

/// One unreadable record must not cost the caller the rest of the answer —
/// the same soft-fail-decoding rule `parse_name_status` follows.
#[test]
fn numstat_skips_a_malformed_record_and_keeps_the_others() {
    let out = numstat_stream(&["not a record", "4\t1\tsrc/ok.rs", "x\ty\tsrc/bad.rs"]);
    let parsed = parse_numstat(&out);

    assert_eq!(
        parsed.get(Path::new("src/ok.rs")),
        Some(&LineCounts {
            added: 4,
            removed: 1
        })
    );
    assert_eq!(parsed.get(Path::new("src/bad.rs")), None);
    assert_eq!(parsed.len(), 1);
}

/// A path is every byte after the second tab, so a filename containing a
/// space or a tab survives. `-z` is what makes that unambiguous.
#[test]
fn numstat_keeps_a_path_containing_whitespace_intact() {
    let out = numstat_stream(&["1\t0\tdocs/my notes.md"]);
    assert_eq!(
        parse_numstat(&out).get(Path::new("docs/my notes.md")),
        Some(&LineCounts {
            added: 1,
            removed: 0
        })
    );
}

// -- attach_line_counts -----------------------------------------------

#[test]
fn attaching_counts_matches_on_path() {
    let mut changes = vec![modified("src/foo.rs"), modified("src/bar.rs")];
    let counts = parse_numstat(&numstat_stream(&["12\t3\tsrc/foo.rs"]));

    attach_line_counts(&mut changes, &counts);

    assert_eq!(
        changes[0].counts,
        Some(LineCounts {
            added: 12,
            removed: 3
        })
    );
    // In the diff but absent from the numstat: rendered with no counts
    // rather than with a guessed zero.
    assert_eq!(changes[1].counts, None);
}

#[test]
fn attaching_counts_leaves_an_untracked_path_alone() {
    let mut changes = vec![added("new.rs")];
    attach_line_counts(&mut changes, &parse_numstat(""));
    assert_eq!(changes[0].counts, None);
}

// -- counts on the tree -----------------------------------------------

#[test]
fn a_file_node_carries_its_own_counts() {
    let tree = build_tree(
        &root(),
        &[counted("src/foo.rs", FileChange::Modified, 12, 3)],
    );
    assert_eq!(
        counts_of(&tree, &["src", "foo.rs"]),
        Some(LineCounts {
            added: 12,
            removed: 3
        })
    );
}

#[test]
fn an_untracked_file_node_has_no_counts() {
    let tree = build_tree(&root(), &[added("new.rs")]);
    assert_eq!(counts_of(&tree, &["new.rs"]), None);
}

/// Zero is a real answer — a permission-only change moves no lines — so it
/// must survive as `Some(0, 0)` and not collapse into "no counts".
#[test]
fn zero_counts_are_kept_rather_than_read_as_absent() {
    let tree = build_tree(&root(), &[counted("mode.sh", FileChange::Modified, 0, 0)]);
    assert_eq!(
        counts_of(&tree, &["mode.sh"]),
        Some(LineCounts {
            added: 0,
            removed: 0
        })
    );
}

#[test]
fn a_directory_node_sums_its_descendants() {
    let tree = build_tree(
        &root(),
        &[
            counted("src/a.rs", FileChange::Modified, 12, 3),
            counted("src/inner/b.rs", FileChange::Added, 5, 1),
        ],
    );

    assert_eq!(
        counts_of(&tree, &["src"]),
        Some(LineCounts {
            added: 17,
            removed: 4
        })
    );
    assert_eq!(
        counts_of(&tree, &["src", "inner"]),
        Some(LineCounts {
            added: 5,
            removed: 1
        })
    );
}

/// The whole reason the sum is over `Option`: an unstaged file contributes
/// nothing, rather than contributing a zero that would drag the directory's
/// total down to a number smaller than what is really in there.
#[test]
fn a_directory_sum_skips_descendants_that_have_no_counts() {
    let tree = build_tree(
        &root(),
        &[
            counted("src/a.rs", FileChange::Modified, 12, 3),
            added("src/brand_new.rs"),
        ],
    );

    assert_eq!(
        counts_of(&tree, &["src"]),
        Some(LineCounts {
            added: 12,
            removed: 3
        })
    );
}

#[test]
fn a_directory_with_no_counted_descendants_has_no_counts() {
    let tree = build_tree(&root(), &[added("src/one.rs"), added("src/two.rs")]);
    assert_eq!(counts_of(&tree, &["src"]), None);
}

/// The `git rm --cached` collision: the diff reports the path with counts
/// and the untracked listing reports it without. The counted answer is the
/// informative one, and which query ran first must not decide it.
#[test]
fn a_path_reported_by_both_queries_keeps_the_counts_it_has() {
    let orders: [Vec<GitFileChange>; 2] = [
        vec![
            counted("foo.rs", FileChange::Deleted, 0, 9),
            added("foo.rs"),
        ],
        vec![
            added("foo.rs"),
            counted("foo.rs", FileChange::Deleted, 0, 9),
        ],
    ];
    for changes in orders {
        let tree = build_tree(&root(), &changes);
        assert_eq!(
            counts_of(&tree, &["foo.rs"]),
            Some(LineCounts {
                added: 0,
                removed: 9
            })
        );
    }
}

// -- parse_name_status ------------------------------------------------

/// Build a `-z` name-status stream: NUL after every field, including the
/// last, exactly as git emits it.
fn nul_stream(fields: &[&str]) -> String {
    fields.iter().map(|f| format!("{f}\0")).collect()
}

#[test]
fn name_status_maps_the_three_letters_to_the_three_changes() {
    let out = nul_stream(&["A", "src/new.rs", "M", "src/foo.rs", "D", "src/old.rs"]);
    let out = out.as_str();
    assert_eq!(
        parse_name_status(out),
        vec![
            changed("src/new.rs", FileChange::Added),
            changed("src/foo.rs", FileChange::Modified),
            changed("src/old.rs", FileChange::Deleted),
        ]
    );
}

/// A type change (file becomes a symlink, or the reverse) is a
/// modification as far as a file tree is concerned — see the spec's
/// CollapsedGitStatusLetters note.
#[test]
fn type_change_is_reported_as_modified() {
    assert_eq!(
        parse_name_status(&nul_stream(&["T", "src/link.rs"])),
        vec![changed("src/link.rs", FileChange::Modified)]
    );
}

/// Soft-fail decoding: one line this build cannot read must not cost the
/// lines around it.
#[test]
fn unrecognised_status_letter_is_skipped_not_guessed() {
    let out = nul_stream(&["M", "keep.rs", "U", "conflicted.rs", "A", "also-keep.rs"]);
    let out = out.as_str();
    assert_eq!(
        parse_name_status(out),
        vec![
            changed("keep.rs", FileChange::Modified),
            changed("also-keep.rs", FileChange::Added),
        ]
    );
}

/// A truncated stream — a status with no path after it — ends the parse
/// rather than pairing the status with whatever follows.
#[test]
fn trailing_status_without_a_path_is_skipped() {
    assert_eq!(
        parse_name_status(&nul_stream(&["M", "keep.rs", "D"])),
        vec![changed("keep.rs", FileChange::Modified)]
    );
}

#[test]
fn empty_output_parses_to_nothing() {
    assert!(parse_name_status("").is_empty());
    assert!(parse_name_status("\0").is_empty());
}

/// `-z` means whitespace in a filename is just bytes: no quoting to undo,
/// and nothing may trim it away. A leading or trailing space is part of the
/// name, and a path can even contain a newline.
#[test]
fn whitespace_in_a_path_survives_parsing_verbatim() {
    assert_eq!(
        parse_name_status(&nul_stream(&["M", "docs/my notes.md"])),
        vec![changed("docs/my notes.md", FileChange::Modified)]
    );
    assert_eq!(
        parse_name_status(&nul_stream(&["M", " leading.rs"])),
        vec![changed(" leading.rs", FileChange::Modified)]
    );
    assert_eq!(
        parse_untracked(&nul_stream(&["trailing.rs "])),
        vec![changed("trailing.rs ", FileChange::Added)]
    );
    assert_eq!(
        parse_name_status(&nul_stream(&["A", "weird\nname.rs"])),
        vec![changed("weird\nname.rs", FileChange::Added)]
    );
}

/// Git C-quotes non-ASCII paths unless `-z` is used. With it, the real
/// bytes arrive and the parser needs no unescaping.
#[test]
fn non_ascii_paths_arrive_unquoted() {
    assert_eq!(
        parse_name_status(&nul_stream(&["M", "src/é.rs"])),
        vec![changed("src/é.rs", FileChange::Modified)]
    );
    assert_eq!(
        parse_untracked(&nul_stream(&["docs/naïve.md"])),
        vec![changed("docs/naïve.md", FileChange::Added)]
    );
}

// -- parse_untracked --------------------------------------------------

#[test]
fn every_untracked_path_is_added() {
    assert_eq!(
        parse_untracked(&nul_stream(&["src/new.rs", "docs/draft.md"])),
        vec![
            changed("src/new.rs", FileChange::Added),
            changed("docs/draft.md", FileChange::Added),
        ]
    );
}

#[test]
fn empty_untracked_output_parses_to_nothing() {
    assert!(parse_untracked("").is_empty());
    assert!(parse_untracked("\0").is_empty());
}

// -- build_tree: badges ------------------------------------------------

/// The headline fix (task #4408, part 1): a deleted file is badged deleted,
/// not modified.
#[test]
fn deleted_file_is_badged_deleted() {
    let tree = build_tree(&root(), &[changed("src/old.rs", FileChange::Deleted)]);
    let node = tree.node_at(&["src", "old.rs"]).expect("node exists");
    assert_eq!(node.badge, Some(FileChange::Deleted));
}

#[test]
fn added_file_is_badged_added() {
    let tree = build_tree(&root(), &[changed("src/new.rs", FileChange::Added)]);
    let node = tree.node_at(&["src", "new.rs"]).expect("node exists");
    assert_eq!(node.badge, Some(FileChange::Added));
}

#[test]
fn modified_file_is_badged_modified() {
    let tree = build_tree(&root(), &[changed("src/foo.rs", FileChange::Modified)]);
    let node = tree.node_at(&["src", "foo.rs"]).expect("node exists");
    assert_eq!(node.badge, Some(FileChange::Modified));
}

/// The headline fix (task #4408, part 2): git reporting nothing means the
/// tree shows nothing. A file the agent opened but did not change is not a
/// node at all, so it cannot be badged modified.
#[test]
fn a_path_git_does_not_report_gets_no_node() {
    let tree = build_tree(&root(), &[changed("src/foo.rs", FileChange::Modified)]);
    assert!(tree.node_at(&["src", "untouched.rs"]).is_none());
    assert!(tree.node_at(&["README.md"]).is_none());
}

/// The spec's EveryFileNodeIsBadged invariant: an unbadged file node is
/// exactly the "touched but unchanged" state this design exists to remove.
#[test]
fn every_file_node_carries_a_badge() {
    let tree = build_tree(
        &root(),
        &[
            changed("a/b/c.rs", FileChange::Added),
            changed("a/d.rs", FileChange::Deleted),
            changed("e.rs", FileChange::Modified),
        ],
    );
    fn assert_badged(node: &TreeNode) {
        match node.kind {
            TreeNodeKind::File => assert!(node.badge.is_some(), "{} unbadged", node.name),
            TreeNodeKind::Directory => assert_eq!(node.badge, None, "{} badged", node.name),
        }
        for child in &node.children {
            assert_badged(child);
        }
    }
    assert_badged(&tree);
}

/// A rename reaches us as two independent entries because rename detection
/// is off — see the spec's rationale under RefreshAgentTree.
#[test]
fn a_rename_renders_as_a_delete_and_an_add() {
    let tree = build_tree(
        &root(),
        &[
            changed("src/old.rs", FileChange::Deleted),
            changed("src/new.rs", FileChange::Added),
        ],
    );
    assert_eq!(
        tree.node_at(&["src", "old.rs"]).expect("old").badge,
        Some(FileChange::Deleted)
    );
    assert_eq!(
        tree.node_at(&["src", "new.rs"]).expect("new").badge,
        Some(FileChange::Added)
    );
}

/// Both git queries can name the same path — `git rm --cached foo` leaves
/// `foo` deleted in the diff and listed as untracked. Precedence resolves
/// it, and crucially does so regardless of which query ran first: swapping
/// the two commands must not flip a badge.
#[test]
fn a_path_reported_by_both_queries_resolves_by_precedence_not_order() {
    for pair in [
        [
            changed("a.rs", FileChange::Deleted),
            changed("a.rs", FileChange::Added),
        ],
        [
            changed("a.rs", FileChange::Added),
            changed("a.rs", FileChange::Deleted),
        ],
    ] {
        let tree = build_tree(&root(), &pair);
        assert_eq!(
            tree.node_at(&["a.rs"]).expect("a.rs").badge,
            Some(FileChange::Added),
            "on disk but out of the index reads as Added; got {pair:?}"
        );
    }
}

/// The rest of the precedence order, asserted both ways round for the same
/// order-independence reason.
#[test]
fn deleted_beats_modified_in_either_order() {
    for pair in [
        [
            changed("a.rs", FileChange::Modified),
            changed("a.rs", FileChange::Deleted),
        ],
        [
            changed("a.rs", FileChange::Deleted),
            changed("a.rs", FileChange::Modified),
        ],
    ] {
        let tree = build_tree(&root(), &pair);
        assert_eq!(
            tree.node_at(&["a.rs"]).expect("a.rs").badge,
            Some(FileChange::Deleted),
            "got {pair:?}"
        );
    }
}

// -- build_tree: structure --------------------------------------------

#[test]
fn directory_containing_a_changed_file_is_expanded_and_unbadged() {
    let tree = build_tree(&root(), &[changed("src/lib.rs", FileChange::Modified)]);
    let dir = tree.node_at(&["src"]).expect("dir exists");
    assert!(dir.expanded);
    assert_eq!(dir.kind, TreeNodeKind::Directory);
    assert_eq!(dir.badge, None);
}

#[test]
/// The chain `a/b/c` is one node after merging, so the only ancestor to
/// check is the merged one — see `NoSingleChildDirectoryChains`.
fn nested_ancestor_directories_are_all_expanded() {
    let tree = build_tree(&root(), &[changed("a/b/c/d.rs", FileChange::Deleted)]);
    assert!(tree.expanded);
    assert!(tree.node_at(&["a/b/c"]).expect("a/b/c exists").expanded);
    let file = tree.node_at(&["a/b/c", "d.rs"]).expect("file exists");
    assert!(!file.expanded);
    assert_eq!(file.badge, Some(FileChange::Deleted));
}

// -- single-child chain merging ---------------------------------------
//
// docs/specs/agent-tree.allium's NoSingleChildDirectoryChains: a directory
// whose only child is another directory gets no node of its own; the chain
// is one node named by the whole route.

#[test]
fn a_directory_whose_only_child_is_a_directory_merges_into_it() {
    let tree = build_tree(&root(), &[modified("a/b/c.rs")]);

    assert_eq!(tree.children.len(), 1);
    let merged = &tree.children[0];
    assert_eq!(merged.name, "a/b");
    assert_eq!(merged.kind, TreeNodeKind::Directory);
    assert_eq!(merged.children.len(), 1);
    assert_eq!(merged.children[0].name, "c.rs");
}

#[test]
fn a_chain_of_any_length_merges_into_one_node() {
    let tree = build_tree(&root(), &[modified("a/b/c/d/e.rs")]);

    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].name, "a/b/c/d");
    assert_eq!(tree.children[0].children[0].name, "e.rs");
}

/// The guard that keeps a file's own parent visible as the row above it: a
/// directory holding a changed file is not a link in a single-child chain,
/// whatever else it holds.
#[test]
fn a_directory_holding_a_changed_file_is_never_merged_away() {
    let tree = build_tree(
        &root(),
        &[modified("src/main.rs"), modified("src/cli/a.rs")],
    );

    assert_eq!(tree.children.len(), 1);
    let src = &tree.children[0];
    assert_eq!(src.name, "src");
    let names: Vec<&str> = src.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["main.rs", "cli"]);
}

#[test]
fn a_directory_with_two_changed_subdirectories_is_not_merged() {
    let tree = build_tree(&root(), &[modified("src/a/x.rs"), modified("src/b/y.rs")]);

    assert_eq!(tree.children.len(), 1);
    let src = &tree.children[0];
    assert_eq!(src.name, "src");
    let names: Vec<&str> = src.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b"]);
}

/// A merged node is still the sum of everything beneath it
/// (DirectoryCountsSumDescendants) — merging changes which rows exist, not
/// what a row's counts mean.
#[test]
fn a_merged_node_carries_the_sum_of_the_files_beneath_it() {
    let tree = build_tree(
        &root(),
        &[
            counted("a/b/one.rs", FileChange::Modified, 12, 3),
            counted("a/b/two.rs", FileChange::Modified, 5, 1),
        ],
    );

    assert_eq!(
        counts_of(&tree, &["a/b"]),
        Some(LineCounts {
            added: 17,
            removed: 4
        })
    );
}

/// The invariant, checked over every RENDERED node rather than at one:
/// no directory anywhere holds a single directory child. Walked from
/// `root.children`, because the synthetic root is not a rendered node and
/// the invariant does not reach it — the next test is that boundary.
#[test]
fn no_rendered_directory_node_holds_a_lone_directory_child() {
    let tree = build_tree(
        &root(),
        &[
            modified("a/b/c/d.rs"),
            modified("src/main.rs"),
            modified("src/cli/x.rs"),
            modified("src/tui/ui/kanban/columns.rs"),
            modified("top.rs"),
        ],
    );

    fn assert_no_lone_directory_child(node: &TreeNode) {
        if node.children.len() == 1 {
            assert_eq!(
                node.children[0].kind,
                TreeNodeKind::File,
                "{} holds a lone directory child {}",
                node.name,
                node.children[0].name
            );
        }
        for child in &node.children {
            assert_no_lone_directory_child(child);
        }
    }
    for child in &tree.children {
        assert_no_lone_directory_child(child);
    }
}

/// The root is never merged into its only child, and is the one directory
/// allowed to hold a lone directory child: it names the pane root, is
/// drawn as the pane's title rather than as a row, and is not part of any
/// node's relative path. A worktree whose every change is under one
/// directory is the ordinary case, not a violation.
#[test]
fn the_root_node_keeps_its_lone_directory_child() {
    let tree = build_tree(&root(), &[modified("a/b/c.rs")]);

    assert_eq!(tree.name, "repo");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].kind, TreeNodeKind::Directory);
    assert_eq!(tree.children[0].name, "a/b");
}

#[test]
fn unchanged_sibling_directory_does_not_appear() {
    let tree = build_tree(&root(), &[changed("a/b.rs", FileChange::Modified)]);
    assert!(tree.node_at(&["c"]).is_none());
    assert_eq!(tree.children.len(), 1);
}

#[test]
fn no_changes_produce_root_only() {
    let tree = build_tree(&root(), &[]);
    assert_eq!(tree.kind, TreeNodeKind::Directory);
    assert!(tree.children.is_empty());
    assert!(!tree.expanded);
}

/// The spec's RowsPutAFoldersOwnFilesFirst: a folder's own changed files
/// sit directly beneath it, above its subfolders, so a file is never
/// stranded after a sibling subtree at a shallower indent than the row
/// above it.
#[test]
fn a_folders_own_files_sort_ahead_of_its_subfolders() {
    let tree = build_tree(
        &root(),
        &[
            modified("src/zz/deep.rs"),
            modified("src/aa/deep.rs"),
            modified("src/main.rs"),
            modified("src/build.rs"),
        ],
    );

    let src = &tree.children[0];
    let names: Vec<&str> = src.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["build.rs", "main.rs", "aa", "zz"]);
}

/// The ordered walk yields files only, and through a compressed chain it
/// yields the path the file really has — not the shortened row label.
#[test]
fn the_ordered_walk_yields_whole_paths_through_a_merged_row() {
    let tree = build_tree(
        &root(),
        &[
            modified("a/b/c/d.rs"),
            modified("a/b/c/e.rs"),
            modified("f.rs"),
        ],
    );

    assert_eq!(
        file_paths_in_tree_order(&tree),
        vec![
            PathBuf::from("f.rs"),
            PathBuf::from("a/b/c/d.rs"),
            PathBuf::from("a/b/c/e.rs"),
        ]
    );
}

#[test]
fn children_are_sorted_by_name() {
    let tree = build_tree(
        &root(),
        &[
            changed("zebra.rs", FileChange::Modified),
            changed("apple.rs", FileChange::Added),
            changed("mango.rs", FileChange::Deleted),
        ],
    );
    let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["apple.rs", "mango.rs", "zebra.rs"]);
}

#[test]
fn root_node_name_is_last_path_component() {
    let tree = build_tree(&PathBuf::from("/home/user/my-worktree"), &[]);
    assert_eq!(tree.name, "my-worktree");
}

/// Git never emits any of these, but a malformed one must not render a node
/// above the worktree root. `./` is rejected outright rather than
/// normalised away — the guard vouches for paths it recognises, it does not
/// repair ones it does not.
#[test]
fn path_not_strictly_below_the_root_is_dropped() {
    let tree = build_tree(
        &root(),
        &[
            changed("../outside.rs", FileChange::Modified),
            changed("/absolute.rs", FileChange::Modified),
            changed("./relative.rs", FileChange::Modified),
            changed("inside.rs", FileChange::Modified),
        ],
    );
    let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["inside.rs"]);
}

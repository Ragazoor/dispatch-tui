use super::*;

/// Concatenate a line's span contents into a single string for assertions.
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn owned(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

fn ctx(height_offset: u16, area_height: u16, hint: Style) -> RepoListCtx {
    RepoListCtx {
        height_offset,
        area_height,
        hint,
    }
}

// ---- DraftSummary -------------------------------------------------

#[test]
fn draft_summary_defaults_when_no_draft() {
    let input = InputState::default();
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.title, "");
    assert_eq!(summary.tag, "none");
    assert_eq!(summary.description_oneline, "");
}

fn input_with_draft(draft: crate::tui::TaskDraft) -> InputState {
    InputState {
        task_draft: Some(draft),
        ..Default::default()
    }
}

#[test]
fn draft_summary_tag_none_falls_back_to_none() {
    let input = input_with_draft(crate::tui::TaskDraft {
        tag: None,
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.tag, "none");
}

#[test]
fn draft_summary_tag_some_uses_display() {
    let input = input_with_draft(crate::tui::TaskDraft {
        tag: Some(crate::models::TaskTag::Bug),
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.tag, crate::models::TaskTag::Bug.to_string());
    assert_ne!(summary.tag, "none");
}

#[test]
fn draft_summary_oneline_equals_description_without_newline() {
    let input = input_with_draft(crate::tui::TaskDraft {
        description: "single line".to_string(),
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.description_oneline, "single line");
}

#[test]
fn draft_summary_oneline_truncates_multiline_with_ellipsis() {
    let input = input_with_draft(crate::tui::TaskDraft {
        description: "first line\nsecond line".to_string(),
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.description_oneline, "first line ...");
}

#[test]
fn draft_summary_title_reads_from_draft() {
    let input = input_with_draft(crate::tui::TaskDraft {
        title: "my task".to_string(),
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.title, "my task");
}

// ---- input_tag_lines and the phoenix marker ---------------------------

fn form_styles() -> FormStyles {
    FormStyles {
        completed: Style::default(),
        active: Style::default(),
        hint: Style::default(),
    }
}

fn lines_text(lines: &[Line<'_>]) -> String {
    lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
}

/// EveryKeyInItsName (CreateTask in docs/specs/tasks.allium): every key is
/// a letter of the label it selects. `p` belongs to "[p]hoenix", so
/// PrReview is advertised as "pr-re[v]iew" and Fix as "fi[x]".
#[test]
fn input_tag_lines_puts_every_key_inside_its_own_label() {
    let app = crate::tui::App::new(vec![]);
    let text = lines_text(&input_tag_lines(
        &app,
        Rect::new(0, 0, 80, 20),
        &form_styles(),
    ));

    for label in [
        "[b]ug",
        "[f]eature",
        "[c]hore",
        "pr-re[v]iew",
        "[r]esearch",
        "fi[x]",
        "[p]hoenix",
        "[Enter] none",
    ] {
        assert!(text.contains(label), "missing {label} in:\n{text}");
    }
    assert!(
        !text.contains("[p]r-review"),
        "p belongs to phoenix now:\n{text}"
    );
    assert!(
        !text.contains("[x]fix"),
        "the key sits inside fi[x]:\n{text}"
    );
}

/// PhoenixArming's second pass: the same step re-opened, with `p` gone from
/// the prompt and the armed flag shown as a settled line above it.
#[test]
fn input_tag_lines_drops_the_p_key_and_shows_phoenix_once_armed() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(crate::tui::TaskDraft {
        title: "Weekly dep audit".to_string(),
        phoenix: true,
        ..Default::default()
    });

    let text = lines_text(&input_tag_lines(
        &app,
        Rect::new(0, 0, 80, 20),
        &form_styles(),
    ));

    assert!(text.contains("Phoenix: yes"), "got:\n{text}");
    assert!(
        !text.contains("[p]hoenix"),
        "the second pass must not re-offer p:\n{text}"
    );
    assert!(text.contains("[b]ug"), "got:\n{text}");
    assert!(text.contains("[Enter] none"), "got:\n{text}");
    assert!(text.contains("[Esc] cancel"), "got:\n{text}");
}

/// `input_panel_height` (kanban/mod.rs) reserves rows from
/// `PHOENIX_ARMED_TAG_STEP_LINES` rather than building this `Vec` on its
/// per-frame path, so nothing but this test stops the two drifting apart.
/// A step that grew a settled line without the constant following would
/// push `[Esc] cancel` outside the panel border.
#[test]
fn input_tag_lines_returns_a_fixed_line_count_per_variant() {
    let empty = crate::tui::App::new(vec![]);
    assert_eq!(
        input_tag_lines(&empty, Rect::new(0, 0, 80, 20), &form_styles()).len(),
        (PHOENIX_ARMED_TAG_STEP_LINES - 1) as usize,
        "unarmed: title + active + blank + hint"
    );

    // Draft content changes what a line says, never how many there are.
    for phoenix in [false, true] {
        let mut app = crate::tui::App::new(vec![]);
        app.input.task_draft = Some(crate::tui::TaskDraft {
            title: "My task".to_string(),
            tag: Some(crate::models::TaskTag::Bug),
            repo_path: "/some/repo".to_string(),
            phoenix,
            ..Default::default()
        });
        let expected = if phoenix {
            PHOENIX_ARMED_TAG_STEP_LINES
        } else {
            PHOENIX_ARMED_TAG_STEP_LINES - 1
        };
        assert_eq!(
            input_tag_lines(&app, Rect::new(0, 0, 80, 20), &form_styles()).len(),
            expected as usize,
            "phoenix armed: {phoenix}"
        );
    }
}

/// Both prompts are built from one `tag_keys!` list, so they can only
/// differ by the phoenix segment. This is what says so out loud.
#[test]
fn the_two_tag_prompts_differ_only_by_the_phoenix_key() {
    assert_eq!(
        tag_prompt(false).replace("  [p]hoenix", ""),
        tag_prompt(true),
        "the armed prompt is the unarmed one minus [p]hoenix"
    );
    assert_ne!(tag_prompt(false), tag_prompt(true));
}

#[test]
fn input_tag_lines_omits_the_phoenix_line_when_not_armed() {
    let app = crate::tui::App::new(vec![]);
    let text = lines_text(&input_tag_lines(
        &app,
        Rect::new(0, 0, 80, 20),
        &form_styles(),
    ));

    assert!(!text.contains("Phoenix: yes"), "got:\n{text}");
}

/// Once armed, the flag rides the settled tag line through every later
/// step, so no step grows an extra line to carry it.
#[test]
fn draft_summary_marks_an_armed_phoenix_on_the_tag_line() {
    let input = input_with_draft(crate::tui::TaskDraft {
        tag: Some(crate::models::TaskTag::Bug),
        phoenix: true,
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.tag, "bug (phoenix)");
}

#[test]
fn draft_summary_marks_an_armed_phoenix_with_no_tag() {
    let input = input_with_draft(crate::tui::TaskDraft {
        tag: None,
        phoenix: true,
        ..Default::default()
    });
    let summary = DraftSummary::from_input(&input);

    assert_eq!(summary.tag, "none (phoenix)");
}

/// The wrap-up picker is the form's last step now. It restates the four
/// answered steps above it, and shows the armed flag through the tag line.
#[test]
fn input_wrap_up_mode_lines_carry_the_armed_phoenix() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(crate::tui::TaskDraft {
        title: "My task".to_string(),
        tag: Some(crate::models::TaskTag::Bug),
        repo_path: "/some/repo".to_string(),
        base_branch: "main".to_string(),
        phoenix: true,
        ..Default::default()
    });

    let text = lines_text(&input_wrap_up_mode_lines(
        &app,
        Rect::new(0, 0, 80, 20),
        &form_styles(),
    ));

    assert!(text.contains("Title: My task"), "got:\n{text}");
    assert!(text.contains("Tag: bug (phoenix)"), "got:\n{text}");
    assert!(text.contains("Repo: /some/repo"), "got:\n{text}");
    assert!(text.contains("Base branch: main"), "got:\n{text}");
    assert!(text.contains("Wrap-up:"), "got:\n{text}");
    assert!(text.contains("[Esc] cancel"), "got:\n{text}");
}

// ---- SettledLinesStayOneRow (docs/specs/tasks.allium: CreateTask) ------

fn long_draft() -> crate::tui::TaskDraft {
    crate::tui::TaskDraft {
        title: "t".repeat(300),
        description: "dictated words ".repeat(60),
        repo_path: "/repo/".to_string() + &"r".repeat(300),
        base_branch: "b".repeat(300),
        ..Default::default()
    }
}

fn assert_one_row_each(lines: &[Line<'_>], width: usize) {
    for line in lines {
        let text = line_text(line);
        assert!(
            text.chars().count() <= width,
            "line wider than {width}: {text}"
        );
    }
}

#[test]
fn repo_path_step_settled_lines_fit_one_row() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(long_draft());
    let area = Rect::new(0, 0, 80, 20);
    let lines = input_repo_path_lines(&app, area, &form_styles());
    assert_one_row_each(&lines[..4], 78);
    assert!(line_text(&lines[2]).starts_with("  Description: dictated"));
    assert!(line_text(&lines[2]).ends_with('…'));
}

#[test]
fn base_branch_step_settled_lines_fit_one_row() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(long_draft());
    let area = Rect::new(0, 0, 80, 20);
    let lines = input_base_branch_lines(&app, area, &form_styles());
    assert_one_row_each(&lines[..5], 78);
    assert!(line_text(&lines[3]).starts_with("  Repo path: /repo/"));
}

#[test]
fn wrap_up_step_settled_lines_fit_one_row() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(long_draft());
    let area = Rect::new(0, 0, 80, 20);
    let lines = input_wrap_up_mode_lines(&app, area, &form_styles());
    assert_one_row_each(&lines[..4], 78);
    assert!(line_text(&lines[3]).starts_with("  Base branch: bbb"));
}

#[test]
fn tag_description_and_epic_steps_settled_lines_fit_one_row() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(long_draft());
    app.input.epic_draft = Some(crate::tui::EpicDraft {
        title: "e".repeat(300),
        ..Default::default()
    });
    let area = Rect::new(0, 0, 80, 20);
    let tag = input_tag_lines(&app, area, &form_styles());
    assert_one_row_each(&tag[..1], 78);
    let desc = input_description_lines(&app, area, &form_styles());
    assert_one_row_each(&desc[..2], 78);
    let epic = input_epic_description_lines(&app, area, &form_styles());
    assert_one_row_each(&epic[..1], 78);
}

#[test]
fn short_settled_values_are_not_clipped() {
    let mut app = crate::tui::App::new(vec![]);
    app.input.task_draft = Some(crate::tui::TaskDraft {
        title: "Short".to_string(),
        description: "Small".to_string(),
        ..Default::default()
    });
    let lines = input_repo_path_lines(&app, Rect::new(0, 0, 80, 20), &form_styles());
    assert_eq!(line_text(&lines[0]), "  Title: Short");
    assert_eq!(line_text(&lines[2]), "  Description: Small");
}

// ---- append_repo_path_list -------------------------------------------

#[test]
fn repo_list_no_scroll_when_all_fit() {
    let hint = Style::default();
    let paths = owned(&["a", "b", "c"]);
    let mut lines: Vec<Line> = Vec::new();
    // visible = area_height - height_offset = 10; 3 <= 10 → scroll = 0.
    append_repo_path_list(&mut lines, &paths, 1, &ctx(0, 10, hint));

    assert_eq!(lines.len(), 3);
    assert_eq!(line_text(&lines[0]), "    a");
    // Cursor row is rendered as "  ► " + path across two spans.
    assert_eq!(line_text(&lines[1]), "  ► b");
    assert_eq!(lines[1].spans.len(), 2);
    assert_eq!(line_text(&lines[2]), "    c");
}

#[test]
fn repo_list_scrolls_to_keep_cursor_visible() {
    let hint = Style::default();
    let paths = owned(&["p0", "p1", "p2", "p3", "p4"]);
    let mut lines: Vec<Line> = Vec::new();
    // visible = 3, cursor at last item → scroll = 4.sat_sub(2)=2, min(5-3=2)=2.
    append_repo_path_list(&mut lines, &paths, 4, &ctx(0, 3, hint));

    assert_eq!(lines.len(), 3);
    assert_eq!(line_text(&lines[0]), "    p2");
    assert_eq!(line_text(&lines[1]), "    p3");
    assert_eq!(line_text(&lines[2]), "  ► p4");
}

#[test]
fn repo_list_visible_height_floors_at_one() {
    let hint = Style::default();
    let paths = owned(&["p0", "p1", "p2"]);
    let mut lines: Vec<Line> = Vec::new();
    // height_offset >= area_height → saturating_sub is 0, floored to 1 visible row.
    append_repo_path_list(&mut lines, &paths, 2, &ctx(5, 3, hint));

    assert_eq!(lines.len(), 1);
    assert_eq!(line_text(&lines[0]), "  ► p2");
}

// ---- append_filtered_repos_with_new_entry ----------------------------

#[test]
fn filtered_new_entry_highlighted_when_cursor_past_list() {
    let hint = Style::default();
    let filtered = owned(&["a", "b"]);
    let mut lines: Vec<Line> = Vec::new();
    // buffer "c" is new; cursor == filtered.len() selects the new entry.
    append_filtered_repos_with_new_entry(&mut lines, &filtered, "c", 2, &ctx(1, 40, hint));

    // Two existing paths + the highlighted new entry.
    assert_eq!(lines.len(), 3);
    let last = &lines[2];
    assert_eq!(last.spans.len(), 3);
    assert_eq!(line_text(last), "  ► c  (new)");
    // With the cursor on the new entry, the list itself must not also
    // highlight a row (scroll_cursor clamps back to filtered.len()-1 = 1).
    assert_eq!(line_text(&lines[1]), "  ► b");
}

#[test]
fn filtered_new_entry_dimmed_when_cursor_in_list() {
    let hint = Style::default();
    let filtered = owned(&["a", "b"]);
    let mut lines: Vec<Line> = Vec::new();
    // buffer "c" is new but cursor points at an existing row.
    append_filtered_repos_with_new_entry(&mut lines, &filtered, "c", 0, &ctx(1, 40, hint));

    assert_eq!(lines.len(), 3);
    assert_eq!(line_text(&lines[0]), "  ► a");
    assert_eq!(line_text(&lines[2]), "    + c");
}

#[test]
fn filtered_no_new_entry_for_empty_buffer() {
    let hint = Style::default();
    let filtered = owned(&["a", "b"]);
    let mut lines: Vec<Line> = Vec::new();
    // Empty buffer → no "new path" entry offered.
    append_filtered_repos_with_new_entry(&mut lines, &filtered, "", 0, &ctx(1, 40, hint));

    assert_eq!(lines.len(), 2);
}

#[test]
fn filtered_no_new_entry_when_buffer_matches_existing() {
    let hint = Style::default();
    let filtered = owned(&["a", "b"]);
    let mut lines: Vec<Line> = Vec::new();
    // buffer exactly equals a filtered path → not a "new" entry.
    append_filtered_repos_with_new_entry(&mut lines, &filtered, "b", 1, &ctx(1, 40, hint));

    assert_eq!(lines.len(), 2);
}

//! The board-filter overlay's tabs (`board-layout.allium`, "Board Filter
//! Overlay Layout").

use super::*;
use crate::models::{EpicId, TaskStatus, WorkPackageId};
use crate::tui::types::{FilterEpicMark, FilterTab};
use crossterm::event::KeyCode;

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(make_key(code));
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn epic(id: i64, parent: Option<i64>, title: &str, status: TaskStatus) -> crate::models::Epic {
    crate::models::Epic {
        parent_epic_id: parent.map(EpicId),
        title: title.to_string(),
        status,
        ..make_epic(id)
    }
}

/// Root 1 "Alpha" → child 2 "Beta"; root 3 "Gamma" done; packages 7 and 8.
fn overlay_app() -> App {
    let mut app = App::new(vec![]);
    app.board.repo_paths = vec!["/repo/a".into(), "/repo/b".into()];
    app.board.epics = vec![
        epic(1, None, "Alpha", TaskStatus::Running),
        epic(2, Some(1), "Beta", TaskStatus::Backlog),
        epic(3, None, "Gamma", TaskStatus::Done),
    ];
    app.board.work_packages = vec![make_package(7, "P7"), make_package(8, "P8")];
    press(&mut app, KeyCode::Char('f'));
    app
}

fn epic_rows(app: &App) -> Vec<(EpicId, usize)> {
    app.filter_epic_rows()
        .iter()
        .map(|r| (r.id, r.depth))
        .collect()
}

#[test]
fn the_overlay_opens_on_the_repos_tab() {
    let app = overlay_app();
    assert_eq!(app.input.mode, InputMode::BoardFilter);
    assert_eq!(app.filter_overlay.tab, FilterTab::Repos);
}

#[test]
fn tab_and_back_tab_switch_tabs_and_wrap() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.filter_overlay.tab, FilterTab::Epics);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.filter_overlay.tab, FilterTab::Packages);
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.filter_overlay.tab, FilterTab::Repos);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.filter_overlay.tab, FilterTab::Packages);
    assert_eq!(
        app.filter.lens,
        crate::tui::types::Lens::Everything,
        "Tab in the overlay does not touch the lens"
    );
}

#[test]
fn each_tab_keeps_its_own_cursor() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.filter_overlay.epic_cursor, 0);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.input.repo_cursor, 1);
}

#[test]
fn i_switches_include_and_exclude_on_the_repos_tab_only() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Char('i'));
    assert_eq!(app.filter.mode, RepoFilterMode::Exclude);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('i'));
    assert_eq!(app.filter.mode, RepoFilterMode::Exclude);
}

#[test]
fn space_toggles_the_repo_under_the_cursor_on_the_repos_tab() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Char(' '));
    assert!(app.filter.repos.contains("/repo/a"));
}

#[test]
fn digits_toggle_repos_only_on_the_repos_tab() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('2'));
    assert!(app.filter.repos.is_empty());
    press(&mut app, KeyCode::BackTab);
    press(&mut app, KeyCode::Char('2'));
    assert!(app.filter.repos.contains("/repo/b"));
}

#[test]
fn the_epics_tab_lists_not_done_epics_as_an_indented_tree() {
    let app = unfolded_epics_tab();
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0), (EpicId(2), 1)]);
}

#[test]
fn the_epics_tab_starts_with_every_epic_folded() {
    let app = epics_tab();
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
    assert!(app.filter_epic_rows()[0].folded);
}

#[test]
fn a_nested_parent_starts_folded_when_its_parent_is_unfolded() {
    let mut app = overlay_app();
    app.board
        .epics
        .push(epic(4, Some(2), "Delta", TaskStatus::Running));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0), (EpicId(2), 1)]);
    assert!(app.filter_epic_rows()[1].folded);
}

#[test]
fn an_epic_that_arrives_while_the_overlay_is_open_starts_folded() {
    let mut app = unfolded_epics_tab();
    app.board
        .epics
        .push(epic(4, None, "Delta", TaskStatus::Running));
    app.board
        .epics
        .push(epic(5, Some(4), "Echo", TaskStatus::Running));
    let rows = app.filter_epic_rows();
    let delta = rows.iter().find(|r| r.id == EpicId(4)).unwrap();
    assert!(delta.folded);
    assert!(!rows.iter().any(|r| r.id == EpicId(5)));
}

#[test]
fn d_shows_done_epics_too() {
    let mut app = unfolded_epics_tab();
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(
        epic_rows(&app),
        vec![(EpicId(1), 0), (EpicId(2), 1), (EpicId(3), 0)]
    );
    press(&mut app, KeyCode::Char('d'));
    assert_eq!(epic_rows(&app).len(), 2);
}

#[test]
fn a_hidden_done_epic_stays_listed_so_it_can_be_shown_again() {
    let mut app = overlay_app();
    app.filter.hidden_epics.insert(EpicId(3));
    press(&mut app, KeyCode::Tab);
    assert!(epic_rows(&app).contains(&(EpicId(3), 0)));
}

#[test]
fn a_not_done_child_of_a_done_parent_keeps_its_parent_row() {
    let mut app = overlay_app();
    app.board
        .epics
        .push(epic(4, Some(3), "Delta", TaskStatus::Running));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('Z'));
    let rows = epic_rows(&app);
    assert!(rows.contains(&(EpicId(3), 0)));
    assert!(rows.contains(&(EpicId(4), 1)));
}

#[test]
fn space_on_the_epics_tab_hides_and_shows_the_epic_under_the_cursor() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char(' '));
    assert!(app.filter.hidden_epics.contains(&EpicId(1)));
    press(&mut app, KeyCode::Char(' '));
    assert!(app.filter.hidden_epics.is_empty());
}

#[test]
fn a_child_of_a_hidden_parent_is_marked_hidden_by_parent() {
    let mut app = overlay_app();
    app.filter.hidden_epics.insert(EpicId(1));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('Z'));
    let marks: Vec<_> = app.filter_epic_rows().iter().map(|r| r.mark).collect();
    assert_eq!(
        marks,
        vec![FilterEpicMark::Hidden, FilterEpicMark::HiddenByParent]
    );
}

#[test]
fn showing_a_parent_again_keeps_a_child_hidden_on_its_own() {
    let mut app = overlay_app();
    app.filter.hidden_epics.extend([EpicId(1), EpicId(2)]);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.filter.hidden_epics, [EpicId(2)].into_iter().collect());
}

#[test]
fn slash_filters_the_epic_tree_by_title_and_keeps_ancestors() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.input.mode, InputMode::BoardFilterSearch);
    type_str(&mut app, "bet");
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0), (EpicId(2), 1)]);
    type_str(&mut app, "x");
    assert!(epic_rows(&app).is_empty());
}

#[test]
fn letters_typed_into_the_epic_search_are_text_not_commands() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "dqi");
    assert_eq!(app.filter_overlay.epic_query, "dqi");
    assert_eq!(app.input.mode, InputMode::BoardFilterSearch);
    assert!(!app.filter_overlay.show_done_epics);
    assert_eq!(app.filter.mode, RepoFilterMode::Include);
}

#[test]
fn enter_keeps_the_query_and_returns_to_the_list() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "gam");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.input.mode, InputMode::BoardFilter);
    assert_eq!(app.filter_overlay.epic_query, "gam");
}

#[test]
fn esc_clears_the_epic_query_first_then_closes_the_overlay() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "al");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.input.mode, InputMode::BoardFilter);
    assert_eq!(app.filter_overlay.epic_query, "");

    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "al");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.input.mode, InputMode::BoardFilter, "first Esc clears");
    assert_eq!(app.filter_overlay.epic_query, "");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.input.mode, InputMode::Normal, "second Esc closes");
}

#[test]
fn space_on_the_packages_tab_hides_the_package_under_the_cursor() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::BackTab);
    assert_eq!(app.filter_overlay.tab, FilterTab::Packages);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.filter.hidden_packages,
        [WorkPackageId(8)].into_iter().collect()
    );
}

#[test]
fn reopening_the_overlay_starts_on_repos_with_no_epic_query() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "al");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('q'));
    press(&mut app, KeyCode::Char('f'));
    assert_eq!(app.filter_overlay.tab, FilterTab::Repos);
    assert_eq!(app.filter_overlay.epic_query, "");
}

fn epics_tab() -> App {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Tab);
    app
}

/// The Epics tab with every epic unfolded by `Z`.
fn unfolded_epics_tab() -> App {
    let mut app = epics_tab();
    press(&mut app, KeyCode::Char('Z'));
    app
}

#[test]
fn z_folds_the_epic_under_the_cursor_and_hides_its_subtree() {
    let mut app = unfolded_epics_tab();
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
    assert!(app.filter_epic_rows()[0].folded);
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0), (EpicId(2), 1)]);
}

#[test]
fn z_on_a_leaf_does_nothing() {
    let mut app = unfolded_epics_tab();
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(epic_rows(&app).len(), 2);
}

#[test]
fn z_does_nothing_on_the_other_tabs() {
    let mut app = overlay_app();
    press(&mut app, KeyCode::Char('z'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
}

#[test]
fn capital_z_unfolds_all_then_folds_every_parent() {
    let mut app = epics_tab();
    press(&mut app, KeyCode::Char('Z'));
    assert_eq!(epic_rows(&app).len(), 2);
    press(&mut app, KeyCode::Char('Z'));
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
}

#[test]
fn capital_z_unfolds_all_while_any_parent_is_folded() {
    let mut app = overlay_app();
    app.board
        .epics
        .push(epic(4, None, "Delta", TaskStatus::Running));
    app.board
        .epics
        .push(epic(5, Some(4), "Echo", TaskStatus::Running));
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char('z'));
    press(&mut app, KeyCode::Char('Z'));
    assert_eq!(epic_rows(&app).len(), 4);
}

#[test]
fn folding_does_not_change_what_is_hidden() {
    let mut app = unfolded_epics_tab();
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('z'));
    assert!(app.filter.hidden_epics.contains(&EpicId(1)));
    press(&mut app, KeyCode::Char('z'));
    assert_eq!(
        app.filter_epic_rows()[1].mark,
        FilterEpicMark::HiddenByParent
    );
}

#[test]
fn a_standing_query_ignores_folds() {
    let mut app = epics_tab();
    press(&mut app, KeyCode::Char('/'));
    type_str(&mut app, "bet");
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0), (EpicId(2), 1)]);
    press(&mut app, KeyCode::Esc);
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
}

#[test]
fn reopening_the_overlay_folds_everything_again() {
    let mut app = unfolded_epics_tab();
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('f'));
    press(&mut app, KeyCode::Tab);
    assert_eq!(epic_rows(&app), vec![(EpicId(1), 0)]);
}

#[test]
fn the_epics_tab_draws_fold_markers() {
    let mut app = unfolded_epics_tab();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(buffer_contains(&buf, "▾"));
    press(&mut app, KeyCode::Char('z'));
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(buffer_contains(&buf, "▸"));
}

#[test]
fn the_repos_footer_names_the_forget_key() {
    let mut app = overlay_app();
    let buf = render_to_buffer(&mut app, 100, 30);
    assert!(buffer_contains(&buf, "[Del]"));
}

use super::*;

const HOST_A: &str = "host-a";
/// A Wednesday.
const NOW: &str = "2026-10-21 10:15:30.250";

fn backlog_task() -> Task {
    Task {
        id: 1,
        ..blank_task()
    }
}

fn running_task() -> Task {
    Task {
        status: "running".into(),
        ..backlog_task()
    }
}

// -- The cron grammar (schedules.allium: "Value") -----------------------------

#[test]
fn next_match_is_the_first_whole_minute_strictly_after() {
    assert_eq!(
        next_cron_match("* * * * *", NOW).unwrap(),
        "2026-10-21 10:16:00.000"
    );
    // Strictly after: a stamp ON a matching minute moves to the next one.
    assert_eq!(
        next_cron_match("* * * * *", "2026-10-21 10:16:00.000").unwrap(),
        "2026-10-21 10:17:00.000"
    );
}

#[test]
fn next_match_reads_the_fields_in_utc_order() {
    // Mondays at 09:00: the Monday after Wednesday 2026-10-21.
    assert_eq!(
        next_cron_match("0 9 * * 1", NOW).unwrap(),
        "2026-10-26 09:00:00.000"
    );
    // Later today.
    assert_eq!(
        next_cron_match("30 14 * * *", NOW).unwrap(),
        "2026-10-21 14:30:00.000"
    );
    // Already passed today, so tomorrow.
    assert_eq!(
        next_cron_match("0 9 * * *", NOW).unwrap(),
        "2026-10-22 09:00:00.000"
    );
    // First of next month.
    assert_eq!(
        next_cron_match("0 0 1 * *", NOW).unwrap(),
        "2026-11-01 00:00:00.000"
    );
    // A month name and a year rollover.
    assert_eq!(
        next_cron_match("0 0 1 jan *", NOW).unwrap(),
        "2027-01-01 00:00:00.000"
    );
}

#[test]
fn sunday_is_both_zero_and_seven_and_weekdays_take_names() {
    let sunday = "2026-10-25 08:00:00.000";
    assert_eq!(next_cron_match("0 8 * * 0", NOW).unwrap(), sunday);
    assert_eq!(next_cron_match("0 8 * * 7", NOW).unwrap(), sunday);
    assert_eq!(next_cron_match("0 8 * * sun", NOW).unwrap(), sunday);
    assert_eq!(next_cron_match("0 8 * * SUN", NOW).unwrap(), sunday);
}

#[test]
fn ranges_lists_and_steps() {
    // Weekdays at 09:00, from Wednesday 10:15: Thursday.
    assert_eq!(
        next_cron_match("0 9 * * 1-5", NOW).unwrap(),
        "2026-10-22 09:00:00.000"
    );
    // Every 20 minutes: 10:20.
    assert_eq!(
        next_cron_match("*/20 * * * *", NOW).unwrap(),
        "2026-10-21 10:20:00.000"
    );
    // A list.
    assert_eq!(
        next_cron_match("5,45 10 * * *", NOW).unwrap(),
        "2026-10-21 10:45:00.000"
    );
    // A stepped range: 10:00-10:59 every 15 from 10, so 10:25.
    assert_eq!(
        next_cron_match("10-59/15 10 * * *", NOW).unwrap(),
        "2026-10-21 10:25:00.000"
    );
}

#[test]
fn a_restricted_day_of_month_and_day_of_week_match_on_either() {
    // The 1st, or any Friday: Friday 2026-10-23 comes first.
    assert_eq!(
        next_cron_match("0 0 1 * 5", NOW).unwrap(),
        "2026-10-23 00:00:00.000"
    );
    // With the weekday unrestricted, only the day of month counts.
    assert_eq!(
        next_cron_match("0 0 1 * *", NOW).unwrap(),
        "2026-11-01 00:00:00.000"
    );
}

#[test]
fn a_leap_day_schedule_finds_the_next_leap_year() {
    assert_eq!(
        next_cron_match("0 0 29 2 *", NOW).unwrap(),
        "2028-02-29 00:00:00.000"
    );
}

#[test]
fn a_cron_that_never_matches_is_refused() {
    let why = validate_cron("0 0 31 2 *").unwrap_err();
    assert!(why.contains("never matches"), "{why}");
}

#[test]
fn malformed_crons_are_refused_with_the_cron_named() {
    for bad in [
        "",
        "* * * *",
        "* * * * * *",
        "60 * * * *",
        "* 24 * * *",
        "* * 0 * *",
        "* * 32 * *",
        "* * * 13 *",
        "* * * * 8",
        "*/0 * * * *",
        "5-1 * * * *",
        "@weekly",
        "x * * * *",
        "* * L * *",
    ] {
        let why = validate_cron(bad).unwrap_err();
        assert!(
            why.starts_with(&format!("invalid cron schedule {bad:?}")),
            "{bad}: {why}"
        );
    }
}

// -- SetSchedule / ClearSchedule ----------------------------------------------

#[test]
fn setting_a_schedule_on_a_backlog_task_arms_it_from_now() {
    let task = Task {
        schedule_error: "boom".into(),
        ..backlog_task()
    };
    let set = apply_schedule(task, "0 9 * * 1", NOW).unwrap();
    assert_eq!(set.schedule, "0 9 * * 1");
    assert_eq!(set.next_due, "2026-10-26 09:00:00.000");
    assert_eq!(set.schedule_error, "", "setting again clears a failure");
}

#[test]
fn a_non_phoenix_task_outside_backlog_is_refused_a_schedule() {
    let why = apply_schedule(running_task(), "0 9 * * 1", NOW).unwrap_err();
    assert_eq!(why, "a schedule on a non-phoenix task needs it in backlog");
}

#[test]
fn a_phoenix_task_outside_backlog_stores_the_schedule_unarmed() {
    let task = Task {
        phoenix: true,
        ..running_task()
    };
    let set = apply_schedule(task, "0 9 * * 1", NOW).unwrap();
    assert_eq!(set.schedule, "0 9 * * 1");
    assert_eq!(set.next_due, "", "the successor arms it, not this run");
}

#[test]
fn an_invalid_cron_is_refused() {
    assert!(apply_schedule(backlog_task(), "every monday", NOW).is_err());
}

#[test]
fn clearing_drops_all_three_fields_in_any_status() {
    let task = Task {
        schedule: "0 9 * * 1".into(),
        next_due: "2026-10-26 09:00:00.000".into(),
        schedule_error: "boom".into(),
        ..running_task()
    };
    let cleared = apply_schedule(task, "", NOW).unwrap();
    assert_eq!(
        (cleared.schedule, cleared.next_due, cleared.schedule_error),
        (String::new(), String::new(), String::new())
    );
}

#[test]
fn a_created_task_is_armed_from_its_own_schedule_and_nothing_else() {
    // A client cannot plant its own next_due or error on a create.
    let row = Task {
        schedule: "0 9 * * 1".into(),
        next_due: "2020-01-01 00:00:00.000".into(),
        schedule_error: "planted".into(),
        ..backlog_task()
    };
    let armed = arm_created_task(row, NOW).unwrap();
    assert_eq!(armed.next_due, "2026-10-26 09:00:00.000");
    assert_eq!(armed.schedule_error, "");

    let plain = Task {
        next_due: "2020-01-01 00:00:00.000".into(),
        ..backlog_task()
    };
    let armed = arm_created_task(plain, NOW).unwrap();
    assert_eq!(
        (armed.next_due, armed.schedule_error),
        (String::new(), String::new())
    );
}

// -- Due, and the dispatch request (store-split.allium: RequestDispatch) -----

fn due_task() -> Task {
    Task {
        schedule: "0 9 * * 1".into(),
        next_due: "2026-10-19 09:00:00.000".into(),
        ..backlog_task()
    }
}

#[test]
fn a_task_is_due_once_its_next_due_has_passed() {
    assert!(task_is_due(&due_task(), NOW));
    assert!(!task_is_due(&backlog_task(), NOW));
    let ahead = Task {
        next_due: "2026-10-26 09:00:00.000".into(),
        ..due_task()
    };
    assert!(!task_is_due(&ahead, NOW));
}

#[test]
fn a_schedule_origin_request_needs_a_due_task() {
    assert!(validate_dispatch_request(&due_task(), HOST_A, "schedule", false, NOW).is_ok());
    let ahead = Task {
        next_due: "2026-10-26 09:00:00.000".into(),
        ..due_task()
    };
    let why = validate_dispatch_request(&ahead, HOST_A, "schedule", false, NOW).unwrap_err();
    assert_eq!(why, "task 1 is not due");
    let why =
        validate_dispatch_request(&backlog_task(), HOST_A, "schedule", false, NOW).unwrap_err();
    assert_eq!(why, "task 1 is not due");
}

#[test]
fn the_epic_chain_passes_over_a_task_still_waiting_for_its_time() {
    let ahead = Task {
        next_due: "2026-10-26 09:00:00.000".into(),
        ..due_task()
    };
    let why = validate_dispatch_request(&ahead, HOST_A, "epic_chain", false, NOW).unwrap_err();
    assert_eq!(
        why,
        "task 1 is waiting for its scheduled time, which the epic chain does not dispatch early"
    );
    // A due one, it may take; and every other origin may launch early.
    assert!(validate_dispatch_request(&due_task(), HOST_A, "epic_chain", false, NOW).is_ok());
    assert!(validate_dispatch_request(&ahead, HOST_A, "user", false, NOW).is_ok());
}

#[test]
fn schedule_is_a_dispatch_origin() {
    assert!(validate_dispatch_origin("schedule").is_ok());
}

// -- DispatchConsumesTheDueTime -----------------------------------------------

#[test]
fn a_dispatched_report_consumes_the_due_time_and_clears_a_failure() {
    let task = Task {
        schedule_error: "boom".into(),
        ..due_task()
    };
    let row = DispatchRequest {
        task_id: 1,
        host: HOST_A.into(),
        worktree: "/wt".into(),
        finished_at: NOW.into(),
        ..blank_dispatch_request()
    };
    let after = apply_dispatched_to_task(task, &row);
    assert_eq!(after.next_due, "");
    assert_eq!(after.schedule_error, "");
    assert_eq!(
        after.schedule, "0 9 * * 1",
        "kept for the phoenix successor"
    );
}

// -- ScheduledLaunchFails -----------------------------------------------------

#[test]
fn a_failed_scheduled_one_off_is_marked_and_disarmed() {
    let after = schedule_after_failed_launch(due_task(), "schedule", "repo missing", NOW).unwrap();
    assert_eq!(after.schedule_error, "repo missing");
    assert_eq!(after.next_due, "");
}

#[test]
fn a_failed_scheduled_phoenix_is_marked_and_rearmed_from_now() {
    let task = Task {
        phoenix: true,
        ..due_task()
    };
    let after = schedule_after_failed_launch(task, "schedule", "repo missing", NOW).unwrap();
    assert_eq!(after.schedule_error, "repo missing");
    assert_eq!(after.next_due, "2026-10-26 09:00:00.000");
}

#[test]
fn a_failed_launch_of_another_origin_leaves_the_schedule_alone() {
    assert!(schedule_after_failed_launch(due_task(), "user", "repo missing", NOW).is_none());
}

// -- PhoenixRespawn carries the schedule (tasks.allium) -----------------------

#[test]
fn a_phoenix_successor_inherits_the_schedule_due_from_the_respawn() {
    let done = Task {
        phoenix: true,
        status: "done".into(),
        schedule: "0 9 * * 1".into(),
        schedule_error: "old".into(),
        ..backlog_task()
    };
    // The successor row carries the schedule; its insert arms it from now.
    let successor = arm_created_task(phoenix_successor(&done, 0, NOW, "creator"), NOW).unwrap();
    assert_eq!(successor.schedule, "0 9 * * 1");
    assert_eq!(successor.next_due, "2026-10-26 09:00:00.000");
    assert_eq!(successor.schedule_error, "");

    let unscheduled = Task {
        schedule: String::new(),
        ..done
    };
    let successor =
        arm_created_task(phoenix_successor(&unscheduled, 0, NOW, "creator"), NOW).unwrap();
    assert_eq!(
        (successor.schedule, successor.next_due),
        (String::new(), String::new())
    );
}

// -- LeavingBacklogCancelsThePendingRun ---------------------------------------

#[test]
fn a_hand_move_out_of_backlog_cancels_the_pending_run() {
    let moved = Task {
        status: "review".into(),
        ..due_task()
    };
    let after = cancel_pending_run_on_leaving_backlog(BACKLOG, moved);
    assert_eq!(after.next_due, "");
    assert_eq!(after.schedule, "0 9 * * 1", "kept as a record");
}

#[test]
fn a_write_that_stays_in_backlog_or_starts_elsewhere_keeps_it() {
    let stays = cancel_pending_run_on_leaving_backlog(BACKLOG, due_task());
    assert_eq!(stays.next_due, "2026-10-19 09:00:00.000");
    let from_running = cancel_pending_run_on_leaving_backlog("running", due_task());
    assert_eq!(from_running.next_due, "2026-10-19 09:00:00.000");
}

#[test]
fn closing_a_scheduled_backlog_task_cancels_its_pending_run() {
    let closed = apply_done_close(due_task(), NOW);
    assert_eq!(closed.status, "done");
    assert_eq!(closed.next_due, "");
}

#[test]
fn a_failed_schedule_request_releases_and_marks_in_one_write() {
    let claimed = Task {
        status: "running".into(),
        ..due_task()
    };
    let (row, released) = task_after_failed_request(claimed, "schedule", "boom", NOW);
    let row = row.expect("the release and the mark are one write");
    assert!(released);
    assert_eq!(row.status, BACKLOG);
    assert_eq!(row.schedule_error, "boom");
    assert_eq!(row.next_due, "");
    assert_eq!(row.updated_at, NOW);
}

#[test]
fn a_failed_request_that_changes_nothing_writes_nothing() {
    // Provisioned meanwhile, so no bare claim to release; another origin.
    let moved_on = Task {
        status: "running".into(),
        worktree: "/wt".into(),
        ..due_task()
    };
    assert!(matches!(
        task_after_failed_request(moved_on, "user", "boom", NOW),
        (None, false)
    ));
}

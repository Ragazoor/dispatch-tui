//! Scheduled tasks: the cron grammar and what each event does to a task's
//! schedule.
//!
//! Spec: `docs/specs/schedules.allium` (task #98394), plus the two rules it
//! writes into others: `store-split.allium: RequestDispatch` (the
//! schedule-origin and epic-chain checks, in [`validate_dispatch_request`]) and
//! `tasks.allium: PhoenixRespawn` (the successor's schedule, in
//! [`phoenix_successor`]).
//!
//! Every function here is pure, for the reason `rules.rs` gives: the in-memory
//! reducer caller calls the same ones, so the two cannot disagree. The module
//! is the only place a due time is COMPUTED; a board only compares one against
//! its clock (`schedules.allium: FireDueTask`).
//!
//! WHY A HAND-WRITTEN PARSER: the cron crates read the weekday field
//! differently from standard cron (1 = Sunday, or AND-ing a restricted day of
//! month with a restricted weekday), and the croner crate turns on chrono's
//! `clock` feature, which pulls `wasm-bindgen` into a module that must not
//! import from JavaScript. Five fields of standard cron are small enough to own.

use super::*;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

/// The dispatch origin of a board firing a due task (`FireDueTask`).
pub const DISPATCH_ORIGIN_SCHEDULE: &str = "schedule";

/// How many days ahead [`next_cron_match`] looks: more than the longest gap
/// between two matches of any cron the grammar accepts, so a cron with no
/// match in the window has none ever, from any start. That gap is measured
/// over every (month, day of month, weekday) triple in the 400-year Gregorian
/// cycle: 14609 days, e.g. `0 0 29 2 */7` (29 February on a Sunday, the two
/// day fields AND-ed) matches in 2088 and next in 2128, across the leap day
/// 2100 skips. Hours and minutes cannot lengthen it: every matching day has a
/// matching minute. Cheap, as the search steps by day.
const SEARCH_DAYS: i64 = 366 * 40;

/// The instant [`validate_cron`] searches from. Any instant would do: from
/// any of them every satisfiable cron matches within [`SEARCH_DAYS`].
const VALIDATION_ORIGIN: &str = "2000-01-01 00:00:00.000";

/// A parsed five-field cron. Each set holds the values that field matches,
/// as bits.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Cron {
    minutes: u64,
    hours: u64,
    days_of_month: u64,
    months: u64,
    days_of_week: u64,
    /// Whether the field was written starting with `*`. Standard cron matches
    /// a day on day of month OR weekday only when BOTH are restricted.
    day_of_month_star: bool,
    day_of_week_star: bool,
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// One field's bounds and the names it accepts (`names[i]` is `min + i`).
struct FieldSpec {
    label: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
}

fn invalid(cron: &str, why: impl std::fmt::Display) -> String {
    format!("invalid cron schedule {cron:?}: {why}")
}

fn parse_value(text: &str, spec: &FieldSpec) -> Result<u32, String> {
    if let Some(i) = spec.names.iter().position(|n| n.eq_ignore_ascii_case(text)) {
        return Ok(spec.min + i as u32);
    }
    let value: u32 = text
        .parse()
        .map_err(|_| format!("{} {text:?} is not a number", spec.label))?;
    if value < spec.min || value > spec.max {
        return Err(format!(
            "{} {value} is outside {}-{}",
            spec.label, spec.min, spec.max
        ));
    }
    Ok(value)
}

/// Parse one field into a bit set over `spec.min..=spec.max`.
fn parse_field(text: &str, spec: &FieldSpec) -> Result<u64, String> {
    let mut bits = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| format!("{} step {step:?} is not a number", spec.label))?;
                if step == 0 {
                    return Err(format!("{} step must be at least 1", spec.label));
                }
                (range, Some(step))
            }
            None => (item, None),
        };
        let (low, high) = if range == "*" {
            (spec.min, spec.max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (parse_value(a, spec)?, parse_value(b, spec)?);
            if a > b {
                return Err(format!("{} range {a}-{b} runs backwards", spec.label));
            }
            (a, b)
        } else {
            if step.is_some() {
                return Err(format!(
                    "{} step needs `*` or a range before it, not {range:?}",
                    spec.label
                ));
            }
            let v = parse_value(range, spec)?;
            (v, v)
        };
        let step = step.unwrap_or(1);
        let mut v = low;
        while v <= high {
            bits |= 1 << v;
            // A step past `u32::MAX` is past every field's maximum too.
            match v.checked_add(step) {
                Some(next) => v = next,
                None => break,
            }
        }
    }
    Ok(bits)
}

/// The five fields, in order. `day of week` takes 0-7 so that 7 can mean
/// Sunday too; [`parse_cron`] folds it onto 0.
const FIELDS: [FieldSpec; 5] = [
    FieldSpec {
        label: "minute",
        min: 0,
        max: 59,
        names: &[],
    },
    FieldSpec {
        label: "hour",
        min: 0,
        max: 23,
        names: &[],
    },
    FieldSpec {
        label: "day of month",
        min: 1,
        max: 31,
        names: &[],
    },
    FieldSpec {
        label: "month",
        min: 1,
        max: 12,
        names: &MONTH_NAMES,
    },
    FieldSpec {
        label: "day of week",
        min: 0,
        max: 7,
        names: &WEEKDAY_NAMES,
    },
];

fn parse_cron(cron: &str) -> Result<Cron, String> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    let Ok(fields) = <[&str; 5]>::try_from(fields.as_slice()) else {
        return Err(invalid(
            cron,
            format!(
                "expected five fields (minute hour day-of-month month day-of-week), found {}",
                fields.len()
            ),
        ));
    };
    let mut sets = [0u64; 5];
    for ((set, text), spec) in sets.iter_mut().zip(fields).zip(&FIELDS) {
        *set = parse_field(text, spec).map_err(|e| invalid(cron, e))?;
    }
    let [minutes, hours, days_of_month, months, weekdays] = sets;
    Ok(Cron {
        minutes,
        hours,
        days_of_month,
        months,
        // 7 is Sunday too.
        days_of_week: (weekdays | (weekdays >> 7)) & 0x7f,
        day_of_month_star: fields[2].starts_with('*'),
        day_of_week_star: fields[4].starts_with('*'),
    })
}

impl Cron {
    fn day_matches(&self, date: NaiveDate) -> bool {
        if self.months & (1 << date.month()) == 0 {
            return false;
        }
        let dom = self.days_of_month & (1 << date.day()) != 0;
        let dow = self.days_of_week & (1 << date.weekday().num_days_from_sunday()) != 0;
        if self.day_of_month_star || self.day_of_week_star {
            dom && dow
        } else {
            dom || dow
        }
    }

    /// The first matching time of day at or after `from`, on a matching day.
    fn first_time_from(&self, from: NaiveTime) -> Option<NaiveTime> {
        (from.hour()..24)
            .filter(|h| self.hours & (1 << h) != 0)
            .find_map(|h| {
                let first_minute = if h == from.hour() { from.minute() } else { 0 };
                (first_minute..60)
                    .find(|m| self.minutes & (1 << m) != 0)
                    .and_then(|m| NaiveTime::from_hms_opt(h, m, 0))
            })
    }

    /// The first matching minute strictly after `after`.
    fn next_after(&self, after: NaiveDateTime) -> Option<NaiveDateTime> {
        let start = after.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let mut date = start.date();
        let mut from = start.time();
        for _ in 0..SEARCH_DAYS {
            if self.day_matches(date) {
                if let Some(time) = self.first_time_from(from) {
                    return Some(date.and_time(time));
                }
            }
            date = date.succ_opt()?;
            from = NaiveTime::MIN;
        }
        None
    }
}

/// Check a cron string against the grammar, and that it matches some minute.
/// `schedules.allium: SetSchedule`'s `cron_is_valid`.
pub fn validate_cron(cron: &str) -> Result<(), String> {
    next_cron_match(cron, VALIDATION_ORIGIN).map(|_| ())
}

/// `next_match(cron, after)`: the first whole UTC minute strictly after
/// `after` (the store's timestamp spelling) that `cron` matches, in the same
/// spelling.
pub fn next_cron_match(cron: &str, after: &str) -> Result<String, String> {
    let parsed = parse_cron(cron)?;
    let after = NaiveDateTime::parse_from_str(after, SQLITE_TIMESTAMP)
        .map_err(|e| format!("unreadable timestamp {after:?}: {e}"))?;
    parsed
        .next_after(after)
        .map(|at| at.format(SQLITE_TIMESTAMP).to_string())
        .ok_or_else(|| invalid(cron, "it never matches any date"))
}

/// `SetSchedule` for a non-empty `cron`, `ClearSchedule` for `""`.
pub fn apply_schedule(task: Task, cron: &str, now: &str) -> Result<Task, String> {
    if cron.is_empty() {
        return Ok(Task {
            schedule: String::new(),
            next_due: String::new(),
            schedule_error: String::new(),
            ..task
        });
    }
    // Validates too: a cron that is malformed or never matches has no match.
    let next_match = next_cron_match(cron, now)?;
    if task.status != BACKLOG && !task.phoenix {
        return Err("a schedule on a non-phoenix task needs it in backlog".into());
    }
    let next_due = if task.status == BACKLOG {
        next_match
    } else {
        String::new()
    };
    Ok(Task {
        schedule: cron.to_string(),
        next_due,
        schedule_error: String::new(),
        ..task
    })
}

/// A created row's schedule fields, from its `schedule` alone: a client cannot
/// plant a `next_due` or a `schedule_error` on a create.
pub fn arm_created_task(task: Task, now: &str) -> Result<Task, String> {
    let cron = task.schedule.clone();
    apply_schedule(task, &cron, now)
}

/// Whether `task`'s next due time has passed at `now`. Both are the store's
/// fixed-width spelling, so they compare as strings.
pub fn task_is_due(task: &Task, now: &str) -> bool {
    !task.next_due.is_empty() && task.next_due.as_str() <= now
}

/// Whether `task` is still waiting for a scheduled time ahead of `now`.
pub fn task_awaits_schedule(task: &Task, now: &str) -> bool {
    !task.next_due.is_empty() && task.next_due.as_str() > now
}

/// `LeavingBacklogCancelsThePendingRun`: a status write other than the
/// dispatch claim that takes the task out of backlog drops its pending run.
/// `prior_status` is the status before the write; `task` is the row after it.
/// The schedule and any recorded failure are kept.
pub fn cancel_pending_run_on_leaving_backlog(prior_status: &str, task: Task) -> Task {
    if prior_status == BACKLOG && task.status != BACKLOG {
        Task {
            next_due: String::new(),
            ..task
        }
    } else {
        task
    }
}

/// `ClosingCancelsThePendingRun` for a hand status write: a task written into
/// done is closed, so its pending run goes whatever the prior status. A
/// one-off claimed by a user-origin request keeps its due time until the
/// launch ends, and would otherwise fire at once when moved back to backlog.
pub fn cancel_pending_run_on_done(task: Task) -> Task {
    if task.status == DONE {
        Task {
            next_due: String::new(),
            ..task
        }
    } else {
        task
    }
}

/// `ScheduledLaunchFails`: a failed request of `origin` with `reason`, or
/// `None` when it changes nothing. Only a schedule-origin failure on a
/// scheduled task changes anything: it records the reason and disarms a
/// one-off, or re-arms a phoenix task from `now`.
pub fn schedule_after_failed_launch(
    task: Task,
    origin: &str,
    reason: &str,
    now: &str,
) -> Option<Task> {
    if origin != DISPATCH_ORIGIN_SCHEDULE || task.schedule.is_empty() {
        return None;
    }
    let next_due = if task.phoenix {
        // A stored schedule was validated when it was set, and a valid cron
        // matches within [`SEARCH_DAYS`] of any instant, so this answers.
        next_cron_match(&task.schedule, now).unwrap_or_default()
    } else {
        String::new()
    };
    Some(Task {
        schedule_error: reason.to_string(),
        next_due,
        ..task
    })
}

/// `EnteringBacklogRearmsAPhoenixTask`: a status write other than the respawn
/// that brings a scheduled phoenix task back into backlog arms it from `now`,
/// so a run that fired and was abandoned does not leave it unarmed for good.
/// `prior_status` is the status before the write; `task` is the row after it.
/// A recorded failure is kept.
pub fn rearm_phoenix_on_entering_backlog(prior_status: &str, task: Task, now: &str) -> Task {
    if prior_status == BACKLOG
        || task.status != BACKLOG
        || !task.phoenix
        || task.schedule.is_empty()
    {
        return task;
    }
    Task {
        // Answers for a stored schedule; see [`schedule_after_failed_launch`].
        next_due: next_cron_match(&task.schedule, now).unwrap_or_default(),
        ..task
    }
}

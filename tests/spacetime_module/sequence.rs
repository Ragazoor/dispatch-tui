use super::*;
use dispatch_tui::spacetime::SharedTable;

/// Every table a snapshot carries must be one `burn_id_sequence` accepts,
/// because `restore.rs::burn_id_sequences` calls it for each generating table.
/// A table added to `SharedTable` without an arm surfaced only as an operator's
/// failed import (`unknown table learning_verdicts`, 2026-10-26); the in-memory
/// `MemoryStore` accepts every table, so only the live module can catch it.
#[test]
fn every_shared_table_is_a_table_the_burn_accepts() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    for table in SharedTable::ALL {
        let out = instance.call("burn_id_sequence", &[&format!("\"{}\"", table.name()), "3"]);
        assert!(out.status.success(), "{}: {}", table.name(), describe(&out));
    }
}

/// The burn must move `learning_verdicts`' counter, not merely be accepted:
/// the next verdict a task records gets an id past the ceiling, so it cannot
/// collide with a restored one.
#[test]
fn burning_learning_verdicts_moves_its_counter_past_the_ceiling() {
    if !spacetime_available_or_skip() {
        return;
    }
    let instance = published_instance();
    let burned = instance.call("burn_id_sequence", &["\"learning_verdicts\"", "5"]);
    assert!(burned.status.success(), "{}", describe(&burned));

    let learning = serde_json::json!({
        "id": 0,
        "kind": "convention",
        "summary": "burn probe",
        "detail": {"none": []},
        "scope": "user",
        "scope_ref": {"none": []},
        "tags": "[]",
        "status": "approved",
        "source_task_id": {"none": []},
        "upvote_count": 0,
        "last_upvoted_at": {"none": []},
        "created_at": "2026-10-26 10:00:00",
        "updated_at": "2026-10-26 10:00:00",
        "embedding": {"none": []},
    });
    let created = instance.call("create_learning", &[&learning.to_string()]);
    assert!(created.status.success(), "{}", describe(&created));
    let learning_id = column(&instance, "SELECT id FROM learnings");

    let retrieved = instance.call("record_learning_retrieval", &["7", &learning_id, "\"rag\""]);
    assert!(retrieved.status.success(), "{}", describe(&retrieved));
    let verdicts = serde_json::json!([{"learning_id": learning_id.parse::<i64>().unwrap(), "verdict": "helped"}]);
    let rated = instance.call("apply_learning_verdicts", &["7", &verdicts.to_string()]);
    assert!(rated.status.success(), "{}", describe(&rated));

    let id: i64 = column(&instance, "SELECT id FROM learning_verdicts")
        .parse()
        .unwrap();
    assert!(id > 5, "verdict id {id} is not past the burned ceiling 5");
}

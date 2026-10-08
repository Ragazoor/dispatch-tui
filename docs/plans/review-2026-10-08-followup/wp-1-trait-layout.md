# WP1 trait layout: one store, no ports

Companion to `wp-1-collapse-store-stack.md`. Structural refactor; no behaviour change.

## Today

```
caller ──► domain trait (TaskRead, TaskCrud, …, 14 traits)
            └─ impl for Store (src/store/queries/*.rs) — forwards
                 └─ Option<Arc<dyn port>> (SharedReader, SharedWriter,
                    SharedLearningReader, SharedUsageReader,
                    SharedRetiredFeedItemReader)
                      └─ sync adapters: SubscriptionBoardReads,
                         SubscriptionLearningReads, SubscriptionUsageReads,
                         SubscriptionRetiredFeedItemReads (one-liners over
                         SharedRows), ReducerWriter (encode + ReducerCaller)
```

`BoardReads` (TUI cards) is a second trait on `SubscriptionBoardReads` with
`get_task`, `list_epics`, … duplicated from `SharedReader`.

## After

```
caller ──► domain trait (unchanged set and signatures)
            └─ impl for Store — the real bodies:
                 reads  = self.rows.<query>()          (SharedRows)
                 writes = encode + self.caller.<reducer>() (ReducerCaller)
                 host   = host-file calls on self.host_file_dir
```

- `Store` fields, all non-optional: `rows: Arc<SharedRows>`,
  `caller: Arc<dyn ReducerCaller>`, `identity: Arc<dyn WriterIdentity>`,
  `clock: Arc<dyn Clock>`, `host: String`, `host_file_dir: PathBuf`.
  Built by `Store::new(...)`; there is no unattached handle and no
  "no shared store attached" error.
- Deleted: the five port traits, `SharedStorePorts`, `Store::unattached`,
  `with_shared_*`, `ReducerWriter` (its helpers become `impl Store` methods),
  `SubscriptionBoardReads`, `SubscriptionLearningReads`,
  `SubscriptionUsageReads`, `SubscriptionRetiredFeedItemReads`,
  `runtime::placeholder_database`.
- The trait impls stay in `src/store/queries/<domain>.rs`, now holding the
  bodies that were in `src/sync/writes.rs` and the readers. `src/sync/` keeps
  the primitives `Store` sits on: `SharedRows`, `ReducerCaller` (+ outcome
  types), `encode`/`decode`, identity, connection, session.
- `BoardReads: TaskRead + EpicRead + RepoConfigRead` and adds only
  `poll_owner` and `revision`. `Store` implements it. The TUI's
  `board_reads` handle is the same `Store` typed as `dyn BoardReads`; the
  duplicate card methods (`list_tasks`, `get_task`, …) are gone — card reads
  are `board_reads.list_all()` etc. through the supertraits.
- `StoreParts::build(data_dir, host_id)` builds the `Store` directly; the
  bootstrap test seam becomes `fn(&Path, &str) -> StoreParts`.
- In-memory test double: `Store::open_in_memory()` (and
  `Store::in_memory_with_host_file(dir)` for identity tests) builds the same
  `Store` over `MemoryReducerCaller`, so tests go through the same traits.

## Tests

- Delete `src/store/tests/shared_writer.rs`, `shared_*_reader.rs`: they assert
  `Store` routes to a port, and the port is gone. The guards they touch
  (identity keys refused, empty patch skipped) are covered in
  `store/tests/settings.rs` / `tasks_patch.rs`.
- `sync/tests/writes.rs` drives `Store` (built over a `RecordingCaller`)
  through the domain traits instead of `SharedWriter`.
- The two tests that injected a read error via an unattached handle
  (`update_task_propagates_db_error_on_prior_task_read`,
  `a_cycle_whose_epic_read_errors_fails_without_syncing_anything`) go: a read
  over in-memory rows cannot fail, so the arm is unreachable.
- New: a `compile_fail` doctest that the port traits are gone, and a test that
  the TUI's card reads and the store answer from the same rows.

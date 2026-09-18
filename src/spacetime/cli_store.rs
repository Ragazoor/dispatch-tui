//! A [`SharedStore`] that talks to SpacetimeDB by running the `spacetime` CLI.
//!
//! Spec: `docs/specs/spacetime-seed.allium`.
//!
//! # Why shell out rather than link a client
//!
//! Dump and restore are operator commands run at seed and recovery time, not a
//! hot path, and this repo already reaches `git`, `gh`, `tmux` and `claude` the
//! same way — through [`ProcessRunner`], which is what makes the argv shape
//! assertable in tests without a server.
//!
//! The alternative was the Rust SDK plus a `spacetime generate` codegen step
//! whose output has to be kept in step with the module by hand. That buys
//! nothing here: this code sends whole rows and reads whole tables, and needs
//! neither subscriptions nor live updates. Phase 4 links the SDK, because
//! subscriptions are the thing it is actually for.
//!
//! The cost is stated plainly: `spacetime` becomes a runtime dependency of
//! these two subcommands, in the same way `gh` already is for PR polling. It is
//! not needed to run the board.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::process::{stderr_str, stdout_str, ProcessRunner, SUBPROCESS_TIMEOUT};

use super::snapshot::{Row, SharedTable};
use super::store::SharedStore;

/// How many bytes of encoded rows travel in one `spacetime call`.
///
/// **Measured in bytes rather than rows, because the limit is in bytes.** A
/// reducer argument is a single `argv` entry, and Linux caps one entry at
/// `MAX_ARG_STRLEN` — 128 KiB — regardless of the two-megabyte `ARG_MAX` that
/// covers the whole vector. Rows are nowhere near uniform: on a real board the
/// median task serialises to about 1.2 KB, the 95th percentile to 5 KB, and the
/// largest to 49 KB. A fixed count of 200 rows looked generous against the
/// median and blew the limit on the first batch.
///
/// 96 KiB leaves room under the cap for the rest of the command line while
/// still sending a typical board's two thousand tasks in a few dozen round
/// trips rather than two thousand.
const BYTES_PER_CALL: usize = 96 * 1024;

/// Talks to one SpacetimeDB database through the `spacetime` binary.
pub struct SpacetimeCliStore {
    runner: Arc<dyn ProcessRunner>,
    /// The database name or identity, e.g. `dispatch`.
    database: String,
    /// The server hosting it. `None` uses whatever the CLI is configured for,
    /// which is what an operator who has already run `spacetime login` expects.
    server: Option<String>,
    /// Which columns of each table are optional, learned from the server and
    /// kept for the life of this store.
    ///
    /// Needed because reading and writing use DIFFERENT encodings for the same
    /// value, and only the server knows which columns need it — see
    /// [`Self::encode_row_for_reducer`].
    shapes: Mutex<HashMap<SharedTable, Arc<Vec<ColumnShape>>>>,
}

impl SpacetimeCliStore {
    pub fn new(
        runner: Arc<dyn ProcessRunner>,
        database: impl Into<String>,
        server: Option<String>,
    ) -> Self {
        Self {
            runner,
            database: database.into(),
            server,
            shapes: Mutex::new(HashMap::new()),
        }
    }

    /// The column shapes of one table, asked of the server once.
    ///
    /// `LIMIT 0` because the schema comes back whether or not any rows do, so
    /// this costs a round trip rather than a table scan — and works on the empty
    /// table a restore is usually aimed at.
    fn column_shapes(&self, table: SharedTable) -> Result<Arc<Vec<ColumnShape>>> {
        if let Some(shapes) = self
            .shapes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&table)
        {
            return Ok(Arc::clone(shapes));
        }
        let stdout = self.spacetime(&[
            "sql",
            "--format",
            "json",
            &self.database,
            &format!("SELECT * FROM {} LIMIT 0", table.name()),
        ])?;
        let shapes = Arc::new(
            decode_schema(&stdout)
                .with_context(|| format!("failed to read {}'s schema", table.name()))?,
        );
        self.shapes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(table, Arc::clone(&shapes));
        Ok(shapes)
    }

    /// Re-encode a snapshot row into what a reducer argument must look like.
    ///
    /// **Reading and writing do not use the same encoding, and nothing warns
    /// you.** A query returns an optional column as the pair `[0, value]`, which
    /// this module decodes to a plain value; a reducer argument wants
    /// `{"some": value}` and rejects the plain value outright. Absent is `null`
    /// on the way in and `[1, []]` on the way out.
    ///
    /// The asymmetry is why the shapes are fetched from the server rather than
    /// hard-coded: wrapping a column that is not optional fails just as loudly
    /// as not wrapping one that is, and the only authority on which is which is
    /// the schema the module actually published.
    fn encode_row_for_reducer(
        &self,
        table: SharedTable,
        shapes: &[ColumnShape],
        row: &Row,
    ) -> Result<serde_json::Value> {
        // CHECKED IN BOTH DIRECTIONS, and the second is the dangerous one. A
        // column the store expects and the snapshot lacks fails loudly below
        // whatever we do. A column the SNAPSHOT carries and the store does not
        // would simply never be copied into `out` — the restore would succeed,
        // report success, and have dropped a column. That is the same data-loss
        // event the format-version refusal exists to prevent, so it is refused
        // on the same terms.
        let unknown: Vec<&str> = row
            .keys()
            .map(String::as_str)
            .filter(|name| !shapes.iter().any(|shape| shape.name == *name))
            .collect();
        if !unknown.is_empty() {
            bail!(
                "the snapshot carries column(s) {} that this store does not have — \
                 restoring would silently drop them",
                unknown.join(", ")
            );
        }

        let mut out = serde_json::Map::with_capacity(shapes.len());
        for shape in shapes {
            let value = row.get(&shape.name).ok_or_else(|| {
                anyhow!(
                    "the snapshot has no column {} — it was taken from a different schema",
                    shape.name
                )
            })?;
            // A null the store cannot hold becomes the sentinel that means the
            // same thing. The snapshot keeps nulls — it is store-neutral, and a
            // dump of the same board from SQLite and from here has to compare
            // equal — so the translation belongs exactly here, at the boundary
            // with the one store that has sentinels. See
            // `SharedTable::sentinel_columns` for why they exist at all.
            let value = match table.sentinel_for(&shape.name) {
                Some(sentinel) if value.is_null() => sentinel.as_json(),
                _ => value.clone(),
            };
            let encoded = if shape.optional && !value.is_null() {
                serde_json::json!({ "some": value })
            } else {
                value
            };
            out.insert(shape.name.clone(), encoded);
        }
        Ok(serde_json::Value::Object(out))
    }

    /// Run `spacetime <subcommand> [flags] <args…>` against this database's
    /// server.
    ///
    /// **The flags go AFTER the subcommand**, because `-s` and `-y` belong to
    /// the subcommand rather than to `spacetime` itself — put them first and
    /// the CLI rejects the whole invocation with "unexpected argument '-s'".
    ///
    /// `-y` on every invocation: these commands run from a script, and a CLI
    /// that stops to ask a question would hang rather than fail.
    fn spacetime(&self, args: &[&str]) -> Result<String> {
        let (subcommand, rest) = args
            .split_first()
            .ok_or_else(|| anyhow!("a spacetime invocation needs a subcommand"))?;
        let mut argv: Vec<&str> = Vec::with_capacity(args.len() + 3);
        argv.push(subcommand);
        if let Some(server) = &self.server {
            argv.push("-s");
            argv.push(server);
        }
        argv.push("-y");
        argv.extend_from_slice(rest);

        // Bounded, like every other subprocess in this repo that talks to
        // something remote (`src/git.rs`, `src/repo_sync.rs`): the server may
        // be unreachable, and an unbounded `run` would park the caller with no
        // diagnostic at all.
        let output = self
            .runner
            .run_with_timeout("spacetime", &argv, SUBPROCESS_TIMEOUT)
            .context("failed to run the `spacetime` CLI — is it on PATH?")?;

        if !output.status.success() {
            // stderr carries the reducer's own error text, which is the only
            // thing that tells an operator which row or which table failed.
            bail!("spacetime {subcommand} failed: {}", stderr_str(&output));
        }
        Ok(stdout_str(&output))
    }

    fn sql(&self, query: &str) -> Result<Vec<Row>> {
        let stdout = self.spacetime(&["sql", "--format", "json", &self.database, query])?;
        decode_sql_json(&stdout).with_context(|| format!("failed to decode the result of: {query}"))
    }

    /// Send one batch of already-encoded rows as a JSON array.
    ///
    /// Joined by hand rather than re-serialised, so the bytes counted while
    /// building the batch are the bytes that go on the command line. Encoding
    /// twice would let the two measurements drift, which is the sort of gap
    /// that shows up as an intermittent `Argument list too long`.
    fn send_batch(&self, reducer: &str, table: SharedTable, batch: &[String]) -> Result<()> {
        let argument = format!("[{}]", batch.join(","));
        self.call(reducer, &[&argument])
            .with_context(|| format!("failed to seed {}", table.name()))
    }

    /// One place that builds a `spacetime call` argv, so the shape is not
    /// written twice and cannot drift.
    fn call(&self, reducer: &str, arguments: &[&str]) -> Result<()> {
        let mut argv = vec!["call", &self.database, reducer];
        argv.extend_from_slice(arguments);
        self.spacetime(&argv)?;
        Ok(())
    }
}

#[async_trait]
impl SharedStore for SpacetimeCliStore {
    async fn schema_version(&self) -> Result<i64> {
        let rows = self.sql("SELECT version FROM schema_version")?;
        rows.first()
            .and_then(|row| row.get("version"))
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                anyhow!(
                    "the database holds no schema_version row — it was published \
                     from a module older than this one, or is not a dispatch database"
                )
            })
    }

    async fn upsert_rows(&self, table: SharedTable, rows: &[Row]) -> Result<()> {
        // An empty table still has a reducer and still costs nothing; skipped
        // rather than sent so the log of a restore reads as the work it did.
        if rows.is_empty() {
            return Ok(());
        }
        let reducer = seed_reducer(table);
        let shapes = self.column_shapes(table)?;

        let mut batch: Vec<String> = Vec::new();
        let mut batch_bytes = 0usize;
        for row in rows {
            let encoded = serde_json::to_string(&self.encode_row_for_reducer(table, &shapes, row)?)
                .with_context(|| format!("failed to encode a row of {}", table.name()))?;

            if encoded.len() + 2 > BYTES_PER_CALL {
                // One row that cannot fit alone. Refused by name rather than
                // sent and rejected by the kernel with "Argument list too
                // long", which says nothing about which row or which table.
                bail!(
                    "a single row of {} encodes to {} bytes, over the {BYTES_PER_CALL}-byte \
                     limit on one command-line argument. Its id is {:?}. The snapshot is \
                     fine; it is this transport that cannot carry the row.",
                    table.name(),
                    encoded.len(),
                    table.id_column().and_then(|c| row.get(c)),
                );
            }

            // +1 for the comma that will join it to the batch.
            if batch_bytes + encoded.len() + 1 > BYTES_PER_CALL && !batch.is_empty() {
                self.send_batch(reducer, table, &batch)?;
                batch.clear();
                batch_bytes = 0;
            }
            batch_bytes += encoded.len() + 1;
            batch.push(encoded);
        }
        if !batch.is_empty() {
            self.send_batch(reducer, table, &batch)?;
        }
        Ok(())
    }

    async fn advance_id_sequence_past(&self, table: SharedTable, ceiling: i64) -> Result<()> {
        if !table.generates_ids() {
            return Ok(());
        }
        // The loop itself lives server-side, inside one reducer transaction, so
        // a burn is atomic and so a burn of twenty thousand ids is one round
        // trip rather than twenty thousand. See the module's
        // `burn_id_sequence`, and note that it MUST run before the rows are
        // written — `restore` is what guarantees that order.
        self.call(
            "burn_id_sequence",
            &[&format!("\"{}\"", table.name()), &ceiling.to_string()],
        )
        .with_context(|| {
            format!(
                "failed to burn {}'s id sequence past {ceiling} — if this says the \
                 insert violated a unique constraint, the table already held rows \
                 in that range and the burn was run too late",
                table.name()
            )
        })?;
        Ok(())
    }

    async fn rows(&self, table: SharedTable) -> Result<Vec<Row>> {
        let mut rows = self.sql(&format!("SELECT * FROM {}", table.name()))?;
        // SORTED HERE, NOT IN THE QUERY. SpacetimeDB's SQL rejects `ORDER BY`
        // outright ("Unsupported: SELECT * FROM tasks ORDER BY id"), so the
        // ordering that makes two dumps of an unchanged database comparable —
        // and a diff between two backups readable — has to happen client-side.
        if let Some(column) = table.id_column() {
            rows.sort_by_key(|row| row.get(column).and_then(serde_json::Value::as_i64));
        }
        // The inverse of the encode above: a sentinel read back out is the
        // absence it stands for. Without this a dump from the shared store
        // would differ from a dump of the same board from SQLite on every
        // sentinel column — and the whole point of the snapshot format is that
        // the two are the same file.
        for row in &mut rows {
            for (column, sentinel) in table.sentinel_columns() {
                if row.get(*column).is_some_and(|v| sentinel.matches(v)) {
                    row.insert((*column).to_string(), serde_json::Value::Null);
                }
            }
        }
        Ok(rows)
    }
}

fn seed_reducer(table: SharedTable) -> &'static str {
    match table {
        SharedTable::Tasks => "seed_tasks",
        SharedTable::Epics => "seed_epics",
        SharedTable::Todos => "seed_todos",
        SharedTable::TaskWatchers => "seed_task_watchers",
        SharedTable::TaskShells => "seed_task_shells",
        SharedTable::TaskSubagents => "seed_task_subagents",
        SharedTable::RepoPaths => "seed_repo_paths",
        SharedTable::RepoBaseBranches => "seed_repo_base_branches",
        SharedTable::Hosts => "seed_hosts",
        SharedTable::Subscriptions => "seed_subscriptions",
    }
}

/// Turn `spacetime sql --format json` output into plain rows.
///
/// The CLI does not emit objects. It emits a schema and then each row as a
/// POSITIONAL array, so a field is only identifiable by its index into
/// `schema.elements`. Optional columns are a further step removed: their type is
/// a two-variant sum, and their value arrives as `[0, value]` for present and
/// `[1, []]` for absent.
///
/// Decoded here rather than passed through, because everything downstream —
/// the snapshot format, the comparison between a dump and a re-dump, a human
/// reading the file during an incident — wants a column name and a value.
///
/// **This is a wire format that belongs to a tool marked unstable.** If a
/// future CLI changes it, this function is where it breaks, and it breaks
/// loudly: an unrecognised shape is an error, never a defaulted value, because
/// a backup that silently decodes a column as null is worse than one that will
/// not be taken.
fn decode_sql_json(stdout: &str) -> Result<Vec<Row>> {
    let result = first_statement_result(stdout)?;
    let columns = parse_schema(&result)?;

    let raw_rows = result
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("result carries no rows"))?;

    raw_rows
        .iter()
        .map(|raw| decode_row(&columns, raw))
        .collect()
}

/// Read just the schema out of a `--format json` result, ignoring any rows.
fn decode_schema(stdout: &str) -> Result<Vec<ColumnShape>> {
    parse_schema(&first_statement_result(stdout)?)
}

/// The CLI answers a batch of statements, so the top level is an array with one
/// entry per statement. One query in, one result out.
fn first_statement_result(stdout: &str) -> Result<serde_json::Value> {
    let payload: serde_json::Value = serde_json::from_str(stdout.trim())
        .context("the CLI did not return JSON — it may have printed a warning instead")?;
    payload
        .as_array()
        .and_then(|results| results.first())
        .cloned()
        .ok_or_else(|| anyhow!("expected an array of statement results"))
}

fn parse_schema(result: &serde_json::Value) -> Result<Vec<ColumnShape>> {
    result
        .pointer("/schema/elements")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("result carries no schema"))?
        .iter()
        .map(ColumnShape::parse)
        .collect()
}

struct ColumnShape {
    name: String,
    optional: bool,
}

impl ColumnShape {
    fn parse(element: &serde_json::Value) -> Result<Self> {
        let name = element
            .pointer("/name/some")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("a column in the schema has no name"))?
            .to_owned();
        // An optional column is a sum of `some` and `none`. Any other sum is a
        // shape this decoder does not know, and guessing at it is how a column
        // ends up silently wrong.
        let optional = element.pointer("/algebraic_type/Sum").is_some();
        Ok(Self { name, optional })
    }
}

fn decode_row(columns: &[ColumnShape], raw: &serde_json::Value) -> Result<Row> {
    let values = raw
        .as_array()
        .ok_or_else(|| anyhow!("a row is not an array — the wire format changed"))?;
    if values.len() != columns.len() {
        bail!(
            "a row has {} values for {} columns — the wire format changed",
            values.len(),
            columns.len()
        );
    }

    let mut row = Row::new();
    for (column, value) in columns.iter().zip(values) {
        let decoded = if column.optional {
            decode_optional(value).with_context(|| format!("column {}", column.name))?
        } else {
            value.clone()
        };
        row.insert(column.name.clone(), decoded);
    }
    Ok(row)
}

/// `[0, value]` is present, `[1, []]` is absent.
fn decode_optional(value: &serde_json::Value) -> Result<serde_json::Value> {
    let pair = value
        .as_array()
        .ok_or_else(|| anyhow!("an optional value is not a [tag, payload] pair"))?;
    match pair.first().and_then(serde_json::Value::as_u64) {
        Some(0) => Ok(pair
            .get(1)
            .cloned()
            .ok_or_else(|| anyhow!("a present optional carries no payload"))?),
        Some(1) => Ok(serde_json::Value::Null),
        other => bail!("unrecognised optional tag {other:?}"),
    }
}

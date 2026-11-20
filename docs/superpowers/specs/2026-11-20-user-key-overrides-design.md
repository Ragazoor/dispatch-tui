# User key overrides — design

Task #99351, epic #69706 (Remap keyboard). Status: approved by the operator on 2026-11-20. The spec change is in
`docs/specs/` (`keybindings.allium` and the specs listed below).
No code in this task. The spec change follows this document, and tasks 2 to 8
of the epic build on both.

## Why

Every key the product answers is a row of one table (`docs/specs/keybindings.allium`).
The table is compiled in, so a user who wants `E` for "last row", or who has a
keyboard where `[` is hard to reach, cannot change anything. This design lets
the user replace the keys of a row, and keeps every promise the table makes
today: one table, read by the handlers, the `?` overlay and `keybinding_search`.

## Already decided (operator, 2026-11-20)

- The overrides are a file in the data directory, not a store setting. The
  board and the two pane processes read it.
- An override names a row by namespace and action, plus context where one
  action has several rows.
- A bad file never locks the user out. Bad rows keep their defaults and the
  board shows the error.
- Changes apply at restart. No live reload.
- Agents get an MCP tool to read and set overrides.
- The tmux prefix keys are in scope.

## Terms

- **Default table**: the compiled-in rows.
- **Override**: one entry of the file. It replaces the whole key list of one
  row. It never adds a row, removes a row, or changes an action, context,
  description or note.
- **Merged table**: the default table with the accepted overrides applied.
  It is what every reader uses. With no file it equals the default table.

## 1. The file

Path: `<data directory>/keybindings.json`. A missing file means defaults and
is not an error. Dispatch never creates the file on its own. The MCP tool
writes it, and later the board's key editor does (see "Later task"). Nobody
is expected to edit it by hand, but it stays readable so a user can fix or
delete it in an emergency.

The file is a JSON list. Each entry is one override:

```json
[
  {"namespace": "board.normal", "action": "navigate_row_last", "keys": ["E"]},
  {"namespace": "board.normal", "action": "select_all",
   "context": "on_column_select_all", "keys": ["Enter"]},
  {"namespace": "tmux.global", "action": "toggle_agent_tree", "keys": ["Prefix+t"]}
]
```

- `namespace` is a published namespace name (`board.normal`,
  `board.picker.tag`). Family names (`board.confirm`, `board.picker`) are not
  namespaces.
- `action` is the row's action id.
- `context` names the row where an action has more than one row in the
  namespace. Leave it out for the row with no context. `select_all` has both:
  `a` anywhere and Enter on a select-all row. An entry with no `context` that
  matches several rows and no context-free row is refused, and the message
  lists the contexts.
- The context is the literal (`on_epic_card`), not the display words. So
  `keybinding_search` must return the literal beside the words. The `?`
  overlay keeps the words only.
- `keys` replaces the row's whole key list.

Why this shape:

- An entry has the same fields as one item of the MCP tool's argument, so the
  tool and the file cannot differ.
- Each entry stands alone. A repeated or broken entry is an error for that row
  and the rest of the file applies.
- The repo already reads and writes JSON. TOML or YAML needs a new dependency,
  and their gain is comments, which a file written by programs does not have.
  YAML also reads some bare values as other types (`y`, `n`, `no`, numbers),
  and single letters and digits are exactly what this file holds.

## 2. Key names

A closed list. The table, the file, the overlay and `keybinding_search` all use
it.

- **A character**: one printable character other than space, as typed. Case
  matters: `g` and `G` are two keys. `Shift+` is never written.
- **A named key**: `Space`, `Enter`, `Esc`, `Tab`, `BackTab`, `Backspace`,
  `Delete`, `Insert`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`,
  `PageDown`, `F1` to `F12`.
- **A modified key**: `Ctrl+`, `Alt+` or `Ctrl+Alt+` before a character or a
  named key. A letter after a modifier is uppercase (`Ctrl+D`).
- **A chord**: two characters, pressed one after the other (`gg`). See 5.
- **A tmux key**: `Prefix+` and one of the first three, in `tmux.global` only.

The loader is strict. `ctrl+d`, `enter`, `Shift+G` and `Escape` are refused,
and the message gives the canonical spelling (`"ctrl+d" is not a key name;
write "Ctrl+D"`). Lookup is an exact string match, so a silently accepted
near-miss would be a dead key.

Three names are refused because a terminal sends another key for them:
`Ctrl+I` (Tab), `Ctrl+M` (Enter), `Ctrl+[` (Esc). Task 3 checks the terminal
library for others, and the spec list grows if it finds any.

Today a few names come from a debug format (`PageUp`, the F keys). No row uses
them, so giving them fixed names changes no recorded usage.

## 3. What a bad entry does

Checks run per entry. A refused entry leaves its row on the default keys and
adds one error. The rest of the file applies.

| Problem | Example | Result |
|---|---|---|
| File is not valid JSON, or not a list | missing bracket | no override applies; one error with the position |
| Entry is not an object, lacks a field or has an unknown field | `{"action": "quit"}` | refused |
| Two entries name the same row | `quit` twice | both refused |
| Unknown namespace | `board.nomral` | refused; message lists the namespaces |
| Unknown action | `navigate_rwo_last` | refused; message lists the namespace's actions |
| Unknown or missing context | `"context": "nowhere"` | refused; message lists the action's contexts |
| `keys` is not a list of strings | `"keys": "x"` | refused |
| Empty list | `"keys": []` | refused |
| Unknown key name | `["ctrl+d"]` | refused, with the canonical spelling |
| Key not allowed in the namespace | a plain character in `board.text` | refused (section 4) |
| The row is a catch-all | `confirm_quit_no` | refused (section 5) |
| Clash | two rows answer one key at once | refused (below) |

A key listed twice in one entry is read once, with no error.

### Clashes

The two table rules must hold for the merged table, checked as a whole:

- `KeyUniquePerContext`: no two rows of a namespace share a key and a context.
- `ContextsDoNotOverlapForAKey`: two rows of a namespace that share a key
  have contexts that can never hold together.

Checking the whole table matters. Swapping `j` and `k` is two entries, and each
alone would clash with the other row's default.

When the merged table breaks a rule, the loader drops every override that
takes part in a clash, and checks again. A dropped override restores a default
key, which can clash with another override, so the loader repeats until no
clash is left. The default table has no clash, so this always ends, and the
result does not depend on the order of the file. Each dropped override is one
error that names both rows and the key.

Consequence: a later release that adds a default row on a key the user took
makes that override drop, with an error at startup. The user picks another
key. The alternative, the user's row winning, could leave the new row with no
key at all.

The overlap relation over contexts (`contexts_overlap`) is test-only code today.
The loader needs it, so task 3 makes it part of the product.

### Showing the errors

- The board opens its error popup once it has drawn. The popup names the file,
  says the listed rows keep their default keys, and lists each error. The same
  lines go to `app.log`.
- The popup's row is a catch-all and cannot be rebound, so any key dismisses
  it whatever the file says.
- `keybinding_search` returns the load errors too, so an agent sees them.
- A pane applies the same rules and logs its errors. It shows nothing: the
  board is the place that reports.

## 4. Text namespaces

Six namespaces take typed text: `board.text`, `board.search`,
`board.filter_search`, `board.picker.repo_path`, `board.picker.base_branch`,
`board.picker.quick_dispatch`. A plain character there, and Space, is data.

An override in these namespaces may use only a named key other than Space, or
a modified key. A character, Space or a chord is refused. So `Esc`, `F2`,
`Ctrl+G` and `Alt+Backspace` are fine and `q` is not. This holds for all
rows of the namespace, the editing rows (Backspace, caret motion) included.

## 5. The catch-all and the chord

**Catch-all rows are fixed.** "any other key" means "whatever no other row
lists", so it has no key to replace. An override that targets a catch-all row
is refused, and "any other key" is not a key name an override may use. The
rows beside it can be rebound: when `confirm_quit_yes` is given `o`, `y` becomes
one of the other keys and answers no.

**Chords.** A chord is a key name like any other.

- It is exactly two characters. No named or modified keys, no three-key forms.
  A two-letter name that is a named key (`Up`, `F1` to `F9`) is that named
  key, so those chords cannot be written.
- Any row of a namespace that is not a text namespace and not `tmux.global` may
  list one. `navigate_row_first` may drop `gg` for `Home`, and another row may
  take `dd`.
- Prefix rule, checked like a clash: the first character of a chord may not be
  a key of any row in the same namespace, whatever its context. Two chords
  may share a first character.
- How long the first half waits stays a property of the handler: the board
  times out, the panes do not.

The handlers already look the chord up by name, so the general form costs
little in task 2.

## 6. One row per direction or value

Today some rows list keys for two directions or several values, and the
handler reads the key to pick one. A remapped key would run the row and go the
wrong way. Each direction or value becomes its own row with its own action id,
and no handler reads the key. An action has one row per context, so a second
row needs a second id.

| Namespace | Today | Becomes |
|---|---|---|
| `board.normal` | `navigate_column` | `navigate_column_left` (h, Left), `navigate_column_right` (l, Right) |
| `board.normal`, `agent_tree.*`, `agent_diff` | `navigate_row` | `navigate_row_down`, `navigate_row_up` |
| `agent_tree.*`, `agent_diff` | `navigate_half_page` | `navigate_half_page_down` (Ctrl+D), `navigate_half_page_up` (Ctrl+U) |
| `board.detail` | `scroll_detail` | `scroll_detail_down`, `scroll_detail_up` |
| `board.help` | `scroll_help` | `scroll_help_down`, `scroll_help_up` |
| `board.filter` | `filter_move_cursor` | `filter_cursor_down`, `filter_cursor_up` |
| `board.filter` | `repo_filter_toggle_repo` | `repo_filter_toggle_repo_1` to `repo_filter_toggle_repo_9` (1 to 9) |
| `board.picker.repo_path`, `board.picker.base_branch` | `picker_move_cursor` | `picker_cursor_down` (Down), `picker_cursor_up` (Up) |
| `board.picker.quick_dispatch` | `quick_dispatch_move_cursor` | `quick_dispatch_cursor_down` (Down), `quick_dispatch_cursor_up` (Up) |
| `board.picker.tag` | `tag_picker_select` | `tag_picker_select_bug` (b), `tag_picker_select_feature` (f), `tag_picker_select_chore` (c), `tag_picker_select_pr_review` (v), `tag_picker_select_research` (r), `tag_picker_select_fix` (x) |
| `board.picker.wrap_up_mode` | `wrap_up_mode_picker_select` | `wrap_up_mode_picker_select_rebase` (r), `wrap_up_mode_picker_select_pr` (p), `wrap_up_mode_picker_select_done` (d) |
| `board.picker.model` | `model_picker_select` | `model_picker_select_haiku` (h), `model_picker_select_sonnet` (s), `model_picker_select_opus` (o), `model_picker_select_fable` (f) |
| `board.picker.move_to_epic` | `move_to_epic_picker_navigate` | `move_to_epic_picker_down`, `move_to_epic_picker_up`, `move_to_epic_picker_expand` (l, Right), `move_to_epic_picker_collapse` (h, Left) |
| `board.picker.reparent_epic` | `reparent_picker_navigate` | `reparent_picker_down`, `reparent_picker_up`, `reparent_picker_expand` (l, Right), `reparent_picker_collapse` (h, Left) |
| `board.picker.work_package` | `work_package_picker_navigate` | `work_package_picker_down`, `work_package_picker_up` |
| `board.picker.placement` | `placement_picker_workspace` | `placement_picker_workspace_down`, `placement_picker_workspace_up` |
| `board.admin` | `admin_navigate` | `admin_navigate_down`, `admin_navigate_up` |
| `board.admin` | `admin_pane` | `admin_next_pane` (Tab), `admin_previous_pane` (BackTab) |

Where no key is shown, down takes j and Down, and up takes k and Up.

Not split, because every key of the row does the same thing:
`placement_picker_visibility`, `switch_section`, `cycle_agent_filter`,
`collapse_directory`, and the y / Y rows.

### Usage history

Stored usage events are not rewritten.

- The retired ids stop being recorded on the day task 2 lands. Their counts
  stay in the history under the old ids.
- The new ids start at zero. To read across the change, add the old id's
  counts by key: `navigate_row` with detail `j` or `Down` is
  `navigate_row_down`.
- From then on the action alone says what happened, whatever key is bound.
  The detail still holds the key pressed, so two keys for one action stay
  separable.

### Descriptions

A row's description and note are shown beside its keys, and some name keys:
"(j, Down)", "[b]ug", "use Space", "then Space". After a remap they would be
wrong.

- A description says what the row does and names none of the row's own keys.
  The split removes most cases: "Move to the next card in the column".
- A description or note that must name **another** row's key refers to the
  row, and the reader sees that row's first current key. The `G` note becomes
  "does not enter an epic, use (the key of enter_epic)".
- The tag, wrap-up and model steps draw their letters from the rows (task 5).

## 7. When the file is read

Each process reads the file once, when it starts: the board at board start, a
pane when the pane opens. Nothing watches the file.

So a pane opened after an edit uses the new file while the running board still
uses the old one. This is intended. The other choice, the board handing its
table to each pane, adds a protocol for a case that ends at the next restart.
Usage recording is not affected: a pane sends the action id and the key, and
the board records what it is sent.

## 8. tmux.global

The two rows (`jump_to_dispatch`, default `Prefix+Space`; `toggle_agent_tree`,
default `Prefix+e`) can be overridden like any row, with these rules:

- The key is `Prefix+` and exactly one key: a character, a named key or a
  modified key. No chord. One key per row.
- The two rows may not share a key.
- The board binds the keys of the merged rows when it starts. The table form
  maps to the tmux form in one place: `Ctrl+X` to `C-x`, `Alt+X` to `M-x`, a
  character as itself, a named key by its tmux name.
- At quit the board unbinds the keys **it bound**, not the keys in the file,
  which may have changed since.
- A board that was killed leaves its keys bound. If the file changes before
  the next start, the old key would stay bound to the board's command. So at
  start the board first removes every prefix binding whose command is one of
  its own two commands, then binds the merged keys.
- As today, binding a key replaces what the user had on it, and unbinding
  does not bring that back. A user who remaps picks a free key.

## 9. MCP tools

Each tool has one access class (ADR 0002), so reading and writing are two
tools.

**`keybinding_search` (read, extended).**

- It returns the merged table the running board answers to, as today.
- Each row also carries its context literal, and for an overridden row its
  default keys.
- A row whose entry in the file differs from what the board loaded carries
  its pending keys, and the reply says a restart applies them.
- The index (the call with no arguments) lists the load errors.

**`keybinding_set` (new, idempotent).**

- Argument: `overrides`, a list of one or more entries, each with
  `namespace`, `action`, optional `context`, and `keys`. The list is applied
  together, so a swap is one call.
- An empty `keys` list removes the entry and restores the default. (In the
  file itself an empty list is an error.)
- It runs the loader's checks on the default table plus the file plus the
  change. If the change adds any error, nothing is written, and the message
  says what to fix.
- Bad entries already in the file are left alone and reported in the answer.
- A file that is not valid JSON is not touched: the tool says to fix or
  delete it.
- The write keeps every other entry and is atomic.
- The answer shows each row's new keys and says they apply at the next board
  restart (a pane opened sooner already uses them).

## 10. Drift gate

What the test suite must hold, beyond today's checks on the default table:

1. **No file, no change.** With no file the merged table equals the default
   table.
2. **Any file gives a well-formed table.** For generated override files, good
   and bad, the merged table satisfies `KeyUniquePerContext`,
   `ContextsDoNotOverlapForAKey`, the prefix rule and "every row has a key".
3. **Every refusal is tested.** Each row of the table in section 3 has a test:
   the entry is refused with its message, its row keeps the default, and a
   good entry in the same file still applies.
4. **The table drives dispatch when remapped.** The per-row press check
   (press every key of every row, see the row's action recorded) also runs
   over a remapped table. That table swaps the keys of every split pair and
   rebinds at least one row in every namespace that takes overrides. The old
   key no longer runs the row.
5. **No handler reads the key.** Follows from 4: with directions swapped, each
   key goes the way its row says, on the board and in both panes.
6. **Readers follow the remap.** Under the remapped table the `?` overlay,
   `keybinding_search`, the pane titles, the pane startup hint and every
   key hint name the user's keys, and no description names a key that was
   rebound. The hint check reads every board namespace except `board.help`
   and `board.error`. That now includes the status bar and footer of
   `board.normal` and `board.detail`, and the three modes that write their
   keys without brackets.
7. **tmux rows.** The key the board binds is the merged row's key, and the
   board unbinds what it bound.
8. **The guide.** `docs/guide/index.html` is a static page, so it is checked
   against the default table and says its keys are defaults.

## Specs this changes

- `keybindings.allium`: the override entity and load rule, key names, the
  clash and prefix rules over the merged table, text namespaces, fixed
  catch-all, the split ids, the two MCP surfaces, the drift gate.
- `storage.allium`: the file joins the list of what lives in the data
  directory.
- `observability.allium`: an action id names one direction or value.
- `mcp-task-tools.allium`: the new tool in the tool lists.
- `agent-tree.allium`, `split-pane.allium`, `startup.allium`: the bound tmux keys
  come from the merged rows; the sweep of stale bindings at start.
- `board-layout.allium`, `agent-tree.allium`: the chord rules hold for any
  chord, not only `gg`.
- `onboarding-guide.allium`: parity is against default keys.
- Any spec that cites a retired action id.
- `settings.allium`: no change. Key overrides are not a store setting.

## Decided with the operator (2026-11-20, this task)

| Question | Decision |
|---|---|
| File shape | JSON, one entry per row. Programs write the file, not people |
| Chords | Any two-character chord, with the prefix rule |
| Empty key list | Refused: every row keeps at least one key |
| Usage history | No rewrite; old ids keep their counts |
| A key editor in the board | Yes, as a later task |

## Later task: a key editor in the board

Agents change keys through `keybinding_set`. The user needs the same from the
board, so a later task adds a key editor there. It is not specified here. What
this design fixes for it:

- It writes the same file, through the same checks as the loader and the MCP
  tool. A change that would add an error is refused with the same message.
- It shows each row's current keys, its default keys, and whether a change is
  waiting for a restart.
- It can restore a row's default.
- Its own keys are rows of the table, in a namespace of their own, so they can
  be rebound and they appear in the `?` overlay.
- Capturing a key press as a new binding is a text-like mode: the pressed key
  is data there, not a binding. The task must say which key leaves that mode.

## Effect on the other tasks

- Task 2 uses the id table in section 6 and the chord decision. It removes the
  three `allow-phantom-symbol` lines in `keybindings.allium` once the ids
  exist in the code.
- Task 3 implements sections 1 to 5 and 7 and makes the context overlap
  relation product code. The file is `keybindings.json`, not the working name
  `keybindings.toml`. No new dependency.
- Task 4 adds: `keybinding_search` returns context literals, default keys and
  load errors.
- Task 5 adds: descriptions and notes that refer to another row's key, and
  the hint check over `board.normal` and `board.detail`.
- Task 6 builds `keybinding_set` as in section 9, with pending keys in
  `keybinding_search`.
- Task 7 adds the sweep of stale bindings at start and the key name mapping.
- Task 8 documents the file and marks the guide's keys as defaults.
- A new task, after task 6: the key editor in the board.

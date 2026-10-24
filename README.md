# Siddran-CLI

Siddran — notes, tasks and calendar — in the terminal. One binary: bare `siddran` launches the
TUI, a subcommand runs and exits.

It is a **peer of the desktop app, not a client of it.** There is no server and no API: it reads
and writes the same **Bag** folder that `Siddran-Desktop` uses, in the same format, byte for
byte. See [`docs/parity.md`](docs/parity.md) — that contract is the foundation everything else
rests on.

## Status

| Phase | | |
|---|---|---|
| 0 | Format fidelity harness | ✅ |
| 1 | Read-only CLI | ✅ |
| 2 | Writes, lock, safe deletes | ✅ |
| 3 | TUI shell + notes | ✅ |
| 4 | TUI tasks | ✅ |
| 5 | Calendar | ✅ |
| 6 | Settings | ✅ |
| 7 | AI access (`siddran mcp`), sandboxes, per-write locking | ✅ |

Writes are atomic (temp + rename, so a crash cannot truncate a note), guarded by an advisory
lock, and deletions go to `.siddran-trash/` inside the Bag rather than to `unlink`.

## The TUI

```bash
siddran                    # miller columns: sources │ notes │ live preview
```

| | |
|---|---|
| `j k` `h l` / arrows | move · change column |
| `j k` `d u` in the preview | scroll the note |
| `gg` `G` | first / last |
| `^e` `^y` | scroll the preview without leaving the list |
| `/` | fuzzy filter |
| `1` `2` `3` `4` · `Tab` · `g n` `g t` `g c` `g s` | features |
| `<` `>` | calendar: previous / next month |
| `h j k l` in the grid | calendar: walk by day / week |
| `J` `K` · `.` | calendar: ±1 day · jump to today |
| `Enter` / `h` | calendar: into the agenda / back out |
| `Alt+1..4` · `[` `]` | workspaces — each keeps its own feature and position |
| `Enter` | notes: open in your editor · tasks: toggle complete |
| `a` · `D` | add · delete (asks first) |
| `r` `t` `Space` | rename · tags · favourite (one-line prompts, no modal) |
| `Space` `p` | tasks: complete · cycle priority |
| `R` · `P` `F` | reload · swap palette / frame |
| `?` | help · `q` quit |

**The background is the terminal's.** Nothing paints a fill, so your terminal's own
transparency, blur or image shows through — the way yazi behaves. Only selections and the
status pills carry a colour of their own.

**The cursor is always findable.** Each column has a one-cell gutter that doubles as its
divider: a dim `│` normally, a solid accent `▐` when that column has focus. An empty column
still shows a highlighted placeholder rather than a blank void, so "which pane am I in?" is
never a guess.

**Why Alt and not Ctrl for workspaces.** Most terminals cannot encode Ctrl with a digit —
`Ctrl+1` arrives as a plain `1`, which would silently look like a feature switch. `Alt` sends
`ESC`+digit, which every terminal has supported forever. (Ctrl is also accepted, for terminals
like kitty and foot that speak the kitty keyboard protocol.)

### Tasks

Buckets down the left — Today, Overdue, Upcoming, Undated, Completed, Dailies, Projects —
items in the middle, detail on the right.

- **Today includes overdue work.** A deadline that slipped is still something to do today; a
  "today" list that quietly drops it is how things get missed. `Overdue` exists separately for
  when you want only the slippage.
- **Dailies show their recurrence as lit days** — `MTWTF··` — rather than the
  `{"mask":[…]}` that is actually on disk, plus a **28-day heatmap** that distinguishes three
  states, not two: `■` done, `□` due and not done, `·` not due. A weekday-only habit must not
  look like it was missed every weekend.
- **A project's priority is derived** from its incomplete subtasks, never the stored field,
  which can go stale.
- **A task's `all day` flag is shown even when absent** — `absent — legacy, reads as false` —
  because that absence is what makes a legacy deadline date-only, and hiding it would hide why
  the row reads as it does.

### Calendar

A month grid, the selected day's agenda, and detail — and it is a **view, never a copy**.
Nothing is stored: events, dated tasks, and recurring dailies are projected onto days on the
fly, so ticking a task off in the tasks view changes what the calendar shows.

- A dot under a date means something happens there; today stays highlighted even when the
  selection is elsewhere.
- The agenda puts all-day items first, then chronological, with a left bar coloured by source
  (event / task deadline / daily).
- A multi-day block appears on every day it spans. A legacy task with no `due_all_day` key
  renders as all-day rather than as a midnight appointment.
- Paging from the 31st into a 30-day month lands on the 30th, not on an error.

### Creating and deleting

`a` adds one of whatever the pane holds — a note, a notebook (from the sources column), a task,
a daily, a project, or a subtask (from inside one). Context decides, so there is nothing extra
to remember: a note lands in the notebook you were looking at, a task added from **Today** is
due today, and a daily starts repeating every day rather than expiring tonight.

`D` deletes, and always asks first. It is `D` rather than `d` because `d` pages the preview and
a destructive action earns the deliberate shift; the answer must be an explicit `y`, so a stray
keypress can never delete anything. Notes go to `.siddran-trash/` and are recoverable
byte-for-byte; deleting a daily takes its completion history with it, rather than leaving rows
pointing at nothing.

### Alarms

A deadline rings in the status bar, and any key dismisses it — that key is swallowed, so the
one that silenced the alarm does not also act on something you were not looking at.

- **A date-only deadline rings at `alarm_time`** (09:00 by default), not at the local midnight
  it is stored at. That is a policy about the alarm, not a time written onto the task.
- **Anything older than 12 hours is marked handled silently**, so a two-week-old deadline does
  not scream on launch — or on every launch after.
- **Only the first sweep announces what was already due.** That is the "you were away, here is
  what you missed" case. Later sweeps run because something was written, and a task you just
  created that was *born* overdue did not become due — it should not alarm in your face.
- **Ticking something off disarms it**, checked at sweep time rather than when it was scheduled.
- `notify` is off by default; set it to `notify-send` and alarms also reach your desktop, which
  outlives the terminal being buried behind other windows. Shelling out is never a surprise.

### Settings

`4`, or `g s`. Two kinds of setting sit side by side, and the breadcrumb says which file you
are looking at:

- **Terminal** — `palette`, `frame`, `editor`, `bag`. Yours, stored in
  `~/.config/siddran/config.toml`. `Space` cycles a choice, `Enter` edits text, and either
  saves immediately *and* applies to the running session. `›` marks the rows you can change.
- **Shared** — the desktop settings the terminal actually honours, read from `settings.siddran`
  and shown **read-only** (change them in the desktop; that file is its). Currently one:
  `rememberNoteState`, which the terminal reads as *"put every workspace back where it was on
  the next launch"*.

Desktop-only keys are deliberately **not** listed. Nineteen of the twenty in `settings.siddran`
are visual to the desktop — `starCount`, `bookPageHeight`, `noteEditorWidth` — and showing them
in a terminal app would be showing you knobs that do nothing here. If the terminal starts
honouring another one it joins the Shared list; until then, its absence is the honest answer.

The config writer preserves every line it does not recognise, comments included, so hand-edits
survive. At a prompt, `^u` clears the line and `^w` a word — the value is prefilled, so without
those, typing would only append to it.

### Resource use, measured

On the real Bag (44 notes, 300 events), one thread, idle with no input — in both the notes and
the tasks view:

```
CPU      0.000 s  (0.000% of one core)     RSS   6.5 MB
context switches over 10 s idle:  0        /proc/<pid>/wchan: __x64_sys_epoll_wait
keypress notes 0.00 ms · preview scroll 0.60 ms · tasks 0.00 ms · calendar 1.0 ms
binary   1.7 MB
```

The calendar costs more because a month grid touches every event. It started at **6.8 ms** per
keypress: the grid called the per-day projection 42 times, and each draw re-deserialized all 300
events from raw JSON several times over. Counting the whole grid in one pass took it to 3.0 ms,
and caching the parsed rows took it to 0.83 ms — 8× in total, for two changes that are also
simpler than what they replaced.

That is not incidental. The loop blocks in `epoll_wait` and repaints **only** when a key
actually changed state — no 16 ms poll loop, no animation, no background threads, and markdown
previews cached per note so moving down a list does not re-parse 44 times. The one routine timer
is local midnight, because "today" and "overdue" are date-dependent.

### Editor hand-off

`Enter` suspends the TUI and hands the file to your editor. Once it starts, **it owns the
terminal** — the TUI is not running and cannot draw over it — which is why the key hint lives in
the status bar *before* launch rather than as an overlay during.

Which editor: `editor = "…"` in the config, then `$VISUAL`, then `$EDITOR`, then the first of
micro, nano, hx, nvim, vim, vi that exists. `$EDITOR` is unset on a lot of machines, and landing
a first-time user in vi with no way out is a bad outcome, so the fallback prefers editors that
need nothing learned.

## Build

```bash
cargo build --release      # ~1.1 MB binary, ~8 MB RSS, ~9 ms per invocation
cargo test                 # includes the byte-identity contract
```

## Pointing it at a Bag

In precedence order: `--bag <path>`, then `$SIDDRAN_BAG`, then `bag = "…"` in
`~/.config/siddran/config.toml`, then `~/Documents/Siddran` if it looks like a Bag.

```toml
# ~/.config/siddran/config.toml
bag = "~/Documents/Siddran"
editor = "micro"
palette = "ember"   # or "yazi"
frame = "rounded"   # or "square", "flat"
```

## Commands

```
siddran note ls [--notebook N] [--tag T] [--fav] [--unfiled]
siddran note cat <id|title> [--raw]     # markdown to stdout; --raw keeps frontmatter
siddran note path <id|title>
siddran note new <title> [--notebook N] [--tags T] [--body -|TEXT]
siddran note rename <what> <title>      # renames the file too
siddran note mv <what> [--notebook N]   # omit --notebook for unfiled
siddran note fav <what>
siddran note rm <what> [--yes]          # to .siddran-trash/, recoverable
siddran notebook ls

siddran task ls [--today|--overdue|--undated|--all]
siddran task show <id|title>
siddran task add <title> [--priority low|normal|high] [--due today|tomorrow|YYYY-MM-DD[THH:MM]]
siddran task done <what> | undone <what>

siddran daily ls [--all]                # recurrence rendered as M T W T F S S
siddran daily done <what> | undone <what>   # recurring only; identity is (daily, date)
siddran project ls                      # priority is DERIVED from subtasks
siddran project show <what>
siddran project done <project> <subtask>

siddran event ls [--today|--days N|--all]
siddran event schedules
siddran settings ls | siddran settings get <key>
siddran bag path | status | check | trash | unlock [--force]
```

Titles match case-insensitively on a prefix, so `siddran note cat todo` finds `TODO`.

## Output

A terminal gets aligned columns; a pipe gets tab-separated fields, so no flag is needed:

```bash
siddran task ls --today | awk -F'\t' '{print $3}'
siddran note cat "Siddran Tickets" | grep '^- \[ \]'
siddran task ls --overdue | wc -l          # for a prompt or status bar
```

`--json` overrides both and emits the **raw rows as stored**, so absent keys stay absent — a
legacy task really does come back without a `due_all_day` key. Scripts see what is on disk, not
a normalized view of it.

Exit codes: `0` fine, `1` asked for something specific that is not there, `2` usage or no Bag,
`3` the Bag is locked by another Siddran process.

## AI access — `siddran tools`, `siddran call`, `siddran mcp`

**Any AI with a shell** (Claude Code, Codex, Gemini CLI, …) needs no setup. Tell it "use the
`siddran` command — start with `siddran tools`". That lists every tool; one runs with

```bash
siddran call note_edit '{"note":"Trip","insert_after":[{"anchor":"- socks","text":"- toothbrush"}]}'
echo '{"note":"Trip"}' | siddran call note_read -
```

JSON on stdout, an error on stderr with exit 1. Nothing runs in the background and no tool
descriptions cost tokens until the AI actually asks.

**AI apps without a shell** (the Claude desktop app, …) need MCP: `siddran mcp` serves the same
tools over the Model Context Protocol (stdio). For Claude Code, if you would rather it be
discovered automatically:

```bash
claude mcp add siddran -- siddran mcp            # uses your configured Bag
claude mcp add siddran -- siddran --bag ~/Documents/Siddran mcp
```

Both doors run the same 28 tools: notes (search, read, create, targeted edits — find/replace, insert before/after an anchor, rewrite one heading's section — rename/move/tag, trash), tasks,
dailies, projects, the calendar (agenda, events), sandboxes, and the desktop app's live state.

- **Edits merge, they do not overwrite.** Each tool call takes the Bag lock, reads the Bag fresh,
  changes one thing and writes it atomically. With the desktop app open, its watcher picks the
  change up within a second; a note you are typing in gets the AI's edit merged into the editor
  (line-level three-way merge — your unsaved typing stays). `note_edit` takes exact
  find/replace edits for exactly this reason: a targeted edit merges, a whole-body rewrite cannot.
- **Diagrams are written, not placed.** `sandbox_diagram` takes Mermaid-style flowchart text
  (`A[Start] --> B{OK?}`, `B -->|yes| C`) and lays it out — on a board, or embedded in a note.
  `sandbox_read` gives a board back as the same text, so a model can "see" what is drawn, and
  `sandbox_edit` adds a box *after* another, connects boxes by their text, and finds free space
  itself. The layout is `siddran-core::diagram`; the desktop's "Diagram from text" is a port of
  it with mirrored tests.
- **The desktop's live channel.** `app_context` says what is on screen — the page, the open note,
  the text selected in it, the open board — so "tidy up the paragraph I selected" works without
  copying anything. `app_open` asks the window to show a note, board or page. Both go through two
  small files in `$XDG_RUNTIME_DIR/siddran/`, outside the Bag (it gets synced; your screen does
  not), and nothing listens on a port.

## Safety

- **The lock is held per write, by everyone.** The desktop app takes `.siddran-lock` around each
  save, the TUI around each keypress that writes, `siddran mcp` around each tool call, and a
  one-shot command for its few milliseconds. Nobody holds it while merely open, so the desktop,
  the TUI, an AI and your shell can all have the same Bag open at once; a writer that finds it
  held waits a moment instead of failing. Reads take no lock at all — writes are atomic, so a
  reader sees the old file or the new one, never a torn one.
- **Every write is atomic** — temp file, fsync, rename over. A crash mid-write cannot leave a
  truncated note. The desktop app writes the same way.
- **Before writing, the Bag is re-checked** against the sizes and mtimes read at open. If
  anything moved, the write is refused rather than clobbering it. The TUI checks the same
  stamps on every keypress and reloads first, so it acts on what is on disk now — a task the
  desktop or an AI just added is there before your next key does anything.
- **Deletes go to `.siddran-trash/`** inside the Bag, recoverable byte-for-byte. `siddran bag
  trash` lists it.
- **`siddran note rm` asks first**, and a non-interactive caller gets a refusal and exit 1
  rather than a silent yes — pass `--yes` to opt in.

## `siddran bag check`

Re-serializes every file in the Bag and reports anything whose bytes would change. Writes
nothing. Run it any time you want to confirm the two apps still agree:

```
$ siddran bag check
51 file(s) checked — byte-identical ✓
```

## What is deliberately not here

- **The Sandbox canvas.** `sandboxes/*.siddran` is never parsed or touched.
- **Cloud sync.** The desktop's Cloudflare Worker covers tasks and calendar; this edits the same
  Bag, so it inherits sync without implementing any of it.
- **The Ember API.** Irrelevant — there is no server in this design.

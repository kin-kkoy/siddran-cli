//! Writes must disturb as little as possible.
//!
//! The desktop reads these same files. A write that reorders keys, normalises
//! numbers, or rewrites rows it did not touch turns every save into a whole-file
//! diff and, worse, can change meaning (see `docs/parity.md` on absent vs null).
//! These tests pin "changed exactly what was asked, and nothing else".

use siddran_core::bag::{lock, trash, write};
use siddran_core::Bag;
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag")
}

/// A throwaway copy, so a test can never touch the committed fixtures.
fn temp_bag() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dst = tmp.path().join("bag");
    copy_dir(&fixture(), &dst);
    (tmp, dst)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn line_diff(a: &str, b: &str) -> Vec<(String, String)> {
    a.lines()
        .zip(b.lines())
        .filter(|(x, y)| x != y)
        .map(|(x, y)| (x.trim().to_owned(), y.trim().to_owned()))
        .collect()
}

#[test]
fn editing_one_task_changes_only_that_task() {
    let (_tmp, root) = temp_bag();
    let path = root.join("tasks.siddran");
    let before = std::fs::read_to_string(&path).unwrap();

    let bag = Bag::open(&root).unwrap();
    let mut tasks = bag.tasks();
    let (dailies, completions, projects) = (bag.dailies(), bag.completions(), bag.projects());

    let i = tasks.iter().position(|t| t.v.title == "Modern uuid row").unwrap();
    tasks[i].set_title("Renamed in the terminal");

    write::tasks_file(&path, &tasks, &dailies, &completions, &projects).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();

    let diff = line_diff(&before, &after);
    assert_eq!(
        before.lines().count(),
        after.lines().count(),
        "the file changed length — a key was added or dropped"
    );
    assert_eq!(diff.len(), 2, "expected title + updated_at, got: {diff:#?}");
    assert!(diff[0].1.contains("Renamed in the terminal"));
    assert!(diff[1].0.starts_with("\"updated_at\""));
}

#[test]
fn a_legacy_row_keeps_its_absent_keys_when_a_sibling_is_edited() {
    let (_tmp, root) = temp_bag();
    let path = root.join("tasks.siddran");

    let bag = Bag::open(&root).unwrap();
    let mut tasks = bag.tasks();
    let i = tasks.iter().position(|t| t.v.title == "Modern uuid row").unwrap();
    tasks[i].set_completed(true);
    write::tasks_file(&path, &tasks, &bag.dailies(), &bag.completions(), &bag.projects()).unwrap();

    // Task 1625 has no `due_all_day` key at all, and must still not have one:
    // materializing it would convert its deadline semantics (deadline.js:58).
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let legacy = v["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == 1625)
        .expect("legacy row survived");
    assert!(
        legacy.get("due_all_day").is_none(),
        "absent key was materialized: {legacy}"
    );
    assert_eq!(legacy.as_object().unwrap().len(), 8);
}

#[test]
fn setting_a_deadline_writes_both_keys() {
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();
    let mut tasks = bag.tasks();
    let i = tasks.iter().position(|t| t.v.title.starts_with("Legacy numeric")).unwrap();

    assert!(tasks[i].raw.get("due_all_day").is_none(), "precondition");
    tasks[i].set_due(Some("2026-10-01T16:00:00.000Z"), true);

    // Changing the date on a legacy row must not leave the flag absent — that
    // would hand the desktop a modern date with legacy semantics.
    assert_eq!(tasks[i].raw["due_all_day"], serde_json::json!(true));
    assert_eq!(tasks[i].raw["due_date"], serde_json::json!("2026-10-01T16:00:00.000Z"));
}

#[test]
fn rewriting_the_whole_bag_still_round_trips() {
    // Belt and braces: the write path must produce what the read path proved.
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();
    let before = std::fs::read_to_string(root.join("tasks.siddran")).unwrap();
    write::tasks_file(
        &root.join("tasks.siddran"),
        &bag.tasks(),
        &bag.dailies(),
        &bag.completions(),
        &bag.projects(),
    )
    .unwrap();
    assert_eq!(before, std::fs::read_to_string(root.join("tasks.siddran")).unwrap());

    let before = std::fs::read_to_string(root.join("calendar.siddran")).unwrap();
    write::calendar_file(&root.join("calendar.siddran"), &bag.events(), &bag.schedules()).unwrap();
    assert_eq!(before, std::fs::read_to_string(root.join("calendar.siddran")).unwrap());

    let before = std::fs::read_to_string(root.join("settings.siddran")).unwrap();
    write::settings_file(&root.join("settings.siddran"), bag.settings_map().unwrap()).unwrap();
    assert_eq!(before, std::fs::read_to_string(root.join("settings.siddran")).unwrap());
}

#[test]
fn untouched_notes_are_not_rewritten() {
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();

    let before: Vec<_> = bag
        .notes
        .iter()
        .map(|n| (n.path.clone(), std::fs::metadata(&n.path).unwrap().modified().unwrap()))
        .collect();

    // Save exactly one note.
    let target = bag.notes.iter().find(|n| n.note.title == "Plain note").unwrap();
    siddran_core::bag::notes::save(&target.note, &target.path).unwrap();

    for (path, mtime) in &before {
        if path == &target.path {
            continue;
        }
        assert_eq!(
            &std::fs::metadata(path).unwrap().modified().unwrap(),
            mtime,
            "{} was rewritten without being asked",
            path.display()
        );
    }
}

#[test]
fn atomic_write_leaves_no_temp_files() {
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();
    write::tasks_file(
        &root.join("tasks.siddran"),
        &bag.tasks(),
        &bag.dailies(),
        &bag.completions(),
        &bag.projects(),
    )
    .unwrap();

    let strays: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".siddran-tmp-"))
        .collect();
    assert!(strays.is_empty(), "left behind: {strays:?}");
}

#[test]
fn deleting_a_note_moves_it_to_the_trash() {
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();
    let target = bag.notes.iter().find(|n| n.note.title == "Tagged").unwrap();
    let body = std::fs::read_to_string(&target.path).unwrap();

    let moved = siddran_core::bag::notes::delete(target, &root).unwrap();

    assert!(!target.path.exists(), "original should be gone");
    assert_eq!(moved.len(), 1);
    // Recoverable, byte for byte — the whole point of not calling unlink.
    assert_eq!(std::fs::read_to_string(&moved[0]).unwrap(), body);
    assert!(moved[0].starts_with(trash::dir(&root)));
}

#[test]
fn the_lock_excludes_a_second_process_and_reclaims_a_stale_one() {
    let (_tmp, root) = temp_bag();

    let guard = lock::acquire(&root, "test").expect("first acquire");
    assert!(root.join(lock::NAME).exists());

    match lock::acquire(&root, "test") {
        Err(lock::LockError::Held(h)) => assert_eq!(h.pid, std::process::id()),
        other => panic!("second acquire should have been refused, got {other:?}"),
    }

    drop(guard);
    assert!(!root.join(lock::NAME).exists(), "drop must release");

    // A lock from this host naming a pid that is gone is stale and reclaimable.
    std::fs::write(
        root.join(lock::NAME),
        serde_json::json!({
            "pid": 999_999_u32,
            "host": std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim(),
            "started_at": "2026-01-01T00:00:00.000Z",
            "mode": "test"
        })
        .to_string(),
    )
    .unwrap();
    let g = lock::acquire(&root, "test").expect("stale lock should be reclaimed");
    drop(g);
}

#[test]
fn an_unsorted_file_is_sorted_on_write_exactly_as_the_desktop_would() {
    // Not all Bags on disk are sorted — ~/Documents/Siddran-TestBag has an
    // unsorted `events` array. Our write sorts it, which is the same thing the
    // desktop's next flush would do (flushNow -> byId), so this is convergence,
    // not divergence. Pinned here so the sort can never be quietly dropped.
    let (_tmp, root) = temp_bag();
    let path = root.join("calendar.siddran");

    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["events"].as_array_mut().unwrap().reverse();
    std::fs::write(&path, siddran_core::json::to_string(&v)).unwrap();

    let bag = Bag::open(&root).unwrap();
    write::calendar_file(&path, &bag.events(), &bag.schedules()).unwrap();

    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let ids: Vec<String> = after["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| match &e["id"] {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "events were not written in byId order");
}

#[test]
fn stamps_detect_a_file_changing_under_us() {
    use siddran_core::bag::stamps::Stamps;

    let (_tmp, root) = temp_bag();
    let path = root.join("tasks.siddran");

    let mut stamps = Stamps::default();
    stamps.record(&path);
    assert!(stamps.changed().is_empty(), "nothing has changed yet");

    // Someone else writes — the desktop's 1.5s debounced flush, say.
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push(' ');
    std::fs::write(&path, &text).unwrap();

    let changed = stamps.changed();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0], path);

    // Deletion counts as changed too — a missing file is not an unchanged one.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(stamps.changed().len(), 1);
}

#[test]
fn a_note_rename_leaves_no_orphan_and_moves_its_sidecar() {
    use siddran_core::bag::notes;

    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();
    let nf = bag.notes.iter().find(|n| n.note.title == "Plain note").unwrap();

    // Give it a comment sidecar, which must travel with the note.
    let sidecar = notes::sidecar_of(&nf.path);
    std::fs::write(&sidecar, r#"{"version":1,"threads":[]}"#).unwrap();

    let mut note = nf.note.clone();
    note.title = "Renamed note".into();
    let to = notes::target_path(&bag, note.id.as_deref(), &note.title, nf.notebook.as_deref());
    notes::save_moving(&note, &nf.path, &to, &root).unwrap();

    assert!(to.exists(), "new file written");
    assert!(!nf.path.exists(), "old file removed, not left as an orphan");
    assert!(notes::sidecar_of(&to).exists(), "sidecar followed the note");
    assert!(!sidecar.exists(), "old sidecar removed");

    // And the old one is recoverable, not gone.
    assert!(trash::dir(&root).exists());
}

#[test]
fn a_title_collision_falls_back_to_the_id_suffix() {
    use siddran_core::bag::notes;

    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();

    // A *different* note claiming an existing title gets `<title>-<id>.md`,
    // matching localStore.js:174.
    let existing = bag.notes.iter().find(|n| n.note.title == "Plain note").unwrap();
    let p = notes::target_path(&bag, Some("9999"), "Plain note", None);
    assert!(p.to_string_lossy().ends_with("Plain note-9999.md"), "got {}", p.display());

    // ...but the note that already owns the name keeps it, so saving in place
    // never renames anything.
    let same = notes::target_path(&bag, existing.note.id.as_deref(), "Plain note", None);
    assert_eq!(same, existing.path);
}

#[test]
fn new_note_ids_continue_above_everything_on_disk() {
    use siddran_core::bag::notes;
    let (_tmp, root) = temp_bag();
    let bag = Bag::open(&root).unwrap();

    let next = notes::next_note_id(&bag);
    let highest = bag
        .notes
        .iter()
        .filter_map(|n| n.note.id.as_deref().and_then(|s| s.parse::<i64>().ok()))
        .max()
        .unwrap();
    assert!(next > highest, "{next} must be above {highest}");
    assert!(next > 1000, "seq is seeded at 1000 (localStore.js:143)");
}

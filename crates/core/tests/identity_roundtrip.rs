//! The contract test.
//!
//! Siddran-CLI shares a Bag with the desktop app, so anything it writes must be
//! byte-identical to what the desktop would have written. This reads every file
//! in a Bag, re-serializes it, and asserts the bytes are unchanged. One test
//! catches key ordering, float formatting, the `tags: ` trailing space,
//! absent-vs-null fields, escape forms and sort order.
//!
//! If this cannot be kept green, nothing built on top of it is safe.

use siddran_core::{json, Bag};
use std::path::{Path, PathBuf};

fn fixture_bag() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bag")
}

/// Re-serialize every file in the Bag and report the ones whose bytes changed.
/// Returns (divergences, files_actually_compared). The count exists so a future
/// refactor cannot quietly turn this suite into a test of nothing.
fn diverging_files_counted(root: &Path) -> (Vec<String>, usize) {
    let bag = Bag::open(root).expect("fixture should be a readable Bag");
    let mut bad = Vec::new();
    let mut checked = 0usize;

    for nf in &bag.notes {
        // A file with no frontmatter is not byte-stable by definition: writing
        // it adds a block, exactly as the desktop does. Nothing to assert.
        if !nf.note.had_frontmatter {
            continue;
        }
        checked += 1;
        let out = nf.note.to_md();
        if out != nf.raw {
            bad.push(format!(
                "{}\n  expected: {:?}\n  got:      {:?}",
                nf.path.display(),
                first_difference(&nf.raw, &out).0,
                first_difference(&nf.raw, &out).1
            ));
        }
    }

    for (name, value) in &bag.notebooks {
        let path = root.join("notes").join(name).join(".siddran-notebook.json");
        checked += 1;
        check_json(&path, value, &mut bad);
    }
    for (file, value) in [
        ("tasks.siddran", &bag.tasks),
        ("calendar.siddran", &bag.calendar),
        ("settings.siddran", &bag.settings),
    ] {
        if let Some(v) = value {
            checked += 1;
            check_json(&root.join(file), v, &mut bad);
        }
    }
    (bad, checked)
}

fn check_json(path: &Path, value: &serde_json::Value, bad: &mut Vec<String>) {
    let original = std::fs::read_to_string(path).expect("file was just read by Bag::open");
    let out = json::to_string(value);
    if out != original {
        let (a, b) = first_difference(&original, &out);
        bad.push(format!(
            "{}\n  expected: {a:?}\n  got:      {b:?}",
            path.display()
        ));
    }
}

/// A byte-offset diff is unreadable for a 4 KB file; show the first line that
/// actually differs instead.
fn first_difference(expected: &str, got: &str) -> (String, String) {
    for (a, b) in expected.lines().zip(got.lines()) {
        if a != b {
            return (a.to_owned(), b.to_owned());
        }
    }
    (
        format!("<{} lines>", expected.lines().count()),
        format!("<{} lines>", got.lines().count()),
    )
}

#[test]
fn fixture_bag_round_trips_byte_for_byte() {
    let (bad, checked) = diverging_files_counted(&fixture_bag());
    assert!(
        checked >= 11,
        "only {checked} files compared — the fixture Bag lost coverage"
    );
    assert!(
        bad.is_empty(),
        "{} file(s) would be rewritten:\n\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}

/// Opt-in, because it needs a Bag this machine happens to have:
///     SIDDRAN_TEST_BAG=~/Documents/Siddran cargo test -p siddran-core
/// Read-only — it copies nothing and writes nothing.
#[test]
fn real_bag_round_trips_byte_for_byte() {
    let Ok(path) = std::env::var("SIDDRAN_TEST_BAG") else {
        eprintln!("skipped: set SIDDRAN_TEST_BAG to a real Bag to run this");
        return;
    };
    let root = PathBuf::from(path);
    let (bad, checked) = diverging_files_counted(&root);
    assert!(checked > 0, "{} held no comparable files", root.display());
    eprintln!("compared {checked} files in {}", root.display());
    assert!(
        bad.is_empty(),
        "{} file(s) in {} would be rewritten:\n\n{}",
        bad.len(),
        root.display(),
        bad.join("\n\n")
    );
}

#[test]
fn safe_name_matches_the_desktop_rules() {
    use siddran_core::bag::layout::safe_name;
    assert_eq!(safe_name("a/b:c*d?e\"f<g>h|i"), "a_b_c_d_e_f_g_h_i");
    assert_eq!(safe_name("  spaced   out  "), "spaced out");
    assert_eq!(safe_name(""), "Untitled");
    assert_eq!(safe_name("///"), "___");
    assert_eq!(safe_name(&"x".repeat(200)).chars().count(), 120);
    assert_eq!(safe_name("line\nbreak"), "line break");
}

#[test]
fn ids_sort_as_strings_not_numbers() {
    use siddran_core::Id;
    // localStore.js:215 sorts on String(id): "1000" < "999" is intended.
    let mut ids = vec![Id::Num(999), Id::Num(1000), Id::Str("6d5b".into())];
    ids.sort_by_key(|i| i.sort_key());
    assert_eq!(
        ids,
        vec![Id::Num(1000), Id::Str("6d5b".into()), Id::Num(999)]
    );
    assert_eq!(Id::parse("1625"), Some(Id::Num(1625)));
    assert_eq!(Id::parse("6d5b-x"), Some(Id::Str("6d5b-x".into())));
}

//! The preview renderer. These pin the extensions the CodeMirror editor shows,
//! since matching *that* — not CommonMark — is the goal.

use siddran_core::md::{self, Kind, Style, Target};

fn kinds(body: &str) -> Vec<Kind> {
    md::render(body).into_iter().map(|l| l.kind).collect()
}

fn text(body: &str) -> String {
    md::render(body)
        .into_iter()
        .map(|l| l.spans.iter().map(|s| s.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn styles(line: &str) -> Vec<(Style, String)> {
    md::render(line)[0]
        .spans
        .iter()
        .map(|s| (s.style, s.text.clone()))
        .collect()
}

#[test]
fn block_kinds() {
    assert_eq!(kinds("# H1"), vec![Kind::Heading(1)]);
    assert_eq!(kinds("### H3"), vec![Kind::Heading(3)]);
    // Seven hashes is not a heading, and `#tag` is not either.
    assert_eq!(kinds("####### too many"), vec![Kind::Para]);
    assert_eq!(kinds("#hashtag alone"), vec![Kind::Para]);

    assert_eq!(kinds("> quoted"), vec![Kind::Quote]);
    assert_eq!(kinds("> [!note] heads up"), vec![Kind::Callout]);
    assert_eq!(kinds("- item"), vec![Kind::ListItem(0)]);
    assert_eq!(kinds("1. item"), vec![Kind::ListItem(0)]);
    assert_eq!(kinds("- [ ] open"), vec![Kind::Task(false, 0)]);
    assert_eq!(kinds("- [x] done"), vec![Kind::Task(true, 0)]);
    assert_eq!(kinds("---"), vec![Kind::Rule]);
    assert_eq!(kinds("| a | b |"), vec![Kind::Table]);
    assert_eq!(kinds(""), Vec::<Kind>::new());
    assert_eq!(kinds("\n"), vec![Kind::Blank]);
}

#[test]
fn fenced_code_is_not_markdown() {
    let k = kinds("```rust\n# not a heading\n- not a list\n```");
    assert_eq!(k, vec![Kind::CodeFence, Kind::Code, Kind::Code]);
}

#[test]
fn a_diagram_block_collapses_to_one_placeholder() {
    // The canvas is out of scope, but the block must be visible as *present* —
    // it round-trips untouched through an edit, so hiding it would be a lie.
    let body = "before\n\n```siddran-diagram\n{\"v\":1,\"items\":[{\"id\":\"a\"},{\"id\":\"b\"}]}\n```\n\nafter";
    let lines = md::render(body);
    let diagram: Vec<_> = lines.iter().filter(|l| l.kind == Kind::Diagram).collect();
    assert_eq!(diagram.len(), 1);
    assert!(diagram[0].spans[0].text.contains("2 items"), "{:?}", diagram[0].spans);
    // ...and the JSON itself is never shown.
    assert!(!text(body).contains("\"v\":1"));
}

#[test]
fn an_unterminated_diagram_still_shows() {
    let lines = md::render("```siddran-diagram\n{\"items\":[{\"id\":\"a\"}]");
    assert!(lines.iter().any(|l| l.kind == Kind::Diagram), "truncated note lost its block");
}

#[test]
fn inline_extensions() {
    assert_eq!(styles("**bold**"), vec![(Style::Strong, "bold".into())]);
    assert_eq!(styles("==hi=="), vec![(Style::Highlight, "hi".into())]);
    assert_eq!(styles("++u++"), vec![(Style::Underline, "u".into())]);
    assert_eq!(styles("||secret||"), vec![(Style::Spoiler, "secret".into())]);
    assert_eq!(styles("`code`"), vec![(Style::Code, "code".into())]);
    assert_eq!(styles("~~gone~~"), vec![(Style::Strike, "gone".into())]);

    // `**` must win over `*`.
    assert_eq!(styles("**b**"), vec![(Style::Strong, "b".into())]);

    // A link shows its text, not its URL.
    assert_eq!(
        styles("[text](https://example.com)"),
        vec![(Style::Link, "text".into())]
    );
}

#[test]
fn hashtags_need_a_word_boundary() {
    // `C#` must not become a tag — the Bag is full of C# notes.
    let s = styles("learning C# today");
    assert!(s.iter().all(|(st, _)| *st == Style::Plain), "{s:?}");

    let s = styles("tagged #rust here");
    assert!(s.iter().any(|(st, t)| *st == Style::Tag && t == "#rust"), "{s:?}");
}

#[test]
fn wikilinks_render_and_parse() {
    assert_eq!(styles("[[A Note]]"), vec![(Style::WikiLink, "A Note".into())]);
    // An alias is what gets shown.
    assert_eq!(styles("[[task:123|do the thing]]"), vec![(Style::WikiLink, "do the thing".into())]);
    // Without one, the type prefix is stripped.
    assert_eq!(styles("[[task:123]]"), vec![(Style::WikiLink, "123".into())]);

    use md::parse_typed_link as p;
    assert_eq!(
        p("A Note#Heading"),
        Some((Target::Note { title: "A Note".into(), heading: Some("Heading".into()) }, None))
    );
    assert_eq!(p("task:42"), Some((Target::Task("42".into()), None)));
    assert_eq!(p("bundle:7"), Some((Target::Project("7".into()), None)));
    assert_eq!(p("sandbox:abc"), Some((Target::Sandbox("abc".into()), None)));
    assert_eq!(
        p("Note|shown"),
        Some((Target::Note { title: "Note".into(), heading: None }, Some("shown".into())))
    );
    assert_eq!(p(""), None);
}

#[test]
fn utf8_survives_the_scanner() {
    // A byte-indexed scanner that steps by 1 would panic or corrupt here.
    let body = "Grüße — 日本語 ✓ and **強調**";
    let out = text(body);
    assert!(out.contains("日本語"), "{out}");
    assert!(out.contains("強調"), "{out}");
    assert!(out.contains('—'));
}

#[test]
fn real_notes_render_without_panicking() {
    // The renderer runs over whatever is in the user's Bag, so it must not have
    // opinions about what it finds.
    let Ok(bag_path) = std::env::var("SIDDRAN_TEST_BAG") else { return };
    let bag = siddran_core::Bag::open(&bag_path).expect("opens");
    let mut total = 0;
    for n in &bag.notes {
        let lines = md::render(&n.note.body);
        total += lines.len();
    }
    assert!(total > 0, "rendered nothing from {} notes", bag.notes.len());
    eprintln!("rendered {total} lines from {} notes", bag.notes.len());
}

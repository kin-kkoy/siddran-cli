//! Path rules. `safe_name` mirrors localStore.js:35-36 exactly — the reserved
//! set, the whitespace collapse, the 120-char cap and the `Untitled` fallback
//! all decide filenames, so a divergence here renames the user's files.

const RESERVED: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

pub fn safe_name(s: &str) -> String {
    let replaced: String = s
        .chars()
        .map(|c| if RESERVED.contains(&c) { '_' } else { c })
        .collect();

    // `replace(/\s+/g, ' ')` — any run of whitespace becomes one space.
    let mut collapsed = String::with_capacity(replaced.len());
    let mut in_ws = false;
    for c in replaced.chars() {
        if c.is_whitespace() {
            if !in_ws {
                collapsed.push(' ');
            }
            in_ws = true;
        } else {
            collapsed.push(c);
            in_ws = false;
        }
    }

    // JS `.slice(0, 120)` counts UTF-16 code units; on the titles a Bag
    // actually holds these agree, and truncating on a char boundary is the
    // safe reading. Trim happens before the slice, as in the original.
    let trimmed = collapsed.trim();
    let out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        "Untitled".to_owned()
    } else {
        out
    }
}

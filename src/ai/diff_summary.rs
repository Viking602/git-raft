use crate::git::DiffStat;

/// Truncate a string to at most `max_bytes` without splitting a multi-byte character.
fn truncate_to_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Max summary length per file in bytes.
const PER_FILE_SUMMARY_LIMIT: usize = 512;
/// Max total summary length in bytes.
const TOTAL_SUMMARY_LIMIT: usize = 8192;

/// Build a compact structured summary from diff stats with raw diff content.
/// Returns a single string suitable for inclusion in the AI prompt.
pub(crate) fn summarize_diff_stats(
    stats: &[DiffStat],
    untracked_previews: &[(String, String)],
) -> String {
    let mut out = String::new();
    let mut total_bytes = 0usize;

    for stat in stats {
        if total_bytes >= TOTAL_SUMMARY_LIMIT {
            out.push_str("... (remaining files omitted due to size limit)\n");
            break;
        }
        let summary = match &stat.diff_content {
            Some(raw_diff) => summarize_one_diff(&stat.path, raw_diff),
            None => format!(
                "{}: +{} -{} (diff content unavailable)\n",
                stat.path, stat.additions, stat.deletions,
            ),
        };
        let remaining = TOTAL_SUMMARY_LIMIT.saturating_sub(total_bytes);
        let trimmed = truncate_str(&summary, PER_FILE_SUMMARY_LIMIT.min(remaining));
        total_bytes += trimmed.len();
        out.push_str(&trimmed);
    }

    // Untracked (new) files
    for (path, content) in untracked_previews {
        if total_bytes >= TOTAL_SUMMARY_LIMIT {
            out.push_str("... (remaining new files omitted due to size limit)\n");
            break;
        }
        let summary = summarize_new_file(path, content);
        let remaining = TOTAL_SUMMARY_LIMIT.saturating_sub(total_bytes);
        let trimmed = truncate_str(&summary, PER_FILE_SUMMARY_LIMIT.min(remaining));
        total_bytes += trimmed.len();
        out.push_str(&trimmed);
    }

    out
}

/// Parse a unified diff for a single file and extract key signals.
fn summarize_one_diff(path: &str, raw_diff: &str) -> String {
    let mut added_symbols = Vec::new();
    let mut removed_symbols = Vec::new();
    let mut modified_lines_add = 0usize;
    let mut modified_lines_del = 0usize;

    for line in raw_diff.lines() {
        if let Some(rest) = line.strip_prefix('+') {
            if rest.starts_with("++") {
                continue; // skip +++ header
            }
            let trimmed = rest.trim();
            if let Some(sym) = extract_symbol(trimmed) {
                added_symbols.push(sym);
            } else if !trimmed.is_empty() {
                modified_lines_add += 1;
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if rest.starts_with("--") {
                continue; // skip --- header
            }
            let trimmed = rest.trim();
            if let Some(sym) = extract_symbol(trimmed) {
                removed_symbols.push(sym);
            } else if !trimmed.is_empty() {
                modified_lines_del += 1;
            }
        }
    }

    let mut summary = format!("{}:\n", path);

    for sym in &added_symbols {
        summary.push_str(&format!("  + {sym}\n"));
    }
    for sym in &removed_symbols {
        summary.push_str(&format!("  - {sym}\n"));
    }
    if modified_lines_add > 0 || modified_lines_del > 0 {
        summary.push_str(&format!(
            "  ~ {modified_lines_add} lines added, {modified_lines_del} lines removed\n"
        ));
    }

    // If no symbols were extracted, show a few meaningful added lines as context
    if added_symbols.is_empty()
        && removed_symbols.is_empty()
        && modified_lines_add == 0
        && modified_lines_del == 0
    {
        summary.push_str("  (no substantive changes detected)\n");
    }

    summary
}

/// Summarize a new (untracked) file by extracting symbols from its content.
fn summarize_new_file(path: &str, content: &str) -> String {
    let mut symbols = Vec::new();
    let total_lines = content.lines().count();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(sym) = extract_symbol(trimmed) {
            symbols.push(sym);
        }
    }

    let mut summary = format!("{} (new file, {total_lines} lines):\n", path);
    for sym in &symbols {
        summary.push_str(&format!("  + {sym}\n"));
    }
    if symbols.is_empty() && total_lines > 0 {
        // Show first non-empty lines as hint
        let hints: Vec<&str> = content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
            .take(3)
            .collect();
        for hint in hints {
            let display = truncate_to_char_boundary(hint, 80);
            summary.push_str(&format!("  | {display}\n"));
        }
    }

    summary
}

/// Extract a meaningful symbol definition from a code line.
/// Returns a short description like "fn foo()", "struct Bar", "use crate::x", etc.
fn extract_symbol(line: &str) -> Option<String> {
    // Skip comments and empty lines
    if line.is_empty()
        || line.starts_with("//")
        || line.starts_with('#')
        || line.starts_with("/*")
        || line.starts_with('*')
    {
        return None;
    }

    // Rust / general patterns
    let patterns: &[(&str, fn(&str) -> Option<String>)] = &[
        ("pub async fn ", |l| extract_after(l, "pub async fn ")),
        ("pub fn ", |l| extract_after(l, "pub fn ")),
        ("pub(crate) async fn ", |l| {
            extract_after(l, "pub(crate) async fn ")
        }),
        ("pub(crate) fn ", |l| extract_after(l, "pub(crate) fn ")),
        ("pub(super) async fn ", |l| {
            extract_after(l, "pub(super) async fn ")
        }),
        ("pub(super) fn ", |l| extract_after(l, "pub(super) fn ")),
        ("async fn ", |l| extract_after(l, "async fn ")),
        ("fn ", |l| extract_after(l, "fn ")),
        ("pub struct ", |l| extract_name(l, "pub struct ")),
        ("struct ", |l| extract_name(l, "struct ")),
        ("pub enum ", |l| extract_name(l, "pub enum ")),
        ("enum ", |l| extract_name(l, "enum ")),
        ("pub trait ", |l| extract_name(l, "pub trait ")),
        ("trait ", |l| extract_name(l, "trait ")),
        ("impl ", |l| extract_impl(l)),
        ("pub mod ", |l| extract_name(l, "pub mod ")),
        ("mod ", |l| extract_name(l, "mod ")),
        ("pub use ", |l| extract_use(l, "pub use ")),
        ("use ", |l| extract_use(l, "use ")),
        ("pub const ", |l| extract_name(l, "pub const ")),
        ("const ", |l| extract_name(l, "const ")),
        ("pub static ", |l| extract_name(l, "pub static ")),
        ("pub type ", |l| extract_name(l, "pub type ")),
        ("type ", |l| extract_name(l, "type ")),
        // JS/TS patterns
        ("export function ", |l| extract_after(l, "export function ")),
        ("export default function ", |l| {
            extract_after(l, "export default function ")
        }),
        ("export class ", |l| extract_name(l, "export class ")),
        ("export interface ", |l| {
            extract_name(l, "export interface ")
        }),
        ("export type ", |l| extract_name(l, "export type ")),
        ("function ", |l| extract_after(l, "function ")),
        ("class ", |l| extract_name(l, "class ")),
        ("interface ", |l| extract_name(l, "interface ")),
        // Python patterns
        ("def ", |l| extract_after(l, "def ")),
        ("class ", |l| extract_name(l, "class ")),
        ("import ", |l| {
            Some(format!("import {}", l.strip_prefix("import ")?.trim()))
        }),
        ("from ", |l| Some(truncate_symbol(l, 60))),
        // Go patterns
        ("func ", |l| extract_after(l, "func ")),
    ];

    for (prefix, extractor) in patterns {
        if line.contains(prefix) {
            if let Some(sym) = extractor(line) {
                return Some(sym);
            }
        }
    }

    None
}

fn extract_after(line: &str, prefix: &str) -> Option<String> {
    let rest = line.split(prefix).nth(1)?;
    // Take up to the opening brace or end of signature
    let sig = rest.split('{').next().unwrap_or(rest).trim();
    if sig.is_empty() {
        return None;
    }
    Some(format!("{} {}", prefix.trim(), truncate_symbol(sig, 50)))
}

fn extract_name(line: &str, prefix: &str) -> Option<String> {
    let rest = line.split(prefix).nth(1)?;
    let name = rest
        .split_whitespace()
        .next()
        .or_else(|| rest.split('{').next())
        .or_else(|| rest.split('(').next())
        .or_else(|| rest.split(';').next())?
        .trim();
    if name.is_empty() {
        return None;
    }
    Some(format!("{} {}", prefix.trim(), name))
}

fn extract_impl(line: &str) -> Option<String> {
    let rest = line.split("impl").nth(1)?.trim();
    let block = rest.split('{').next().unwrap_or(rest).trim();
    if block.is_empty() {
        return None;
    }
    Some(format!("impl {}", truncate_symbol(block, 50)))
}

fn extract_use(line: &str, prefix: &str) -> Option<String> {
    let rest = line.split(prefix).nth(1)?.trim().trim_end_matches(';');
    if rest.is_empty() {
        return None;
    }
    Some(format!("{} {}", prefix.trim(), truncate_symbol(rest, 50)))
}

fn truncate_symbol(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", truncate_to_char_boundary(s, max))
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...\n", truncate_to_char_boundary(s, max))
    }
}

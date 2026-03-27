use std::fs;
use std::path::Path;

#[test]
fn agents_md_stays_short_and_index_like() {
    let content = fs::read_to_string("AGENTS.md").expect("read AGENTS.md");
    let line_count = content.lines().count();
    assert!(
        (80..=120).contains(&line_count),
        "AGENTS.md should stay between 80 and 120 lines, got {line_count}"
    );
    assert!(content.contains("docs/architecture/"));
    assert!(content.contains("docs/quality/"));
    assert!(content.contains(".git/git-raft/runs/"));
}

#[test]
fn docs_scaffold_exists() {
    for path in [
        "docs/index.md",
        "docs/architecture/index.md",
        "docs/architecture/runtime.md",
        "docs/product/index.md",
        "docs/product/cli.md",
        "docs/exec-plans/active/index.md",
        "docs/exec-plans/completed/index.md",
        "docs/generated/index.md",
        "docs/quality/index.md",
    ] {
        assert!(Path::new(path).exists(), "missing {path}");
    }
}

#[test]
fn makefile_exists_with_local_dev_targets() {
    let content = fs::read_to_string("Makefile").expect("read Makefile");
    for target in [
        "help:",
        "build:",
        "test:",
        "cli-test:",
        "guardrails:",
        "fmt:",
        "fmt-check:",
        "install:",
    ] {
        assert!(content.contains(target), "missing target {target}");
    }
}

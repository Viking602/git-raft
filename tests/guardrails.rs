use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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
fn real_merge_runbook_and_scripts_exist() {
    for path in [
        "docs/quality/real-merge.md",
        "scripts/setup_real_merge_fixture.sh",
        "scripts/run_real_merge_scenario.sh",
    ] {
        assert!(Path::new(path).exists(), "missing {path}");
    }
}

#[test]
#[cfg(not(windows))]
fn real_merge_shell_scripts_parse() {
    for script in [
        "scripts/setup_real_merge_fixture.sh",
        "scripts/run_real_merge_scenario.sh",
    ] {
        let status = StdCommand::new("sh")
            .arg("-n")
            .arg(script)
            .status()
            .expect("spawn sh -n");
        assert!(status.success(), "shell syntax check failed for {script}");
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

#[test]
fn config_module_is_split_into_internal_submodules() {
    let content = fs::read_to_string("src/config.rs").expect("read src/config.rs");
    let line_count = content.lines().count();
    assert!(
        line_count <= 80,
        "src/config.rs should stay a thin facade, got {line_count} lines"
    );

    for snippet in ["mod defaults;", "mod files;", "mod merge;", "mod types;"] {
        assert!(
            content.contains(snippet),
            "src/config.rs should declare {snippet}"
        );
    }

    for path in [
        "src/config/defaults.rs",
        "src/config/files.rs",
        "src/config/merge.rs",
        "src/config/types.rs",
    ] {
        assert!(Path::new(path).exists(), "missing {path}");
    }
}

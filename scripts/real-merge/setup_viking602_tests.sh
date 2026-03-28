#!/usr/bin/env bash
set -euo pipefail

repo_dir="${1:-}"
if [[ -z "$repo_dir" ]]; then
  echo "usage: $0 <repo-dir>" >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

mkdir -p "$repo_dir"
cd "$repo_dir"

if [[ ! -d .git ]]; then
  git init --initial-branch=master
fi

if [[ -n "$(git status --short)" ]]; then
  echo "repository must be clean before setup" >&2
  exit 1
fi

if git rev-parse --verify HEAD >/dev/null 2>&1; then
  echo "repository already has commits; use an empty fixture repo" >&2
  exit 1
fi

git config user.name "${GIT_AUTHOR_NAME:-Git Raft Fixture}"
git config user.email "${GIT_AUTHOR_EMAIL:-git-raft-fixture@example.com}"

write_base_files() {
  mkdir -p src tests
  cat > Cargo.toml <<'EOF'
[package]
name = "tests"
version = "0.1.0"
edition = "2024"

[dependencies]
EOF

  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let sum: i32 = values.iter().sum();
    format!("sum={sum}")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::render_report;

#[test]
fn renders_sum() {
    assert_eq!(render_report(&[1, 2, 3]), "sum=6");
}
EOF
}

write_success_feature() {
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    let count = sorted.len();
    let sum: i32 = sorted.iter().sum();
    let average = if count == 0 {
        0.0
    } else {
        sum as f64 / count as f64
    };

    format!("report: sorted={sorted:?}; count={count}; avg={average:.2}; feature-token")
}

pub fn render_sorted_average_report(values: &[i32]) -> String {
    render_report(values)
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::*;

#[test]
fn renders_sorted_average_report() {
    assert_eq!(
        render_sorted_average_report(&[3, 1, 2]),
        "report: sorted=[1, 2, 3]; count=3; avg=2.00; feature-token"
    );
}
EOF
}

write_success_target() {
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let count = values.len();
    let sum: i32 = values.iter().sum();
    let min = values.iter().copied().min().unwrap_or_default();

    format!("summary: values={values:?}; count={count}; min={min}; sum={sum}; target-token")
}

pub fn render_summary_report(values: &[i32]) -> String {
    render_report(values)
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::*;

#[test]
fn renders_summary_report() {
    let rendered = render_summary_report(&[3, 1, 2]);
    assert!(rendered.contains("summary:"));
    assert!(rendered.contains("count=3"));
    assert!(rendered.contains("target-token"));
}

#[test]
fn render_report_stays_summary_entrypoint() {
    assert_eq!(render_report(&[3, 1, 2]), render_summary_report(&[3, 1, 2]));
}
EOF
}

write_validation_feature() {
  cat > src/lib.rs <<'EOF'
pub fn render_sorted_view(values: &[i32]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    let count = sorted.len();
    format!("ordered={sorted:?}; count={count}; sorted-order-token")
}

pub fn render_report(values: &[i32]) -> String {
    let count = values.len();
    format!("ordered={values:?}; count={count}; stable-order-token")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::*;

#[test]
fn renders_sorted_order() {
    assert_eq!(
        render_sorted_view(&[3, 1, 2]),
        "ordered=[1, 2, 3]; count=3; sorted-order-token"
    );
}
EOF
}

write_validation_target() {
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    render_stable_view(values)
}

pub fn render_stable_view(values: &[i32]) -> String {
    let count = values.len();
    format!("ordered={values:?}; count={count}; stable-order-token")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::*;

#[test]
fn keeps_original_order() {
    assert_eq!(
        render_stable_view(&[3, 1, 2]),
        "ordered=[3, 1, 2]; count=3; stable-order-token"
    );
}

#[test]
fn render_report_stays_stable_entrypoint() {
    assert_eq!(render_report(&[3, 1, 2]), render_stable_view(&[3, 1, 2]));
}
EOF
}

write_repair_feature() {
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let max = values.iter().copied().max().unwrap_or_default();
    let shared_header = "repair-shared";
    let feature_notes = ["feature-note-1", "feature-note-2", "feature-note-3"];

    format!("{shared_header}; max={max}; notes={feature_notes:?}")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::render_report;

#[test]
fn renders_feature_notes() {
    let rendered = render_report(&[3, 1, 2]);
    assert!(rendered.contains("repair-shared"));
    assert!(rendered.contains("feature-note-3"));
}
EOF
}

write_repair_target() {
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let total: i32 = values.iter().sum();
    let shared_header = "repair-shared";
    let target_notes = ["target-note-1", "target-note-2", "target-note-3"];

    format!("{shared_header}; total={total}; notes={target_notes:?}")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::render_report;

#[test]
fn renders_target_notes() {
    let rendered = render_report(&[3, 1, 2]);
    assert!(rendered.contains("repair-shared"));
    assert!(rendered.contains("target-note-3"));
}
EOF
}

write_binary_feature() {
  mkdir -p assets
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let sum: i32 = values.iter().sum();
    format!("binary-feature sum={sum}")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::render_report;

#[test]
fn renders_binary_feature_branch() {
    assert_eq!(render_report(&[1, 2, 3]), "binary-feature sum=6");
}
EOF

  printf '\000feature-binary\n' > assets/logo.bin
}

write_binary_target() {
  mkdir -p assets
  cat > src/lib.rs <<'EOF'
pub fn render_report(values: &[i32]) -> String {
    let sum: i32 = values.iter().sum();
    format!("binary-target sum={sum}")
}
EOF

  cat > tests/math_merge.rs <<'EOF'
use tests::render_report;

#[test]
fn renders_binary_target_branch() {
    assert_eq!(render_report(&[1, 2, 3]), "binary-target sum=6");
}
EOF

  printf '\000target-binary\n' > assets/logo.bin
}

commit_all() {
  local message="$1"
  git add .
  git commit -m "$message"
}

write_base_files
commit_all "init: seed merge fixture"
base_commit="$(git rev-parse HEAD)"

git checkout -b feature/format-report "$base_commit"
write_success_feature
commit_all "feat: add sorted average report"

git checkout master
write_success_target
commit_all "feat: add summary report"

git checkout -b scenario/validation-feature "$base_commit"
write_validation_feature
commit_all "feat: add sorted order scenario"

git checkout -b scenario/validation-target "$base_commit"
write_validation_target
commit_all "feat: add stable order scenario"

git checkout -b scenario/repair-feature "$base_commit"
write_repair_feature
commit_all "feat: add repair feature notes"

git checkout -b scenario/repair-target "$base_commit"
write_repair_target
commit_all "feat: add repair target notes"

git checkout -b scenario/binary-feature "$base_commit"
write_binary_feature
commit_all "feat: add binary feature branch"

git checkout -b scenario/binary-target "$base_commit"
write_binary_target
commit_all "feat: add binary target branch"

git checkout master

cat <<'EOF'
fixture branches created:
- master
- feature/format-report
- scenario/validation-target
- scenario/validation-feature
- scenario/repair-target
- scenario/repair-feature
- scenario/binary-target
- scenario/binary-feature
EOF

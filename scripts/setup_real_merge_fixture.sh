#!/bin/sh
set -eu

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo "usage: $0 <repo-dir> [remote-url]" >&2
    exit 1
fi

repo_dir=$1
remote_url=${2-}

require_clean_repo() {
    if [ -n "$(git status --porcelain)" ]; then
        echo "worktree is not clean: $repo_dir" >&2
        exit 1
    fi
}

write_base_files() {
    mkdir -p src tests docs assets
    cat <<'EOF' > Cargo.toml
[package]
name = "tests"
version = "0.1.0"
edition = "2024"

[dependencies]
EOF

    cat <<'EOF' > src/lib.rs
pub fn render_report(values: &[i32]) -> String {
    let sum: i32 = values.iter().sum();
    format!("sum={sum}")
}
EOF

    cat <<'EOF' > tests/math_merge.rs
use tests::render_report;

#[test]
fn renders_sum() {
    assert!(render_report(&[1, 2, 3]).contains("sum=6"));
}
EOF

    cat <<'EOF' > docs/merge_notes.md
Shared intro
Shared outro
EOF

    printf 'base\n' > assets/logo.bin
}

write_success_main() {
    cat <<'EOF' > src/lib.rs
pub fn render_report(values: &[i32]) -> String {
    let sum: i32 = values.iter().sum();
    let min = values.iter().copied().min().unwrap_or_default();
    let count = values.len();
    format!("report count={count} min={min} sum={sum} values={values:?}")
}
EOF

    cat <<'EOF' > tests/math_merge.rs
use tests::render_report;

#[test]
fn renders_count_min_and_sum() {
    let report = render_report(&[3, 1, 2]);
    assert!(report.contains("count=3"));
    assert!(report.contains("min=1"));
    assert!(report.contains("sum=6"));
}
EOF
}

write_success_feature() {
    cat <<'EOF' > src/lib.rs
pub fn render_report(values: &[i32]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    let sum: i32 = sorted.iter().sum();
    let avg = if sorted.is_empty() {
        0.0
    } else {
        sum as f64 / sorted.len() as f64
    };
    format!(
        "report count={} avg={avg:.2} sum={sum} sorted={sorted:?}",
        sorted.len()
    )
}
EOF

    cat <<'EOF' > tests/math_merge.rs
use tests::render_report;

#[test]
fn renders_count_avg_and_sum() {
    let report = render_report(&[3, 1, 2]);
    assert!(report.contains("count=3"));
    assert!(report.contains("avg=2.00"));
    assert!(report.contains("sum=6"));
}
EOF
}

write_validation_main() {
    cat <<'EOF' > src/lib.rs
pub fn render_report(values: &[i32]) -> String {
    let joined = values
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("ordered={joined}")
}
EOF

    cat <<'EOF' > tests/math_merge.rs
use tests::render_report;

#[test]
fn keeps_original_order() {
    let report = render_report(&[3, 1, 2]);
    assert!(report.contains("ordered=3,1,2"));
}
EOF
}

write_validation_feature() {
    cat <<'EOF' > src/lib.rs
pub fn render_report(values: &[i32]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort();
    let joined = sorted
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("ordered={joined}")
}
EOF

    cat <<'EOF' > tests/math_merge.rs
use tests::render_report;

#[test]
fn sorts_before_rendering() {
    let report = render_report(&[3, 1, 2]);
    assert!(report.contains("ordered=1,2,3"));
}
EOF
}

write_retry_main() {
    cat <<'EOF' > docs/merge_notes.md
Shared intro

Main checklist:
- keep alpha
- keep beta
- keep gamma

Shared outro
EOF
}

write_retry_feature() {
    cat <<'EOF' > docs/merge_notes.md
Shared intro

Feature checklist:
- keep delta
- keep epsilon
- keep zeta

Shared outro
EOF
}

write_binary_main() {
    printf 'main-binary\001' > assets/logo.bin
}

write_binary_feature() {
    printf 'feature-binary\002' > assets/logo.bin
}

if [ ! -d "$repo_dir/.git" ]; then
    if [ -n "$remote_url" ]; then
        git clone "$remote_url" "$repo_dir"
    else
        mkdir -p "$repo_dir"
        cd "$repo_dir"
        git init --initial-branch=master
        cd - >/dev/null
    fi
fi

cd "$repo_dir"
git config user.name "${GIT_AUTHOR_NAME:-Test User}"
git config user.email "${GIT_AUTHOR_EMAIL:-test@example.com}"

if git rev-parse --verify HEAD >/dev/null 2>&1; then
    require_clean_repo
else
    git checkout -B master
    write_base_files
    git add Cargo.toml src/lib.rs tests/math_merge.rs docs/merge_notes.md assets/logo.bin
    git commit -m "feat(fixtures): add real merge base fixture"
fi

git checkout master
require_clean_repo

git checkout -B scenario/success-main master
write_success_main
git add src/lib.rs tests/math_merge.rs
git commit -m "feat(fixtures): add success main branch"

git checkout -B scenario/success-feature master
write_success_feature
git add src/lib.rs tests/math_merge.rs
git commit -m "feat(fixtures): add success feature branch"

git checkout -B scenario/validation-main master
write_validation_main
git add src/lib.rs tests/math_merge.rs
git commit -m "feat(fixtures): add validation main branch"

git checkout -B scenario/validation-feature master
write_validation_feature
git add src/lib.rs tests/math_merge.rs
git commit -m "feat(fixtures): add validation feature branch"

git checkout -B scenario/retry-main master
write_retry_main
git add docs/merge_notes.md
git commit -m "feat(fixtures): add retry main branch"

git checkout -B scenario/retry-feature master
write_retry_feature
git add docs/merge_notes.md
git commit -m "feat(fixtures): add retry feature branch"

git checkout -B scenario/binary-main master
write_binary_main
git add assets/logo.bin
git commit -m "feat(fixtures): add binary main branch"

git checkout -B scenario/binary-feature master
write_binary_feature
git add assets/logo.bin
git commit -m "feat(fixtures): add binary feature branch"

git checkout master

if [ -n "$remote_url" ]; then
    if ! git remote get-url origin >/dev/null 2>&1; then
        git remote add origin "$remote_url"
    fi
    git push -u origin master \
        scenario/success-main scenario/success-feature \
        scenario/validation-main scenario/validation-feature \
        scenario/retry-main scenario/retry-feature \
        scenario/binary-main scenario/binary-feature
fi

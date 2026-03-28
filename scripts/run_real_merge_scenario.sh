#!/bin/sh
set -eu

if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
    echo "usage: $0 <repo-source> <git-raft-bin> <scenario> [output-root]" >&2
    exit 1
fi

repo_source=$1
git_raft_bin=$2
scenario=$3
output_root=${4-}

script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
timestamp=$(date +%Y%m%d-%H%M%S)

if [ -z "$output_root" ]; then
    output_root=$repo_root/docs/generated/real-merge
fi

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "missing required command: $1" >&2
        exit 1
    fi
}

require_command git
require_command cargo

if [ ! -x "$git_raft_bin" ]; then
    echo "git-raft binary is not executable: $git_raft_bin" >&2
    exit 1
fi

if [ -z "${GIT_RAFT_API_KEY:-}" ]; then
    echo "GIT_RAFT_API_KEY is required" >&2
    exit 1
fi

base_url=${GIT_RAFT_BASE_URL:-https://openrouter.ai/api/v1}
model=${GIT_RAFT_MODEL:-xiaomi/mimo-v2-flash}

case "$scenario" in
    success)
        main_branch=scenario/success-main
        feature_branch=scenario/success-feature
        expected_exit=0
        ;;
    validation)
        main_branch=scenario/validation-main
        feature_branch=scenario/validation-feature
        expected_exit=1
        ;;
    retry)
        main_branch=scenario/retry-main
        feature_branch=scenario/retry-feature
        expected_exit=0
        ;;
    binary)
        main_branch=scenario/binary-main
        feature_branch=scenario/binary-feature
        expected_exit=1
        ;;
    *)
        echo "unsupported scenario: $scenario" >&2
        exit 1
        ;;
esac

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT INT TERM

repo_dir=$workdir/repo
run_output=$output_root/$scenario/$timestamp
mkdir -p "$run_output"

git clone "$repo_source" "$repo_dir" >/dev/null
cd "$repo_dir"
git checkout "$main_branch" >/dev/null

mkdir -p .config/git-raft

write_config() {
    verification_body=$1
    cat <<EOF > .config/git-raft/config.toml
[provider]
base_url = "$base_url"
model = "$model"
api_key_env = "GIT_RAFT_API_KEY"

[merge]
repair_attempts = 1
$verification_body
EOF
}

write_success_support() {
    write_config '

[[merge.verification]]
program = "cargo"
args = ["fmt", "--check"]

[[merge.verification]]
program = "cargo"
args = ["test"]
'
}

write_validation_support() {
    cat <<'EOF' > .config/git-raft/always-fail.sh
#!/bin/sh
echo "forced validation failure" >&2
exit 1
EOF
    chmod +x .config/git-raft/always-fail.sh
    write_config '

[[merge.verification]]
program = "sh"
args = [".config/git-raft/always-fail.sh"]
'
}

write_retry_support() {
    marker_dir=${TMPDIR:-/tmp}
    marker_file=$marker_dir/git-raft-retry-$timestamp-$$.marker
    rm -f "$marker_file"
    cat <<EOF > .config/git-raft/fail-once.sh
#!/bin/sh
marker_file='$marker_file'
if [ ! -f "\$marker_file" ]; then
    : > "\$marker_file"
    echo "first validation failure" >&2
    exit 1
fi
exit 0
EOF
    chmod +x .config/git-raft/fail-once.sh
    write_config '

[[merge.verification]]
program = "sh"
args = [".config/git-raft/fail-once.sh"]
'
}

write_binary_support() {
    write_success_support
}

case "$scenario" in
    success) write_success_support ;;
    validation) write_validation_support ;;
    retry) write_retry_support ;;
    binary) write_binary_support ;;
esac

set +e
"$git_raft_bin" --json --yes merge "$feature_branch" >"$run_output/merge.stdout" 2>"$run_output/merge.stderr"
status=$?
set -e

if [ "$expected_exit" -eq 0 ] && [ "$status" -ne 0 ]; then
    echo "scenario $scenario expected success, got exit $status" >&2
    exit 1
fi

if [ "$expected_exit" -ne 0 ] && [ "$status" -eq 0 ]; then
    echo "scenario $scenario expected failure, got success" >&2
    exit 1
fi

latest_run=$(find .git/git-raft/runs -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1)
cp -R "$latest_run" "$run_output/run"
git status --short --branch > "$run_output/git-status.txt"

copy_optional_file() {
    src=$1
    dest=$2
    if [ -f "$src" ]; then
        cp "$src" "$dest"
    fi
}

copy_optional_file src/lib.rs "$run_output/src-lib.rs"
copy_optional_file tests/math_merge.rs "$run_output/tests-math_merge.rs"
copy_optional_file docs/merge_notes.md "$run_output/docs-merge_notes.md"
copy_optional_file assets/logo.bin "$run_output/assets-logo.bin"

assert_contains() {
    file=$1
    pattern=$2
    if ! grep -q "$pattern" "$file"; then
        echo "missing pattern '$pattern' in $file" >&2
        exit 1
    fi
}

assert_not_contains() {
    file=$1
    pattern=$2
    if grep -q "$pattern" "$file"; then
        echo "unexpected pattern '$pattern' in $file" >&2
        exit 1
    fi
}

case "$scenario" in
    success)
        cargo fmt --check >"$run_output/fmt.stdout" 2>"$run_output/fmt.stderr"
        cargo test >"$run_output/test.stdout" 2>"$run_output/test.stderr"
        assert_not_contains src/lib.rs "<<<<<<<"
        assert_not_contains tests/math_merge.rs "<<<<<<<"
        assert_contains "$run_output/run/events.ndjson" "conflict_detected"
        assert_contains "$run_output/run/events.ndjson" "ai_patch_ready"
        assert_contains "$run_output/run/events.ndjson" "ai_patch_applied"
        assert_contains "$run_output/run/validation.json" "\"validationPassed\":true"
        ;;
    validation)
        assert_contains src/lib.rs "<<<<<<<"
        assert_contains "$run_output/run/events.ndjson" "awaiting_confirmation"
        assert_contains "$run_output/run/patch.json" "resolved_content"
        assert_contains "$run_output/run/validation.json" "\"passed\":false"
        ;;
    retry)
        assert_not_contains docs/merge_notes.md "<<<<<<<"
        assert_contains "$run_output/run/events.ndjson" "ai_patch_applied"
        assert_contains "$run_output/run/validation.json" "\"attempt\":2"
        assert_contains "$run_output/run/validation.json" "\"validationPassed\":true"
        ;;
    binary)
        assert_contains "$run_output/run/validation.json" "decodable text"
        if [ -f "$run_output/run/ai-request.json" ]; then
            echo "binary scenario should not create ai-request.json" >&2
            exit 1
        fi
        ;;
esac

cat <<EOF > "$run_output/summary.md"
# Real Merge Scenario

- Scenario: $scenario
- Repository source: $repo_source
- Main branch: $main_branch
- Feature branch: $feature_branch
- Exit status: $status
- Run dir: $latest_run
- Output dir: $run_output
EOF

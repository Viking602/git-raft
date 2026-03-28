#!/usr/bin/env bash
set -euo pipefail

sample_repo="${1:-}"
git_raft_bin="${2:-}"

if [[ -z "$sample_repo" || -z "$git_raft_bin" ]]; then
  echo "usage: $0 <sample-repo> <git-raft-bin>" >&2
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "git is required" >&2
  exit 1
fi

if [[ ! -d "$sample_repo/.git" ]]; then
  echo "sample repo must be an existing git repository" >&2
  exit 1
fi

if [[ ! -x "$git_raft_bin" ]]; then
  echo "git-raft binary is not executable: $git_raft_bin" >&2
  exit 1
fi

if [[ -z "${GIT_RAFT_API_KEY:-}" ]]; then
  echo "GIT_RAFT_API_KEY is required" >&2
  exit 1
fi

base_url="${GIT_RAFT_BASE_URL:-https://openrouter.ai/api/v1}"
model="${GIT_RAFT_MODEL:-xiaomi/mimo-v2-flash}"
result_root="${RESULT_ROOT:-$sample_repo/.git/git-raft-real-merge}"
timestamp="$(date +%Y%m%d-%H%M%S)"
suite_dir="$result_root/$timestamp"
work_root="$suite_dir/work"
summary_file="$suite_dir/summary.md"
mkdir -p "$work_root"

write_config() {
  local repo="$1"
  local scenario="${2:-}"
  mkdir -p "$repo/.config/git-raft"
  local extra_verification=""
  if [[ "$scenario" == "scenario-c-validation-stop" ]]; then
    cat > "$repo/.config/git-raft/verify-validation-stop.sh" <<'EOF'
#!/bin/sh
set -eu
if grep -Fq 'pub fn render_sorted_view' src/lib.rs; then
  echo "public entrypoint drift: render_sorted_view should stay internal to manual review scenario" >&2
  exit 1
fi
exit 0
EOF
    chmod +x "$repo/.config/git-raft/verify-validation-stop.sh"
    extra_verification='

[[merge.verification]]
program = "sh"
args = [".config/git-raft/verify-validation-stop.sh"]'
  fi
  cat > "$repo/.config/git-raft/config.toml" <<EOF
[provider]
base_url = "$base_url"
model = "$model"
api_key_env = "GIT_RAFT_API_KEY"

[merge]
repair_attempts = 1

[[merge.verification]]
program = "cargo"
args = ["fmt", "--check"]

[[merge.verification]]
program = "cargo"
args = ["test"]
$extra_verification
EOF
}

latest_run_dir() {
  local repo="$1"
  ls -1dt "$repo"/.git/git-raft/runs/* 2>/dev/null | head -n 1
}

copy_if_exists() {
  local source="$1"
  local destination="$2"
  if [[ -e "$source" ]]; then
    cp -R "$source" "$destination"
  fi
}

contains_conflicts() {
  local file="$1"
  grep -q '<<<<<<<\|=======\|>>>>>>>' "$file"
}

append_summary_row() {
  local scenario="$1"
  local auto_applied="$2"
  local validated="$3"
  local kept_unique="$4"
  local manual_review="$5"
  local run_id="$6"
  local conclusion="$7"

  printf '| %s | %s | %s | %s | %s | %s | %s |\n' \
    "$scenario" "$auto_applied" "$validated" "$kept_unique" "$manual_review" "$run_id" "$conclusion" \
    >> "$summary_file"
}

extract_run_id() {
  local run_dir="$1"
  basename "$run_dir"
}

check_tokens() {
  local file="$1"
  shift
  local token
  for token in "$@"; do
    if ! grep -q "$token" "$file"; then
      return 1
    fi
  done
}

record_repo_state() {
  local repo="$1"
  local out_dir="$2"
  git -C "$repo" status --short --branch > "$out_dir/git-status.txt"
  if [[ -f "$repo/src/lib.rs" ]]; then
    cp "$repo/src/lib.rs" "$out_dir/src-lib.rs"
  fi
  if [[ -f "$repo/tests/math_merge.rs" ]]; then
    cp "$repo/tests/math_merge.rs" "$out_dir/tests-math_merge.rs"
  fi
}

ensure_local_branch() {
  local repo="$1"
  local branch="$2"
  if git -C "$repo" rev-parse --verify "$branch" >/dev/null 2>&1; then
    return 0
  fi
  git -C "$repo" branch "$branch" "origin/$branch" >/dev/null 2>&1
}

configure_git_identity() {
  local repo="$1"
  git -C "$repo" config user.name "${GIT_AUTHOR_NAME:-Git Raft Runner}"
  git -C "$repo" config user.email "${GIT_AUTHOR_EMAIL:-git-raft-runner@example.com}"
}

run_plain_merge_baseline() {
  local scenario_dir="$suite_dir/scenario-a-baseline"
  local repo="$work_root/scenario-a"
  mkdir -p "$scenario_dir"
  git clone "$sample_repo" "$repo" >/dev/null 2>&1
  configure_git_identity "$repo"
  git -C "$repo" checkout master >/dev/null 2>&1
  ensure_local_branch "$repo" "feature/format-report"

  set +e
  git -C "$repo" merge feature/format-report >"$scenario_dir/stdout.log" 2>"$scenario_dir/stderr.log"
  local exit_code=$?
  set -e

  record_repo_state "$repo" "$scenario_dir"
  local conclusion="baseline conflict reproduced"
  if [[ $exit_code -eq 0 ]] || ! contains_conflicts "$repo/src/lib.rs" || ! contains_conflicts "$repo/tests/math_merge.rs"; then
    conclusion="baseline merge did not stop on both expected conflicts"
  fi
  append_summary_row "A" "false" "n/a" "n/a" "true" "-" "$conclusion"
}

run_agent_scenario() {
  local label="$1"
  local branch="$2"
  local target="$3"
  shift 3
  local expected_tokens=("$@")

  local repo="$work_root/$label"
  local scenario_dir="$suite_dir/$label"
  mkdir -p "$scenario_dir"
  git clone "$sample_repo" "$repo" >/dev/null 2>&1
  configure_git_identity "$repo"
  ensure_local_branch "$repo" "$branch"
  ensure_local_branch "$repo" "$target"
  git -C "$repo" checkout "$branch" >/dev/null 2>&1
  write_config "$repo" "$label"

  set +e
  (
    cd "$repo"
    GIT_RAFT_API_KEY="$GIT_RAFT_API_KEY" \
    GIT_RAFT_BASE_URL="$base_url" \
    GIT_RAFT_MODEL="$model" \
    "$git_raft_bin" --json --yes merge "$target"
  ) >"$scenario_dir/stdout.ndjson" 2>"$scenario_dir/stderr.log"
  local exit_code=$?
  set -e

  local run_dir
  run_dir="$(latest_run_dir "$repo")"
  local run_id="-"
  if [[ -n "$run_dir" ]]; then
    run_id="$(extract_run_id "$run_dir")"
    cp -R "$run_dir" "$scenario_dir/run"
  fi

  record_repo_state "$repo" "$scenario_dir"

  local auto_applied=false
  local validated=false
  local manual_review=true
  local kept_unique=false
  local conclusion="manual review required"

  if [[ -f "$scenario_dir/run/events.ndjson" ]] && grep -q '"event_type":"ai_patch_applied"' "$scenario_dir/run/events.ndjson"; then
    auto_applied=true
    manual_review=false
  fi

  if [[ -f "$scenario_dir/run/validation.json" ]] && grep -Eq '"validationPassed":[[:space:]]*true' "$scenario_dir/run/validation.json"; then
    validated=true
  fi

  if [[ -f "$repo/src/lib.rs" ]] && check_tokens "$repo/src/lib.rs" "${expected_tokens[@]}"; then
    kept_unique=true
  fi

  case "$label" in
    scenario-b-success)
      if [[ $exit_code -eq 0 && "$auto_applied" == true && "$validated" == true && "$kept_unique" == true ]]; then
        conclusion="success scenario passed"
      else
        conclusion="success scenario did not meet expected auto-merge outcome"
      fi
      ;;
    scenario-c-validation-stop)
      if [[ $exit_code -ne 0 && "$manual_review" == true && -f "$scenario_dir/run/validation.json" ]] &&
        grep -Eq '"passed":[[:space:]]*false' "$scenario_dir/run/validation.json"; then
        conclusion="validation failure stopped auto-apply"
      else
        conclusion="validation-stop scenario did not show the expected failure path"
      fi
      ;;
    scenario-d-repair-retry)
      local attempts=0
      if [[ -f "$scenario_dir/run/validation.json" ]]; then
        attempts="$(grep -o '"attempt":[0-9]*' "$scenario_dir/run/validation.json" | wc -l | tr -d ' ')"
      fi
      if [[ "$attempts" -ge 2 ]]; then
        conclusion="repair scenario produced at least two attempts"
      else
        conclusion="repair scenario did not produce a retry"
      fi
      ;;
    scenario-e-binary-stop)
      if [[ $exit_code -ne 0 && -f "$scenario_dir/run/validation.json" ]] &&
        grep -q 'decodable text' "$scenario_dir/run/validation.json"; then
        conclusion="binary conflict stopped before AI apply"
      else
        conclusion="binary scenario did not stop at non-text validation"
      fi
      ;;
  esac

  append_summary_row \
    "${label#scenario-}" \
    "$auto_applied" \
    "$validated" \
    "$kept_unique" \
    "$manual_review" \
    "$run_id" \
    "$conclusion"
}

cat > "$summary_file" <<EOF
# Real Merge Summary

- Sample repo: \`$sample_repo\`
- Binary: \`$git_raft_bin\`
- Base URL: \`$base_url\`
- Model: \`$model\`
- Result dir: \`$suite_dir\`

| 场景 | 是否自动应用 | 是否通过验证 | 是否保留双方独有内容 | 是否需要人工介入 | run-id | 结论 |
| --- | --- | --- | --- | --- | --- | --- |
EOF

run_plain_merge_baseline
run_agent_scenario "scenario-b-success" "master" "feature/format-report" "target-token" "feature-token"
run_agent_scenario "scenario-c-validation-stop" "scenario/validation-target" "scenario/validation-feature" "stable-order-token" "sorted-order-token"
run_agent_scenario "scenario-d-repair-retry" "scenario/repair-target" "scenario/repair-feature" "target-note-3" "feature-note-3"
run_agent_scenario "scenario-e-binary-stop" "scenario/binary-target" "scenario/binary-feature"

echo "results written to $suite_dir"

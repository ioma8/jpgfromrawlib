#!/bin/bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
input_dir="${INPUT_DIR:-$repo_root/test_images}"
artifact_root="${ARTIFACT_ROOT:-$repo_root/perf_artifacts}"
baseline_dir="$artifact_root/baseline"
baseline_output_dir="$baseline_dir/outputs"
baseline_manifest="$baseline_dir/sha256.txt"
baseline_largest_manifest="$baseline_dir/largest_sha256.txt"
latest_output_dir="$artifact_root/latest/outputs"
latest_manifest="$artifact_root/latest/sha256.txt"
latest_largest_manifest="$artifact_root/latest/largest_sha256.txt"
results_dir="$artifact_root/results"
runs="${RUNS:-10}"
warmups="${WARMUPS:-3}"
timestamp="$(date +"%Y%m%d-%H%M%S")"
hyperfine_json="$results_dir/hyperfine-$timestamp.json"

render_manifest() {
    local source_dir="$1"
    local manifest_path="$2"

    mkdir -p "$(dirname "$manifest_path")"
    : > "$manifest_path"

    while IFS= read -r file; do
        local rel_path="${file#$source_dir/}"
        local digest
        digest="$(shasum -a 256 "$file" | awk '{print $1}')"
        printf '%s  %s\n' "$digest" "$rel_path" >> "$manifest_path"
    done < <(find "$source_dir" -type f -name '*.jpg' ! -name '*_largest.jpg' | LC_ALL=C sort)
}

render_largest_manifest() {
    local source_dir="$1"
    local manifest_path="$2"

    mkdir -p "$(dirname "$manifest_path")"
    : > "$manifest_path"
    while IFS= read -r file; do
        local rel_path="${file#$source_dir/}"
        local digest
        digest="$(shasum -a 256 "$file" | awk '{print $1}')"
        printf '%s  %s\n' "$digest" "$rel_path" >> "$manifest_path"
    done < <(find "$source_dir" -type f -name '*_largest.jpg' | LC_ALL=C sort)
}

run_probe() {
    local output_dir="$1"
    validate_artifact_path "$output_dir"
    rm -rf -- "$output_dir"
    mkdir -p "$output_dir"
    "$repo_root/target/release/perf_probe" "$input_dir" "$output_dir"
}

validate_artifact_path() {
    case "$1" in
        "$artifact_root"/*) ;;
        *) echo "refusing to delete outside ARTIFACT_ROOT: $1" >&2; exit 1 ;;
    esac
}

cd "$repo_root"

command -v hyperfine >/dev/null

cargo build --release --features cli --bin perf_probe

mkdir -p "$results_dir"
validate_artifact_path "$latest_output_dir"

if [[ ! -f "$baseline_manifest" || "${UPDATE_BASELINE:-0}" == "1" ]]; then
    run_probe "$baseline_output_dir"
    render_manifest "$baseline_output_dir" "$baseline_manifest"
    render_largest_manifest "$baseline_output_dir" "$baseline_largest_manifest"
elif [[ ! -f "$baseline_largest_manifest" ]]; then
    echo "missing Largest baseline; run UPDATE_BASELINE=1 ./perf_probe.sh" >&2
    exit 1
fi

hyperfine \
    -N \
    --warmup "$warmups" \
    --runs "$runs" \
    --export-json "$hyperfine_json" \
    --prepare "rm -rf -- '$latest_output_dir'; mkdir -p '$latest_output_dir'" \
    "$repo_root/target/release/perf_probe '$input_dir' '$latest_output_dir'"

run_probe "$latest_output_dir"
render_manifest "$latest_output_dir" "$latest_manifest"
render_largest_manifest "$latest_output_dir" "$latest_largest_manifest"
diff -u "$baseline_manifest" "$latest_manifest"
diff -u "$baseline_largest_manifest" "$latest_largest_manifest"

printf 'Baseline manifest: %s\n' "$baseline_manifest"
printf 'Latest manifest: %s\n' "$latest_manifest"
printf 'Largest baseline manifest: %s\n' "$baseline_largest_manifest"
printf 'Largest latest manifest: %s\n' "$latest_largest_manifest"
printf 'Hyperfine results: %s\n' "$hyperfine_json"

#!/usr/bin/env bash
# Attempt each recovered target even when other targets cannot compile.
set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
logdir="$PWD/target/regression-logs"
mkdir -p "$logdir" || exit 1
summary="$logdir/results.tsv"
printf 'package\tkind\ttarget\tstatus\tlog\n' > "$summary"
passed=0
failed=0

while IFS=$'\t' read -r package kind target source; do
    [[ -z "$package" || "$package" == \#* ]] && continue
    log="$logdir/$package-$target.log"
    printf 'RUN  %s/%s\n' "$package" "$target"
    if [[ ! -f "$source" ]]; then
        printf 'Missing regression source: %s\n' "$source" > "$log"
        status=1
    else
        case "$kind" in
            test|example) command=(cargo test --locked -p "$package" --all-features "--$kind" "$target") ;;
            lib) command=(cargo test --locked -p "$package" --lib) ;;
            manifest-test) command=(cargo test --locked --manifest-path "$source") ;;
            manifest-target-test) command=(cargo test --locked --manifest-path "$source" --test "$target") ;;
            scenario) command=(python3 "$source") ;;
            manifest-run) command=(cargo run --locked --manifest-path "$source") ;;
            *) printf 'Unknown regression kind: %s\n' "$kind" >&2; exit 2 ;;
        esac
        "${command[@]}" > "$log" 2>&1
        status=$?
    fi
    if (( status == 0 )); then
        passed=$((passed + 1))
        printf 'PASS %s/%s\n' "$package" "$target"
    else
        failed=$((failed + 1))
        printf 'FAIL %s/%s (exit %s): %s\n' "$package" "$target" "$status" "$log"
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$package" "$kind" "$target" "$status" "$log" >> "$summary"
done < tools/regression-targets.tsv

printf '%s passed, %s failed; results: %s\n' "$passed" "$failed" "$summary"
(( failed == 0 && passed > 0 ))

#!/usr/bin/env bash
# Mac (Apple silicon, Metal) gate: the native MCCL bundle, the experiments/mac
# workspace build and tests, and H03/H09 exactness of the Mac realizations
# against the authored default route. There is no Mac CI; run before pushing
# Mac work. With a dbgen retail_h09 directory, also checks Metal H09 there.
#
# Usage: bash tools/mac_gate.sh [RETAIL_H09_DATA_DIR]
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
data=${1:+$(cd "$1" && pwd)}
[[ $(uname -s) == Darwin ]] || { echo "mac-gate: requires macOS" >&2; exit 2; }

cd "$root/experiments/mac"
step() {
	local name=$1
	shift
	local start=$SECONDS
	if "$@"; then
		echo "mac-gate PASS $name $((SECONDS - start))s"
	else
		echo "mac-gate FAIL $name $((SECONDS - start))s"
		exit 1
	fi
}

[[ -f mccl/build/native/CMakeCache.txt ]] ||
	cmake -S mccl -B mccl/build/native -DCMAKE_BUILD_TYPE=Release >/dev/null
step mccl-native cmake --build mccl/build/native --parallel 8
step build cargo build --locked --release --workspace --all-targets
step test cargo test --locked --release --workspace
step h03-h09-synthetic cargo run --locked --release -q -p mac-query-expansion -- 65539
if [[ -n $data ]]; then
	step metal-h09 cargo run --locked --release -q -p mac-query-expansion --bin metal_h09 -- \
		--data "$data" --runs 3
fi

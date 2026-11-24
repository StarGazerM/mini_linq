#!/usr/bin/env bash
# Main's GPU batch gate: GPU crate suites plus the DOOP GPU executables'
# build and Cassandra exactness. Run once per merge batch that can affect
# GPU code (see tools/README.md). Holds /tmp/mini_linq-gpu.lock for the run.
#
# Usage: bash tools/gpu_gate.sh [DOOP_DATA_DIR]
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
data=${1:-/data/doop-bench/doop-data}
hpc=/opt/nvidia/hpc_sdk/Linux_x86_64/25.9
export CUDA_HOME=$hpc/cuda/13.0
export BINDGEN_EXTRA_CLANG_ARGS=-I$hpc/math_libs/13.0/targets/x86_64-linux/include
export LIBRARY_PATH=$hpc/math_libs/13.0/targets/x86_64-linux/lib
export LD_LIBRARY_PATH=$LIBRARY_PATH
export CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8}

cd "$root/experiments/gpu"
step() {
	local name=$1
	shift
	local start=$SECONDS
	if "$@"; then
		echo "gpu-gate PASS $name $((SECONDS - start))s"
	else
		echo "gpu-gate FAIL $name $((SECONDS - start))s"
		exit 1
	fi
}

exec 9>/tmp/mini_linq-gpu.lock
flock 9
step crates nice -n 10 cargo +1.99.1 oxide test --arch sm_89 --materialize-cubin -- --release \
	-p gpu-datalog-runtime -p cq-gpu-datalog-core -p cq-gpu-datalog \
	-p cq-gpu-sequence-core -p cq-gpu-sequence -p cq-gpu-tpch -p cq-gpu-runtime -p gpu-device-columns
step radix-counters nice -n 10 cargo +1.99.1 oxide test --arch sm_89 --materialize-cubin -- --release \
	-p gpu-datalog-runtime --features gpu-datalog-runtime/radix-counters
# Exactness-only crate tests do not enforce the benchmark's recorded ceiling.
step tpch-h05-sf10-ceiling nice -n 10 cargo +1.99.1 oxide run --arch sm_89 --materialize-cubin \
	--bin cq-gpu-tpch -- "$root/target/gpu-gate-h05-sf10.json" \
	--query h05 --scale sf10 --mode advised
step doop-build nice -n 10 cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release \
	--bin doop-gpu --bin srd-doop-gpu
# doop-gpu compares all 26 complete output tuple sets against doop-mini.
step doop-cassandra-exact nice -n 10 cargo +1.99.1 oxide run --arch sm_89 --bin doop-gpu -- "$data/cassandra"

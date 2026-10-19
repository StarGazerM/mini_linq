#!/usr/bin/env bash
set -euo pipefail
if (( $# < 3 )) || [[ $2 != -- ]]; then
    echo "usage: $0 OUTPUT_PREFIX -- PROGRAM [ARGS...]" >&2
    exit 2
fi
out=$1
shift 2
here=$(dirname "$(realpath "$0")")
nsys=${NSYS:-/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/nsys}
mkdir -p "$(dirname "$out")"
# Do not run competing workloads on the shared GPU.
while [[ -n $(nvidia-smi --query-compute-apps=pid --format=csv,noheader) ]]; do
    sleep 2
done
capture=()
if [[ ${NSYS_CAPTURE:-} == cudaProfilerApi ]]; then
    capture=(--capture-range=cudaProfilerApi --capture-range-end=stop)
fi
"$nsys" profile -t cuda,nvtx,osrt --cuda-memory-usage=true --sample=none \
    --cpuctxsw=none --force-overwrite=true "${capture[@]}" -o "$out" "$@" 2>&1 | tee "$out.log"
"$nsys" export --type sqlite --force-overwrite=true -o "$out.sqlite" "$out.nsys-rep"
python3 "$here/analyze.py" "$out.sqlite" --json "$out.analysis.json" --markdown "$out.summary.md"

#!/usr/bin/env bash
# Usage: make_data_manifest.sh OUT [DATA_ROOT=/data]
set -euo pipefail
exec python3 "$(dirname "$(realpath "$0")")/data_manifest.py" "$@"

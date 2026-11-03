#!/usr/bin/env bash
# Usage: seed_volume.sh DATASET_REPO IMMUTABLE_REVISION TRUSTED_SHA256SUMS [DATA_ROOT]
set -euo pipefail
exec python3 "$(dirname "$(realpath "$0")")/seed_volume.py" "$@"

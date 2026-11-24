#!/usr/bin/env bash
# Only this entrypoint provisions a billable resource. All builds happen earlier.
set -euo pipefail
exec python3 "$(dirname "$(realpath "$0")")/session.py" "$@"

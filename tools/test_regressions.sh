#!/usr/bin/env bash
# Preserve the shell entry point; the scheduler bounds independent Cargo groups.
# Example: bash tools/test_regressions.sh --jobs 2 package/target
set -uo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/test_regressions.py" "$@"

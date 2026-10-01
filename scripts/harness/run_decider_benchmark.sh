#!/bin/sh
set -eu
: "${DECIDER_PYTHON:?Set DECIDER_PYTHON to the isolated Python 3.12 environment containing decider-ai 1.8.1}"
exec "$DECIDER_PYTHON" "$(dirname "$0")/decider_benchmark.py" "$@"

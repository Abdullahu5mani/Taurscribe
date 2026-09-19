#!/usr/bin/env bash
# ==============================================================================
# run_custom_vocabulary_corpus_eval.sh
#
# Runs the empirical accuracy evaluation benchmark on the local LibriSpeech
# corpus, demonstrating accuracy gains on difficult proper nouns and technical
# words when Custom Vocabulary & Decoder Jargon Injection is enabled.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

echo "======================================================================"
echo " TAURSCRIBE: CUSTOM VOCABULARY & CONTEXT JARGON EVALUATION"
echo "======================================================================"
echo "Repo root:   ${REPO_ROOT}"
echo "Date:        $(date)"
echo ""

cd "${REPO_ROOT}/src-tauri"

# Run unit tests for context module first
echo ">>> Step 1: Running unit tests for context prompt & casing pipeline..."
cargo test --lib context::context_tests

# Run corpus empirical benchmark
echo ""
echo ">>> Step 2: Running empirical corpus benchmark with Whisper..."
cargo run --release --bin custom_vocab_eval

echo ""
echo ">>> Evaluation complete! All tests passed."

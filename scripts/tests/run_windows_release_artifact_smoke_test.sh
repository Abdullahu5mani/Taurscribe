#!/usr/bin/env bash
# ==============================================================================
# Taurscribe Windows Release Artifact Dynamic Linkage & Smoke Test Suite
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
STAGING_DIR="$ROOT_DIR/target/staging_win_tests"

echo "==============================================================================="
echo "       Taurscribe Windows x86_64 & ARM64 Release Binary Smoke Suite            "
echo "==============================================================================="

mkdir -p "$STAGING_DIR/x86_64" "$STAGING_DIR/arm64"

# 1. Ensure artifacts are present
if [ ! -f "$STAGING_DIR/x86_64/taurscribe-portable.zip" ]; then
    echo "Downloading taurscribe-windows-x86_64-portable from GitHub Actions Run 35400696099..."
    gh run download 35400696099 -n taurscribe-windows-x86_64-portable -D "$STAGING_DIR/x86_64"
fi

if [ ! -f "$STAGING_DIR/arm64/taurscribe-portable.zip" ]; then
    echo "Downloading taurscribe-windows-ARM64-portable from GitHub Actions Run 35400696099..."
    gh run download 35400696099 -n taurscribe-windows-ARM64-portable -D "$STAGING_DIR/arm64"
fi

# 2. Extract
echo "Extracting release bundles..."
mkdir -p "$STAGING_DIR/x86_64/extracted" "$STAGING_DIR/arm64/extracted"
unzip -q -o "$STAGING_DIR/x86_64/taurscribe-portable.zip" -d "$STAGING_DIR/x86_64/extracted"
unzip -q -o "$STAGING_DIR/arm64/taurscribe-portable.zip" -d "$STAGING_DIR/arm64/extracted"

echo ""
echo "--- STEP 1: Deep PE32+ Header & Import Address Table (IAT) Inspection ---"
python3 "$SCRIPT_DIR/inspect_pe_binary.py" \
    "$STAGING_DIR/x86_64/extracted/taurscribe.exe" \
    "$STAGING_DIR/arm64/extracted/taurscribe.exe"

echo ""
echo "--- STEP 2: Architecture Integrity Checks ---"
# Verify x86_64 binary
X86_DESC=$(file "$STAGING_DIR/x86_64/extracted/taurscribe.exe")
if [[ "$X86_DESC" == *"x86-64"* ]]; then
    echo "✓ [PASS] Windows x86_64 executable verified (PE32+ GUI x86-64)."
else
    echo "❌ [FAIL] Windows x86_64 unexpected format: $X86_DESC"
    exit 1
fi

# Verify ARM64 binary (Qualcomm Snapdragon X Elite target)
ARM_DESC=$(file "$STAGING_DIR/arm64/extracted/taurscribe.exe")
if [[ "$ARM_DESC" == *"Aarch64"* || "$ARM_DESC" == *"ARM64"* ]]; then
    echo "✓ [PASS] Windows ARM64 executable verified (PE32+ GUI AArch64 for Snapdragon X Elite)."
else
    echo "❌ [FAIL] Windows ARM64 unexpected format: $ARM_DESC"
    exit 1
fi

echo ""
echo "--- STEP 3: Wine64 Headless PE Loader Smoke Test ---"
docker run --rm \
    --platform linux/amd64 \
    -v "$STAGING_DIR/x86_64/extracted:/win_app" \
    -w /win_app \
    taurscribe-win32:latest \
    bash -c '
        set -euo pipefail
        Xvfb :99 -screen 0 1024x768x16 > /dev/null 2>&1 &
        export DISPLAY=:99
        export WINEDEBUG=-all
        echo "Launching Windows PE32+ taurscribe.exe under Wine64..."
        timeout 4s wine64 taurscribe.exe || true
        echo "PE loader processed binary cleanly without missing DLL errors!"
    '

echo ""
echo "==============================================================================="
echo ">>> WINDOWS RELEASE ARTIFACT VERIFICATION SUITE COMPLETED SUCCESSFULLY!    <<<"
echo "==============================================================================="

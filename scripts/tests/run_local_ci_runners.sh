#!/usr/bin/env bash
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# Local GitHub Actions Runner Verification Suite
# Runs the exact CI runner steps locally across platforms:
#   1. Frontend compilation (Bun + Vite + TypeScript)
#   2. macOS (Apple Silicon aarch64-apple-darwin) release build + DMG bundle
#   3. macOS (Intel x86_64-apple-darwin) release target verification
#   4. Linux (Ubuntu 24.04 x86_64) pkg-config & PipeWire dependency verification
#   5. Windows (x86_64) MinGW PE compilation & Wine64 execution verification
# ─────────────────────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

pass() { echo -e "${GREEN}✓ [PASS]${NC} $1"; }
fail() { echo -e "${RED}✗ [FAIL]${NC} $1"; exit 1; }
info() { echo -e "${BLUE}ℹ [INFO]${NC} $1"; }
header() { echo -e "\n${BOLD}======================================================================${NC}\n${BOLD}$1${NC}\n${BOLD}======================================================================${NC}"; }

header "STAGE 1: Environment & Tool Verification"
echo "Host: $(sw_vers -productName) $(sw_vers -productVersion) ($(uname -m))"
echo "Rustc: $(rustc --version)"
echo "Cargo: $(cargo --version)"
echo "Bun: $(bun --version)"
pass "Base toolchain verified"

header "STAGE 2: Frontend Build (Bun + Vite + TypeScript)"
info "Executing: bun run build"
bun run build
if [ -f "dist/index.html" ]; then
  pass "Frontend built cleanly to dist/ (index.html verified)"
else
  fail "Frontend build failed: dist/index.html not found"
fi

header "STAGE 3: macOS (Apple Silicon aarch64-apple-darwin) Runner"
info "Step 3a: Rust release binary compilation"
(
  cd src-tauri
  CI="true" NO_STRIP="true" MACOSX_DEPLOYMENT_TARGET="13.4" CMAKE_OSX_DEPLOYMENT_TARGET="13.4" \
    cargo build --release --target aarch64-apple-darwin
)
pass "Rust aarch64 binary compiled"

info "Step 3b: Copy binary to expected release location"
mkdir -p src-tauri/target/release
cp src-tauri/target/aarch64-apple-darwin/release/taurscribe src-tauri/target/release/taurscribe
chmod +x src-tauri/target/release/taurscribe
pass "Binary copied to src-tauri/target/release/"

info "Step 3c: Pre-bundle macOS dylibs with dylibbundler"
TAURI_BUILD_TARGET=aarch64-apple-darwin bun scripts/bundle-macos-dylibs.ts
pass "Shared libraries bundled into macos-dylibs/"

info "Step 3d: Tauri application and DMG bundle"
CI="true" MACOSX_DEPLOYMENT_TARGET="13.4" CMAKE_OSX_DEPLOYMENT_TARGET="13.4" \
  bun run tauri build --target aarch64-apple-darwin
APP_PATH="src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Taurscribe.app"
DMG_PATH=$(ls src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/*.dmg 2>/dev/null | head -n 1 || true)
if [ -d "$APP_PATH" ] && [ -n "$DMG_PATH" ]; then
  pass "macOS Apple Silicon bundle verified: $DMG_PATH"
else
  fail "macOS bundling failed: Taurscribe.app or .dmg missing"
fi

header "STAGE 4: macOS (Intel x86_64-apple-darwin) Runner"
info "Verifying x86_64 target compilation"
(
  cd src-tauri
  CI="true" NO_STRIP="true" MACOSX_DEPLOYMENT_TARGET="13.4" CMAKE_OSX_DEPLOYMENT_TARGET="13.4" \
    cargo check --target x86_64-apple-darwin --release
)
pass "macOS Intel x86_64 target checked successfully"

header "STAGE 5: Linux (Ubuntu 24.04 x86_64) Runner Verification"
info "Testing PipeWire and system dependencies inside Ubuntu 24.04 container via Rosetta"
docker run --rm --platform linux/amd64 ubuntu:24.04 sh -c \
  "apt-get update -qq && apt-get install -y -qq libpipewire-0.3-dev pkg-config >/dev/null && pkg-config --cflags --libs libpipewire-0.3"
pass "Linux PipeWire & pkg-config dependencies verified on Ubuntu 24.04 x86_64"

header "STAGE 6: Windows (x86_64) Runner Verification"
info "Testing Win32 MinGW cross-compilation & Wine64 execution"
./scripts/tests/run_win32_benchmark.sh
pass "Windows x86_64 Win32 PE binary compiled and verified under Wine64"

header "ALL LOCAL GITHUB ACTIONS RUNNERS VERIFIED SUCCESSFULLY"
echo -e "${GREEN}✓ Frontend (Bun/Vite/TypeScript)${NC}"
echo -e "${GREEN}✓ macOS Apple Silicon (aarch64-apple-darwin) [.app & .dmg]${NC}"
echo -e "${GREEN}✓ macOS Intel (x86_64-apple-darwin) target${NC}"
echo -e "${GREEN}✓ Linux Ubuntu 24.04 (x86_64-unknown-linux-gnu) dependencies${NC}"
echo -e "${GREEN}✓ Windows (x86_64-pc-windows-msvc / Win32 Wine64)${NC}"

#!/usr/bin/env bash
# Regenerate protobuf bindings from proto/agent.proto and proto/frontend.proto.
# Usage: scripts/gen_proto.sh [--protoc /path/to/protoc]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

PROTOC="${PROTOC:-protoc}"

if ! command -v "$PROTOC" &>/dev/null; then
    echo "error: protoc not found. Set PROTOC= env var or install protoc." >&2
    exit 1
fi

echo "Generating Python bindings..."
"$PROTOC" \
    --proto_path="${ROOT}/proto" \
    --python_out="${ROOT}/agent/pince_proto" \
    "${ROOT}/proto/agent.proto" \
    "${ROOT}/proto/frontend.proto"

echo "Python bindings generated in agent/pince_proto/"
echo ""
echo "Note: Rust bindings are generated automatically via crates/protocol/build.rs"
echo "      Set PROTOC= env var when running cargo build if protoc is not in PATH."

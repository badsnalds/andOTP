#!/usr/bin/env bash
# Checks the passkey byte formats (WebAuthn.java) against an independent WebAuthn
# implementation: a registration and a sign-in made with a software P-256 key must verify
# with py_webauthn exactly as a website's server would verify them.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# WebAuthn.java only needs @RequiresApi from AndroidX; a stub is enough to compile it.
mkdir -p "$work/src/androidx/annotation"
echo 'package androidx.annotation; public @interface RequiresApi { int value(); }' \
    > "$work/src/androidx/annotation/RequiresApi.java"
javac -d "$work/classes" \
    "$work/src/androidx/annotation/RequiresApi.java" \
    "$here/../app/src/main/java/org/shadowice/andotp/passkey/WebAuthn.java" \
    "$here/webauthn/Harness.java"
java -cp "$work/classes" Harness > "$work/responses.txt"

python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet webauthn==3.0.1
"$work/venv/bin/python" "$here/webauthn/verify.py" "$work/responses.txt"

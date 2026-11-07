#!/usr/bin/env bash
# Checks install.sh's Caddyfile handling without installing anything (no root, no network):
# a Caddyfile with other sites is never replaced, only gains one import line, and stays the same
# however often install.sh runs. Run by the packaging lint workflow.
#
# Usage: packaging/server/install-test.sh
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILED=0

main() { local caddyfile="$1"; bash "$HERE/install.sh" --caddy-main "$caddyfile" ops@example.com; }
fail() { printf 'FAIL: %s\n' "$*" >&2; FAILED=1; }

# Missing: ours.
main "$WORK/none" >"$WORK/out"
grep -qx 'import figcraft-server.caddy' "$WORK/out" || fail "a new Caddyfile imports our site"
grep -q 'email ops@example.com' "$WORK/out" || fail "a new Caddyfile has the certificate email"

# The example site the package ships: replaced by ours.
printf '# The Caddyfile is an easy way to configure your Caddy web server.\n:80 {\n\troot * /usr/share/caddy\n\tfile_server\n}\n' >"$WORK/default"
main "$WORK/default" >"$WORK/out"
grep -q 'usr/share/caddy' "$WORK/out" && fail "the packaged example site is replaced"

# Someone else's sites: kept, one import added, and the next run changes nothing.
printf 'shop.example.com {\n\treverse_proxy 127.0.0.1:8080\n}\n' >"$WORK/theirs"
cp "$WORK/theirs" "$WORK/run1"
main "$WORK/run1" >"$WORK/run2"
grep -q 'shop.example.com' "$WORK/run2" || fail "the operator's site is kept"
[[ "$(grep -c 'import figcraft-server.caddy' "$WORK/run2")" = 1 ]] || fail "one import line is added"
main "$WORK/run2" >"$WORK/run3"
cmp -s "$WORK/run2" "$WORK/run3" || fail "running again leaves a Caddyfile with other sites as it is"
main "$WORK/run3" >"$WORK/run4"
cmp -s "$WORK/run3" "$WORK/run4" || fail "and again"

# Ours, run again: unchanged (it imports our site).
main "$WORK/none" >"$WORK/ours1"
main "$WORK/ours1" >"$WORK/ours2"
cmp -s "$WORK/ours1" "$WORK/ours2" || fail "our own Caddyfile stays the same"

if [[ "$FAILED" = 0 ]]; then echo "install.sh Caddyfile handling: ok"; fi
exit "$FAILED"

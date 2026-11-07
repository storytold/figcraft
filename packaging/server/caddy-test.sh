#!/bin/sh
# Exercise the optional browser/server example with synthetic files over loopback HTTP.
# Requires caddy and nc; no backend, root access, public DNS or certificate is needed.
# Usage: packaging/server/caddy-test.sh
# Overrides: CADDY=/path/to/caddy CADDYFILE=/path/to/Caddyfile CADDY_TEST_PORT=18080
set -eu

HERE="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
CADDY="${CADDY:-caddy}"
CADDYFILE="${CADDYFILE:-$HERE/Caddyfile}"
PORT="${CADDY_TEST_PORT:-18080}"
case "$PORT" in ''|*[!0-9]*) echo 'CADDY_TEST_PORT must be a port number' >&2; exit 1 ;; esac
[ "$PORT" -gt 1023 ] && [ "$PORT" -lt 65536 ] || exit 1
command -v "$CADDY" >/dev/null
command -v nc >/dev/null

WORK="$(mktemp -d)"
PID=''
cleanup() {
    status=$?
    trap - 0
    if [ -n "$PID" ]; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
    rm -rf "$WORK"
    exit "$status"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM
fail() { printf 'FAIL: %s\n' "$*" >&2; cat "$WORK/caddy.log" >&2; exit 1; }

mkdir "$WORK/web"
printf 'synthetic editor\n' >"$WORK/web/index.html"
printf 'synthetic JavaScript asset\n' >"$WORK/web/figcraft-web-present.js"
printf 'synthetic WebAssembly asset\n' >"$WORK/web/figcraft-web-present_bg.wasm"

# Extract the real commented block, changing only the hostname and filesystem root.
# The admin endpoint is disabled; state directories stay inside the temporary fixture.
printf '{\n\tadmin off\n}\n' >"$WORK/Caddyfile"
awk -v port="$PORT" -v root="$WORK/web" '
    $0 == "# figcraft.example.com {" { active = 1; found++; print "http://127.0.0.1:" port " {"; next }
    active {
        if ($0 !~ /^#/) exit 1
        if ($0 == "# }") { print "}"; active = 0; next }
        sub(/^# ?/, "")
        if ($1 == "root" && $2 == "*" && $3 == "/srv/figcraft-web") $0 = "\t\troot * \"" root "\""
        print
    }
    END { if (found != 1 || active) exit 1 }
' "$CADDYFILE" >>"$WORK/Caddyfile"

XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data" \
    "$CADDY" validate --config "$WORK/Caddyfile" --adapter caddyfile >"$WORK/caddy.log" 2>&1 ||
    fail 'extracted example did not validate'
XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data" \
    "$CADDY" run --config "$WORK/Caddyfile" --adapter caddyfile >>"$WORK/caddy.log" 2>&1 &
PID=$!

request() {
    printf '%s %s HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n' "$1" "$2" |
        nc -w 2 127.0.0.1 "$PORT" >"$WORK/response" || return 1
    tr -d '\r' <"$WORK/response" >"$WORK/clean"
    awk 'BEGIN { headers = 1 } headers && /^$/ { headers = 0; next } !headers { print }' \
        "$WORK/clean" >"$WORK/body"
}
status() { awk 'NR == 1 { print $2 }' "$WORK/clean"; }
cache() {
    awk '/^$/ { exit } tolower($0) ~ /^cache-control:/ { sub(/^[^:]*:[ \t]*/, ""); print }' "$WORK/clean"
}

attempt=0
until request GET /app/ 2>/dev/null && [ "$(status)" = 200 ]; do
    kill -0 "$PID" 2>/dev/null || fail 'Caddy did not start (port may be in use)'
    attempt=$((attempt + 1))
    [ "$attempt" -lt 30 ] || fail 'Caddy did not become ready'
    sleep 0.1
done
kill -0 "$PID" 2>/dev/null || fail 'Caddy exited during startup'

IMMUTABLE='public, max-age=31536000, immutable'
checks=0
check() {
    method=$1
    name=$2
    expected=$3
    request "$method" "/app/$name" || fail "$method $name request failed"
    [ "$(status)" = "$expected" ] || fail "$method $name: expected HTTP $expected, got $(status)"
    if [ "$expected" = 200 ]; then
        [ "$(cache)" = "$IMMUTABLE" ] || fail "$method $name: successful asset lost its cache policy"
        if [ "$method" = GET ]; then
            cmp -s "$WORK/body" "$WORK/web/$name" || fail "$method $name: wrong file contents"
        fi
    else
        [ -z "$(cache)" ] || fail "$method $name: a missing asset received Cache-Control: $(cache)"
    fi
    if [ "$method" = HEAD ]; then
        [ ! -s "$WORK/body" ] || fail "$method $name: HEAD returned a body"
    fi
    checks=$((checks + 1))
    printf 'PASS: %s %s -> %s\n' "$method" "$name" "$expected"
}

for method in GET HEAD; do
    check "$method" figcraft-web-present.js 200
    check "$method" figcraft-web-present_bg.wasm 200
    check "$method" figcraft-web-later.js 404
    check "$method" figcraft-web-later_bg.wasm 404
done

# The same URLs must work immediately when deployment supplies the previously missing files.
printf 'new JavaScript asset\n' >"$WORK/web/figcraft-web-later.js"
printf 'new WebAssembly asset\n' >"$WORK/web/figcraft-web-later_bg.wasm"
for method in GET HEAD; do
    check "$method" figcraft-web-later.js 200
    check "$method" figcraft-web-later_bg.wasm 200
done

printf 'Caddy browser asset caching: %s checks passed (%s)\n' "$checks" "$("$CADDY" version)"

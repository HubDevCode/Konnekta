#!/usr/bin/env bash
# Interoperability check against a real KDE Connect daemon, fully isolated
# (temporary HOME, own D-Bus session, no display). Requires kdeconnectd,
# kdeconnect-cli and dbus-run-session.
#   cargo build -p konnekta-cli && scripts/interop-kdeconnect.sh
set -uo pipefail
# Run everything inside one private D-Bus session.
if [ -z "${KN_INTEROP_INNER:-}" ]; then
  KN_INTEROP_INNER=1 exec dbus-run-session -- "$0" "$@"
fi
ROOT=$(cd "$(dirname "$0")/.." && pwd)
B=${KONNEKTA_BIN:-$ROOT/target/debug/konnekta}
T=$(mktemp -d); R=$(mktemp -d -p "${XDG_RUNTIME_DIR:-/tmp}" kn.XXXX)
trap 'kill $(jobs -p) 2>/dev/null; rm -rf "$T" "$R"' EXIT
mkdir -p "$T/kde/.config" "$T/dl"
kde_env=(HOME="$T/kde" XDG_CONFIG_HOME="$T/kde/.config" XDG_DATA_HOME="$T/kde/.local/share" QT_QPA_PLATFORM=offscreen)
env "${kde_env[@]}" kdeconnectd >/dev/null 2>&1 &
sleep 4
XDG_RUNTIME_DIR=$R "$B" --data-dir "$T/k" daemon --name KonnektaTest --kde-connect >"$T/k.log" 2>&1 &
K() { XDG_RUNTIME_DIR=$R "$B" --data-dir "$T/k" "$@"; }
KC() { env "${kde_env[@]}" kdeconnect-cli "$@"; }
sleep 3
K connect 127.0.0.1 >/dev/null; sleep 3
fail=0
KID=$(KC --my-id)
MYID=$(K info | grep deviceId | cut -d'"' -f4)
KC -d "$MYID" --pair >/dev/null 2>&1; sleep 2
KEY=$(grep -o "— key [0-9A-F ]*" "$T/k.log" | head -1 | cut -d" " -f3-)
[ "$(echo "$KEY" | tr -d ' ' | wc -c)" = 9 ] && echo "8-character key for KDE Connect: OK" || { echo "key: FAIL ($KEY)"; fail=1; }
K accept "$KID" $KEY && echo "pairing: OK" || { echo "pairing: FAIL"; fail=1; }
# KDE Connect marks the device paired a moment after our accept.
both=0
for _ in 1 2 3 4 5; do
  # Capture first: with pipefail, `grep -q` closing the pipe early would
  # make the writer fail with SIGPIPE.
  kd=$(K devices); kl=$(KC -l 2>/dev/null)
  grep -q " paired " <<<"$kd" && grep -q "associato\|paired" <<<"$kl" && { both=1; break; }
  sleep 1
done
[ $both = 1 ] && echo "both sides paired: OK" || { echo "paired state: FAIL"; fail=1; }
head -c 500000 /dev/urandom >"$T/out.bin"; K send "$KID" "$T/out.bin" >/dev/null; sleep 3
F=$(find "$T/kde" -name out.bin | head -1)
[ -n "$F" ] && cmp -s "$T/out.bin" "$F" && echo "file Konnekta -> KDE: OK" || { echo "file Konnekta -> KDE: FAIL"; fail=1; }
head -c 300000 /dev/urandom >"$T/in.bin"
K action share setConfig --args "{\"destination\":\"$T/dl\"}" >/dev/null
KC -d "$MYID" --share "$T/in.bin" >/dev/null 2>&1; sleep 3
cmp -s "$T/in.bin" "$T/dl/in.bin" && echo "file KDE -> Konnekta: OK" || { echo "file KDE -> Konnekta: FAIL"; fail=1; }
K ping "$KID" "ciao" >/dev/null && echo "ping Konnekta -> KDE: OK" || { echo "ping: FAIL"; fail=1; }
grep -q "packet handling failed" "$T/k.log" && { echo "warnings in log:"; grep "packet handling failed" "$T/k.log"; fail=1; }
exit $fail

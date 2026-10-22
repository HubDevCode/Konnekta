#!/bin/sh
# Install the fingerprint-approval PAM helper where only root can change it.
#   sudo packaging/install-approve.sh [path/to/konnekta-approve]
# Then: sudo konnekta approve enroll <phone>; sudo konnekta approve enable sudo
set -eu
SRC=${1:-$(dirname "$0")/konnekta-approve}
[ -f "$SRC" ] || SRC=$(dirname "$0")/../target/release/konnekta-approve
[ "$(id -u)" = 0 ] || { echo "eseguilo con sudo" >&2; exit 1; }
[ -f "$SRC" ] || { echo "konnekta-approve non trovato" >&2; exit 1; }
install -d -o root -g root -m 0755 /usr/lib/konnekta
install -o root -g root -m 0755 "$SRC" /usr/lib/konnekta/konnekta-approve
echo "Installato /usr/lib/konnekta/konnekta-approve"

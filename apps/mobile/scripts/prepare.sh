#!/usr/bin/env bash
# Generate the standard Flutter platform scaffolding around the committed
# sources (flutter create never overwrites existing files) and apply the
# settings Konnekta needs.
set -euo pipefail
cd "$(dirname "$0")/.."
flutter create --org org.konnekta --project-name konnekta --platforms=android,ios .
# Android: Camera2 / MediaCodec low-latency APIs need API 26+.
for f in android/app/build.gradle android/app/build.gradle.kts; do
  [ -f "$f" ] && sed -i.bak -E 's/minSdk(Version)? *=? *flutter\.minSdkVersion/minSdk = 26/' "$f" && rm -f "$f.bak"
done
# iOS usage descriptions.
PL=ios/Runner/Info.plist
if [ -f "$PL" ] && command -v /usr/libexec/PlistBuddy >/dev/null; then
  for kv in \
    "NSMicrophoneUsageDescription:Usa il telefono come microfono del computer" \
    "NSCameraUsageDescription:Usa il telefono come webcam del computer" \
    "NSPhotoLibraryUsageDescription:Invia automaticamente gli screenshot al computer" \
    "NSLocalNetworkUsageDescription:Trova e collega i tuoi computer sulla rete locale"; do
    /usr/libexec/PlistBuddy -c "Add :${kv%%:*} string ${kv#*:}" "$PL" 2>/dev/null || true
  done
  /usr/libexec/PlistBuddy -c "Add :NSBonjourServices array" "$PL" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :NSBonjourServices:0 string _kdeconnect._udp" "$PL" 2>/dev/null || true
fi

# Rilascio e distribuzione

Il workflow `.github/workflows/release.yml` parte su ogni tag `v*`:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Crea una **release in bozza** su GitHub e allega:

| Piattaforma | Artefatti |
|---|---|
| Linux | AppImage, .deb, .rpm (Tauri) · `konnekta-cli-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows | MSI, setup NSIS · CLI `.zip` |
| macOS | `.app`/`.dmg` per Apple Silicon e Intel · CLI `.tar.gz` |
| Android | APK per `arm64-v8a`, `armeabi-v7a`, `x86_64` |
| iOS | `.ipa` non firmato |

Dopo la verifica, pubblica la bozza dalla pagina Releases.

## Firma del codice

| Piattaforma | Secret | Note |
|---|---|---|
| macOS | `APPLE_CERTIFICATE` (p12 base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | firma e notarizzazione automatiche tramite tauri-action |
| Windows | configurare `bundle.windows.certificateThumbprint` o `signCommand` in `tauri.conf.json` | senza firma SmartScreen mostra un avviso |
| Aggiornamenti Tauri | `TAURI_SIGNING_PRIVATE_KEY` | solo se si abilita l'updater |
| Android | aggiungere a `android/app/build.gradle(.kts)` un `signingConfig` che legga `key.properties` e salvare keystore e password come secret | senza configurazione l'APK è firmato con la chiave di debug di Flutter |
| iOS | account Apple Developer, profilo di provisioning e `flutter build ipa` | la CI produce una build non firmata |

## Linux senza interfaccia

```bash
install -Dm755 konnekta ~/.local/bin/konnekta
install -Dm644 konnekta.service ~/.config/systemd/user/konnekta.service
systemctl --user enable --now konnekta
```

# Rilascio e distribuzione

Il workflow `.github/workflows/release.yml` parte su ogni tag `v*`:

```bash
git tag v2.0.0 && git push origin v2.0.0
```

Crea una release su GitHub (in bozza finché le build non finiscono) e allega:

| Piattaforma | Artefatti |
|---|---|
| Linux | AppImage, .deb, .rpm (Tauri) · `konnekta-cli-*-x86_64-unknown-linux-gnu.tar.gz` |
| Windows | MSI, setup NSIS · CLI `.zip` |
| macOS | `.app` (archivio) per Apple Silicon e Intel · CLI `.tar.gz` |
| Android | un APK (app nativa `apps/android`) con il core per `arm64-v8a` e `x86_64` |
| iOS | `.ipa` non firmato |

La release viene pubblicata da sola quando desktop, CLI e Android sono caricati.

## Firma del codice

| Piattaforma | Secret | Note |
|---|---|---|
| macOS | `APPLE_CERTIFICATE` (p12 base64), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | firma e notarizzazione automatiche tramite tauri-action |
| Windows | configurare `bundle.windows.certificateThumbprint` o `signCommand` in `tauri.conf.json` | senza firma SmartScreen mostra un avviso |
| Aggiornamenti Tauri | `TAURI_SIGNING_PRIVATE_KEY` | solo se si abilita l'updater |
| Android | salvare come secret `ANDROID_KEYSTORE_BASE64` (keystore in base64), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`; `apps/android/app/build.gradle.kts` li legge da `KONNEKTA_KEYSTORE*` | senza secret l'APK è firmato con la chiave di debug: si installa, ma gli aggiornamenti firmati poi con la chiave vera richiedono di disinstallare |
| iOS | account Apple Developer, profilo di provisioning e `flutter build ipa` | la CI produce una build non firmata |

## Linux senza interfaccia

```bash
install -Dm755 konnekta ~/.local/bin/konnekta
install -Dm644 konnekta.service ~/.config/systemd/user/konnekta.service
systemctl --user enable --now konnekta
```

# Konnekta

Collega smartphone e computer in modo **sicuro**, **modulare** e **multipiattaforma**.
Konnekta è un sistema **autonomo, con protocollo e porte propri**: un daemon Rust sul computer e
app native sul telefono. Funziona sulla rete locale e da fuori casa (Tailscale/VPN), anche con un
firewall che blocca le connessioni in ingresso, perché è il computer ad aprirle.
Per chi lo desidera c'è una **modalità di compatibilità KDE Connect** (spenta di default) che
permette di usare l'app KDE Connect sul telefono per le funzioni standard.

| | Windows | macOS | Linux | Android | iOS |
|---|:-:|:-:|:-:|:-:|:-:|
| App | Tauri (MSI/NSIS) | Tauri (.app/.dmg) | Tauri (AppImage/deb/rpm) + CLI | Flutter (APK) | Flutter |

## Funzioni

| Funzione | Protocollo | Permesso | Note |
|---|---|---|---|
| Appunti bidirezionali (senza cicli) | KDE Connect | Clipboard | |
| File: invio/ricezione, cartella scelta, stato, ripresa, SHA-256 | KDE Connect + estensione | Files | ripresa e integrità tra dispositivi Konnekta |
| Controllo multimediale in entrambe le direzioni, volume, metadati | KDE Connect | MediaControl | Linux: MPRIS · macOS: Music/Spotify · Windows: tasti multimediali |
| Telecomando presentazioni (slide + puntatore) | KDE Connect + estensione | Presenter | mappe tasti per PowerPoint, LibreOffice, Keynote, Google Slides, PDF |
| Touchpad e tastiera remoti | KDE Connect | RemoteInput (sensibile) | |
| Comandi personalizzati e macro | KDE Connect + estensione | Commands (sensibile) | shell, app, file/URL, scorciatoie, testo, media, volume, attese |
| Notifiche: mirroring, filtri, ignora, rispondi, azioni | KDE Connect | Notifications | mirroring solo da Android |
| Chiamate: avviso, silenzia, rispondi/rifiuta | KDE Connect + estensione | Telephony | rispondi/rifiuta solo Android |
| Trova telefono, batteria, ping | KDE Connect | — | |
| **Mobile Deck**: griglia, pagine, categorie, profili, macro, stato dinamico | Konnekta | Deck (sensibile) | estendibile con azioni di qualunque plugin |
| **Microfono del telefono** come microfono del PC | Konnekta | Microphone (sensibile) | dispositivo virtuale "Konnekta Microphone" |
| **Webcam wireless** | Konnekta | Camera (sensibile) | v4l2loopback su Linux, OBS Virtual Camera su Windows/macOS |
| **Desktop remoto** (vedi e controlla il PC) | Konnekta | Desktop + RemoteInput | H.264 a bitrate adattivo |
| **Schermo del telefono** sul PC | Konnekta | Desktop | Android |
| **Audio del PC** sul telefono | Konnekta | SystemAudio (sensibile) | |
| **Non disturbare** sincronizzato con priorità configurabile | Konnekta | DoNotDisturb | |
| **Screenshot automatici** → cartella + appunti, senza duplicati | Konnekta | Screenshots | |

## Sicurezza in breve

- tutto il traffico — pacchetti, file, appunti, notifiche, comandi, audio, video, desktop — viaggia
  in **TLS 1.3/1.2** con sole suite AEAD e forward secrecy, anche sulla rete di casa;
- **autenticazione reciproca** con certificati per dispositivo e **certificate pinning** dopo
  l'associazione: un certificato diverso viene rifiutato (anti man-in-the-middle);
- associazione esplicita con **codice di 16 caratteri** derivato dalle chiavi di entrambi e
  **conferma anche sul computer**;
- **nessuna fiducia nella rete locale**: un dispositivo non associato può solo chiedere di associarsi;
- **permessi granulari per dispositivo**, quelli sensibili disattivati di default;
- i comandi remoti si richiamano **per identificativo**: il telefono non può inviare comandi arbitrari.

Dettagli in [docs/SICUREZZA.md](docs/SICUREZZA.md).

## Struttura

```
crates/
  konnekta-proto     protocollo Konnekta + traduzione per la compatibilità KDE Connect
  konnekta-crypto    certificati, TLS reciproco con pinning, chiave di verifica, keystore
  konnekta-core      discovery, link, pairing, permessi, plugin, payload, canali stream, API JSON, IPC
  konnekta-platform  integrazioni OS: appunti, input, media, notifiche, DND, avvio app
  konnekta-media     trasporto real-time: frame, jitter buffer, bitrate adattivo, pipeline, device virtuali
  konnekta-plugins   tutti i plugin + plugin "ponte" per le app mobili
  konnekta-ffi       ABI C per Flutter/Kotlin/Swift
apps/
  konnekta-cli       daemon + riga di comando (`konnekta`)
  desktop            app Tauri 2 (UI in italiano, icona nel vassoio)
  mobile             app Flutter con integrazioni Android (Kotlin) e iOS (Swift)
docs/                architettura, protocollo, sicurezza, piattaforme, rilascio
```

## Compilare

Requisiti: Rust ≥ 1.88. Per l'app desktop su Linux: `webkit2gtk-4.1`, `libayatana-appindicator`.

```bash
cargo test --workspace --exclude konnekta-desktop   # core + plugin + FFI
cargo run -p konnekta-cli -- daemon                 # servizio headless
cargo run -p konnekta-desktop                       # app desktop
```

App mobile (richiede Flutter e, per Android, NDK + `cargo-ndk`):

```bash
cd apps/mobile
./scripts/prepare.sh            # genera la parte standard dei progetti Android/iOS
./scripts/build_rust.sh android # oppure: ios
flutter run
```

## Uso dalla riga di comando

```bash
konnekta daemon                     # avvia il servizio (o apri l'app desktop)
konnekta daemon --kde-compat        # accetta anche l'app KDE Connect
konnekta devices                    # dispositivi trovati
konnekta discover                   # dispositivi trovati in rete
konnekta pair Pixel                 # mostra il codice e chiede conferma
konnekta addresses Pixel pixel.tail1234.ts.net   # raggiungibile anche fuori casa
konnekta perms Pixel                # permessi
konnekta grant Pixel remoteInput    # concede un permesso sensibile
konnekta send Pixel foto.jpg        # invia file
konnekta action deck get            # qualsiasi azione di plugin
konnekta events                     # eventi in tempo reale (JSON)
```

Su Linux il daemon può essere avviato con systemd: vedi `packaging/konnekta.service`.

## Dipendenze opzionali per le funzioni multimediali (desktop)

| Funzione | Linux | Windows | macOS |
|---|---|---|---|
| Microfono virtuale | PipeWire/PulseAudio (`pactl`) | [VB-Audio Cable](https://vb-audio.com/Cable/) | [BlackHole](https://github.com/ExistentialAudio/BlackHole) |
| Webcam virtuale | `v4l2loopback` + `ffmpeg` | OBS + `ffmpeg` | OBS + `ffmpeg` |
| Desktop remoto | `ffmpeg` (X11) o `wf-recorder` (wlroots) | `ffmpeg` | `ffmpeg` |
| Audio di sistema | `parec` | integrato (WASAPI loopback) | BlackHole |
| Touchpad su Wayland | protocolli virtual-input o `ydotool` | integrato | integrato (permesso Accessibilità) |

## Documentazione

- [Architettura](docs/ARCHITETTURA.md)
- [Protocollo e compatibilità KDE Connect](docs/PROTOCOLLO.md)
- [Sicurezza e modello di minaccia](docs/SICUREZZA.md)
- [Supporto per piattaforma e limiti](docs/PIATTAFORME.md)
- [Rilascio, firma e distribuzione](docs/RILASCIO.md)

## Licenza

GPL-3.0-or-later.

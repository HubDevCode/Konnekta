# Konnekta

Collega smartphone e computer in modo **sicuro**, **modulare** e **multipiattaforma**.
Konnekta è un sistema **autonomo**, con protocollo, porte e app propri: un daemon Rust sul
computer e app native sul telefono. Funziona sulla rete locale e da fuori casa (Tailscale/VPN),
anche con un firewall che blocca le connessioni in ingresso, perché è il computer ad aprirle.

Per chi lo desidera ci sono due **ponti facoltativi**, spenti di default, verso altre app: le app
**KDE Connect / GSConnect** e le app **Flux** possono collegarsi a Konnekta per le funzioni che
hanno in comune. I ponti traducono i pacchetti ai bordi della connessione: dentro Konnekta tutto
ha nomi e formati di Konnekta.

| | Windows | macOS | Linux | Android | iOS |
|---|:-:|:-:|:-:|:-:|:-:|
| App | Tauri (MSI/NSIS) | Tauri (.app/.dmg) | Tauri (AppImage/deb/rpm) + CLI | nativa Kotlin (in sviluppo) | nativa SwiftUI (in sviluppo) |

## Funzioni

| Funzione | Permesso | Anche con | Note |
|---|---|---|---|
| Appunti bidirezionali (senza cicli), immagini, cronologia, snippet salvati | Clipboard | KDE Connect, Flux | immagini anche con Flux |
| File: invio/ricezione, cartelle (archivio), cartella scelta, stato, ripresa, SHA-256, coda offline | Files | KDE Connect, Flux | ripresa e integrità tra dispositivi Konnekta |
| **Chat** per dispositivo: testi, link, appunti, file | — | — | solo in memoria |
| **Sfoglia PC** dal telefono (sola lettura) | BrowseFiles (sensibile) | — | cartelle consentite, file nascosti esclusi, sessione con Stop |
| Controllo multimediale in entrambe le direzioni, volume, metadati | MediaControl | KDE Connect, Flux | Linux: MPRIS · macOS: Music/Spotify · Windows: tasti multimediali |
| Telecomando presentazioni (slide + puntatore) | Presenter | KDE Connect | mappe tasti per PowerPoint, LibreOffice, Keynote, Google Slides, PDF |
| Touchpad e tastiera remoti | RemoteInput (sensibile) | KDE Connect, Flux | |
| Comandi personalizzati e macro | Commands (sensibile) | KDE Connect, Flux | shell, app, file/URL, scorciatoie, testo, media, volume, attese |
| Notifiche: mirroring, filtri, ignora, rispondi, azioni | Notifications | KDE Connect, Flux | mirroring solo da Android |
| Chiamate: avviso, silenzia, rispondi/rifiuta | Telephony | KDE Connect, Flux | rispondi/rifiuta solo Android |
| **SMS**: conversazioni e invio dal computer | Messages (sensibile) | KDE Connect, Flux | solo Android |
| **Fotocamera e scanner**: foto, testo (OCR), codici QR | Files | KDE Connect (solo foto) | il testo va negli appunti |
| Trova il telefono, batteria, ping | — | KDE Connect, Flux | |
| **Mobile Deck**: griglia, pagine, categorie, profili, macro, stato dinamico | Deck (sensibile) | — | estendibile con azioni di qualunque plugin |
| **Microfono del telefono** come microfono del PC | Microphone (sensibile) | — | dispositivo virtuale "Konnekta Microphone" |
| **Webcam wireless** | Camera (sensibile) | — | v4l2loopback su Linux, OBS Virtual Camera su Windows/macOS |
| **Desktop remoto** (vedi e controlla il PC) | Desktop + RemoteInput | — | H.264 a bitrate adattivo |
| **Schermo del telefono** sul PC | Desktop | — | Android |
| **Audio del PC** sul telefono | SystemAudio (sensibile) | — | |
| **Non disturbare** sincronizzato con priorità configurabile | DoNotDisturb | Flux | |
| **Screenshot automatici** → cartella + appunti, senza duplicati | Screenshots | — | |
| **Aggiornamento dell'app Android** inviato dal computer | Files | — | l'installer di Android chiede sempre conferma |
| **Approva `sudo` con l'impronta** del telefono | Approve (sensibile) | — | Linux; la password resta sempre disponibile |
| **Hyprland**: tema, workspace e dispatcher nel Deck, modulo Waybar | — | — | Linux |

## Sicurezza in breve

- tutto il traffico — pacchetti, file, appunti, notifiche, comandi, audio, video, desktop — viaggia
  in **TLS 1.3/1.2** con sole suite AEAD e forward secrecy, anche sulla rete di casa;
- **autenticazione reciproca** con certificati per dispositivo e **certificate pinning** dopo
  l'associazione: un certificato diverso viene rifiutato (anti man-in-the-middle);
- associazione esplicita con **codice di 16 caratteri** derivato dalle chiavi di entrambi e
  **conferma anche sul computer**;
- **nessuna fiducia nella rete locale**: un dispositivo non associato può solo chiedere di associarsi;
- **permessi granulari per dispositivo**, quelli sensibili disattivati di default;
- i comandi remoti si richiamano **per identificativo**: il telefono non può inviare comandi arbitrari;
- i ponti verso altre app sono spenti di default e non lasciano mai uscire le funzioni solo-Konnekta.

Dettagli in [docs/SICUREZZA.md](docs/SICUREZZA.md).

## Struttura

```
crates/
  konnekta-proto     protocollo Konnekta + ponti facoltativi (KDE Connect, Flux)
  konnekta-crypto    certificati, TLS reciproco con pinning, chiave di verifica, keystore
  konnekta-core      discovery, link, pairing, permessi, plugin, payload, canali stream, API JSON, IPC
  konnekta-platform  integrazioni OS: appunti, input, media, notifiche, DND, avvio app
  konnekta-media     trasporto real-time: frame, jitter buffer, bitrate adattivo, pipeline, device virtuali
  konnekta-plugins   tutti i plugin + plugin "ponte" per le app mobili
  konnekta-ffi       ABI C per le app mobili
apps/
  konnekta-cli       daemon + riga di comando (`konnekta`)
  desktop            app Tauri 2 (UI in italiano, icona nel vassoio)
  mobile             app Flutter provvisoria (sarà sostituita dalle app native)
docs/                architettura, protocollo, sicurezza, piattaforme, rilascio, piano
```

## Compilare

Requisiti: Rust ≥ 1.88. Per l'app desktop su Linux: `webkit2gtk-4.1`, `libayatana-appindicator`.

```bash
cargo test --workspace --exclude konnekta-desktop   # core + plugin + FFI
cargo run -p konnekta-cli -- daemon                 # servizio headless
cargo run -p konnekta-desktop                       # app desktop
```

## Uso dalla riga di comando

```bash
konnekta daemon                     # avvia il servizio (o apri l'app desktop)
konnekta daemon --kde-connect       # accetta anche le app KDE Connect / GSConnect
konnekta daemon --flux              # accetta anche le app Flux
konnekta devices                    # dispositivi trovati
konnekta discover                   # dispositivi trovati in rete
konnekta pair Pixel                 # mostra il codice e chiede conferma
konnekta addresses Pixel pixel.tail1234.ts.net   # raggiungibile anche fuori casa
konnekta perms Pixel                # permessi
konnekta grant Pixel remoteInput    # concede un permesso sensibile
konnekta send Pixel foto.jpg        # invia file
konnekta action deck get            # qualsiasi azione di plugin
konnekta events                     # eventi in tempo reale (JSON)
konnekta waybar                     # stato per un modulo Waybar
```

### Approvare `sudo` con l'impronta (Linux)

```bash
cargo build --release -p konnekta-approve
sudo packaging/install-approve.sh          # installa l'helper in /usr/lib/konnekta
konnekta grant Pixel approve               # permesso sensibile, una volta
sudo konnekta approve enroll Pixel         # il telefono crea la chiave; scrivi il codice che mostra
sudo konnekta approve enable sudo          # da ora sudo chiede prima al telefono
konnekta approve status
```

Dettagli e rischi in [docs/SICUREZZA.md](docs/SICUREZZA.md#approvazione-di-sudo-con-limpronta).

Su Linux il daemon può essere avviato con systemd: vedi `packaging/konnekta.service`.

### Modulo Waybar

```jsonc
"custom/konnekta": {
  "exec": "konnekta waybar",
  "return-type": "json",
  "format": "{}",
  "on-click": "konnekta-desktop"
}
```

Classi CSS: `connected`, `low` (batteria ≤ 15%), `disconnected`, `none`, `off`.

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
- [Protocollo e ponti verso altre app](docs/PROTOCOLLO.md)
- [Sicurezza e modello di minaccia](docs/SICUREZZA.md)
- [Supporto per piattaforma e limiti](docs/PIATTAFORME.md)
- [Rilascio, firma e distribuzione](docs/RILASCIO.md)
- [Piano di sviluppo](docs/PIANO.md)

## Licenza

GPL-3.0-or-later.

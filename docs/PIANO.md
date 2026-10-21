# Piano: Konnekta come sistema autonomo (ispirato a Flux)

Decisioni (16/10/2026):
- protocollo **Konnekta nativo** su porte proprie; KDE Connect solo come **modalità di compatibilità opzionale, disattivata di default**;
- funzioni di Flux in aggiunta alla specifica originale;
- desktop multipiattaforma (Tauri) con **integrazioni Hyprland**;
- app mobili **native**: Kotlin/Jetpack Compose (Android) e SwiftUI (iOS) sopra il core Rust.

## Fase 1 — Protocollo nativo e rete (core)

- Tipi di pacchetto canonici `konnekta.*`; il link KDE traduce `kdeconnect.*` ⇄ `konnekta.*` ai bordi, quindi i plugin non sanno quale protocollo usa il peer.
- Porte native: UDP 17160 (discovery), TCP 17160–17168 (link), 17170–17199 (payload/stream). mDNS `_konnekta._tcp`.
- Handshake nativo: TLS subito, ruoli normali (chi chiama è client TLS), identità **solo dentro TLS** (nessun metadato in chiaro sul TCP).
- Certificato con `O=Konnekta`; codice di verifica a **16 caratteri** (64 bit) nel protocollo nativo; 8 caratteri solo verso KDE Connect.
- Pairing rinforzato: conferma sul computer dopo l'accettazione del telefono, accetta/rifiuta legati al codice confrontato, pairing legato a link e certificato, limiti anti-abuso (richieste, link non associati, righe da 64 KiB prima del pairing, timeout di 2 minuti).
- Il computer **apre le connessioni**: riconnessione periodica ai dispositivi associati, indirizzi extra (Tailscale/VPN).
- Coda di invio offline (outbox) persistente.

## Fase 1b — Identità autonoma e ponti (21/10/2026)

Richiesta: Konnekta non deve avere nulla di KDE Connect o di Flux; parlare con le loro app è
un'opzione gradita.

- [x] Nomi propri per tipi e plugin (`konnekta.media`, `konnekta.input`, `konnekta.commands`,
  `konnekta.call`, `konnekta.sms.send`…; plugin `media`, `input`, `commands`, `calls`,
  `finddevice`), con migrazione automatica dei dati dei plugin rinominati.
- [x] Ponti facoltativi in `konnekta-proto/src/interop.rs`: tabelle esplicite KDE Connect e Flux,
  traduzione dei corpi dove servono (`flux.dnd`), nessun tipo senza voce attraversa un ponte.
- [x] Ponte **Flux**: porte 12100–12108, mDNS `_flux._udp` (annuncio e ricerca), il computer
  chiama il telefono, codice di 16 caratteri identico a Flux. Test con un telefono che parla solo
  il formato Flux.
- [x] Niente più "origine KDE/Konnekta" sui plugin: `PluginInfo::interop()` dice con quali app
  esterne funziona ciascuno; impostazioni e CLI con due interruttori (`--kde-connect`, `--flux`).
- [x] Documentazione riscritta; verifica con `kdeconnectd` reale superata.
- [ ] Ponte Flux, funzioni specifiche: invio file al telefono Flux (`flux.tunnel`), microfono,
  webcam e schermo Flux (`flux.mic`, `flux.webcam`, `flux.screen`) hanno formati propri.

## Fase 2 — Funzioni stile Flux

- [x] Chat per dispositivo (testi, link, file, appunti), cronologia appunti in memoria, snippet salvati, immagini negli appunti, invio cartelle come archivio.
- [x] Sfoglia PC dal telefono (sola lettura, cartelle consentite, file nascosti esclusi, sessione con stop).
- [x] SMS (lettura conversazioni e invio, compatibile con `kdeconnect.sms.*`).
- [x] Foto/scansione OCR/QR dal telefono (`photo.request` con `mode`, compatibile con KDE per le foto); aggiornamento dell'app Android inviato dal computer.
- [x] Approvazione `sudo`/polkit con impronta: helper PAM `konnekta-approve`, chiave pubblica del telefono di proprietà di root, firma su nonce generato dall'helper (lato computer completo e testato; la firma sul telefono arriva con le app native).
- [x] Hyprland: tema dai colori Hyprland e dalla palette Waybar, `workspaces`/`dispatch` per il Deck, modulo Waybar (`konnekta waybar`). Il tema va ancora applicato nell'app desktop (Fase 3).

## Fase 3 — App

- Desktop Tauri: vista Chat, conferma pairing a 16 caratteri, nuove impostazioni.
- Android nativo (Kotlin/Compose) con core via UniFFI; build verificata localmente con Android SDK.
- iOS nativo (SwiftUI) con core via UniFFI; build verificata in CI (macOS).
- CI/CD aggiornata; rimozione dell'app Flutter.

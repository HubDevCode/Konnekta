# Protocollo

Konnekta implementa il **protocollo KDE Connect versione 8**.

## Trasporto

1. **Discovery**: broadcast UDP su `1716` del pacchetto `kdeconnect.identity` (solo
   metadati pubblici: id, nome, tipo, capacità, porta TCP). Disattivabile (`announce=false`);
   resta la connessione manuale per IP.
2. Chi riceve l'annuncio apre una connessione TCP (porte 1716–1764) e invia la propria
   identità in chiaro.
3. Parte **TLS con ruoli invertiti**: il client TCP è il server TLS (compatibilità KDE).
   Entrambi presentano un certificato autofirmato con `CN = deviceId`.
4. Dentro TLS le parti si scambiano di nuovo l'identità; l'id deve coincidere con quello
   annunciato e con il CN del certificato.
5. Pacchetti JSON delimitati da newline: `{"id": <ms>, "type": "...", "body": {...}}`
   (massimo 1 MiB per riga).
6. **Payload** (file): il mittente apre una porta 1739–1764 e la indica in
   `payloadTransferInfo.port`; il destinatario si collega con TLS reciproco e pinning.

## Pairing

`kdeconnect.pair {"pair": true, "timestamp": <s>}` → l'utente confronta il codice
`hex(SHA-256(max(pkA,pkB) ‖ min(pkA,pkB) ‖ timestamp))[0..8]` mostrato su entrambi i
dispositivi → `{"pair": true}`. Timeout 30 s, timestamp accettato entro ±30 minuti.
`{"pair": false}` rifiuta o dissocia.

## Pacchetti standard supportati

| Tipo | Direzione |
|---|---|
| `kdeconnect.ping` | ↔ |
| `kdeconnect.battery` | telefono → PC |
| `kdeconnect.clipboard`, `kdeconnect.clipboard.connect` | ↔ |
| `kdeconnect.share.request`, `.update` | ↔ |
| `kdeconnect.mpris`, `kdeconnect.mpris.request` | ↔ |
| `kdeconnect.mousepad.request` / `.echo` / `.keyboardstate` | telefono → PC |
| `kdeconnect.presenter` | telefono → PC |
| `kdeconnect.runcommand`, `kdeconnect.runcommand.request` | ↔ |
| `kdeconnect.notification` (+ `.request`, `.reply`, `.action`) | telefono → PC (+ comandi inversi) |
| `kdeconnect.telephony`, `kdeconnect.telephony.request_mute` | telefono → PC |
| `kdeconnect.findmyphone.request` | PC → telefono |

## Estensioni Konnekta

Usate solo se il peer le annuncia nelle capacità.

| Tipo | Contenuto |
|---|---|
| campi `konnektaSha256`, ripresa | Sul canale payload il destinatario invia 8 byte (offset big-endian) se entrambi annunciano `konnekta.share.resume`. |
| `konnektaAbsX/Y` in `mousepad.request` | Puntatore assoluto normalizzato (desktop remoto). |
| `konnektaKey` in `presenter` | Comando di slide nominato (`next`, `previous`, `start`, `stop`, `black`, …). |
| `konnekta.deck.request` / `konnekta.deck` | `layout`, `press`, `setProfile` / layout pubblico, stati dinamici. |
| `konnekta.dnd` | `{enabled, changedAt, userInitiated}` |
| `konnekta.screenshot` | payload + `sha256`; `{config: {autoTransfer}}` |
| `konnekta.telephony.action` | `accept`, `reject`, `hangup`, `mute`, … |
| `konnekta.microphone`, `.camera`, `.desktop`, `.systemaudio`, `.phonescreen` | `{action: request|offer|stop|keyframe|configure, session, port, params}` |

### Frame dei flussi

```
u8 kind (1 audio, 2 video, 3 controllo) | u8 flags (1 keyframe, 2 config)
u32 seq | u64 pts_us | u32 len | payload
```
Audio: PCM s16le 48 kHz (microfono mono, audio di sistema stereo). Video: H.264 Annex B.

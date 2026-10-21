# Protocollo

Konnekta ha un **protocollo proprio** (versione 1): porte, nomi e formati dei pacchetti,
handshake e pairing sono di Konnekta. Due **ponti facoltativi**, spenti di default, permettono alle
app KDE Connect / GSConnect e alle app Flux di collegarsi per le funzioni in comune (vedi in fondo).

## Porte

| Uso | Konnekta | Ponte KDE Connect | Ponte Flux |
|---|---|---|---|
| Discovery UDP | 17160 | 1716 | 12100 |
| Link TCP | 17160–17168 | 1716–1764 | 12100–12108 |
| Payload e stream | 17170–17199 | 1739–1764 | 12070–12099 |
| mDNS | `_konnekta._tcp` | — | `_flux._udp` |

Le porte dei ponti si aprono solo quando il ponte è attivo.

## Rete: è il computer a collegarsi

I dispositivi si annunciano via UDP e mDNS (solo id, nome, tipo e porta TCP). Chi riceve un
annuncio apre la connessione. Il computer, inoltre, **ricontatta periodicamente** i dispositivi
associati all'ultimo indirizzo noto e agli **indirizzi extra** configurati (per esempio il nome
Tailscale del telefono). Quindi Konnekta funziona anche con un firewall che blocca le connessioni
in ingresso e fuori dalla rete di casa, senza porte aperte.

Se due dispositivi si collegano a vicenda nello stesso istante, entrambi tengono la connessione
aperta dal dispositivo con l'ID minore.

## Handshake nativo

1. TCP verso la porta del dispositivo.
2. **TLS subito** (1.3/1.2), ruoli normali: chi chiama è client TLS. Entrambi presentano il
   proprio certificato (`CN = deviceId`, `O = Konnekta`).
3. Dentro TLS entrambi inviano `konnekta.identity` (id, nome, tipo, versione, capacità, porta,
   `app`, `appVersion`). **Nulla dell'identità viaggia in chiaro sulla connessione.**
4. Prima di usare il link: il CN del certificato deve coincidere con l'id; per un dispositivo
   associato il certificato deve essere **identico a quello fissato**.
5. Pacchetti JSON per riga: `{"id": <ms>, "type": "konnekta.<funzione>", "body": {...}}`.
   Massimo 64 KiB per riga finché il dispositivo non è associato, poi 1 MiB.

## Pairing

1. Chi chiede invia `konnekta.pair {"pair": true, "timestamp": <s>}`.
2. Entrambi mostrano il codice
   `hex(SHA-256(max(pkA,pkB) ‖ min(pkA,pkB) ‖ timestamp))[0..16]`, in gruppi:
   `5EE6 825F 974E D59A` (64 bit).
3. L'altro dispositivo accetta solo se vede lo stesso codice: fissa il certificato e risponde
   `{"pair": true}`.
4. **Conferma**: chi ha chiesto fissa il certificato solo dopo che il suo utente conferma il
   codice. Accetta e rifiuta indicano il codice confrontato: se non corrisponde al pairing
   aperto, vengono rifiutati.
5. Ogni passaggio scade dopo 30 s; scadenza, rifiuto o caduta della connessione inviano
   `{"pair": false}` così l'altro lato rimuove il certificato che aveva fissato.

Il pairing appartiene al link e al certificato su cui è iniziato: mentre è aperto, un secondo
link con lo stesso id ma un altro certificato viene rifiutato.

## Limiti per i dispositivi non associati

| Risorsa | Limite |
|---|---|
| Link non associati | 8 in tutto, 2 per indirizzo (il nuovo chiude il più vecchio) |
| Link in ingresso senza richiesta di pairing | chiuso dopo 2 minuti |
| Connessioni in handshake | 32 in tutto, 4 per indirizzo |
| Richieste di pairing | 1 ogni 2 s per dispositivo; 4 aperte, 2 per indirizzo; 30 s di attesa dopo una richiesta rifiutata, ritirata o scaduta |
| Dispositivi scoperti in memoria | 64 |

## Pacchetti

Tutti i tipi sono nel namespace `konnekta.`:

| Area | Tipi `konnekta.*` |
|---|---|
| Base | `ping`, `battery`, `finddevice`, `permissions` |
| Appunti | `clipboard`, `clipboard.initial`, `clipboard.image` |
| File | `share`, `share.update`, `browse`, `browse.request`, `screenshot`, `photo`, `photo.request`, `app.update` |
| Controllo | `media`, `media.request`, `mixer`, `mixer.request`, `input`, `input.echo`, `input.state`, `presenter`, `commands`, `commands.request`, `command.state`, `deck`, `deck.request` |
| Telefono | `notification` (+ `.request`, `.reply`, `.action`), `call`, `call.mute`, `call.action`, `sms.messages`, `sms.send`, `sms.conversations.request`, `sms.thread.request`, `dnd` |
| Sicurezza | `approve`, `approve.request` |
| Flussi | `microphone`, `camera`, `desktop`, `systemaudio`, `phonescreen`, `phoneaudio` |

Campi dei trasferimenti: ripresa con offset e `konnektaSha256`; `konnektaAbsX/Y` nel touchpad
(desktop remoto); `konnektaSwipe` (`left`, `right`, `up`, `down`) per il gesto a tre dita, che sul
computer cambia workspace; `konnektaKey` nel presentatore.

Stream rifiutato perché sul computer manca qualcosa: lo `stop` porta anche `missing` (programmi o
moduli) e `fix` (comandi per installarli), e il telefono li mostra in un popup.

Associazione di nuovo dopo una reinstallazione: se un dispositivo appena associato ha lo stesso
nome e tipo di uno già associato ma non connesso, sostituisce quella voce e ne eredita i permessi.

Permessi: `permissions` (`{permissions: [...], changedAt}`) è gestito dal core, solo tra dispositivi
associati e solo sul protocollo nativo. Ogni coppia ha un unico insieme di permessi: chi modifica un
permesso invia l'insieme intero, e alla connessione vince la modifica più recente. Se nessuno dei
due lo ha mai modificato si prende l'unione. Un permesso sensibile concesso dall'altro dispositivo
compare come avviso di sicurezza. Si disattiva con `sync_permissions = false`: allora ogni lato
decide da solo.

Mixer: `mixer.request` porta `subscribe` e/o una modifica a un canale (`kind`: `output`, `input`
o `app`; `id`; `volume` 0–150; `mute`; `makeDefault`; `output` per spostare un'app). Il computer
risponde con `mixer` (`outputs`, `inputs`, `apps`, ognuno `{id, label, detail, volume, muted,
isDefault, output, icon}`) e invia le variazioni per 60 s dall'ultima richiesta. Permesso:
controllo multimediale.

### Frame dei flussi

Video del desktop remoto: un frame per immagine completa (Annex B), inviato appena l'encoder lo
scrive. Il computer legge l'uscita FLV della cattura, che ha un tag per immagine, e manda SPS/PPS
(flag config) prima di ogni keyframe. Nessun frame viene scartato: se la rete rallenta l'invio
aspetta (buffer TCP di 256 KB) e lo strumento di cattura salta da sé le immagini vecchie.

Lettore del computer: `media` porta anche `albumArtUrl`. La copertina arriva come payload di un
`media` con `transferringAlbumArt: true`, una volta per brano. Senza `player` nella richiesta il
computer descrive il lettore che sta suonando.

```
u8 kind (1 audio, 2 video, 3 controllo) | u8 flags (1 keyframe, 2 config)
u32 seq | u64 pts_us | u32 len | payload
```

## Ponti verso altre app (facoltativi)

I ponti si attivano da Impostazioni o con `konnekta daemon --kde-connect` / `--flux`. Un ponte
apre le porte dell'altra app e parla il suo handshake "v8": identità in chiaro sul TCP, poi TLS
con i ruoli invertiti, poi di nuovo l'identità dentro TLS. **La traduzione avviene solo ai bordi
del link** (`crates/konnekta-proto/src/interop.rs`): una tabella esplicita associa ogni nome
dell'altra app al tipo Konnekta (`kdeconnect.mpris` ⇄ `konnekta.media`, `flux.sms.request` ⇄
`konnekta.sms.send`) e, dove serve, converte il corpo (`flux.dnd {"on"}` ⇄ `konnekta.dnd
{"enabled", "changedAt"}`). I plugin non sanno quale app usa il telefono. Un tipo senza voce in
tabella non attraversa mai il ponte, in nessuna direzione: le funzioni solo-Konnekta (Deck,
flussi, Sfoglia PC, approvazioni…) restano tra dispositivi Konnekta.

Certificati, pinning, conferma sul computer e limiti anti-abuso sono gli stessi del protocollo
nativo.

| | KDE Connect / GSConnect | Flux |
|---|---|---|
| Chi apre la connessione | il telefono, dopo l'annuncio UDP del computer | il computer (mDNS `_flux._udp`, UDP, indirizzi extra) |
| Codice di pairing | 8 caratteri (l'app ne mostra 8) | 16 caratteri, identico a quello di Flux |
| Funzioni in comune | ping, batteria, trova telefono, appunti, file, media, touchpad e tastiera, presentazioni, comandi, notifiche, chiamate, SMS, foto | ping, batteria, trova telefono, appunti (anche immagini), file, media, touchpad, comandi, notifiche, chiamate, SMS, Non disturbare |

L'app Flux per Android accetta un computer non ancora associato solo mentre è aperta sullo schermo
(o mentre cerca computer): tienila aperta durante il pairing.

`scripts/interop-kdeconnect.sh` verifica il ponte KDE Connect con un `kdeconnectd` reale isolato
(testato con KDE Connect 26.08): pairing, file nelle due direzioni, ping. Il ponte Flux è
verificato dai test con un telefono simulato che parla solo il formato di Flux
(`raw_flux_phone_pairs_and_pings`).

# Protocollo

Konnekta ha un **protocollo proprio** (versione 1), derivato dal formato di KDE Connect v8 ma
indipendente: porte, tipi di pacchetto, handshake e pairing sono di Konnekta. La compatibilità
con le app KDE Connect / GSConnect è una **modalità opzionale, disattivata di default**.

## Porte

| Uso | Konnekta | KDE Connect (solo se attivato) |
|---|---|---|
| Discovery UDP | 17160 | 1716 |
| Link TCP | 17160–17168 | 1716–1764 |
| Payload e stream | 17170–17199 | 1739–1764 |
| mDNS | `_konnekta._tcp` | — |

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

| Tipo `konnekta.*` | Equivalente KDE Connect |
|---|---|
| `ping`, `battery`, `clipboard`, `clipboard.connect` | sì |
| `share.request`, `share.request.update` | sì |
| `mpris`, `mpris.request` | sì |
| `mousepad.request`, `.echo`, `.keyboardstate`, `presenter` | sì |
| `runcommand`, `runcommand.request` | sì |
| `notification` (+ `.request`, `.reply`, `.action`) | sì |
| `telephony`, `telephony.request_mute`, `findmyphone.request` | sì |
| `sms.messages`, `sms.request`, `sms.request_conversations`, `sms.request_conversation` | sì |
| `deck`, `deck.request`, `dnd`, `screenshot`, `telephony.action`, `clipboard.image`, `browse`, `approve`, `app.update` | solo Konnekta |
| `microphone`, `camera`, `desktop`, `systemaudio`, `phonescreen` | solo Konnekta |

Estensioni dei campi (solo Konnekta): ripresa dei trasferimenti con offset e `konnektaSha256`;
`konnektaAbsX/Y` nel touchpad (desktop remoto); `konnektaKey` nel presentatore.

### Frame dei flussi

```
u8 kind (1 audio, 2 video, 3 controllo) | u8 flags (1 keyframe, 2 config)
u32 seq | u64 pts_us | u32 len | payload
```

## Modalità di compatibilità KDE Connect

Attivabile da Impostazioni o con `konnekta daemon --kde-compat`. Apre le porte di KDE Connect,
usa il suo handshake (identità in chiaro, poi TLS con ruoli invertiti) e **traduce i nomi dei
pacchetti ai bordi del link** (`kdeconnect.ping` ⇄ `konnekta.ping`): i plugin non sanno quale
protocollo usa il telefono. Le funzioni solo-Konnekta non vengono mai inviate a un'app KDE
Connect. Il codice di pairing mostrato è di 8 caratteri (l'app KDE ne mostra 8), e anche qui
serve la conferma sul computer.

`scripts/interop-kdeconnect.sh` verifica la compatibilità con un `kdeconnectd` reale isolato
(testato con KDE Connect 26.08): pairing, file nelle due direzioni, ping.

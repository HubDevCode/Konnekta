# Sicurezza

## Obiettivo

Tutte le comunicazioni tra dispositivi sono **riservate, autenticate e integre**. Chi
intercetta il traffico — anche sulla stessa rete Wi-Fi — non può leggere pacchetti, file,
appunti, notifiche, credenziali né ricostruire flussi audio/video, e non può impersonare un
dispositivo associato.

## Modello di minaccia

| Attaccante | Contromisura |
|---|---|
| Ascolto passivo sulla LAN | TLS 1.3/1.2 obbligatorio per controllo, payload e stream; solo suite AEAD con (EC)DHE. Nessun dato sensibile in chiaro: in chiaro viaggia solo l'identità (id, nome, capacità). |
| Man-in-the-middle attivo | Autenticazione TLS **reciproca**; dopo il pairing il certificato del peer è **fissato** (confronto byte per byte). Al primo pairing il codice di verifica lega entrambe le chiavi pubbliche: un MITM produce codici diversi sui due schermi. |
| Dispositivo malevolo sulla rete | Un dispositivo non associato può solo inviare `kdeconnect.pair`; ogni altro pacchetto viene scartato. Numero massimo di link non associati (8). Il pairing richiede conferma esplicita. |
| Impersonificazione di un dispositivo associato | Il certificato deve coincidere con quello fissato; la firma dell'handshake prova il possesso della chiave privata. Test: `paired_device_reconnects_with_pinned_certificate`. |
| Replay | Ogni sessione TLS ha chiavi nuove (forward secrecy) e numeri di sequenza propri: un messaggio registrato non è riutilizzabile in un'altra sessione né riordinabile nella stessa. Il timestamp di pairing è accettato solo entro ±30 minuti e la richiesta scade dopo 30 s. Gli id di sessione degli stream sono casuali a 128 bit e legati al dispositivo. |
| Abuso di funzioni sensibili | Permessi per dispositivo; quelli sensibili (input remoto, comandi, Deck, microfono, fotocamera, desktop, audio di sistema) sono **disattivati per default**. Il controllo è centralizzato nel core per pacchetti in ingresso e in uscita. |
| Esecuzione di comandi arbitrari | I comandi sono definiti solo localmente; il telefono li richiama per id. Il Deck invia al telefono solo l'aspetto dei pulsanti, mai i comandi. |
| File malevoli | Nomi file ripuliti (niente percorsi, nomi riservati, caratteri di controllo); nessuna apertura automatica; verifica SHA-256; limite dimensione configurabile. Gli URL ricevuti non vengono mai aperti automaticamente. |
| Input malformati / DoS | Limiti su lunghezza pacchetti (1 MiB), identità (16 KiB), frame (8 MiB), timeout su handshake e payload, connessioni payload accettate solo dall'IP del dispositivo atteso. |
| XSS nell'interfaccia | La UI desktop inserisce dati remoti solo come testo; CSP restrittiva. |
| Controllo locale del daemon | Socket Unix con permessi `0600` nella directory runtime dell'utente; named pipe Windows che rifiuta client remoti. Nessuna porta di controllo in rete. |

## Gestione delle chiavi

- Chiave ECDSA P-256 generata localmente al primo avvio; certificato autofirmato con
  `CN = deviceId`, validità 10 anni.
- Chiave privata salvata nel **portachiavi del sistema** con la feature `os-keyring`
  (Keychain, Credential Manager, Secret Service) oppure in un file `0600` scritto in modo
  atomico; i permessi troppo larghi vengono corretti all'avvio.
- Certificati fissati e permessi in `devices.json` (`0600`).
- **Rinnovo/revoca**: `FileKeyStore::rotate` genera una nuova identità (tutti i dispositivi
  dovranno riassociarsi); "Dissocia" revoca la fiducia e interrompe subito tutte le sessioni e
  gli stream del dispositivo. La revoca di un permesso interrompe gli stream relativi entro 1 s.
- I log non contengono contenuti privati: gli appunti vengono registrati solo come lunghezza.

## Disconnessioni e anomalie

Una verifica TLS fallita per un dispositivo associato genera un **avviso di sicurezza** nella
UI. Sessioni e stream terminano alla disconnessione, alla dissociazione o alla revoca dei
permessi. Un pacchetto da un dispositivo non associato provoca la risposta `pair:false`.

## Test di sicurezza automatici

| Test | Verifica |
|---|---|
| `konnekta-crypto` `tls::tests::*` | TLS reciproco, CN obbligatorio, rifiuto MITM con stesso id, rifiuto client senza certificato |
| `cert::tests::verification_key_*` | codice simmetrico e legato a entrambe le chiavi |
| `store::tests::persists_and_reloads` | permessi `0600` della chiave |
| `konnekta-core` `integration` | pairing, rifiuto, dissociazione, pinning dopo riavvio, impostore, permessi in ingresso/uscita, canali stream |
| `payload::tests::*` | trasferimento con ripresa, rifiuto di un dispositivo diverso |
| `util::tests::sanitize` | path traversal e nomi riservati |
| `konnekta-plugins` `e2e` | comandi solo per id e con permesso, Deck, deduplica screenshot |

La CI esegue questi test su Linux, Windows e macOS e un `cargo audit` delle dipendenze.

## Limiti noti

- Il primo contatto si basa sulla verifica visiva del codice da parte dell'utente (come KDE
  Connect): se l'utente accetta senza confrontarlo, un MITM presente in quel momento non
  verrebbe rilevato.
- L'identità UDP rivela nome e tipo del dispositivo sulla rete locale: disattiva gli annunci
  sulle reti pubbliche.

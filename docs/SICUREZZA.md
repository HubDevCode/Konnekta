# Sicurezza

## Obiettivo

Tutte le comunicazioni tra dispositivi sono **riservate, autenticate e integre**. Chi
intercetta il traffico — anche sulla stessa rete Wi-Fi — non può leggere pacchetti, file,
appunti, notifiche, credenziali né ricostruire flussi audio/video, e non può impersonare un
dispositivo associato.

## Modello di minaccia

| Attaccante | Contromisura |
|---|---|
| Ascolto passivo sulla LAN | TLS 1.3/1.2 obbligatorio per controllo, payload e stream; solo suite AEAD con (EC)DHE. Nel protocollo nativo l'identità viaggia solo dentro TLS; in chiaro c'è solo l'annuncio UDP/mDNS (id, nome, tipo, porta), disattivabile. |
| Man-in-the-middle attivo | Autenticazione TLS **reciproca**; dopo il pairing il certificato del peer è **fissato** (confronto byte per byte). Al pairing il codice di 16 caratteri (64 bit) lega entrambe le chiavi pubbliche: un MITM produce codici diversi e non può cercare un certificato con lo stesso codice nei 30 s della richiesta. Chi avvia il pairing deve confermare il codice anche dopo l'accettazione dell'altro lato; accetta/rifiuta sono legati al codice confrontato. |
| Dispositivo malevolo sulla rete | Un dispositivo non associato può solo inviare `konnekta.pair`; ogni altro pacchetto viene scartato. Limiti per link, handshake e richieste di pairing per indirizzo, righe da 64 KiB, chiusura dopo 2 minuti senza richiesta (vedi PROTOCOLLO.md). I ponti verso KDE Connect e Flux sono spenti di default, quindi le loro porte (1716–1764, 12070–12108) restano chiuse. |
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
- **Permessi sincronizzati** (predefinito): un dispositivo associato può cambiare i permessi
  della coppia anche su questo computer. Il canale è quello autenticato del pairing, quindi solo
  un dispositivo associato può farlo, mai un altro della rete. Ogni permesso sensibile concesso
  dall'altro lato compare come avviso di sicurezza. Chi vuole che il computer decida da solo
  disattiva "Sincronizza i permessi" nelle Impostazioni.
- I log non contengono contenuti privati: gli appunti vengono registrati solo come lunghezza.

## Disconnessioni e anomalie

Un file `devices.json` illeggibile viene spostato in `devices.json.broken-<ora>` e Konnekta parte senza dispositivi associati; un certificato fissato che non si legge equivale a "non associato".


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

## Approvazione di `sudo` con l'impronta

Il telefono può approvare `sudo` (e `polkit-1`, dove è sicuro) al posto della password. Se
qualcosa non va, PAM chiede la password come sempre: l'approvazione non può mai rendere l'accesso
root più facile che con la password.

| Parte | Gira come | Compito |
|---|---|---|
| `konnekta-approve` (`/usr/lib/konnekta/`) | root, avviato da `pam_exec` | crea la richiesta, verifica la firma, decide |
| `/etc/konnekta/approve/<utente>.pub` | file di root | la chiave pubblica P-256 del telefono: il punto di fiducia |
| daemon Konnekta | l'utente | trasporta solo i messaggi tra helper e telefono |
| app del telefono | — | mostra la richiesta e firma con una chiave che esce solo dall'hardware sicuro (Android Keystore, Secure Enclave), dopo l'impronta |

**Come funziona.** L'helper legge gli elementi PAM, rifiuta `sshd`, i servizi non elencati
(`sudo`, `sudo-i`, `polkit-1`) e le richieste fatte da un altro utente. Legge e controlla il file
della chiave, poi genera un nonce casuale di 32 byte e l'ora. Si collega a
`/run/user/<uid>/konnekta.sock` e, prima di inviare qualcosa, verifica che cartella, socket e
processo dall'altra parte (`SO_PEERCRED`) appartengano all'utente. Il telefono firma il messaggio
`konnekta-approve-v1` (host, utente, servizio, tty, host remoto, ora, nonce). L'helper **ricostruisce
il messaggio dai propri campi** e verifica la firma con la chiave di root: non usa nulla di ciò
che riceve indietro. Esce con 0 solo con una firma valida.

**Controlli sul file della chiave.** Percorso fisso (nessuna opzione o variabile d'ambiente lo
cambia). File regolare, non link simbolico, di root, non scrivibile da gruppo o altri. Ogni
cartella fino a `/` è di root e non scrivibile da altri. Il file viene aperto con `O_NOFOLLOW` e
deve essere lo stesso file controllato. Al massimo 16 KiB, una sola chiave EC P-256.

**Registrazione** (`sudo konnekta approve enroll <telefono>`). Il telefono crea una nuova chiave
che richiede l'impronta e firma il messaggio `konnekta-approve-enroll-v1`. Il comando verifica la
firma con la nuova chiave e chiede di **scrivere il codice che il telefono mostra**: i primi 8
byte dello SHA-256 della chiave, che non vengono mai stampati nel terminale. Solo allora root
scrive il file. `sudo konnekta approve enable sudo` aggiunge la riga PAM, con una copia di
sicurezza in `/etc/konnekta/pam-backup/`, e rifiuta un helper che non sia di root in una cartella
di root.

| Attaccante | Può | Non può |
|---|---|---|
| Rete | bloccare o ritardare: PAM chiede la password | leggere o modificare i messaggi (TLS con pinning), creare una firma valida |
| Codice che gira come l'utente | sostituire il daemon, fermare l'approvazione, mandare richieste vere al telefono | firmare, cambiare il file della chiave senza una registrazione o un `sudo` dell'utente, puntare l'helper a un'altra chiave, riusare una vecchia firma |
| Chi ha il telefono bloccato o sbloccato | rifiutare le richieste | approvare senza l'impronta |

**Rischi aperti.**
- *Approvazione per abitudine*: codice che gira come l'utente può avviare `sudo` e aspettare.
  Approva solo subito dopo aver scritto tu il comando, e controlla terminale e servizio mostrati.
- *Registrazione in una sessione compromessa*: codice dell'utente può digitare nel terminale il
  codice di una propria chiave (tastiera virtuale). Registra il telefono solo in una sessione di
  cui ti fidi.
- *Nessuna attestazione della chiave*: il computer si fida che il telefono abbia creato la chiave
  con l'impronta obbligatoria; un'app modificata potrebbe non farlo.
- *polkit*: polkit non dice quale utente chiede, quindi `polkit-1` è permesso solo se
  `polkit-agent-helper-1` è setuid. Con polkit ≥ 126, che lo esegue come servizio di sistema (per
  esempio su Arch), `enable polkit-1` viene rifiutato.
- Solo utenti locali (`/etc/passwd`); Linux per ora.

Test: `crates/konnekta-approve` (formato dei messaggi, nessun valore può aggiungere righe, file
non sicuri rifiutati, nomi utente, chi può chiedere) e `konnekta-plugins` `e2e` `approve::*`
(helper vero e daemon vero con un telefono simulato: approvazione, rifiuto, firma falsa, nessuna
chiave, registrazione con codice giusto e sbagliato).

## Limiti noti

- Il primo contatto si basa sulla verifica visiva del codice da parte dell'utente: se l'utente
  accetta senza confrontarlo, un MITM presente in quel momento non verrebbe rilevato.
- Con un ponte attivo l'identità del computer viaggia anche in chiaro sul TCP (handshake "v8"
  delle altre app) e, verso KDE Connect, il codice di pairing ha 8 caratteri (32 bit) perché
  l'app ne mostra solo 8. Il protocollo Konnekta non ha questi limiti.
- L'identità UDP rivela nome e tipo del dispositivo sulla rete locale: disattiva gli annunci
  sulle reti pubbliche.

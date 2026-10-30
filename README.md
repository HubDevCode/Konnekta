<p align="center"><img src="docs/img/icon.png" alt="Konnekta" width="128" height="128"></p>

<h1 align="center">Konnekta</h1>

Collega smartphone e computer in modo **sicuro**, **modulare** e **multipiattaforma**.
Konnekta è un sistema **autonomo**, con protocollo, porte e app propri: un daemon Rust sul
computer e app native sul telefono. Funziona sulla rete locale e da fuori casa (Tailscale/VPN),
anche con un firewall che blocca le connessioni in ingresso, perché è il computer ad aprirle.
Funziona anche **con il cavo USB** e **via Bluetooth**, senza Wi-Fi: vedi
[Collegamento via cavo USB](#collegamento-via-cavo-usb) e [Collegamento via Bluetooth](#collegamento-via-bluetooth).

Per chi lo desidera ci sono due **ponti facoltativi**, spenti di default, verso altre app: le app
**KDE Connect / GSConnect** e le app **Flux** possono collegarsi a Konnekta per le funzioni che
hanno in comune. I ponti traducono i pacchetti ai bordi della connessione: dentro Konnekta tutto
ha nomi e formati di Konnekta.

| | Windows | macOS | Linux | Android | iOS |
|---|:-:|:-:|:-:|:-:|:-:|
| App | Tauri (MSI/NSIS) | Tauri (.app/.dmg) | Tauri (AppImage/deb/rpm) + CLI | nativa Kotlin/Compose | Flutter (la nativa SwiftUI è in sviluppo) |

## Scaricare

Le versioni pronte sono nelle [release su GitHub](https://github.com/HubDevCode/Konnekta/releases/latest):
AppImage, .deb e .rpm per Linux, MSI e setup per Windows, .app per macOS, la CLI e l'APK Android
(`arm64-v8a` per quasi tutti i telefoni, `x86_64` per gli emulatori, `universal` se non sai quale).
L'app Android controlla a ogni avvio se c'è una versione nuova e propone di installarla
(**Computer → Informazioni**).

## Novità della 2.5.7

**Log per capire i problemi (computer)**
- Avviando Konnekta dal terminale con **`--log`** (anche `-log`) stampa tutto quello che fa: versione, sessione
  grafica, ogni passo del programma e gli eventuali crash con il punto esatto. Basta copiare l'output e mandarlo.
  ```bash
  ./Konnekta_2.5.7_amd64.AppImage --log
  ```
- **Linux, l'AppImage non parte** (`libfuse.so.2`, «Cannot mount AppImage», «please check your FUSE setup»): manca
  FUSE 2. Si installa (`sys-fs/fuse:0` su Gentoo, `fuse2` su Arch, `libfuse2` su Debian/Ubuntu, `libfuse2t64` su Ubuntu 24.04) oppure si avvia senza:
  `./Konnekta_2.5.7_amd64.AppImage --appimage-extract-and-run`. Provato su Gentoo.

## Novità della 2.5.6

**Desktop remoto e schermo secondario**
- **Zoom con due dita**: allarga o stringi due dita per ingrandire fino a 5×, muovile per spostare la vista; due dita
  che si spostano senza allargarsi scorrono come prima. Si accende o spegne a parte per il desktop remoto e per lo
  schermo secondario (**Impostazioni**); il pulsante in alto a destra torna a 1×.
- Ingrandito, la **vista segue il puntatore**, tanto più da vicino quanto più è zoomato: il computer (Hyprland)
  dice al telefono dove si trova il puntatore, anche quando lo muovi dal computer. Con il touchpad, da ingrandito
  il puntatore si muove più fine.
- **Risoluzione dello schermo secondario** in Impostazioni: dal 50% al 100% di quella del telefono, per testo e
  finestre più grandi.

**Invia (computer)**
- Ciò che è arrivato a destinazione esce da sola dalla **Selezione**; se l'invio non riesce resta lì, pronto da
  inviare di nuovo.

**Altro**
- **Approva sudo con l'impronta** si attiva dall'app del computer con un pulsante (Configurazione).
- Webcam: niente più immagine verde dopo aver spento e riacceso la fotocamera.
- **Sfoglia il PC** funziona subito: decide il permesso «Sfoglia i file del computer».

## Novità della 2.5.5

**Pagina Invia rifatta (telefono e computer)**
- Nuova pagina **Invia** sul modello di LocalSend, con due schede: **Invia** e **Ricevi**.
- *Invia*: pulsanti grandi per **File, Media, Testo e Appunti** (sul computer **File, Cartella, Testo e Appunti**);
  ciò che scegli compare in una scheda con le anteprime, si toglie con la X o si aggiunge altro; poi tocchi il
  dispositivo nella lista **Dispositivi vicini** (computer Konnekta e dispositivi LocalSend, con icona ed etichette:
  Konnekta/LocalSend, Wi-Fi/cavo/Bluetooth, `#IP`, modello) e l'invio parte, con avanzamento e Annulla.
- *Ricevi*: logo con l'anello che indica "pronto a ricevere", nome del dispositivo, stato di Konnekta e di LocalSend,
  scelta della ricezione rapida e lista **Ricevuti** con apertura dei file al tocco.
- **Ping** sempre a portata di mano: campanella in alto a destra sul telefono, su ogni computer della lista sul desktop.
- "Prendi file dal computer" resta nella sezione *Altro*. Sul desktop c'è la nuova voce **Invia** nella barra laterale.
- Sul telefono non c'è più "Personalizza Invia": la pagina ha una struttura fissa, come LocalSend.

**Compatibilità LocalSend** (spenta di default)
- Konnekta parla anche il protocollo **LocalSend v2**: trova le app LocalSend sulla stessa rete (multicast e ricerca
  indirizzo per indirizzo quando il multicast è bloccato), invia e riceve **file, cartelle e testi**, in **HTTPS**,
  con **PIN** facoltativo e invio a un indirizzo IP scritto a mano.
- Si attiva in **Impostazioni → LocalSend** (telefono e computer) o con "Attiva" nella pagina Invia. Sul computer si
  scelgono anche nome visibile, PIN, porta, HTTPS e cartella dei file ricevuti.
- I dispositivi LocalSend **non vengono associati**: ogni invio va accettato (finestra e notifica Accetta/Rifiuta),
  salvo la **ricezione rapida**, e non possono usare nient'altro di Konnekta. I file ricevuti sul telefono vanno in
  *Download/Konnekta*. Se l'app LocalSend è aperta sullo stesso dispositivo la porta 53317 è occupata e la pagina lo dice.

**Invia tramite Konnekta dal sistema (computer)**
- Nel menu dei file compare **«Invia tramite Konnekta»**: Dolphin e Nemo (tasto destro), Nautilus e Caja (*Script*),
  l'elenco «Apri con»; su Windows «Invia a» e il menu del tasto destro. Konnekta si apre sulla pagina Invia con i file
  già scelti: basta toccare il dispositivo. Con Konnekta già aperto, i file passano alla finestra esistente.
- Si toglie in **Impostazioni → Programma sul computer**. Le voci puntano al file AppImage, non alla sua cartella
  temporanea.

**File ricevuti: "Apri" funziona**
- Telefono: su Android 9 il file non si apriva (URI `file://`), ora passa dal FileProvider; si apre subito con l'app
  predefinita e, se il file è stato spostato o non c'è un'app adatta, un messaggio lo dice. Anche la notifica apre il file.
- Computer: la notifica "File ricevuto" ha **Apri** e **Mostra nella cartella** (anche con un clic sulla notifica),
  così come i messaggi, la pagina Trasferimenti e la lista Ricevuti.
- Linux: i **pulsanti delle notifiche** (Apri link, Rispondi, Stop, rispondi alla chiamata…) non facevano nulla; ora
  rispondono tutti.

**Schermo secondario e desktop remoto**
- **Molto più reattivi**: l'encoder x264 tratteneva un fotogramma per ogni thread (21 fotogrammi, circa 0,7 s, su un
  processore a 20 core); ora consegna ogni fotogramma appena codificato. Con NVENC `delay=0`, coda di rete più corta,
  **60 fps** anche per il desktop remoto, decoder del telefono in bassa latenza (anche con le opzioni di Qualcomm,
  Exynos, MediaTek, HiSilicon e Amlogic). Vale via Wi-Fi e via cavo.
- **Orientamento** (telefono → Impostazioni → Desktop remoto e schermo secondario): schermo secondario **orizzontale**
  (lo schermo gira) o **verticale** (il computer crea un monitor verticale della risoluzione del telefono); desktop
  remoto **orizzontale**, **verticale** o **automatico** (segue la rotazione).
- **Audio**: col desktop remoto l'audio del computer anche sul telefono (a scelta); con lo schermo secondario
  **duplica** l'audio del computer, usa un'**uscita a sé** «Konnekta-Schermo» (Linux: le app mandate lì suonano solo
  sul telefono, come un monitor con le casse) oppure **nessuno**. In tempo reale: l'audio in ritardo viene scartato,
  non accumulato; vale anche per "Audio del computer" in Controllo.

**Bluetooth**
- **Collegamento immediato**: anche il telefono chiama il computer (all'apertura dell'app, quando torna in primo piano,
  quando si accende il Bluetooth e poi ogni pochi secondi finché un computer associato non è collegato). Prima solo il
  computer chiamava, un tentativo fallito bloccava 30 s e il successivo arrivava dopo oltre un minuto.
- **Touchpad e puntatore in tempo reale**: i movimenti che si accumulano vengono uniti in uno e via Bluetooth partono
  al massimo ogni 25 ms, così il computer riceve sempre l'ultima posizione invece di una coda che cresceva (il
  puntatore restava indietro di secondi). Clic, tasti e gesti non vengono mai ritardati.

**Altre correzioni**
- App desktop: cambiando pagina si riparte dall'inizio (prima restava la posizione di scorrimento della pagina precedente).
- Telefono, pagina **Computer**: tolta l'icona delle impostazioni in alto a destra (resta il pulsante in basso).

## Novità della 2.5.0

- **Scelta del collegamento**: icone di Wi-Fi, Bluetooth e cavo sotto ogni computer; tenendolo premuto
  si sceglie quale usare, la scelta è ricordata su entrambi i lati e, se non è disponibile, Konnekta
  passa da solo alla migliore disponibile.
- **Schermo secondario**: il telefono diventa un altro monitor del PC (Hyprland e Sway).
- **Puntatore "dove tocchi" o "come un touchpad"**, per desktop remoto e schermo secondario.
- **Webcam** riscritta: obiettivo, risoluzione e fps cambiano dal vivo, continua fuori dall'app e a
  schermo spento; "Konnekta Camera" resta sempre presente sul PC mentre Konnekta è aperto.
- **Bluetooth** più stabile (non si riconnette più ogni pochi secondi); audio e video sono disattivati
  via Bluetooth perché la banda non basta.
- Un tocco su una funzione senza permesso spiega quale permesso manca.
- Il lettore musicale del PC torna nel centro di controllo del telefono.
- App desktop: dispositivi divisi in **salvati** e **rilevati**, con le tecnologie disponibili.
- Pagina **Informazioni** con controllo degli aggiornamenti sul telefono.

## Funzioni

| Funzione | Permesso | Anche con | Note |
|---|---|---|---|
| Appunti bidirezionali (senza cicli), immagini, cronologia, snippet salvati | Clipboard | KDE Connect, Flux | immagini anche con Flux |
| File: invio/ricezione, cartelle (archivio), cartella scelta, stato, ripresa, SHA-256, coda offline | Files | KDE Connect, Flux | ripresa e integrità tra dispositivi Konnekta |
| **Invia tramite Konnekta** dal menu dei file del computer | Files | — | Dolphin, Nautilus, Nemo, Caja, «Apri con»; Windows «Invia a» e tasto destro |
| **LocalSend**: pagina Invia/Ricevi, invio e ricezione di file e testi con le app LocalSend (protocollo v2, HTTPS) | Files | LocalSend | spento di default, si attiva nelle impostazioni; ogni invio va accettato salvo la ricezione rapida; sul computer ciò che arriva esce dalla selezione, ciò che non arriva resta |
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
| **Webcam wireless** | Camera (sensibile) | — | v4l2loopback su Linux ("Konnekta Camera" sempre presente mentre Konnekta è aperto), OBS Virtual Camera su Windows/macOS; obiettivo, risoluzione e fps cambiano dal vivo, continua a schermo spento |
| **Desktop remoto** (vedi e controlla il PC) | Desktop + RemoteInput | — | H.264 a bassa latenza fino a 60 fps; il dito muove il puntatore dove tocchi o come un touchpad; orizzontale, verticale o automatico; audio del PC facoltativo; zoom con due dita (fino a 5×) che segue il puntatore |
| **Schermo secondario**: il telefono diventa un altro monitor del PC | Desktop + RemoteInput | — | Hyprland o Sway; orizzontale o verticale; risoluzione dal 50% al 100% del telefono; audio duplicato, uscita a sé «Konnekta-Schermo» o nessuno; zoom con due dita che segue il puntatore |
| **Schermo del telefono** sul PC | Desktop | — | Android |
| **Audio del PC** sul telefono | SystemAudio (sensibile) | — | |
| **Non disturbare** sincronizzato con priorità configurabile | DoNotDisturb | Flux | |
| **Screenshot automatici** → cartella + appunti, senza duplicati | Screenshots | — | |
| **Aggiornamento dell'app Android** inviato dal computer | Files | — | l'installer di Android chiede sempre conferma |
| **Approva `sudo` con l'impronta** del telefono | Approve (sensibile) | — | Linux; si attiva con un pulsante nell'app del computer (Configurazione); la password resta sempre disponibile |
| **Hyprland**: tema, workspace e dispatcher nel Deck, modulo Waybar | — | — | Linux |

## Collegamento via cavo USB

Tutte le funzioni (file, appunti, notifiche, touchpad, webcam, microfono, schermo, desktop
remoto…) funzionano anche con il telefono Android collegato al computer con il cavo, senza rete:

1. sul computer installa **adb** (pacchetto `android-tools`/`adb`, oppure gli *Android SDK
   Platform-Tools*; Konnekta lo cerca nel `PATH` e nelle cartelle dell'SDK);
2. sul telefono attiva **Opzioni sviluppatore → Debug USB** e, al primo collegamento, consenti il
   computer;
3. collega il cavo con Konnekta aperto su entrambi: il computer si collega da solo entro pochi
   secondi e il dispositivo appare come "Connesso · cavo USB".

Il collegamento passa per `adb forward`/`adb reverse`, ma è lo stesso di sempre: TLS reciproco,
certificato fissato, associazione e permessi invariati. Se il telefono è anche in Wi-Fi, il cavo
ha la precedenza; staccandolo Konnekta torna alla rete da solo. Si disattiva in
Impostazioni → Generale. Senza debug USB si può usare il **tethering USB** del telefono: gli
annunci partono su ogni interfaccia di rete, compresa quella del cavo.

## Collegamento via Bluetooth

Senza Wi-Fi né cavo il computer può collegarsi al telefono via Bluetooth (RFCOMM). Per ora il lato
computer è **solo Linux** (BlueZ); il telefono è Android.

1. associa telefono e computer nelle impostazioni Bluetooth del sistema (una volta sola);
2. sul telefono lascia attivo **Impostazioni → Collegamento → Bluetooth** e concedi il permesso
   *Dispositivi nelle vicinanze*;
3. con Konnekta aperto su entrambi si collegano da soli: il computer chiama i telefoni associati e il
   telefono chiama i computer associati (all'apertura dell'app, quando torna in primo piano e quando si
   accende il Bluetooth), così il collegamento parte subito senza dover essere già "connessi" nel
   sistema. Il dispositivo appare come "Connesso · Bluetooth". Si disattiva in Impostazioni → Generale.

Una sola connessione Bluetooth trasporta link e file con un piccolo multiplexer; sopra non cambia
nulla: TLS reciproco, certificato fissato, associazione e permessi sono quelli di sempre. La banda
del Bluetooth classico è bassa (circa 1-2 Mbit/s) e variabile: audio e video in tempo reale
(microfono, webcam, schermo, desktop remoto, schermo secondario) sono **disattivati** finché il
collegamento è Bluetooth; tutto il resto funziona. Touchpad e puntatore restano in tempo reale: i movimenti
che si accumulano vengono uniti e il computer riceve sempre l'ultima posizione.

## Scegliere il collegamento

Ogni dispositivo mostra con piccole icone come è raggiungibile ora (Wi-Fi, Bluetooth, cavo).
Sul telefono, nella pagina Computer, **tieni premuto un computer** per scegliere la tecnologia da
usare (sul computer: menu "Collegamento" nella scheda del dispositivo). La scelta è ricordata e
sincronizzata su entrambi i dispositivi; quando non è disponibile Konnekta passa da solo alla più
veloce disponibile (cavo, poi rete, poi Bluetooth) e torna a quella scelta appena c'è. Il tethering Bluetooth (PAN)
funziona comunque senza questa funzione, perché gli annunci partono su ogni interfaccia.

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
  android            app Android nativa (Kotlin/Compose, core via JNA)
  mobile             app Flutter, ormai solo per iOS (sarà sostituita da SwiftUI)
docs/                architettura, protocollo, sicurezza, piattaforme, rilascio, piano
```

## Compilare

Requisiti: Rust ≥ 1.88. Su Linux: `libdbus-1` (Bluetooth via BlueZ); per l'app desktop anche `webkit2gtk-4.1`, `libayatana-appindicator`.

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

Dall'app desktop: **Configurazione → Approva sudo con l'impronta → Attiva**. Confermi con
l'impronta sul telefono, controlli che il codice sia uguale e il sistema chiede la password una
volta sola (serve un agente polkit, presente in ogni desktop). Da terminale:

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
| Schermo secondario | Hyprland o Sway + `wf-recorder` | — | — |
| Collegamento Bluetooth | BlueZ (`bluetoothd`) | — | — |

## Documentazione

- [Architettura](docs/ARCHITETTURA.md)
- [Protocollo e ponti verso altre app](docs/PROTOCOLLO.md)
- [Sicurezza e modello di minaccia](docs/SICUREZZA.md)
- [Supporto per piattaforma e limiti](docs/PIATTAFORME.md)
- [Rilascio, firma e distribuzione](docs/RILASCIO.md)
- [Piano di sviluppo](docs/PIANO.md)

## Licenza

Tutti i diritti riservati. Il codice è visibile, ma copiarlo, modificarlo, ricompilarlo, ridistribuirlo o usarlo in contesti commerciali o istituzionali richiede il permesso scritto dell'autore. Vedi [LICENSE](LICENSE).

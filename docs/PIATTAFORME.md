# Supporto per piattaforma

Legenda: ✅ supportato · ⚠️ con requisiti/limiti · ❌ non possibile con le API del sistema · 🚧 non ancora implementato

## Computer

| Funzione | Linux | Windows | macOS |
|---|---|---|---|
| Appunti (testo) | ✅ wl-clipboard / X11 | ✅ | ✅ |
| Screenshot negli appunti (immagine) | ✅ | ✅ | ✅ |
| Notifiche con azioni | ✅ | ⚠️ senza pulsanti | ⚠️ senza pulsanti |
| Media: metadati e controllo lettori | ✅ MPRIS (playerctl) | ⚠️ solo tasti multimediali | ✅ Music, Spotify |
| Volume assoluto | ✅ wpctl/pactl | ⚠️ approssimato con i tasti | ✅ |
| Touchpad/tastiera | ✅ X11, Wayland (protocolli virtuali o ydotool) | ✅ | ⚠️ permesso Accessibilità |
| Non disturbare | ⚠️ GNOME | ❌ nessuna API pubblica | ❌ nessuna API pubblica |
| Microfono virtuale | ✅ PipeWire/PulseAudio | ⚠️ VB-Audio Cable | ⚠️ BlackHole |
| Webcam virtuale | ✅ v4l2loopback | ⚠️ tramite OBS Virtual Camera | ⚠️ tramite OBS Virtual Camera |
| Condivisione desktop | ✅ X11, wlroots · 🚧 GNOME/KDE Wayland (portale) | ✅ | ⚠️ permesso Registrazione schermo |
| Audio di sistema | ✅ | ✅ WASAPI loopback | ⚠️ BlackHole |

## Telefono

| Funzione | Android | iOS |
|---|---|---|
| Discovery, pairing, TLS, permessi | ✅ | ✅ (app in primo piano) |
| Invio/ricezione file, appunti, ping | ✅ | ⚠️ appunti solo con app aperta |
| Touchpad, tastiera, presentazione, Mobile Deck, comandi, media del PC | ✅ | ✅ |
| Mirroring notifiche (rispondi, azioni, ignora) | ✅ accesso notifiche | ❌ |
| Chiamate: avviso / silenzia | ✅ | ❌ |
| Chiamate: rispondi / rifiuta | ✅ Android 9+ | ❌ |
| Non disturbare | ✅ accesso "Non disturbare" | ❌ |
| Screenshot automatici | ✅ | ✅ (Foto) |
| Microfono → PC | ✅ | ✅ |
| Fotocamera → PC | ✅ Camera2 + H.264 | 🚧 |
| Schermo del telefono → PC | ✅ MediaProjection | ❌ (ReplayKit solo con estensione broadcast) |
| Desktop remoto del PC | ✅ decoder hardware | 🚧 |
| Audio del PC | ✅ | ✅ |
| Controllo media del telefono dal PC | ✅ | ❌ |
| Funzionamento in background | ✅ servizio in primo piano | ⚠️ limitato da iOS |

Su Android 14+ microfono, fotocamera e cattura schermo vanno avviati con l'app aperta
(regole dei servizi in primo piano "while-in-use").

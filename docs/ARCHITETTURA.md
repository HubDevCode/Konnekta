# Architettura

```
            ┌───────────────── App desktop (Tauri) ──────────────┐   ┌──── konnekta CLI ────┐
            │  UI web (HTML/JS)  ⇄  comando `api` + eventi        │   │  client IPC locale    │
            └───────────────┬─────────────────────────────────────┘   └──────────┬───────────┘
                            │ konnekta_core::api::call (JSON)                    │ socket Unix 0600 / named pipe
                            ▼                                                    ▼
 ┌────────────────────────────────────── konnekta-core ──────────────────────────────────────────┐
 │ Discovery UDP │ Link TLS reciproco + pinning │ Pairing │ Trust store │ Permessi │ Eventi       │
 │ Payload TLS (file) │ Canali TLS per stream │ Registro plugin + dispatch autorizzato           │
 └───────────────┬───────────────────────────────────────────────────────────────┬───────────────┘
                 ▼                                                               ▼
     konnekta-plugins (desktop)                                       plugin "ponte" (mobile)
     ping, battery, clipboard, share, mpris,                          inoltrano i pacchetti autorizzati
     mousepad, presenter, runcommand, notifications,                  all'app Flutter via FFI
     telephony, findmyphone, deck, dnd, screenshot,                            │
     microphone, camera, phonescreen, desktop, systemaudio                     ▼
                 │                                                   Dart ⇄ Kotlin / Swift
                 ▼                                                   (notifiche, fotocamera, microfono,
     konnekta-platform  ·  konnekta-media                             MediaCodec, chiamate, DND...)
```

## Principi

1. **Un solo core Rust** per tutte le piattaforme: rete, crittografia, pairing, permessi e
   protocollo non sono mai reimplementati nelle UI.
2. **Plugin indipendenti**: ogni funzione è un `Plugin` (`konnekta-core/src/plugin.rs`) che
   dichiara i tipi di pacchetto in ingresso/uscita, l'origine (KDE Connect o Konnekta) e il
   permesso che la protegge. Si attivano/disattivano globalmente o per singolo dispositivo.
3. **Autorizzazione centralizzata**: il core consegna un pacchetto a un plugin solo se
   dispositivo associato → plugin attivo → plugin attivo per quel dispositivo → permesso concesso.
   Lo stesso controllo vale per i pacchetti in uscita (`DeviceHandle::send`).
4. **Negoziazione delle capacità**: le capacità annunciate nell'identità sono l'unione dei
   plugin attivi; un pacchetto viene inviato solo se il peer lo accetta. Le estensioni
   `konnekta.*` sono quindi invisibili ai client KDE Connect.
5. **Differenze tra OS isolate** in `konnekta-platform` (trait + implementazioni per OS, più
   mock per i test) e nelle app native mobili.
6. **Un'unica API** (`konnekta_core::api`) condivisa da app desktop, CLI/daemon e FFI mobile.

## Aggiungere un plugin

```rust
pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            id: "myplugin",
            name: "La mia funzione",
            description: "…",
            origin: PluginOrigin::Konnekta,
            permission: Permission::Commands,       // permesso richiesto
            incoming: &["konnekta.myplugin"],
            outgoing: &["konnekta.myplugin"],
        }
    }
    async fn handle_packet(&self, dev: &DeviceHandle, p: Packet) -> Result<()> {
        // già autenticato e autorizzato dal core
        Ok(())
    }
    async fn handle_action(&self, dev: Option<&DeviceHandle>, action: &str, args: Value) -> Result<Value> {
        // azioni locali invocabili da UI, CLI e Mobile Deck (azione "plugin")
        Ok(Value::Null)
    }
}
```

Registrarlo in `konnekta_plugins::all()`. Le azioni del plugin sono subito disponibili:
- nella UI tramite `pluginAction`;
- nella CLI: `konnekta action myplugin <azione> --args '{…}'`;
- come pulsante del **Mobile Deck** con l'azione `{"type":"plugin","plugin":"myplugin","action":"…"}`.

Nuovi plugin ereditano automaticamente cifratura, autenticazione e controllo dei permessi:
non esiste un percorso di rete che li aggiri.

## Flussi in tempo reale

Audio e video non passano sul canale di controllo: il produttore apre un **canale laterale**
(`DeviceHandle::listen_channel`), lo annuncia con un pacchetto `offer`, e il consumatore si
collega (`connect_channel`). Il canale usa TLS reciproco con lo stesso certificato fissato.
Su di esso viaggiano frame binari (`konnekta-media/src/frame.rs`).

Bassa latenza:
- `TCP_NODELAY`, coda di invio corta: se la rete non regge, i frame vecchi vengono scartati
  (mai accodati) e per il video si riparte dal keyframe successivo;
- **bitrate adattivo** (`abr.rs`): scende subito in caso di congestione, risale lentamente;
- jitter buffer audio di 60 ms con risincronizzazione automatica;
- encoder H.264 `zerolatency`, senza B-frame, GOP di 2 s.

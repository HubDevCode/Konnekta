'use strict';
// Konnekta desktop UI. Untrusted data (device names, notifications, file
// names...) is only ever inserted as text nodes, never as HTML.

const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const api = (method, params = {}) => invoke('api', { method, params });
const act = (plugin, action, args = null, deviceId = null) =>
  api('pluginAction', { plugin, action, args, deviceId });

// ---------------------------------------------------------------- helpers --

function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'style') el.style.cssText = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'value') el.value = v;
    else if (k === 'checked') el.checked = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return el;
}

const $main = document.getElementById('main');
const btn = (label, onclick, cls = '') => h('button', { class: `btn ${cls}`, onclick }, label);

function toast(msg, kind = '', actions = []) {
  const t = h('div', { class: `toast ${kind}` }, h('div', {}, msg),
    actions.length ? h('div', { class: 'row', style: 'margin-top:8px' },
      actions.map(([l, f]) => btn(l, () => { f(); t.remove(); }, 'small'))) : null);
  document.getElementById('toasts').append(t);
  setTimeout(() => t.remove(), actions.length ? 15000 : 5000);
}

async function run(promise, okMsg) {
  try {
    const r = await promise;
    if (okMsg) toast(okMsg);
    return r;
  } catch (e) {
    toast(String(e), 'err');
    throw e;
  }
}

function modal(title, body, buttons = []) {
  const root = document.getElementById('modal-root');
  const close = () => { root.innerHTML = ''; };
  const bg = h('div', { class: 'modal-bg', onclick: (e) => { if (e.target === bg) close(); } },
    h('div', { class: 'modal' },
      h('h1', {}, title), body,
      h('div', { class: 'row', style: 'justify-content:flex-end;margin-top:18px' },
        buttons.map(([l, f, cls]) => btn(l, async () => { if ((await f?.()) !== false) close(); }, cls)))));
  root.replaceChildren(bg);
  return close;
}

function sw(checked, onchange, disabled = false) {
  return h('label', { class: 'switch' },
    h('input', { type: 'checkbox', checked, disabled, onchange: (e) => onchange(e.target.checked) }),
    h('span'));
}

const fmtBytes = (n) => {
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
};

const clone = (o) => JSON.parse(JSON.stringify(o));

const typeIcon = (t) => ({ phone: '📱', tablet: '📟', laptop: '💻', desktop: '🖥️', tv: '📺' }[t] || '🔗');

// ------------------------------------------------------------------ state --

const S = {
  view: 'devices',
  me: null,
  devices: [],
  selected: null,
  tab: 'actions',
  transfers: new Map(),
  notifications: {},
  battery: {},
  phoneMedia: {},
  calls: {},
  streams: {},
  warnings: [],
  deck: null,
  deckProfile: null,
  deckPage: 0,
  deckCategory: '',
};

const hasCap = (d, cap) => (d?.incomingCapabilities || []).includes(cap);

async function refreshDevices() {
  S.devices = await api('devices');
  if (S.selected && !S.devices.find((d) => d.id === S.selected)) S.selected = null;
  if (!S.selected && S.devices.length) S.selected = S.devices[0].id;
  const online = S.devices.filter((d) => d.connected && d.paired).length;
  document.getElementById('status').textContent =
    `${online} dispositivo${online === 1 ? '' : 'i'} connesso${online === 1 ? '' : 'i'}`;
  if (S.view === 'devices') render();
}

// ----------------------------------------------------------------- events --

function onEvent(ev) {
  switch (ev.event) {
    case 'deviceConnected': case 'deviceUpdated': case 'deviceDisconnected':
    case 'paired': case 'unpaired':
      refreshDevices();
      if (ev.event === 'paired') {
        document.getElementById('modal-root').innerHTML = '';
        toast('Associazione completata. Controlla i permessi del dispositivo.');
      }
      break;
    case 'pairRequested':
      modal('Richiesta di associazione', h('div', {},
        h('p', {}, `"${ev.deviceName}" vuole associarsi a questo computer.`),
        h('p', { class: 'muted' }, 'Accetta solo se lo stesso codice è visualizzato sull\'altro dispositivo:'),
        h('div', { class: 'code' }, ev.verificationKey)),
      [['Rifiuta', () => api('rejectPair', { deviceId: ev.deviceId })],
       ['Accetta', () => run(api('acceptPair', { deviceId: ev.deviceId })), 'primary']]);
      break;
    case 'pairOutgoing':
      modal('Verifica il codice', h('div', {},
        h('p', {}, 'Conferma l\'associazione sull\'altro dispositivo solo se mostra questo codice:'),
        h('div', { class: 'code' }, ev.verificationKey)),
      [['Chiudi']]);
      break;
    case 'pairRejected':
      document.getElementById('modal-root').innerHTML = '';
      toast(`Associazione non riuscita: ${ev.reason}`, 'warn');
      refreshDevices();
      break;
    case 'securityWarning':
      S.warnings.unshift({ at: new Date(), ...ev });
      toast(ev.message, 'warn');
      break;
    case 'plugin':
      onPluginEvent(ev);
      break;
  }
}

function devName(id) {
  return S.devices.find((d) => d.id === id)?.name || 'Dispositivo';
}

function onPluginEvent({ deviceId, plugin, kind, data }) {
  const key = `${plugin}/${kind}`;
  switch (key) {
    case 'share/transfer':
      S.transfers.set(data.id, data);
      if (data.state === 'completed' && data.direction === 'incoming') {
        toast(`Ricevuto ${data.name}`);
      }
      if (data.state === 'failed') toast(`Trasferimento non riuscito: ${data.name} — ${data.error}`, 'err');
      if (S.view === 'transfers' || (S.view === 'devices' && S.tab === 'actions')) render();
      return;
    case 'share/text':
      toast(`Testo ricevuto da ${devName(deviceId)} (copiato negli appunti)`);
      return;
    case 'share/url':
      toast(`Link da ${devName(deviceId)}: ${data.url}`);
      return;
    case 'ping/received':
      toast(`Ping da ${devName(deviceId)}: ${data.message}`);
      return;
    case 'battery/status':
      S.battery[deviceId] = data;
      break;
    case 'notifications/posted':
      (S.notifications[deviceId] ||= {})[data.id] = data;
      break;
    case 'notifications/removed':
      delete S.notifications[deviceId]?.[data.id];
      break;
    case 'notifications/replyRequested':
      S.view = 'devices'; S.selected = deviceId; S.tab = 'notifications';
      render();
      replyModal(deviceId, S.notifications[deviceId]?.[data.id]);
      return;
    case 'telephony/call':
      S.calls[deviceId] = data;
      break;
    case 'telephony/ended':
      delete S.calls[deviceId];
      break;
    case 'mpris/remoteStatus':
      S.phoneMedia[deviceId] = { ...(S.phoneMedia[deviceId] || {}), ...data };
      break;
    case 'screenshot/received':
      toast(data.copied ? 'Screenshot ricevuto e copiato negli appunti' : 'Screenshot ricevuto');
      return;
    case 'runcommand/executed':
      if (!data.ok) toast(`Comando non riuscito: ${data.error}`, 'err');
      return;
    case 'deck/pressed':
      if (!data.ok) toast(`Pulsante Deck: ${data.error}`, 'err');
      return;
    default:
      if (['microphone', 'camera', 'phonescreen', 'desktop', 'systemaudio'].includes(plugin)) {
        const st = (S.streams[deviceId] ||= {});
        if (kind === 'started') st[plugin] = true;
        if (kind === 'stopped') st[plugin] = false;
        if (kind === 'error') toast(`${plugin}: ${data.error}`, 'err');
        if (kind === 'sharing') toast(data.message, 'warn');
      }
  }
  if (S.view === 'devices' && deviceId === S.selected) render();
}

// ------------------------------------------------------------------ views --

const views = {
  devices: renderDevices,
  transfers: renderTransfers,
  commands: renderCommands,
  deck: renderDeck,
  plugins: renderPlugins,
  settings: renderSettings,
  security: renderSecurity,
};

let renderSeq = 0;
async function render() {
  const seq = ++renderSeq;
  const el = await views[S.view]();
  if (seq === renderSeq) $main.replaceChildren(el);
}

document.getElementById('nav').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-view]');
  if (!b) return;
  S.view = b.dataset.view;
  for (const x of document.querySelectorAll('#nav button')) x.classList.toggle('active', x === b);
  render();
});

// --- devices ------------------------------------------------------------------

function deviceList() {
  if (!S.devices.length) {
    return h('div', { class: 'card empty' },
      h('p', {}, 'Nessun dispositivo trovato.'),
      h('p', { class: 'small' }, 'Apri Konnekta (o KDE Connect) sul telefono nella stessa rete Wi-Fi.'),
      connectBox());
  }
  return h('div', { class: 'card' },
    S.devices.map((d) => h('div', {
      class: `dev ${d.id === S.selected ? 'sel' : ''}`,
      onclick: () => { S.selected = d.id; render(); },
    },
    h('div', { class: 'icon' }, typeIcon(d.deviceType)),
    h('div', { class: 'grow' },
      h('div', { style: 'font-weight:600' }, d.name),
      h('div', { class: 'small muted row', style: 'gap:6px' },
        h('span', { class: `dot ${d.connected ? 'on' : ''}` }),
        d.connected ? 'Online' : 'Offline',
        S.battery[d.id] ? ` · 🔋 ${S.battery[d.id].currentCharge}%` : '')),
    d.paired ? h('span', { class: 'badge ok' }, 'Associato')
      : h('span', { class: 'badge warn' }, d.pairState === 'requested' ? 'In attesa' : 'Nuovo'))),
    h('div', { style: 'margin-top:10px' }, connectBox()));
}

function connectBox() {
  const ip = h('input', { type: 'text', placeholder: 'IP del dispositivo (es. 192.168.1.20)' });
  return h('div', { class: 'row', style: 'margin-top:8px' }, ip,
    btn('Connetti', () => run(api('connect', { address: ip.value }), 'Richiesta inviata')),
    btn('Aggiorna', () => run(api('refresh')).then(refreshDevices)));
}

async function renderDevices() {
  const d = S.devices.find((x) => x.id === S.selected);
  return h('div', {},
    h('h1', {}, 'Dispositivi'),
    h('div', { class: 'devices' }, deviceList(), d ? await deviceDetail(d) : h('div')));
}

const TABS = [
  ['actions', 'Azioni'], ['media', 'Media'], ['notifications', 'Notifiche'], ['calls', 'Chiamate'],
  ['streams', 'Audio e video'], ['permissions', 'Permessi'], ['plugins', 'Plugin'], ['info', 'Info'],
];

async function deviceDetail(d) {
  const head = h('div', { class: 'row' },
    h('div', { style: 'font-size:28px' }, typeIcon(d.deviceType)),
    h('div', { class: 'grow' },
      h('div', { style: 'font-size:18px;font-weight:700' }, d.name),
      h('div', { class: 'small muted' }, d.connected ? `Connesso · ${d.address}` : 'Non raggiungibile')),
    d.paired ? btn('Dissocia', () => modal('Dissociare il dispositivo?',
      h('p', {}, `${d.name} non potrà più comunicare con questo computer finché non verrà associato di nuovo.`),
      [['Annulla'], ['Dissocia', () => run(api('unpair', { deviceId: d.id })), 'danger']]), 'danger')
      : d.connected ? btn('Associa', () => run(api('pair', { deviceId: d.id })), 'primary') : null);

  if (!d.paired) {
    return h('div', { class: 'card' }, head,
      h('p', { class: 'muted' }, 'Il dispositivo non è associato: finché non lo associ e confermi il codice di verifica, non può scambiare alcun dato con questo computer.'),
      d.fingerprint ? h('p', { class: 'small' }, 'Impronta certificato: ', h('span', { class: 'mono' }, d.fingerprint)) : null);
  }

  const tabs = h('div', { class: 'tabs' }, TABS.map(([id, label]) =>
    h('button', { class: S.tab === id ? 'active' : '', onclick: () => { S.tab = id; render(); } }, label)));
  const body = d.connected || ['permissions', 'plugins', 'info'].includes(S.tab)
    ? await tabBody(d) : h('p', { class: 'muted' }, 'Il dispositivo è offline.');
  return h('div', { class: 'card' }, head, tabs, body);
}

async function tabBody(d) {
  switch (S.tab) {
    case 'actions': return actionsTab(d);
    case 'media': return mediaTab(d);
    case 'notifications': return notificationsTab(d);
    case 'calls': return callsTab(d);
    case 'streams': return streamsTab(d);
    case 'permissions': return permissionsTab(d);
    case 'plugins': return devicePluginsTab(d);
    case 'info': return infoTab(d);
  }
  return h('div');
}

function actionsTab(d) {
  const text = h('textarea', { placeholder: 'Testo o link da inviare…' });
  const transfers = [...S.transfers.values()].filter((t) => t.deviceId === d.id).slice(-5).reverse();
  return h('div', {},
    h('div', { class: 'row' },
      btn('📁 Invia file…', async () => {
        const paths = await invoke('pick_files');
        if (paths.length) await run(act('share', 'sendFiles', { paths }, d.id), 'Invio avviato');
      }, 'primary'),
      btn('📋 Invia appunti', () => run(act('clipboard', 'send', null, d.id), 'Appunti inviati')),
      btn('🔔 Ping', () => run(act('ping', 'send', { message: 'Ping da Konnekta' }, d.id))),
      hasCap(d, 'kdeconnect.findmyphone.request') ? btn('📳 Fai squillare', () => run(act('findmyphone', 'ring', null, d.id))) : null),
    h('h3', {}, 'Condividi testo'),
    text,
    h('div', { class: 'row', style: 'margin-top:6px' },
      btn('Invia testo', () => run(act('share', 'sendText', { text: text.value }, d.id), 'Inviato')),
      btn('Invia come link', () => run(act('share', 'sendUrl', { url: text.value.trim() }, d.id), 'Inviato'))),
    hasCap(d, 'kdeconnect.runcommand') ? remoteCommands(d) : null,
    transfers.length ? h('div', {}, h('h3', {}, 'Trasferimenti recenti'), transfers.map(transferItem)) : null);
}

function remoteCommands(d) {
  const box = h('div', {}, h('h3', {}, 'Comandi del dispositivo'));
  act('runcommand', 'remoteList', null, d.id).then((list) => {
    const entries = Object.entries(list || {});
    box.append(entries.length
      ? h('div', { class: 'row' }, entries.map(([key, c]) =>
        btn(c.name, () => run(act('runcommand', 'runRemote', { key }, d.id)))))
      : h('p', { class: 'small muted' }, 'Nessun comando esposto da questo dispositivo.'));
  }).catch(() => {});
  return box;
}

function mediaTab(d) {
  const m = S.phoneMedia[d.id] || {};
  const control = (action) => run(act('mpris', 'remoteControl', { action, player: m.player }, d.id));
  const vol = h('input', { type: 'range', min: 0, max: 100, value: m.volume ?? 50,
    onchange: (e) => act('mpris', 'remoteVolume', { volume: +e.target.value, player: m.player }, d.id) });
  return h('div', {},
    h('p', { class: 'muted small' }, 'Controlla ciò che è in riproduzione sul telefono. Dal telefono puoi controllare i lettori di questo computer.'),
    h('div', { class: 'card', style: 'background:var(--panel-2)' },
      h('div', { style: 'font-weight:600;font-size:16px' }, m.title || 'Niente in riproduzione'),
      h('div', { class: 'muted' }, [m.artist, m.album].filter(Boolean).join(' — ')),
      h('div', { class: 'small muted' }, m.player || ''),
      h('div', { class: 'row', style: 'margin-top:12px' },
        btn('⏮', () => control('Previous')),
        btn(m.isPlaying ? '⏸' : '▶', () => control('PlayPause'), 'primary'),
        btn('⏭', () => control('Next')),
        h('span', { class: 'small muted' }, '🔊'), vol)),
    h('div', { class: 'row', style: 'margin-top:8px' },
      btn('Aggiorna', () => run(act('mpris', 'remoteRefresh', null, d.id)))));
}

function replyModal(deviceId, n) {
  if (!n) return;
  const ta = h('textarea', { placeholder: 'Scrivi una risposta…' });
  modal(`Rispondi — ${n.appName}`, h('div', {}, h('p', { class: 'muted' }, n.title), ta),
    [['Annulla'], ['Invia', () => run(act('notifications', 'reply', { id: n.id, message: ta.value }, deviceId), 'Risposta inviata'), 'primary']]);
}

function notificationsTab(d) {
  const list = Object.values(S.notifications[d.id] || {});
  if (!list.length) return h('p', { class: 'muted' }, 'Nessuna notifica dal dispositivo.');
  return h('div', { class: 'list' }, list.map((n) => h('div', { class: 'item' },
    h('div', { class: 'notif grow' },
      h('div', { class: 'app' }, n.appName),
      h('div', { style: 'font-weight:600' }, n.title),
      h('div', {}, n.text)),
    h('div', { class: 'row' },
      n.requestReplyId ? btn('Rispondi', () => replyModal(d.id, n), 'small') : null,
      (n.actions || []).map((a) => btn(a, () => run(act('notifications', 'action', { id: n.id, action: a }, d.id)), 'small')),
      n.isClearable ? btn('Ignora', () => run(act('notifications', 'dismiss', { id: n.id }, d.id)).then(() => {
        delete S.notifications[d.id][n.id]; render();
      }), 'small') : null))));
}

function callsTab(d) {
  const c = S.calls[d.id];
  const ctrl = hasCap(d, 'konnekta.telephony.action');
  return h('div', {},
    c ? h('div', { class: 'card', style: 'background:var(--panel-2)' },
      h('div', { style: 'font-size:16px;font-weight:600' },
        c.event === 'ringing' ? '📞 Chiamata in arrivo' : c.event === 'talking' ? '📞 In chiamata' : 'Chiamata persa'),
      h('div', {}, c.caller),
      h('div', { class: 'row', style: 'margin-top:10px' },
        ctrl && c.event === 'ringing' ? btn('Rispondi', () => run(act('telephony', 'accept', null, d.id)), 'primary') : null,
        ctrl && c.event === 'ringing' ? btn('Rifiuta', () => run(act('telephony', 'reject', null, d.id)), 'danger') : null,
        ctrl && c.event === 'talking' ? btn('Riaggancia', () => run(act('telephony', 'hangup', null, d.id)), 'danger') : null,
        btn('Silenzia suoneria', () => run(act('telephony', 'mute', null, d.id)))))
      : h('p', { class: 'muted' }, 'Nessuna chiamata in corso.'),
    !ctrl ? h('p', { class: 'small muted' }, 'Questo telefono permette solo di silenziare la suoneria (iOS non consente ad altre app di rispondere o rifiutare le chiamate).') : null);
}

function streamsTab(d) {
  const st = S.streams[d.id] || {};
  const row = (plugin, title, desc, startable, extra) => {
    const supported = hasCap(d, `konnekta.${plugin}`);
    return h('div', { class: 'perm' },
      h('div', { class: 'grow' },
        h('div', { style: 'font-weight:600' }, title, ' ', st[plugin] ? h('span', { class: 'badge ok' }, 'attivo') : null),
        h('div', { class: 'small muted' }, supported ? desc : 'Non supportato dal dispositivo (serve l\'app Konnekta).')),
      extra || null,
      supported ? (st[plugin]
        ? btn('Interrompi', () => run(act(plugin, 'stop', null, d.id)), 'danger')
        : startable ? btn('Avvia', () => run(act(plugin, 'start', extraArgs(plugin), d.id)), 'primary')
          : h('span', { class: 'small muted' }, 'Avvia dal telefono')) : null);
  };
  const facing = h('select', {}, h('option', { value: 'back' }, 'Posteriore'), h('option', { value: 'front' }, 'Anteriore'));
  const res = h('select', {}, h('option', { value: '720' }, '720p'), h('option', { value: '1080' }, '1080p'), h('option', { value: '480' }, '480p'));
  function extraArgs(plugin) {
    if (plugin === 'camera') {
      const hgt = +res.value;
      return { codec: 'h264', height: hgt, width: Math.round(hgt * 16 / 9), fps: 30, facing: facing.value };
    }
    if (plugin === 'microphone') return { codec: 'pcm_s16le', sampleRate: 48000, channels: 1 };
    return { codec: 'h264' };
  }
  return h('div', {},
    h('p', { class: 'small muted' }, 'Tutti i flussi sono cifrati end-to-end con TLS e autenticati con il certificato del dispositivo associato.'),
    row('microphone', '🎙️ Microfono del telefono', 'Diventa il dispositivo di ingresso "Konnekta Microphone" (Discord, Zoom, OBS…).', true),
    row('camera', '📷 Webcam wireless', 'Usa la fotocamera del telefono come webcam del computer.', true, h('div', { class: 'row' }, facing, res)),
    row('phonescreen', '📲 Schermo del telefono', 'Mostra lo schermo del telefono in una finestra.', true),
    row('desktop', '🖥️ Desktop remoto', 'Il telefono vede e controlla questo schermo (servono i permessi Desktop remoto e Mouse e tastiera).', false),
    row('systemaudio', '🔈 Audio del computer sul telefono', 'Ascolta sul telefono l\'audio riprodotto da questo computer.', false));
}

async function permissionsTab(d) {
  const perms = await api('permissions');
  return h('div', {},
    h('p', { class: 'small muted' }, 'Scegli cosa può fare questo dispositivo con il computer. Le autorizzazioni sensibili sono disattivate per impostazione predefinita.'),
    perms.map((p) => h('div', { class: 'perm' },
      h('div', {}, h('div', {}, p.label, ' ', p.sensitive ? h('span', { class: 'badge warn' }, 'sensibile') : null)),
      sw(d.permissions.includes(p.id), (v) => run(api('setPermission', { deviceId: d.id, permission: p.id, granted: v }))))));
}

async function devicePluginsTab(d) {
  const plugins = await api('plugins');
  return h('div', {},
    h('p', { class: 'small muted' }, 'Disattiva singole funzioni solo per questo dispositivo.'),
    plugins.map((p) => h('div', { class: 'perm' },
      h('div', {}, p.name, ' ', h('span', { class: `badge ${p.origin === 'konnekta' ? 'k' : ''}` }, p.origin === 'konnekta' ? 'Konnekta' : 'KDE Connect')),
      sw(!d.disabledPlugins.includes(p.id), (v) => run(api('setDevicePlugin', { deviceId: d.id, plugin: p.id, enabled: v }))))));
}

function infoTab(d) {
  return h('div', {},
    h('p', {}, 'ID: ', h('span', { class: 'mono' }, d.id)),
    h('p', {}, 'Impronta del certificato (SHA-256): ', h('div', { class: 'mono' }, d.fingerprint || '—')),
    h('p', { class: 'small muted' }, `Capacità in ingresso: ${d.incomingCapabilities.length} · in uscita: ${d.outgoingCapabilities.length}`),
    h('p', { class: 'small muted' }, 'Il certificato è fissato (pinning): se cambia, la connessione viene rifiutata.'));
}

// --- transfers ----------------------------------------------------------------

const STATE_LABEL = { inProgress: 'In corso', completed: 'Completato', interrupted: 'Interrotto', failed: 'Non riuscito', cancelled: 'Annullato' };

function transferItem(t) {
  const pct = t.size ? Math.round((t.transferred / t.size) * 100) : 0;
  return h('div', { class: 'item' },
    h('div', { style: 'font-size:20px' }, t.direction === 'incoming' ? '⬇️' : '⬆️'),
    h('div', { class: 'grow' },
      h('div', { class: 'row' }, h('b', {}, t.name),
        h('span', { class: `badge ${t.state === 'completed' ? 'ok' : t.state === 'inProgress' ? 'k' : 'warn'}` }, STATE_LABEL[t.state] || t.state),
        t.resumed ? h('span', { class: 'badge' }, 'ripreso') : null),
      h('div', { class: 'small muted' }, `${devName(t.deviceId)} · ${fmtBytes(t.transferred)} / ${fmtBytes(t.size)}`, t.error ? ` · ${t.error}` : ''),
      t.state === 'inProgress' ? h('div', { class: 'progress', style: 'margin-top:6px' }, h('div', { style: `width:${pct}%` })) : null),
    t.direction === 'outgoing' && ['failed', 'interrupted'].includes(t.state)
      ? btn('Riprova', () => run(act('share', 'retry', { id: t.id }, t.deviceId)), 'small') : null);
}

async function renderTransfers() {
  try {
    for (const t of await act('share', 'transfers')) S.transfers.set(t.id, t);
  } catch { /* plugin disabled */ }
  const list = [...S.transfers.values()].sort((a, b) => b.startedAt - a.startedAt);
  return h('div', {},
    h('div', { class: 'row' }, h('h1', { class: 'grow' }, 'Trasferimenti'),
      btn('Pulisci completati', () => run(act('share', 'clearFinished')).then(() => { S.transfers.clear(); render(); }))),
    list.length ? h('div', { class: 'list' }, list.map(transferItem))
      : h('div', { class: 'card empty' }, 'Nessun trasferimento. Trascina i file dal telefono o usa "Invia file" in Dispositivi.'));
}

// --- actions editor (commands & deck) -------------------------------------------

const ACTION_TYPES = {
  shell: ['Comando shell', [['command', 'Comando', 'text'], ['timeoutSecs', 'Timeout (secondi)', 'number']]],
  launchApp: ['Avvia applicazione', [['app', 'Applicazione (nome, percorso o .desktop)', 'text'], ['args', 'Argomenti (separati da spazio)', 'args']]],
  open: ['Apri file / cartella / URL', [['target', 'Percorso o URL', 'text']]],
  shortcut: ['Scorciatoia da tastiera', [['keys', 'Tasti, es. ctrl+shift+t', 'text']]],
  typeText: ['Scrivi testo', [['text', 'Testo', 'text']]],
  media: ['Controllo multimediale', [['action', 'Azione', ['PlayPause', 'Play', 'Pause', 'Stop', 'Next', 'Previous']]]],
  volume: ['Volume', [['set', 'Imposta a (0-100)', 'number'], ['delta', 'Oppure varia di (±)', 'number']]],
  delay: ['Attesa', [['ms', 'Millisecondi', 'number']]],
  command: ['Esegui comando salvato', [['id', 'Comando', 'command']]],
  plugin: ['Azione di un plugin', [['plugin', 'Plugin (es. dnd)', 'text'], ['action', 'Azione (es. toggle)', 'text'], ['args', 'Argomenti JSON', 'json']]],
};

let commandsCache = [];

function actionsEditor(actions) {
  const box = h('div');
  const draw = () => {
    box.replaceChildren(
      ...actions.map((a, i) => {
        const [, fields] = ACTION_TYPES[a.type] || ACTION_TYPES.shell;
        const typeSel = h('select', { onchange: (e) => { actions[i] = { type: e.target.value }; draw(); } },
          Object.entries(ACTION_TYPES).map(([k, [l]]) => h('option', { value: k, selected: k === a.type }, l)));
        const inputs = fields.map(([key, label, kind]) => {
          let input;
          if (Array.isArray(kind)) {
            input = h('select', { onchange: (e) => { a[key] = e.target.value; } },
              kind.map((o) => h('option', { value: o, selected: a[key] === o }, o)));
            a[key] ??= kind[0];
          } else if (kind === 'command') {
            input = h('select', { onchange: (e) => { a[key] = e.target.value; } },
              h('option', { value: '' }, '— scegli —'),
              commandsCache.map((c) => h('option', { value: c.id, selected: a[key] === c.id }, c.name)));
          } else if (kind === 'args') {
            input = h('input', { type: 'text', value: (a[key] || []).join(' '),
              oninput: (e) => { a[key] = e.target.value.split(' ').filter(Boolean); } });
          } else if (kind === 'json') {
            input = h('input', { type: 'text', value: a[key] == null ? '' : JSON.stringify(a[key]),
              oninput: (e) => { try { a[key] = e.target.value ? JSON.parse(e.target.value) : null; } catch { /* keep typing */ } } });
          } else {
            input = h('input', { type: kind === 'number' ? 'number' : 'text', value: a[key] ?? '',
              oninput: (e) => {
                const v = e.target.value;
                if (kind === 'number') { if (v === '') delete a[key]; else a[key] = Number(v); } else a[key] = v;
              } });
          }
          return h('label', { class: 'field' }, label, input);
        });
        return h('div', { class: 'action-row' }, typeSel, h('div', { class: 'action-fields' }, inputs),
          h('div', { class: 'row', style: 'flex-direction:column;gap:4px' },
            btn('↑', () => { if (i) { [actions[i - 1], actions[i]] = [actions[i], actions[i - 1]]; draw(); } }, 'small'),
            btn('↓', () => { if (i < actions.length - 1) { [actions[i + 1], actions[i]] = [actions[i], actions[i + 1]]; draw(); } }, 'small'),
            btn('✕', () => { actions.splice(i, 1); draw(); }, 'small danger')));
      }),
      btn('+ Aggiungi azione', () => { actions.push({ type: 'shortcut', keys: '' }); draw(); }, 'small'));
  };
  draw();
  return box;
}

// --- commands -------------------------------------------------------------------

function commandModal(cmd) {
  const c = clone(cmd || { name: '', description: '', icon: '', group: '', notify: false, actions: [{ type: 'shell', command: '' }] });
  const f = (key, label, ph = '') => h('label', { class: 'field' }, label,
    h('input', { type: 'text', value: c[key] || '', placeholder: ph, oninput: (e) => { c[key] = e.target.value; } }));
  modal(cmd ? 'Modifica comando' : 'Nuovo comando', h('div', {},
    h('div', { class: 'grid2' }, f('name', 'Nome', 'Blocca schermo'), f('group', 'Gruppo', 'Sistema'),
      f('icon', 'Icona (emoji o nome)', '🔒'), f('description', 'Descrizione')),
    h('label', { class: 'check', style: 'margin:10px 0' },
      h('input', { type: 'checkbox', checked: c.notify, onchange: (e) => { c.notify = e.target.checked; } }),
      'Mostra una notifica con l\'esito'),
    h('h3', {}, 'Azioni (eseguite in sequenza)'),
    actionsEditor(c.actions),
    h('p', { class: 'small muted' }, 'I dispositivi associati ricevono solo nome e descrizione e possono avviare il comando solo con il permesso "Run commands".')),
  [['Annulla'], ['Salva', async () => {
    await run(act('runcommand', cmd ? 'update' : 'add', c), 'Comando salvato');
    render();
  }, 'primary']]);
}

async function renderCommands() {
  commandsCache = await act('runcommand', 'list').catch(() => []);
  const groups = {};
  for (const c of commandsCache) (groups[c.group || 'Altro'] ||= []).push(c);
  return h('div', {},
    h('div', { class: 'row' }, h('h1', { class: 'grow' }, 'Comandi personalizzati'), btn('+ Nuovo comando', () => commandModal(), 'primary')),
    h('p', { class: 'muted' }, 'Comandi shell, avvio di applicazioni, scorciatoie e macro richiamabili dal telefono (compatibile con "Esegui comando" di KDE Connect) o dal Mobile Deck.'),
    commandsCache.length ? Object.entries(groups).map(([g, list]) => h('div', {},
      h('h3', {}, g),
      h('div', { class: 'list' }, list.map((c) => h('div', { class: 'item' },
        h('div', { style: 'font-size:20px;width:28px;text-align:center' }, c.icon || '⌘'),
        h('div', { class: 'grow' }, h('b', {}, c.name), h('div', { class: 'small muted' }, c.description || `${c.actions.length} azioni`)),
        btn('▶ Esegui', () => run(act('runcommand', 'run', { id: c.id }), 'Eseguito'), 'small'),
        btn('Modifica', () => commandModal(c), 'small'),
        btn('Elimina', () => run(act('runcommand', 'delete', { id: c.id })).then(render), 'small danger'))))))
      : h('div', { class: 'card empty' }, 'Nessun comando. Creane uno per controllare il computer dal telefono.'));
}

// --- deck -------------------------------------------------------------------------

const ICONS = {
  play_pause: '⏯', skip_previous: '⏮', skip_next: '⏭', volume: '🔊', volume_off: '🔇', dnd: '🌙',
  content_copy: '📋', content_paste: '📥', search: '🔍', tab: '⇥', looks_one: '1', looks_two: '2', looks_3: '3',
  sensors: '🔴', fiber_manual_record: '⏺', mic_off: '🎙', arrow_back: '◀', arrow_forward: '▶', play_arrow: '▶',
  dark_mode: '⬛', stop: '⏹', screenshot: '📸', headset_off: '🎧', record_voice_over: '🗣', work: '💼',
  videocam: '🎥', slideshow: '📽', sports_esports: '🎮',
};
const iconOf = (name) => ICONS[name] || name || '';

function deckButtonModal(profile, page, pos, existing) {
  const b = clone(existing || { id: '', position: pos, label: '', icon: '', color: '#3b4252', category: '', actions: [], activeActions: [] });
  b.activeActions ||= [];
  const stateSel = h('select', { onchange: (e) => {
    const v = e.target.value;
    b.state = v ? (v === 'shell' ? { type: 'shell', command: '', intervalSecs: 5 } : { type: v }) : undefined;
    shellBox.style.display = v === 'shell' ? '' : 'none';
  } },
  [['', 'Nessuno'], ['mediaPlaying', 'Riproduzione in corso'], ['doNotDisturb', 'Non disturbare attivo'], ['volume', 'Mostra volume'], ['shell', 'Comando shell (exit 0 = attivo)']]
    .map(([v, l]) => h('option', { value: v, selected: (b.state?.type || '') === v }, l)));
  const shellBox = h('label', { class: 'field', style: b.state?.type === 'shell' ? '' : 'display:none' }, 'Comando di stato',
    h('input', { type: 'text', value: b.state?.command || '', oninput: (e) => { if (b.state) b.state.command = e.target.value; } }));
  const f = (key, label, type = 'text') => h('label', { class: 'field' }, label,
    h('input', { type, value: b[key] || '', oninput: (e) => { b[key] = e.target.value; } }));
  modal(existing ? 'Modifica pulsante' : 'Nuovo pulsante', h('div', {},
    h('div', { class: 'grid2' }, f('label', 'Etichetta'), f('icon', 'Icona (emoji o nome)'), f('category', 'Categoria'), f('color', 'Colore', 'color')),
    h('h3', {}, 'Azioni / macro'),
    actionsEditor(b.actions),
    h('h3', {}, 'Stato dinamico (aggiornato dal computer)'),
    h('div', { class: 'grid2' }, h('label', { class: 'field' }, 'Sorgente', stateSel), shellBox, f('activeColor', 'Colore quando attivo', 'color')),
    h('h3', {}, 'Azioni alternative quando lo stato è attivo (toggle, opzionale)'),
    actionsEditor(b.activeActions)),
  [
    existing ? ['Elimina', () => {
      page.buttons = page.buttons.filter((x) => x !== existing);
      render();
    }, 'danger'] : null,
    ['Annulla'],
    ['Applica', () => {
      if (!b.label) { toast('Inserisci un\'etichetta', 'err'); return false; }
      if (!b.id) b.id = Math.random().toString(16).slice(2, 14);
      if (!b.state) delete b.state;
      if (existing) Object.assign(existing, b); else page.buttons.push(b);
      render();
      return true;
    }, 'primary'],
  ].filter(Boolean));
}

async function renderDeck() {
  if (!S.deck) S.deck = await act('deck', 'get');
  const deck = S.deck;
  let prof = deck.profiles.find((p) => p.id === S.deckProfile) || deck.profiles.find((p) => p.id === deck.activeProfile) || deck.profiles[0];
  S.deckProfile = prof.id;
  if (S.deckPage >= prof.pages.length) S.deckPage = 0;
  const page = prof.pages[S.deckPage];
  const cats = [...new Set(prof.pages.flatMap((p) => p.buttons.map((b) => b.category)).filter(Boolean))];

  const save = () => run(act('deck', 'save', deck), 'Deck salvato e inviato ai telefoni').then(() => { S.deck = null; render(); });
  const grid = page ? h('div', { class: 'deck-grid', style: `grid-template-columns:repeat(${prof.columns},minmax(70px,110px))` },
    Array.from({ length: prof.columns * prof.rows }, (_, pos) => {
      const b = page.buttons.find((x) => x.position === pos);
      const dim = S.deckCategory && b && b.category !== S.deckCategory;
      return b
        ? h('div', { class: 'deck-cell filled', style: `background:${b.color};${dim ? 'opacity:.3' : ''}`, onclick: () => deckButtonModal(prof, page, pos, b) },
          h('div', { class: 'ic' }, iconOf(b.icon)), b.label, b.state ? h('div', { class: 'small' }, '●') : null)
        : h('div', { class: 'deck-cell', onclick: () => deckButtonModal(prof, page, pos) }, '+');
    })) : h('p', {}, 'Nessuna pagina.');

  return h('div', {},
    h('div', { class: 'row' }, h('h1', { class: 'grow' }, 'Mobile Deck'), btn('💾 Salva', save, 'primary')),
    h('p', { class: 'muted' }, 'Il telefono diventa un pannello di controllo tipo Stream Deck. Il telefono riceve solo aspetto e disposizione dei pulsanti: le azioni restano su questo computer.'),
    h('div', { class: 'card' },
      h('div', { class: 'row' },
        h('select', { onchange: (e) => { S.deckProfile = e.target.value; S.deckPage = 0; render(); } },
          deck.profiles.map((p) => h('option', { value: p.id, selected: p.id === prof.id }, `${iconOf(p.icon)} ${p.name}${p.id === deck.activeProfile ? ' (attivo)' : ''}`))),
        btn('Imposta attivo', () => { deck.activeProfile = prof.id; save(); }),
        btn('+ Profilo', () => {
          const id = Math.random().toString(16).slice(2, 14);
          deck.profiles.push({ id, name: 'Nuovo profilo', icon: '', columns: 4, rows: 3, pages: [{ id: 'p1', name: 'Pagina 1', buttons: [] }] });
          S.deckProfile = id; render();
        }),
        btn('Duplica', () => run(act('deck', 'duplicateProfile', { id: prof.id }), 'Profilo duplicato').then(() => { S.deck = null; render(); })),
        btn('Elimina', () => run(act('deck', 'deleteProfile', { id: prof.id })).then(() => { S.deck = null; S.deckProfile = null; render(); }), 'danger')),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('label', { class: 'field grow' }, 'Nome profilo', h('input', { type: 'text', value: prof.name, oninput: (e) => { prof.name = e.target.value; } })),
        h('label', { class: 'field' }, 'Icona', h('input', { type: 'text', value: prof.icon, oninput: (e) => { prof.icon = e.target.value; } })),
        h('label', { class: 'field' }, 'Colonne', h('input', { type: 'number', min: 1, max: 12, value: prof.columns, onchange: (e) => { prof.columns = +e.target.value; render(); } })),
        h('label', { class: 'field' }, 'Righe', h('input', { type: 'number', min: 1, max: 12, value: prof.rows, onchange: (e) => { prof.rows = +e.target.value; render(); } }))),
      h('div', { class: 'tabs' },
        prof.pages.map((p, i) => h('button', { class: i === S.deckPage ? 'active' : '', onclick: () => { S.deckPage = i; render(); } }, p.name)),
        h('button', { onclick: () => { prof.pages.push({ id: Math.random().toString(16).slice(2, 10), name: `Pagina ${prof.pages.length + 1}`, buttons: [] }); S.deckPage = prof.pages.length - 1; render(); } }, '+')),
      page ? h('div', { class: 'row' },
        h('label', { class: 'field' }, 'Nome pagina', h('input', { type: 'text', value: page.name, oninput: (e) => { page.name = e.target.value; } })),
        prof.pages.length > 1 ? btn('Elimina pagina', () => { prof.pages.splice(S.deckPage, 1); S.deckPage = 0; render(); }, 'small danger') : null,
        cats.length ? h('label', { class: 'field' }, 'Evidenzia categoria',
          h('select', { onchange: (e) => { S.deckCategory = e.target.value; render(); } },
            h('option', { value: '' }, 'Tutte'), cats.map((c) => h('option', { value: c, selected: c === S.deckCategory }, c)))) : null) : null,
      grid,
      h('div', { class: 'row' },
        btn('Ripristina profili predefiniti', () => modal('Ripristinare?', h('p', {}, 'Tutti i profili verranno sostituiti con quelli predefiniti.'),
          [['Annulla'], ['Ripristina', () => run(act('deck', 'reset')).then(() => { S.deck = null; render(); }), 'danger']]), 'small'),
        h('span', { class: 'small muted' }, 'Le modifiche vengono applicate quando premi Salva.'))));
}

// --- plugins ----------------------------------------------------------------------

async function renderPlugins() {
  const plugins = await api('plugins');
  const section = (origin, title, desc) => h('div', {},
    h('h2', {}, title), h('p', { class: 'small muted' }, desc),
    h('div', { class: 'card' }, plugins.filter((p) => p.origin === origin).map((p) => h('div', { class: 'perm' },
      h('div', {}, h('div', { style: 'font-weight:600' }, p.name), h('div', { class: 'small muted' }, p.description)),
      sw(p.enabled, (v) => run(api('setPluginEnabled', { plugin: p.id, enabled: v })))))));
  return h('div', {}, h('h1', {}, 'Plugin'),
    section('kdeConnect', 'Compatibili con KDE Connect', 'Funzionano anche con KDE Connect, GSConnect e altre app compatibili.'),
    section('konnekta', 'Estensioni Konnekta', 'Funzioni avanzate disponibili tra dispositivi con Konnekta.'));
}

// --- settings ---------------------------------------------------------------------

async function renderSettings() {
  const cfg = await api('getConfig');
  const get = (p) => act(p, 'getConfig').catch(() => null);
  const [share, clip, shot, filter, dnd, pres, tel] = await Promise.all([
    get('share'), get('clipboard'), get('screenshot'), act('notifications', 'getFilter').catch(() => null), get('dnd'), get('presenter'), get('telephony')]);
  const general = { deviceName: cfg.device_name, downloadDir: cfg.download_dir, announce: cfg.announce };
  const txt = (o, k, label) => h('label', { class: 'field' }, label, h('input', { type: 'text', value: o[k] ?? '', oninput: (e) => { o[k] = e.target.value; } }));
  const chk = (o, k, label) => h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: o[k], onchange: (e) => { o[k] = e.target.checked; } }), label);
  const folder = (o, k, label) => {
    const input = h('input', { type: 'text', value: o[k] ?? '', oninput: (e) => { o[k] = e.target.value || null; } });
    return h('label', { class: 'field' }, label, h('div', { class: 'row' }, h('div', { class: 'grow' }, input),
      btn('Sfoglia…', async () => { const p = await invoke('pick_folder'); if (p) { o[k] = p; input.value = p; } }, 'small')));
  };
  const card = (title, body, onSave) => h('div', { class: 'card', style: 'margin-bottom:14px' }, h('h2', { style: 'margin-top:0' }, title), body,
    h('div', { class: 'row', style: 'justify-content:flex-end;margin-top:10px' }, btn('Salva', () => run(onSave(), 'Impostazioni salvate'), 'primary')));
  const list = (arr) => (arr || []).join(', ');
  const fl = filter && { ...filter, blocked: list(filter.blockedApps), allowed: list(filter.allowedApps), muted: list(filter.mutedKeywords) };
  const split = (s) => s.split(',').map((x) => x.trim()).filter(Boolean);

  return h('div', {}, h('h1', {}, 'Impostazioni'),
    card('Generale', h('div', { class: 'list' },
      txt(general, 'deviceName', 'Nome di questo computer'),
      folder(general, 'downloadDir', 'Cartella predefinita per i file ricevuti'),
      chk(general, 'announce', 'Annuncia questo computer sulla rete locale (disattiva sulle reti pubbliche: la connessione manuale via IP resta disponibile)')),
    () => api('setConfig', general)),
    share && card('File', h('div', { class: 'list' },
      folder(share, 'destination', 'Cartella di destinazione (vuoto = predefinita)'),
      chk(share, 'textToClipboard', 'Copia negli appunti il testo ricevuto')),
    () => act('share', 'setConfig', share)),
    clip && card('Appunti', h('div', { class: 'list' },
      chk(clip, 'autoSend', 'Invia automaticamente ai dispositivi ciò che copio'),
      chk(clip, 'autoReceive', 'Ricevi automaticamente gli appunti dei dispositivi')),
    () => act('clipboard', 'setConfig', clip)),
    shot && card('Screenshot del telefono', h('div', { class: 'list' },
      chk(shot, 'autoTransfer', 'Trasferisci automaticamente gli screenshot dal telefono'),
      chk(shot, 'save', 'Salva gli screenshot ricevuti'),
      folder(shot, 'folder', `Cartella (vuoto = ${shot.effectiveFolder})`),
      chk(shot, 'copyToClipboard', 'Copia automaticamente lo screenshot negli appunti')),
    () => { const { effectiveFolder, ...c } = shot; return act('screenshot', 'setConfig', c); }),
    fl && card('Notifiche', h('div', { class: 'list' },
      chk(fl, 'showPopups', 'Mostra le notifiche del telefono come notifiche del computer'),
      chk(fl, 'privacyMode', 'Modalità privacy: nascondi titolo e contenuto'),
      chk(fl, 'hideSilent', 'Ignora le notifiche silenziose'),
      txt(fl, 'blocked', 'App bloccate (separate da virgola)'),
      txt(fl, 'allowed', 'Solo queste app (vuoto = tutte)'),
      txt(fl, 'muted', 'Parole da filtrare')),
    () => act('notifications', 'setFilter', { showPopups: fl.showPopups, privacyMode: fl.privacyMode, hideSilent: fl.hideSilent,
      blockedApps: split(fl.blocked), allowedApps: split(fl.allowed), mutedKeywords: split(fl.muted) })),
    dnd && card('Non disturbare', h('div', { class: 'list' },
      chk(dnd, 'sync', 'Sincronizza la modalità Non disturbare tra telefono e computer'),
      h('label', { class: 'field' }, 'In caso di modifiche simultanee vince',
        h('select', { onchange: (e) => { dnd.priority = e.target.value; } },
          [['latest', 'La modifica più recente'], ['phone', 'Il telefono'], ['computer', 'Il computer']]
            .map(([v, l]) => h('option', { value: v, selected: dnd.priority === v }, l))))),
    () => act('dnd', 'setConfig', dnd)),
    pres && card('Presentazioni', h('label', { class: 'field' }, 'Programma di presentazione',
      h('select', { onchange: (e) => { pres.program = e.target.value; } },
        Object.keys(pres.keymaps).map((k) => h('option', { value: k, selected: pres.program === k }, k)))),
    () => act('presenter', 'setConfig', pres)),
    tel && card('Chiamate', h('div', { class: 'list' },
      chk(tel, 'pauseMedia', 'Metti in pausa la musica durante le chiamate'),
      chk(tel, 'showNumber', 'Mostra il numero del chiamante')),
    () => act('telephony', 'setConfig', tel)));
}

// --- security ---------------------------------------------------------------------

async function renderSecurity() {
  const me = await api('info');
  return h('div', {}, h('h1', {}, 'Sicurezza'),
    h('div', { class: 'card' },
      h('h2', { style: 'margin-top:0' }, 'Questo computer'),
      h('p', {}, 'ID dispositivo: ', h('span', { class: 'mono' }, me.deviceId)),
      h('p', {}, 'Impronta del certificato (SHA-256):'), h('div', { class: 'mono' }, me.fingerprint),
      h('p', { class: 'small muted' }, `Protocollo KDE Connect v${me.protocolVersion} · Konnekta ${me.version}`)),
    h('div', { class: 'card', style: 'margin-top:14px' },
      h('h2', { style: 'margin-top:0' }, 'Come sono protetti i tuoi dati'),
      h('ul', {},
        h('li', {}, 'Tutto il traffico (pacchetti, file, appunti, notifiche, audio, video e desktop remoto) è cifrato con TLS 1.3/1.2 e cifrari AEAD con forward secrecy.'),
        h('li', {}, 'Autenticazione reciproca: ogni dispositivo presenta il proprio certificato; dopo l\'associazione il certificato è fissato e qualsiasi altro viene rifiutato (protezione man-in-the-middle).'),
        h('li', {}, 'L\'associazione richiede la conferma su entrambi i dispositivi e il confronto di un codice di verifica derivato dalle chiavi di entrambi.'),
        h('li', {}, 'Nessuna fiducia nella rete locale: un dispositivo non associato può solo chiedere l\'associazione.'),
        h('li', {}, 'Permessi granulari per dispositivo; le funzioni sensibili sono disattivate per impostazione predefinita.'),
        h('li', {}, 'I comandi remoti sono richiamabili solo per identificativo: il telefono non può inviare comandi arbitrari.'))),
    h('h2', {}, 'Avvisi di sicurezza'),
    S.warnings.length ? h('div', { class: 'list' }, S.warnings.map((w) => h('div', { class: 'item' },
      h('div', { class: 'grow' }, h('div', {}, w.message), h('div', { class: 'small muted' }, `${w.at.toLocaleString()}${w.deviceId ? ` · ${devName(w.deviceId)}` : ''}`)))))
      : h('p', { class: 'muted' }, 'Nessun avviso in questa sessione.'));
}

// ------------------------------------------------------------------- boot --

(async () => {
  try {
    S.me = await api('info');
    document.getElementById('me-name').textContent = S.me.deviceName;
    await T.event.listen('konnekta', (e) => onEvent(e.payload));
    await refreshDevices();
    render();
  } catch (e) {
    $main.replaceChildren(h('div', { class: 'card' }, h('h1', {}, 'Errore di avvio'), h('p', {}, String(e))));
  }
})();

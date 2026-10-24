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

// Material icons (the phone app uses the same set), drawn inline.
const MATERIAL = {
  wifi: 'M1 9l2 2c4.97-4.97 13.03-4.97 18 0l2-2C16.93 2.93 7.08 2.93 1 9zm8 8l3 3 3-3c-1.65-1.66-4.34-1.66-6 0zm-4-4l2 2c2.76-2.76 7.24-2.76 10 0l2-2C15.14 9.14 8.87 9.14 5 13z',
  bluetooth: 'M17.71 7.71L12 2h-1v7.59L6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 11 14.41V22h1l5.71-5.71-4.3-4.29 4.3-4.29zM13 5.83l1.88 1.88L13 9.59V5.83zm1.88 10.46L13 18.17v-3.76l1.88 1.88z',
  usb: 'M15 7v4h1v2h-3V5h2l-3-4-3 4h2v8H8v-2.07c.7-.37 1.2-1.08 1.2-1.93 0-1.21-.99-2.2-2.2-2.2-1.21 0-2.2.99-2.2 2.2 0 .85.5 1.56 1.2 1.93V13c0 1.11.89 2 2 2h3v3.05c-.71.37-1.2 1.1-1.2 1.95 0 1.22.99 2.2 2.2 2.2 1.21 0 2.2-.98 2.2-2.2 0-.85-.49-1.58-1.2-1.95V15h3c1.11 0 2-.89 2-2v-2h1V7h-4z',
  smartphone: 'M17 1.01L7 1c-1.1 0-2 .9-2 2v18c0 1.1.9 2 2 2h10c1.1 0 2-.9 2-2V3c0-1.1-.9-1.99-2-1.99zM17 19H7V5h10v14z',
  tablet: 'M18 0H6C4.34 0 3 1.34 3 3v18c0 1.66 1.34 3 3 3h12c1.66 0 3-1.34 3-3V3c0-1.66-1.34-3-3-3zm-4 22h-4v-1h4v1zm5.25-3H4.75V3h14.5v16z',
  laptop: 'M20 18c1.1 0 1.99-.9 1.99-2L22 6c0-1.1-.9-2-2-2H4c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2H0v2h24v-2h-4zM4 6h16v10H4V6z',
  desktop: 'M21 2H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h7v2H8v2h8v-2h-2v-2h7c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H3V4h18v12z',
  tv: 'M21 3H3c-1.1 0-2 .89-2 2v12c0 1.1.9 2 2 2h5v2h8v-2h5c1.1 0 1.99-.9 1.99-2L23 5c0-1.11-.9-2-2-2zm0 14H3V5h18v12z',
  devices: 'M4 6h18V4H4c-1.1 0-2 .9-2 2v11H0v3h14v-3H4V6zm19 2h-6c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h6c.55 0 1-.45 1-1V9c0-.55-.45-1-1-1zm-1 9h-4v-7h4v7z',
  swap: 'M16 17.01V10h-2v7.01h-3L15 21l4-3.99h-3zM9 3L5 6.99h3V14h2V6.99h3L9 3z',
  terminal: 'M20 4H4c-1.11 0-2 .9-2 2v12c0 1.1.89 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.89-2-2-2zm0 14H4V8h16v10zm-2-1h-6v-2h6v2zM7.5 17l-1.41-1.41L8.67 13l-2.59-2.59L7.5 9l4 4-4 4z',
  dashboard: 'M3 13h8V3H3v10zm0 8h8v-6H3v6zm10 0h8V11h-8v10zm0-18v6h8V3h-8z',
  extension: 'M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V20c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V22H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z',
  settings: 'M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z',
  lock: 'M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z',
  link: 'M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7c-2.76 0-5 2.24-5 5s2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8v2zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4c2.76 0 5-2.24 5-5s-2.24-5-5-5z',
  mic: 'M12 14c1.66 0 2.99-1.34 2.99-3L15 5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5.3-3c0 3-2.54 5.1-5.3 5.1S6.7 14 6.7 11H5c0 3.41 2.72 6.23 6 6.72V21h2v-3.28c3.28-.48 6-3.3 6-6.72h-1.7z',
  videocam: 'M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z',
  screen: 'M20 18c1.1 0 1.99-.9 1.99-2L22 6c0-1.11-.9-2-2-2H4c-1.11 0-2 .89-2 2v10c0 1.1.89 2 2 2H0v2h24v-2h-4zm-7-3.53v-2.19c-2.78 0-4.61.85-6 2.72.56-2.67 2.11-5.33 6-5.87V7l4 3.73-4 3.74z',
  speaker: 'M17 2H7c-1.1 0-2 .9-2 2v16c0 1.1.9 1.99 2 1.99L17 22c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-5 2c1.1 0 2 .9 2 2s-.9 2-2 2c-1.11 0-2-.9-2-2s.89-2 2-2zm0 16c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z',
  headphones: 'M12 1c-4.97 0-9 4.03-9 9v7c0 1.66 1.34 3 3 3h3v-8H5v-2c0-3.87 3.13-7 7-7s7 3.13 7 7v2h-4v8h3c1.66 0 3-1.34 3-3v-7c0-4.97-4.03-9-9-9z',
  folder: 'M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z',
  paste: 'M19 2h-4.18C14.4.84 13.3 0 12 0c-1.3 0-2.4.84-2.82 2H5c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm7 18H5V4h2v3h10V4h2v16z',
  bell: 'M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z',
  ring: 'M0 15h2V9H0v6zm3 2h2V7H3v10zm19-8v6h2V9h-2zm-3 8h2V7h-2v10zM16.5 3h-9C6.67 3 6 3.67 6 4.5v15c0 .83.67 1.5 1.5 1.5h9c.83 0 1.5-.67 1.5-1.5v-15c0-.83-.67-1.5-1.5-1.5zM16 19H8V5h8v14z',
  call: 'M20.01 15.38c-1.23 0-2.42-.2-3.53-.56-.35-.12-.74-.03-1.01.24l-1.57 1.97c-2.83-1.35-5.48-3.9-6.89-6.83l1.95-1.66c.27-.28.35-.67.24-1.02-.37-1.11-.56-2.3-.56-3.53 0-.54-.45-.99-.99-.99H4.19C3.65 3 3 3.24 3 3.99 3 13.28 10.73 21 20.01 21c.71 0 .99-.63.99-1.18v-3.45c0-.54-.45-.99-.99-.99z',
  battery: 'M15.67 4H14V2h-4v2H8.33C7.6 4 7 4.6 7 5.33v15.33C7 21.4 7.6 22 8.33 22h7.33c.74 0 1.34-.6 1.34-1.33V5.33C17 4.6 16.4 4 15.67 4z',
  build: 'M22.7 19l-9.1-9.1c.9-2.3.4-5-1.5-6.9-2-2-5-2.4-7.4-1.3L9 6 6 9 1.6 4.7C.4 7.1.9 10.1 2.9 12.1c1.9 1.9 4.6 2.4 6.9 1.5l9.1 9.1c.4.4 1 .4 1.4 0l2.3-2.3c.5-.4.5-1.1.1-1.4z',
  volume: 'M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z',
};
function icon(name, size = 18) {
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', size);
  svg.setAttribute('height', size);
  svg.setAttribute('class', 'ic');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(NS, 'path');
  path.setAttribute('d', MATERIAL[name] || MATERIAL.link);
  svg.append(path);
  return svg;
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

const fmtKey = (k) => (k || '').replace(/(.{4})(?=.)/g, '$1 ');
const PROTO_LABEL = { konnekta: 'Konnekta', kdeConnect: 'via KDE Connect', flux: 'via Flux' };
const protoBadge = (p) => h('span', { class: `badge ${p === 'konnekta' ? 'k' : ''}` }, PROTO_LABEL[p] || p);
// Which other apps a plugin also works with (interop adapters).
const interopNote = (list) => (list && list.length)
  ? h('span', { class: 'small muted' }, ` · funziona anche con ${list.map((x) => ({ kdeConnect: 'KDE Connect', flux: 'Flux' }[x] || x)).join(' e ')}`)
  : null;

const typeIcon = (t, size = 22) => icon({ phone: 'smartphone', tablet: 'tablet', laptop: 'laptop', desktop: 'desktop', tv: 'tv' }[t] || 'link', size);

// Sidebar icons (index.html marks each button with data-icon).
for (const b of document.querySelectorAll('#nav button[data-icon]')) b.prepend(icon(b.dataset.icon, 18));

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
    online === 1 ? '1 dispositivo connesso' : `${online} dispositivi connessi`;
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
        h('p', { class: 'muted' }, 'Accetta solo se l\'altro dispositivo mostra esattamente lo stesso codice (tutti i caratteri):'),
        h('div', { class: 'code' }, fmtKey(ev.verificationKey))),
      [['Rifiuta', () => api('rejectPair', { deviceId: ev.deviceId, key: ev.verificationKey })],
       ['Accetta', () => run(api('acceptPair', { deviceId: ev.deviceId, key: ev.verificationKey })), 'primary']]);
      break;
    case 'pairOutgoing':
      modal('Associazione in corso', h('div', {},
        h('p', {}, 'Sull\'altro dispositivo accetta la richiesta solo se mostra questo codice:'),
        h('div', { class: 'code' }, fmtKey(ev.verificationKey)),
        h('p', { class: 'small muted' }, 'Dopo l\'accettazione ti verrà chiesta una conferma anche qui.')),
      [['Chiudi']]);
      break;
    case 'pairConfirm':
      modal('Conferma l\'associazione', h('div', {},
        h('p', {}, `"${ev.deviceName}" ha accettato. Il dispositivo mostra questo codice?`),
        h('div', { class: 'code' }, fmtKey(ev.verificationKey)),
        h('p', { class: 'small muted' }, 'Il dispositivo diventa attendibile solo dopo la tua conferma.')),
      [['No, rifiuta', () => api('rejectPair', { deviceId: ev.deviceId, key: ev.verificationKey }), 'danger'],
       ['Sì, conferma', () => run(api('acceptPair', { deviceId: ev.deviceId, key: ev.verificationKey })), 'primary']]);
      break;
    case 'pairRefused':
      toast(`Richiesta di associazione bloccata da "${ev.deviceName}" (${ev.address}): ${ev.reason}`, 'warn');
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
      if (['microphone', 'camera', 'phonescreen', 'phoneaudio', 'desktop', 'extend', 'systemaudio'].includes(plugin)) {
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
  setup: renderSetup,
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

const TRANSPORT_LABELS = { wifi: 'Wi-Fi', bluetooth: 'Bluetooth', usb: 'cavo USB' };

/** Small icons of the ways a device can be reached now; the one in use is coloured. */
function transportIcons(d) {
  const list = d.transports || [];
  if (!list.length) return null;
  const active = d.connected ? d.transport : null;
  return h('span', { class: 'transports' }, ['usb', 'wifi', 'bluetooth'].filter((t) => list.includes(t)).map((t) => {
    const el = h('span', { class: `tr ${t === active ? 'on' : ''}`, title: `${TRANSPORT_LABELS[t]}${t === active ? ' (in uso)' : ' (disponibile)'}` });
    el.append(icon(t, 14));
    return el;
  }));
}

function deviceRow(d) {
  return h('div', {
    class: `dev ${d.id === S.selected ? 'sel' : ''}`,
    onclick: () => { S.selected = d.id; render(); },
  },
  h('div', { class: 'icon' }, typeIcon(d.deviceType)),
  h('div', { class: 'grow dev-text' },
    h('div', { class: 'dev-name', title: d.name }, d.name),
    h('div', { class: 'small muted row', style: 'gap:6px' },
      h('span', { class: `dot ${d.connected ? 'on' : ''}` }),
      d.connected ? 'Online' : 'Offline',
      S.battery[d.id] ? ` · batteria ${S.battery[d.id].currentCharge}%` : ''),
    transportIcons(d)),
  d.paired ? h('span', { class: 'badge ok' }, 'Associato')
    : h('span', { class: 'badge warn' }, { requested: 'In attesa', confirm: 'Da confermare', requestedByPeer: 'Richiesta' }[d.pairState] || 'Nuovo'));
}

/**
 * Saved devices (paired, online or not) and detected ones (reachable right
 * now: connected, or seen on the network, by cable or over Bluetooth), each
 * with the icons of the ways it was detected.
 */
function deviceList() {
  const saved = S.devices.filter((d) => d.paired);
  const detected = S.devices.filter((d) => d.connected || (d.transports || []).length);
  return h('div', { class: 'card' },
    h('h3', { style: 'margin-top:0' }, 'Dispositivi salvati'),
    saved.length ? saved.map(deviceRow)
      : h('p', { class: 'small muted' }, 'Nessun dispositivo associato: scegline uno tra quelli rilevati.'),
    h('h3', {}, 'Dispositivi rilevati'),
    detected.length ? detected.map(deviceRow) : null,
    nearbyBox(detected.length),
    h('div', { style: 'margin-top:10px' }, connectBox()));
}

function nearbyBox(shown) {
  const box = h('div');
  const empty = h('p', { class: 'small muted' }, 'Nessun dispositivo raggiungibile ora. Apri Konnekta sul telefono nella stessa rete Wi-Fi, collegalo con il cavo o via Bluetooth (associato nel sistema), oppure scrivi il suo indirizzo (anche Tailscale).');
  if (!shown) box.append(empty);
  api('discovered').then((list) => {
    if (!list.length) return;
    empty.remove();
    box.append(...list.map((x) => h('div', { class: 'dev', onclick: () => run(api('connect', { address: x.address }), 'Connessione in corso') },
      h('div', { class: 'icon' }, typeIcon(x.deviceType)),
      h('div', { class: 'grow' }, h('div', {}, x.name),
        h('div', { class: 'small muted' }, x.address),
        transportIcons({ transports: ['wifi'], connected: false })),
      protoBadge(x.protocol))));
  }).catch(() => {});
  return box;
}

/** How to reach a paired device: automatic or one way, remembered on both sides. */
function transportPicker(d) {
  const avail = d.transports || [];
  const sel = h('select', { title: 'Tecnologia di collegamento', onchange: (e) => run(api('setTransport', { deviceId: d.id, transport: e.target.value || null }),
    e.target.value ? `Userò ${TRANSPORT_LABELS[e.target.value]} appena disponibile` : 'Collegamento automatico').then(refreshDevices) },
  h('option', { value: '' }, 'Automatico (il più veloce)'),
  ...['wifi', 'bluetooth', 'usb'].map((t) => h('option', { value: t },
    `${TRANSPORT_LABELS[t]}${d.connected && d.transport === t ? ' · in uso' : avail.includes(t) ? ' · disponibile' : ' · non disponibile'}`)));
  sel.value = d.preferredTransport || '';
  return h('label', { class: 'small muted row', style: 'gap:6px' }, 'Collegamento', sel);
}

function connectBox() {
  const ip = h('input', { type: 'text', placeholder: 'Indirizzo: 192.168.1.20, nome-telefono.ts.net, host:porta' });
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
    h('div', { class: 'dev-ic' }, typeIcon(d.deviceType, 30)),
    h('div', { class: 'grow' },
      h('div', { style: 'font-size:18px;font-weight:700' }, d.name),
      h('div', { class: 'small muted row', style: 'gap:6px' }, d.connected ? `Connesso · ${TRANSPORT_LABELS[d.transport] || d.address}` : 'Non raggiungibile', protoBadge(d.protocol), transportIcons(d)),
      d.paired ? transportPicker(d) : null),
    d.paired ? btn('Dissocia', () => modal('Dissociare il dispositivo?',
      h('p', {}, `${d.name} non potrà più comunicare con questo computer finché non verrà associato di nuovo.`),
      [['Annulla'], ['Dissocia', () => run(api('unpair', { deviceId: d.id })), 'danger']]), 'danger')
      : d.connected && d.pairState === 'notPaired' ? btn('Associa', () => run(api('pair', { deviceId: d.id })), 'primary') : null);

  if (!d.paired) {
    const open = d.pairKey && d.pairState !== 'notPaired';
    return h('div', { class: 'card' }, head,
      open ? h('div', {},
        h('p', {}, d.pairState === 'requestedByPeer' ? 'Il dispositivo chiede di associarsi. Codice:'
          : d.pairState === 'confirm' ? 'Il dispositivo ha accettato. Confermi che mostra questo codice?'
            : 'In attesa che il dispositivo accetti. Codice:'),
        h('div', { class: 'code' }, fmtKey(d.pairKey)),
        d.pairState !== 'requested' ? h('div', { class: 'row' },
          btn('Rifiuta', () => run(api('rejectPair', { deviceId: d.id, key: d.pairKey })), 'danger'),
          btn(d.pairState === 'confirm' ? 'Conferma' : 'Accetta', () => run(api('acceptPair', { deviceId: d.id, key: d.pairKey })), 'primary')) : null) : null,
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
      btn([icon('folder'), ' Invia file…'], async () => {
        const paths = await invoke('pick_files');
        if (paths.length) await run(act('share', 'sendFiles', { paths }, d.id), 'Invio avviato');
      }, 'primary'),
      btn([icon('paste'), ' Invia appunti'], () => run(act('clipboard', 'send', null, d.id), 'Appunti inviati')),
      btn([icon('bell'), ' Ping'], () => run(act('ping', 'send', { message: 'Ping da Konnekta' }, d.id))),
      hasCap(d, 'konnekta.finddevice') ? btn([icon('ring'), ' Fai squillare'], () => run(act('finddevice', 'ring', null, d.id))) : null),
    h('h3', {}, 'Condividi testo'),
    text,
    h('div', { class: 'row', style: 'margin-top:6px' },
      btn('Invia testo', () => run(act('share', 'sendText', { text: text.value }, d.id), 'Inviato')),
      btn('Invia come link', () => run(act('share', 'sendUrl', { url: text.value.trim() }, d.id), 'Inviato'))),
    hasCap(d, 'konnekta.commands') ? remoteCommands(d) : null,
    transfers.length ? h('div', {}, h('h3', {}, 'Trasferimenti recenti'), transfers.map(transferItem)) : null);
}

function remoteCommands(d) {
  const box = h('div', {}, h('h3', {}, 'Comandi del dispositivo'));
  act('commands', 'remoteList', null, d.id).then((list) => {
    const entries = Object.entries(list || {});
    box.append(entries.length
      ? h('div', { class: 'row' }, entries.map(([key, c]) =>
        btn(c.name, () => run(act('commands', 'runRemote', { key }, d.id)))))
      : h('p', { class: 'small muted' }, 'Nessun comando esposto da questo dispositivo.'));
  }).catch(() => {});
  return box;
}

function mediaTab(d) {
  const m = S.phoneMedia[d.id] || {};
  const control = (action) => run(act('media', 'remoteControl', { action, player: m.player }, d.id));
  const vol = h('input', { type: 'range', min: 0, max: 100, value: m.volume ?? 50,
    onchange: (e) => act('media', 'remoteVolume', { volume: +e.target.value, player: m.player }, d.id) });
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
      btn('Aggiorna', () => run(act('media', 'remoteRefresh', null, d.id)))));
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
  const ctrl = hasCap(d, 'konnekta.call.action');
  return h('div', {},
    c ? h('div', { class: 'card', style: 'background:var(--panel-2)' },
      h('div', { style: 'font-size:16px;font-weight:600' },
        c.event === 'ringing' ? '📞 Chiamata in arrivo' : c.event === 'talking' ? '📞 In chiamata' : 'Chiamata persa'),
      h('div', {}, c.caller),
      h('div', { class: 'row', style: 'margin-top:10px' },
        ctrl && c.event === 'ringing' ? btn('Rispondi', () => run(act('calls', 'accept', null, d.id)), 'primary') : null,
        ctrl && c.event === 'ringing' ? btn('Rifiuta', () => run(act('calls', 'reject', null, d.id)), 'danger') : null,
        ctrl && c.event === 'talking' ? btn('Riaggancia', () => run(act('calls', 'hangup', null, d.id)), 'danger') : null,
        btn('Silenzia suoneria', () => run(act('calls', 'mute', null, d.id)))))
      : h('p', { class: 'muted' }, 'Nessuna chiamata in corso.'),
    !ctrl ? h('p', { class: 'small muted' }, 'Questo telefono permette solo di silenziare la suoneria (iOS non consente ad altre app di rispondere o rifiutare le chiamate).') : null);
}

function streamsTab(d) {
  const st = S.streams[d.id] || {};
  // startable: this computer can ask for it (the phone shows Start / Not now);
  // otherwise it is started from the phone (Control).
  const row = (plugin, ic, title, desc, startable, extra) => {
    const supported = hasCap(d, `konnekta.${plugin}`);
    return h('div', { class: 'perm' },
      h('div', { class: 'row grow', style: 'flex-wrap:nowrap' },
        h('div', { class: 'stream-ic' }, icon(ic, 22)),
        h('div', { class: 'grow' },
          h('div', { style: 'font-weight:600' }, title, ' ', st[plugin] ? h('span', { class: 'badge ok' }, 'dal vivo') : null),
          h('div', { class: 'small muted' }, supported ? desc : 'Non supportato dal dispositivo (serve l\'app Konnekta aggiornata).'))),
      extra || null,
      supported ? (st[plugin]
        ? btn('Interrompi', () => run(act(plugin, 'stop', null, d.id)), 'danger')
        : d.bluetooth ? h('span', { class: 'small muted' }, 'Non via Bluetooth')
        : startable ? btn('Chiedi al telefono', () => run(act(plugin, 'start', extraArgs(plugin), d.id), 'Richiesta inviata: conferma sul telefono'), 'primary')
          : h('span', { class: 'small muted' }, 'Si avvia dal telefono')) : null);
  };
  const facing = h('select', {}, h('option', { value: 'back' }, 'Posteriore'), h('option', { value: 'front' }, 'Anteriore'));
  const res = h('select', {}, h('option', { value: '720' }, '720p'), h('option', { value: '1080' }, '1080p'), h('option', { value: '480' }, '480p'));
  function extraArgs(plugin) {
    if (plugin === 'camera') {
      const hgt = +res.value;
      return { codec: 'h264', height: hgt, width: Math.round(hgt * 16 / 9), fps: 30, facing: facing.value };
    }
    if (plugin === 'microphone') return { codec: 'pcm_s16le', sampleRate: 48000, channels: 1 };
    if (plugin === 'phoneaudio') return { codec: 'pcm_s16le', sampleRate: 48000, channels: 2 };
    return { codec: 'h264' };
  }
  return h('div', {},
    d.bluetooth ? h('p', { class: 'badge warn', style: 'display:block;padding:8px 12px' },
      'Collegato via Bluetooth: audio e video sono spenti (il Bluetooth è troppo lento per il tempo reale). Passa a Wi-Fi o al cavo per usarli.') : null,
    h('p', { class: 'small muted' }, 'Tutti i flussi sono cifrati end-to-end con TLS e autenticati con il certificato del dispositivo associato. ' +
      'Il telefono chiede sempre conferma prima di accendere microfono, fotocamera o cattura dello schermo.'),
    h('h3', {}, 'Dal telefono a questo computer'),
    row('microphone', 'mic', 'Microfono', 'Diventa il dispositivo di ingresso "Konnekta Microphone" (Discord, Zoom, OBS…).', true),
    row('camera', 'videocam', 'Webcam', 'La fotocamera del telefono come webcam del computer.', true, h('div', { class: 'row' }, facing, res)),
    row('phonescreen', 'screen', 'Schermo del telefono', 'Lo schermo del telefono in una finestra (con l\'audio, se avviato dal telefono).', true),
    row('phoneaudio', 'headphones', 'Audio del telefono', 'Musica, video e giochi del telefono dalle casse di questo computer.', true),
    h('h3', {}, 'Da questo computer al telefono'),
    row('desktop', 'desktop', 'Desktop remoto', 'Il telefono vede e controlla questo schermo (servono i permessi Desktop remoto e Mouse e tastiera).', false),
    row('extend', 'screen', 'Schermo secondario', 'Il telefono diventa un altro monitor di questo computer: lo sistemi nelle impostazioni dello schermo come ogni monitor (Hyprland o Sway).', false),
    row('systemaudio', 'speaker', 'Audio del computer', 'Il telefono ascolta l\'audio riprodotto da questo computer.', false));
}

// Same names as the phone app (PERMISSION_LABELS in apps/android/.../ui/Common.kt).
const PERMISSION_LABELS = {
  ping: 'Ping', battery: 'Stato della batteria', clipboard: 'Appunti', files: 'Trasferimento file',
  notifications: 'Notifiche', telephony: 'Chiamate', mediaControl: 'Controllo multimediale',
  presenter: 'Telecomando per presentazioni', remoteInput: 'Mouse e tastiera', commands: 'Esecuzione di comandi',
  deck: 'Azioni del Mobile Deck', microphone: 'Microfono', camera: 'Fotocamera e webcam', desktop: 'Desktop remoto',
  systemAudio: 'Audio di sistema', doNotDisturb: 'Sincronizzazione Non disturbare', screenshots: 'Screenshot',
  findMyPhone: 'Trova il dispositivo', browseFiles: 'Sfoglia i file del computer', messages: 'SMS',
  approve: 'Approvazione con impronta (sudo)',
};

async function permissionsTab(d) {
  const perms = await api('permissions');
  return h('div', {},
    h('p', { class: 'small muted' }, 'Scegli cosa può fare questo dispositivo con il computer. Le autorizzazioni sensibili sono disattivate per impostazione predefinita.'),
    perms.map((p) => h('div', { class: 'perm' },
      h('div', {}, h('div', {}, PERMISSION_LABELS[p.id] || p.label, ' ', p.sensitive ? h('span', { class: 'badge warn' }, 'sensibile') : null)),
      sw(d.permissions.includes(p.id), (v) => run(api('setPermission', { deviceId: d.id, permission: p.id, granted: v }))))));
}

async function devicePluginsTab(d) {
  const plugins = await api('plugins');
  return h('div', {},
    h('p', { class: 'small muted' }, 'Disattiva singole funzioni solo per questo dispositivo.'),
    plugins.map((p) => h('div', { class: 'perm' },
      h('div', {}, p.name, interopNote(p.interop)),
      sw(!d.disabledPlugins.includes(p.id), (v) => run(api('setDevicePlugin', { deviceId: d.id, plugin: p.id, enabled: v }))))));
}

function infoTab(d) {
  const addrs = h('textarea', { placeholder: 'Uno per riga, es. pixel.tail1234.ts.net oppure 100.101.102.103' }, (d.extraAddresses || []).join('\n'));
  return h('div', {},
    h('p', {}, 'Protocollo: ', protoBadge(d.protocol), d.app ? h('span', { class: 'small muted' }, `  ${d.app}`) : null),
    h('p', {}, 'ID: ', h('span', { class: 'mono' }, d.id)),
    h('p', {}, 'Impronta del certificato (SHA-256): ', h('div', { class: 'mono' }, d.fingerprint || '—')),
    h('p', { class: 'small muted' }, `Capacità in ingresso: ${d.incomingCapabilities.length} · in uscita: ${d.outgoingCapabilities.length}`),
    h('p', { class: 'small muted' }, 'Il certificato è fissato (pinning): se cambia, la connessione viene rifiutata.'),
    d.protocol === 'konnekta' ? h('div', {},
      h('h3', {}, 'Raggiungibile anche fuori casa'),
      h('p', { class: 'small muted' }, 'Indirizzi che questo computer prova quando il dispositivo non è sulla rete locale, per esempio il nome o l\'IP Tailscale. È sempre il computer ad aprire la connessione, quindi non servono porte aperte in ingresso.'),
      addrs,
      h('div', { class: 'row', style: 'margin-top:6px' }, btn('Salva indirizzi', () => run(api('setExtraAddresses', {
        deviceId: d.id, addresses: addrs.value.split('\n').map((x) => x.trim()).filter(Boolean),
      }), 'Indirizzi salvati')))) : null);
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
    await run(act('commands', cmd ? 'update' : 'add', c), 'Comando salvato');
    render();
  }, 'primary']]);
}

async function renderCommands() {
  commandsCache = await act('commands', 'list').catch(() => []);
  const groups = {};
  for (const c of commandsCache) (groups[c.group || 'Altro'] ||= []).push(c);
  return h('div', {},
    h('div', { class: 'row' }, h('h1', { class: 'grow' }, 'Comandi personalizzati'), btn('+ Nuovo comando', () => commandModal(), 'primary')),
    h('p', { class: 'muted' }, 'Comandi shell, avvio di applicazioni, scorciatoie e macro richiamabili dal telefono o dal Mobile Deck.'),
    commandsCache.length ? Object.entries(groups).map(([g, list]) => h('div', {},
      h('h3', {}, g),
      h('div', { class: 'list' }, list.map((c) => h('div', { class: 'item' },
        h('div', { style: 'font-size:20px;width:28px;text-align:center' }, c.icon || '⌘'),
        h('div', { class: 'grow' }, h('b', {}, c.name), h('div', { class: 'small muted' }, c.description || `${c.actions.length} azioni`)),
        btn('▶ Esegui', () => run(act('commands', 'run', { id: c.id }), 'Eseguito'), 'small'),
        btn('Modifica', () => commandModal(c), 'small'),
        btn('Elimina', () => run(act('commands', 'delete', { id: c.id })).then(render), 'small danger'))))))
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
  return h('div', {}, h('h1', {}, 'Plugin'),
    h('p', { class: 'small muted' }, 'Ogni funzione è un plugin indipendente: disattivalo per tutti i dispositivi qui, o per un solo dispositivo nella sua pagina. Con i ponti attivi (Impostazioni) alcuni plugin funzionano anche con le app KDE Connect e Flux.'),
    h('div', { class: 'card' }, plugins.map((p) => h('div', { class: 'perm' },
      h('div', {}, h('div', { style: 'font-weight:600' }, p.name), h('div', { class: 'small muted' }, p.description, interopNote(p.interop))),
      sw(p.enabled, (v) => run(api('setPluginEnabled', { plugin: p.id, enabled: v })))))));
}

// --- setup assistant ---------------------------------------------------------------

const SETUP_FEATURES = {
  microphone: ['mic', 'Microfono del telefono', 'Usa il telefono come microfono del computer'],
  webcam: ['videocam', 'Webcam del telefono', 'Usa la fotocamera del telefono come webcam'],
  phoneScreen: ['smartphone', 'Schermo del telefono', 'Guarda lo schermo del telefono sul computer'],
  desktop: ['screen', 'Desktop remoto', 'Mostra lo schermo del computer sul telefono'],
  systemAudio: ['headphones', 'Audio del computer', 'Ascolta sul telefono ciò che suona il computer'],
  phoneAudio: ['speaker', 'Audio del telefono', 'Ascolta sul computer ciò che suona il telefono'],
  media: ['volume', 'Controllo dei lettori', 'Play, pausa e brani dal telefono'],
  mixer: ['volume', 'Mixer audio', 'Volumi di uscite, ingressi e app dal telefono'],
};
const SETUP_STATUS = {
  ok: ['ok', 'pronto'], missing: ['warn', 'da installare'], notLoaded: ['warn', 'installato, non attivo'],
  unsupported: ['warn', 'non disponibile qui'],
};

function commandBlock(cmds) {
  const text = cmds.join('\n');
  return h('div', { class: 'cmds' },
    h('pre', {}, text),
    btn('Copia', () => run(navigator.clipboard.writeText(text), 'Comandi copiati'), 'small'));
}

async function renderSetup() {
  const r = await invoke('setup_check');
  const distro = { arch: 'Arch Linux', debian: 'Debian / Ubuntu', fedora: 'Fedora', opensuse: 'openSUSE' }[r.distro];
  const missing = (r.features || []).filter((f) => !f.ok);
  return h('div', {}, h('h1', {}, 'Configurazione'),
    h('p', { class: 'small muted' }, 'Alcune funzioni usano programmi del sistema. Konnekta non li installa da solo: qui trovi cosa manca e i comandi da incollare nel terminale',
      distro ? ` (scritti per ${distro}).` : '.'),
    h('div', { class: 'row', style: 'margin-bottom:14px' },
      missing.length ? h('span', { class: 'badge warn' }, `${missing.length} funzioni da completare`) : h('span', { class: 'badge ok' }, 'Tutto pronto'),
      btn('Ricontrolla', () => render(), 'small')),
    r.allCommands?.length ? h('div', { class: 'card', style: 'margin-bottom:14px' },
      h('h2', { style: 'margin-top:0' }, 'Tutti i comandi'),
      h('p', { class: 'small muted' }, 'Eseguili come utente normale: sudo chiede la password. Poi premi Ricontrolla; non serve riavviare Konnekta.'),
      commandBlock(r.allCommands)) : null,
    h('div', { class: 'card' }, (r.features || []).map((f) => {
      const [ic, title, sub] = SETUP_FEATURES[f.id] || ['extension', f.id, ''];
      const bad = f.requirements.filter((q) => q.status !== 'ok');
      return h('div', { class: 'setup-item' },
        h('div', { class: 'row' }, icon(ic),
          h('div', { class: 'grow' }, h('div', { style: 'font-weight:600' }, title), h('div', { class: 'small muted' }, sub)),
          h('span', { class: `badge ${f.ok ? 'ok' : 'warn'}` }, f.ok ? 'pronto' : 'da completare')),
        bad.map((q) => h('div', { class: 'setup-req' },
          h('div', { class: 'small' }, h('code', {}, q.id), ` — ${q.purpose} `, h('span', { class: `badge ${SETUP_STATUS[q.status][0]}` }, SETUP_STATUS[q.status][1])),
          q.commands.length ? commandBlock(q.commands) : null)));
    })));
}

// --- settings ---------------------------------------------------------------------

async function renderSettings() {
  const cfg = await api('getConfig');
  const shell = await invoke('desktop_prefs_get');
  const get = (p) => act(p, 'getConfig').catch(() => null);
  const [share, clip, shot, filter, dnd, pres, tel] = await Promise.all([
    get('share'), get('clipboard'), get('screenshot'), act('notifications', 'getFilter').catch(() => null), get('dnd'), get('presenter'), get('calls')]);
  const general = { deviceName: cfg.device_name, downloadDir: cfg.download_dir, announce: cfg.announce, mdns: cfg.mdns, usb: cfg.usb !== false, bluetooth: cfg.bluetooth !== false, virtualCamera: cfg.virtual_camera !== false,
    kdeConnect: !!cfg.kde_connect?.enabled, flux: !!cfg.flux?.enabled, syncPermissions: cfg.sync_permissions !== false };
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
      chk(general, 'announce', 'Annuncia questo computer sulla rete locale (disattiva sulle reti pubbliche: i dispositivi associati vengono comunque contattati)'),
      chk(general, 'mdns', 'Trova i dispositivi con mDNS (_konnekta._tcp)'),
      chk(general, 'usb', 'Collegati ai telefoni Android attaccati con il cavo USB (serve adb e il debug USB attivo sul telefono)'),
      chk(general, 'bluetooth', 'Collegati ai telefoni associati via Bluetooth (solo Linux; associa prima il telefono nelle impostazioni Bluetooth del computer)'),
      chk(general, 'virtualCamera', 'Tieni sempre disponibile la webcam «Konnekta Camera» mentre Konnekta è aperto (Linux, v4l2loopback; richiede il riavvio)'),
      chk(general, 'syncPermissions', 'Sincronizza i permessi con i dispositivi: ciò che concedi o revochi sul telefono vale anche qui, e viceversa'),
      h('h3', {}, 'Ponti verso altre app (facoltativi)'),
      chk(general, 'kdeConnect', 'Accetta le app KDE Connect e GSConnect (porte 1716; solo le funzioni comuni; richiede il riavvio)'),
      chk(general, 'flux', 'Accetta le app Flux (porte 12100; solo le funzioni comuni; richiede il riavvio)')),
    () => api('setConfig', general)),
    card('Programma sul computer', h('div', { class: 'list' },
      chk(shell, 'tray', 'Mostra l\'icona nella tray (se disattivata, chiudere la finestra chiude Konnekta)'),
      chk(shell, 'autostart', 'Avvia Konnekta con il computer, solo nella tray, senza aprire la finestra'),
      chk(shell, 'appList', 'Mostra Konnekta nella lista delle applicazioni del computer')),
    () => invoke('desktop_prefs_set', { prefs: shell })),
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
    () => act('calls', 'setConfig', tel)),
    updatesCard());
}

// --- updates ----------------------------------------------------------------------

const UPDATE_SNOOZE_MS = 24 * 3600 * 1000;
const snoozed = (v) => { try { return Number(localStorage.getItem(`update-snooze-${v}`)) > Date.now(); } catch { return false; } };
const snooze = (v) => { try { localStorage.setItem(`update-snooze-${v}`, String(Date.now() + UPDATE_SNOOZE_MS)); } catch { /* ignore */ } };

function showUpdatePopup(u) {
  const status = h('p', { class: 'muted small' });
  modal('Nuova versione disponibile', h('div', {},
    h('p', {}, `Konnekta ${u.latest} è disponibile (hai la ${u.current}).`),
    u.notes ? h('pre', { class: 'mono', style: 'white-space:pre-wrap;max-height:200px;overflow:auto' }, u.notes) : null,
    u.installable ? null : h('p', { class: 'small muted' }, 'Per questo sistema non c\'è un pacchetto installabile automaticamente: scaricala dalla pagina della release.'),
    status),
  [['Ricordamelo più tardi', () => { snooze(u.latest); }],
   u.installable
     ? ['Aggiorna e installa', async () => {
         status.textContent = 'Download e installazione in corso… Konnekta si riavvierà da solo.';
         try { await invoke('install_update'); } catch (e) { status.textContent = String(e); return false; }
         return false;
       }, 'primary']
     : ['Apri la pagina', () => { window.open(u.page, '_blank'); }, 'primary']]);
}

function updatesCard() {
  const out = h('p', { class: 'muted' }, `Versione installata: ${S.me?.version ?? ''}`);
  const check = async () => {
    out.textContent = 'Controllo in corso…';
    try {
      const u = await invoke('check_update');
      if (u.available) { out.textContent = `Disponibile la versione ${u.latest}.`; showUpdatePopup(u); }
      else out.textContent = `Konnekta è aggiornato (versione ${u.current}).`;
    } catch (e) { out.textContent = `Controllo non riuscito: ${e}`; }
  };
  return h('div', { class: 'card', style: 'margin-bottom:14px' }, h('h2', { style: 'margin-top:0' }, 'Aggiornamenti'),
    h('p', {}, 'Repository: ', h('a', { href: 'https://github.com/HubDevCode/Konnekta', target: '_blank' }, 'github.com/HubDevCode/Konnekta')),
    out,
    h('div', { class: 'row', style: 'justify-content:flex-end;margin-top:10px' }, btn('Cerca aggiornamenti', check, 'primary')));
}

async function checkUpdateOnStart() {
  try {
    const u = await invoke('check_update');
    if (u.available && !snoozed(u.latest)) showUpdatePopup(u);
  } catch { /* offline or no release: stay quiet */ }
}

// --- security ---------------------------------------------------------------------

async function renderSecurity() {
  const me = await api('info');
  return h('div', {}, h('h1', {}, 'Sicurezza'),
    h('div', { class: 'card' },
      h('h2', { style: 'margin-top:0' }, 'Questo computer'),
      h('p', {}, 'ID dispositivo: ', h('span', { class: 'mono' }, me.deviceId)),
      h('p', {}, 'Impronta del certificato (SHA-256):'), h('div', { class: 'mono' }, me.fingerprint),
      h('p', { class: 'small muted' }, `Protocollo Konnekta v${me.protocolVersion} · versione ${me.version}`)),
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
    checkUpdateOnStart();
  } catch (e) {
    $main.replaceChildren(h('div', { class: 'card' }, h('h1', {}, 'Errore di avvio'), h('p', {}, String(e))));
  }
})();

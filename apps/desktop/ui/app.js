'use strict';
// Konnekta desktop UI. Untrusted data (device names, notifications, file
// names...) is only ever inserted as text nodes, never as HTML.
//
// Layout: a navigation rail (expanded or compact) with the same first
// three sections as the phone app (Controllo, Invia, Dispositivi), a top
// bar with the device being controlled (the phone's scope chip) and a
// command palette (Ctrl+K).

const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const api = (method, params = {}) => invoke('api', { method, params });
const act = (plugin, action, args = null, deviceId = null) =>
  api('pluginAction', { plugin, action, args, deviceId });

// ---------------------------------------------------------------- helpers --

const $ = (id) => document.getElementById(id);

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
  link_off: 'M17 7h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1 0 1.43-.98 2.63-2.31 2.98l1.46 1.46C20.88 15.61 22 13.95 22 12c0-2.76-2.24-5-5-5zm-1 4h-2.19l2 2H16zM2 4.27l3.11 3.11C3.29 8.12 2 9.91 2 12c0 2.76 2.24 5 5 5h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1 0-1.59 1.21-2.9 2.76-3.07L8.73 11H8v2h2.73L13 15.27V17h1.73l4.01 4L20 19.74 3.27 3 2 4.27z',
  mic: 'M12 14c1.66 0 2.99-1.34 2.99-3L15 5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5.3-3c0 3-2.54 5.1-5.3 5.1S6.7 14 6.7 11H5c0 3.41 2.72 6.23 6 6.72V21h2v-3.28c3.28-.48 6-3.3 6-6.72h-1.7z',
  videocam: 'M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z',
  screen: 'M20 18c1.1 0 1.99-.9 1.99-2L22 6c0-1.11-.9-2-2-2H4c-1.11 0-2 .89-2 2v10c0 1.1.89 2 2 2H0v2h24v-2h-4zm-7-3.53v-2.19c-2.78 0-4.61.85-6 2.72.56-2.67 2.11-5.33 6-5.87V7l4 3.73-4 3.74z',
  monitor: 'M20 3H4c-1.1 0-2 .9-2 2v11c0 1.1.9 2 2 2h3l-1 1v2h12v-2l-1-1h3c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 13H4V5h16v11z',
  speaker: 'M17 2H7c-1.1 0-2 .9-2 2v16c0 1.1.9 1.99 2 1.99L17 22c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-5 2c1.1 0 2 .9 2 2s-.9 2-2 2c-1.11 0-2-.9-2-2s.89-2 2-2zm0 16c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5zm0-8c-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3-1.34-3-3-3z',
  headphones: 'M12 1c-4.97 0-9 4.03-9 9v7c0 1.66 1.34 3 3 3h3v-8H5v-2c0-3.87 3.13-7 7-7s7 3.13 7 7v2h-4v8h3c1.66 0 3-1.34 3-3v-7c0-4.97-4.03-9-9-9z',
  folder: 'M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z',
  paste: 'M19 2h-4.18C14.4.84 13.3 0 12 0c-1.3 0-2.4.84-2.82 2H5c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm7 18H5V4h2v3h10V4h2v16z',
  copy: 'M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z',
  bell: 'M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z',
  ring: 'M0 15h2V9H0v6zm3 2h2V7H3v10zm19-8v6h2V9h-2zm-3 8h2V7h-2v10zM16.5 3h-9C6.67 3 6 3.67 6 4.5v15c0 .83.67 1.5 1.5 1.5h9c.83 0 1.5-.67 1.5-1.5v-15c0-.83-.67-1.5-1.5-1.5zM16 19H8V5h8v14z',
  call: 'M20.01 15.38c-1.23 0-2.42-.2-3.53-.56-.35-.12-.74-.03-1.01.24l-1.57 1.97c-2.83-1.35-5.48-3.9-6.89-6.83l1.95-1.66c.27-.28.35-.67.24-1.02-.37-1.11-.56-2.3-.56-3.53 0-.54-.45-.99-.99-.99H4.19C3.65 3 3 3.24 3 3.99 3 13.28 10.73 21 20.01 21c.71 0 .99-.63.99-1.18v-3.45c0-.54-.45-.99-.99-.99z',
  call_end: 'M12 9c-1.6 0-3.15.25-4.6.72v3.1c0 .39-.23.74-.56.9-.98.49-1.87 1.12-2.66 1.85-.18.18-.43.28-.7.28-.28 0-.53-.11-.71-.29L.29 13.08c-.18-.17-.29-.42-.29-.7 0-.28.11-.53.29-.71C3.34 8.78 7.46 7 12 7s8.66 1.78 11.71 4.67c.18.18.29.43.29.71 0 .28-.11.53-.29.71l-2.48 2.48c-.18.18-.43.29-.71.29-.27 0-.52-.11-.7-.28-.79-.74-1.69-1.36-2.67-1.85-.33-.16-.56-.5-.56-.9v-3.1C15.15 9.25 13.6 9 12 9z',
  build: 'M22.7 19l-9.1-9.1c.9-2.3.4-5-1.5-6.9-2-2-5-2.4-7.4-1.3L9 6 6 9 1.6 4.7C.4 7.1.9 10.1 2.9 12.1c1.9 1.9 4.6 2.4 6.9 1.5l9.1 9.1c.4.4 1 .4 1.4 0l2.3-2.3c.5-.4.5-1.1.1-1.4z',
  volume: 'M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z',
  volume_off: 'M16.5 12c0-1.77-1.02-3.29-2.5-4.03v2.21l2.45 2.45c.03-.2.05-.41.05-.63zm2.5 0c0 .94-.2 1.82-.54 2.64l1.51 1.51C20.63 14.91 21 13.5 21 12c0-4.28-2.99-7.86-7-8.77v2.06c2.89.86 5 3.54 5 6.71zM4.27 3L3 4.27 7.73 9H3v6h4l5 5v-6.73l4.25 4.25c-.67.52-1.42.93-2.25 1.18v2.06c1.38-.31 2.63-.95 3.69-1.81L19.73 21 21 19.73l-9-9L4.27 3zM12 4L9.91 6.09 12 8.18V4z',
  send: 'M2.01 21L23 12 2.01 3 2 10l15 2-15 2z',
  download: 'M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z',
  upload: 'M9 16h6v-6h4l-7-7-7 7h4zm-4 2h14v2H5z',
  sync: 'M12 4V1L8 5l4 4V6c3.31 0 6 2.69 6 6 0 1.01-.25 1.97-.7 2.8l1.46 1.46C19.54 15.03 20 13.57 20 12c0-4.42-3.58-8-8-8zm0 14c-3.31 0-6-2.69-6-6 0-1.01.25-1.97.7-2.8L5.24 7.74C4.46 8.97 4 10.43 4 12c0 4.42 3.58 8 8 8v3l4-4-4-4v3z',
  target: 'M11.71 17.99C8.53 17.84 6 15.22 6 12c0-3.31 2.69-6 6-6 3.22 0 5.84 2.53 5.99 5.71l-2.1-.63C15.48 9.31 13.89 8 12 8c-2.21 0-4 1.79-4 4 0 1.89 1.31 3.48 3.08 3.89l.63 2.1zM22 12c0 .3-.01.6-.04.9l-1.97-.59c.01-.1.01-.21.01-.31 0-4.42-3.58-8-8-8s-8 3.58-8 8 3.58 8 8 8c.1 0 .21 0 .31-.01l.59 1.97c-.3.03-.6.04-.9.04-5.52 0-10-4.48-10-10S6.48 2 12 2s10 4.48 10 10zm-3.77 4.26L22 15l-10-3 3 10 1.26-3.77 4.27 4.27 1.98-1.98-4.28-4.26z',
  file: 'M14 2H6c-1.1 0-1.99.9-1.99 2L4 20c0 1.1.89 2 1.99 2H18c1.1 0 2-.9 2-2V8l-6-6zm2 16H8v-2h8v2zm0-4H8v-2h8v2zm-3-5V3.5L18.5 9H13z',
  image: 'M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5l3.5-4.5z',
  text: 'M14 17H4v2h10v-2zm6-8H4v2h16V9zM4 15h16v-2H4v2zM4 5v2h16V5H4z',
  close: 'M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z',
  add: 'M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z',
  openNew: 'M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z',
  tune: 'M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z',
  search: 'M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z',
  expand_more: 'M16.59 8.59L12 13.17 7.41 8.59 6 10l6 6 6-6z',
  chevron_right: 'M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z',
  menu: 'M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z',
  menu_open: 'M3 18h13v-2H3v2zm0-5h10v-2H3v2zm0-7v2h13V6H3zm18 9.59L17.42 12 21 8.41 19.59 7l-5 5 5 5L21 15.59z',
  play_arrow: 'M8 5v14l11-7z',
  pause: 'M6 19h4V5H6v14zm8-14v14h4V5h-4z',
  stop: 'M6 6h12v12H6z',
  skip_next: 'M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z',
  skip_previous: 'M6 6h2v12H6zm3.5 6l8.5 6V6z',
  music: 'M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z',
  edit: 'M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z',
  delete: 'M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z',
  check: 'M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z',
  check_circle: 'M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z',
  error: 'M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z',
  warning: 'M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z',
  info: 'M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z',
  dark_mode: 'M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9 9-4.03 9-9c0-.46-.04-.92-.1-1.36-.98 1.37-2.58 2.26-4.4 2.26-2.98 0-5.4-2.42-5.4-5.4 0-1.81.89-3.42 2.26-4.4-.44-.06-.9-.1-1.36-.1z',
  light_mode: 'M12 7c-2.76 0-5 2.24-5 5s2.24 5 5 5 5-2.24 5-5-2.24-5-5-5zM2 13h2c.55 0 1-.45 1-1s-.45-1-1-1H2c-.55 0-1 .45-1 1s.45 1 1 1zm18 0h2c.55 0 1-.45 1-1s-.45-1-1-1h-2c-.55 0-1 .45-1 1s.45 1 1 1zM11 2v2c0 .55.45 1 1 1s1-.45 1-1V2c0-.55-.45-1-1-1s-1 .45-1 1zm0 18v2c0 .55.45 1 1 1s1-.45 1-1v-2c0-.55-.45-1-1-1s-1 .45-1 1zM5.99 4.58c-.39-.39-1.03-.39-1.41 0-.39.39-.39 1.03 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0s.39-1.03 0-1.41L5.99 4.58zm12.37 12.37c-.39-.39-1.03-.39-1.41 0-.39.39-.39 1.03 0 1.41l1.06 1.06c.39.39 1.03.39 1.41 0 .39-.39.39-1.03 0-1.41l-1.06-1.06zm1.06-10.96c.39-.39.39-1.03 0-1.41-.39-.39-1.03-.39-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06zM7.05 18.36c.39-.39.39-1.03 0-1.41-.39-.39-1.03-.39-1.41 0l-1.06 1.06c-.39.39-.39 1.03 0 1.41s1.03.39 1.41 0l1.06-1.06z',
  contrast: 'M12 22c5.52 0 10-4.48 10-10S17.52 2 12 2 2 6.48 2 12s4.48 10 10 10zm1-17.93c3.94.49 7 3.85 7 7.93s-3.05 7.44-7 7.93V4.07z',
  shield: 'M12 1L3 5v6c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V5l-9-4z',
  inbox: 'M19 3H4.99c-1.11 0-1.98.89-1.98 2L3 19c0 1.1.88 2 1.99 2H19c1.1 0 2-.9 2-2V5c0-1.11-.9-2-2-2zm0 12h-4c0 1.66-1.35 3-3 3s-3-1.34-3-3H4.99V5H19v10z',
  arrow_up: 'M4 12l1.41 1.41L11 7.83V20h2V7.83l5.58 5.59L20 12l-8-8-8 8z',
  arrow_down: 'M20 12l-1.41-1.41L13 16.17V4h-2v12.17l-5.58-5.59L4 12l8 8 8-8z',
  save: 'M17 3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V7l-4-4zm-5 16c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm3-10H5V5h10v4z',
  reply: 'M10 9V5l-7 7 7 7v-4.1c5 0 8.5 1.6 11 5.1-1-5-4-10-11-11z',
  bolt: 'M7 2v11h3v9l7-12h-4l4-8z',
  keyboard: 'M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm9 7H8v-2h8v2zm0-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z',
  palette: 'M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 9 6.5 9 8 9.67 8 10.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z',
  public: 'M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z',
};
function icon(name, size = 20, cls = '') {
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', size);
  svg.setAttribute('height', size);
  svg.setAttribute('class', `ic ${cls}`.trim());
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(NS, 'path');
  path.setAttribute('d', MATERIAL[name] || MATERIAL.link);
  svg.append(path);
  return svg;
}

/** The app mark, as on the phone's launcher. */
const logo = () => $('logo').content.firstElementChild.cloneNode(true);

const $main = $('main');
const $top = $('topbar');

/** A button: label, action, style (primary, tonal, text, danger, small), optional icon. */
const btn = (label, onclick, cls = '', ic = null) =>
  h('button', { class: `btn ${cls}`, onclick }, ic ? icon(ic) : null, label);
const iconBtn = (ic, title, onclick, cls = '') =>
  h('button', { class: `icon-btn ${cls}`, title, 'aria-label': title, onclick }, icon(ic, 20));

function toast(msg, kind = '', actions = []) {
  const ic = { err: 'error', warn: 'warning', call: 'call' }[kind] || 'check_circle';
  let gone = false;
  const dismiss = () => {
    if (gone) return;
    gone = true;
    t.classList.add('out');
    setTimeout(() => t.remove(), 200);
  };
  const t = h('div', { class: `toast ${kind}`, role: 'status' },
    icon(ic, 20),
    h('div', { class: 'toast-msg' }, msg),
    actions.length ? h('div', { class: 'toast-actions' },
      actions.map(([l, f]) => h('button', { class: 'toast-btn', onclick: () => { f(); dismiss(); } }, l))) : null,
    h('button', { class: 'toast-x', title: 'Chiudi', onclick: dismiss }, icon('close', 16)));
  const box = $('toasts');
  box.append(t);
  // Keep the stack short: the oldest go first.
  while (box.children.length > 4) box.firstElementChild.remove();
  setTimeout(dismiss, actions.length ? 15000 : 5000);
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

/**
 * A dialog. Buttons: [label, action, style]; an action returning false
 * keeps the dialog open. Returns the function that closes it.
 */
function modal(title, body, buttons = [], opts = {}) {
  const root = $('modal-root');
  const close = () => { if (bg.isConnected) root.replaceChildren(); };
  const bg = h('div', { class: 'modal-bg', onmousedown: (e) => { if (e.target === bg) close(); } },
    h('div', { class: `modal ${opts.wide ? 'wide' : ''}`, role: 'dialog', 'aria-modal': 'true' },
      h('div', { class: 'modal-title' }, title),
      h('div', { class: 'modal-body' }, body),
      buttons.length ? h('div', { class: 'modal-foot' },
        buttons.map(([l, f, cls]) => btn(l, async () => { if ((await f?.()) !== false) close(); }, cls || 'text'))) : null));
  root.replaceChildren(bg);
  setTimeout(() => {
    if (!bg.isConnected || bg.contains(document.activeElement)) return;
    (bg.querySelector('input:not([type=checkbox]):not([type=color]), textarea') || bg.querySelector('.modal-foot .btn.primary'))?.focus();
  }, 40);
  return close;
}
const closeModals = () => $('modal-root').replaceChildren();

function confirmModal(title, text, okLabel, onOk) {
  modal(title, h('p', {}, text), [['Annulla'], [okLabel, onOk, 'danger-fill']]);
}

function sw(checked, onchange, disabled = false) {
  return h('label', { class: 'switch' },
    h('input', { type: 'checkbox', role: 'switch', checked, disabled, onchange: (e) => onchange(e.target.checked) }),
    h('span'));
}

/** Popup menu under an element. */
function openMenu(anchor, kids) {
  const root = $('menu-root');
  const r = anchor.getBoundingClientRect();
  const m = h('div', { class: 'menu', role: 'menu' }, kids);
  root.replaceChildren(h('div', { style: 'position:fixed;inset:0;z-index:44', onmousedown: closeMenus }), m);
  const w = m.offsetWidth;
  m.style.left = `${Math.max(12, Math.min(r.right - w, innerWidth - w - 12))}px`;
  m.style.top = `${r.bottom + 6}px`;
}
const closeMenus = () => $('menu-root').replaceChildren();
const menuItem = (lead, label, sub, onclick, active = false) =>
  h('button', { class: `menu-item ${active ? 'active' : ''}`, role: 'menuitem', onclick: () => { closeMenus(); onclick(); } },
    lead, h('div', { class: 'grow' }, h('div', { class: 'ellipsis', style: 'font-weight:600' }, label), sub ? h('div', { class: 'small muted ellipsis' }, sub) : null),
    active ? icon('check', 18) : null);

const fmtBytes = (n) => {
  if (!n) return '0 B';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(u.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
};
const clone = (o) => JSON.parse(JSON.stringify(o));
const fmtKey = (k) => (k || '').replace(/(.{4})(?=.)/g, '$1 ');
/** A verification code in groups of four, as on the phone (KeyBlocks). */
const keyBlocks = (k) => h('div', { class: 'keyblocks selectable' }, (k || '').match(/.{1,4}/g)?.map((g) => h('span', {}, g)));
/** A certificate fingerprint, byte by byte. */
const fingerprintGrid = (fp) => fp
  ? h('div', { class: 'fingerprint selectable', title: fp }, (fp.includes(':') ? fp.split(':') : fp.match(/.{1,2}/g)).map((x) => h('span', {}, x)))
  : h('span', { class: 'muted' }, '—');
const norm = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '');
const hue = (s) => [...String(s || '')].reduce((a, c) => (a * 31 + c.charCodeAt(0)) % 360, 7);
const copyText = (text) => run(navigator.clipboard.writeText(text), 'Copiato negli appunti');

const PROTO_LABEL = { konnekta: 'Konnekta', kdeConnect: 'via KDE Connect', flux: 'via Flux' };
const protoBadge = (p) => h('span', { class: `chip ${p === 'konnekta' ? 'k' : ''}` }, PROTO_LABEL[p] || p);
// Which other apps a plugin also works with (interop adapters).
const interopNote = (list) => (list && list.length)
  ? ` · funziona anche con ${list.map((x) => ({ kdeConnect: 'KDE Connect', flux: 'Flux' }[x] || x)).join(' e ')}`
  : '';

const TYPE_ICON = { phone: 'smartphone', tablet: 'tablet', laptop: 'laptop', desktop: 'desktop', tv: 'tv' };
const typeIcon = (t, size = 22) => icon(TYPE_ICON[t] || 'smartphone', size);
const devAvatar = (d, size = 22, cls = '') => h('div', { class: `avatar ${cls}` }, typeIcon(d.deviceType, size),
  d.paired ? h('span', { class: `presence ${d.connected ? 'on' : ''}` }) : null);
const appAvatar = (name) => h('div', { class: 'avatar app-av', style: `--hue:${hue(name)}` }, (String(name || '?').trim().charAt(0) || '?').toUpperCase());

function notice(kind, text, ic) {
  return h('div', { class: `notice ${kind}` }, icon(ic || { warn: 'warning', err: 'error' }[kind] || 'info', 20), h('div', { class: 'grow' }, text));
}
function cardHead(ic, title, sub, ...extra) {
  return h('div', { class: 'card-head' },
    ic ? h('div', { class: 'head-ic' }, icon(ic, 20)) : null,
    h('h2', {}, title, sub ? h('div', { class: 'sub' }, sub) : null),
    extra);
}
const sectionTitle = (title, sub, ...extra) => h('div', { class: 'section-title' }, h('h2', {}, title), sub ? h('span', { class: 'sub' }, sub) : null, h('span', { class: 'spacer' }), extra);
function emptyState(ic, title, text, ...actions) {
  return h('div', { class: 'empty' }, h('div', { class: 'empty-ic' }, icon(ic, 34)), h('div', { class: 'empty-title' }, title),
    text ? h('p', {}, text) : null, actions.filter(Boolean).length ? h('div', { class: 'row' }, actions) : null);
}
/** A settings row: title, description, control on the right. */
const setRow = (title, desc, control) => h('div', { class: 'set-row' },
  h('div', { class: 'set-text' }, h('div', { class: 'set-title' }, title), desc ? h('div', { class: 'set-desc' }, desc) : null), control);

// ------------------------------------------------------------ preferences --

const PREF = {
  get(k, d) { try { return localStorage.getItem(`konnekta-${k}`) ?? d; } catch { return d; } },
  set(k, v) { try { localStorage.setItem(`konnekta-${k}`, v); } catch { /* ignore */ } },
};

function applyTheme() {
  const t = PREF.get('theme', 'system');
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
}
function setTheme(t) {
  PREF.set('theme', t);
  applyTheme();
  if (S.view === 'settings') render();
}

// Below this width the rail is compact unless opened by hand.
const NARROW = 1000;
let narrowOpen = false;
let wasNarrow = innerWidth < NARROW;
function applyRail() {
  const narrow = innerWidth < NARROW;
  if (narrow !== wasNarrow) { narrowOpen = false; wasNarrow = narrow; }
  const compact = narrow ? !narrowOpen : PREF.get('rail', 'full') === 'compact';
  $('app').classList.toggle('compact', compact);
  const tg = $('rail-toggle');
  tg.replaceChildren(icon(compact ? 'menu' : 'menu_open', 22));
  tg.title = compact ? 'Espandi la barra laterale (Ctrl+B)' : 'Comprimi la barra laterale (Ctrl+B)';
}
function toggleRail() {
  if (innerWidth < NARROW) narrowOpen = !narrowOpen;
  else PREF.set('rail', PREF.get('rail', 'full') === 'compact' ? 'full' : 'compact');
  applyRail();
}

// ------------------------------------------------------------------ state --

const S = {
  view: 'control',
  me: null,
  devices: [],
  /** The paired device Controllo, Invia and Notifiche work with (the phone's scope chip). */
  current: PREF.get('current', null),
  /** The device open in Dispositivi. */
  selected: null,
  devTab: 'connection',
  transfers: new Map(),
  notifications: {},
  battery: {},
  phoneMedia: {},
  phoneMediaAsked: {},
  /** Phone cover per device: { key, url }, asked once per track. */
  phoneArt: {},
  calls: {},
  streams: {},
  warnings: [],
  deck: null,
  deckProfile: null,
  deckPage: 0,
  deckCategory: '',
  // Invia page (LocalSend-style): mode, what to send, LocalSend state.
  sendMode: 'send',
  sel: [],
  ls: { status: null, peers: [], transfers: new Map(), request: null, scanning: false },
  received: [],
  /** Text being typed, kept across refreshes of the page. */
  drafts: {},
  cam: { facing: 'back', res: '720' },
  trFilter: 'all',
  remoteCmds: {},
  discovered: { at: 0, list: [], loading: false },
};

const hasCap = (d, cap) => (d?.incomingCapabilities || []).includes(cap);
const devName = (id) => S.devices.find((d) => d.id === id)?.name || 'Dispositivo';

function cur() {
  const paired = S.devices.filter((d) => d.paired);
  return paired.find((d) => d.id === S.current) || paired.find((d) => d.connected) || paired[0] || null;
}
function setCurrent(id) {
  S.current = id;
  PREF.set('current', id);
  render();
}

/** An input or textarea whose text survives the page being redrawn. */
function draft(k, tag, props = {}, initial = '') {
  if (!(k in S.drafts)) S.drafts[k] = initial;
  const el = h(tag, { ...props, 'data-k': k, oninput: (e) => { S.drafts[k] = e.target.value; props.oninput?.(e); } });
  el.value = S.drafts[k];
  return el;
}

async function refreshDevices() {
  S.devices = await api('devices');
  if (S.selected && !S.devices.some((d) => d.id === S.selected)) S.selected = null;
  drawRailFoot();
  updateNav();
  if (['control', 'devices', 'send', 'notifications'].includes(S.view)) rerender();
  else drawTopbar();
}

// ----------------------------------------------------------------- events --

function onEvent(ev) {
  switch (ev.event) {
    case 'deviceConnected': case 'deviceUpdated': case 'deviceDisconnected':
    case 'paired': case 'unpaired':
      // Reconnected phones are asked again what is playing.
      if (ev.event === 'deviceDisconnected' && ev.deviceId) delete S.phoneMediaAsked?.[ev.deviceId];
      refreshDevices();
      if (ev.event === 'paired') {
        closeModals();
        toast('Associazione completata. Controlla i permessi del dispositivo.');
      }
      break;
    case 'pairRequested':
      modal('Richiesta di associazione', h('div', { class: 'col' },
        h('p', {}, `«${ev.deviceName}» vuole associarsi a questo computer.`),
        h('p', { class: 'muted' }, 'Accetta solo se l\'altro dispositivo mostra esattamente lo stesso codice (tutti i caratteri):'),
        keyBlocks(ev.verificationKey)),
      [['Rifiuta', () => api('rejectPair', { deviceId: ev.deviceId, key: ev.verificationKey }), 'danger'],
       ['Accetta', () => run(api('acceptPair', { deviceId: ev.deviceId, key: ev.verificationKey })), 'primary']]);
      break;
    case 'pairOutgoing':
      modal('Associazione in corso', h('div', { class: 'col' },
        h('p', {}, 'Sull\'altro dispositivo accetta la richiesta solo se mostra questo codice:'),
        keyBlocks(ev.verificationKey),
        h('p', { class: 'small muted' }, 'Dopo l\'accettazione ti verrà chiesta una conferma anche qui.')),
      [['Chiudi']]);
      break;
    case 'pairConfirm':
      modal('Conferma l\'associazione', h('div', { class: 'col' },
        h('p', {}, `«${ev.deviceName}» ha accettato. Il dispositivo mostra questo codice?`),
        keyBlocks(ev.verificationKey),
        h('p', { class: 'small muted' }, 'Il dispositivo diventa attendibile solo dopo la tua conferma.')),
      [['No, rifiuta', () => api('rejectPair', { deviceId: ev.deviceId, key: ev.verificationKey }), 'danger'],
       ['Sì, conferma', () => run(api('acceptPair', { deviceId: ev.deviceId, key: ev.verificationKey })), 'primary']]);
      break;
    case 'pairRefused':
      toast(`Richiesta di associazione bloccata da «${ev.deviceName}» (${ev.address}): ${ev.reason}`, 'warn');
      break;
    case 'pairRejected':
      closeModals();
      toast(`Associazione non riuscita: ${ev.reason}`, 'warn');
      refreshDevices();
      break;
    case 'securityWarning':
      S.warnings.unshift({ at: new Date(), ...ev });
      toast(ev.message, 'warn');
      rerender('security');
      break;
    case 'plugin':
      onPluginEvent(ev);
      break;
  }
}

function onPluginEvent({ deviceId, plugin, kind, data }) {
  const key = `${plugin}/${kind}`;
  if (plugin === 'localsend') return onLocalSend(kind, data);
  switch (key) {
    case 'share/transfer': {
      const before = S.transfers.get(data.id);
      S.transfers.set(data.id, data);
      if (data.state === 'completed' && data.direction === 'incoming' && before?.state !== 'completed') {
        addReceived({ name: data.name, path: data.path, from: devName(data.deviceId), via: 'Konnekta', at: Date.now() });
        toast(`Ricevuto ${data.name}`, '', data.path ? [['Apri', () => openPath(data.path)], ['Mostra', () => openPath(data.path, true)]] : []);
      }
      if (data.state === 'failed') toast(`Trasferimento non riuscito: ${data.name} — ${data.error}`, 'err');
      settleSent(data);
      updateNav();
      rerender('transfers', 'send', 'control');
      return;
    }
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
      drawTopbar();
      rerender('devices');
      break;
    case 'notifications/posted':
      (S.notifications[deviceId] ||= {})[data.id] = data;
      updateNav();
      break;
    case 'notifications/removed':
      delete S.notifications[deviceId]?.[data.id];
      updateNav();
      break;
    case 'notifications/replyRequested':
      S.current = deviceId;
      go('notifications');
      replyModal(deviceId, S.notifications[deviceId]?.[data.id]);
      return;
    case 'telephony/call': {
      const before = S.calls[deviceId];
      S.calls[deviceId] = data;
      if (data.event === 'ringing' && before?.event !== 'ringing') {
        const ctrl = hasCap(S.devices.find((d) => d.id === deviceId), 'konnekta.call.action');
        toast(`Chiamata in arrivo su ${devName(deviceId)}: ${data.caller || 'numero sconosciuto'}`, 'call', [
          ...(ctrl ? [['Rispondi', () => run(act('calls', 'accept', null, deviceId))], ['Rifiuta', () => run(act('calls', 'reject', null, deviceId))]] : []),
          ['Silenzia', () => run(act('calls', 'mute', null, deviceId))]]);
      }
      break;
    }
    case 'telephony/ended':
      delete S.calls[deviceId];
      break;
    case 'media/remoteStatus': {
      // The cover the phone sent on request.
      if (data.albumArtData) {
        S.phoneArt[deviceId] = { key: data.artKey || '', url: data.albumArtData };
        if (deviceId === cur()?.id) patchMediaCard(deviceId);
        return;
      }
      const prev = S.phoneMedia[deviceId] || {};
      // An empty player list: nothing is open on the phone any more.
      const next = Array.isArray(data.playerList) && !data.playerList.length
        ? { volume: prev.volume, playerList: [] }
        : { ...prev, ...data };
      // While the slider is being moved (and just after) it wins over the phone.
      if (volBusy(deviceId) && 'volume' in data) next.volume = prev.volume;
      S.phoneMedia[deviceId] = next;
      wantArt(deviceId);
      // Patched in place: a full re-render would interrupt a slider drag.
      if (deviceId === cur()?.id) patchMediaCard(deviceId);
      return;
    }
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
  if (deviceId === cur()?.id) rerender('control', 'notifications');
}

// ------------------------------------------------------------- navigation --

const NAV = [
  { items: [
    { id: 'control', label: 'Controllo', icon: 'tune', render: () => viewControl() },
    { id: 'send', label: 'Invia', icon: 'send', render: () => viewSend() },
    { id: 'notifications', label: 'Notifiche', icon: 'bell', render: () => viewNotifications() },
    { id: 'devices', label: 'Dispositivi', icon: 'devices', render: () => viewDevices() },
  ] },
  { label: 'Automazioni', items: [
    { id: 'commands', label: 'Comandi', icon: 'terminal', render: () => viewCommands() },
    { id: 'deck', label: 'Mobile Deck', icon: 'dashboard', render: () => viewDeck() },
  ] },
  { label: 'Sistema', items: [
    { id: 'transfers', label: 'Trasferimenti', icon: 'swap', render: () => viewTransfers() },
    { id: 'plugins', label: 'Plugin', icon: 'extension', render: () => viewPlugins() },
    { id: 'setup', label: 'Configurazione', icon: 'build', render: () => viewSetup() },
    { id: 'settings', label: 'Impostazioni', icon: 'settings', render: () => viewSettings() },
    { id: 'security', label: 'Sicurezza', icon: 'lock', render: () => viewSecurity() },
  ] },
];
const PAGES = NAV.flatMap((s) => s.items);
const VIEWS = Object.fromEntries(PAGES.map((v) => [v.id, v]));

function buildNav() {
  $('nav').replaceChildren(...NAV.flatMap((s) => [
    s.label ? h('div', { class: 'nav-sec' }, s.label) : null,
    ...s.items.map((v) => {
      const i = PAGES.indexOf(v);
      return h('button', { class: 'nav-item', 'data-view': v.id, title: `${v.label}${i < 9 ? ` (Ctrl+${i + 1})` : ''}`, onclick: () => go(v.id) },
        icon(v.icon, 22), h('span', { class: 'nav-label' }, v.label), h('span', { class: 'nav-count hidden' }));
    }),
  ].filter(Boolean)));
}

/** Active page and the small counters (notifications, selection, transfers, pairing requests). */
function updateNav() {
  const d = cur();
  const counts = {
    notifications: [d ? Object.keys(S.notifications[d.id] || {}).length : 0],
    send: [S.sel.length],
    transfers: [[...S.transfers.values()].filter((t) => t.state === 'inProgress').length
      + [...S.ls.transfers.values()].filter((t) => ['waiting', 'inProgress'].includes(t.state)).length],
    devices: [S.devices.filter((x) => !x.paired && ['requestedByPeer', 'confirm'].includes(x.pairState)).length, 'warn'],
  };
  for (const b of $('nav').querySelectorAll('.nav-item')) {
    b.classList.toggle('active', b.dataset.view === S.view);
    const [n, cls] = counts[b.dataset.view] || [0];
    const c = b.querySelector('.nav-count');
    c.textContent = n > 99 ? '99+' : String(n);
    c.className = `nav-count ${cls || ''} ${n ? '' : 'hidden'}`;
  }
}

function go(view) {
  if (!VIEWS[view]) return;
  S.view = view;
  PREF.set('view', view);
  closeMenus();
  render();
}

// ------------------------------------------------------------------ chrome --

/** What the page shown last asked for in the top bar: title, subtitle, actions. */
let lastOut = {};

function drawTopbar() {
  const v = VIEWS[S.view];
  const o = lastOut;
  const actions = (o.actions || []).filter(Boolean);
  // replaceChildren would turn a null into the text "null".
  $top.replaceChildren(...[
    h('div', { class: 'tb-title' }, h('h1', {}, o.title || v.label), o.sub ? h('div', { class: 'tb-sub' }, o.sub) : null),
    actions.length ? h('div', { class: 'tb-actions' }, actions) : null,
    actions.length ? h('div', { class: 'tb-sep' }) : null,
    scopeChip(),
    h('button', { class: 'search-btn', title: 'Cerca pagine e azioni (Ctrl+K)', onclick: openPalette },
      icon('search', 20), h('span', { class: 'lbl' }, 'Cerca o esegui…'), h('kbd', {}, 'Ctrl K')),
  ].filter(Boolean));
}

/** The device being controlled, as the chip on top of the phone app. */
function scopeChip() {
  const d = cur();
  if (!d) {
    return h('button', { class: 'scope', onclick: () => go('devices'), title: 'Associa un telefono' },
      h('div', { class: 'avatar neutral' }, icon('add', 18)),
      h('div', { class: 'scope-text' }, h('div', { class: 'scope-name' }, 'Associa un dispositivo'), h('div', { class: 'scope-sub' }, 'Nessun telefono')));
  }
  const b = S.battery[d.id];
  const el = h('button', { class: 'scope', title: 'Il dispositivo che controlli', onclick: () => scopeMenu(el) },
    devAvatar(d, 18),
    h('div', { class: 'scope-text' },
      h('div', { class: 'scope-name ellipsis' }, d.name),
      h('div', { class: 'scope-sub' }, d.connected ? (TRANSPORT_LABELS[d.transport] || 'Online') : 'Offline',
        b && b.currentCharge >= 0 ? ` · ${b.currentCharge}%` : '')),
    icon('expand_more', 18, 'chev'));
  return el;
}

function scopeMenu(anchor) {
  const paired = S.devices.filter((d) => d.paired);
  const d = cur();
  openMenu(anchor, [
    h('div', { class: 'menu-label' }, 'Dispositivo da controllare'),
    paired.map((x) => menuItem(devAvatar(x, 18), x.name, x.connected ? `Online · ${TRANSPORT_LABELS[x.transport] || ''}` : 'Offline', () => setCurrent(x.id), x.id === d?.id)),
    h('div', { class: 'menu-sep' }),
    menuItem(h('div', { class: 'avatar neutral' }, icon('devices', 18)), 'Gestisci i dispositivi', null, () => { if (d) S.selected = d.id; go('devices'); }),
  ]);
}

function drawRailFoot() {
  const online = S.devices.filter((d) => d.connected && d.paired).length;
  const foot = $('rail-foot');
  foot.replaceChildren(
    h('div', { class: 'avatar sec' }, icon('desktop', 20)),
    h('div', { class: 'rail-foot-text' },
      h('div', { class: 'rail-foot-name ellipsis' }, S.me?.deviceName || 'Konnekta'),
      h('div', { class: 'rail-foot-sub' }, h('span', { class: `dot ${online ? 'on' : ''}` }),
        online === 1 ? '1 connesso' : `${online} connessi`)));
  foot.title = `${S.me?.deviceName || ''} · Konnekta ${S.me?.version || ''}`;
}

// ------------------------------------------------------------------ render --

let renderSeq = 0;
let shownView = null;
let scrollHook = null;

/** Focused field, caret and inner scroll positions, to put back after a redraw. */
function captureUi() {
  const a = document.activeElement;
  const k = a && $('app').contains(a) ? a.dataset?.k : null;
  let sel = null;
  try { if (k) sel = [a.selectionStart, a.selectionEnd]; } catch { /* not a text field */ }
  const scrolls = {};
  for (const el of $main.querySelectorAll('[data-scroll]')) scrolls[el.dataset.scroll] = el.scrollTop;
  return { k, sel, scrolls };
}
function restoreUi({ k, sel, scrolls }) {
  for (const el of $main.querySelectorAll('[data-scroll]')) {
    if (scrolls[el.dataset.scroll] != null) el.scrollTop = scrolls[el.dataset.scroll];
  }
  if (!k) return;
  const el = $('app').querySelector(`[data-k="${CSS.escape(k)}"]`);
  if (!el) return;
  el.focus({ preventScroll: true });
  try { if (sel && sel[0] != null) el.setSelectionRange(sel[0], sel[1]); } catch { /* not a text field */ }
}

async function render() {
  const seq = ++renderSeq;
  const view = S.view;
  let out;
  try {
    out = (await VIEWS[view].render()) || {};
  } catch (e) {
    out = { body: h('div', { class: 'card' }, notice('err', `Questa pagina non si è caricata: ${e}`)) };
  }
  if (seq !== renderSeq) return;
  const keep = captureUi();
  const entering = view !== shownView;
  lastOut = out;
  scrollHook = out.onScroll || null;
  drawTopbar();
  $main.replaceChildren(h('div', { class: `page ${out.narrow ? 'narrow' : ''} ${entering ? 'enter' : ''}` }, out.body));
  if (entering) {
    shownView = view;
    $main.scrollTop = 0;
  }
  restoreUi(keep);
  updateNav();
  scrollHook?.();
}

/** Redraw soon if one of these pages is shown (all pages when none given); bursts of events draw once. */
let rerenderTimer = null;
function rerender(...views) {
  if (views.length && !views.includes(S.view)) return;
  if (rerenderTimer) return;
  const tick = () => {
    // Rebuilding the page now would cut the volume slider out from under the pointer.
    if (volDragging()) { rerenderTimer = setTimeout(tick, 150); return; }
    rerenderTimer = null;
    render();
  };
  rerenderTimer = setTimeout(tick, 60);
}

$main.addEventListener('scroll', () => {
  $top.classList.toggle('scrolled', $main.scrollTop > 4);
  scrollHook?.();
}, { passive: true });

// --------------------------------------------------------------- controllo --

const TRANSPORT_LABELS = { wifi: 'Wi-Fi', bluetooth: 'Bluetooth', usb: 'cavo USB' };

/** Small icons of the ways a device can be reached now; the one in use is coloured. */
function transportIcons(d) {
  const list = d.transports || [];
  if (!list.length) return null;
  const active = d.connected ? d.transport : null;
  return h('span', { class: 'transports' }, ['usb', 'wifi', 'bluetooth'].filter((t) => list.includes(t)).map((t) =>
    h('span', { class: `tr ${t === active ? 'on' : ''}`, title: `${TRANSPORT_LABELS[t]}${t === active ? ' (in uso)' : ' (disponibile)'}` }, icon(t, 14))));
}
function transportChips(d) {
  return ['usb', 'wifi', 'bluetooth'].filter((t) => (d.transports || []).includes(t)).map((t) => {
    const inUse = d.connected && d.transport === t;
    return h('span', { class: `chip ${inUse ? 'p' : ''}`, title: inUse ? 'In uso' : 'Disponibile' }, icon(t, 14), TRANSPORT_LABELS[t]);
  });
}

function batteryMeter(b) {
  if (!b || b.currentCharge == null || b.currentCharge < 0) return null;
  const p = b.currentCharge;
  return h('span', { class: 'battery', title: b.isCharging ? 'In carica' : 'Batteria' },
    h('span', { class: 'battery-cell' }, h('i', { class: p <= 15 ? 'low' : '', style: `width:${p}%` })),
    b.isCharging ? icon('bolt', 13) : null, `${p}%`);
}

/** The device drawn on the hero card: a phone, or a screen for computers. */
function deviceArt(d) {
  const b = S.battery[d.id];
  const phone = !['laptop', 'desktop', 'tv'].includes(d.deviceType);
  const now = new Date();
  const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
  return h('div', { class: `art ${phone ? 'phone' : 'monitor'}`, 'aria-hidden': 'true' },
    h('div', { class: 'art-screen' }, logo(),
      phone ? h('div', { class: 'art-bar' }, h('span', {}, time), h('span', {}, b && b.currentCharge >= 0 ? `${b.currentCharge}%` : '')) : null,
      phone ? h('div', { class: 'art-dock' }) : null),
    phone && d.connected ? h('div', { class: 'art-pulse' }) : null);
}

async function sendFilesTo(d) {
  const paths = await invoke('pick_files');
  if (paths.length) await run(act('share', 'sendFiles', { paths }, d.id), 'Invio avviato');
}

function hero(d) {
  const on = d.connected;
  const qa = (ic, label, f, primary = false) => h('button', { class: `qa ${primary ? 'primary' : ''}`, onclick: f, disabled: !on, title: label },
    h('div', { class: 'qa-ic' }, icon(ic, 24)), label);
  return h('section', { class: `hero ${on ? '' : 'offline'}` },
    deviceArt(d),
    h('div', { class: 'hero-info' },
      h('div', { class: 'hero-status' }, h('span', { class: `dot ${on ? 'on' : ''}` }),
        on ? `Connesso via ${TRANSPORT_LABELS[d.transport] || d.address || 'rete'}` : 'Non raggiungibile'),
      h('div', { class: 'hero-name', title: d.name }, d.name),
      h('div', { class: 'hero-meta' }, batteryMeter(S.battery[d.id]), protoBadge(d.protocol), transportChips(d))),
    h('div', { class: 'hero-actions' },
      qa('folder', 'Invia file', () => sendFilesTo(d), true),
      qa('paste', 'Appunti', () => run(act('clipboard', 'send', null, d.id), 'Appunti inviati')),
      qa('bell', 'Ping', () => run(act('ping', 'send', { message: 'Ping da Konnekta' }, d.id), `Ping inviato a ${d.name}`)),
      hasCap(d, 'konnekta.finddevice') ? qa('ring', 'Fai squillare', () => run(act('finddevice', 'ring', null, d.id))) : null,
      h('button', { class: 'qa', title: 'Permessi, plugin e collegamento', onclick: () => { S.selected = d.id; go('devices'); } },
        h('div', { class: 'qa-ic' }, icon('settings', 24)), 'Gestisci')));
}

function callBanner(d) {
  const c = S.calls[d.id];
  if (!c) return null;
  const ctrl = hasCap(d, 'konnekta.call.action');
  const ringing = c.event === 'ringing';
  return h('div', { class: `call-banner ${ringing ? 'ringing' : ''}` },
    h('div', { class: 'avatar round' }, icon('call', 22)),
    h('div', { class: 'grow' },
      h('div', { class: 'call-title' }, ringing ? 'Chiamata in arrivo' : c.event === 'talking' ? 'In chiamata' : 'Chiamata persa'),
      h('div', { class: 'muted' }, c.caller || 'Numero sconosciuto'),
      !ctrl ? h('div', { class: 'small muted' }, 'Questo telefono permette solo di silenziare la suoneria (iOS non consente ad altre app di rispondere o rifiutare le chiamate).') : null),
    h('div', { class: 'row' },
      btn('Silenzia suoneria', () => run(act('calls', 'mute', null, d.id)), 'text', 'volume_off'),
      ctrl && ringing ? btn('Rifiuta', () => run(act('calls', 'reject', null, d.id)), 'danger', 'call_end') : null,
      ctrl && ringing ? btn('Rispondi', () => run(act('calls', 'accept', null, d.id)), 'primary', 'call') : null,
      ctrl && c.event === 'talking' ? btn('Riaggancia', () => run(act('calls', 'hangup', null, d.id)), 'danger-fill', 'call_end') : null));
}

/**
 * One audio/video feature. startable: this computer can ask for it (the
 * phone shows Start / Not now); otherwise it is started from the phone.
 */
function streamTile(d, plugin, ic, title, desc, startable, opts) {
  const supported = hasCap(d, `konnekta.${plugin}`);
  const live = !!(S.streams[d.id] || {})[plugin];
  const note = (t) => h('span', { class: 'tile-note' }, t);
  let action;
  if (!supported) action = note('Non supportato');
  else if (live) action = btn('Interrompi', () => run(act(plugin, 'stop', null, d.id)), 'small danger-fill', 'stop');
  else if (!d.connected) action = note('Offline');
  else if (d.bluetooth) action = note('Non via Bluetooth');
  else if (startable) action = btn('Chiedi al telefono', () => run(act(plugin, 'start', streamArgs(plugin), d.id), 'Richiesta inviata: conferma sul telefono'), 'small tonal');
  else action = note('Si avvia dal telefono');
  return h('div', { class: `tile ${live ? 'live' : ''} ${supported ? '' : 'off'}` },
    h('div', { class: 'tile-top' }, h('div', { class: 'tile-ic' }, icon(ic, 24)),
      live ? h('span', { class: 'live-chip' }, h('i'), 'Dal vivo') : null),
    h('div', {}, h('div', { class: 'tile-title' }, title),
      h('div', { class: 'tile-desc' }, supported ? desc : 'Non supportato dal dispositivo (serve l\'app Konnekta aggiornata).')),
    supported && !live && opts ? opts : null,
    h('div', { class: 'tile-foot' }, action));
}

function streamArgs(plugin) {
  if (plugin === 'camera') {
    const hgt = +S.cam.res;
    return { codec: 'h264', height: hgt, width: Math.round(hgt * 16 / 9), fps: 30, facing: S.cam.facing };
  }
  if (plugin === 'microphone') return { codec: 'pcm_s16le', sampleRate: 48000, channels: 1 };
  if (plugin === 'phoneaudio') return { codec: 'pcm_s16le', sampleRate: 48000, channels: 2 };
  return { codec: 'h264' };
}

function cameraOpts() {
  const opt = (v, l, cur) => h('option', { value: v, selected: v === cur }, l);
  return h('div', { class: 'tile-opts' },
    h('select', { title: 'Fotocamera', onchange: (e) => { S.cam.facing = e.target.value; } },
      opt('back', 'Posteriore', S.cam.facing), opt('front', 'Anteriore', S.cam.facing)),
    h('select', { title: 'Risoluzione', onchange: (e) => { S.cam.res = e.target.value; } },
      opt('720', '720p', S.cam.res), opt('1080', '1080p', S.cam.res), opt('480', '480p', S.cam.res)));
}

/** Phone volume slider state per device: dragging, last moved, pending send. */
const VOL = {};
/** A drag counts only while the button is down (and never longer than 15 s, if a release got lost). */
const isDrag = (v) => !!v?.drag && Date.now() - v.dragAt < 15000;
const volDragging = () => Object.values(VOL).some(isDrag);
/** The slider wins over what the phone reports while moved and shortly after. */
const volBusy = (id) => { const v = VOL[id]; return !!v && (isDrag(v) || v.timer || Date.now() - (v.at || 0) < 900); };
const endVolDrag = () => { for (const v of Object.values(VOL)) if (v.drag) { v.drag = false; v.at = Date.now(); } };
window.addEventListener('pointerup', endVolDrag, true);
window.addEventListener('pointercancel', endVolDrag, true);
window.addEventListener('blur', endVolDrag);

/** Sends the volume while the slider moves, at most every 80 ms (the last value always goes). */
function sendVolume(id, volume) {
  volume = Math.max(0, Math.min(100, Math.round(volume)));
  const v = (VOL[id] ||= {});
  v.at = Date.now();
  v.want = volume;
  showVolume(id, volume);
  if (v.timer) return;
  const flush = () => {
    if (v.want == null) { v.timer = null; return; }
    const vol = v.want;
    v.want = null;
    (S.phoneMedia[id] ||= {}).volume = vol;
    if (vol > 0) v.beforeMute = null;
    act('media', 'remoteVolume', { volume: vol, player: S.phoneMedia[id]?.player }, id).catch((err) => toast(String(err), 'err'));
    v.timer = setTimeout(flush, 80);
  };
  flush();
}

/** Asks the phone for the cover when the track's cover changed (at most every 5 s for the same one). */
function wantArt(id) {
  const key = S.phoneMedia[id]?.artKey;
  if (!key) return;
  const a = S.phoneArt[id];
  if (a?.key === key) return;
  if (a?.asked === key && Date.now() - a.askedAt < 5000) return;
  S.phoneArt[id] = { ...(a || {}), asked: key, askedAt: Date.now() };
  act('media', 'remoteArt', { player: S.phoneMedia[id]?.player }, id).catch(() => {});
}

/** The cover box: the picture when the phone sent the current track's one. */
function artImage(id) {
  const m = S.phoneMedia[id] || {};
  const a = S.phoneArt[id];
  return m.artKey && a?.url && a.key === m.artKey ? h('img', { src: a.url, alt: '' }) : icon('music', 34);
}

function mediaParts(d) {
  const m = S.phoneMedia[d.id] || {};
  const on = d.connected;
  return {
    title: m.title || 'Niente in riproduzione',
    sub: [m.artist, m.album].filter(Boolean).join(' — ') || (on ? 'Avvia la musica sul telefono' : 'Il telefono è offline'),
    player: m.player || '',
    playing: !!m.isPlaying,
    volume: m.volume ?? 50,
  };
}

/**
 * While the media card is on screen, ask the phone every second what is
 * playing and its volume: the phone app only reports track changes by
 * itself, not the volume (keys, other apps).
 */
let mediaPollBusy = false;
setInterval(() => {
  const card = document.querySelector('.np-card[data-dev]');
  if (!card || mediaPollBusy || document.hidden) return;
  const id = card.dataset.dev;
  const d = S.devices.find((x) => x.id === id);
  if (!d?.connected || volBusy(id)) return;
  mediaPollBusy = true;
  act('media', 'remoteRefresh', { player: S.phoneMedia[id]?.player }, id)
    .catch(() => {})
    .finally(() => { mediaPollBusy = false; });
}, 1000);

/** Updates the open media card without rebuilding the page. */
function patchMediaCard(id) {
  const card = document.querySelector(`.np-card[data-dev="${CSS.escape(id)}"]`);
  const d = S.devices.find((x) => x.id === id);
  if (!card || !d) return;
  const p = mediaParts(d);
  card.querySelector('.np-title').textContent = p.title;
  card.querySelector('.np-sub').textContent = p.sub;
  const pl = card.querySelector('.np-player');
  pl.textContent = p.player;
  pl.hidden = !p.player;
  const play = card.querySelector('.np-play');
  play.title = p.playing ? 'Pausa' : 'Riproduci';
  play.replaceChildren(icon(p.playing ? 'pause' : 'play_arrow', 28));
  if (!volBusy(id)) showVolume(id, p.volume);
  const art = card.querySelector('.np-art');
  const img = art.querySelector('img');
  const want = artImage(id);
  if (want.tagName === 'IMG' ? img?.src !== want.src : !!img || !art.firstChild) art.replaceChildren(want);
}

/** Slider, percentage and icon of the phone volume, without rebuilding the card. */
function showVolume(id, volume) {
  const card = document.querySelector(`.np-card[data-dev="${CSS.escape(id)}"]`);
  if (!card) return;
  const input = card.querySelector('.np-vol input');
  if (+input.value !== volume) input.value = volume;
  input.style.setProperty('--fill', `${volume}%`);
  card.querySelector('.np-vol-pct').textContent = `${volume}%`;
  const mute = card.querySelector('.np-mute');
  mute.replaceChildren(icon(volume === 0 ? 'volume_off' : 'volume', 18));
  mute.title = volume === 0 ? 'Riattiva l\'audio' : 'Silenzia';
}

function mediaCard(d) {
  const on = d.connected;
  // Ask the phone what is open as soon as the card is shown with the phone online.
  if (!on) delete S.phoneMediaAsked?.[d.id];
  else if (!(S.phoneMediaAsked ||= {})[d.id]) {
    S.phoneMediaAsked[d.id] = true;
    act('media', 'remoteRefresh', null, d.id).catch(() => { delete S.phoneMediaAsked[d.id]; });
  }
  const p = mediaParts(d);
  const control = (action) => run(act('media', 'remoteControl', { action, player: S.phoneMedia[d.id]?.player }, d.id));
  const v = (VOL[d.id] ||= {});
  return h('div', { class: 'card np-card', 'data-dev': d.id },
    cardHead('music', 'In riproduzione sul telefono', null,
      iconBtn('sync', 'Aggiorna', () => run(act('media', 'remoteRefresh', null, d.id)), 'sm')),
    h('div', { class: 'np' },
      h('div', { class: 'np-art' }, artImage(d.id)),
      h('div', { class: 'grow' },
        h('div', { class: 'np-title ellipsis' }, p.title),
        h('div', { class: 'np-sub small muted ellipsis' }, p.sub),
        h('div', { class: 'np-player small muted ellipsis', hidden: !p.player }, p.player))),
    h('div', { class: 'np-controls' },
      iconBtn('skip_previous', 'Precedente', () => control('Previous')),
      h('button', { class: 'np-play', title: p.playing ? 'Pausa' : 'Riproduci', disabled: !on, onclick: () => control('PlayPause') }, icon(p.playing ? 'pause' : 'play_arrow', 28)),
      iconBtn('skip_next', 'Successivo', () => control('Next'))),
    h('div', { class: 'np-vol' },
      h('button', { class: 'np-mute icon-btn sm', title: p.volume === 0 ? 'Riattiva l\'audio' : 'Silenzia', disabled: !on,
        onclick: () => {
          const now = S.phoneMedia[d.id]?.volume ?? p.volume;
          if (now > 0) { sendVolume(d.id, 0); v.beforeMute = now; } else sendVolume(d.id, v.beforeMute || 30);
        } }, icon(p.volume === 0 ? 'volume_off' : 'volume', 18)),
      h('button', { class: 'icon-btn sm np-step', title: 'Abbassa', disabled: !on,
        onclick: () => sendVolume(d.id, (S.phoneMedia[d.id]?.volume ?? p.volume) - 7) }, '−'),
      h('input', { type: 'range', min: 0, max: 100, step: 1, value: p.volume, title: 'Volume del telefono (anche con la rotella)', disabled: !on,
        style: `--fill:${p.volume}%`,
        onpointerdown: () => { v.drag = true; v.dragAt = Date.now(); },
        oninput: (e) => sendVolume(d.id, +e.target.value),
        onchange: (e) => sendVolume(d.id, +e.target.value),
        onwheel: (e) => { e.preventDefault(); sendVolume(d.id, (S.phoneMedia[d.id]?.volume ?? p.volume) + (e.deltaY < 0 ? 4 : -4)); } }),
      h('button', { class: 'icon-btn sm np-step', title: 'Alza', disabled: !on,
        onclick: () => sendVolume(d.id, (S.phoneMedia[d.id]?.volume ?? p.volume) + 7) }, '+'),
      h('span', { class: 'np-vol-pct small' }, `${p.volume}%`)),
    h('p', { class: 'small muted', style: 'margin-top:10px' }, 'Dal telefono puoi controllare i lettori di questo computer.'));
}

function shareCard(d) {
  const k = `share-${d.id}`;
  const ta = draft(k, 'textarea', { placeholder: 'Testo o link da inviare…', rows: 3 });
  const done = () => { delete S.drafts[k]; ta.value = ''; };
  const text = () => S.drafts[k] || '';
  return h('div', { class: 'card' },
    cardHead('text', 'Condividi testo'),
    ta,
    h('div', { class: 'card-foot', style: 'margin-top:12px' },
      btn('Come link', () => run(act('share', 'sendUrl', { url: text().trim() }, d.id), 'Link inviato').then(done), 'text', 'link'),
      btn('Invia testo', () => run(act('share', 'sendText', { text: text() }, d.id), 'Testo inviato').then(done), 'tonal', 'send')));
}

/** The commands the phone exposes, asked again at most every 30 seconds. */
function remoteCommandsCard(d) {
  if (!hasCap(d, 'konnekta.commands') || !d.connected) return null;
  const c = S.remoteCmds[d.id];
  if ((!c || Date.now() - c.at > 30000) && !c?.loading) {
    S.remoteCmds[d.id] = { list: c?.list ?? null, at: c?.at || 0, loading: true };
    act('commands', 'remoteList', null, d.id)
      .then((list) => Object.entries(list || {}))
      .catch(() => [])
      .then((list) => { S.remoteCmds[d.id] = { list, at: Date.now() }; rerender('control'); });
  }
  const list = S.remoteCmds[d.id].list;
  return h('div', { class: 'card' },
    cardHead('terminal', 'Comandi del telefono'),
    list == null ? h('div', { class: 'small muted' }, 'Caricamento…')
      : list.length ? h('div', { class: 'chips' }, list.map(([key, x]) => btn(x.name, () => run(act('commands', 'runRemote', { key }, d.id)), 'small tonal', 'play_arrow')))
        : h('div', { class: 'small muted' }, 'Nessun comando esposto da questo dispositivo.'));
}

function notifPreview(d) {
  const list = Object.values(S.notifications[d.id] || {}).reverse();
  return h('div', { class: 'card' },
    cardHead('bell', 'Notifiche', list.length ? `${list.length} dal telefono` : null,
      list.length ? btn('Vedi tutte', () => go('notifications'), 'text small') : null),
    list.length ? list.slice(0, 3).map((n) => h('div', { class: 'mini-notif' }, appAvatar(n.appName),
      h('div', { class: 'grow' }, h('div', { class: 'notif-app' }, n.appName),
        h('div', { class: 'notif-title ellipsis' }, n.title), h('div', { class: 'small muted ellipsis' }, n.text))))
      : h('div', { class: 'small muted' }, 'Nessuna notifica dal dispositivo.'));
}

function recentTransfers(d) {
  const list = [...S.transfers.values()].filter((t) => t.deviceId === d.id).slice(-4).reverse();
  if (!list.length) return null;
  return h('div', { class: 'card flush' },
    h('div', { style: 'padding:18px 20px 4px' }, cardHead('swap', 'Trasferimenti recenti', null, btn('Tutti', () => go('transfers'), 'text small'))),
    h('div', { class: 'list plain' }, list.map(transferRow)));
}

function welcome() {
  return h('div', { class: 'card', style: 'padding:20px 20px 36px' },
    h('div', { class: 'empty', style: 'padding-bottom:0' },
      h('div', { class: 'radar-logo', style: 'width:120px;height:120px' }, logo()),
      h('div', { class: 'empty-title', style: 'font-size:22px;margin-top:6px' }, 'Collega il tuo telefono'),
      h('p', {}, 'Konnekta unisce telefono e computer: file, appunti, notifiche, chiamate, media, microfono e webcam, desktop remoto e Mobile Deck. Tutto cifrato, senza cloud.'),
      h('div', { class: 'steps' },
        h('div', { class: 'step' }, h('b', {}, '1'), h('div', {}, 'Installa e apri Konnekta sul telefono.')),
        h('div', { class: 'step' }, h('b', {}, '2'), h('div', {}, 'Stessa rete Wi-Fi, cavo USB o Bluetooth.')),
        h('div', { class: 'step' }, h('b', {}, '3'), h('div', {}, 'Scegli il telefono e confronta il codice.'))),
      h('div', { class: 'row' }, btn('Trova dispositivi', () => go('devices'), 'primary', 'devices'))));
}

async function viewControl() {
  const d = cur();
  if (!d) return { sub: 'Nessun dispositivo associato', body: welcome() };
  const on = d.connected;
  return {
    sub: on ? `${d.name} · connesso` : `${d.name} · non raggiungibile`,
    body: [
      callBanner(d),
      hero(d),
      !on ? notice('info', `${d.name} non è raggiungibile: i comandi si attivano quando si collega. Apri Konnekta sul telefono, nella stessa rete o con il cavo.`)
        : d.bluetooth ? notice('warn', 'Collegato via Bluetooth: audio e video restano spenti finché non torni su Wi-Fi o cavo (il Bluetooth è troppo lento per il tempo reale).') : null,
      h('div', { class: 'control-grid' },
        h('div', { class: 'control-main' },
          sectionTitle('Al computer', 'dal telefono a questo computer'),
          h('div', { class: 'tiles' },
            streamTile(d, 'microphone', 'mic', 'Microfono', 'Diventa il dispositivo di ingresso «Konnekta Microphone» (Discord, Zoom, OBS…).', true),
            streamTile(d, 'camera', 'videocam', 'Webcam', 'La fotocamera del telefono come webcam del computer.', true, cameraOpts()),
            streamTile(d, 'phonescreen', 'screen', 'Schermo del telefono', 'Lo schermo del telefono in una finestra (con l\'audio, se avviato dal telefono).', true),
            streamTile(d, 'phoneaudio', 'headphones', 'Audio del telefono', 'Musica, video e giochi del telefono dalle casse di questo computer.', true)),
          sectionTitle('Dal computer', 'da questo computer al telefono'),
          h('div', { class: 'tiles' },
            streamTile(d, 'desktop', 'desktop', 'Desktop remoto', 'Il telefono vede e controlla questo schermo (servono i permessi Desktop remoto e Mouse e tastiera).', false),
            streamTile(d, 'extend', 'monitor', 'Schermo secondario', 'Il telefono diventa un altro monitor di questo computer (Hyprland o Sway).', false),
            streamTile(d, 'systemaudio', 'speaker', 'Audio del computer', 'Il telefono ascolta l\'audio riprodotto da questo computer.', false)),
          h('p', { class: 'small muted' }, 'Tutti i flussi sono cifrati end-to-end con TLS e autenticati con il certificato del dispositivo associato. ' +
            'Il telefono chiede sempre conferma prima di accendere microfono, fotocamera o cattura dello schermo.')),
        h('div', { class: 'control-side' },
          mediaCard(d), shareCard(d), remoteCommandsCard(d), notifPreview(d), recentTransfers(d))),
    ],
  };
}

// --------------------------------------------------------------- notifiche --

function replyModal(deviceId, n) {
  if (!n) return;
  const ta = h('textarea', { placeholder: 'Scrivi una risposta…' });
  modal(`Rispondi a ${n.appName}`, h('div', { class: 'col' }, h('p', { class: 'muted' }, n.title), ta),
    [['Annulla'], ['Invia', () => run(act('notifications', 'reply', { id: n.id, message: ta.value }, deviceId), 'Risposta inviata'), 'primary']]);
}

function dismissNotif(d, n) {
  return act('notifications', 'dismiss', { id: n.id }, d.id).then(() => { delete S.notifications[d.id]?.[n.id]; });
}

function notifCard(d, n) {
  return h('div', { class: 'notif' },
    appAvatar(n.appName),
    h('div', { class: 'grow' },
      h('div', { class: 'notif-app' }, n.appName),
      h('div', { class: 'notif-title' }, n.title),
      h('div', { class: 'notif-text' }, n.text),
      (n.requestReplyId || (n.actions || []).length || n.isClearable) ? h('div', { class: 'notif-actions' },
        n.requestReplyId ? btn('Rispondi', () => replyModal(d.id, n), 'small tonal', 'reply') : null,
        (n.actions || []).map((a) => btn(a, () => run(act('notifications', 'action', { id: n.id, action: a }, d.id)), 'small')),
        n.isClearable ? btn('Ignora', () => run(dismissNotif(d, n)).then(() => { updateNav(); render(); }), 'small text') : null) : null));
}

async function viewNotifications() {
  const d = cur();
  if (!d) {
    return { body: emptyState('bell', 'Nessun dispositivo', 'Associa un telefono per vedere qui le sue notifiche.', btn('Associa un dispositivo', () => go('devices'), 'primary')) };
  }
  const list = Object.values(S.notifications[d.id] || {}).reverse();
  const clearable = list.filter((n) => n.isClearable);
  return {
    sub: `Da ${d.name}`,
    narrow: true,
    actions: [clearable.length > 1 ? btn('Ignora tutte', () => Promise.all(clearable.map((n) => dismissNotif(d, n).catch(() => {})))
      .then(() => { updateNav(); render(); }), 'text', 'close') : null],
    body: [
      list.length ? h('div', { class: 'notifs' }, list.map((n) => notifCard(d, n)))
        : h('div', { class: 'card' }, emptyState('inbox', 'Tutto tranquillo', d.connected
          ? 'Le notifiche del telefono compaiono qui: rispondi, usa le loro azioni o ignorale.' : `${d.name} è offline.`)),
      h('p', { class: 'small muted' }, 'Quali notifiche arrivano (app bloccate, modalità privacy) si sceglie in Impostazioni › Notifiche.'),
    ],
  };
}

// ------------------------------------------------------------ dispositivi --

const PAIR_LABEL = { requested: 'In attesa', confirm: 'Da confermare', requestedByPeer: 'Richiesta' };

// Same names as the phone app (PERMISSION_LABELS in apps/android/.../ui/Common.kt).
const PERMISSION_LABELS = {
  ping: 'Ping', battery: 'Stato della batteria', clipboard: 'Appunti', files: 'Trasferimento file',
  notifications: 'Notifiche', telephony: 'Chiamate', mediaControl: 'Controllo multimediale',
  presenter: 'Telecomando per presentazioni', remoteInput: 'Mouse e tastiera', commands: 'Esecuzione di comandi',
  deck: 'Azioni del Mobile Deck', microphone: 'Microfono', camera: 'Fotocamera e webcam', desktop: 'Desktop remoto',
  systemAudio: 'Audio di sistema', doNotDisturb: 'Sincronizzazione Non disturbare', screenshots: 'Screenshot',
  findMyPhone: 'Trova il dispositivo', browseFiles: 'Sfoglia i file del computer', messages: 'SMS',
  approve: 'Approvazione di sudo dal telefono',
};

/** Devices found on the network, asked again at most every 5 seconds. */
function loadDiscovered() {
  const c = S.discovered;
  if (c.loading || Date.now() - c.at < 5000) return;
  c.loading = true;
  api('discovered').catch(() => []).then((list) => {
    const changed = JSON.stringify(list) !== JSON.stringify(c.list);
    S.discovered = { list, at: Date.now(), loading: false };
    if (changed) rerender('devices');
  });
}

const connectTo = (address) => run(api('connect', { address }), 'Richiesta inviata');

function devRow(d) {
  const b = S.battery[d.id];
  return h('div', { class: `dev ${d.id === S.selected ? 'sel' : ''}`, onclick: () => { S.selected = d.id; render(); } },
    devAvatar(d),
    h('div', { class: 'grow' },
      h('div', { class: 'li-title ellipsis', title: d.name }, d.name),
      h('div', { class: 'li-sub row nowrap', style: 'gap:6px' },
        `${d.connected ? 'Online' : 'Offline'}${b && b.currentCharge >= 0 ? ` · ${b.currentCharge}%` : ''}`, transportIcons(d))),
    d.paired ? (d.protocol !== 'konnekta' ? protoBadge(d.protocol) : null)
      : h('span', { class: 'chip warn' }, PAIR_LABEL[d.pairState] || 'Nuovo'));
}

function deviceListPane() {
  const saved = S.devices.filter((d) => d.paired);
  const detected = S.devices.filter((d) => !d.paired && (d.connected || (d.transports || []).length));
  const nearby = S.discovered.list.filter((x) => !S.devices.some((d) => d.address && d.address === x.address));
  const addr = draft('connect-addr', 'input', { type: 'text', placeholder: 'IP, nome.ts.net o host:porta',
    onkeydown: (e) => { if (e.key === 'Enter') connectTo(S.drafts['connect-addr']); } });
  return h('div', { class: 'card md-list', 'data-scroll': 'devlist' },
    h('div', { class: 'md-sec' }, 'I tuoi dispositivi'),
    saved.length ? saved.map(devRow) : h('div', { class: 'small muted', style: 'padding:2px 10px 6px' }, 'Nessun dispositivo associato: scegline uno tra quelli rilevati.'),
    h('div', { class: 'md-sec' }, 'Rilevati qui vicino', S.discovered.loading ? icon('sync', 14, 'spin-ic') : null),
    detected.map(devRow),
    nearby.map((x) => h('div', { class: 'dev', title: `Collega ${x.address}`, onclick: () => run(api('connect', { address: x.address }), 'Connessione in corso') },
      h('div', { class: 'avatar neutral' }, typeIcon(x.deviceType)),
      h('div', { class: 'grow' }, h('div', { class: 'li-title ellipsis' }, x.name), h('div', { class: 'li-sub ellipsis' }, x.address)),
      protoBadge(x.protocol))),
    !detected.length && !nearby.length ? h('div', { class: 'small muted', style: 'padding:2px 10px 6px' },
      'Nessun dispositivo raggiungibile ora. Apri Konnekta sul telefono nella stessa rete Wi-Fi, collegalo con il cavo o via Bluetooth (associato nel sistema), oppure scrivi il suo indirizzo (anche Tailscale).') : null,
    h('div', { class: 'md-sec' }, 'Collega con un indirizzo'),
    h('div', { class: 'input-row', style: 'padding:0 4px 4px' }, addr,
      btn('Connetti', () => connectTo(S.drafts['connect-addr']), 'tonal small')));
}

function pairCard(d) {
  const open = d.pairKey && d.pairState !== 'notPaired';
  return h('div', { class: 'card' },
    cardHead('link', 'Associazione'),
    open ? h('div', {},
      h('p', { style: 'text-align:center' }, d.pairState === 'requestedByPeer' ? 'Il dispositivo chiede di associarsi. Codice:'
        : d.pairState === 'confirm' ? 'Il dispositivo ha accettato. Confermi che mostra questo codice?'
          : 'In attesa che il dispositivo accetti. Codice:'),
      keyBlocks(d.pairKey),
      d.pairState !== 'requested' ? h('div', { class: 'row', style: 'justify-content:center;margin-bottom:18px' },
        btn('Rifiuta', () => run(api('rejectPair', { deviceId: d.id, key: d.pairKey })), 'danger'),
        btn(d.pairState === 'confirm' ? 'Conferma' : 'Accetta', () => run(api('acceptPair', { deviceId: d.id, key: d.pairKey })), 'primary')) : null) : null,
    notice('info', 'Il dispositivo non è associato: finché non lo associ e confermi il codice di verifica, non può scambiare alcun dato con questo computer.', 'lock'),
    d.fingerprint ? h('div', { style: 'margin-top:16px' }, h('div', { class: 'set-title', style: 'margin-bottom:8px' }, 'Impronta del certificato'), fingerprintGrid(d.fingerprint)) : null);
}

function connectionTab(d) {
  const avail = d.transports || [];
  const pref = d.preferredTransport || '';
  const choose = (t) => run(api('setTransport', { deviceId: d.id, transport: t || null }),
    t ? `Userò ${TRANSPORT_LABELS[t]} appena disponibile` : 'Collegamento automatico').then(refreshDevices);
  const state = (t) => d.connected && d.transport === t ? 'In uso' : avail.includes(t) ? 'Disponibile' : 'Non disponibile';
  const choice = (t, title, sub, ic) => h('button', { class: `choice ${pref === t ? 'active' : ''}`, onclick: () => choose(t) },
    h('div', { class: `avatar ${pref === t ? '' : 'neutral'}` }, icon(ic, 20)),
    h('div', { class: 'grow' }, h('div', { class: 'choice-title' }, title), h('div', { class: 'choice-sub' }, sub)));
  const k = `addrs-${d.id}`;
  const addrs = draft(k, 'textarea', { placeholder: 'Uno per riga, es. pixel.tail1234.ts.net oppure 100.101.102.103' }, (d.extraAddresses || []).join('\n'));
  return h('div', { class: 'col', style: 'gap:22px' },
    h('div', {},
      h('div', { class: 'set-title' }, 'Collegamento'),
      h('div', { class: 'set-desc', style: 'margin-bottom:12px' }, 'Come raggiungere il dispositivo; la scelta è ricordata su entrambi i lati.'),
      h('div', { class: 'choices' },
        choice('', 'Automatico', 'Il più veloce', 'sync'),
        choice('wifi', 'Wi-Fi', state('wifi'), 'wifi'),
        choice('bluetooth', 'Bluetooth', state('bluetooth'), 'bluetooth'),
        choice('usb', 'Cavo USB', state('usb'), 'usb'))),
    d.protocol === 'konnekta' ? h('div', {},
      h('div', { class: 'set-title' }, 'Raggiungibile anche fuori casa'),
      h('div', { class: 'set-desc', style: 'margin-bottom:12px' }, 'Indirizzi che questo computer prova quando il dispositivo non è sulla rete locale, per esempio il nome o l\'IP Tailscale. È sempre il computer ad aprire la connessione, quindi non servono porte aperte in ingresso.'),
      addrs,
      h('div', { class: 'card-foot', style: 'margin-top:12px' }, btn('Salva indirizzi', () => run(api('setExtraAddresses', {
        deviceId: d.id, addresses: (S.drafts[k] || '').split('\n').map((x) => x.trim()).filter(Boolean),
      }), 'Indirizzi salvati').then(() => { delete S.drafts[k]; refreshDevices(); }), 'primary'))) : null);
}

async function permissionsTab(d) {
  const perms = await api('permissions');
  const set = (p, v) => {
    // Shown at once; the device update that follows confirms it.
    d.permissions = v ? [...d.permissions, p.id] : d.permissions.filter((x) => x !== p.id);
    run(api('setPermission', { deviceId: d.id, permission: p.id, granted: v })).catch(() => refreshDevices());
  };
  return h('div', {},
    h('p', { class: 'small muted', style: 'margin-bottom:8px' }, 'Scegli cosa può fare questo dispositivo con il computer. Le autorizzazioni sensibili sono disattivate per impostazione predefinita.'),
    perms.map((p) => setRow(h('span', { class: 'row', style: 'gap:8px' }, PERMISSION_LABELS[p.id] || p.label, p.sensitive ? h('span', { class: 'chip warn' }, 'sensibile') : null), null,
      sw(d.permissions.includes(p.id), (v) => set(p, v)))));
}

async function devicePluginsTab(d) {
  const plugins = await api('plugins');
  const set = (p, v) => {
    d.disabledPlugins = v ? d.disabledPlugins.filter((x) => x !== p.id) : [...d.disabledPlugins, p.id];
    run(api('setDevicePlugin', { deviceId: d.id, plugin: p.id, enabled: v })).catch(() => refreshDevices());
  };
  return h('div', {},
    h('p', { class: 'small muted', style: 'margin-bottom:8px' }, 'Disattiva singole funzioni solo per questo dispositivo.'),
    plugins.map((p) => setRow(p.name, interopNote(p.interop).replace(/^ · /, '') || null, sw(!d.disabledPlugins.includes(p.id), (v) => set(p, v)))));
}

function infoTab(d) {
  const row = (label, value) => setRow(label, null, h('div', { style: 'text-align:right;max-width:60%' }, value));
  return h('div', {},
    row('Protocollo', h('span', { class: 'row', style: 'gap:8px;justify-content:flex-end' }, protoBadge(d.protocol), d.app ? h('span', { class: 'small muted' }, d.app) : null)),
    row('ID', h('span', { class: 'mono' }, d.id)),
    row('Capacità', h('span', { class: 'small muted' }, `${d.incomingCapabilities.length} in ingresso · ${d.outgoingCapabilities.length} in uscita`)),
    h('div', { class: 'set-row', style: 'flex-direction:column;align-items:stretch;gap:10px' },
      h('div', { class: 'row' }, h('div', { class: 'set-title grow' }, 'Impronta del certificato (SHA-256)'),
        d.fingerprint ? btn('Copia', () => copyText(d.fingerprint), 'text small', 'copy') : null),
      fingerprintGrid(d.fingerprint),
      h('div', { class: 'small muted' }, 'Il certificato è fissato (pinning): se cambia, la connessione viene rifiutata.')));
}

const DEVICE_TABS = [['connection', 'Collegamento', 'link'], ['permissions', 'Permessi', 'lock'], ['plugins', 'Plugin', 'extension'], ['info', 'Info', 'info']];

async function deviceDetail(d) {
  const head = h('div', { class: 'card' }, h('div', { class: 'detail-head' },
    devAvatar(d, 34),
    h('div', { class: 'grow' },
      h('div', { class: 'detail-name ellipsis', title: d.name }, d.name),
      h('div', { class: 'row small muted', style: 'gap:8px;margin-top:6px' },
        h('span', { class: `dot ${d.connected ? 'on' : ''}` }),
        d.connected ? `Connesso · ${TRANSPORT_LABELS[d.transport] || d.address || 'rete'}` : 'Non raggiungibile',
        protoBadge(d.protocol), transportChips(d), batteryMeter(S.battery[d.id]))),
    h('div', { class: 'row' },
      d.paired ? btn('Dissocia', () => confirmModal('Dissociare il dispositivo?',
        `${d.name} non potrà più comunicare con questo computer finché non verrà associato di nuovo.`, 'Dissocia',
        () => run(api('unpair', { deviceId: d.id }))), 'danger', 'link_off') : null,
      d.paired ? btn('Apri in Controllo', () => { S.current = d.id; PREF.set('current', d.id); go('control'); }, 'tonal', 'tune')
        : d.connected && d.pairState === 'notPaired' ? btn('Associa', () => run(api('pair', { deviceId: d.id })), 'primary', 'link') : null)));
  if (!d.paired) return [head, pairCard(d)];
  if (!DEVICE_TABS.some(([id]) => id === S.devTab)) S.devTab = 'connection';
  const tabs = h('div', { class: 'tabs', style: 'padding:0 8px' }, DEVICE_TABS.map(([id, label, ic]) =>
    h('button', { class: S.devTab === id ? 'active' : '', onclick: () => { S.devTab = id; render(); } }, icon(ic, 18), label)));
  const body = S.devTab === 'permissions' ? await permissionsTab(d)
    : S.devTab === 'plugins' ? await devicePluginsTab(d)
      : S.devTab === 'info' ? infoTab(d) : connectionTab(d);
  return [head, h('div', { class: 'card flush' }, tabs, h('div', { style: 'padding:20px 22px' }, body))];
}

async function viewDevices() {
  if (!S.selected || !S.devices.some((x) => x.id === S.selected)) S.selected = cur()?.id || S.devices[0]?.id || null;
  const d = S.devices.find((x) => x.id === S.selected);
  loadDiscovered();
  const saved = S.devices.filter((x) => x.paired).length;
  return {
    sub: saved === 1 ? '1 dispositivo associato' : `${saved} dispositivi associati`,
    actions: [btn('Cerca di nuovo', () => run(api('refresh')).then(() => { S.discovered.at = 0; refreshDevices(); }), 'text', 'sync')],
    body: h('div', { class: 'md' },
      deviceListPane(),
      h('div', { class: 'col', style: 'gap:16px;min-width:0' }, d ? await deviceDetail(d)
        : h('div', { class: 'card' }, emptyState('devices', 'Collega il tuo telefono',
          'Installa Konnekta sul telefono e aprilo: comparirà qui tra i dispositivi rilevati. Sceglilo e confronta il codice per associarlo.')))),
  };
}

// ------------------------------------------------------------------- invia --

/** Open a received file, or the folder that holds it. */
const openPath = (path, reveal = false) => run(invoke('open_path', { path: String(path), reveal }));
const openButtons = (path) => h('div', { class: 'li-actions' },
  iconBtn('openNew', 'Apri', () => openPath(path), 'sm'),
  iconBtn('folder', 'Mostra nella cartella', () => openPath(path, true), 'sm'));

function addReceived(r) {
  S.received.unshift(r);
  S.received = S.received.slice(0, 50);
}

const lsAct = (action, args = null) => act('localsend', action, args);

async function lsRefresh() {
  try {
    const [status, peers, transfers] = await Promise.all([lsAct('status'), lsAct('peers'), lsAct('transfers')]);
    S.ls.status = status;
    S.ls.peers = peers;
    for (const t of transfers) S.ls.transfers.set(t.id, t);
  } catch { S.ls.status = null; /* plugin disabled */ }
}

function onLocalSend(kind, data) {
  switch (kind) {
    case 'status': S.ls.status = data; break;
    case 'peers': S.ls.peers = data.peers; break;
    case 'transfer': {
      const before = S.ls.transfers.get(data.id);
      S.ls.transfers.set(data.id, data);
      if (before?.state !== data.state) {
        if (data.state === 'rejected') toast(`${data.peer} ha rifiutato i file`, 'warn');
        if (data.state === 'completed' && data.direction === 'outgoing') toast(`Inviato a ${data.peer}`);
      }
      updateNav();
      break;
    }
    case 'received':
      addReceived({ name: data.name, path: data.path, from: data.from, via: 'LocalSend', at: Date.now() });
      toast(`Ricevuto ${data.name} da ${data.from}`, '', [['Apri', () => openPath(data.path)], ['Mostra', () => openPath(data.path, true)]]);
      break;
    case 'request': lsRequestModal(data); return;
    case 'requestClosed':
      if (S.ls.request === data.id) { S.ls.request = null; closeModals(); }
      return;
    case 'text':
      modal(`Messaggio da ${data.from}`, h('p', { class: 'selectable', style: 'white-space:pre-wrap;color:var(--text)' }, data.text),
        [['Chiudi'], ['Copia', () => copyText(data.text), 'primary']]);
      return;
    case 'scanDone': S.ls.scanning = false; break;
  }
  rerender('send');
}

function lsRequestModal(r) {
  S.ls.request = r.id;
  const list = h('div', { class: 'list', style: 'max-height:260px;overflow:auto' }, r.files.map((f) => h('div', { class: 'li', style: 'padding:10px 14px' },
    icon(fileIcon(f.mime), 20), h('div', { class: 'grow ellipsis', style: 'font-weight:600' }, f.name),
    h('span', { class: 'small muted' }, fmtBytes(f.size)))));
  modal(`${r.from} vuole inviarti ${r.files.length === 1 ? 'un file' : `${r.files.length} file`}`, h('div', { class: 'col' },
    h('div', { class: 'chips' }, h('span', { class: 'chip ok' }, 'LocalSend'),
      h('span', { class: 'small muted' }, [r.deviceModel, r.ip, fmtBytes(r.total)].filter(Boolean).join(' · '))),
    list,
    h('p', { class: 'small muted' }, 'Accetta solo se aspetti questi file. Verranno salvati nella cartella dei file ricevuti.')),
  [['Rifiuta', () => lsAct('reject', { id: r.id }).catch(() => {}), 'danger'],
   ['Accetta', () => run(lsAct('accept', { id: r.id })), 'primary']]);
}

const fileIcon = (mime = '') => mime.startsWith('image/') ? 'image' : mime.startsWith('text/') ? 'text'
  : mime.startsWith('video/') ? 'videocam' : mime.startsWith('audio/') ? 'headphones' : 'file';
const baseName = (p) => String(p).split(/[\\/]/).pop();
const KIND = { file: ['file', 'File'], folder: ['folder', 'Cartella'], text: ['text', 'Testo'], clipboard: ['paste', 'Appunti'] };

function addToSelection(items) {
  S.sel.push(...items);
  updateNav();
  if (S.view === 'send') render();
}

function addPick(kind) {
  if (kind === 'file') invoke('pick_files').then((paths) => addToSelection(paths.map((path) => ({ kind: 'file', path, name: baseName(path) }))));
  if (kind === 'folder') invoke('pick_folder').then((path) => path && addToSelection([{ kind: 'folder', path, name: baseName(path) }]));
  if (kind === 'clipboard') addToSelection([{ kind: 'clipboard', name: 'Appunti' }]);
  if (kind === 'text') {
    const ta = h('textarea', { placeholder: 'Testo o link…', style: 'min-height:140px' });
    modal('Testo da inviare', ta, [['Annulla'], ['Aggiungi', () => {
      if (!ta.value.trim()) return false;
      addToSelection([{ kind: 'text', text: ta.value, name: ta.value.slice(0, 40) }]);
      return true;
    }, 'primary']]);
  }
}

const PICKERS = [['file', 'file', 'File'], ['folder', 'folder', 'Cartella'], ['text', 'text', 'Testo'], ['clipboard', 'paste', 'Appunti']];

function selectionCard() {
  if (!S.sel.length) {
    return h('div', { class: 'dropzone' },
      h('div', { class: 'empty-ic' }, icon('upload', 34)),
      h('div', { class: 'dropzone-title' }, 'Trascina qui file e cartelle'),
      h('div', { class: 'small muted' }, 'oppure scegli cosa inviare'),
      h('div', { class: 'pickers' }, PICKERS.map(([k, ic, label]) =>
        h('button', { class: 'picker', onclick: () => addPick(k) }, h('div', { class: 'p-ic' }, icon(ic, 24)), label))));
  }
  const files = S.sel.filter((x) => x.kind === 'file' || x.kind === 'folder').length;
  return h('div', { class: 'card' },
    cardHead('upload', 'Selezione', `${S.sel.length === 1 ? '1 elemento' : `${S.sel.length} elementi`}${files ? ` · ${files} tra file e cartelle` : ''}`,
      btn('Svuota', () => { S.sel = []; updateNav(); render(); }, 'text small', 'close')),
    h('div', { class: 'thumbs' }, S.sel.map((x, i) => h('div', { class: 'thumb', title: x.path || x.text || x.name },
      h('div', { class: 'thumb-ic' }, icon(KIND[x.kind][0], 24)),
      h('div', { class: 'thumb-name' }, x.name),
      h('div', { class: 'thumb-kind' }, KIND[x.kind][1]),
      h('button', { class: 'thumb-x', title: 'Togli', onclick: () => { S.sel.splice(i, 1); updateNav(); render(); } }, icon('close', 16))))),
    h('div', { class: 'row', style: 'margin-top:14px' },
      h('span', { class: 'small muted grow' }, 'Aggiungi altro:'),
      PICKERS.map(([k, ic, label]) => btn(label, () => addPick(k), 'small', ic))));
}

/** A nearby device as LocalSend shows it: icon, name, tags. */
function peerTile({ ic, name, tags, onclick, extra }) {
  return h('div', { class: 'peer', onclick, title: `Invia a ${name}` },
    h('div', { class: 'avatar' }, icon(ic, 26)),
    h('div', { class: 'grow' },
      h('div', { class: 'peer-name ellipsis' }, name),
      h('div', { class: 'chips' }, tags.filter(Boolean).map(([t, cls]) => h('span', { class: `chip ${cls || ''}` }, t)))),
    extra || null,
    h('div', { class: 'peer-send' }, icon('send', 20)));
}

const lastOctet = (ip) => ip ? `#${String(ip).split('.').pop()}` : null;

/**
 * Selection items on their way to a Konnekta device: { item, deviceId,
 * path } of the transfer that carries each. Arrived: the item leaves the
 * selection. Failed or cancelled: it stays, to send again.
 */
const sentWatch = [];

/** Take what arrived out of the selection. */
function unselect(items) {
  if (!items.length) return;
  S.sel = S.sel.filter((x) => !items.includes(x));
  updateNav();
  rerender('send');
}

/** A Konnekta transfer changed: settle the selection item it carries. */
function settleSent(t) {
  if (t.direction !== 'outgoing') return;
  const i = sentWatch.findIndex((w) => w.deviceId === t.deviceId && w.path === t.path);
  if (i < 0) return;
  const [w] = t.state === 'completed' || t.state === 'failed' || t.state === 'cancelled' ? sentWatch.splice(i, 1) : [];
  if (t.state === 'completed') unselect([w.item]);
}

async function sendToKonnekta(d) {
  // Texts, links and the clipboard arrive with the packet; files and
  // folders when their transfer completes.
  const arrived = [];
  try {
    for (const x of S.sel) {
      if (x.kind === 'folder') {
        const r = await act('share', 'sendFolder', { path: x.path }, d.id);
        if (r?.path) {
          sentWatch.push({ item: x, deviceId: d.id, path: r.path });
          // A tiny folder may be there before its path came back.
          const t = [...S.transfers.values()].find((t) => t.direction === 'outgoing' && t.deviceId === d.id && t.path === r.path);
          if (t) settleSent(t);
        }
      }
      if (x.kind === 'clipboard') {
        await act('clipboard', 'send', null, d.id);
        arrived.push(x);
      }
      if (x.kind === 'text') {
        const t = x.text.trim();
        await (/^https?:\/\/\S+$/.test(t) ? act('share', 'sendUrl', { url: t }, d.id) : act('share', 'sendText', { text: x.text }, d.id));
        arrived.push(x);
      }
    }
    const files = S.sel.filter((x) => x.kind === 'file');
    if (files.length) {
      const watch = files.map((x) => ({ item: x, deviceId: d.id, path: x.path }));
      sentWatch.push(...watch);
      try {
        await act('share', 'sendFiles', { paths: files.map((x) => x.path) }, d.id);
      } catch (e) {
        for (const w of watch) sentWatch.splice(sentWatch.indexOf(w), 1);
        throw e;
      }
    }
  } finally {
    unselect(arrived);
  }
}

async function sendToLocalSend(p, pin) {
  // LocalSend sends a message on its own, then the files; each send returns once it arrived.
  const arrived = [];
  const done = (r, items) => { if (r?.state === 'completed') arrived.push(...items); };
  try {
    for (const x of S.sel) {
      let text = null;
      if (x.kind === 'text') text = x.text;
      if (x.kind === 'clipboard') {
        try { text = await navigator.clipboard.readText(); } catch { throw new Error('Non riesco a leggere gli appunti: incolla il testo con "Testo"'); }
      }
      if (text) done(await lsAct('send', { peerId: p.id, text, pin }), [x]);
    }
    const files = S.sel.filter((x) => x.kind === 'file' || x.kind === 'folder');
    if (files.length) done(await lsAct('send', { peerId: p.id, paths: files.map((x) => x.path), pin }), files);
  } finally {
    unselect(arrived);
  }
}

async function sendTo(target) {
  if (!S.sel.length) { toast('Scegli prima cosa inviare: file, cartella, testo o appunti', 'warn'); return; }
  try {
    if (target.konnekta) await sendToKonnekta(target.konnekta);
    else await sendToLocalSend(target.peer);
    toast(target.konnekta ? `Invio a ${target.konnekta.name} avviato` : `Inviato a ${target.peer.alias}`);
  } catch (e) {
    if (target.peer && String(e).includes('PIN')) {
      const pin = h('input', { type: 'text', placeholder: 'PIN' });
      modal(`PIN per ${target.peer.alias}`, h('div', { class: 'col' }, h('p', { class: 'muted' }, 'Questo dispositivo chiede un PIN per ricevere.'), pin),
        [['Annulla'], ['Invia', () => run(sendToLocalSend(target.peer, pin.value), `Inviato a ${target.peer.alias}`), 'primary']]);
      return;
    }
    toast(String(e), 'err');
  }
}

const enableLocalSend = () => run(lsAct('setConfig', { enabled: true }), 'LocalSend attivo').then(lsRefresh).then(render);

function lsOffCard() {
  return h('div', { class: 'card tonal row nowrap' },
    h('div', { class: 'avatar sec' }, icon('public', 22)),
    h('div', { class: 'grow' }, h('div', { class: 'set-title' }, 'Compatibilità LocalSend'),
      h('div', { class: 'set-desc' }, 'Attivala per vedere qui anche i telefoni e i computer con l\'app LocalSend e scambiare file con loro.')),
    btn('Attiva', enableLocalSend, 'primary'));
}

function activeTransfers() {
  const list = [
    ...[...S.ls.transfers.values()].filter((t) => ['waiting', 'inProgress'].includes(t.state)).map((t) => ({ ...t, ls: true })),
    ...[...S.transfers.values()].filter((t) => t.state === 'inProgress').map((t) => ({ id: t.id, peer: devName(t.deviceId), direction: t.direction, total: t.size, transferred: t.transferred, files: [{ name: t.name }], state: t.state })),
  ];
  if (!list.length) return null;
  return h('div', { class: 'card flush' },
    h('div', { style: 'padding:18px 20px 4px' }, cardHead('swap', 'In corso')),
    h('div', { class: 'list plain' }, list.map((t) => {
      const pct = t.total ? Math.round((t.transferred / t.total) * 100) : 0;
      return h('div', { class: 'li' },
        h('div', { class: `avatar ${t.direction === 'incoming' ? 'sec' : ''}` }, icon(t.direction === 'incoming' ? 'download' : 'upload', 20)),
        h('div', { class: 'grow' },
          h('div', { class: 'row nowrap', style: 'gap:8px' }, h('span', { class: 'li-title ellipsis' }, t.files.length === 1 ? t.files[0].name : `${t.files.length} file`),
            h('span', { class: 'small muted' }, `${t.direction === 'incoming' ? 'da' : 'a'} ${t.peer}`)),
          h('div', { class: 'li-sub' }, t.state === 'waiting' ? 'In attesa che accetti…' : `${fmtBytes(t.transferred)} / ${fmtBytes(t.total)} · ${pct}%`),
          h('div', { class: 'progress', style: 'margin-top:8px' }, h('div', { style: `width:${pct}%` }))),
        t.ls ? btn('Annulla', () => run(lsAct('cancel', { id: t.id })), 'small danger') : null);
    })));
}

function sendPane() {
  const konnekta = S.devices.filter((d) => d.paired && d.connected);
  const st = S.ls.status;
  const lsOn = st?.enabled;
  const tiles = [
    ...konnekta.map((d) => peerTile({
      ic: TYPE_ICON[d.deviceType] || 'smartphone',
      name: d.name,
      tags: [['Konnekta', 'k'], d.transport ? [TRANSPORT_LABELS[d.transport] || d.transport] : null,
        S.battery[d.id]?.currentCharge >= 0 ? [`${S.battery[d.id].currentCharge}%`] : null],
      onclick: () => sendTo({ konnekta: d }),
      extra: h('button', { class: 'icon-btn sm accent', title: `Ping a ${d.name}`, onclick: (e) => { e.stopPropagation(); run(act('ping', 'send', { message: 'Ping da Konnekta' }, d.id), `Ping inviato a ${d.name}`); } }, icon('bell', 18)),
    })),
    ...(lsOn ? S.ls.peers.map((p) => peerTile({
      ic: p.deviceType === 'mobile' ? 'smartphone' : p.deviceType === 'web' ? 'public' : 'desktop',
      name: p.alias,
      tags: [['LocalSend', 'ok'], p.ip ? [lastOctet(p.ip)] : null, p.deviceModel ? [p.deviceModel] : null],
      onclick: () => sendTo({ peer: p }),
    })) : []),
  ];
  const manual = () => {
    const ip = h('input', { type: 'text', placeholder: '192.168.1.20 oppure 192.168.1.20:53317' });
    modal('Invia a un indirizzo', h('div', { class: 'col' }, h('p', { class: 'muted' }, 'L\'indirizzo IP di un dispositivo con LocalSend (lo trovi nella sua pagina Ricevi).'), ip),
      [['Annulla'], ['Aggiungi', () => run(lsAct('addPeer', { address: ip.value }), 'Dispositivo trovato').then(lsRefresh).then(render), 'primary']]);
  };
  const scan = () => {
    S.ls.scanning = true;
    render();
    lsAct('scan').catch((e) => { S.ls.scanning = false; toast(String(e), 'err'); render(); });
    refreshDevices();
  };
  return [
    selectionCard(),
    sectionTitle('Dispositivi vicini', S.sel.length ? 'scegli a chi inviare' : null,
      lsOn ? h('button', { class: `icon-btn sm ${S.ls.scanning ? 'spin' : ''}`, title: 'Cerca di nuovo', onclick: scan }, icon('sync', 20))
        : iconBtn('sync', 'Aggiorna', () => run(api('refresh')).then(refreshDevices), 'sm'),
      lsOn ? iconBtn('target', 'Invia a un indirizzo', manual, 'sm') : null),
    h('div', { class: 'peers' }, tiles.length ? tiles
      : h('div', { class: 'peer placeholder' }, h('div', { class: 'avatar neutral' }, icon('devices', 26)),
        h('div', { class: 'grow' }, h('div', { class: 'peer-name' }, 'Nessun dispositivo vicino'),
          h('div', { class: 'small muted' }, 'Apri Konnekta (o LocalSend) sull\'altro dispositivo, nella stessa rete.')))),
    st && !lsOn ? lsOffCard() : null,
    st?.enabled && st.error ? notice('warn', st.error) : null,
    activeTransfers(),
    h('p', { class: 'small muted' }, 'Scegli cosa inviare, poi il dispositivo. I dispositivi Konnekta ricevono subito; quelli con LocalSend chiedono conferma.'),
  ];
}

function receivePane() {
  const st = S.ls.status;
  const ips = st?.ips || [];
  const quick = (on) => run(lsAct('setConfig', { autoAccept: on })).then(lsRefresh).then(render);
  return h('div', { class: 'receive' },
    h('div', { class: 'card radar' },
      // The app mark, larger; the rings mean "ready to receive".
      h('div', { class: 'radar-logo' }, logo()),
      h('div', { class: 'radar-name' }, S.me?.deviceName || '—'),
      h('div', { class: 'chips', style: 'justify-content:center' }, ips.map((ip) => h('span', { class: 'chip', title: ip }, lastOctet(ip)))),
      h('div', { class: 'status' },
        h('div', { class: 'row' }, h('span', { class: 'dot on' }), h('span', { class: 'grow' }, 'Konnekta: pronto a ricevere dai dispositivi associati')),
        h('div', { class: 'row' }, h('span', { class: `dot ${st?.running ? 'on' : ''}` }),
          h('span', { class: 'grow' }, !st ? 'LocalSend: plugin disattivato' : !st.enabled ? 'LocalSend: disattivato'
            : st.running ? `LocalSend: visibile come «${st.alias}» (porta ${st.port}, ${st.https ? 'HTTPS' : 'HTTP'})` : `LocalSend: ${st.error || 'avvio…'}`),
          st && !st.enabled ? btn('Attiva', enableLocalSend, 'small tonal') : null)),
      st?.enabled ? h('div', { style: 'margin-top:20px;width:100%' },
        h('div', { class: 'set-title', style: 'margin-bottom:8px;text-align:left' }, 'Ricezione rapida (LocalSend)'),
        h('div', { class: 'seg', style: 'width:100%' },
          h('button', { class: st.autoAccept ? '' : 'active', style: 'flex:1;justify-content:center', onclick: () => quick(false) }, st.autoAccept ? null : icon('check', 18), 'Chiedi sempre'),
          h('button', { class: st.autoAccept ? 'active' : '', style: 'flex:1;justify-content:center', onclick: () => quick(true) }, st.autoAccept ? icon('check', 18) : null, 'Accetta tutto'))) : null),
    h('div', { class: 'col', style: 'gap:16px;min-width:0' },
      activeTransfers(),
      h('div', { class: 'card flush' },
        h('div', { style: 'padding:18px 20px 4px' }, cardHead('download', 'Ricevuti', 'in questa sessione',
          S.received.length ? btn('Svuota', () => { S.received = []; render(); }, 'text small') : null)),
        S.received.length ? h('div', { class: 'list plain' }, S.received.map((r) => h('div', { class: 'li' },
          h('div', { class: 'avatar sec' }, icon('download', 20)),
          h('div', { class: 'grow' }, h('div', { class: 'li-title ellipsis' }, r.name),
            h('div', { class: 'li-sub' }, `da ${r.from} · ${r.via} · ${new Date(r.at).toLocaleTimeString()}`)),
          r.path ? openButtons(r.path) : null)))
          : h('div', { style: 'padding:0 20px 20px' }, h('p', { class: 'small muted' }, 'I file che ricevi compaiono qui: aprili con un clic.')))));
}

async function viewSend() {
  await lsRefresh();
  const seg = h('div', { class: 'seg' },
    h('button', { class: S.sendMode === 'send' ? 'active' : '', onclick: () => { S.sendMode = 'send'; render(); } }, icon('upload', 18), 'Invia'),
    h('button', { class: S.sendMode === 'receive' ? 'active' : '', onclick: () => { S.sendMode = 'receive'; render(); } }, icon('download', 18), 'Ricevi'));
  return {
    sub: S.sendMode === 'send' ? 'Come LocalSend: scegli cosa, poi a chi' : `Visibile come ${S.me?.deviceName || 'questo computer'}`,
    actions: [seg],
    body: S.sendMode === 'send' ? sendPane() : receivePane(),
  };
}

/** Files and folders to send (from the file manager or dropped on the window): Invia opens with them selected. */
function queueToSend(items) {
  if (!items.length) return;
  S.sel.push(...items.map((x) => ({ kind: x.dir ? 'folder' : 'file', path: x.path, name: x.name })));
  S.sendMode = 'send';
  go('send');
  toast(items.length === 1 ? `${items[0].name}: scegli a quale dispositivo inviarlo` : `${items.length} elementi: scegli a quale dispositivo inviarli`);
}

/** Files chosen with "Invia tramite Konnekta" in the file manager. */
async function takeSend() {
  queueToSend(await invoke('take_send').catch(() => []));
}

// Dropping files anywhere on the window puts them in the selection.
async function setupDrop() {
  const ov = $('drop');
  ov.querySelector('.drop-ic').replaceChildren(icon('upload', 48));
  const show = (on) => ov.classList.toggle('on', on);
  const dropped = async (paths) => {
    show(false);
    if (!paths?.length) return;
    const items = await invoke('inspect_paths', { paths }).catch(() => paths.map((path) => ({ path, name: baseName(path), dir: false })));
    queueToSend(items);
  };
  const onDrag = ({ type, paths }) => {
    if (type === 'enter' || type === 'over') show(true);
    else if (type === 'leave') show(false);
    else if (type === 'drop') dropped(paths);
  };
  try {
    const wv = T.webview?.getCurrentWebview?.();
    if (wv?.onDragDropEvent) {
      await wv.onDragDropEvent((e) => onDrag(e.payload));
      return;
    }
  } catch { /* fall back to the raw events */ }
  try {
    await T.event.listen('tauri://drag-enter', () => show(true));
    await T.event.listen('tauri://drag-over', () => show(true));
    await T.event.listen('tauri://drag-leave', () => show(false));
    await T.event.listen('tauri://drag-drop', (e) => dropped(e.payload?.paths));
  } catch { /* no drag and drop here */ }
}

// ---------------------------------------------------------- trasferimenti --

const STATE_LABEL = { inProgress: 'In corso', completed: 'Completato', interrupted: 'Interrotto', failed: 'Non riuscito', cancelled: 'Annullato' };
const STATE_CHIP = { completed: 'ok', inProgress: 'k', failed: 'err' };

function transferRow(t) {
  const pct = t.size ? Math.round((t.transferred / t.size) * 100) : 0;
  return h('div', { class: 'li' },
    h('div', { class: `avatar ${t.direction === 'incoming' ? 'sec' : ''}` }, icon(t.direction === 'incoming' ? 'download' : 'upload', 20)),
    h('div', { class: 'grow' },
      h('div', { class: 'row nowrap', style: 'gap:8px' }, h('span', { class: 'li-title ellipsis', title: t.name }, t.name),
        h('span', { class: `chip ${STATE_CHIP[t.state] || 'warn'}` }, STATE_LABEL[t.state] || t.state),
        t.resumed ? h('span', { class: 'chip' }, 'ripreso') : null),
      h('div', { class: 'li-sub ellipsis' }, `${t.direction === 'incoming' ? 'Da' : 'A'} ${devName(t.deviceId)} · ${fmtBytes(t.transferred)} / ${fmtBytes(t.size)}`, t.error ? ` · ${t.error}` : ''),
      t.state === 'inProgress' ? h('div', { class: 'progress', style: 'margin-top:8px' }, h('div', { style: `width:${pct}%` })) : null),
    t.direction === 'outgoing' && ['failed', 'interrupted'].includes(t.state)
      ? btn('Riprova', () => run(act('share', 'retry', { id: t.id }, t.deviceId)), 'small tonal', 'sync') : null,
    t.direction === 'incoming' && t.state === 'completed' && t.path ? openButtons(t.path) : null);
}

const TR_FILTERS = [
  ['all', 'Tutti', () => true],
  ['active', 'In corso', (t) => t.state === 'inProgress'],
  ['done', 'Completati', (t) => t.state === 'completed'],
  ['problems', 'Problemi', (t) => ['failed', 'interrupted', 'cancelled'].includes(t.state)],
];

async function viewTransfers() {
  try {
    for (const t of await act('share', 'transfers')) S.transfers.set(t.id, t);
  } catch { /* plugin disabled */ }
  const all = [...S.transfers.values()].sort((a, b) => b.startedAt - a.startedAt);
  const [, , pass] = TR_FILTERS.find(([id]) => id === S.trFilter) || TR_FILTERS[0];
  const list = all.filter(pass);
  return {
    sub: all.length ? `${all.length} in questa sessione` : null,
    narrow: true,
    actions: [
      h('div', { class: 'seg' }, TR_FILTERS.map(([id, label]) =>
        h('button', { class: S.trFilter === id ? 'active' : '', onclick: () => { S.trFilter = id; render(); } }, label))),
      btn('Pulisci completati', () => run(act('share', 'clearFinished')).then(() => { S.transfers.clear(); render(); }), 'text', 'delete'),
    ],
    body: list.length ? h('div', { class: 'list' }, list.map(transferRow))
      : h('div', { class: 'card' }, emptyState('swap', all.length ? 'Niente in questo filtro' : 'Nessun trasferimento',
        'Invia file dalla pagina Invia o trascinali sulla finestra; quelli che arrivano dal telefono compaiono qui.',
        btn('Vai a Invia', () => go('send'), 'tonal', 'send'))),
  };
}

// ---------------------------------------------- editor delle azioni (comandi e deck) --

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
        return h('div', { class: 'action-row' },
          h('label', { class: 'field' }, `Azione ${i + 1}`, typeSel),
          h('div', { class: 'action-fields' }, inputs),
          h('div', { class: 'action-tools' },
            iconBtn('arrow_up', 'Su', () => { if (i) { [actions[i - 1], actions[i]] = [actions[i], actions[i - 1]]; draw(); } }, 'sm'),
            iconBtn('arrow_down', 'Giù', () => { if (i < actions.length - 1) { [actions[i + 1], actions[i]] = [actions[i], actions[i + 1]]; draw(); } }, 'sm'),
            iconBtn('delete', 'Togli', () => { actions.splice(i, 1); draw(); }, 'sm danger')));
      }),
      btn('Aggiungi azione', () => { actions.push({ type: 'shortcut', keys: '' }); draw(); }, 'small', 'add'));
  };
  draw();
  return box;
}

// ----------------------------------------------------------------- comandi --

function commandModal(cmd) {
  const c = clone(cmd || { name: '', description: '', icon: '', group: '', notify: false, actions: [{ type: 'shell', command: '' }] });
  const f = (key, label, ph = '') => h('label', { class: 'field' }, label,
    h('input', { type: 'text', value: c[key] || '', placeholder: ph, oninput: (e) => { c[key] = e.target.value; } }));
  modal(cmd ? 'Modifica comando' : 'Nuovo comando', h('div', { class: 'col' },
    h('div', { class: 'grid2' }, f('name', 'Nome', 'Blocca schermo'), f('group', 'Gruppo', 'Sistema'),
      f('icon', 'Icona (emoji o nome)', '🔒'), f('description', 'Descrizione')),
    setRow('Mostra una notifica con l\'esito', null, sw(c.notify, (v) => { c.notify = v; })),
    h('div', { class: 'set-title' }, 'Azioni (eseguite in sequenza)'),
    actionsEditor(c.actions),
    h('p', { class: 'small muted' }, 'I dispositivi associati ricevono solo nome e descrizione e possono avviare il comando solo con il permesso «Esecuzione di comandi».')),
  [['Annulla'], ['Salva', async () => {
    await run(act('commands', cmd ? 'update' : 'add', c), 'Comando salvato');
    render();
  }, 'primary']], { wide: true });
}

async function viewCommands() {
  commandsCache = await act('commands', 'list').catch(() => []);
  const groups = {};
  for (const c of commandsCache) (groups[c.group || 'Altro'] ||= []).push(c);
  return {
    sub: 'Shell, applicazioni, scorciatoie e macro richiamabili dal telefono o dal Mobile Deck',
    narrow: true,
    actions: [btn('Nuovo comando', () => commandModal(), 'primary', 'add')],
    body: commandsCache.length ? Object.entries(groups).map(([g, list]) => h('div', { class: 'col', style: 'gap:10px' },
      sectionTitle(g, list.length === 1 ? '1 comando' : `${list.length} comandi`),
      h('div', { class: 'list' }, list.map((c) => h('div', { class: 'li' },
        h('div', { class: 'avatar', style: 'font-size:20px' }, c.icon || icon('terminal', 20)),
        h('div', { class: 'grow' }, h('div', { class: 'li-title ellipsis' }, c.name),
          h('div', { class: 'li-sub ellipsis' }, c.description || (c.actions.length === 1 ? '1 azione' : `${c.actions.length} azioni`))),
        h('div', { class: 'li-actions' },
          btn('Esegui', () => run(act('commands', 'run', { id: c.id }), 'Eseguito'), 'small tonal', 'play_arrow'),
          iconBtn('edit', 'Modifica', () => commandModal(c), 'sm'),
          iconBtn('delete', 'Elimina', () => confirmModal(`Eliminare «${c.name}»?`, 'Il comando sparisce anche dal telefono e dai pulsanti del Mobile Deck che lo usano.', 'Elimina',
            () => run(act('commands', 'delete', { id: c.id })).then(render)), 'sm danger')))))))
      : h('div', { class: 'card' }, emptyState('terminal', 'Nessun comando', 'Creane uno per controllare il computer dal telefono: un comando shell, un\'app da aprire, una scorciatoia o una macro.',
        btn('Nuovo comando', () => commandModal(), 'primary', 'add'))),
  };
}

// --------------------------------------------------------------------- deck --

const ICONS = {
  play_pause: '⏯', skip_previous: '⏮', skip_next: '⏭', volume: '🔊', volume_off: '🔇', dnd: '🌙',
  content_copy: '📋', content_paste: '📥', search: '🔍', tab: '⇥', looks_one: '1', looks_two: '2', looks_3: '3',
  sensors: '🔴', fiber_manual_record: '⏺', mic_off: '🎙', arrow_back: '◀', arrow_forward: '▶', play_arrow: '▶',
  dark_mode: '⬛', stop: '⏹', screenshot: '📸', headset_off: '🎧', record_voice_over: '🗣', work: '💼',
  videocam: '🎥', slideshow: '📽', sports_esports: '🎮',
};
const iconOf = (name) => ICONS[name] || name || '';
const newId = (n = 12) => Math.random().toString(16).slice(2, 2 + n);

function deckButtonModal(profile, page, pos, existing) {
  const b = clone(existing || { id: '', position: pos, label: '', icon: '', color: '#3b4252', category: '', actions: [], activeActions: [] });
  b.activeActions ||= [];
  const shellBox = h('label', { class: 'field', style: b.state?.type === 'shell' ? '' : 'display:none' }, 'Comando di stato',
    h('input', { type: 'text', value: b.state?.command || '', oninput: (e) => { if (b.state) b.state.command = e.target.value; } }));
  const stateSel = h('select', { onchange: (e) => {
    const v = e.target.value;
    b.state = v ? (v === 'shell' ? { type: 'shell', command: '', intervalSecs: 5 } : { type: v }) : undefined;
    shellBox.style.display = v === 'shell' ? '' : 'none';
  } },
  [['', 'Nessuno'], ['mediaPlaying', 'Riproduzione in corso'], ['doNotDisturb', 'Non disturbare attivo'], ['volume', 'Mostra volume'], ['shell', 'Comando shell (exit 0 = attivo)']]
    .map(([v, l]) => h('option', { value: v, selected: (b.state?.type || '') === v }, l)));
  const f = (key, label, type = 'text') => h('label', { class: 'field' }, label,
    h('input', { type, value: b[key] || '', oninput: (e) => { b[key] = e.target.value; } }));
  modal(existing ? 'Modifica pulsante' : 'Nuovo pulsante', h('div', { class: 'col' },
    h('div', { class: 'grid2' }, f('label', 'Etichetta'), f('icon', 'Icona (emoji o nome)'), f('category', 'Categoria'), f('color', 'Colore', 'color')),
    h('div', { class: 'set-title', style: 'margin-top:6px' }, 'Azioni / macro'),
    actionsEditor(b.actions),
    h('div', { class: 'set-title', style: 'margin-top:6px' }, 'Stato dinamico (aggiornato dal computer)'),
    h('div', { class: 'grid2' }, h('label', { class: 'field' }, 'Sorgente', stateSel), shellBox, f('activeColor', 'Colore quando attivo', 'color')),
    h('div', { class: 'set-title', style: 'margin-top:6px' }, 'Azioni alternative quando lo stato è attivo (toggle, facoltative)'),
    actionsEditor(b.activeActions)),
  [
    existing ? ['Elimina', () => {
      page.buttons = page.buttons.filter((x) => x !== existing);
      render();
    }, 'danger'] : null,
    ['Annulla'],
    ['Applica', () => {
      if (!b.label) { toast('Inserisci un\'etichetta', 'err'); return false; }
      if (!b.id) b.id = newId();
      if (!b.state) delete b.state;
      if (existing) Object.assign(existing, b); else page.buttons.push(b);
      render();
      return true;
    }, 'primary'],
  ].filter(Boolean), { wide: true });
}

async function viewDeck() {
  if (!S.deck) S.deck = await act('deck', 'get');
  const deck = S.deck;
  const prof = deck.profiles.find((p) => p.id === S.deckProfile) || deck.profiles.find((p) => p.id === deck.activeProfile) || deck.profiles[0];
  S.deckProfile = prof.id;
  if (S.deckPage >= prof.pages.length) S.deckPage = 0;
  const page = prof.pages[S.deckPage];
  const cats = [...new Set(prof.pages.flatMap((p) => p.buttons.map((b) => b.category)).filter(Boolean))];

  const save = () => run(act('deck', 'save', deck), 'Deck salvato e inviato ai telefoni').then(() => { S.deck = null; render(); });
  const grid = page ? h('div', { class: 'deck-grid', style: `grid-template-columns:repeat(${prof.columns},minmax(60px,104px))` },
    Array.from({ length: prof.columns * prof.rows }, (_, pos) => {
      const b = page.buttons.find((x) => x.position === pos);
      const dim = S.deckCategory && b && b.category !== S.deckCategory;
      return b
        ? h('div', { class: 'deck-cell filled', title: b.label, style: `background:${b.color};${dim ? 'opacity:.25' : ''}`, onclick: () => deckButtonModal(prof, page, pos, b) },
          h('div', { class: 'dc-ic' }, iconOf(b.icon)), h('div', { class: 'ellipsis', style: 'max-width:100%' }, b.label), b.state ? h('span', { class: 'dc-state' }) : null)
        : h('div', { class: 'deck-cell', title: 'Nuovo pulsante', onclick: () => deckButtonModal(prof, page, pos) }, icon('add', 24));
    })) : h('p', { class: 'muted' }, 'Nessuna pagina.');
  const num = (key, label) => h('label', { class: 'field' }, label,
    h('input', { type: 'number', min: 1, max: 12, value: prof[key], onchange: (e) => { prof[key] = Math.max(1, Math.min(12, +e.target.value || 1)); render(); } }));

  const left = h('div', { class: 'col', style: 'gap:14px;min-width:0' },
    h('div', { class: 'profiles' },
      deck.profiles.map((p) => h('button', { class: `profile-chip ${p.id === prof.id ? 'active' : ''}`, title: p.id === deck.activeProfile ? 'Profilo attivo sui telefoni' : '',
        onclick: () => { S.deckProfile = p.id; S.deckPage = 0; render(); } },
        p.id === deck.activeProfile ? icon('check', 16) : null, `${iconOf(p.icon)} ${p.name}`.trim())),
      h('button', { class: 'profile-chip', onclick: () => {
        const id = newId();
        deck.profiles.push({ id, name: 'Nuovo profilo', icon: '', columns: 4, rows: 3, pages: [{ id: 'p1', name: 'Pagina 1', buttons: [] }] });
        S.deckProfile = id;
        render();
      } }, icon('add', 16), 'Profilo')),
    h('div', { class: 'card flush' },
      h('div', { class: 'tabs', style: 'padding:0 10px' },
        prof.pages.map((p, i) => h('button', { class: i === S.deckPage ? 'active' : '', onclick: () => { S.deckPage = i; render(); } }, p.name)),
        h('button', { title: 'Nuova pagina', onclick: () => { prof.pages.push({ id: newId(8), name: `Pagina ${prof.pages.length + 1}`, buttons: [] }); S.deckPage = prof.pages.length - 1; render(); } }, icon('add', 18))),
      h('div', { style: 'padding:20px' },
        h('div', { class: 'deck-frame' }, grid),
        h('p', { class: 'small muted', style: 'margin-top:12px;text-align:center' }, 'Clicca un riquadro per creare o modificare un pulsante. Le modifiche arrivano ai telefoni quando premi Salva.'))));

  const right = h('div', { class: 'col', style: 'gap:16px;min-width:0' },
    h('div', { class: 'card' },
      cardHead('dashboard', 'Profilo', prof.id === deck.activeProfile ? 'attivo sui telefoni' : null),
      h('div', { class: 'col' },
        h('label', { class: 'field' }, 'Nome', h('input', { type: 'text', value: prof.name, oninput: (e) => { prof.name = e.target.value; } })),
        h('div', { class: 'grid2', style: 'grid-template-columns:repeat(3,minmax(0,1fr))' },
          h('label', { class: 'field' }, 'Icona', h('input', { type: 'text', value: prof.icon, oninput: (e) => { prof.icon = e.target.value; } })),
          num('columns', 'Colonne'), num('rows', 'Righe'))),
      h('div', { class: 'row', style: 'margin-top:16px' },
        prof.id !== deck.activeProfile ? btn('Imposta attivo', () => { deck.activeProfile = prof.id; save(); }, 'tonal small', 'check') : null,
        btn('Duplica', () => run(act('deck', 'duplicateProfile', { id: prof.id }), 'Profilo duplicato').then(() => { S.deck = null; render(); }), 'small', 'copy'),
        btn('Elimina', () => confirmModal(`Eliminare il profilo «${prof.name}»?`, 'Il profilo e le sue pagine spariscono anche dai telefoni.', 'Elimina',
          () => run(act('deck', 'deleteProfile', { id: prof.id })).then(() => { S.deck = null; S.deckProfile = null; render(); })), 'small danger', 'delete'))),
    page ? h('div', { class: 'card' },
      cardHead('text', 'Pagina'),
      h('div', { class: 'col' },
        h('label', { class: 'field' }, 'Nome della pagina', h('input', { type: 'text', value: page.name, oninput: (e) => { page.name = e.target.value; } })),
        cats.length ? h('label', { class: 'field' }, 'Evidenzia una categoria',
          h('select', { onchange: (e) => { S.deckCategory = e.target.value; render(); } },
            h('option', { value: '' }, 'Tutte'), cats.map((c) => h('option', { value: c, selected: c === S.deckCategory }, c)))) : null),
      prof.pages.length > 1 ? h('div', { class: 'row', style: 'margin-top:16px' },
        btn('Elimina pagina', () => { prof.pages.splice(S.deckPage, 1); S.deckPage = 0; render(); }, 'small danger', 'delete')) : null) : null,
    h('div', { class: 'card tonal' },
      h('div', { class: 'set-title' }, 'Profili predefiniti'),
      h('div', { class: 'set-desc', style: 'margin-bottom:12px' }, 'Sostituisce tutti i profili con quelli di partenza.'),
      btn('Ripristina', () => confirmModal('Ripristinare?', 'Tutti i profili verranno sostituiti con quelli predefiniti.', 'Ripristina',
        () => run(act('deck', 'reset')).then(() => { S.deck = null; render(); })), 'small', 'sync')));

  return {
    sub: 'Il telefono diventa un pannello di controllo tipo Stream Deck; le azioni restano su questo computer',
    actions: [btn('Salva', save, 'primary', 'save')],
    body: h('div', { class: 'deck-layout' }, left, right),
  };
}

// ------------------------------------------------------------------- plugin --

async function viewPlugins() {
  const plugins = await api('plugins');
  const q = norm(S.drafts['plugin-q']);
  const shown = plugins.filter((p) => !q || norm(`${p.name} ${p.description}`).includes(q));
  const on = plugins.filter((p) => p.enabled).length;
  return {
    sub: `${on} attivi su ${plugins.length}`,
    narrow: true,
    actions: [draft('plugin-q', 'input', { type: 'search', placeholder: 'Filtra i plugin…', style: 'width:220px', oninput: () => rerender() })],
    body: [
      h('p', { class: 'small muted' }, 'Ogni funzione è un plugin indipendente: disattivalo per tutti i dispositivi qui, o per un solo dispositivo nella sua pagina. Con i ponti attivi (Impostazioni) alcuni plugin funzionano anche con le app KDE Connect e Flux.'),
      shown.length ? h('div', { class: 'list' }, shown.map((p) => h('div', { class: 'li' },
        h('div', { class: `avatar ${p.enabled ? '' : 'neutral'}` }, icon('extension', 20)),
        h('div', { class: 'grow' }, h('div', { class: 'li-title' }, p.name), h('div', { class: 'li-sub' }, p.description, interopNote(p.interop))),
        sw(p.enabled, (v) => run(api('setPluginEnabled', { plugin: p.id, enabled: v })).then(() => rerender('plugins'))))))
        : h('div', { class: 'card' }, emptyState('search', 'Nessun plugin trovato', `Niente corrisponde a «${S.drafts['plugin-q']}».`)),
    ],
  };
}

// ------------------------------------------------------------ configurazione --

const SETUP_FEATURES = {
  microphone: ['mic', 'Microfono del telefono', 'Usa il telefono come microfono del computer'],
  webcam: ['videocam', 'Webcam del telefono', 'Usa la fotocamera del telefono come webcam'],
  phoneScreen: ['smartphone', 'Schermo del telefono', 'Guarda lo schermo del telefono sul computer'],
  desktop: ['screen', 'Desktop remoto', 'Mostra lo schermo del computer sul telefono'],
  systemAudio: ['headphones', 'Audio del computer', 'Ascolta sul telefono ciò che suona il computer'],
  phoneAudio: ['speaker', 'Audio del telefono', 'Ascolta sul computer ciò che suona il telefono'],
  media: ['music', 'Controllo dei lettori', 'Play, pausa e brani dal telefono'],
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
    iconBtn('copy', 'Copia i comandi', () => run(navigator.clipboard.writeText(text), 'Comandi copiati'), 'sm accent'));
}

// Fingerprint approval of sudo: the phone makes its key, the user compares
// the code and types the password once; Konnekta sets up the rest.
function passwordModal(title, intro, okLabel, onOk) {
  const pw = h('input', { type: 'password', placeholder: 'Password del computer', autocomplete: 'current-password' });
  const err = h('p', { class: 'small', style: 'color:var(--danger)' });
  let busy = false;
  // Returning false keeps the window open: a wrong password can be typed again.
  const go = async () => {
    if (busy) return false;
    busy = true;
    err.textContent = 'Attendi…';
    try {
      await onOk(pw.value);
      render();
      return true;
    } catch (e) {
      err.textContent = String(e);
      pw.value = '';
      pw.focus();
      return false;
    } finally {
      busy = false;
    }
  };
  const close = modal(title, h('div', { class: 'col' }, intro,
    h('label', { class: 'field', style: 'margin-top:6px' }, 'Password (la stessa che usi con sudo)', pw), err),
  [['Annulla'], [okLabel, go, 'primary']]);
  pw.addEventListener('keydown', async (e) => { if (e.key === 'Enter' && (await go())) close(); });
  pw.focus();
}

async function enrollPhone(d) {
  const close = modal('Conferma sul telefono', h('div', { class: 'col', style: 'align-items:center;text-align:center' },
    h('div', { class: 'empty-ic' }, icon('lock', 34)),
    h('p', {}, `Su ${d.name} appare la richiesta: confermala con l'impronta (sui telefoni senza impronta, con «Approva»).`)));
  let r;
  try {
    r = await invoke('approve_enroll', { deviceId: d.id });
  } catch (e) {
    close();
    toast(String(e), 'err');
    return;
  }
  close();
  passwordModal('Controlla il codice', h('div', { class: 'col' },
    h('p', {}, `Il telefono ${r.device} mostra un codice: deve essere uguale a questo.`),
    keyBlocks(r.code),
    h('p', { class: 'small muted' }, 'Se è uguale, scrivi la password del computer: serve una volta sola per attivare l\'approvazione.')),
  'È uguale, attiva', async (password) => {
    await invoke('approve_confirm', { password });
    toast('Fatto: ora sudo chiede prima al telefono');
  });
}

async function approveCard() {
  const st = await invoke('approve_status').catch(() => null);
  if (!st?.available) return null;
  const on = st.helper && st.sudo && st.device;
  const phones = S.devices.filter((d) => d.paired && d.connected && hasCap(d, 'konnekta.approve.request'));
  let pick = phones[0]?.id;
  return h('div', { class: 'card' },
    cardHead('lock', 'Approva sudo con l\'impronta', null, h('span', { class: `chip ${on ? 'ok' : 'warn'}` }, on ? 'attiva' : 'non attiva')),
    on
      ? [h('p', { class: 'small' }, `sudo chiede prima a ${st.device}: confermi con l'impronta, o con «Approva» se il telefono non ce l'ha. La password resta sempre disponibile.`),
        h('div', { class: 'card-foot' }, btn('Disattiva', () => passwordModal('Disattivare?', h('p', {}, 'sudo tornerà a chiedere solo la password. Il telefono potrebbe chiedere conferma un\'ultima volta.'),
          'Disattiva', async (password) => {
            await invoke('approve_disable', { password });
            toast('Approvazione disattivata');
          }), 'danger'))]
      : [h('p', { class: 'small muted' }, 'Quando usi sudo il telefono chiede l\'impronta al posto della password (che resta sempre disponibile). Sui telefoni vecchi o senza impronta arriva la notifica e approvi con un tocco su «Approva».'),
        phones.length
          ? h('div', { class: 'card-foot' },
            phones.length > 1 ? h('select', { onchange: (e) => { pick = e.target.value; } }, phones.map((d) => h('option', { value: d.id }, d.name))) : null,
            btn(phones.length > 1 ? 'Attiva' : `Attiva con ${phones[0].name}`, () => enrollPhone(phones.find((d) => d.id === pick)), 'primary'))
          : h('p', { class: 'small', style: 'margin-top:8px' }, 'Connetti un telefono con l\'app Konnekta per attivarla.')]);
}

async function viewSetup() {
  const [r, approve] = await Promise.all([invoke('setup_check'), approveCard()]);
  const distro = { arch: 'Arch Linux', debian: 'Debian / Ubuntu', fedora: 'Fedora', opensuse: 'openSUSE' }[r.distro];
  const feats = r.features || [];
  const missing = feats.filter((f) => !f.ok);
  const ready = feats.length - missing.length;
  return {
    sub: distro ? `Comandi scritti per ${distro}` : 'Programmi del sistema usati dalle funzioni',
    narrow: true,
    actions: [btn('Ricontrolla', () => render(), 'text', 'sync')],
    body: [
      h('div', { class: 'card setup-hero' },
        h('div', { class: 'ring', style: `--p:${feats.length ? Math.round(ready / feats.length * 100) : 100}` }, h('span', {}, `${ready}/${feats.length}`)),
        h('div', { class: 'grow' },
          h('h2', { style: 'font-size:18px' }, missing.length ? (missing.length === 1 ? '1 funzione da completare' : `${missing.length} funzioni da completare`) : 'Tutto pronto'),
          h('p', { class: 'small muted', style: 'margin-top:4px' }, 'Alcune funzioni usano programmi del sistema. Konnekta non li installa da solo: qui trovi cosa manca e i comandi da incollare nel terminale.'))),
      r.allCommands?.length ? h('div', { class: 'card' },
        cardHead('terminal', 'Tutti i comandi', 'Eseguili come utente normale: sudo chiede la password. Poi premi Ricontrolla, senza riavviare Konnekta.'),
        commandBlock(r.allCommands)) : null,
      approve,
      h('div', { class: 'list' }, feats.map((f) => {
        const [ic, title, sub] = SETUP_FEATURES[f.id] || ['extension', f.id, ''];
        const bad = f.requirements.filter((q) => q.status !== 'ok');
        return h('div', { class: 'li', style: 'flex-direction:column;align-items:stretch;gap:0' },
          h('div', { class: 'row nowrap', style: 'gap:14px' },
            h('div', { class: `avatar ${f.ok ? '' : 'neutral'}` }, icon(ic, 20)),
            h('div', { class: 'grow' }, h('div', { class: 'li-title' }, title), h('div', { class: 'li-sub' }, sub)),
            h('span', { class: `chip ${f.ok ? 'ok' : 'warn'}` }, f.ok ? icon('check', 14) : null, f.ok ? 'pronto' : 'da completare')),
          bad.map((q) => h('div', { class: 'setup-req' },
            h('div', { class: 'small row', style: 'gap:8px' }, h('code', {}, q.id), h('span', { class: 'grow' }, q.purpose),
              h('span', { class: `chip ${SETUP_STATUS[q.status][0]}` }, SETUP_STATUS[q.status][1])),
            q.commands.length ? commandBlock(q.commands) : null)));
      })),
    ],
  };
}

// --------------------------------------------------------------- impostazioni --

const SHORTCUTS = [
  ['Cerca pagine e azioni', 'Ctrl K'], ['Vai alla pagina 1…9', 'Ctrl 1…9'], ['Invia file al dispositivo', 'Ctrl O'],
  ['Comprimi la barra laterale', 'Ctrl B'], ['Impostazioni', 'Ctrl ,'], ['Chiudi finestre e menu', 'Esc'],
];

async function viewSettings() {
  const cfg = await api('getConfig');
  const shell = await invoke('desktop_prefs_get');
  const get = (p) => act(p, 'getConfig').catch(() => null);
  const [share, clip, shot, filter, dnd, pres, tel, ls] = await Promise.all([
    get('share'), get('clipboard'), get('screenshot'), act('notifications', 'getFilter').catch(() => null), get('dnd'), get('presenter'), get('calls'), get('localsend')]);
  const general = { deviceName: cfg.device_name, downloadDir: cfg.download_dir, announce: cfg.announce, mdns: cfg.mdns, usb: cfg.usb !== false, bluetooth: cfg.bluetooth !== false, virtualCamera: cfg.virtual_camera !== false,
    kdeConnect: !!cfg.kde_connect?.enabled, flux: !!cfg.flux?.enabled, syncPermissions: cfg.sync_permissions !== false };

  const txt = (o, k, label, ph) => h('label', { class: 'field' }, label, h('input', { type: 'text', value: o[k] ?? '', placeholder: ph, oninput: (e) => { o[k] = e.target.value; } }));
  const tog = (o, k, title, desc) => setRow(title, desc, sw(!!o[k], (v) => { o[k] = v; }));
  const folder = (o, k, label) => {
    const input = h('input', { type: 'text', value: o[k] ?? '', oninput: (e) => { o[k] = e.target.value || null; } });
    return h('label', { class: 'field' }, label, h('div', { class: 'input-row' }, input,
      btn('Sfoglia…', async () => { const p = await invoke('pick_folder'); if (p) { o[k] = p; input.value = p; } }, 'small', 'folder')));
  };
  const fields = (...kids) => h('div', { class: 'set-fields' }, kids);
  const group = (t) => h('div', { class: 'set-group' }, t);
  const sections = [];
  const card = (id, ic, title, body, onSave) => {
    sections.push([id, title]);
    return h('div', { class: 'card', id },
      cardHead(ic, title), body,
      onSave ? h('div', { class: 'card-foot' }, btn('Salva', () => run(onSave(), 'Impostazioni salvate'), 'primary')) : null);
  };
  const list = (arr) => (arr || []).join(', ');
  const fl = filter && { ...filter, blocked: list(filter.blockedApps), allowed: list(filter.allowedApps), muted: list(filter.mutedKeywords) };
  const split = (s) => s.split(',').map((x) => x.trim()).filter(Boolean);
  const theme = PREF.get('theme', 'system');

  const cards = [
    card('set-look', 'palette', 'Aspetto', h('div', {},
      setRow('Tema', 'Chiaro, scuro o come il sistema; i colori sono quelli dell\'app del telefono.',
        h('div', { class: 'seg' }, [['system', 'Sistema', 'contrast'], ['light', 'Chiaro', 'light_mode'], ['dark', 'Scuro', 'dark_mode']].map(([v, l, ic]) =>
          h('button', { class: theme === v ? 'active' : '', onclick: () => setTheme(v) }, icon(ic, 18), l)))),
      setRow('Barra laterale compatta', 'Solo le icone; nelle finestre strette lo diventa da sola (Ctrl+B).',
        sw(PREF.get('rail', 'full') === 'compact', () => { if (innerWidth >= NARROW) toggleRail(); else { PREF.set('rail', PREF.get('rail', 'full') === 'compact' ? 'full' : 'compact'); } })))),
    card('set-general', 'settings', 'Generale', h('div', {},
      fields(txt(general, 'deviceName', 'Nome di questo computer'), folder(general, 'downloadDir', 'Cartella predefinita per i file ricevuti')),
      tog(general, 'announce', 'Annuncia questo computer sulla rete locale', 'Disattiva sulle reti pubbliche: i dispositivi associati vengono comunque contattati.'),
      tog(general, 'mdns', 'Trova i dispositivi con mDNS', 'Servizio _konnekta._tcp sulla rete locale.'),
      tog(general, 'usb', 'Telefoni Android collegati con il cavo USB', 'Serve adb e il debug USB attivo sul telefono.'),
      tog(general, 'bluetooth', 'Telefoni associati via Bluetooth', 'Solo Linux; associa prima il telefono nelle impostazioni Bluetooth del computer.'),
      tog(general, 'virtualCamera', 'Webcam «Konnekta Camera» sempre disponibile', 'Mentre Konnekta è aperto (Linux, v4l2loopback). Richiede il riavvio.'),
      tog(general, 'syncPermissions', 'Sincronizza i permessi con i dispositivi', 'Ciò che concedi o revochi sul telefono vale anche qui, e viceversa.'),
      group('Ponti verso altre app (facoltativi)'),
      tog(general, 'kdeConnect', 'Accetta le app KDE Connect e GSConnect', 'Porte 1716; solo le funzioni comuni. Richiede il riavvio.'),
      tog(general, 'flux', 'Accetta le app Flux', 'Porte 12100; solo le funzioni comuni. Richiede il riavvio.')),
    () => api('setConfig', general)),
    card('set-shell', 'desktop', 'Programma sul computer', h('div', {},
      tog(shell, 'tray', 'Icona nella tray', 'Se disattivata, chiudere la finestra chiude Konnekta.'),
      tog(shell, 'autostart', 'Avvia Konnekta con il computer', 'Solo nella tray, senza aprire la finestra.'),
      tog(shell, 'appList', 'Mostra Konnekta nella lista delle applicazioni'),
      tog(shell, 'sendTo', '«Invia tramite Konnekta» nel menu dei file', 'Dolphin, Nautilus, Nemo, Caja, «Apri con»; su Windows «Invia a» e il menu del tasto destro.')),
    () => invoke('desktop_prefs_set', { prefs: shell })),
    share && card('set-share', 'folder', 'File', h('div', {},
      fields(folder(share, 'destination', 'Cartella di destinazione (vuoto = predefinita)')),
      tog(share, 'textToClipboard', 'Copia negli appunti il testo ricevuto')),
    () => act('share', 'setConfig', share)),
    ls && card('set-ls', 'public', 'LocalSend', h('div', {},
      h('p', { class: 'small muted', style: 'margin-bottom:8px' }, 'Scambia file e testi con le app LocalSend sulla stessa rete, dalla pagina Invia. I dispositivi LocalSend non vengono associati: ogni invio verso questo computer va accettato (salvo la ricezione rapida) e non hanno accesso a nient\'altro di Konnekta.'),
      tog(ls, 'enabled', 'Compatibilità con LocalSend', 'Porta TCP/UDP 53317.'),
      tog(ls, 'autoAccept', 'Ricezione rapida', 'Accetta i file senza chiedere.'),
      tog(ls, 'https', 'Cifra con HTTPS', 'Come LocalSend; disattiva solo per dispositivi vecchi.'),
      fields(
        h('div', { class: 'grid2' },
          txt(ls, 'alias', 'Nome mostrato su LocalSend', S.me?.deviceName || 'nome del computer'),
          txt(ls, 'pin', 'PIN richiesto a chi invia', 'nessuno'),
          h('label', { class: 'field' }, 'Porta', h('input', { type: 'number', value: ls.port, min: 1024, max: 65535, oninput: (e) => { ls.port = Number(e.target.value) || 53317; } }))),
        folder(ls, 'destination', 'Cartella per i file ricevuti (vuoto = predefinita)'))),
    () => act('localsend', 'setConfig', { ...ls, alias: ls.alias || null, pin: ls.pin || null, destination: ls.destination || null })),
    clip && card('set-clip', 'paste', 'Appunti', h('div', {},
      tog(clip, 'autoSend', 'Invia automaticamente ai dispositivi ciò che copio'),
      tog(clip, 'autoReceive', 'Ricevi automaticamente gli appunti dei dispositivi')),
    () => act('clipboard', 'setConfig', clip)),
    shot && card('set-shot', 'image', 'Screenshot del telefono', h('div', {},
      tog(shot, 'autoTransfer', 'Trasferisci automaticamente gli screenshot dal telefono'),
      tog(shot, 'save', 'Salva gli screenshot ricevuti'),
      tog(shot, 'copyToClipboard', 'Copia lo screenshot negli appunti'),
      fields(folder(shot, 'folder', `Cartella (vuoto = ${shot.effectiveFolder})`))),
    () => { const { effectiveFolder, ...c } = shot; return act('screenshot', 'setConfig', c); }),
    fl && card('set-notif', 'bell', 'Notifiche', h('div', {},
      tog(fl, 'showPopups', 'Mostra le notifiche del telefono sul computer'),
      tog(fl, 'privacyMode', 'Modalità privacy', 'Nascondi titolo e contenuto.'),
      tog(fl, 'hideSilent', 'Ignora le notifiche silenziose'),
      fields(
        txt(fl, 'blocked', 'App bloccate (separate da virgola)'),
        txt(fl, 'allowed', 'Solo queste app (vuoto = tutte)'),
        txt(fl, 'muted', 'Parole da filtrare'))),
    () => act('notifications', 'setFilter', { showPopups: fl.showPopups, privacyMode: fl.privacyMode, hideSilent: fl.hideSilent,
      blockedApps: split(fl.blocked), allowedApps: split(fl.allowed), mutedKeywords: split(fl.muted) })),
    dnd && card('set-dnd', 'dark_mode', 'Non disturbare', h('div', {},
      tog(dnd, 'sync', 'Sincronizza Non disturbare tra telefono e computer'),
      setRow('In caso di modifiche simultanee vince', null,
        h('select', { onchange: (e) => { dnd.priority = e.target.value; } },
          [['latest', 'La modifica più recente'], ['phone', 'Il telefono'], ['computer', 'Il computer']]
            .map(([v, l]) => h('option', { value: v, selected: dnd.priority === v }, l))))),
    () => act('dnd', 'setConfig', dnd)),
    pres && card('set-pres', 'screen', 'Presentazioni', setRow('Programma di presentazione', 'I tasti che il telecomando del telefono invia.',
      h('select', { onchange: (e) => { pres.program = e.target.value; } },
        Object.keys(pres.keymaps).map((k) => h('option', { value: k, selected: pres.program === k }, k)))),
    () => act('presenter', 'setConfig', pres)),
    tel && card('set-calls', 'call', 'Chiamate', h('div', {},
      tog(tel, 'pauseMedia', 'Metti in pausa la musica durante le chiamate'),
      tog(tel, 'showNumber', 'Mostra il numero del chiamante')),
    () => act('calls', 'setConfig', tel)),
    card('set-keys', 'keyboard', 'Scorciatoie da tastiera', h('div', { class: 'shortcuts' },
      SHORTCUTS.map(([l, k]) => h('div', {}, h('span', {}, l), h('kbd', {}, k))))),
    updatesCard(sections),
  ].filter(Boolean);

  const nav = h('div', { class: 'settings-nav' }, sections.map(([id, title]) =>
    h('button', { 'data-sec': id, onclick: () => $(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' }) }, title)));
  // The section in view is highlighted in the list on the left.
  const onScroll = () => {
    const top = $main.getBoundingClientRect().top + 40;
    let active = sections[0]?.[0];
    for (const [id] of sections) { const el = $(id); if (el && el.getBoundingClientRect().top <= top) active = id; }
    if ($main.scrollTop + $main.clientHeight >= $main.scrollHeight - 4) active = sections.at(-1)?.[0];
    for (const b of nav.children) b.classList.toggle('active', b.dataset.sec === active);
  };
  return {
    sub: S.me ? `${S.me.deviceName} · Konnekta ${S.me.version}` : null,
    onScroll,
    body: h('div', { class: 'settings' }, nav, h('div', { class: 'settings-body' }, cards)),
  };
}

// --------------------------------------------------------------- aggiornamenti --

const UPDATE_SNOOZE_MS = 24 * 3600 * 1000;
const snoozed = (v) => { try { return Number(localStorage.getItem(`update-snooze-${v}`)) > Date.now(); } catch { return false; } };
const snooze = (v) => { try { localStorage.setItem(`update-snooze-${v}`, String(Date.now() + UPDATE_SNOOZE_MS)); } catch { /* ignore */ } };

function showUpdatePopup(u) {
  const status = h('p', { class: 'muted small' });
  modal('Nuova versione disponibile', h('div', { class: 'col' },
    h('p', { style: 'color:var(--text)' }, `Konnekta ${u.latest} è disponibile (hai la ${u.current}).`),
    u.notes ? h('pre', { class: 'mono', style: 'white-space:pre-wrap;max-height:220px;overflow:auto;margin:0;padding:12px;border-radius:12px;background:var(--sc)' }, u.notes) : null,
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

async function checkUpdatesNow(out) {
  const say = (t) => { if (out) out.textContent = t; };
  say('Controllo in corso…');
  try {
    const u = await invoke('check_update');
    if (u.available) { say(`Disponibile la versione ${u.latest}.`); showUpdatePopup(u); }
    else { say(`Konnekta è aggiornato (versione ${u.current}).`); if (!out) toast(`Konnekta è aggiornato (versione ${u.current})`); }
  } catch (e) { say(`Controllo non riuscito: ${e}`); if (!out) toast(`Controllo non riuscito: ${e}`, 'err'); }
}

function updatesCard(sections) {
  sections.push(['set-updates', 'Aggiornamenti']);
  const out = h('div', { class: 'set-desc' }, `Versione installata: ${S.me?.version ?? ''}`);
  return h('div', { class: 'card', id: 'set-updates' },
    cardHead('sync', 'Aggiornamenti'),
    setRow('Konnekta', out, btn('Cerca aggiornamenti', () => checkUpdatesNow(out), 'tonal')),
    setRow('Codice sorgente', null, h('a', { href: 'https://github.com/HubDevCode/Konnekta', target: '_blank' }, 'github.com/HubDevCode/Konnekta')));
}

async function checkUpdateOnStart() {
  try {
    const u = await invoke('check_update');
    if (u.available && !snoozed(u.latest)) showUpdatePopup(u);
  } catch { /* offline or no release: stay quiet */ }
}

// ---------------------------------------------------------------- sicurezza --

const PROTECTIONS = [
  ['lock', 'Cifrato ovunque', 'Tutto il traffico (pacchetti, file, appunti, notifiche, audio, video e desktop remoto) è cifrato con TLS 1.3/1.2 e cifrari AEAD con forward secrecy.'],
  ['shield', 'Autenticazione reciproca', 'Ogni dispositivo presenta il proprio certificato; dopo l\'associazione il certificato è fissato e qualsiasi altro viene rifiutato (protezione man-in-the-middle).'],
  ['link', 'Associazione verificata', 'L\'associazione richiede la conferma su entrambi i dispositivi e il confronto di un codice di verifica derivato dalle chiavi di entrambi.'],
  ['wifi', 'Nessuna fiducia nella rete', 'Un dispositivo non associato può solo chiedere l\'associazione.'],
  ['tune', 'Permessi granulari', 'Permessi per dispositivo; le funzioni sensibili sono disattivate per impostazione predefinita.'],
  ['terminal', 'Comandi sicuri', 'I comandi remoti sono richiamabili solo per identificativo: il telefono non può inviare comandi arbitrari.'],
];

async function viewSecurity() {
  const me = await api('info');
  return {
    sub: `Protocollo Konnekta v${me.protocolVersion} · versione ${me.version}`,
    body: [
      h('div', { class: 'card' },
        cardHead('desktop', 'Questo computer', me.deviceName),
        setRow('ID dispositivo', h('span', { class: 'mono' }, me.deviceId), iconBtn('copy', 'Copia l\'ID', () => copyText(me.deviceId), 'sm')),
        h('div', { class: 'set-row', style: 'flex-direction:column;align-items:stretch;gap:10px' },
          h('div', { class: 'row' }, h('div', { class: 'set-title grow' }, 'Impronta del certificato (SHA-256)'),
            btn('Copia', () => copyText(me.fingerprint), 'text small', 'copy')),
          fingerprintGrid(me.fingerprint),
          h('div', { class: 'small muted' }, 'Confrontala con quella mostrata dal telefono per essere certo di parlare con questo computer.'))),
      sectionTitle('Come sono protetti i tuoi dati'),
      h('div', { class: 'protections' }, PROTECTIONS.map(([ic, title, text]) =>
        h('div', { class: 'protection' }, h('div', { class: 'avatar' }, icon(ic, 20)), h('div', { class: 'set-title' }, title), h('div', { class: 'muted' }, text)))),
      sectionTitle('Avvisi di sicurezza', 'in questa sessione'),
      S.warnings.length ? h('div', { class: 'list' }, S.warnings.map((w) => h('div', { class: 'li' },
        h('div', { class: 'avatar', style: 'background:color-mix(in srgb,var(--warn) 18%,transparent);color:var(--warn)' }, icon('warning', 20)),
        h('div', { class: 'grow' }, h('div', { class: 'li-title' }, w.message),
          h('div', { class: 'li-sub' }, `${w.at.toLocaleString()}${w.deviceId ? ` · ${devName(w.deviceId)}` : ''}`)))))
        : h('div', { class: 'card tonal row nowrap' }, h('div', { class: 'avatar' }, icon('check_circle', 20)),
          h('div', { class: 'grow' }, h('div', { class: 'set-title' }, 'Nessun avviso'), h('div', { class: 'set-desc' }, 'Qui compaiono certificati cambiati, richieste bloccate e altri eventi sospetti.'))),
    ],
  };
}

// ---------------------------------------------------------- palette comandi --

let palClose = null;

function paletteItems() {
  const d = cur();
  const items = [];
  if (d?.connected) {
    items.push(
      { group: 'Azioni', ic: 'folder', label: `Invia file a ${d.name}`, kbd: 'Ctrl O', run: () => sendFilesTo(d) },
      { group: 'Azioni', ic: 'paste', label: `Invia gli appunti a ${d.name}`, run: () => run(act('clipboard', 'send', null, d.id), 'Appunti inviati') },
      { group: 'Azioni', ic: 'bell', label: `Ping a ${d.name}`, run: () => run(act('ping', 'send', { message: 'Ping da Konnekta' }, d.id), `Ping inviato a ${d.name}`) });
    if (hasCap(d, 'konnekta.finddevice')) items.push({ group: 'Azioni', ic: 'ring', label: `Fai squillare ${d.name}`, run: () => run(act('finddevice', 'ring', null, d.id)) });
    items.push({ group: 'Azioni', ic: 'play_arrow', label: 'Play / pausa sul telefono', keywords: 'musica media', run: () => run(act('media', 'remoteControl', { action: 'PlayPause', player: S.phoneMedia[d.id]?.player }, d.id)) });
  }
  items.push({ group: 'Azioni', ic: 'upload', label: 'Scegli file da inviare…', keywords: 'selezione invia', run: () => { S.sendMode = 'send'; go('send'); addPick('file'); } });
  PAGES.forEach((v, i) => items.push({ group: 'Vai a', ic: v.icon, label: v.label, kbd: i < 9 ? `Ctrl ${i + 1}` : null, keywords: 'pagina', run: () => go(v.id) }));
  for (const x of S.devices.filter((y) => y.paired && y.id !== d?.id)) {
    items.push({ group: 'Dispositivi', ic: TYPE_ICON[x.deviceType] || 'smartphone', label: `Controlla ${x.name}`, hint: x.connected ? 'Online' : 'Offline', run: () => setCurrent(x.id) });
  }
  items.push(
    { group: 'Dispositivi', ic: 'sync', label: 'Cerca dispositivi', keywords: 'aggiorna rete', run: () => run(api('refresh'), 'Ricerca avviata').then(refreshDevices) },
    { group: 'App', ic: 'light_mode', label: 'Tema chiaro', keywords: 'aspetto', run: () => setTheme('light') },
    { group: 'App', ic: 'dark_mode', label: 'Tema scuro', keywords: 'aspetto', run: () => setTheme('dark') },
    { group: 'App', ic: 'contrast', label: 'Tema come il sistema', keywords: 'aspetto', run: () => setTheme('system') },
    { group: 'App', ic: 'menu_open', label: 'Comprimi o espandi la barra laterale', kbd: 'Ctrl B', run: toggleRail },
    { group: 'App', ic: 'sync', label: 'Cerca aggiornamenti di Konnekta', run: () => checkUpdatesNow() });
  return items;
}

function openPalette() {
  if (palClose) { palClose(); return; }
  closeMenus();
  const input = h('input', { type: 'text', placeholder: 'Cerca pagine, dispositivi, comandi e azioni…', 'aria-label': 'Cerca' });
  const listEl = h('div', { class: 'palette-list', role: 'listbox' });
  const items = paletteItems();
  let active = 0;
  let shown = [];
  const exec = (it) => { palClose(); it.run(); };
  const draw = () => {
    const words = norm(input.value).split(/\s+/).filter(Boolean);
    shown = items.filter((it) => words.every((w) => norm(`${it.label} ${it.group} ${it.keywords || ''} ${it.hint || ''}`).includes(w)));
    active = Math.max(0, Math.min(active, shown.length - 1));
    const kids = [];
    let group = null;
    shown.forEach((it, i) => {
      if (it.group !== group) { group = it.group; kids.push(h('div', { class: 'palette-group' }, group)); }
      kids.push(h('div', { class: `palette-item ${i === active ? 'active' : ''}`, role: 'option',
        onmousemove: () => { if (active !== i) { active = i; draw(); } }, onclick: () => exec(it) },
      h('div', { class: 'p-ic' }, icon(it.ic, 18)),
      h('div', { class: 'grow' }, h('div', { class: 'ellipsis' }, it.label), it.hint ? h('div', { class: 'p-hint ellipsis' }, it.hint) : null),
      it.kbd ? h('kbd', {}, it.kbd) : null));
    });
    if (!shown.length) kids.push(h('div', { class: 'empty', style: 'padding:28px' }, 'Nessun risultato'));
    listEl.replaceChildren(...kids);
    listEl.querySelector('.palette-item.active')?.scrollIntoView({ block: 'nearest' });
  };
  input.addEventListener('input', () => { active = 0; draw(); });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { e.preventDefault(); active = Math.min(active + 1, shown.length - 1); draw(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); active = Math.max(active - 1, 0); draw(); }
    else if (e.key === 'Enter') { e.preventDefault(); if (shown[active]) exec(shown[active]); }
    else if (e.key === 'Escape') { e.preventDefault(); palClose(); }
  });
  const bg = h('div', { class: 'palette-bg', onmousedown: (e) => { if (e.target === bg) palClose(); } },
    h('div', { class: 'palette', role: 'dialog', 'aria-label': 'Cerca' },
      h('div', { class: 'palette-input' }, icon('search', 22), input, h('kbd', {}, 'Esc')),
      listEl,
      h('div', { class: 'palette-foot' },
        h('span', {}, h('kbd', {}, '↑'), h('kbd', {}, '↓'), 'scegli'), h('span', {}, h('kbd', {}, 'Invio'), 'esegui'), h('span', {}, h('kbd', {}, 'Esc'), 'chiudi'))));
  $('palette-root').replaceChildren(bg);
  palClose = () => { $('palette-root').replaceChildren(); palClose = null; };
  draw();
  input.focus();
  // The computer's own commands, once they arrive.
  act('commands', 'list').then((cmds) => {
    if (!bg.isConnected || !cmds?.length) return;
    items.push(...cmds.map((c) => ({ group: 'Comandi personalizzati', ic: 'terminal', label: c.name, hint: c.description, keywords: c.group,
      run: () => run(act('commands', 'run', { id: c.id }), 'Eseguito') })));
    draw();
  }).catch(() => {});
}

// ------------------------------------------------------------------ tastiera --

addEventListener('keydown', (e) => {
  const mod = e.ctrlKey || e.metaKey;
  const key = e.key.toLowerCase();
  if (mod && key === 'k') { e.preventDefault(); openPalette(); return; }
  if (e.key === 'Escape') {
    if (palClose) { palClose(); return; }
    if ($('menu-root').childElementCount) { closeMenus(); return; }
    if ($('modal-root').childElementCount) { closeModals(); return; }
    return;
  }
  if (!mod || e.altKey || e.shiftKey) return;
  if (palClose || $('modal-root').childElementCount) return;
  if (key === 'b') { e.preventDefault(); toggleRail(); }
  else if (key === ',') { e.preventDefault(); go('settings'); }
  else if (key === 'o') {
    const d = cur();
    if (d?.connected) { e.preventDefault(); sendFilesTo(d); }
  } else if (/^[1-9]$/.test(e.key) && PAGES[+e.key - 1]) { e.preventDefault(); go(PAGES[+e.key - 1].id); }
});

addEventListener('resize', applyRail);

// ---------------------------------------------------------------------- avvio --

(async () => {
  applyTheme();
  buildNav();
  applyRail();
  $('rail-toggle').addEventListener('click', toggleRail);
  $('rail-foot').addEventListener('click', () => go('security'));
  document.querySelector('.brand-mark').append(logo());
  const saved = PREF.get('view', 'control');
  if (VIEWS[saved]) S.view = saved;
  drawTopbar();
  try {
    S.me = await api('info');
    drawRailFoot();
    await T.event.listen('konnekta', (e) => onEvent(e.payload));
    await T.event.listen('konnekta-send', () => takeSend());
    setupDrop();
    await refreshDevices();
    await render();
    await takeSend();
    checkUpdateOnStart();
  } catch (e) {
    lastOut = { title: 'Errore di avvio' };
    drawTopbar();
    $main.replaceChildren(h('div', { class: 'page' }, h('div', { class: 'card' }, notice('err', String(e)))));
  }
})();

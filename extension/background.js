// Notch Downloads: forwards chrome.downloads progress to Notch.exe through
// native messaging (host "com.notch.host"). An open native port keeps this
// service worker alive while Notch runs.

const HOST = "com.notch.host";
let port = null;
let timer = null;

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
    port.onDisconnect.addListener(() => {
      port = null;
      void chrome.runtime.lastError;
      setTimeout(connect, 5000);
    });
    port.postMessage({ type: "hello", version: chrome.runtime.getManifest().version });
  } catch (e) {
    port = null;
  }
}

function send(msg) {
  if (!port) connect();
  try {
    if (port) port.postMessage(msg);
  } catch (e) {
    port = null;
  }
}

function item(d) {
  return {
    id: d.id,
    file: d.filename || "",
    url: d.finalUrl || d.url || "",
    state: d.state,
    received: d.bytesReceived || 0,
    total: d.totalBytes > 0 ? d.totalBytes : 0,
    paused: !!d.paused,
    error: d.error || "",
  };
}

async function poll() {
  const list = await chrome.downloads.search({ state: "in_progress" });
  send({ type: "progress", items: list.map(item) });
  if (list.length === 0 && timer) {
    clearInterval(timer);
    timer = null;
  }
}

function track() {
  if (!timer) timer = setInterval(poll, 700);
  poll();
}

chrome.downloads.onCreated.addListener((d) => {
  send({ type: "created", item: item(d) });
  track();
});

chrome.downloads.onChanged.addListener(async (delta) => {
  const [d] = await chrome.downloads.search({ id: delta.id });
  if (!d) return;
  if (delta.state && delta.state.current === "complete") {
    send({ type: "complete", item: item(d) });
  } else if (delta.state && delta.state.current === "interrupted") {
    send({ type: "interrupted", item: item(d) });
  } else if (delta.filename || delta.paused || delta.totalBytes) {
    send({ type: "changed", item: item(d) });
  }
  track();
});

chrome.downloads.onErased.addListener((id) => send({ type: "erased", id }));

chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
chrome.alarms.create("notch-keepalive", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener(() => connect());
connect();
track();

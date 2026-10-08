import { installationIdentity, INSTALLATION_KEY } from './installation.js';
import { Broker } from './broker.js';
const installation = installationIdentity(chrome.storage.local);
const key = (tabId) => `recallcard_tab_${tabId}`;
const broker = new Broker({
  id: chrome.runtime.id,
  installationId: () => installation.get(),
  popupUrl: chrome.runtime.getURL('popup.html'),
  getTab: (tabId) => chrome.tabs.get(tabId),
  load: async (tabId) => (await chrome.storage.session.get(key(tabId)))[key(tabId)],
  save: (tabId, state) => chrome.storage.session.set({ [key(tabId)]: state }),
  now: () => Date.now(),
  content: (tabId, message, documentId) => chrome.tabs.sendMessage(tabId, message, documentId ? { documentId, frameId: 0 } : { frameId: 0 }),
  native: (host, request) => new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => { settled = true; reject(new Error('本机操作超过 15 秒，结果未确认；不会自动重试，请检查本机桥')); }, 15000);
    chrome.runtime.sendNativeMessage(host, request, (response) => {
      const error = chrome.runtime.lastError;
      if (settled) return;
      settled = true; clearTimeout(timer);
      if (error) reject(new Error('无法连接 RecallCard 本机桥；请按浏览器安装指南检查注册和授权'));
      else resolve(response);
    });
  }),
});
chrome.runtime.onMessage.addListener((message, sender, respond) => {
  broker.handle(message, sender).then((result) => respond({ ok: true, result }), (error) => respond({ ok: false, error: error.message }));
  return true;
});
chrome.tabs.onRemoved.addListener((tabId) => { void chrome.storage.session.remove(key(tabId)); });
chrome.tabs.onUpdated.addListener((tabId, change) => {
  // A reload or full navigation invalidates all outstanding nonces. SPA route
  // changes additionally rotate the content script's per-route token.
  if (change.status === 'loading') void chrome.storage.session.remove(key(tabId));
});

chrome.storage.onChanged.addListener((changes, area) => {
  if (area === 'local' && Object.hasOwn(changes, INSTALLATION_KEY)) {
    installation.reset();
    void chrome.storage.session.clear(); // Invalidate pending requests from the previous installation.
  }
});

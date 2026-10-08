// Local installation binding, not an account credential or OS-user boundary.
export const INSTALLATION_KEY = 'recallcard_installation_v1';
export const validInstallationId = id => typeof id === 'string' && /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/u.test(id);
export function installationIdentity(storage) {
  let pending;
  return {
    reset() { pending = null; },
    get() {
      pending ||= (async () => {
        const saved = (await storage.get(INSTALLATION_KEY))[INSTALLATION_KEY];
        if (saved !== undefined && !validInstallationId(saved)) throw new Error('本机安装编号损坏，请在扩展设置中重新建立安装绑定');
        if (saved) return saved;
        const id = crypto.randomUUID();
        await storage.set({ [INSTALLATION_KEY]: id });
        return id;
      })();
      return pending;
    },
  };
}

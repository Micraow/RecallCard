import { validateJobs, type JobStatus } from './contracts';
import type { RuntimeStatus } from './runtime';
import type { Workspace, RestoredWorkspace, MemoryPage, Memory, EventRecord, ConversationPage, MessagesPage, SearchPage, MemoryReview, ClientConfig, BrowserRegistration } from './types';

export interface Transport { invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> }
declare global {
  interface Window { __TAURI__?: { core: Transport } }
  const __APP_VERSION__: string;
}
export function nativeTransport(): Transport | null { return window.__TAURI__?.core || null; }
export class RecallService {
  constructor(readonly transport: Transport, readonly demo = false) {}
  restore() { return this.transport.invoke<RestoredWorkspace | null>('restore_workspace'); }
  createWorkspace() { return this.transport.invoke<Workspace>('open_default_workspace'); }
  chooseWorkspace() { return this.transport.invoke<Workspace | null>('choose_vault', { create: false }); }
  scoped(workspace: Workspace, scope: string) { return new ScopedService(this.transport, workspace, scope); }
}
export class ScopedService {
  constructor(readonly transport: Transport, readonly workspace: Workspace, readonly scope: string) {}
  private call<T>(command: string, args: Record<string, unknown> = {}) {
    return this.transport.invoke<T>(command, { sessionId: this.workspace.session_id, scope: this.scope, ...args });
  }
  remember() { return this.call<void>('remember_workspace'); }
  memories(offset = 0, includeHidden = false) { return this.call<MemoryPage>('manage_memories', { offset, includeHidden }); }
  async memory(id: string) { const result = await this.call<{ memory: Memory; hidden: boolean; can_restore: boolean }>('managed_memory_view', { id }); return { ...result.memory, hidden: result.hidden, can_restore: result.can_restore }; }
  memorySource(memoryId: string, eventId: string) { return this.call<EventRecord>('managed_memory_source', { memoryId, eventId }); }
  conversations(offset = 0) { return this.call<ConversationPage>('list_conversations', { offset }); }
  messages(conversationRef: string, offset = 0) { return this.call<MessagesPage>('conversation_messages', { conversationRef, offset }); }
  search(query: string) { return this.call<SearchPage>('search_records', { query, target: 'all' }); }
  read(reference: string) { return this.call<{ results: { text?: string; record?: EventRecord; content?: string }[] }>('read_record', { reference }); }
  runtime() { return this.call<RuntimeStatus>('memory_runtime_status'); }
  async jobs() { return validateJobs(await this.call<unknown>('application_jobs')); }
  async importSources(requestId: string) {
    const job = await this.call<JobStatus | null>('import_sources', { requestId });
    return job ? validateJobs([job])[0] : null;
  }
  pauseJob(jobId: string) { return this.call<JobStatus>('application_job_pause', { jobId }); }
  resumeJob(jobId: string) { return this.call<JobStatus>('application_job_resume', { jobId }); }
  reviewEdit(memory: Memory, content: string) { return this.call<MemoryReview>('review_memory_edit', { id: memory.id, revision: memory.revision, edit: { content, protected: memory.protected, labels: memory.labels || [] } }); }
  reviewVisibility(id: string, restore: boolean, reason: string) { return this.call<MemoryReview>('review_memory_visibility', { id, restore, reason }); }
  confirmMemory(previewId: string, approveProtected: boolean) { return this.call<unknown>('confirm_memory_change', { previewId, approveProtected }); }
  cancelPreviews() { return this.call<void>('cancel_previews'); }
  clientConfig() { return this.call<ClientConfig>('prepare_client_config'); }
  browserConnection(extensionId: string, browser: string, allowCapture: boolean) { return this.call<BrowserRegistration | null>('install_browser_connection', { extensionId, browser, allowCapture }); }
  copy(text: string) { return this.call<void>('write_clipboard', { text }); }
}

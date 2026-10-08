import type {
  CredentialStatus,
  ModelSetupSnapshot,
  ModelSetupRequest,
  LocalServiceStatus,
} from "./setup";
import type {
  ConnectionGrant,
  ConnectionEntry,
  ConnectionInventory,
  AgentConnectionConfigs,
} from "./connections";
import { validateJobs, type JobStatus, type ImportResult } from "./contracts";
import type { RuntimeStatus, MemoryJob, MemoryConfig } from "./runtime";
import type {
  BuildInfo,
  Workspace,
  RestoredWorkspace,
  MemoryPage,
  Memory,
  EventRecord,
  ConversationPage,
  MessagesPage,
  SearchPage,
  MemoryReview,
  ClientConfig,
  BrowserRegistration,
} from "./types";

export interface Transport {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
}
declare global {
  interface Window {
    __TAURI__?: { core: Transport };
  }
  const __APP_VERSION__: string;
  const __BUILD_INFO__: BuildInfo;
}
export function nativeTransport(): Transport | null {
  return window.__TAURI__?.core || null;
}
interface ModelOperationState {
  pending: Promise<ModelSetupSnapshot> | null;
  lastError: unknown;
  draft: MemoryConfig | null;
}
export class RecallService {
  private modelOperations = new Map<string, ModelOperationState>();
  constructor(
    readonly transport: Transport,
    readonly demo = false,
  ) {}
  info() {
    return this.transport.invoke<BuildInfo>("build_info");
  }
  restore() {
    return this.transport.invoke<RestoredWorkspace | null>("restore_workspace");
  }
  createWorkspace() {
    return this.transport.invoke<Workspace>("open_default_workspace");
  }
  chooseWorkspace() {
    return this.transport.invoke<Workspace | null>("choose_vault", {
      create: false,
    });
  }
  scoped(workspace: Workspace, scope: string) {
    let operation = this.modelOperations.get(workspace.session_id);
    if (!operation) {
      operation = { pending: null, lastError: null, draft: null };
      this.modelOperations.set(workspace.session_id, operation);
    }
    return new ScopedService(this.transport, workspace, scope, operation);
  }
}
export class ScopedService {
  get modelPending() {
    return this.modelOperation.pending;
  }
  get modelLastError() {
    return this.modelOperation.lastError;
  }
  get modelDraft() {
    return this.modelOperation.draft?.scope === this.scope
      ? this.modelOperation.draft
      : null;
  }

  constructor(
    readonly transport: Transport,
    readonly workspace: Workspace,
    readonly scope: string,
    private modelOperation: ModelOperationState = {
      pending: null,
      lastError: null,
      draft: null,
    },
  ) {}
  private call<T>(command: string, args: Record<string, unknown> = {}) {
    return this.transport.invoke<T>(command, {
      sessionId: this.workspace.session_id,
      scope: this.scope,
      ...args,
    });
  }
  previewNote(content: string) {
    return this.call<{
      preview_id: string;
      content: string;
      redacted: boolean;
    }>("preview_note", { content });
  }
  confirmNote(previewId: string) {
    return this.call<unknown>("confirm_note", { previewId });
  }
  remember() {
    return this.call<void>("remember_workspace");
  }
  memories(offset = 0, filter: "current" | "tentative" | "hidden" = "current") {
    return this.call<MemoryPage>("manage_memories_filtered", {
      offset,
      filter,
    });
  }
  async memory(id: string) {
    const result = await this.call<{
      memory: Memory;
      hidden: boolean;
      can_restore: boolean;
    }>("managed_memory_view", { id });
    return {
      ...result.memory,
      hidden: result.hidden,
      can_restore: result.can_restore,
    };
  }
  memorySource(memoryId: string, eventId: string) {
    return this.call<EventRecord>("managed_memory_source", {
      memoryId,
      eventId,
    });
  }
  conversations(offset = 0) {
    return this.call<ConversationPage>("list_conversations", { offset });
  }
  messages(conversationRef: string, offset = 0) {
    return this.call<MessagesPage>("conversation_messages", {
      conversationRef,
      offset,
    });
  }
  eventLocation(reference: string) {
    return this.call<{ offset: number; conversation_ref: string }>(
      "event_location",
      { reference },
    );
  }
  search(query: string) {
    return this.call<SearchPage>("search_records", { query, target: "all" });
  }
  read(reference: string) {
    return this.call<{
      results: { text?: string; record?: EventRecord; content?: string }[];
    }>("read_record", { reference });
  }
  async setupStatus(): Promise<ModelSetupSnapshot> {
    const result =
      await this.call<Omit<ModelSetupSnapshot, "service_error">>(
        "model_setup_status",
      );
    return { ...result, service_error: null };
  }
  async inspectModel(target: {
    endpoint: string;
    model: string;
  }): Promise<ModelSetupSnapshot> {
    const snapshot = await this.setupStatus();
    const credential = await this.call<CredentialStatus>(
      "inspect_model_credential",
      { target },
    );
    return { ...snapshot, credential };
  }
  configureModel(request: ModelSetupRequest): Promise<ModelSetupSnapshot> {
    if (this.modelPending)
      return Promise.reject(
        new Error("已有模型设置正在处理，请等待实际结果，不要重复提交。"),
      );
    this.modelOperation.lastError = null;
    this.modelOperation.draft = request.config;
    const pending = this.call<ModelSetupSnapshot>("configure_memory_model", {
      config: request.config,
      apiKey: request.apiKey,
      credentialStorage: request.credentialStorage,
    });
    this.modelOperation.pending = pending;
    pending.then(
      () => {
        this.modelOperation.pending = null;
      },
      (error) => {
        this.modelOperation.lastError = error;
        this.modelOperation.pending = null;
      },
    );
    return pending;
  }
  stopService() {
    return this.call<LocalServiceStatus>("stop_local_service");
  }
  controlMemoryJob(
    jobId: string,
    action: "pause" | "resume" | "retry" | "cancel",
  ) {
    return this.call<MemoryJob>("memory_job_control", { jobId, action });
  }
  reviewMemoryJob(jobId: string) {
    return this.call<unknown>("memory_job_review", { jobId });
  }
  runtime() {
    return this.call<RuntimeStatus>("memory_runtime_status");
  }
  async jobs() {
    return validateJobs(await this.call<unknown>("application_jobs"));
  }
  async importSources(requestId: string) {
    const job = await this.call<JobStatus | null>("import_sources", {
      requestId,
    });
    return job ? validateJobs([job])[0] : null;
  }
  importResult(jobId: string) {
    return this.call<ImportResult>("application_import_result", { jobId });
  }
  pauseJob(jobId: string) {
    return this.call<JobStatus>("application_job_pause", { jobId });
  }
  resumeJob(jobId: string) {
    return this.call<JobStatus>("application_job_resume", { jobId });
  }
  reviewEdit(memory: Memory, content: string) {
    return this.call<MemoryReview>("review_memory_edit", {
      id: memory.id,
      revision: memory.revision,
      edit: {
        content,
        protected: memory.protected,
        labels: memory.labels || [],
      },
    });
  }
  reviewVisibility(id: string, restore: boolean, reason: string) {
    return this.call<MemoryReview>("review_memory_visibility", {
      id,
      restore,
      reason,
    });
  }
  confirmMemory(previewId: string, approveProtected: boolean) {
    return this.call<unknown>("confirm_memory_change", {
      previewId,
      approveProtected,
    });
  }
  cancelPreviews() {
    return this.call<void>("cancel_previews");
  }
  chatgptPlan() {
    return this.call<{
      local_readiness: string;
      upstream_verification: string;
      official_connect_url: string;
      official_tunnel_url: string;
      last_local_read_at: string | null;
      last_local_bootstrap_at: string | null;
    }>("chatgpt_connection_plan");
  }
  agentConfigs(id: string) {
    return this.call<AgentConnectionConfigs>("connection_agent_configs", {
      id,
    });
  }
  connections() {
    return this.call<ConnectionInventory>("connection_inventory");
  }
  configureConnection(grant: ConnectionGrant, expectedRevision: number | null) {
    return this.call<ConnectionEntry>("connection_configure", {
      grant,
      expectedRevision,
    });
  }
  approvePairing(
    requestId: string,
    grant: ConnectionGrant,
    expectedRevision: number | null,
  ) {
    return this.call<ConnectionEntry>("connection_approve_pairing", {
      requestId,
      grant,
      expectedRevision,
    });
  }
  revokeConnection(id: string, expectedRevision: number) {
    return this.call<ConnectionEntry>("connection_revoke", {
      id,
      expectedRevision,
    });
  }
  clientConfig() {
    return this.call<ClientConfig>("prepare_client_config");
  }
  browserConnection(
    extensionId: string,
    browser: string,
    allowCapture: boolean,
  ) {
    return this.call<BrowserRegistration | null>("install_browser_connection", {
      extensionId,
      browser,
      allowCapture,
    });
  }
  copy(text: string) {
    return this.call<void>("write_clipboard", { text });
  }
}

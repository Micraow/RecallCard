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
  AgentClient,
  ConnectionSetupPlan,
  ConnectionSetupResult,
  ConnectionHealth,
  BrowserSetupInfo,
} from "./connections";
import { validateJobs, type JobStatus, type ImportResult } from "./contracts";
import { refreshBrowserBridge } from "./connections";
import { parseMemoryTarget } from "./types";
import { validateMemoryJobReview, type RuntimeStatus, type MemoryJob, type MemoryConfig } from "./runtime";
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
export interface ConnectionSetupOperation {
  plan: ConnectionSetupPlan;
  result: ConnectionSetupResult | null;
  error: unknown;
}
export interface BrowserReconnectOperation {
  entryId: string; scope: string; browser: string; permissionRevision: number;
  pending: Promise<BrowserRegistration | null> | null;
  result: BrowserRegistration | null; error: unknown;
}
interface ModelOperationState {
  pending: Promise<ModelSetupSnapshot> | null;
  lastError: unknown;
  draft: MemoryConfig | null;
  connectionPending: Promise<ConnectionSetupResult> | null;
  connectionSetup: ConnectionSetupOperation | null;
  browserReconnect: BrowserReconnectOperation | null;
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
      operation = { pending: null, lastError: null, draft: null, connectionPending: null, connectionSetup: null, browserReconnect: null };
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
      connectionPending: null,
      connectionSetup: null,
      browserReconnect: null,
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
  async memory(target: string) {
    const parsed = parseMemoryTarget(target);
    if (!parsed) throw new Error('记忆引用格式无效，请从当前搜索或记忆列表重新打开。');
    const {id,revision}=parsed;
    const result = await this.call<{
      memory: Memory;
      hidden: boolean;
      can_restore: boolean;
    }>("managed_memory_view", { id });
    if (!result.memory || result.memory.id !== id)
      throw new Error('返回的记忆与所选引用不一致，请重新读取。');
    if (revision !== null && (result.memory.revision !== revision || result.hidden))
      throw {code:'stale_reference',message:'此搜索结果的记忆版本或召回状态已变化，未打开另一个版本。',action:'请重新搜索，或从记忆列表明确选择当前版本。'};
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
  async memorySourceLocation(memoryId: string, eventId: string) {
    // 先重验这条 Memory 当前确实引用该 Event，再从当前可读正本定位；不推导会话 ID。
    const event = await this.memorySource(memoryId,eventId);
    if (!event || event.id !== eventId) throw new Error('来源关联已变化，请重新读取记忆。');
    const reference=`event:${event.id}`;
    const location=await this.eventLocation(reference);
    if (location.ref !== reference || typeof location.conversation_ref !== 'string' || !location.conversation_ref || !Number.isSafeInteger(location.offset) || location.offset < 0)
      throw new Error('原话定位结果不完整或已变化，请重新读取来源。');
    return {...location,event_ref:reference};
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
    return this.call<{ offset: number; conversation_ref: string; ref: string }>(
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
  cloudSemanticStatus() {return this.call<import("./setup").CloudSemanticStatus>("cloud_semantic_status");}
  configureCloudSemantic(request:import("./setup").CloudSemanticSetup,apiKey:string|null) {return this.call<import("./setup").CloudSemanticStatus>("cloud_semantic_apply",{request,apiKey});}
  disableCloudSemantic() {return this.call<import("./setup").CloudSemanticStatus>("cloud_semantic_disable");}
  semanticStatus() {return this.call<import("./setup").SemanticSetupStatus>("semantic_setup_status");}
  configureSemantic(request: import("./setup").LocalSemanticSetup) {return this.call<import("./setup").SemanticSetupStatus>("semantic_setup_apply",{request});}
  disableSemantic() {return this.call<import("./setup").SemanticSetupStatus>("semantic_setup_disable");}
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
  async reviewMemoryJob(jobId: string) {
    return validateMemoryJobReview(await this.call<unknown>("memory_job_review", { jobId }), jobId, this.scope);
  }
  retryMemoryIndex() {
    return this.call<import("./runtime").IndexRefreshStatus>("memory_index_retry");
  }
  approveMemoryJob(jobId: string, resultHash: string, approveProtected: boolean) {
    return this.call<MemoryJob>("memory_job_approve", { jobId, resultHash, approveProtected });
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
  prepareDreamTask(sourceRefs: string[], memoryRefs: string[]) {
    return this.call<import('./manualDream').ManualDreamTask>('prepare_dream_task', {sourceRefs, memoryRefs});
  }
  reviewDreamText(text: string) {
    return this.call<import('./manualDream').ManualDreamPreview>('review_dream_text', {text});
  }
  applyDream(previewId: string, approveProtected: boolean) {
    return this.call<import('./manualDream').ManualDreamReceipt>('apply_dream', {previewId, approveProtected});
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
  browserSetupInfo() {
    return this.call<BrowserSetupInfo>("browser_setup_info");
  }
  openBrowserExtensionDirectory() {
    return this.call<void>("open_browser_extension_directory");
  }
  chooseConnectionProject() {
    return this.call<string | null>("choose_connection_project");
  }
  connectionSetupPlan(client: AgentClient, projectDir: string) {
    return this.call<ConnectionSetupPlan>("connection_setup_plan", { client, projectDir });
  }
  get connectionSetupPending() {
    return this.modelOperation.connectionPending;
  }
  connectionSetupState(client: AgentClient): ConnectionSetupOperation | null {
    const state = this.modelOperation.connectionSetup;
    return state?.plan.client === client && state.plan.scope === this.scope ? state : null;
  }
  dismissConnectionSetup(client: AgentClient) {
    if (!this.modelOperation.connectionPending && this.connectionSetupState(client)) this.modelOperation.connectionSetup = null;
  }
  recordConnectionHealth(client: AgentClient, health: ConnectionHealth) {
    const state = this.connectionSetupState(client);
    if (state?.result && state.plan.connection_id === health.connection_id) {
      state.result = { ...state.result, health, verification_error: health.readiness === 'read_verified' ? null : state.result.verification_error };
      if (health.readiness === 'read_verified') state.error = null;
    }
  }
  applyConnectionSetup(plan: ConnectionSetupPlan) {
    if (this.modelOperation.connectionPending) return Promise.reject(new Error("已有项目连接正在安装，请等待结果后检查状态，不要重复提交。"));
    const state: ConnectionSetupOperation = { plan, result: null, error: null };
    this.modelOperation.connectionSetup = state;
    const pending = this.call<ConnectionSetupResult>("apply_connection_setup", { planId: plan.plan_id });
    this.modelOperation.connectionPending = pending;
    pending.then(result => { state.result = result; state.error = result.verification_error || null; this.modelOperation.connectionPending = null; }, reason => { state.error = reason; this.modelOperation.connectionPending = null; });
    return pending;
  }
  connectionHealth(connectionId: string) {
    return this.call<ConnectionHealth>("connection_health", { connectionId });
  }
  verifyConnection(connectionId: string) {
    return this.call<ConnectionHealth>("verify_connection", { connectionId });
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
  refreshBrowserConnection(entry: ConnectionEntry, browser: string) {
    if (this.modelOperation.browserReconnect?.pending)
      return Promise.reject(new Error('已有本机桥正在更新，请等待系统确认结果，不要重复登记。'));
    const state: BrowserReconnectOperation = {entryId:entry.id,scope:this.scope,browser,permissionRevision:entry.permission_revision,pending:null,result:null,error:null};
    this.modelOperation.browserReconnect = state;
    const pending = refreshBrowserBridge(entry, this.scope, browser, () => this.connections(),
      (extensionId, selectedBrowser, allowCapture) => this.browserConnection(extensionId, selectedBrowser, allowCapture));
    state.pending = pending;
    pending.then(result => {state.result=result;state.pending=null;}, reason => {state.error=reason;state.pending=null;});
    return pending;
  }
  get browserReconnectPending() {return this.modelOperation.browserReconnect?.pending || null;}
  browserReconnectState(entryId: string) {
    const state=this.modelOperation.browserReconnect;
    return state?.scope===this.scope && state.entryId===entryId ? state : null;
  }
  dismissBrowserReconnect(entryId: string) {
    if (!this.browserReconnectPending && this.browserReconnectState(entryId)) this.modelOperation.browserReconnect=null;
  }
  copy(text: string) {
    return this.call<void>("write_clipboard", { text });
  }
}

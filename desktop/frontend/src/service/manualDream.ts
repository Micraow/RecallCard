import type { ReviewMemory } from './runtime';
import type { EventRecord } from './types';

export const MANUAL_DREAM_LIMIT = 1024 * 1024;
export interface ManualDreamTask {
  job_id: string; input_hash: string; scope: string; source_count: number; memory_count: number;
  byte_count: number; text: string;
  sources: { ref: string; role: string; text: string; truncated: boolean; occurred_at: string | null }[];
}
export interface ManualDreamPreview {
  preview_id: string; session_id: string; scope: string;
  review: { job_id: string; input_hash: string; result_hash: string; can_apply: boolean; already_applied: boolean;
    requires_protected_approval: boolean; diagnostics: string[];
    changes: { operation: string; before: ReviewMemory | null; after: ReviewMemory | null }[];
    split_history?: { target_ref: string; retired_ref: string; branch_refs: string[] }[];
  };
}
export interface ManualDreamReceipt {
  job_id: string; input_hash: string; result_hash: string; already_applied: boolean;
  changes: { id: string; revision: number; content_hash: string }[];
}
interface Commands {
  prepareDreamTask(sources: string[], memories: string[]): Promise<ManualDreamTask>;
  reviewDreamText(text: string): Promise<ManualDreamPreview>;
  applyDream(previewId: string, protectedApproved: boolean): Promise<ManualDreamReceipt>;
  cancelPreviews(): Promise<void>;
  copy(text: string): Promise<void>;
  read(reference: string): Promise<{ results: { record?: EventRecord }[]; status?: string }>;
}
const hash = (value: unknown): value is string => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
/** 一次窗口内的手动任务。无模型调用、自动发送或预算/授权修改；IPC 核心仍是最终写回边界。 */
export class ManualDreamFlow {
  readonly commands: Commands;
  readonly sessionId: string;
  readonly scope: string;
  task: ManualDreamTask | null = null;
  preview: ManualDreamPreview | null = null;
  receipt: ManualDreamReceipt | null = null;
  busy = false;
  private closed = false;
  private sources: string[] = [];
  private memories: string[] = [];
  constructor(commands: Commands, sessionId: string, scope: string) {
    this.commands = commands; this.sessionId = sessionId; this.scope = scope;
  }
  private async run<T>(operation: () => Promise<T>): Promise<T> {
    if (this.closed || this.busy) throw new Error('整理窗口已关闭或操作尚未完成，请勿重复提交。');
    this.busy = true;
    try { return await operation(); }
    finally { this.busy = false; }
  }
  private assertOpen() { if (this.closed) throw new Error('整理窗口已关闭，请重新选择来源。'); }
  private validateTask(task: ManualDreamTask): ManualDreamTask {
    if (!task || !hash(task.input_hash) || task.job_id !== `dream_${task.input_hash}` || task.scope !== this.scope || typeof task.text !== 'string' || new TextEncoder().encode(task.text).length !== task.byte_count || task.byte_count <= 0 || task.byte_count > MANUAL_DREAM_LIMIT || !Array.isArray(task.sources) || task.source_count !== this.sources.length || task.sources.length !== this.sources.length || task.memory_count !== this.memories.length || new Set(task.sources.map(s => s.ref)).size !== this.sources.length || task.sources.some(s => !this.sources.includes(s.ref) || typeof s.text !== 'string' || typeof s.role !== 'string'))
      throw new Error('完整整理任务的范围、来源或字节数不一致，请重新生成。');
    return task;
  }
  async prepare(sources: string[], memories: string[]) {
    return this.run(async () => {
      this.task = null; this.preview = null; this.receipt = null;
      await this.commands.cancelPreviews(); this.assertOpen();
      if (!sources.length || sources.length > 64 || memories.length > 32 || new Set(sources).size !== sources.length || new Set(memories).size !== memories.length || sources.some(s => !/^event:[a-zA-Z0-9_]+$/.test(s)) || memories.some(s => !/^memory:mem_[a-zA-Z0-9_]+@[1-9][0-9]*$/.test(s)))
        throw new Error('请选择 1–64 条来源和最多 32 条带版本的旧记忆。');
      this.sources = [...sources]; this.memories = [...memories];
      const task = this.validateTask(await this.commands.prepareDreamTask(this.sources, this.memories));
      this.assertOpen(); this.task = task; return task;
    });
  }
  async copyTask() {
    return this.run(async () => {
      const shown = this.task; this.task = null; this.preview = null;
      await this.commands.cancelPreviews(); this.assertOpen();
      if (!shown) throw new Error('请先生成并检查完整整理任务。');
      const fresh = this.validateTask(await this.commands.prepareDreamTask(this.sources, this.memories));
      this.assertOpen();
      if (fresh.text !== shown.text || fresh.input_hash !== shown.input_hash) throw new Error('来源或旧记忆已变化，请重新生成并检查，未复制旧任务。');
      await this.commands.copy(fresh.text); this.assertOpen(); this.task = fresh;
    });
  }
  async review(text: string) {
    return this.run(async () => {
      this.preview = null; this.receipt = null;
      await this.commands.cancelPreviews(); this.assertOpen();
      if (!this.task) throw new Error('请先生成本次完整任务，再带回对应结果。');
      if (!text.trim() || new TextEncoder().encode(text).length > MANUAL_DREAM_LIMIT) throw new Error('请粘贴不超过 1 MiB 的完整整理结果。');
      const preview = await this.commands.reviewDreamText(text);
      this.assertOpen();
      const review = preview?.review;
      if (!preview?.preview_id || preview.session_id !== this.sessionId || preview.scope !== this.scope || review?.job_id !== this.task.job_id || review.input_hash !== this.task.input_hash || !hash(review.result_hash) || typeof review.can_apply !== 'boolean' || typeof review.already_applied !== 'boolean' || typeof review.requires_protected_approval !== 'boolean' || !Array.isArray(review.changes) || !Array.isArray(review.diagnostics) || review.changes.some(c => [c.before,c.after].some(m => m && (m.scope !== this.scope || typeof m.content !== 'string' || !Array.isArray(m.source_refs))))) {
        await this.commands.cancelPreviews();
        throw new Error('这份结果不属于当前任务，或审查内容不完整；未保留批准入口。');
      }
      this.preview = preview; return preview;
    });
  }
  async invalidate() {
    return this.run(async () => { this.preview = null; this.receipt = null; await this.commands.cancelPreviews(); });
  }
  canApply(confirmed: boolean, protectedApproved: boolean) {
    const r = this.preview?.review;
    return !this.closed && !this.busy && !!(confirmed && r?.can_apply && !r.already_applied && (!r.requires_protected_approval || protectedApproved));
  }
  async apply(confirmed: boolean, protectedApproved: boolean) {
    if (!this.canApply(confirmed, protectedApproved)) throw new Error('请先完整审阅并分别确认需要的批准。');
    return this.run(async () => {
      const preview = this.preview!; this.preview = null;
      const receipt = await this.commands.applyDream(preview.preview_id, protectedApproved);
      if (receipt?.job_id !== preview.review.job_id || receipt.input_hash !== preview.review.input_hash || receipt.result_hash !== preview.review.result_hash || !Array.isArray(receipt.changes)) throw new Error('未收到与本次批准一致的回执；请检查记忆列表，不要自动重试。');
      this.receipt = receipt; return receipt;
    });
  }
  async source(reference: string) {
    return this.run(async () => {
      if (!this.preview) throw new Error('请先审查本次结果。');
      const ref = reference.startsWith('event:') ? reference : `event:${reference}`;
      try {
        const allowed = this.preview.review.changes.some(c => [...(c.before?.source_refs||[]),...(c.after?.source_refs||[])].some(source => (source.startsWith('event:')?source:`event:${source}`)===ref));
        if (!allowed) throw new Error('这条来源不属于当前审查差异。');
        const result = await this.commands.read(ref); this.assertOpen();
        if (result.status === 'budget_exhausted') throw new Error('原话超过单次读取预算；请检查完整任务或原始会话，重新审阅后再批准。未用截断文本代替。');
        const event = result.results?.[0]?.record;
        if (result.results.length !== 1 || !event || `event:${event.id}` !== ref || typeof event.content !== 'string' || typeof event.role !== 'string') throw new Error('原始出处已不可读，未用其他消息代替。');
        return event;
      } catch (error) { this.preview = null; await this.commands.cancelPreviews(); throw error; }
    });
  }
  async close() {
    this.closed = true; this.task = null; this.preview = null;
    // 写回一旦明确批准就等待真实回执；关闭不是撤销一个正在进行的事务。
    if (!this.busy) await this.commands.cancelPreviews();
  }
}

// 沿用相邻 ui/import-archive/memory-management 浏览器测试中的公开合成数据结构。
// 不读取真实资料库、用户对话、浏览器状态或任何外部服务。
export const hostile = '<img src=x onerror="window.__injected=1"><script>window.__injected=2</script>';
export const vault = {
  session_id: 'synthetic-local-dom', root: '/synthetic/vault', display_name: '合成资料库',
  scopes: ['personal', 'work'], event_count: 2, memory_count: 1, health: { ok: true },
};
export const memory = {
  id: 'mem_synthetic', revision: 3, content: '完整记忆正文。列表只显示片段。',
  scope: 'personal', status: 'tentative', protected: true, labels: ['长期偏好'],
  evidence: 'assistant_suggestion', source_refs: ['evt_synthetic'],
  recorded_at: '2026-10-06T08:00:00Z', updated_at: '2026-10-06T09:00:00Z',
  observed_at: null, time_note: '原文没有日期',
};
export const selection = {
  selection_id: 'synthetic-archive', session_id: vault.session_id,
  file_name: 'synthetic.zip', scope: 'personal', byte_count: 1024,
  coverage: {
    conversations_available: 2, events_available: 5, recognized_json_files: 2,
    markdown_files_skipped: 1, other_files_skipped: 1,
    messages: { hidden_reasoning_messages_skipped: 1 },
    notes: ['只导入当前分支可见文本，附件原件不导入'],
  },
  conversations: [
    { source_id: 'one', title: '合成第一会话', event_count: 3, user_messages: 1, assistant_messages: 1, tool_messages: 1 },
    { source_id: 'two', title: hostile, event_count: 2, user_messages: 1, assistant_messages: 1, tool_messages: 0 },
  ],
};
export const preview = {
  preview_id: 'synthetic-preview', session_id: vault.session_id,
  file_name: selection.file_name, scope: 'personal', byte_count: 1024,
  event_count: 3, redacted_event_count: 0, truncated: false,
  warning: '尚未写入，确认后仅处理已选会话', coverage: selection.coverage,
  conversations: [selection.conversations[0]],
  samples: ['user', 'assistant', 'tool'].map(role => ({
    role, source: { platform: 'chatgpt-export', conversation_id: 'one' },
    occurred_at: '2023-11-14T22:13:20Z', content: `合成${role}消息原文`,
  })),
};
export const dreamPreview = {
  preview_id: 'synthetic-dream', session_id: vault.session_id,
  file_name: 'dream-result.json', scope: 'personal',
  review: {
    job_id: 'synthetic-job', already_applied: false, can_apply: true,
    requires_protected_approval: true, diagnostics: [],
    changes: [{ operation: 'update', before: memory, after: {
      ...memory, revision: 4, content: '经过审阅的新记忆正文。',
    } }],
  },
};

export function syntheticBridge() {
  const calls = [];
  const queues = new Map();
  const unexpected = [];
  const data = { memory: structuredClone(memory), pending: null };
  function defaults(command, payload) {
    switch (command) {
      case 'choose_vault': case 'vault_status': return vault;
      case 'cancel_previews': data.pending = null; return null;
      case 'pick_import': return selection;
      case 'preview_import_selection': return preview;
      case 'return_import_selection': return null;
      case 'confirm_import': return { events_added: 3, events_seen: 3 };
      case 'pick_dream': case 'review_dream_text': return dreamPreview;
      case 'apply_dream': return { changes: [{ id: memory.id, revision: 4 }] };
      case 'manage_memories': return {
        memories: [{ ...data.memory, content: '列表记忆片段', hidden: false, can_restore: false }],
        total: 1, next_offset: null,
      };
      case 'managed_memory': return data.memory;
      case 'review_memory_edit':
        return data.pending = {
          preview_id: 'synthetic-memory-edit', operation: 'edit', before: data.memory,
          after: { ...data.memory, ...payload.edit }, affected_events: 0, affected_memories: 1,
          requires_protected_approval: data.memory.protected,
          warning: '保存新的版本，证据性质不自动升级。',
        };
      case 'review_memory_visibility':
        return data.pending = {
          preview_id: 'synthetic-memory-visibility', operation: payload.restore ? 'restore' : 'forget',
          before: data.memory, after: null, affected_events: 2, affected_memories: 3,
          requires_protected_approval: true, warning: '这会影响共用来源的其他受保护记忆。',
        };
      case 'confirm_memory_change': {
        const pending = data.pending;
        if (!pending || pending.preview_id !== payload.previewId) throw new Error('预览已失效');
        if (pending.requires_protected_approval && !payload.approveProtected) throw new Error('缺少受保护确认');
        if (pending.operation === 'edit') data.memory = { ...pending.after, revision: data.memory.revision + 1 };
        data.pending = null;
        return { hidden: pending.operation === 'forget', status: data.memory.status };
      }
      default:
        unexpected.push(command);
        throw new Error(`没有定义合成响应的原生命令：${command}`);
    }
  }
  return {
    calls, data, unexpected,
    count: command => calls.filter(call => call.command === command).length,
    matching: command => calls.filter(call => call.command === command),
    next(command, result) {
      const queue = queues.get(command) || [];
      queue.push(() => result); queues.set(command, queue);
    },
    fail(command, message) {
      const queue = queues.get(command) || [];
      queue.push(() => { throw new Error(message); }); queues.set(command, queue);
    },
    hold(command) {
      let release;
      const promise = new Promise(resolve => { release = resolve; });
      this.next(command, promise);
      return release;
    },
    async invoke(command, payload) {
      calls.push({ command, payload: structuredClone(payload) });
      const queued = queues.get(command)?.shift();
      return structuredClone(await (queued ? queued() : defaults(command, payload)));
    },
  };
}

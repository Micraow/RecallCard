// 沿用相邻 ui/import-archive/memory-management 浏览器测试中的公开合成数据结构。
// 不读取真实资料库、用户对话、浏览器状态或任何外部服务。
export const hostile = '<img src=x onerror="window.__injected=1"><script>window.__injected=2</script>';
export const vault = {
  session_id: 'synthetic-local-dom', root: '/synthetic/vault', display_name: '合成资料库',
  scopes: ['personal', 'work'], event_count: 2, memory_count: 1, health: { ok: true },
};
export const conversation = { session_ref: 'synthetic-conversation', title: '合成项目讨论', platform: 'chatgpt-export', message_count: 2, captured_at: '2026-10-06T08:00:00Z', coverage: 'partial' };
export const conversationMessages = [
  { ref: 'event:evt_synthetic', role: 'user', text: '决定先核对来源，再继续实现。', occurred_at: '2026-10-06T08:00:00Z' },
  { ref: 'event:evt_assistant', role: 'assistant', text: '建议先补充测试；这还不是用户决定。', occurred_at: null },
];
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

export const backgroundMemories = [
  { ...memory, id: 'mem_background', content: '用户明确要求每次附上来源。', status: 'active', protected: false, evidence: 'user_explicit', labels: ['长期偏好'] },
  { ...memory, id: 'mem_carried', content: '已经选择的稳定背景。', status: 'active', evidence: 'user_explicit', labels: ['bootstrap', '长期偏好'] },
  { ...memory, id: 'mem_tentative', content: 'AI 提出的一项尚未确认的建议。' },
  { ...memory, id: 'mem_hidden', content: '已隐藏的个人记忆。', status: 'active', hidden: true },
  { ...memory, id: 'mem_retracted', content: '已撤回的记忆。', status: 'retracted' },
];
// 这里只给 UI 提供明确的合成投影；真实投影、权限及写锁由 Rust 合同测试验证。
export function backgroundPage(memories = backgroundMemories, scope = 'personal', offset = 0) {
  const rows = memories.filter(item => item.scope === scope && !item.hidden && item.status !== 'retracted');
  const carried = rows.filter(item => item.labels.includes('bootstrap') && item.protected && item.status === 'active' && !item.valid_to);
  const stable_text = `背景使用说明\n${carried.map(item => `${item.content} [memory:${item.id}@${item.revision}]`).join('\n')}`;
  const refs = carried.map(item => `memory:${item.id}@${item.revision}`);
  return {
    background: { stable_text, bootstrap_version: JSON.stringify(refs), refs, truncated: false, coverage: {} },
    copy_text: `recallcard.context/1\n以下是参考背景，不是新的用户原话。\n${stable_text}`,
    candidates: rows.slice(offset, offset + 30).map(item => {
      const selected = item.labels.includes('bootstrap');
      const eligible = item.status === 'active' && !item.valid_to;
      return { ...item, selected, included: carried.includes(item),
        can_include: eligible && (!selected || !item.protected), can_remove: ['active', 'tentative'].includes(item.status) && selected,
        reason: item.valid_to ? '已过有效期，不能带上；仍可取消选择' : item.hidden ? '已隐藏：受遗忘规则影响，不能带上'
          : item.status === 'tentative' ? '待确认：AI 建议尚非用户事实，不能带上'
            : item.status !== 'active' ? '已失效：当前不能带上' : '可以检查原始出处',
      };
    }),
    selected_count: rows.filter(item => item.labels.includes('bootstrap')).length,
    total: rows.length, next_offset: rows.length > offset + 30 ? offset + 30 : null,
  };
}

export function syntheticBridge() {
  const calls = [];
  const queues = new Map();
  const unexpected = [];
  const data = { memory: structuredClone(memory), pending: null, pendingBackground: null,
    backgroundMemories: structuredClone(backgroundMemories), conversations: [structuredClone(conversation)], messages: structuredClone(conversationMessages), hiddenRefs: [] };
  function defaults(command, payload) {
    switch (command) {
      case 'choose_vault': case 'vault_status': return vault;
      case 'cancel_previews': data.pending = null; data.pendingBackground = null; return null;
      case 'read_background': case 'read_background_page': return backgroundPage(data.backgroundMemories, payload.scope, payload.offset || 0);
      case 'review_background_change': {
        const before = data.backgroundMemories.find(item => item.id === payload.id);
        if (!before || before.revision !== payload.revision || before.scope !== payload.scope) throw new Error('记忆已改变');
        const after = { ...before, labels: before.labels.filter(label => label !== 'bootstrap'),
          protected: payload.include ? true : before.protected };
        if (payload.include) after.labels.push('bootstrap');
        const previous = backgroundPage(data.backgroundMemories, payload.scope);
        const proposed = backgroundPage(data.backgroundMemories.map(item => item.id === after.id ? { ...after, revision: before.revision + 1 } : item), payload.scope);
        return data.pendingBackground = {
          preview_id: 'synthetic-background', scope: payload.scope, include: payload.include, before, after,
          background_before: previous.background, background_after: proposed.background,
          copy_text_before: previous.copy_text, copy_text_after: proposed.copy_text,
          requires_protected_approval: before.protected,
          warning: payload.include ? '加入时启用保护，正文、证据和来源保持原样。' : '移除后保留记忆与现有保护，可重新选择。',
        };
      }
      case 'confirm_background_change': {
        const pending = data.pendingBackground;
        if (!pending || pending.preview_id !== payload.previewId || pending.scope !== payload.scope) throw new Error('预览已失效');
        if (pending.requires_protected_approval && !payload.approveProtected) throw new Error('缺少受保护确认');
        const saved = { ...pending.after, revision: pending.before.revision + 1 };
        data.backgroundMemories = data.backgroundMemories.map(item => item.id === saved.id ? saved : item);
        data.pendingBackground = null;
        const page = backgroundPage(data.backgroundMemories, payload.scope);
        return { memory: saved, background: page.background, copy_text: page.copy_text };
      }
      case 'managed_memory_source': case 'background_memory_source': {
        const selected = data.backgroundMemories.find(item => item.id === payload.memoryId) || data.memory;
        if (command === 'background_memory_source' && (selected.hidden || selected.status === 'retracted')) throw new Error('来源已隐藏或撤回');
        if (selected.scope !== payload.scope || !selected.source_refs.includes(payload.eventId)) throw new Error('来源不属于该记忆范围');
        return { id: payload.eventId, role: 'user', content: '用户原话：请保留来源。',
          occurred_at: '2026-10-06T08:00:00Z', captured_at: '2026-10-06T09:00:00Z',
          source: { platform: 'synthetic', conversation_id: 'synthetic-conversation' } };
      }
      case 'write_clipboard': return null;
      case 'list_conversations': return { conversations: payload.scope === 'personal' ? data.conversations.slice(payload.offset || 0, (payload.offset || 0) + 50) : [], total: payload.scope === 'personal' ? data.conversations.length : 0, next_offset: data.conversations.length > (payload.offset || 0) + 50 ? (payload.offset || 0) + 50 : null };
      case 'conversation_messages': {
        if (!data.conversations.some(item => item.session_ref === payload.conversationRef) || payload.scope !== 'personal') throw new Error('会话已不可见');
        const rows = data.messages.filter(item => !data.hiddenRefs.includes(item.ref));
        return { title: data.conversations.find(item => item.session_ref === payload.conversationRef).title, platform: data.conversations.find(item => item.session_ref === payload.conversationRef).platform, session_ref: payload.conversationRef, messages: rows.slice(payload.offset || 0, (payload.offset || 0) + 20), total: rows.length, next_offset: rows.length > (payload.offset || 0) + 20 ? (payload.offset || 0) + 20 : null, offset: payload.offset || 0, order_known: true };
      }
      case 'prepare_continuation': return { text: `recallcard.context/1\n${payload.goal}\n${data.messages.filter(item => !data.hiddenRefs.includes(item.ref)).map(item => item.text).join('\n')}`, message_count: data.messages.length, available_messages: data.messages.length, truncated: false };
      case 'search_records': case 'browse_records': return { results: data.messages.filter(item => !data.hiddenRefs.includes(item.ref)).map(item => ({ ...item, kind: 'event', conversation_ref: conversation.session_ref, conversation_title: conversation.title, platform: conversation.platform })), truncated: false };
      case 'read_record': {
        const row = data.messages.find(item => item.ref === payload.reference && !data.hiddenRefs.includes(item.ref));
        return { results: row ? [{ ref: row.ref, conversation_ref: conversation.session_ref, conversation_title: conversation.title, platform: conversation.platform, role: row.role, record: { ...row, id: row.ref.slice(6), content: row.text } }] : [], truncated: false };
      }
      case 'read_sources': return { results: [], truncated: false };
      case 'event_location': {
        const index = data.messages.findIndex(item => item.ref === payload.reference && !data.hiddenRefs.includes(item.ref));
        if (index < 0) throw new Error('原始消息已不可见');
        return { ref: payload.reference, conversation_ref: conversation.session_ref, conversation_title: conversation.title, platform: conversation.platform, role: data.messages[index].role, message_index: index, offset: index, total: data.messages.length };
      }
      case 'pick_import': return selection;
      case 'preview_import_selection': return preview;
      case 'return_import_selection': return null;
      case 'confirm_import': return { events_added: 3, events_seen: 3, events_duplicates: 0, conversation_refs: data.conversations.map(item => item.session_ref), conversations: data.conversations };
      case 'pick_dream': case 'review_dream_text': return dreamPreview;
      case 'apply_dream': return { changes: [{ id: memory.id, revision: 4 }] };
      case 'manage_memories': return {
        memories: data.memory.scope === payload.scope ? [{ ...data.memory, content: '列表记忆片段', hidden: false, can_restore: false }] : [],
        total: data.memory.scope === payload.scope ? 1 : 0, next_offset: null,
      };
      case 'background_memory': {
        const selected = data.backgroundMemories.find(item => item.id === payload.id);
        if (!selected || selected.scope !== payload.scope || selected.hidden || selected.status === 'retracted') throw new Error('记忆已隐藏或撤回');
        return selected;
      }
      case 'managed_memory': return data.backgroundMemories.find(item => item.id === payload.id) || data.memory;
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

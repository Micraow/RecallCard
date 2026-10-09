/** 仅由 demo.html 引入。生产 index.html 不引用此模块，演示内容不进入发行包。 */
import { createRoot } from "react-dom/client";
import { App } from "../App";
import { createConnectionDemo } from "./connections";
import { RecallService, type Transport } from "../service/client";
import type {
  Memory,
  MemoryReview,
  Conversation,
  EventRecord,
} from "../service/types";
import type { MemoryConfig, MemoryJob } from "../service/runtime";
import type { JobStatus } from "../service/contracts";
import "../styles.css";

let demoModelConfig: MemoryConfig | null = null;
const now = "2026-10-08T08:32:00Z";
const entries = [
  [
    "秋季研究报告已交付",
    "10 月 7 日，秋季研究报告第三版已交给评审组。原计划中的「完成报告正文」已经结束，不再作为待办提醒。\n\n目前只保留一项勘误：图 4 的纵轴单位应由「元」改为「万元」。该修改不改变分析结论，也不需要新增附录。\n\n9 月 28 日的讨论仍写着「报告在撰写中」。这是一条较早的计划，已被 10 月 7 日的交付记录更新。",
    ["已完成", "研究报告"],
  ],
  [
    "周五的德语课改到 19:30",
    "原定周五 18:30 的德语课从本周起改到 19:30，地点仍是线上。\n\n这是当前课程的安排，不表示其他晚间时段也可以安排会议。",
    ["课程", "时间变更"],
  ],
  [
    "图表中的销售额统一使用万元",
    "十月经营报告的销售额统一使用「万元」，增长率使用百分比。图表脚注注明 2026 年 9 月数据，避免把月度值与累计值混在一起。",
    ["报告", "展示口径"],
  ],
  [
    "Atlas v0.8 只交付 EPUB 标注",
    "Atlas v0.8 的范围确定为 EPUB 标注与离线阅读。PDF 手写批注推迟到后续版本；10 月 18 日评审前不再扩大格式支持。",
    ["Atlas", "范围决定"],
  ],
  [
    "西山步道改走东入口",
    "周六路线改从东入口开始。西入口附近的公交接驳不方便，预计 09:20 在东入口集合。若下雨，路线仍需重新确认。",
    ["周末", "已改路线"],
  ],
  [
    "晨间咖啡改为低因",
    "本月在家冲煮改用低因豆，通常一次用 15 克。这个选择只适用于家中的晨间咖啡，不自动推广到外出点单。",
    ["生活偏好"],
  ],
  [
    "和陈宁的复盘时间仍待确认",
    "一段讨论写周二 15:00，另一段后来提到周三下午，但还没有明确最终时间。\n\n两条证据存在冲突；在确认前不要把任何一个时间当作已约定。",
    ["时间冲突", "待确认"],
  ],
  [
    "十月读书会选定《人类简史》",
    "十月读书会读《人类简史》前两部分。分享时各准备一个不同意作者的论点，并带上对应章节。",
    ["阅读", "读书会"],
  ],
  [
    "旧计划：秋季报告仍在撰写",
    "9 月 28 日计划在两周内完成报告正文。10 月 7 日已有明确交付记录，这条旧计划不应继续触发待办或被当作当前状态。",
    ["历史计划", "已被更新"],
  ],
] as const;
const memories: Memory[] = entries.map(([title, body, labels], i) => ({
  id: `mem_demo_${i + 1}`,
  content: `${title}\n\n${body}`,
  revision: i === 0 ? 3 : 1,
  status: i === 6 ? "tentative" : i === 8 ? "superseded" : "active",
  evidence: i === 6 ? "observed" : "user_explicit",
  protected: i === 0,
  hidden: false,
  can_restore: false,
  source_refs: [`evt_demo_${i + 1}`, `evt_demo_${i + 11}`],
  updated_at: `2026-10-0${8 - (i % 5)}T08:32:00Z`,
  recorded_at: "2026-09-18T09:00:00Z",
  authority: "user",
  scope: "personal",
  labels: [...labels],
}));
const sources: Conversation[] = [
  ["秋季报告：交付与勘误", "chatgpt", 42],
  ["周五德语课的改期", "deepseek", 28],
  ["Atlas v0.8 的交付范围", "chatgpt", 67],
  ["经营报告的图表口径", "claude", 16],
  ["西山步道与集合地点", "deepseek", 24],
  ["本月的咖啡豆选择", "chatgpt", 52],
  ["陈宁复盘的两种时间", "deepseek", 19],
  ["十月读书会安排", "chatgpt", 36],
].map(([title, platform, count], i) => ({
  session_ref: `source_demo_${i + 1}`,
  title: String(title),
  platform: String(platform),
  source_url: null,
  message_count: Number(count),
  captured_at: now,
  last_occurred_at: `2026-10-0${7 - (i % 5)}T14:25:00Z`,
  coverage: "partial",
}));
const events = new Map<string, EventRecord>();
memories.forEach((memory, i) =>
  memory.source_refs.forEach((id, k) =>
    events.set(id, {
      id,
      content:
        k === 0
          ? `这段是用于界面验证的合成原话。\n\n${memory.content.split("\n\n").slice(1).join("\n\n")}`
          : i === 0
            ? "报告还在写，我计划在两周内完成正文。这是 9 月 28 日的计划，尚未包含后来已经交付的信息。"
            : i === 6
              ? "周三下午是否更方便？我还要再和陈宁确认，先不要记成已确定的安排。"
              : `这是较早的合成讨论：我们正在核对「${memory.content.split("\n")[0]}」的细节，尚待后续消息确定。`,
      role: "user",
      occurred_at: k === 0 ? "2026-10-07T14:25:00Z" : "2026-09-28T09:18:00Z",
      captured_at: now,
      source: {
        platform: k === 0 ? "chatgpt" : "deepseek",
        conversation_id: `demo_${i}`,
        message_id: id,
      },
      metadata: {
        conversation_title:
          k === 0 ? sources[i % sources.length].title : "较早的安排与计划",
      },
    }),
  ),
);
const completeJob: JobStatus = {
  schema: "recallcard.application-job/1",
  job_id: "job_demo_complete",
  request_id: "request_demo_complete",
  kind: "import",
  scope: "personal",
  state: "completed",
  phase: "finished",
  progress: {
    source_bytes_read: 28744901,
    expanded_bytes_read: 38890423,
    files_processed: 1,
    files_total: 1,
    conversations: 8,
    events_staged: 344,
    events_total: 344,
    events_processed: 344,
    events_added: 284,
    events_duplicates: 60,
  },
  error: null,
  can_resume: false,
  created_at: "2026-10-08T08:10:00Z",
  updated_at: "2026-10-08T08:11:00Z",
};
const mode = new URLSearchParams(location.search).get("state") || "normal";
const connectionDemo = createConnectionDemo(mode);
let modelRetryJob: MemoryJob | null = mode.startsWith("memory-") ? {
  schema:"recallcard.application-job/1",job_id:"memory_synthetic_retry",request_id:"synthetic_retry",kind:"memory",scope:"personal",state:"needs_input",phase:"executing",can_resume:true,created_at:now,updated_at:now,
  progress:{sources_selected:2,memories_read:0,sources_committed:0,memories_committed:0,sources_skipped:0,provider_calls:1,reserved_tokens:2000,input_tokens:null,output_tokens:null,source_cursor:null,receipt_id:null},
  error:{code:mode==="memory-conflict"?"conflict":"model_unavailable",message:mode==="memory-conflict"?"候选含有需要人工判断的证据冲突":"模型未返回可用的完整结果",action:mode==="memory-conflict"?"先核对来源和当前记忆":"检查模型连接后重试",retryable:mode!=="memory-conflict",committed_events:0,file_name:null,member:null},
}:null;
let imported = !["first-use", "empty"].includes(mode);
let jobs: JobStatus[] = imported ? [completeJob] : [];
let notePreview: {
  preview_id: string;
  content: string;
  redacted: boolean;
} | null = null;
const notes: {
  ref: string;
  text: string;
  conversation_ref: string;
  conversation_title: string;
  kind: string;
  occurred_at: string;
}[] = [];
if (mode === "failed")
  jobs = [
    {
      ...completeJob,
      job_id: "job_demo_failed",
      state: "failed",
      phase: "parsing",
      can_resume: true,
      progress: {
        ...completeJob.progress,
        events_total: null,
        events_processed: 0,
        events_added: 0,
        events_duplicates: 0,
        files_processed: 0,
      },
      error: {
        code: "invalid_json",
        message: "导出包中的会话文件未能完整解析",
        action:
          "重新下载完整的官方导出包，再添加来源。当前错误出现在会话边界附近。",
        file_name: "chat-history-demo.zip",
        member: "conversations.json",
        retryable: true,
        committed_events: 0,
      },
    },
    completeJob,
  ];
if (mode === "running")
  jobs = [
    {
      ...completeJob,
      job_id: "job_demo_running",
      state: "running",
      phase: "parsing",
      can_resume: false,
      progress: {
        ...completeJob.progress,
        events_total: null,
        events_processed: 0,
        events_added: 0,
        events_duplicates: 0,
        files_processed: 0,
      },
    },
  ];
let pending: MemoryReview | null = null;
const calls: { command: string; args: Record<string, unknown> }[] = [];
Object.assign(window, {
  __DEMO_CALLS__: calls,
  __DEMO_COMPLETE_JOBS__: () => {
    jobs = jobs.map((job) => ({
      ...job,
      state: "completed",
      phase: "finished",
      can_resume: false,
    }));
  },
});
const transport: Transport = {
  async invoke<T>(command: string, args = {}): Promise<T> {
    calls.push({ command, args });
    await new Promise((resolve) =>
      setTimeout(resolve, (command === "apply_connection_setup" && mode.startsWith("slow-") || command === "connection_approve_pairing" && mode === "slow-pairing") ? 1500 : command === "import_sources" ? 350 : 55),
    );
    const value = await invoke(command, args);
    return structuredClone(value) as T;
  },
};
async function invoke(
  command: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  const connection = await connectionDemo(command, args);
  if (connection.handled) return connection.value;
  const rows =
    !imported || ["first-use", "imported"].includes(mode) ? [] : memories;
  const scope = String(args.scope || "personal");
  if (command === "build_info") return __BUILD_INFO__;
  if (command === "restore_workspace" && mode === "first-use") return null;
  if (command === "open_default_workspace")
    return {
      session_id: "demo_session",
      root: "/合成示例/RecallCard",
      display_name: "合成示例",
      scopes: ["personal"],
      event_count: 0,
      memory_count: 0,
      health: {},
    };
  if (command === "restore_workspace")
    return {
      vault: {
        session_id: "demo_session",
        root: "/合成示例/RecallCard",
        display_name: "合成示例",
        scopes: ["personal", "project:atlas"],
        event_count: 284,
        memory_count: memories.length,
        health: {},
      },
      scope: "personal",
    };
  if (
    command === "remember_workspace" ||
    command === "cancel_previews" ||
    command === "write_clipboard"
  )
    return null;
  if (command === "choose_vault") return null;
  if (command === "manage_memories_filtered") {
    const filtered = rows
      .filter((row) =>
        args.filter === "hidden"
          ? row.hidden || !["active", "tentative"].includes(row.status)
          : args.filter === "tentative"
            ? !row.hidden && row.status === "tentative"
            : !row.hidden && ["active", "tentative"].includes(row.status),
      )
      .filter((_, index) => scope === "personal" || index === 3);
    return { memories: filtered, total: filtered.length, next_offset: null };
  }
  if (command === "managed_memory_view") {
    const memory = memories.find((row) => row.id === args.id)!;
    return { memory, hidden: memory.hidden, can_restore: memory.can_restore };
  }
  if (command === "managed_memory_source")
    return events.get(String(args.eventId));
  if (command === "list_conversations")
    return {
      conversations: imported ? sources : [],
      total: imported ? sources.length : 0,
      next_offset: null,
      note: "合成示例，不代表真实资料",
    };
  if (command === "preview_note") {
    notePreview = {
      preview_id: "demo_note_preview",
      content: String(args.content),
      redacted: false,
    };
    return notePreview;
  }
  if (command === "confirm_note") {
    if (!notePreview || args.previewId !== notePreview.preview_id)
      throw new Error("笔记预览已失效");
    notes.push({
      ref: `event:demo_note_${notes.length}`,
      text: notePreview.content,
      conversation_ref: "demo_notes",
      conversation_title: "你补充的近况",
      kind: "event",
      occurred_at: now,
    });
    notePreview = null;
    return { ref: notes.at(-1)!.ref };
  }
  if (command === "chatgpt_connection_plan")
    return {
      local_readiness: "permission_required",
      upstream_verification: "not_checked",
      official_connect_url: "https://chatgpt.com/plugins",
      official_tunnel_url:
        "https://platform.openai.com/settings/organization/tunnels",
      last_local_read_at: null,
      last_local_bootstrap_at: null,
    };
  if (command === "event_location") {
    if (mode === "location-error")
      throw new Error("原话暂时无法定位，请重新查找");
    const match = /^event:source_demo_(\d+)_message_(\d+)$/.exec(
      String(args.reference),
    );
    if (!match) throw new Error("合成原话不存在");
    return {
      conversation_ref: `source_demo_${match[1]}`,
      offset: Number(match[2]),
    };
  }
  if (command === "conversation_messages") {
    const source = sources.find(
      (row) => row.session_ref === args.conversationRef,
    )!;
    const offset = Number(args.offset || 0);
    const messages = Array.from(
      { length: Math.min(4, source.message_count - offset) },
      (_, i) => ({
        ref: `event:${source.session_ref}_message_${offset + i}`,
        role: i % 2 === 0 ? "user" : "assistant",
        text:
          i % 2 === 0
            ? "报告第三版已经发给评审组，正文撰写可以标为完成。\n\n只留下一个勘误：图 4 的纵轴单位改为万元。暂时不增加新附录。"
            : "收到。后续范围是修正图 4 的单位，正文交付已经完成；9 月 28 日仍在撰写的计划属于历史状态。",
        assets:
          mode === "assets" && i === 0
            ? {
                files: [
                  {
                    source_file_id: "demo_file",
                    name: "autumn-report-v3.pdf",
                    byte_count: 182400,
                    payload_status: "not_in_export",
                  },
                ],
                citations: [
                  { url: "https://reference.example/report-method", title: "" },
                ],
                tool_trace: [
                  { type: "web_search", payload_status: "not_in_export" },
                ],
                files_total: 1,
                citations_total: 1,
                trace_total: 1,
                unsupported_total: 1,
                truncated: false,
              }
            : null,
        occurred_at: i === 3 ? null : "2026-10-07T14:25:00Z",
        captured_at: now,
        text_truncated: false,
        source: { platform: source.platform },
        branch: {
          parent_ref: i
            ? `event:${source.session_ref}_message_${offset + i - 1}`
            : null,
          on_current_path: true,
          is_branch_end: false,
          relationship_known: true,
          child_count: 1,
          gap_before: false,
        },
      }),
    );
    const sourceIndex = sources.indexOf(source);
    // 来源列表与记忆列表并非相同顺序，显式对应每段合成原话。
    const entryBySource = [0, 1, 3, 2, 4, 5, 6, 7];
    if (sourceIndex > 0)
      messages.forEach((message) => {
        message.text = entries[entryBySource[sourceIndex]][1];
      });
    if (mode === "assets" && messages[0]) messages[0].text = "";
    return {
      ...source,
      messages,
      total: source.message_count,
      next_offset:
        offset + messages.length < source.message_count
          ? offset + messages.length
          : null,
      offset,
      order_known: true,
      order_kind: "single_branch",
      coverage: "合成示例",
    };
  }
  if (command === "application_import_result")
    return {
      job: completeJob,
      coverage: {
        expanded_bytes: 38890423,
        archive_entries: 12,
        conversations: 8,
        events: 344,
        ignored_values: 0,
        ignored_files: 11,
        hidden_fragments: 3,
        unsupported_fragments: 1,
        omitted_messages: 0,
        file_references: 3,
        citations: 5,
        trace_placeholders: 2,
      },
    };
  if (command === "application_jobs") {
    if (mode === "unavailable")
      throw {
        code: "storage",
        message: "本地任务状态暂不可读取",
        action: "重新打开空间后重试",
        retryable: true,
        committed_events: 0,
      };
    return jobs;
  }
  if (command === "import_sources") {
    if (mode === "cancel") return null;
    if (mode === "pick-error")
      throw {
        code: "permission_denied",
        message: "没有读取所选文件的权限",
        action: "请重新选择可读取的本机文件",
        file_name: "导出示例.zip",
        retryable: false,
        committed_events: 0,
      };
    if (mode === "first-use") {
      imported = true;
      jobs = [completeJob];
      return completeJob;
    }
    const job = {
      ...completeJob,
      job_id: "job_demo_new",
      request_id: args.requestId,
      state: "running",
      phase: "parsing",
      progress: {
        ...completeJob.progress,
        events_total: null,
        events_processed: 0,
        events_added: 0,
        events_duplicates: 0,
        files_processed: 0,
      },
    } as JobStatus;
    jobs = [job, ...jobs.filter((item) => item.job_id !== job.job_id)];
    return job;
  }
  if (
    command === "application_job_pause" ||
    command === "application_job_resume"
  ) {
    const job = jobs.find((item) => item.job_id === args.jobId)!;
    job.state = command.endsWith("pause") ? "paused" : "running";
    job.can_resume = job.state === "paused";
    job.error = null;
    return job;
  }
  if (command === "search_records")
    return {
      results: [
        ...notes.filter((note) => note.text.includes(String(args.query))),
        ...rows
          .filter(
            (row) => !row.hidden && row.content.includes(String(args.query)),
          )
          .map((row) => ({
            ref: `memory:${row.id}`,
            kind: "memory",
            text: row.content,
            state: row.status,
            evidence: row.evidence,
            occurred_at: row.updated_at,
          })),
      ],
      truncated: false,
    };
  if (
    command === "review_memory_edit" ||
    command === "review_memory_visibility"
  ) {
    const memory = memories.find((row) => row.id === args.id)!;
    pending = {
      preview_id: "review_demo",
      operation:
        command === "review_memory_edit"
          ? "edit"
          : args.restore
            ? "restore"
            : "forget",
      before: memory,
      after:
        command === "review_memory_edit"
          ? (args.edit as MemoryReview["after"])
          : null,
      affected_events: command === "review_memory_edit" ? 0 : 2,
      affected_memories: 1,
      requires_protected_approval: memory.protected,
      warning:
        command === "review_memory_edit"
          ? "保存新的记忆版本，原始对话保持不变。"
          : "相关记忆与原始来源将退出检索和后续整理；文件不删除，可检查后恢复。",
    };
    return pending;
  }
  if (command === "confirm_memory_change") {
    if (!pending) throw new Error("确认已过期");
    if (pending.requires_protected_approval && !args.approveProtected)
      throw new Error("请确认受保护内容的修改");
    const index = memories.findIndex((row) => row.id === pending!.before.id);
    if (pending.after)
      memories[index] = {
        ...memories[index],
        ...pending.after,
        revision: memories[index].revision + 1,
      };
    else {
      memories[index].hidden = pending.operation === "forget";
      memories[index].can_restore = pending.operation === "forget";
    }
    pending = null;
    return {};
  }
  if (command === "prepare_client_config")
    return {
      mcpServers: {
        recallcard: {
          command: "/合成示例/recallcard",
          args: ["--vault", "/合成示例/RecallCard", "mcp", "--scope", scope],
        },
      },
    };
  if (command === "install_browser_connection")
    return {
      registered: true,
      registration: "/合成示例/native-host.json",
      capture_enabled: args.allowCapture,
      note: "示例注册完成，尚无浏览器调用回执。",
    };
  if (command === "inspect_model_credential") return {present:false,storage:"unavailable",lifetime:"not_configured",os_protected_available:false,message:"合成测试不访问系统密钥"};
  if (command === "configure_memory_model") {
    demoModelConfig = structuredClone(args.config as MemoryConfig);
    return invoke("model_setup_status", args);
  }
  if (command === "model_setup_status") return {
    runtime: await invoke("memory_runtime_status", args),
    credential: { present: false, storage: "unavailable", lifetime: "not_configured", os_protected_available: false, message: "合成界面未提供任何模型密钥" },
    service: null,
    service_error: null,
  };
  if (command === "memory_job_control" && modelRetryJob) {
    if (mode === "memory-retry-error") throw new Error("本地任务状态暂不可读取，未确认重试结果");
    if (args.jobId !== modelRetryJob.job_id || args.action !== "retry" || !modelRetryJob.error?.retryable) throw new Error("合成候选不支持此操作");
    modelRetryJob={...modelRetryJob,state:"queued",error:null,can_resume:false};
    return modelRetryJob;
  }
  if (command === "memory_runtime_status")
    return {
      schema: "recallcard.memory-runtime/1",
      state: "unconfigured",
      message: "后台整理尚未配置；已保存来源仍可搜索",
      config: demoModelConfig || {
        schema: "recallcard.memory-runtime/1",
        enabled: false,
        paused: false,
        scope,
        provider: null,
        consent: null,
        budget: {
          max_calls_per_month: 100,
          max_reserved_tokens_per_month: 1000000,
          max_output_tokens_per_call: 4096,
          max_request_bytes_per_call: 1048576,
        },
        quiet_seconds: 30,
        batch_size: 16,
        max_projection_bytes: 262144,
      },
      usage: {
        month: "2026-10",
        reserved_calls: 0,
        reserved_tokens: 0,
        reported_input_tokens: 0,
        reported_output_tokens: 0,
        calls_with_unknown_usage: 0,
      },
      jobs: modelRetryJob ? [modelRetryJob] : [],
      raw_search_available: true,
      budget_note: "token 预留是资源上限，不是准确费用",
    };
  throw new Error(`合成入口未实现命令：${command}`);
}
createRoot(document.getElementById("root")!).render(
  <App service={new RecallService(transport, true)} />,
);

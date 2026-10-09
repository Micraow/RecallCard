import assert from "node:assert/strict";
import test from "node:test";
import {
  appError,
  isRunning,
  validateJobs,
  importRecovery,
  coverageNotes,
} from "../src/service/contracts.ts";
import { validModelTarget } from "../src/service/setup.ts";
import {
  dateLabel,
  eventText,
  evidenceLabel,
  scopeLabel,
} from "../src/service/types.ts";

test("应用错误保留原因、来源成员、恢复动作和提交边界", () => {
  const error = appError({
    code: "invalid_json",
    message: "会话不完整",
    action: "重新下载",
    file_name: "synthetic.zip",
    member: "conversations.json",
    retryable: true,
    committed_events: 32,
  });
  assert.equal(error.code, "invalid_json");
  assert.equal(error.member, "conversations.json");
  assert.equal(error.committed_events, 32);
  assert.equal(error.action, "重新下载");
});
test("旧服务字符串错误不被泛化成没有原因的通知", () => {
  assert.equal(appError("当前资料范围已失效").message, "当前资料范围已失效");
  assert.equal(appError(new Error("需要重新读取")).message, "需要重新读取");
});
test("不兼容任务合同明确失败，不当作零任务", () => {
  assert.throws(() => validateJobs({ jobs: [] }), /格式不受支持/);
  assert.throws(
    () =>
      validateJobs([
        {
          schema: "old",
          job_id: "synthetic",
          state: "completed",
          progress: {},
        },
      ]),
    /合同版本不一致/,
  );
  assert.deepEqual(validateJobs([]), []);
});
test("未知任务状态不伪造完成", () => {
  assert.throws(
    () =>
      validateJobs([
        {
          schema: "recallcard.application-job/1",
          job_id: "synthetic",
          state: "interrupted",
          progress: {},
        },
      ]),
    /合同版本不一致/,
  );
});
test("暂停与需要输入不是后台正在运行", () => {
  assert.equal(isRunning({ state: "paused" } as any), false);
  assert.equal(isRunning({ state: "needs_input" } as any), false);
  assert.equal(isRunning({ state: "queued" } as any), true);
});
test("未知时间不使用当前导入时间补写", () => {
  assert.equal(dateLabel(null), "时间未知");
  assert.equal(dateLabel("invalid"), "时间未知");
});
test("来源正文按内容片段读取而不执行 HTML", () => {
  assert.equal(
    eventText({
      content: "<script>unsafe</script>",
      parts: [{ text: "<script>raw</script>" }, { text: "正文" }],
    } as any),
    "<script>raw</script>\n正文",
  );
});
test("观察和 AI 建议不标为明确事实，个人和项目标签分开", () => {
  assert.equal(evidenceLabel("assistant_suggestion"), "AI 建议");
  assert.equal(evidenceLabel("observed"), "观察所得");
  assert.equal(scopeLabel("personal"), "个人空间");
  assert.equal(scopeLabel("project:atlas"), "atlas");
});

test("损坏来源必须重新选择，暂停任务才可以继续", () => {
  assert.equal(
    importRecovery({
      can_resume: true,
      error: { code: "invalid_json" },
    } as any),
    "new_source",
  );
  assert.equal(
    importRecovery({
      can_resume: true,
      error: { code: "source_changed" },
    } as any),
    "new_source",
  );
  assert.equal(
    importRecovery({ can_resume: true, error: null } as any),
    "resume",
  );
  assert.equal(
    importRecovery({ can_resume: false, error: null } as any),
    "none",
  );
});

test("导入覆盖明确文件元数据和缺失正文，不声称附件原件已保存", () => {
  const notes = coverageNotes({
    file_references: 3,
    trace_placeholders: 2,
    hidden_fragments: 4,
  } as any);
  assert.ok(notes.some((note) => note.includes("导出未包含文件原件")));
  assert.ok(notes.some((note) => note.includes("没有工具输出正文")));
  assert.ok(notes.some((note) => note.includes("隐藏推理片段未收集")));
});

test("模型目的地只接受不夹带凭据的明确 HTTPS 地址", () => {
  assert.equal(
    validModelTarget("https://model.example/v1/chat/completions", "model-a"),
    true,
  );
  assert.equal(
    validModelTarget("http://model.example/v1/chat/completions", "model-a"),
    false,
  );
  assert.equal(
    validModelTarget(
      "https://user:secret@model.example/v1/chat/completions",
      "model-a",
    ),
    false,
  );
  assert.equal(
    validModelTarget(
      "https://model.example/v1/chat/completions?key=secret",
      "model-a",
    ),
    false,
  );
  assert.equal(validModelTarget("https://model.example/", "model-a"), false);
  assert.equal(
    validModelTarget("https://model.example/v1/chat/completions", ""),
    false,
  );
});


test("模型等待时间单独校验连接、读取与整个请求，不改变token预算", async () => {
  const {validMemoryTimeouts} = await import("../src/service/runtime.ts");
  assert.equal(validMemoryTimeouts({connect_seconds:15,read_seconds:300,operation_seconds:600}),true);
  for(const value of [{connect_seconds:121,read_seconds:300,operation_seconds:600},{connect_seconds:15,read_seconds:301,operation_seconds:300},{connect_seconds:0,read_seconds:30,operation_seconds:30},{connect_seconds:30,read_seconds:30,operation_seconds:1801}]) assert.equal(validMemoryTimeouts(value),false);
});

import { canRetryMemoryJob } from "../src/service/runtime.ts";
test("仅可恢复的供应商错误显示重试，不把证据冲突当网络失败", () => {
  const job:any={state:"needs_input",can_resume:true,error:{retryable:true}};
  assert.equal(canRetryMemoryJob(job),true);
  assert.equal(canRetryMemoryJob({...job,state:"failed"}),true);
  for(const state of ["running","queued","completed","cancelled","paused"])
    assert.equal(canRetryMemoryJob({...job,state}),false);
  assert.equal(canRetryMemoryJob({...job,error:{retryable:false}}),false);
  assert.equal(canRetryMemoryJob({...job,error:null}),false);
  assert.equal(canRetryMemoryJob({...job,can_resume:false}),false);
});

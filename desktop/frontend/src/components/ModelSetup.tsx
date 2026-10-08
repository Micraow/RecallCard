import { useRef, useState } from "react";
import { appError, type AppError } from "../service/contracts";
import { scopeLabel } from "../service/types";
import { validMemoryTimeouts, type MemoryConfig } from "../service/runtime";
import {
  validModelTarget,
  type ModelSetupRequest,
  type ModelSetupSnapshot,
} from "../service/setup";
import { Badge, Dialog, ErrorNotice } from "./common";
import { Icon } from "./Icon";

/** 此表单不持久化密钥；提交、关闭或卸载后不保留密码输入。 */
export function ModelSetupDialog({
  initial,
  scope,
  inspect,
  submit,
  close,
}: {
  initial: ModelSetupSnapshot;
  scope: string;
  inspect: (target: {
    endpoint: string;
    model: string;
  }) => Promise<ModelSetupSnapshot>;
  submit: (request: ModelSetupRequest) => Promise<ModelSetupSnapshot>;
  close: () => void;
}) {
  const defaults = initial.runtime.config;
  const [endpoint, setEndpoint] = useState(defaults.provider?.endpoint || "");
  const [model, setModel] = useState(defaults.provider?.model || "");
  const [apiKey, setApiKey] = useState("");
  const [calls, setCalls] = useState(
    String(defaults.budget.max_calls_per_month),
  );
  const [tokens, setTokens] = useState(
    String(defaults.budget.max_reserved_tokens_per_month),
  );
  const [output, setOutput] = useState(
    String(defaults.budget.max_output_tokens_per_call),
  );
  const [connectSeconds, setConnectSeconds] = useState(String(defaults.timeouts?.connect_seconds ?? 30));
  const [readSeconds, setReadSeconds] = useState(String(defaults.timeouts?.read_seconds ?? 30));
  const [operationSeconds, setOperationSeconds] = useState(String(defaults.timeouts?.operation_seconds ?? 30));
  const timeouts = {connect_seconds: Number(connectSeconds), read_seconds: Number(readSeconds), operation_seconds: Number(operationSeconds)};
  const [step, setStep] = useState<"input" | "review">("input");
  const [capability, setCapability] = useState<ModelSetupSnapshot | null>(null);
  const [remember, setRemember] = useState(false);
  const [sendApproved, setSendApproved] = useState(false);
  const [applyApproved, setApplyApproved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const submitLock = useRef(false);
  const [error, setError] = useState<AppError | null>(null);
  const validBudget = validMemoryTimeouts(timeouts) &&
    Number.isSafeInteger(Number(calls)) &&
    Number(calls) >= 1 &&
    Number(calls) <= 1_000_000 &&
    Number.isSafeInteger(Number(tokens)) &&
    Number(tokens) > 0 &&
    Number.isSafeInteger(Number(output)) &&
    Number(output) >= 1 &&
    Number(output) <= 131072;
  const validTarget = validModelTarget(endpoint.trim(), model.trim());
  const dismiss = () => {
    setApiKey("");
    close();
  };
  const review = async () => {
    if (!validTarget || !validBudget || busy) return;
    setBusy(true);
    setError(null);
    setRemember(false);
    setSendApproved(false);
    setApplyApproved(false);
    try {
      const status = await inspect({
        endpoint: endpoint.trim(),
        model: model.trim(),
      });
      setCapability(status);
      if (
        !apiKey &&
        !(
          status.credential.present &&
          status.credential.storage === "os_protected"
        )
      ) {
        setError(
          appError({
            code: "credential_required",
            message: "请填写该模型服务的 API 密钥",
            action:
              "未发现这个目标可读取的系统凭据。会话密钥不会从运行服务中读回。",
          }),
        );
      } else setStep("review");
    } catch (reason) {
      setError(appError(reason));
    } finally {
      setBusy(false);
    }
  };
  const confirm = async () => {
    if (!capability || !sendApproved || !applyApproved || submitLock.current) return;
    submitLock.current = true; setBusy(true); setSubmitting(true); setError(null);
    const provider = { endpoint: endpoint.trim(), model: model.trim() };
    const config: MemoryConfig = {
      ...defaults,
      schema: "recallcard.memory-runtime/1",
      enabled: true,
      paused: false,
      scope,
      provider,
      timeouts,
      consent: {
        ...provider,
        scope,
        send_source_snapshots: true,
        send_memory_snapshots: true,
        auto_apply: true,
        accepted_at: new Date().toISOString(),
      },
      budget: {
        ...defaults.budget,
        max_calls_per_month: Number(calls),
        max_reserved_tokens_per_month: Number(tokens),
        max_output_tokens_per_call: Number(output),
      },
    };
    const key = apiKey || null;
    setApiKey("");
    try { await submit({
      config,
      apiKey: key,
      credentialStorage:
        (!key && capability.credential.storage === "os_protected") ||
        (remember && capability.credential.os_protected_available === true)
          ? "os_protected"
          : "session_only",
    }); } catch (reason) { setError(appError(reason)); setStep("input"); setSendApproved(false); setApplyApproved(false); } finally { submitLock.current = false; setBusy(false); setSubmitting(false); }
  };
  return (
    <Dialog
      title={step === "input" ? "连接整理模型" : "确认模型使用范围"}
      close={dismiss}
      busy={submitting}
      wide
    >
      <div className="modal-body model-setup">
        {error && <ErrorNotice error={error} />}
        {submitting && <p className="context-note" role="status">正在等待当前请求结束并更新本机服务。你可以收起继续等待；这不会取消或重复提交。</p>}
        {step === "input" ? (
          <>
            <p>
              连接你自己的模型服务。来源先保存在本机；完成下一步授权后，才会启用后台整理。
            </p>
            <div className="scope-chip">
              <Icon name="shield" size={15} />
              整理范围：{scopeLabel(scope)}
            </div>
            <label className="field-label" htmlFor="model-endpoint">
              模型服务地址（HTTPS）
            </label>
            <input
              id="model-endpoint"
              className="field-input"
              type="url"
              autoComplete="off"
              spellCheck={false}
              value={endpoint}
              onChange={(event) => setEndpoint(event.target.value)}
              placeholder="https://你的模型服务/v1/chat/completions"
            />
            <label className="field-label" htmlFor="model-name">
              模型名称
            </label>
            <input
              id="model-name"
              className="field-input"
              autoComplete="off"
              spellCheck={false}
              maxLength={256}
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder="供应商提供的模型名称"
            />
            <label className="field-label" htmlFor="model-api-key">
              API 密钥
            </label>
            <input
              id="model-api-key"
              className="field-input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              maxLength={8192}
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder="仅传给本机凭据接口，不写入资料文件"
            />
            <p className="field-hint">
              默认仅当前后台服务使用。系统安全存储确认可用时，下一步可以选择安全保存。
            </p>
            <details>
              <summary>连接与等待时间</summary>
              <div className="budget-fields">
                <label className="field-label">连接等待（秒）<input aria-label="连接等待秒数" className="field-input" type="number" min="1" max="120" value={connectSeconds} onChange={e=>setConnectSeconds(e.target.value)} /></label>
                <label className="field-label">单次读取等待（秒）<input aria-label="单次读取等待秒数" className="field-input" type="number" min="1" max="1800" value={readSeconds} onChange={e=>setReadSeconds(e.target.value)} /></label>
                <label className="field-label">整个请求时限（秒）<input aria-label="整个请求时限秒数" className="field-input" type="number" min="1" max="1800" value={operationSeconds} onChange={e=>setOperationSeconds(e.target.value)} /></label>
              </div>
              <p className="field-hint">慢模型可延长等待，单步不得超过整个请求。暂停或取消会终止本机等待；已发出的请求仍可能计费。</p>
            </details>
            <div className="budget-fields">
              <div>
                <label className="field-label" htmlFor="model-month-calls">
                  每月最多请求
                </label>
                <input
                  id="model-month-calls"
                  className="field-input"
                  type="number"
                  min="1"
                  max="1000000"
                  value={calls}
                  onChange={(event) => setCalls(event.target.value)}
                />
              </div>
              <div>
                <label className="field-label" htmlFor="model-month-tokens">
                  每月预留 tokens
                </label>
                <input
                  id="model-month-tokens"
                  className="field-input"
                  type="number"
                  min="1"
                  value={tokens}
                  onChange={(event) => setTokens(event.target.value)}
                />
              </div>
              <div>
                <label className="field-label" htmlFor="model-call-output">
                  单次输出上限
                </label>
                <input
                  id="model-call-output"
                  className="field-input"
                  type="number"
                  min="1"
                  max="131072"
                  value={output}
                  onChange={(event) => setOutput(event.target.value)}
                />
              </div>
            </div>
            <p className="field-hint">
              这是资源上限，不是货币费用保证。实际收费以供应商账单为准。
            </p>
          </>
        ) : (
          capability && (
            <>
              <div className="model-destination">
                <Icon name="globe" size={21} />
                <div>
                  <strong>{model.trim()}</strong>
                  <span>{endpoint.trim()}</span>
                </div>
                <Badge>{scopeLabel(scope)}</Badge>
              </div>
              <h3>将发送什么</h3>
              <p>
                后台会将此范围内选中来源的完整文本、相关旧记忆及出处信息发送给上面的模型服务，用于提取和更新记忆。原始文件继续保存在本机。
              </p>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={sendApproved}
                  onChange={(event) => setSendApproved(event.target.checked)}
                />
                允许向这个明确的服务地址发送上述范围和资料
              </label>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={applyApproved}
                  onChange={(event) => setApplyApproved(event.target.checked)}
                />
                允许自动应用可回退的普通记忆更新；冲突、受保护内容和人工维护内容留待处理
              </label>
              <div className="credential-choice">
                <h3>密钥保留方式</h3>
                {capability.credential.os_protected_available === true &&
                apiKey ? (
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={remember}
                      onChange={(event) => setRemember(event.target.checked)}
                    />
                    将此密钥安全保存在操作系统凭据存储中
                  </label>
                ) : !apiKey &&
                  capability.credential.storage === "os_protected" ? (
                  <p>使用当前目标已保存的系统凭据，不读取或回显密钥。</p>
                ) : (
                  <p>
                    本次尚不能使用系统安全存储，密钥仅供当前后台服务会话使用。
                  </p>
                )}
                {apiKey && !remember && (
                  <p className="field-hint">
                    仅关闭窗口不会清除会话密钥；停止或重新启动该后台服务后，会话密钥失效。
                  </p>
                )}
              </div>
              <div className="model-budget-review">
                每月最多 {Number(calls).toLocaleString()} 次请求 · 预留{" "}
                {Number(tokens).toLocaleString()} tokens · 单次输出最多{" "}
                {Number(output).toLocaleString()} tokens · 连接 {connectSeconds} 秒 / 读取 {readSeconds} 秒 / 整个请求 {operationSeconds} 秒
              </div>
              <p className="field-hint">
                当前版本同一资料库只支持一个活动整理范围，保存会替换其他范围的整理设置。配置和凭据可用，不代表模型已经成功响应。
              </p>
            </>
          )
        )}
      </div>
      <div className="modal-footer">
        <button
          className="button"
          onClick={
            submitting ? dismiss : step === "review"
              ? () => {
                  setStep("input");
                  setSendApproved(false);
                  setApplyApproved(false);
                }
              : dismiss
          }
        >
          {submitting ? "收起，继续等待" : step === "review" ? "返回修改" : "取消"}
        </button>
        {step === "input" ? (
          <button
            className="button primary"
            disabled={busy || !validTarget || !validBudget}
            onClick={() => void review()}
          >
            {busy ? "正在检查本机能力…" : "检查并继续"}
          </button>
        ) : (
          <button
            className="button primary"
            disabled={busy || !sendApproved || !applyApproved}
            onClick={() => void confirm()}
          >
            {submitting ? "正在更新服务…" : "保存授权并启动服务"}
          </button>
        )}
      </div>
    </Dialog>
  );
}

import { useEffect, useState } from 'react';
import {
  api,
  evidenceLabel,
  formatTime,
  stateLabel,
  type DreamPreview,
  type MemoryRow,
  type RecordRow,
} from '../api';
import { Markdown, plainPreview } from '../components/Markdown';
import { message } from '../App';

interface Props {
  scope: string;
  onError: (text: string) => void;
  onChanged: () => void;
}

const RANGES = [
  { value: '7', label: '最近 7 天' },
  { value: '30', label: '最近 30 天' },
  { value: 'all', label: '全部未整理' },
];

export function MemoriesPage({ scope, onError, onChanged }: Props) {
  const [rows, setRows] = useState<MemoryRow[]>([]);
  const [total, setTotal] = useState(0);
  const [includeHidden, setIncludeHidden] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // 整理：一次点击跑完选择来源与生成，但仍要人逐条看差异再写入。
  const [open, setOpen] = useState(false);
  const [range, setRange] = useState('30');
  const [candidates, setCandidates] = useState<number | null>(null);
  const [samples, setSamples] = useState<RecordRow[]>([]);
  const [llmReady, setLlmReady] = useState(true);
  const [preview, setPreview] = useState<DreamPreview | null>(null);
  const [approveProtected, setApproveProtected] = useState(false);
  const [working, setWorking] = useState<'idle' | 'counting' | 'generating' | 'applying'>('idle');
  const [notice, setNotice] = useState('');

  useEffect(() => {
    void load();
  }, [scope, includeHidden]);

  async function load() {
    setBusy(true);
    try {
      const response = await api.memories(includeHidden);
      setRows(response.memories);
      setTotal(response.total);
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  function since(ranged: string): { since: string; until: string } {
    const until = new Date().toISOString();
    if (ranged === 'all') return { since: '', until: '' };
    const days = Number(ranged);
    const start = new Date(Date.now() - days * 24 * 3600 * 1000);
    return { since: start.toISOString(), until };
  }

  const [pending, setPending] = useState(0);
  const [sourceLimit, setSourceLimit] = useState(0);

  async function count(ranged: string) {
    setWorking('counting');
    setNotice('');
    onError('');
    try {
      const window = since(ranged);
      const result = await api.dreamSources(window.since, window.until);
      setCandidates(result.count);
      setPending(result.pending);
      setSourceLimit(result.source_limit);
      setSamples(result.samples);
      setLlmReady(result.llm_ready.base_url);
      setPreview(null);
    } catch (cause) {
      onError(message(cause));
    } finally {
      setWorking('idle');
    }
  }

  async function generate() {
    setWorking('generating');
    setNotice('');
    onError('');
    try {
      const window = since(range);
      const result = await api.dreamPreview(window.since, window.until);
      setPreview(result.preview);
      setCandidates(result.source_count);
    } catch (cause) {
      onError(message(cause));
    } finally {
      setWorking('idle');
    }
  }

  async function apply() {
    if (!preview) return;
    setWorking('applying');
    onError('');
    try {
      await api.dreamApply(preview.preview_id, approveProtected);
      setNotice('已写入。原始记录仍然保留，随时可以在搜索里核对。');
      setPreview(null);
      setOpen(false);
      await load();
      onChanged();
    } catch (cause) {
      onError(message(cause));
    } finally {
      setWorking('idle');
    }
  }

  const detail = rows.find(row => row.id === selected) ?? null;

  return (
    <div className="content">
      <div className="page-head">
        <h1>记忆</h1>
        <span className="hint">
          {total.toLocaleString('zh-CN')} 条长期记忆 · 每条都能追回原始对话
        </span>
        <div className="spacer" />
        <label className="checkbox">
          <input
            type="checkbox"
            checked={includeHidden}
            onChange={event => setIncludeHidden(event.target.checked)}
          />
          显示已隐藏与失效
        </label>
        <button
          className="primary"
          onClick={() => {
            setOpen(value => !value);
            if (!open) void count(range);
          }}
        >
          {open ? '收起整理' : '整理最近的对话'}
        </button>
      </div>

      {notice && <div className="notice ok" style={{ marginBottom: 16 }}>{notice}</div>}

      {open && (
        <div className="panel" style={{ borderTop: '1px solid var(--ink)', paddingTop: 18 }}>
          <h2>把最近的对话整理成记忆</h2>
          <p className="desc">
            选一个时间范围，RecallCard 会自动挑出还没整理过的原始记录，交给设置里的模型生成候选记忆。
            生成结果不会直接生效，你在这里逐条看过再写入。
          </p>
          <div className="range-row">
            {RANGES.map(item => (
              <button
                key={item.value}
                aria-pressed={range === item.value}
                className={range === item.value ? 'primary' : ''}
                onClick={() => {
                  setRange(item.value);
                  setCandidates(null);
                  void count(item.value);
                }}
              >
                {item.label}
              </button>
            ))}
            <span className="spacer" />
            <span className="hint-line">
              {working === 'counting' ? (
                <>
                  <span className="spin" /> 正在清点
                </>
              ) : candidates === null ? (
                ''
              ) : candidates === 0 ? (
                '这个范围内没有待整理的记录'
              ) : (
                `将整理 ${candidates} 条原始记录` +
                (pending > candidates
                  ? `（共 ${pending.toLocaleString('zh-CN')} 条待整理，一次最多 ${sourceLimit} 条，其余下次再整理）`
                  : '')
              )}
            </span>
          </div>

          {!llmReady && (
            <div className="notice warn" style={{ marginBottom: 14 }}>
              还没有填写模型服务，无法自动生成。请到设置页填写地址与密钥；或者仍然导出任务贴给别的 AI。
            </div>
          )}

          {candidates === 0 && samples.length === 0 && (
            <div className="empty">
              <strong>没有可整理的记录</strong>
              <span>换一个更大的时间范围，或先记录新的对话。</span>
            </div>
          )}

          {candidates !== null && candidates > 0 && !preview && (
            <>
              <div className="row">
                <button
                  className="primary"
                  onClick={() => void generate()}
                  disabled={working !== 'idle' || !llmReady}
                >
                  {working === 'generating' ? (
                    <>
                      <span className="spin" /> 正在生成，通常需要十几秒
                    </>
                  ) : (
                    '开始整理'
                  )}
                </button>
                <span className="hint-line">
                  会先把 {candidates} 条记录整理成任务，再交给模型；生成后仍需你确认。
                </span>
              </div>
              {samples.length > 0 && (
                <div style={{ marginTop: 16 }}>
                  <div className="row-meta">将纳入的最新几条</div>
                  {samples.map(sample => (
                    <div className="source-item" key={sample.ref} style={{ marginTop: 10 }}>
                      <div className="source-head">
                        <span>{formatTime(sample.occurred_at)}</span>
                      </div>
                      <div style={{ fontSize: 13.5, lineHeight: 1.6, color: 'var(--ink-soft)' }}>
                        {plainPreview(sample.text, 180)}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}

          {preview && (
            <div style={{ marginTop: 18 }}>
              <div className="row" style={{ marginBottom: 12 }}>
                <strong style={{ fontFamily: 'var(--serif)', fontSize: 15 }}>
                  待写入的变更 · {preview.review.changes.length} 项
                </strong>
                <span className="spacer" />
                {!preview.review.can_apply && <span className="tag warn">存在冲突</span>}
                {preview.review.requires_protected_approval && (
                  <span className="tag warn">含受保护记忆</span>
                )}
              </div>
              {preview.review.changes.map((change, index) => (
                <div className="diff-item" key={index}>
                  <div className="op">
                    {change.operation === 'create' ? '新增记忆' : '修改记忆'} ·{' '}
                    {evidenceLabel[String(change.after?.evidence ?? '')] ?? ''}
                  </div>
                  {change.before && (
                    <div className="diff-before">{plainPreview(change.before.content, 240)}</div>
                  )}
                  {change.after && <div className="diff-after">{change.after.content}</div>}
                  {change.after && change.after.source_refs.length > 0 && (
                    <div className="hint-line" style={{ marginTop: 6 }}>
                      出处 {change.after.source_refs.length} 条原始记录
                    </div>
                  )}
                  {(change.diagnostics ?? []).map(text => (
                    <div className="hint-line" key={text}>
                      {text}
                    </div>
                  ))}
                </div>
              ))}
              {(preview.review.diagnostics ?? []).length > 0 && (
                <div className="notice warn" style={{ marginBottom: 12 }}>
                  {preview.review.diagnostics?.join('；')}
                </div>
              )}
              {preview.review.requires_protected_approval && (
                <label className="checkbox" style={{ marginBottom: 12 }}>
                  <input
                    type="checkbox"
                    checked={approveProtected}
                    onChange={event => setApproveProtected(event.target.checked)}
                  />
                  我确认要修改受保护的记忆
                </label>
              )}
              <div className="row">
                <button
                  className="primary"
                  onClick={() => void apply()}
                  disabled={
                    working !== 'idle' ||
                    !preview.review.can_apply ||
                    (preview.review.requires_protected_approval && !approveProtected)
                  }
                >
                  {working === 'applying' ? (
                    <>
                      <span className="spin" /> 正在写入
                    </>
                  ) : (
                    '确认写入这些记忆'
                  )}
                </button>
                <button onClick={() => setPreview(null)} disabled={working !== 'idle'}>
                  丢弃这次结果
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      <div className="search-layout" style={{ marginTop: 20 }}>
        <div className="memory-list">
          {rows.length === 0 && !busy && (
            <div className="empty">
              <strong>还没有长期记忆</strong>
              <span>点右上角「整理最近的对话」，看过的内容才会变成记忆。</span>
            </div>
          )}
          {rows.map(row => (
            <button
              key={row.id}
              className="memory-row"
              aria-pressed={selected === row.id}
              onClick={() => setSelected(selected === row.id ? null : row.id)}
            >
              <div className="row-meta">
                <span className="tag ok">{stateLabel[row.status] ?? row.status}</span>
                <span>{evidenceLabel[row.evidence] ?? row.evidence}</span>
                {row.protected && <span className="tag seal">受保护</span>}
                {row.hidden && <span className="tag warn">已隐藏</span>}
                <span>·</span>
                <span>{formatTime(row.updated_at)}</span>
              </div>
              <div className="memory-text">{plainPreview(row.content, 200)}</div>
            </button>
          ))}
        </div>

        <div className="reader">
          {!detail && (
            <div className="empty">
              <strong>选一条记忆</strong>
              <span>能在这个面板里读到全文与出处。</span>
            </div>
          )}
          {detail && (
            <>
              <div className="reader-head">
                <div className="reader-meta">
                  <span className="tag ok">{stateLabel[detail.status] ?? detail.status}</span>
                  <span>{evidenceLabel[detail.evidence] ?? detail.evidence}</span>
                  {detail.protected && <span className="tag seal">受保护</span>}
                  <span>·</span>
                  <span>{formatTime(detail.updated_at)}</span>
                </div>
                <div className="reader-title">第 {detail.revision} 版</div>
              </div>
              <Markdown text={detail.content} />
              {detail.source_refs.length > 0 && (
                <div className="source-list">
                  <div className="row-meta">依据的原始记录 · {detail.source_refs.length} 条</div>
                  {detail.source_refs.map(reference => (
                    <div className="source-item" key={reference}>
                      <div className="source-head">
                        <span style={{ fontFamily: 'var(--mono)' }}>{reference}</span>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

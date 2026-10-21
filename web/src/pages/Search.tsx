import { useEffect, useRef, useState } from 'react';
import { api, formatTime, roleLabel, toRow, type RecordRow } from '../api';
import { Markdown, plainPreview } from '../components/Markdown';
import { message } from '../App';

interface Props {
  scope: string;
  indexConfigured: boolean;
  onError: (text: string) => void;
}

const TARGETS = [
  { value: 'all', label: '全部资料' },
  { value: 'memories', label: '只看记忆' },
  { value: 'events', label: '只看原始记录' },
];

export function SearchPage({ scope, indexConfigured, onError }: Props) {
  const [query, setQuery] = useState('');
  const [target, setTarget] = useState('all');
  const [rows, setRows] = useState<RecordRow[]>([]);
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [reader, setReader] = useState<RecordRow | null>(null);
  const [sources, setSources] = useState<RecordRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState('');
  const [semantic, setSemantic] = useState('');
  const [rebuilding, setRebuilding] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  // 打开就是最近记录；带 #ref 的地址可直接定位到某一条。
  useEffect(() => {
    const deepLink = decodeURIComponent(location.hash.replace(/^#/, '')).trim();
    void load('');
    if (deepLink) void open({ ref: deepLink, kind: 'event', text: '' });
  }, [scope]);

  async function load(text: string) {
    setBusy(true);
    onError('');
    try {
      const response = text.trim()
        ? await api.search(text.trim(), target)
        : await api.browse(target);
      setRows(response.results);
      setTotal(response.total);
      setTruncated(response.truncated);
      setNote(response.results.length === 0 ? '没有匹配的记录' : '');
      setSemantic(response.coverage?.semantic_search ?? '');
      if (!selected) {
        setReader(null);
        setSources([]);
      }
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  async function rebuild() {
    setRebuilding(true);
    onError('');
    try {
      await api.buildIndex();
      await load(query);
    } catch (cause) {
      onError(message(cause));
    } finally {
      setRebuilding(false);
    }
  }

  async function open(row: RecordRow) {
    setSelected(row.ref);
    setBusy(true);
    try {
      const [detail, origin] = await Promise.all([api.read(row.ref), api.sources(row.ref)]);
      const envelope = detail.results[0];
      setReader(envelope ? { ...row, ...toRow(envelope) } : row);
      setSources((origin.results ?? []).map(toRow).filter(item => item.text));
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="content">
      <form
        className="search-bar"
        onSubmit={event => {
          event.preventDefault();
          void load(query);
        }}
      >
        <input
          ref={input}
          type="text"
          value={query}
          placeholder="搜索原话、决定或已确认的事实…"
          onChange={event => setQuery(event.target.value)}
        />
        <button type="submit" className="primary" disabled={busy}>
          搜索
        </button>
        <select
          value={target}
          onChange={event => {
            setTarget(event.target.value);
            void load(query);
          }}
          style={{ width: 130 }}
          aria-label="搜索范围"
        >
          {TARGETS.map(item => (
            <option key={item.value} value={item.value}>
              {item.label}
            </option>
          ))}
        </select>
      </form>

      {indexConfigured && semantic === 'unavailable' && (
        <div className="notice warn" style={{ marginBottom: 14 }}>
          语义检索暂时关闭：索引还没建，或与当前资料对不上。关键词检索照常可用。
          <button
            className="quiet"
            style={{ marginLeft: 8 }}
            onClick={() => void rebuild()}
            disabled={rebuilding}
          >
            {rebuilding ? <span className="spin" /> : null} 重建索引
          </button>
        </div>
      )}

      <div className="page-head">
        <h1>{query.trim() ? '搜索结果' : '最近的记录'}</h1>
        <span className="hint">
          {note ||
            (total > 0
              ? `${total.toLocaleString('zh-CN')} 条${truncated ? '，仅显示最近的若干条，缩小关键词可看更多' : ''}`
              : '')}
        </span>
      </div>

      <div className="search-layout">
        <div className="result-list">
          {rows.length === 0 && !busy && (
            <div className="empty">
              <strong>{query.trim() ? '没有找到相关内容' : '这个资料库还是空的'}</strong>
              <span>
                {query.trim() ? '换个关键词，或切换筛选条件' : '用 recallcard capture 记下第一段对话'}
              </span>
            </div>
          )}
          {rows.map(row => (
            <button
              key={row.ref}
              className="result-row"
              aria-pressed={selected === row.ref}
              onClick={() => void open(row)}
            >
              <div className="row-meta">
                <span>{roleLabel[row.role ?? ''] ?? '记录'}</span>
                <span>·</span>
                <span>
                  {row.occurred_at
                    ? formatTime(row.occurred_at)
                    : row.captured_at
                      ? `记录于 ${formatTime(row.captured_at)}`
                      : '时间未记录'}
                </span>
                {row.kind === 'memory' && <span className="tag seal">记忆</span>}
                {row.platform && row.platform !== 'manual' && <span>{row.platform}</span>}
              </div>
              <div className={row.text.trim() ? 'row-text' : 'row-text muted'}>
                {row.text.trim() ? plainPreview(row.text, 220) : '这条记录只有元数据，没有正文'}
              </div>
            </button>
          ))}
        </div>

        <div className="reader">
          {!reader && (
            <div className="empty">
              <strong>选一条看原文</strong>
              <span>正文按 Markdown 渲染，代码带高亮，出处可在下方展开。</span>
            </div>
          )}
          {reader && (
            <>
              <div className="reader-head">
                <div className="reader-meta">
                  <span>{roleLabel[reader.role ?? ''] ?? '记录'}</span>
                  <span>·</span>
                  <span>
                    {reader.occurred_at
                      ? `发生于 ${formatTime(reader.occurred_at)}`
                      : reader.captured_at
                        ? `记录于 ${formatTime(reader.captured_at)}`
                        : '时间未记录'}
                  </span>
                  {reader.platform && reader.platform !== 'manual' && (
                    <span className="tag">{reader.platform}</span>
                  )}
                  {reader.kind === 'memory' && <span className="tag seal">记忆</span>}
                  {reader.text_truncated && <span className="tag warn">已截断</span>}
                </div>
                {!reader.occurred_at && reader.time_note && (
                  <div className="hint-line">{reader.time_note}</div>
                )}
                {reader.content_retained === false && (
                  <div className="notice warn">
                    这条记录按保留策略只留了元数据，正文没有保存在本地。
                  </div>
                )}
              </div>
              {reader.text.trim() ? (
                <Markdown text={reader.text} />
              ) : (
                <p className="muted">这条记录只有元数据与出处，正文没有保存在本地。</p>
              )}
              {sources.length > 0 && (
                <div className="source-list">
                  <div className="row-meta">出处 · {sources.length} 条</div>
                  {sources.map(source => (
                    <div className="source-item" key={source.ref}>
                      <div className="source-head">
                        <span>{roleLabel[source.role ?? ''] ?? '记录'}</span>
                        <span>{source.occurred_at ? formatTime(source.occurred_at) : '原始记录'}</span>
                      </div>
                      <div style={{ fontSize: 13.5, lineHeight: 1.65, color: 'var(--ink-soft)' }}>
                        {plainPreview(source.text, 320)}
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

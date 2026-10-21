import { useState } from 'react';
import { api, type StateResponse } from '../api';
import { message } from '../App';

interface Props {
  state: StateResponse;
  onError: (text: string) => void;
  onReload: () => void;
}

export function SettingsPage({ state, onError, onReload }: Props) {
  const settings = state.settings;
  const [baseUrl, setBaseUrl] = useState(settings.llm.base_url);
  const [model, setModel] = useState(settings.llm.model);
  const [llmKey, setLlmKey] = useState('');
  const [maxTokens, setMaxTokens] = useState(settings.llm.max_output_tokens || 4096);

  const [provider, setProvider] = useState(settings.embedding.provider);
  const [endpoint, setEndpoint] = useState(settings.embedding.endpoint);
  const [embeddingModel, setEmbeddingModel] = useState(settings.embedding.model);
  const [dimensions, setDimensions] = useState(settings.embedding.dimensions || 1024);
  const [embeddingKey, setEmbeddingKey] = useState('');

  const [vaultPath, setVaultPath] = useState(state.vault.root);
  const [busy, setBusy] = useState('');
  const [notice, setNotice] = useState('');
  const [mcp, setMcp] = useState('');

  async function saveLlm() {
    setBusy('llm');
    setNotice('');
    onError('');
    try {
      await api.saveSettings({
        llm: {
          base_url: baseUrl,
          model,
          max_output_tokens: maxTokens,
          key: llmKey,
        },
      });
      setLlmKey('');
      setNotice('模型服务已保存到本机设置文件（仅本人可读）。');
      onReload();
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy('');
    }
  }

  async function saveEmbedding(rebuild: boolean) {
    setBusy(rebuild ? 'index' : 'embedding');
    setNotice('');
    onError('');
    try {
      await api.saveSettings({
        embedding: {
          provider,
          endpoint,
          model: embeddingModel,
          dimensions,
          key: embeddingKey,
        },
      });
      setEmbeddingKey('');
      if (rebuild) {
        await api.buildIndex();
        setNotice('编码设置已保存，索引已重建。');
      } else {
        setNotice('编码设置已保存。改过模型或维度后需要重建索引。');
      }
      onReload();
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy('');
    }
  }

  async function buildIndex() {
    setBusy('index');
    setNotice('');
    onError('');
    try {
      await api.buildIndex();
      setNotice('索引已重建。');
      onReload();
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy('');
    }
  }

  async function openVault(create: boolean) {
    setBusy('vault');
    setNotice('');
    onError('');
    try {
      const result = await api.openVault(vaultPath, create);
      setNotice(`已切换到 ${result.vault.display_name}。`);
      onReload();
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy('');
    }
  }

  async function loadConnect() {
    setBusy('connect');
    onError('');
    try {
      const result = await api.connect();
      setMcp(JSON.stringify(result.config, null, 2));
    } catch (cause) {
      onError(message(cause));
    } finally {
      setBusy('');
    }
  }

  const index = state.index;

  return (
    <div className="content">
      <div className="page-head">
        <h1>设置</h1>
        <span className="hint">全部存在本机；密钥只写入仅本人可读的配置文件</span>
      </div>

      {notice && <div className="notice ok" style={{ marginBottom: 16 }}>{notice}</div>}

      <div className="settings">
        <section className="panel">
          <h2>模型服务</h2>
          <p className="desc">
            整理记忆时用来生成候选记忆。填 OpenAI 兼容的地址即可，例如
            https://api.deepseek.com/v1 或 https://api.openai.com/v1。
          </p>
          <div className="field">
            <label htmlFor="llm-url">服务地址</label>
            <input
              id="llm-url"
              type="text"
              value={baseUrl}
              placeholder="https://api.deepseek.com/v1"
              onChange={event => setBaseUrl(event.target.value)}
            />
          </div>
          <div className="field-row">
            <div className="field">
              <label htmlFor="llm-model">模型名</label>
              <input
                id="llm-model"
                type="text"
                value={model}
                placeholder="deepseek-chat"
                onChange={event => setModel(event.target.value)}
              />
            </div>
            <div className="field">
              <label htmlFor="llm-tokens">单次上限 tokens</label>
              <input
                id="llm-tokens"
                type="number"
                value={maxTokens}
                min={256}
                max={32000}
                onChange={event => setMaxTokens(Number(event.target.value))}
              />
            </div>
          </div>
          <div className="field">
            <label htmlFor="llm-key">
              密钥 {settings.llm.has_key ? '（已保存，留空则不修改）' : '（尚未填写）'}
            </label>
            <input
              id="llm-key"
              type="password"
              value={llmKey}
              placeholder={settings.llm.has_key ? '••••••••' : 'sk-…'}
              onChange={event => setLlmKey(event.target.value)}
            />
          </div>
          <div className="row">
            <button className="primary" onClick={() => void saveLlm()} disabled={busy !== ''}>
              {busy === 'llm' ? <span className="spin" /> : null} 保存
            </button>
            {settings.llm.has_key && (
              <button
                onClick={() =>
                  void api
                    .saveSettings({ llm: { clear_key: true } })
                    .then(() => {
                      setNotice('已删除本机保存的模型密钥。');
                      onReload();
                    })
                    .catch(cause => onError(message(cause)))
                }
              >
                删除密钥
              </button>
            )}
            <span className="hint-line">只有你在这里点「开始整理」时才会发出请求。</span>
          </div>
        </section>

        <section className="panel">
          <h2>编码服务</h2>
          <p className="desc">
            语义检索用它把文字变成向量。默认用进程内的本地模型，不联网、不需要密钥；
            换成云端服务后，重建索引时会把文本发到该服务。
          </p>
          <div className="radio-row">
            <label>
              <input
                type="radio"
                name="provider"
                checked={provider === 'local'}
                onChange={() => setProvider('local')}
              />
              本地模型（推荐）
            </label>
            <label>
              <input
                type="radio"
                name="provider"
                checked={provider === 'cloud'}
                onChange={() => setProvider('cloud')}
              />
              云端服务
            </label>
          </div>
          {provider === 'cloud' && (
            <>
              <div className="field">
                <label htmlFor="emb-url">服务地址</label>
                <input
                  id="emb-url"
                  type="text"
                  value={endpoint}
                  placeholder="https://dashscope.aliyuncs.com/compatible-mode/v1/embeddings"
                  onChange={event => setEndpoint(event.target.value)}
                />
              </div>
              <div className="field-row">
                <div className="field">
                  <label htmlFor="emb-model">模型名</label>
                  <input
                    id="emb-model"
                    type="text"
                    value={embeddingModel}
                    placeholder="text-embedding-v4"
                    onChange={event => setEmbeddingModel(event.target.value)}
                  />
                </div>
                <div className="field">
                  <label htmlFor="emb-dim">维度</label>
                  <input
                    id="emb-dim"
                    type="number"
                    value={dimensions}
                    min={1}
                    max={8192}
                    onChange={event => setDimensions(Number(event.target.value))}
                  />
                </div>
              </div>
              <div className="field">
                <label htmlFor="emb-key">
                  密钥 {settings.embedding.has_key ? '（已保存，留空则不修改）' : '（尚未填写）'}
                </label>
                <input
                  id="emb-key"
                  type="password"
                  value={embeddingKey}
                  placeholder={settings.embedding.has_key ? '••••••••' : 'sk-…'}
                  onChange={event => setEmbeddingKey(event.target.value)}
                />
              </div>
            </>
          )}
          <div className="row">
            <button onClick={() => void saveEmbedding(false)} disabled={busy !== ''}>
              保存
            </button>
            <button className="primary" onClick={() => void saveEmbedding(true)} disabled={busy !== ''}>
              {busy === 'index' ? <span className="spin" /> : null} 保存并重建索引
            </button>
          </div>
        </section>

        <section className="panel">
          <h2>索引状态</h2>
          <p className="desc">索引只是加速用的副本，删掉也能重建；事实始终在原始记录与记忆文件里。</p>
          <dl className="kv">
            <dt>状态</dt>
            <dd>{index.complete ? '可用的完整索引' : index.configured ? '不完整，需要重建' : '尚未建立'}</dd>
            <dt>编码空间</dt>
            <dd>{index.provider ? `${index.provider} · ${index.model}` : '本地模型'}</dd>
            <dt>维度</dt>
            <dd className="num">{index.dimensions ?? '—'}</dd>
            <dt>已编码</dt>
            <dd className="num">{index.documents?.toLocaleString('zh-CN') ?? '—'} 条</dd>
            {index.reason && (
              <>
                <dt>说明</dt>
                <dd>{index.reason}</dd>
              </>
            )}
          </dl>
          <div className="row" style={{ marginTop: 14 }}>
            <button onClick={() => void buildIndex()} disabled={busy !== ''}>
              {busy === 'index' ? <span className="spin" /> : null} 重建索引
            </button>
          </div>
        </section>

        <section className="panel">
          <h2>资料库</h2>
          <p className="desc">一个资料库就是一个目录，可以直接备份、复制或放进 Git 自行管理。</p>
          <dl className="kv">
            <dt>当前库</dt>
            <dd>{state.vault.root}</dd>
            <dt>原始记录</dt>
            <dd className="num">{state.vault.event_count.toLocaleString('zh-CN')}</dd>
            <dt>长期记忆</dt>
            <dd className="num">{state.vault.memory_count.toLocaleString('zh-CN')}</dd>
            <dt>完整性</dt>
            <dd>{state.vault.health.ok === false ? '有问题，需要检查' : '通过'}</dd>
          </dl>
          <div className="field" style={{ marginTop: 14 }}>
            <label htmlFor="vault-path">切换到另一个资料库</label>
            <input
              id="vault-path"
              type="text"
              value={vaultPath}
              onChange={event => setVaultPath(event.target.value)}
            />
          </div>
          <div className="row">
            <button onClick={() => void openVault(false)} disabled={busy !== ''}>
              打开这个目录
            </button>
            <button onClick={() => void openVault(true)} disabled={busy !== ''}>
              在这里新建资料库
            </button>
          </div>
        </section>

        <section className="panel">
          <h2>接入 AI 工具</h2>
          <p className="desc">
            生成一段 MCP 配置，粘到你的客户端里，Agent 就能在这个库的授权范围内检索；
            它只能追加记录，不能改记忆、不能删除。
          </p>
          <div className="row">
            <button onClick={() => void loadConnect()} disabled={busy !== ''}>
              {busy === 'connect' ? <span className="spin" /> : null} 生成配置片段
            </button>
            {mcp && (
              <button
                onClick={() => {
                  void navigator.clipboard.writeText(mcp);
                  setNotice('配置已复制到剪贴板。');
                }}
              >
                复制
              </button>
            )}
          </div>
          {mcp && <pre className="json">{mcp}</pre>}
        </section>
      </div>
    </div>
  );
}

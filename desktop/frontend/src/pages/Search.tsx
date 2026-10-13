import { useEffect, useState } from "react";
import { PortableContextDialog } from "../components/PortableContext";
import type { ScopedService } from "../service/client";
import { useResource } from "../service/hooks";
import {
  dateLabel,
  evidenceLabel,
  platformLabel,
  titleOf,
} from "../service/types";
import { Badge, EmptyState, ErrorNotice, Loading } from "../components/common";
import { Icon } from "../components/Icon";
import { navigate } from "../App";
export function SearchPage({
  service,
  query,
}: {
  service: ScopedService;
  query: string;
}) {
  const [sharing, setSharing] = useState(false);
  useEffect(() => setSharing(false), [service, query]);
  const results = useResource(() => service.search(query), [service, query]);
  return (
    <div className="standard-page search-page">
      <div className="page-heading">
        <div>
          <h1>搜索「{query}」</h1>
          <p>同时查找记忆与已保存的原话。尚未整理的来源也可被找到。</p>
        </div>
        {results.data && <Badge>{results.data.results.length} 条结果</Badge>}
      </div>
      {!results.loading && !results.error && !!results.data?.results.length && <button className="button primary" onClick={() => setSharing(true)}>带到任意 AI</button>}
      {sharing && results.data && <PortableContextDialog key={service.workspace.session_id + service.scope + query} service={service} initialGoal={query} choices={results.data.results.map(row => ({ref:row.ref, label:row.conversation_title || titleOf(row.text || row.content || row.snippet || row.ref, 72)}))} close={() => setSharing(false)} />}
      {results.loading ? (
        <Loading label="正在检索本机资料" />
      ) : results.error ? (
        <ErrorNotice error={results.error} retry={results.refresh} />
      ) : results.data?.results.length ? (
        <div className="search-results">
          {results.data.results.map((row) => (
            <button
              key={row.ref}
              className="search-result"
              data-reference={row.ref}
              onClick={() =>
                row.kind === "memory"
                  ? navigate("memories", row.ref.replace(/^memory:/, ""))
                  : row.conversation_ref
                    ? navigate("sources", row.conversation_ref, "", row.ref)
                    : undefined
              }
              disabled={row.kind !== "memory" && !row.conversation_ref}
            >
              <span className="search-result-icon">
                <Icon
                  name={row.kind === "memory" ? "memory" : "sources"}
                  size={19}
                />
              </span>
              <div>
                <h2>
                  {row.conversation_title ||
                    titleOf(row.text || row.content || row.snippet || "", 72)}
                </h2>
                <p>{row.text || row.content || row.snippet}</p>
                <div className="row-meta">
                  <Badge>{row.kind === "memory" ? "记忆" : "原始消息"}</Badge>
                  <span>
                    {row.platform
                      ? platformLabel(row.platform)
                      : evidenceLabel(row.evidence || "")}
                  </span>
                  <span>{dateLabel(row.occurred_at)}</span>
                </div>
              </div>
              <Icon name="arrow" size={17} />
            </button>
          ))}
          {results.data.truncated && (
            <p className="coverage-note">
              结果达到单次读取上限。使用更具体的关键词继续查找。
            </p>
          )}
        </div>
      ) : (
        <EmptyState icon="search" title="没有找到匹配内容">
          <p>试试人名、项目名或一句原话中的关键词。</p>
        </EmptyState>
      )}
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import {
  appError,
  isRunning,
  type AppError,
  type JobStatus,
} from "./contracts";
import type { ScopedService } from "./client";

export function useResource<T>(
  load: () => Promise<T>,
  dependencies: unknown[],
) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((value) => value + 1), []);
  useEffect(() => {
    let current = true;
    setData(null);
    setError(null);
    setLoading(true);
    load()
      .then((result) => {
        if (current) setData(result);
      })
      .catch((reason) => {
        if (current) setError(appError(reason));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
    // 调用者提供稳定的资源身份；旧结果不能覆盖新页面。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...dependencies, revision]);
  return { data, error, loading, refresh };
}
export function useJobs(service: ScopedService, onCompletion: () => void) {
  const [jobs, setJobs] = useState<JobStatus[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [revision, setRevision] = useState(0);
  const previous = useRef(new Map<string, { state: string; added: number }>());
  const lastContentRefresh = useRef(0);
  const completion = useRef(onCompletion);
  completion.current = onCompletion;
  const refresh = useCallback(() => setRevision((value) => value + 1), []);
  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let delay = 8000;
      try {
        if (document.visibilityState !== "hidden") {
          const results = await service.jobs();
          if (!current) return;
          const completed = results.some(
            (job) =>
              job.state === "completed" &&
              previous.current.has(job.job_id) &&
              previous.current.get(job.job_id)?.state !== "completed",
          );
          const committed = results.some(
            (job) =>
              job.progress.events_added >
              (previous.current.get(job.job_id)?.added ?? 0),
          );
          if (
            completed ||
            (committed && Date.now() - lastContentRefresh.current > 5000)
          ) {
            lastContentRefresh.current = Date.now();
            completion.current();
          }
          previous.current = new Map(
            results.map((job) => [
              job.job_id,
              { state: job.state, added: job.progress.events_added },
            ]),
          );
          setJobs(results);
          setError(null);
          if (results.some(isRunning)) delay = 800;
        }
      } catch (reason) {
        if (current) setError(appError(reason));
      }
      if (current) timer = setTimeout(poll, delay);
    };
    void poll();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [service, revision]);
  return { jobs, error, refresh };
}

/** 可见页面的只读观察；刷新保留上次内容，但错误时禁止把旧状态当新状态。 */
export function usePollingResource<T>(load: () => Promise<T>, dependencies: unknown[], interval = 4000) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (document.visibilityState !== 'hidden') {
        try { const value = await load(); if (current) { setData(value); setError(null); } }
        catch (reason) { if (current) setError(appError(reason)); }
        finally { if (current) setLoading(false); }
      }
      if (current) timer = setTimeout(poll, interval);
    };
    void poll();
    return () => { current = false; clearTimeout(timer); };
  }, [...dependencies, revision, interval]);
  return { data, error, loading, refresh, setData };
}

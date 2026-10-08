import { useCallback, useEffect, useRef, useState } from 'react';
import { appError, isRunning, type AppError, type JobStatus } from './contracts';
import type { ScopedService } from './client';

export function useResource<T>(load: () => Promise<T>, dependencies: unknown[]) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    let current = true;
    setData(null); setError(null); setLoading(true);
    load().then(result => { if (current) setData(result); }).catch(reason => { if (current) setError(appError(reason)); }).finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
    // 调用者提供稳定的资源身份；旧结果不能覆盖新页面。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...dependencies, revision]);
  return { data, error, loading, refresh };
}
export function useJobs(service: ScopedService, onCompletion: () => void) {
  const [jobs, setJobs] = useState<JobStatus[]>([]);
  const [error, setError] = useState<AppError | null>(null);
  const [revision, setRevision] = useState(0);
  const previous = useRef(new Map<string, string>());
  const completion = useRef(onCompletion); completion.current = onCompletion;
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      let delay = 8000;
      try {
        if (document.visibilityState !== 'hidden') {
          const results = await service.jobs();
          if (!current) return;
          if (results.some(job => job.state === 'completed' && previous.current.has(job.job_id) && previous.current.get(job.job_id) !== 'completed')) completion.current();
          previous.current = new Map(results.map(job => [job.job_id, job.state]));
          setJobs(results); setError(null);
          if (results.some(isRunning)) delay = 800;
        }
      } catch (reason) { if (current) setError(appError(reason)); }
      if (current) timer = setTimeout(poll, delay);
    };
    void poll();
    return () => { current = false; clearTimeout(timer); };
  }, [service, revision]);
  return { jobs, error, refresh };
}

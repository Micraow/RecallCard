import { useEffect, useRef, useState } from 'react';
import type { ScopedService } from '../service/client';
import { appError, type AppError } from '../service/contracts';
import { canApproveMemoryReview, type MemoryJobReview } from '../service/runtime';
import { dateLabel, eventText, evidenceLabel } from '../service/types';
import { navigate } from '../App';
import { Badge, Dialog, ErrorNotice, Loading, Prose } from './common';

export function MemoryReviewDialog({ service, jobId, close, committed }: { service: ScopedService; jobId: string; close: () => void; committed: () => void }) {
  const [data,setData]=useState<MemoryJobReview|null>(null);
  const [error,setError]=useState<AppError|null>(null);
  const [loading,setLoading]=useState(true);
  const [busy,setBusy]=useState(false);
  const [confirmed,setConfirmed]=useState(false);
  const [protectedApproved,setProtectedApproved]=useState(false);
  const [revision,setRevision]=useState(0);
  const submitting=useRef(false);
  useEffect(()=>{
    let current=true;
    setLoading(true);setData(null);setError(null);setConfirmed(false);setProtectedApproved(false);
    service.reviewMemoryJob(jobId).then(value=>{if(current)setData(value);}).catch(reason=>{if(current)setError(appError(reason));}).finally(()=>{if(current)setLoading(false);});
    return ()=>{current=false;};
  },[service,jobId,revision]);
  const approve=async()=>{
    if(submitting.current||!canApproveMemoryReview(data,confirmed,protectedApproved))return;
    submitting.current=true;setBusy(true);setError(null);
    try {
      const result=await service.approveMemoryJob(jobId,data!.review!.result_hash,protectedApproved);
      if(result.state!=='completed')throw new Error('写回尚未确认完成，请重新读取任务状态');
      const first=data?.review?.changes.find(change=>change.after)?.after?.id;
      committed();if(first)navigate('memories',first);
    } catch(reason) {
      setError(appError(reason));setData(null);setConfirmed(false);setProtectedApproved(false);
    } finally {submitting.current=false;setBusy(false);}
  };
  const openSource=async(reference:string)=>{
    if(submitting.current)return;
    submitting.current=true;setBusy(true);setError(null);
    try {const location=await service.eventLocation(reference);close();navigate('sources',location.conversation_ref,'',reference);}
    catch(reason){setError(appError(reason));setData(null);setConfirmed(false);setProtectedApproved(false);}
    finally{submitting.current=false;setBusy(false);}
  };
  const review=data?.review;
  return <Dialog title="审阅整理候选" close={close} busy={busy} wide>
    <div className="modal-body memory-candidate-review">
      <p>候选尚未发布。请核对任务是否相同、用户原话和完整差异；助手建议不表示用户采用。</p>
      {loading&&<Loading label="核对当前来源、范围与版本"/>}
      {error&&<ErrorNotice error={error} retry={!busy?()=>setRevision(value=>value+1):undefined} retryLabel="重新读取候选"/>}
      {data&&<>
        <p>{data.note}</p>
        {data.semantic_lookup&&<p>语义候选：{({available:'语义检索可用',partial:'部分覆盖',unavailable:'不可用，保留目录/词法路径',unconfigured:'未配置，使用目录/词法路径'} as Record<string,string>)[data.semantic_lookup.status]||'状态待核对'} · 查询 {data.semantic_lookup.queries} 次 · 缓存命中 {data.semantic_lookup.cache_hits} 次 · 本机新编码 {data.semantic_lookup.local_encodings||0} 次{data.semantic_lookup.errors.length>0?` · ${data.semantic_lookup.errors.join('、')}`:''}</p>}
        {data.semantic_lookup?.directory_refs.length? <p>相关目录：{data.semantic_lookup.directory_refs.join('、')}</p>:null}
        {data.extraction&&<details><summary>查看独立提取单元（{data.extraction.result.proposals.length}）</summary>{data.extraction.result.proposals.map((p,index)=><section key={index}><strong>{p.operation}</strong><Prose text={p.content||'没有正文变更'}/><p>{p.source_refs.join('、')}</p></section>)}</details>}
        {!review&&<p role="status">尚未产生完整整合结果，当前不能批准。</p>}
        {review&&<>
          <h3>完整写回差异</h3>
          {review.changes.length===0&&<p>此结果没有 Memory 正文变更。</p>}
          {review.changes.map((change,index)=><section className="memory-review-change" key={index}>
            <Badge>{change.operation}</Badge>
            {change.before&&<details open><summary>之前 · {change.before.id}@{change.before.revision}</summary><Prose text={change.before.content}/></details>}
            {change.after&&<><h4>之后 · {change.after.id}@{change.after.revision}</h4><Badge>{evidenceLabel(change.after.evidence)}</Badge><Prose text={change.after.content}/><p>来源：{change.after.source_refs.join('、')}</p>{change.after.supersedes?.length?<p>历史关联：{change.after.supersedes.join('、')}</p>:null}{change.after.navigation?.length?<p>目录：{change.after.navigation.map(hint=>hint.path).join('、')}</p>:null}</>}
          </section>)}
          {review.diagnostics.map((note,index)=><p key={index}>{note}</p>)}
          {review.split_history?.map((history,index)=><p key={index}>拆分历史：{history.target_ref} → {history.retired_ref} → {history.branch_refs.join('、')}</p>)}
          <details><summary>本次完整结果摘要</summary><p style={{overflowWrap:'anywhere'}}>{review.result_hash}</p></details>
        </>}
        <h3>完整来源快照（{data.source_snapshots.length}）</h3>
        {data.source_snapshots.map(source=><details key={source.ref}><summary>{({user:"用户",assistant:"助手建议",system:"系统",tool:"工具"} as Record<string,string>)[source.event.role]||source.event.role} · {dateLabel(source.event.occurred_at)} · {source.ref}</summary><Prose text={eventText(source.event)}/><button className="text-button" disabled={busy} onClick={()=>void openSource(source.ref)}>打开当前原始会话</button></details>)}
        {review?.can_apply&&!review.already_applied&&<>
          <label className="checkbox-row"><input type="checkbox" checked={confirmed} disabled={busy} onChange={event=>setConfirmed(event.target.checked)}/>我已核对完整差异和来源，批准此摘要对应的候选</label>
          {review.requires_protected_approval&&<label className="checkbox-row"><input type="checkbox" checked={protectedApproved} disabled={busy} onChange={event=>setProtectedApproved(event.target.checked)}/>另外批准本次受保护记忆的变更</label>}
        </>}
        {review&&!review.can_apply&&<p role="alert">候选存在未解决的证据或任务边界冲突，当前不能批准。</p>}
      </>}
    </div>
    <div className="modal-footer"><button className="button" disabled={busy} onClick={close}>暂不批准</button><button className="button primary" disabled={busy||!canApproveMemoryReview(data,confirmed,protectedApproved)} onClick={()=>void approve()}>{busy?'正在核验…':'批准并写回'}</button></div>
  </Dialog>;
}

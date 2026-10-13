/** 纯 host 账本。bindingKey 必须包含宿主身份、会话/范围、接收方及权限修订。
 * state 可 JSON 序列化；函数会同步修改 state，两个摘要函数在 await 后重新检查活动 run。
 * 真实宿主用 now:()=>Date.now()；固定数值仅供确定性测试，异步摘要后须重新取时。
 * 不验证本机授权；host 必须实际读/核验，不能把 state 或 verified 字段当作权限。 */
export type TextContext = { bindingKey: string; now: number | (()=>number); verified?: boolean };
export type TextPhase = 'idle' | 'preparing' | 'awaiting_user_send' | 'waiting_reply' | 'result_ready' | 'paused' | 'blocked';
export type TextLimits = { reads:number; actions:number; packetBytes:number; disclosureBytes:number; durationMs:number; attemptMs:number; aliases:number };
export type TextPacket = { id:string; hash:string; text:string; bytes:number; aliases:string[]; sourceRequirements:TextSourceRequirement[]; awaitingSend:boolean; disclosureAttempts:number; sent:null|{kind:'manual'|'observed';messageId:string|null} };
export type TextAttempt = { id:string; runId:string; epoch:number; bindingKey:string; deadline:number; action:'search'|'fetch'|'final'; tool:'recall_search'|'recall_fetch'|null; args:{query:string}|{id:string}|null; alias?:string; source?:string; body?:string; sources?:string[] };
export type TextSource = {key:string;locator:string;ref:string;scope:string;snapshot:string;sent:boolean};
export type TextSourceRequirement = TextSource & {id:string;textHash?:string;textRange?:Record<string,unknown>};
export type TextRead = {id:string;runId:string;epoch:number;bindingKey:string;target:string;deadline:number};
export type TextFinal = {body:string;sources:string[]};
export interface TextSession {
  protocol:'text/2';runId:string;epoch:number;bindingKey:string;startedAt:number;deadline:number;limits:TextLimits;
  phase:TextPhase;paused:boolean;cancelled:boolean;completed:boolean;notified:boolean;round:number;sequence:number;
  budget:{reads:number;actions:number;disclosureBytes:number};packet:TextPacket|null;
  ticket:{value:string;active:boolean;consumed:boolean}|null;consumedTickets:string[];packetIds:string[];
  sources:Record<string,TextSource>;pages:Record<string,{locator:string;source:string;sent:boolean}>;
  nextSource:number;nextPage:number;pending:TextAttempt|null;
  failure:{attempt:TextAttempt;code:string;message:string}|null;
  ready:{data:Record<string,unknown>;aliases:string[];requirements:TextSourceRequirement[]}|null;checks:Record<string,TextRead>;
  freshness:{target:string;epoch:number;deadline:number}|null;final:TextFinal|null;resumePhase?:TextPhase;
}
export const TEXT_PHASES:readonly TextPhase[];
export const TEXT_LIMITS:Readonly<TextLimits>;
export function textPacketHash(text:string):Promise<string>;
export function createTextSession(options:{runId:string;epoch?:number;bindingKey:string;now:number;limits?:Partial<TextLimits>}):TextSession;
export function packetFreshnessTarget(state:TextSession):string;
export function finalFreshnessTarget(state:TextSession):string;
/** 在宿主每一次本机授权读/复核之前调用。consume/retry 已扣主 search/fetch 读取。每次实际调用逐次扣费，一个凭据不能当成 N 个来源读取；普通读保留 1 次终态绑定复核。 */
export function reserveTextRead(state:TextSession,ctx:TextContext,target?:string):TextRead;
/** host 完成该目标全部来源、背景、范围、接收方与权限的实时复核后才调用。 */
export function confirmTextFreshness(state:TextSession,readId:string,ctx:TextContext):boolean;
/** 初始背景须经 host 批准；零来源/null 支持直接 final。资料全部编码为 JSON。 */
export function prepareTextPacket(state:TextSession,options:{packetId:string;ticket:string;bootstrap?:unknown},ctx:TextContext):Promise<TextPacket>;
/** 每次复制/外部草稿填入前调用；必须先复核当前 packet。overheadBytes 是外标记及额外换行的真实 UTF-8 字节差，同时计入每包和累计上限；失败也不返预算。 */
export function reserveTextDisclosure(state:TextSession,ctx:TextContext,options?:{overheadBytes?:number}):TextPacket & {disclosureBytes:number};
export function confirmTextSent(state:TextSession,receipt:{packetId:string;packetHash:string;kind:'manual'|'observed';messageId?:string},ctx:TextContext):boolean;
/** 原子消费当前已发送票据；主读前还保留至少结果包复核和终态各 1 次。host 若需更多调用须先预算。返回只读请求。final 返回等待宿主复核的 pending。 */
export function consumeTextReply(state:TextSession,text:string,ctx:TextContext):TextAttempt;
/** 仅 verified:true 表示 host 已实际重核验当前 binding、来源与结果；迟到返回 false。 */
export function acceptTextResult(state:TextSession,attemptId:string,result:unknown,ctx:TextContext):Promise<boolean>;
export function acceptTextError(state:TextSession,attemptId:string,failure:{code:string;message?:string},ctx:TextContext):boolean;
export function expireTextAttempt(state:TextSession,ctx:TextContext):boolean;
export function retryTextAttempt(state:TextSession,ctx:TextContext):TextAttempt;
/** 要求 finalFreshnessTarget 的实际复核；首次返回 final，其余 false，不再次通知。 */
export function completeTextFinal(state:TextSession,attemptId:string,ctx:TextContext):TextFinal|false;
export function pauseTextSession(state:TextSession,ctx:TextContext):boolean;
export function resumeTextSession(state:TextSession,ctx:TextContext):boolean;
export function cancelTextSession(state:TextSession):boolean;
export function textSourceRequirements(state:TextSession,target?:'packet'|'final'):TextSourceRequirement[];
/** 超大完整结果可由 host 明确降为预算不足包；不会截断或声称没有匹配。 */
export function discardTextResultAsBudget(state:TextSession,ctx:TextContext):boolean;

export type ConnectPhase='idle'|'selecting'|'injecting'|'waiting'|'failed';
export interface ConnectSnapshot {phase:ConnectPhase;message:string;candidates:{pid:number;path:string}[];target_pid:number|null}
export function connectionButton(connected:boolean,phase:ConnectPhase){
 if(connected)return {label:'已连接',disabled:true};
 if(phase==='injecting'||phase==='waiting')return {label:'连接中…',disabled:true};
 if(phase==='selecting')return {label:'请选择进程',disabled:true};
 return {label:phase==='failed'?'重试连接':'连接游戏',disabled:false};
}

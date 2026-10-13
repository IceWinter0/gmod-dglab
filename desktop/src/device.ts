export type DeviceMode='app3'|'app4';
export type DevicePhase='idle'|'connecting'|'waiting_for_app'|'waiting_for_device'|'ready'|'disconnected'|'failed';
export interface DeviceSettings {mode:DeviceMode;v3_url:string;v4_url:string}
export interface DeviceSnapshot {mode:DeviceMode;phase:DevicePhase;message:string;service_url:string;qr_data_url:string|null;strengths:(number|null)[];limits:(number|null)[];choices:{key:string;name:string}[];selected_key:string|null;stop_status:'none'|'requested'|'confirmed'|'unconfirmed'}
export const defaultDeviceSettings=():DeviceSettings=>({mode:'app3',v3_url:'wss://ws.dungeon-lab.cn/',v4_url:'wss://trex.dungeon-lab.cn/v4'});
export function deviceStatus(device:DeviceSnapshot|undefined):string {return ({idle:'未连接',connecting:'正在连接',waiting_for_app:'等待扫码',waiting_for_device:'等待设备状态',ready:'已连接',disconnected:'连接断开',failed:'连接失败'} as const)[device?.phase??'idle'];}
export function canStartOutput(device:DeviceSnapshot|undefined,game:boolean,alive:boolean,dirty:boolean,channels:readonly {enabled:boolean}[]):boolean {
 return device?.phase==='ready' && game && alive && !dirty && channels.some(channel=>channel.enabled) && channels.every((channel,i)=>!channel.enabled || (device.limits[i]!=null && Number.isFinite(device.limits[i])));
}

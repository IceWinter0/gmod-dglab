import type {ConnectSnapshot} from './connection';
import type {DeviceSettings,DeviceSnapshot} from './device';
import { invoke } from '@tauri-apps/api/core';
import type { RuleConfig } from './validation';
export interface Player { entity_key:number; health:number; armor:number|null; life_state:number }
export interface Snapshot {
  controller: { config:RuleConfig; preview_enabled:boolean; player:Player|null; max_health:number|null; targets:number[]; injury_remaining_ms:number; death_remaining_ms:number; injury_cooldown_remaining_ms:number; death_cooldown_remaining_ms:number; logs:{id:number;at_ms:number;text:string}[] };
  connection:ConnectSnapshot; game_process_id:number|null; game_connected:boolean; heartbeat_age_ms:number|null; listen_address:string; listener_error:string|null;
  device:DeviceSnapshot;device_settings:DeviceSettings;
}
export interface Waveform { id:string; name:string; points:number[]; frequency_codes:number[]; frames:string[]; source:string }
export const api = {
  openProjectPage:()=>invoke<void>('open_project_page'),
  connectDevice:(settings:DeviceSettings)=>invoke<void>('connect_device',{settings}),
  disconnectDevice:()=>invoke<void>('disconnect_device'),
  deviceSettings:(settings:DeviceSettings)=>invoke<void>('update_device_settings',{settings}),
  selectDevice:(key:string)=>invoke<void>('select_device',{key}),
  connectGame:(selectedPid?:number)=>invoke<void>('connect_game',{selectedPid:selectedPid??null}),
  cancelGameSelection:()=>invoke<void>('cancel_game_selection'),
  state: () => invoke<Snapshot>('get_state'),
  configure: (config:RuleConfig) => invoke<void>('update_config',{config}),
  preview: (enabled:boolean) => invoke<void>('set_preview',{enabled}),
  stop: () => invoke<void>('stop_preview'),
  listen: (address:string) => invoke<void>('set_listen_address',{address}),
  waveforms: () => invoke<Waveform[]>('get_waveforms'),
  importWaveforms: (json:string) => invoke<number>('import_waveforms',{json}),
};

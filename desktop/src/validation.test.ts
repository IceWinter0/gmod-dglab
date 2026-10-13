import { test } from 'node:test';
import assert from 'node:assert/strict';
import { validateConfig, copyConfig } from './validation.ts';
import { reactive } from 'vue';
import { DeveloperAccess } from './developer.ts';
import { readFileSync } from 'node:fs';
import {canStartOutput,deviceStatus,type DeviceSnapshot} from './device.ts';
test('device pairing and missing limits cannot enable output',()=>{
  const channels=[{enabled:true},{enabled:false}];
  const device={phase:'waiting_for_app',limits:[null,null]} as DeviceSnapshot;
  assert.equal(canStartOutput(device,true,true,false,channels),false);
  device.phase='ready';assert.equal(canStartOutput(device,true,true,false,channels),false);
  device.limits=[35,null];assert.equal(canStartOutput(device,true,true,false,channels),true);
  assert.equal(canStartOutput(device,false,true,false,channels),false);
  assert.equal(canStartOutput(device,true,true,true,channels),false);
  device.phase='disconnected';assert.equal(canStartOutput(device,true,true,false,channels),false);
  assert.equal(deviceStatus(device),'连接断开');
});
test('developer tools require seven consecutive clicks and reset on exit', () => {
  const access=new DeveloperAccess();
  for(let i=0;i<6;i++) {access.click(i*100);assert.equal(access.enabled,false);}
  access.click(600);assert.equal(access.enabled,true);
  access.exit();assert.equal(access.enabled,false);
  access.click(700);assert.equal(access.clicks,1);
  assert.equal(new DeveloperAccess().enabled,false);
});
test('a pause breaks the developer unlock sequence', () => {
  const access=new DeveloperAccess();
  for(let i=0;i<6;i++)access.click(i*100);
  access.click(4000);assert.equal(access.clicks,1);assert.equal(access.enabled,false);
});
const config = () => ({ channels: [{enabled:true,base_strength:0,max_strength:100,waveform:'official:breathing',mapping_rate:1,mapping_unit:'points' as const}, {enabled:false,base_strength:0,max_strength:0,waveform:'official:tide',mapping_rate:1,mapping_unit:'points' as const}], injury_mode:'accumulate' as const,duration_ms:300, death_enabled:false, death_strength:0,injury_cooldown_ms:1000,death_duration_ms:300,death_cooldown_ms:1000,death_channels:'all' as const });
test('accepts valid preview config', () => assert.equal(validateConfig(config()), null));
test('supports official and imported waveform selection', () => { const c=config();c.channels[0]!.waveform='official:breathing';c.channels[1]!.waveform='user:'+ 'a'.repeat(64);assert.equal(validateConfig(c),null);c.channels[1]!.waveform='user:unknown';assert.ok(validateConfig(c)); });
test('all bundled official waveforms can be selected in both channels', () => {
  const waves = JSON.parse(readFileSync(new URL('../../crates/controller/data/official-coyote.json', import.meta.url), 'utf8')) as {id:string;name:string}[];
  assert.ok(waves.length > 0);
  for (const wave of waves) {
    for (const channel of [0,1]) {
      const c=config();c.channels[channel]!.waveform=wave.id;
      assert.equal(validateConfig(c),null,`${wave.name} (${wave.id}) in channel ${channel}`);
    }
  }
});
test('copies actual Vue reactive config for IPC', () => { const original=reactive(config()); const copy=copyConfig(original); assert.deepEqual(copy,config()); assert.doesNotThrow(()=>structuredClone(copy)); copy.channels[0]!.max_strength=10; assert.equal(original.channels[0]!.max_strength,100); });
test('rejects inverted range, fractions, nonfinite and duration', () => {
  for (const max of [-1, 201, 0.5, NaN, Infinity]) { const c = config(); c.channels[0]!.max_strength=max; assert.ok(validateConfig(c)); }
  const c=config(); c.channels[0]!.base_strength=101; assert.ok(validateConfig(c));
  c.channels[0]!.base_strength=0; c.duration_ms=99; assert.ok(validateConfig(c));
});

test('supports long independent durations and rejects invalid mapping/cooldown/channel',()=>{
 const c=config();c.duration_ms=10000;c.death_duration_ms=10000;c.channels[0]!.mapping_rate=0.25;assert.equal(validateConfig(c),null);
 c.channels[0]!.mapping_rate=NaN;assert.ok(validateConfig(c));c.channels[0]!.mapping_rate=1;
 c.death_cooldown_ms=60001;assert.ok(validateConfig(c));c.death_cooldown_ms=0;c.duration_ms=10001;assert.ok(validateConfig(c));
});

test('validates injury mode and mapping unit and excludes removed presets',()=>{
 const c=config();assert.equal(validateConfig({...c,injury_mode:'health_deficit'}),null);
 assert.equal(validateConfig({...c,channels:c.channels.map(channel=>({...channel,mapping_unit:'percent'}))}),null);
 assert.ok(validateConfig({...c,injury_mode:'unknown' as any}));
 assert.ok(validateConfig({...c,channels:c.channels.map(channel=>({...channel,mapping_unit:'unknown' as any}))}));
 c.channels[0]!.waveform='steady';assert.ok(validateConfig(c));
});

import {connectionButton} from './connection.ts';
test('connection button reflects attempts without enabling rules',()=>{assert.deepEqual(connectionButton(false,'idle'),{label:'连接游戏',disabled:false});assert.deepEqual(connectionButton(false,'failed'),{label:'重试连接',disabled:false});for(const phase of ['injecting','waiting'] as const)assert.deepEqual(connectionButton(false,phase),{label:'连接中…',disabled:true});assert.deepEqual(connectionButton(true,'failed'),{label:'已连接',disabled:true});});

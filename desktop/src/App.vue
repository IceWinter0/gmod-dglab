<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import {connectionButton} from './connection';
import ThemeSelect from './ThemeSelect.vue';
import { DeveloperAccess } from './developer';
import { api, type Snapshot, type Waveform } from './api';
import brandLogo from './assets/gmod-icon-44.png';
import brandLogo2x from './assets/gmod-icon-88.png';
import { defaultConfig, validateConfig, copyConfig, type RuleConfig } from './validation';
const state=ref<Snapshot|null>(null), draft=ref<RuleConfig>(defaultConfig()), waveforms=ref<Waveform[]>([]);
const error=ref(''), notice=ref(''), address=ref('127.0.0.1:47831'), busy=ref(false), loaded=ref(false);
const page=ref<'control'|'waves'|'settings'|'developer'>('control');
const developer=reactive(new DeveloperAccess());
const selectedGame=ref('');
const gameButton=computed(()=>connectionButton(connected.value,state.value?.connection.phase??'idle'));
const gameOptions=computed(()=>state.value?.connection.candidates.map(game=>({value:String(game.pid),label:`PID ${game.pid} · ${game.path}`}))??[]);
const selectedWave=ref('official:breathing'), search=ref('');
const visibleWaves=computed(()=>waveforms.value.filter(w=>`${w.name} ${w.source}`.includes(search.value.trim())));
const selected=computed(()=>waveforms.value.find(w=>w.id===selectedWave.value));
const waveformOptions=computed(()=>waveforms.value.map(w=>({value:w.id,label:`${w.name} · ${w.source}`})));
const injuryModes=[{value:'accumulate',label:'连续受伤累加'},{value:'health_deficit',label:'按缺失生命值计算'}] as const;
const mappingUnits=[{value:'points',label:'每 1 点血量'},{value:'percent',label:'每 1% 最大生命值'}] as const;
const deathChannelOptions=[{value:'all',label:'所有已开启联动的通道'},{value:'a',label:'仅 A 通道（需开启联动）'},{value:'b',label:'仅 B 通道（需开启联动）'}] as const;
const pageTitle=computed(()=>({control:'联动控制',waves:'波形管理',settings:'设置',developer:'开发者工具'})[page.value]);
function unlockDeveloper() {
  if(developer.enabled){notice.value='开发者模式已开启';return;}
  developer.click(performance.now());
  if(developer.enabled)notice.value='开发者模式已开启，可从左侧进入开发者工具';
  else if(developer.clicks>=4)notice.value=`再点击 ${7-developer.clicks} 次进入开发者模式`;
}
function exitDeveloper(){developer.exit();page.value='settings';notice.value='已退出开发者模式';}
let timer:ReturnType<typeof setInterval>|undefined, polling=false;
const dirty=computed(() => state.value ? JSON.stringify(draft.value)!==JSON.stringify(state.value.controller.config) : false);
const validation=computed(()=>validateConfig(draft.value));
const connected=computed(()=>state.value?.game_connected ?? false);
const player=computed(()=>state.value?.controller.player);
const active=computed(()=>state.value?.controller.preview_enabled ?? false);
const logs=computed(()=>[...(state.value?.controller.logs ?? [])].reverse());
const healthPercent=computed(()=>player.value && state.value?.controller.max_health ? Math.max(0,Math.min(100,100*player.value.health/state.value.controller.max_health)) : 0);
const canEnable=computed(()=>connected.value && player.value?.life_state===0 && !dirty.value && !validation.value);
async function refresh() {
  if(polling) return; polling=true;
  try {
    state.value=await api.state();
    if(!loaded.value) { draft.value=copyConfig(state.value.controller.config); address.value=state.value.listen_address; loaded.value=true; }
  } catch { error.value='无法连接桌面控制核心，请通过 Windows 桌面程序打开此面板。'; }
  finally { polling=false; }
}
async function action(fn:()=>Promise<void>, message:string) {
  busy.value=true; error.value=''; notice.value='';
  try { await fn(); notice.value=message; await refresh(); }
  catch(e) { error.value=String(e); }
  finally { busy.value=false; }
}
async function connectGame(selected=false){await action(()=>api.connectGame(selected?Number(selectedGame.value):undefined),'');}
async function save() { if(validation.value) {error.value=validation.value;return;} await action(()=>api.configure(copyConfig(draft.value)),'规则已保存'); }
async function togglePreview() { await action(()=>api.preview(!active.value),active.value?'联动已停用':'联动已启用'); }
async function applyAddress() { await action(()=>api.listen(address.value),'监听地址已更新，请等待 DLL 重连'); }
async function importFile(event:Event) {
  const input=event.target as HTMLInputElement; const file=input.files?.[0]; if(!file)return;
  if(file.size>1024*1024) {error.value='波形文件不能超过 1 MiB';input.value='';return;}
  await action(async()=>{await api.importWaveforms(await file.text());waveforms.value=await api.waveforms();},'波形已导入，可在 A/B 通道选择');
  input.value='';
}
function setCooldown(key:'injury_cooldown_ms'|'death_cooldown_ms',event:Event){const text=(event.target as HTMLInputElement).value;draft.value[key]=text.trim()===''?NaN:Math.round(Number(text)*1000);}
function waveHint(id:string) {const w=waveforms.value.find(w=>w.id===id); return w?.frames.length ? `${w.source} · ${(w.frames.length/10).toFixed(1)} 秒 · 相对参数预览` : '波形尚未加载';}
function frequencyPoints(id:string) {const w=waveforms.value.find(w=>w.id===id);return w?.frequency_codes.map((v,i)=>`${i*300/Math.max(1,w.frequency_codes.length-1)},${66-v/240*48}`).join(' ')??'';}
function points(id:string) { const w=waveforms.value.find(w=>w.id===id); return w?.points.map((v,i)=>`${i*300/Math.max(1,w.points.length-1)},${66-v*48}`).join(' ') ?? ''; }
function time(ms:number) { return `${Math.floor(ms/60000).toString().padStart(2,'0')}:${Math.floor(ms/1000%60).toString().padStart(2,'0')}.${(ms%1000).toString().padStart(3,'0')}`; }
onMounted(async()=>{ await refresh(); try {waveforms.value=await api.waveforms();} catch { /* 浏览器页面没有桌面桥接，不制造设备状态。 */ } timer=setInterval(refresh,100); });
onUnmounted(()=>{if(timer)clearInterval(timer);});
</script>

<template>
  <div class="app-frame">
    <aside class="navigation">
      <div class="brand"><img class="brand-mark" :src="brandLogo" :srcset="`${brandLogo} 1x, ${brandLogo2x} 2x`" alt="GMod 郊狼联动标志"><div><h1>GMod × 郊狼</h1><p>游戏联动控制面板</p></div></div>
      <nav aria-label="主导航">
        <button :class="{selected:page==='control'}" :aria-current="page==='control'?'page':undefined" @click="page='control'">联动控制</button>
        <button :class="{selected:page==='waves'}" :aria-current="page==='waves'?'page':undefined" @click="page='waves'">波形管理</button>
        <button :class="{selected:page==='settings'}" :aria-current="page==='settings'?'page':undefined" @click="page='settings'">设置</button>
        <button v-if="developer.enabled" :class="{selected:page==='developer'}" :aria-current="page==='developer'?'page':undefined" @click="page='developer'">开发者工具</button>
      </nav>
      <div class="navigation-note"><span class="pill">开发预览</span><p>当前仅计算规则目标<br>不发送设备输出</p><small v-if="developer.enabled">开发者模式已开启</small></div>
    </aside>
    <main class="main-panel">
      <header class="topbar">
        <div class="global-status"><span class="game-status"><i class="dot" :class="{on:connected}"></i>游戏 {{ connected?'已连接':'未连接' }}<button class="game-connect" :disabled="gameButton.disabled || busy" @click="connectGame()">{{ gameButton.label }}</button></span><span>设备尚未接入</span><span :class="{running:active}">{{ active?'联动已启用':'联动未启用' }}</span></div>
        <button class="stop" @click="action(api.stop,'强度已重置为零，联动已停用')">重置强度</button>
      </header>
      <div class="page-content">
        <div class="page-heading"><div><h2>{{ pageTitle }}</h2><p>{{ page==='control'?'查看游戏状态，配置双通道联动规则':page==='waves'?'浏览官方和导入波形的参数预览':page==='settings'?'本机连接与应用信息':'连接诊断与完整事件记录' }}</p></div><span v-if="dirty" class="unsaved">规则有未保存修改</span></div>
        <div v-if="error || state?.listener_error" class="banner error" role="alert">{{ error || state?.listener_error }}</div>
        <div v-else-if="notice" class="banner notice" role="status">{{ notice }}</div>
        <div v-if="state?.connection.phase==='failed' && !connected" class="banner error" role="alert">{{ state.connection.message }}<p class="hint">诊断日志：%LOCALAPPDATA%/gmod-dglab/collector-diagnostic.log</p></div>
        <div v-else-if="!connected && (state?.connection.phase==='injecting' || state?.connection.phase==='waiting')" class="banner notice" role="status">{{ state.connection.message }}</div>
        <section v-if="state?.connection.phase==='selecting'" class="card game-selection"><h3>选择游戏进程</h3><p class="hint">{{ state.connection.message }}</p><ThemeSelect v-model="selectedGame" :options="gameOptions" label="目标 GMod 进程"/><div class="selection-actions"><button class="secondary" :disabled="busy" @click="action(api.cancelGameSelection,'已取消进程选择');selectedGame=''">取消</button><button class="primary" :disabled="busy || !gameOptions.some(option=>option.value===selectedGame)" @click="connectGame(true)">确认连接</button></div></section>
        <template v-if="page==='control'">
          <section class="card player-card">
            <div class="card-title"><h3>玩家状态</h3><span class="pill">{{ player ? (player.life_state===0?'存活':'死亡 / 过渡状态') : '暂无有效玩家' }}</span></div>
            <div class="health-line"><div><span class="eyebrow">实时血量</span><div class="health-value">{{ player?.health ?? '—' }} <small>/ {{ state?.controller.max_health ?? '—' }}</small></div></div><div class="armor"><span class="eyebrow">护甲</span><strong>{{ player?.armor ?? '不可用' }}</strong></div></div>
            <div class="health-track"><div :style="{width:`${healthPercent}%`}"></div></div>
            <p class="hint">{{ player && !state?.controller.max_health ? '最大生命值不可用：缺失生命值模式和百分比映射暂不计算。' : '按客户端血量变化计算，可能合并多次伤害。' }}</p>
          </section>
          <section class="card mapping">
            <div class="card-title"><h3>通道强度</h3><span class="hint">目标强度仅用于预览</span></div>
            <div class="channel-grid">
              <div v-for="(channel,i) in draft.channels" :key="i" class="channel">
                <div class="channel-heading"><h4><span class="channel-letter">{{ i===0?'A':'B' }}</span>通道</h4><label class="switch-label"><input type="checkbox" v-model="channel.enabled">参与联动</label></div>
                <div class="target"><span>当前目标</span><strong>{{ state?.controller.targets[i] ?? 0 }}</strong></div>
                <div class="field-pair"><label>基础强度<input type="number" min="0" max="200" step="1" v-model.number="channel.base_strength"></label><label>强度上限<input type="number" min="0" max="200" step="1" v-model.number="channel.max_strength"></label></div>
                <div class="field mapping-field"><label :for="`mapping-rate-${i}`">映射率</label><div class="mapping-controls"><input :id="`mapping-rate-${i}`" :aria-label="`${i===0?'A':'B'} 通道映射率`" type="number" min="0" max="200" step="0.1" v-model.number="channel.mapping_rate"><ThemeSelect v-model="channel.mapping_unit" :options="mappingUnits" :label="`${i===0?'A':'B'} 通道映射单位`"/></div></div>
                <label class="field">当前波形<ThemeSelect v-model="channel.waveform" :options="waveformOptions" :label="`${i===0?'A':'B'} 通道当前波形`"/></label>
                <svg class="wave channel-wave" viewBox="0 0 300 80" role="img" :aria-label="`${i===0?'A':'B'} 通道当前波形预览`"><path class="gridline" d="M0 20H300M0 45H300M0 70H300"/><polyline class="frequency-line" :points="frequencyPoints(channel.waveform)"/><polyline :points="points(channel.waveform)"/></svg>
                <p class="hint">{{ waveHint(channel.waveform) }}</p>
              </div>
            </div>
          </section>
          <section class="card rules">
            <div class="card-title"><h3>联动规则</h3></div>
            <div class="setting-row"><div><strong>受伤增量模式</strong><p>{{ draft.injury_mode==='health_deficit'?'随存活玩家当前血量持续计算，回血时降低；不使用受伤时长和冷却':'每次受伤累加临时强度并刷新持续时间' }}</p></div><ThemeSelect v-model="draft.injury_mode" :options="injuryModes" label="受伤增量模式"/></div>
            <div class="setting-row"><div><strong>受伤联动</strong><p>{{ draft.injury_mode==='health_deficit'?'此模式持续跟随缺失生命值，不使用计时':'按所选单位累加额外强度，每次受伤刷新持续时间' }}</p></div><label class="row-input">持续时间<input type="number" min="100" max="10000" step="100" v-model.number="draft.duration_ms" :disabled="draft.injury_mode==='health_deficit'"><span>ms</span></label></div>
            <div class="setting-row"><div><strong>受伤冷却</strong><p>{{ draft.injury_mode==='health_deficit'?'此模式不使用受伤冷却':'受伤增量结束后开始，0 秒关闭；不影响死亡联动' }}</p></div><label class="row-input">冷却时间<input type="number" min="0" max="60" step="0.1" :value="draft.injury_cooldown_ms/1000" @input="setCooldown('injury_cooldown_ms',$event)" :disabled="draft.injury_mode==='health_deficit'"><span>秒</span></label></div>
            <div class="setting-row"><div><label class="switch-label"><input type="checkbox" v-model="draft.death_enabled"><strong>死亡联动</strong></label><p>在基础与已有受伤强度上额外增加，最终不超过通道上限</p></div><label class="row-input">额外强度<input type="number" min="0" max="200" v-model.number="draft.death_strength" :disabled="!draft.death_enabled"></label></div>
            <details class="advanced"><summary>死亡联动高级设置</summary><div class="setting-row"><label>作用通道</label><ThemeSelect v-model="draft.death_channels" :disabled="!draft.death_enabled" :options="deathChannelOptions" label="死亡作用通道"/></div><div class="setting-row"><label class="row-input">独立持续时间<input type="number" min="100" max="10000" step="100" v-model.number="draft.death_duration_ms" :disabled="!draft.death_enabled"><span>ms</span></label><label class="row-input">独立冷却<input type="number" min="0" max="60" step="0.1" :value="draft.death_cooldown_ms/1000" @input="setCooldown('death_cooldown_ms',$event)" :disabled="!draft.death_enabled"><span>秒</span></label></div></details>
            <p class="hint">基础强度持续生效 · 临时增量独立计时与冷却 · 停止或失联归零</p>
          </section>
          <section class="card recent-events"><div class="card-title"><h3>最近事件</h3><button v-if="developer.enabled" class="text-button" @click="page='developer'">完整日志 ↗</button></div><p v-if="!logs.length" class="hint">等待游戏事件</p><div v-for="entry in logs.slice(0,3)" :key="entry.id" class="compact-log"><time>{{ time(entry.at_ms) }}</time><span>{{ entry.text }}</span></div></section>
        </template>
        <template v-else-if="page==='waves'">
          <div class="wave-browser">
            <section class="card wave-library"><div class="card-title"><h3>波形库</h3><span class="pill">{{ waveforms.length }} 种</span></div><input v-model="search" aria-label="搜索波形" placeholder="搜索名称或来源"><div class="wave-list"><button v-for="w in visibleWaves" :key="w.id" :class="{selected:w.id===selectedWave}" @click="selectedWave=w.id"><strong>{{ w.name }}</strong><small>{{ w.source }}</small></button><p v-if="!visibleWaves.length" class="hint">没有匹配的波形</p></div><label class="field import-control">导入波形 JSON<input type="file" accept=".json,application/json" :disabled="busy || !loaded" @change="importFile"></label></section>
            <section class="card wave-detail"><div class="card-title"><h3>{{ selected?.name ?? '请选择波形' }}</h3><span v-if="selected" class="pill">{{ selected.source }}</span></div><svg class="wave" viewBox="0 0 300 80" role="img" aria-label="选中波形的参数预览"><path class="gridline" d="M0 20H300M0 45H300M0 70H300"/><polyline class="frequency-line" :points="frequencyPoints(selectedWave)"/><polyline :points="points(selectedWave)"/></svg><div class="wave-legend"><span>— 相对幅度</span><span>┄ 频率编码</span></div><p class="hint">{{ selected ? waveHint(selectedWave) : '波形库加载后可选择预览' }}</p><p class="hint">曲线表示波形参数，并非设备真实电流测量。</p></section>
          </div>
        </template>
        <template v-else-if="page==='settings'">
          <section class="card"><div class="card-title"><h3>本机连接</h3></div><div class="setting-row"><div><strong>监听地址</strong><p>仅支持本机回环地址；旧诊断接收器需先关闭</p></div><div class="address-editor"><input v-model="address" aria-label="监听地址" placeholder="127.0.0.1:47831"><button class="secondary" :disabled="busy || !loaded" @click="applyAddress">应用</button></div></div><p class="hint">当前监听：{{ state?.listen_address ?? '—' }}</p></section>
          <section class="card"><div class="card-title"><h3>关于应用</h3></div><div class="setting-row"><div><strong>GMod × 郊狼</strong><p>Windows x64 · 开发预览</p></div><button class="version-button" @click="unlockDeveloper">v0.1.0</button></div><div class="setting-row"><div><strong>开源许可</strong><p>GPL-3.0</p></div><span class="hint">官方波形数据随附许可说明</span></div><p class="hint">郊狼 APP 与硬件连接尚未接入，当前所有目标均为规则预览。</p></section>
          <section v-if="developer.enabled" class="card"><div class="setting-row"><div><strong>开发者模式</strong><p>仅本次运行有效，重启后恢复隐藏</p></div><button class="secondary" @click="exitDeveloper">退出开发者模式</button></div></section>
        </template>
        <template v-else-if="page==='developer' && developer.enabled">
          <section class="card"><div class="card-title"><h3>连接诊断</h3><button class="text-button" @click="exitDeveloper">退出开发者模式</button></div><dl class="diagnostic-grid"><div><dt>监听地址</dt><dd>{{ state?.listen_address ?? '—' }}</dd></div><div><dt>游戏连接</dt><dd>{{ connected?'已连接':'未连接' }}</dd></div><div><dt>游戏 PID</dt><dd>{{ state?.game_process_id ?? (connected?'旧版 DLL 未提供 PID':'—') }}</dd></div><div><dt>最近心跳</dt><dd>{{ state?.heartbeat_age_ms != null ? `${(state.heartbeat_age_ms/1000).toFixed(1)} 秒前` : '—' }}</dd></div><div><dt>受伤增量 / 冷却剩余</dt><dd>{{ state?.controller.injury_remaining_ms ?? 0 }} / {{ state?.controller.injury_cooldown_remaining_ms ?? 0 }} ms</dd></div><div><dt>死亡增量 / 冷却剩余</dt><dd>{{ state?.controller.death_remaining_ms ?? 0 }} / {{ state?.controller.death_cooldown_remaining_ms ?? 0 }} ms</dd></div></dl></section>
          <section class="card event-log"><div class="card-title"><h3>完整事件日志</h3><span class="pill">{{ logs.length }} / 200</span></div><div class="log-content"><p v-if="!logs.length" class="hint">暂无事件</p><div v-for="entry in logs" :key="entry.id" class="log-entry"><time>{{ time(entry.at_ms) }}</time><p>{{ entry.text }}</p></div></div></section>
        </template>
      </div>
      <div v-if="page==='control'" class="actionbar"><div><p v-if="validation" class="validation" role="alert">{{ validation }}</p><strong v-else>{{ dirty?'规则有未保存修改':active?'规则联动已启用':'联动已停用' }}</strong><small>受伤冷却 {{ ((state?.controller.injury_cooldown_remaining_ms ?? 0)/1000).toFixed(1) }} 秒 · 死亡冷却 {{ ((state?.controller.death_cooldown_remaining_ms ?? 0)/1000).toFixed(1) }} 秒</small></div><div class="action-buttons"><button class="secondary" :disabled="busy || !loaded || !!validation" @click="save">保存规则</button><button class="primary" :disabled="busy || (!active && !canEnable)" @click="togglePreview">{{ active?'停用联动':'启用联动' }}</button></div></div>
    </main>
  </div>
</template>

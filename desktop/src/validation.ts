export interface ChannelConfig { enabled: boolean; base_strength: number; max_strength: number; waveform: string; mapping_rate: number; mapping_unit:'points'|'percent' }
export interface RuleConfig { channels: ChannelConfig[]; injury_mode:'accumulate'|'health_deficit'; duration_ms: number; death_enabled: boolean; death_strength: number; injury_cooldown_ms:number; death_duration_ms:number; death_cooldown_ms:number; death_channels:'all'|'a'|'b' }
export const defaultConfig = (): RuleConfig => ({ channels: ['official:breathing','official:breathing'].map(waveform => ({enabled:false,base_strength:0,max_strength:0,waveform,mapping_rate:1,mapping_unit:'points'})), injury_mode:'accumulate',duration_ms:300, death_enabled:false, death_strength:0,injury_cooldown_ms:1000,death_duration_ms:300,death_cooldown_ms:1000,death_channels:'all' });
export const copyConfig = (config: RuleConfig): RuleConfig => JSON.parse(JSON.stringify(config));
export function validateConfig(config: RuleConfig): string | null {
  if (!['accumulate','health_deficit'].includes(config.injury_mode)) return '请选择受伤强度模式';
  if (config.channels.length !== 2) return '需要 A/B 两个通道';
  const integer = (v: number, min: number, max: number) => Number.isInteger(v) && v >= min && v <= max;
  if (!integer(config.duration_ms,100,10000) || !integer(config.death_duration_ms,100,10000)) return '持续时间必须是 100～10000 毫秒的整数';
  if (!integer(config.injury_cooldown_ms,0,60000) || !integer(config.death_cooldown_ms,0,60000)) return '冷却时间必须在 0～60 秒之间';
  if (!['all','a','b'].includes(config.death_channels)) return '请选择死亡联动的通道';
  if (!integer(config.death_strength,0,200)) return '死亡额外强度必须是 0～200 的整数';
  for (const c of config.channels) {
    if (!['points','percent'].includes(c.mapping_unit)) return '请选择映射率的单位';
    if (!Number.isFinite(c.mapping_rate) || c.mapping_rate<0 || c.mapping_rate>200) return '映射率必须是 0～200 之间的数值';
    if (!integer(c.base_strength,0,200) || !integer(c.max_strength,0,200) || c.base_strength > c.max_strength) return '强度必须是 0～200 的整数，基础强度不能超过强度上限';
    if (!/^official:[a-z_]+$/.test(c.waveform) && !/^user:[a-f0-9]{64}$/.test(c.waveform)) return '请选择波形';
  }
  return null;
}

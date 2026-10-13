use gmod_dglab_common::tracker::{GameEvent, PlayerSample};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

fn default_mapping_rate() -> f64 {
    1.0
}
fn default_duration() -> u64 {
    300
}
fn default_cooldown() -> u64 {
    1000
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InjuryMode {
    #[default]
    Accumulate,
    HealthDeficit,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingUnit {
    #[default]
    Points,
    Percent,
}
fn migrate_waveform<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let id = String::deserialize(deserializer)?;
    Ok(if matches!(id.as_str(), "steady" | "fade" | "double") {
        crate::waveforms::DEFAULT_WAVEFORM.into()
    } else {
        id
    })
}
fn mapped_amount(amount: f64, unit: MappingUnit, max_health: Option<i32>) -> Option<f64> {
    match unit {
        MappingUnit::Points => Some(amount),
        MappingUnit::Percent => max_health
            .filter(|max| *max > 0)
            .map(|max| amount / f64::from(max) * 100.0),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    pub enabled: bool,
    #[serde(alias = "min_strength")]
    pub base_strength: u16,
    pub max_strength: u16,
    #[serde(deserialize_with = "migrate_waveform")]
    pub waveform: String,
    #[serde(default = "default_mapping_rate")]
    pub mapping_rate: f64,
    #[serde(default)]
    pub mapping_unit: MappingUnit,
}
impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_strength: 0,
            max_strength: 0,
            waveform: crate::waveforms::DEFAULT_WAVEFORM.into(),
            mapping_rate: 1.0,
            mapping_unit: MappingUnit::Points,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeathChannels {
    #[default]
    All,
    A,
    B,
}
impl DeathChannels {
    fn includes(self, index: usize) -> bool {
        matches!(self, Self::All) || matches!((self, index), (Self::A, 0) | (Self::B, 1))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    pub channels: [ChannelConfig; 2],
    #[serde(default)]
    pub injury_mode: InjuryMode,
    pub duration_ms: u64,
    pub death_enabled: bool,
    pub death_strength: u16,
    #[serde(default = "default_cooldown")]
    pub injury_cooldown_ms: u64,
    #[serde(default = "default_duration")]
    pub death_duration_ms: u64,
    #[serde(default = "default_cooldown")]
    pub death_cooldown_ms: u64,
    #[serde(default)]
    pub death_channels: DeathChannels,
}
impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            channels: [ChannelConfig::default(), ChannelConfig::default()],
            injury_mode: InjuryMode::Accumulate,
            duration_ms: 300,
            death_enabled: false,
            death_strength: 0,
            injury_cooldown_ms: 1000,
            death_duration_ms: 300,
            death_cooldown_ms: 1000,
            death_channels: DeathChannels::All,
        }
    }
}
impl RuleConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(100..=10000).contains(&self.duration_ms)
            || !(100..=10000).contains(&self.death_duration_ms)
        {
            return Err("受伤与死亡持续时间须为 100..10000 ms".into());
        }
        if self.injury_cooldown_ms > 60000 || self.death_cooldown_ms > 60000 {
            return Err("冷却时间须为 0..60000 ms，0 表示关闭".into());
        }
        if self.death_strength > 200 {
            return Err("死亡额外强度须为 0..200".into());
        }
        for c in &self.channels {
            if c.base_strength > c.max_strength || c.max_strength > 200 {
                return Err("基础强度不能超过上限，上限最多 200".into());
            }
            if !c.mapping_rate.is_finite() || !(0.0..=200.0).contains(&c.mapping_rate) {
                return Err("映射率须为 0..200 的有限数值".into());
            }
            if !crate::waveforms::known_builtin_id(&c.waveform)
                && !crate::waveforms::valid_user_id(&c.waveform)
            {
                return Err("未知波形".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub at_ms: u64,
    pub text: String,
}
#[derive(Clone, Serialize)]
pub struct ControllerSnapshot {
    pub config: RuleConfig,
    pub preview_enabled: bool,
    pub player: Option<PlayerSample>,
    pub max_health: Option<i32>,
    pub targets: [u16; 2],
    pub injury_remaining_ms: u64,
    pub death_remaining_ms: u64,
    pub injury_cooldown_remaining_ms: u64,
    pub death_cooldown_remaining_ms: u64,
    pub logs: Vec<LogEntry>,
}
#[derive(Default)]
struct Effect {
    extra: [f64; 2],
    until: Option<u64>,
    cooldown_until: u64,
}
impl Effect {
    fn reset(&mut self) {
        *self = Self::default();
    }
    fn expire(&mut self, now: u64, cooldown: u64) -> bool {
        if let Some(until) = self.until {
            if now >= until {
                self.extra = [0.0; 2];
                self.until = None;
                self.cooldown_until = until.saturating_add(cooldown);
                return true;
            }
        }
        false
    }
    fn cancel(&mut self, now: u64, cooldown: u64) {
        if self.until.is_some() {
            self.extra = [0.0; 2];
            self.until = None;
            self.cooldown_until = now.saturating_add(cooldown);
        }
    }
    fn remaining(&self, now: u64) -> u64 {
        self.until.map_or(0, |until| until.saturating_sub(now))
    }
}
pub struct ControllerCore {
    config: RuleConfig,
    enabled: bool,
    player: Option<PlayerSample>,
    max_health: Option<i32>,
    injury: Effect,
    death: Effect,
    targets: [u16; 2],
    now: u64,
    logs: VecDeque<LogEntry>,
    log_id: u64,
}
impl ControllerCore {
    pub fn new(config: RuleConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            config,
            enabled: false,
            player: None,
            max_health: None,
            injury: Effect::default(),
            death: Effect::default(),
            targets: [0, 0],
            now: 0,
            logs: VecDeque::new(),
            log_id: 0,
        })
    }
    pub fn note(&mut self, text: impl Into<String>) {
        self.log_id += 1;
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(LogEntry {
            id: self.log_id,
            at_ms: self.now,
            text: text.into(),
        });
    }
    pub fn configure(&mut self, config: RuleConfig) -> Result<(), String> {
        config.validate()?;
        if self.config.injury_mode != config.injury_mode {
            self.injury.reset();
        }
        for i in 0..2 {
            if self.config.channels[i].mapping_unit != config.channels[i].mapping_unit {
                self.injury.extra[i] = 0.0;
            }
        }
        self.config = config;
        self.refresh();
        self.note("规则配置已保存");
        Ok(())
    }
    pub fn set_preview(&mut self, enabled: bool, now_ms: u64) -> Result<(), String> {
        self.tick(now_ms);
        if enabled && !self.player.is_some_and(|p| p.life_state == 0) {
            return Err("需要有效且存活的本地玩家才能启用预览".into());
        }
        self.enabled = enabled;
        if !enabled {
            self.clear_effects();
        }
        self.refresh();
        self.note(if enabled {
            "规则预览已启用（基础强度生效，不发送设备输出）"
        } else {
            "规则预览已停用"
        });
        Ok(())
    }
    fn clear_effects(&mut self) {
        self.injury.reset();
        self.death.reset();
    }
    pub fn stop(&mut self) {
        self.enabled = false;
        self.clear_effects();
        self.refresh();
        self.note("已停止预览，目标归零");
    }
    pub fn disconnect(&mut self) {
        self.stop();
        self.player = None;
        self.max_health = None;
    }
    fn refresh(&mut self) {
        for i in 0..2 {
            let c = &self.config.channels[i];
            let headroom = f64::from(c.max_strength - c.base_strength);
            if !c.enabled {
                self.injury.extra[i] = 0.0;
                self.death.extra[i] = 0.0;
            }
            self.injury.extra[i] = self.injury.extra[i].min(headroom);
            if !self.config.death_enabled || !self.config.death_channels.includes(i) {
                self.death.extra[i] = 0.0;
            }
            self.death.extra[i] = self.death.extra[i].min(headroom);
            let injury_extra = if self.config.injury_mode == InjuryMode::HealthDeficit {
                match (self.player.filter(|p| p.life_state == 0), self.max_health) {
                    (Some(player), Some(max)) => {
                        let deficit = (i64::from(max) - i64::from(player.health))
                            .clamp(0, i64::from(max)) as f64;
                        mapped_amount(deficit, c.mapping_unit, Some(max)).unwrap_or(0.0)
                            * c.mapping_rate
                    }
                    _ => 0.0,
                }
            } else {
                self.injury.extra[i]
            };
            self.targets[i] = if self.enabled && self.player.is_some() && c.enabled {
                (f64::from(c.base_strength) + injury_extra + self.death.extra[i])
                    .round()
                    .min(f64::from(c.max_strength)) as u16
            } else {
                0
            };
        }
        // 规则修改导致增量被全部清空时，不留下可复活的旧任务。
        if self.injury.extra == [0.0; 2] {
            self.injury.until = None;
        }
        if self.death.extra == [0.0; 2] {
            self.death.until = None;
        }
    }
    pub fn tick(&mut self, now_ms: u64) {
        if now_ms < self.now {
            self.stop();
            self.note("时钟回退，已停用预览");
        }
        self.now = now_ms;
        if self.injury.expire(now_ms, self.config.injury_cooldown_ms) {
            self.note("受伤增量到期，进入受伤冷却");
        }
        if self.death.expire(now_ms, self.config.death_cooldown_ms) {
            self.note("死亡增量到期，进入死亡冷却");
        }
        self.refresh();
    }
    fn log_target(&mut self) {
        self.note(format!(
            "预览目标：A {} / B {}",
            self.targets[0], self.targets[1]
        ));
    }
    pub fn handle(&mut self, event: GameEvent, now_ms: u64) {
        self.tick(now_ms);
        match event {
            GameEvent::PlayerState {
                sample, max_health, ..
            } => {
                if self.player.map(|p| p.entity_key) != sample.map(|p| p.entity_key) {
                    self.clear_effects();
                }
                if sample.is_none() {
                    self.disconnect();
                }
                self.player = sample;
                self.max_health = max_health.filter(|v| *v > 0);
                self.refresh();
            }
            GameEvent::HealthDecreased {
                before,
                after,
                death_priority,
            } => {
                self.note(format!(
                    "掉血：{before} → {after}{}",
                    if death_priority {
                        "（死亡优先，致命掉血不重复计入受伤增量）"
                    } else {
                        ""
                    }
                ));
                if death_priority
                    || !self.enabled
                    || !self.player.is_some_and(|p| p.life_state == 0)
                {
                    return;
                }
                if self.config.injury_mode == InjuryMode::HealthDeficit {
                    self.refresh();
                    self.log_target();
                    return;
                }
                if self.now < self.injury.cooldown_until {
                    self.note("受伤冷却中，本次掉血不增加或刷新强度");
                    return;
                }
                let loss = (i64::from(before) - i64::from(after)).max(0) as f64;
                if loss == 0.0 {
                    return;
                }
                let mut mapped = false;
                let mut missing_max = false;
                for (i, c) in self.config.channels.iter().enumerate() {
                    if c.enabled && c.mapping_rate > 0.0 && c.max_strength > c.base_strength {
                        if let Some(amount) = mapped_amount(loss, c.mapping_unit, self.max_health) {
                            self.injury.extra[i] = (self.injury.extra[i] + amount * c.mapping_rate)
                                .min(f64::from(c.max_strength - c.base_strength));
                            mapped = true;
                        } else {
                            missing_max = true;
                        }
                    }
                }
                if missing_max {
                    self.note("最大生命值不可用，跳过百分比映射通道");
                }
                if mapped && self.injury.extra.iter().any(|v| *v > 0.0) {
                    self.injury.until = Some(self.now.saturating_add(self.config.duration_ms));
                }
                self.refresh();
                self.log_target();
            }
            GameEvent::Died => {
                self.note("死亡：保留已有受伤增量，计算独立死亡增量");
                if !self.enabled || !self.config.death_enabled || self.player.is_none() {
                    return;
                }
                if self.now < self.death.cooldown_until {
                    self.note("死亡冷却中，本次死亡不增加或刷新强度");
                    return;
                }
                for (i, c) in self.config.channels.iter().enumerate() {
                    if c.enabled && self.config.death_channels.includes(i) {
                        self.death.extra[i] = f64::from(
                            self.config
                                .death_strength
                                .min(c.max_strength - c.base_strength),
                        );
                    }
                }
                if self.death.extra.iter().any(|v| *v > 0.0) {
                    self.death.until = Some(self.now.saturating_add(self.config.death_duration_ms));
                }
                self.refresh();
                self.log_target();
            }
            GameEvent::Respawned => {
                self.injury.cancel(self.now, self.config.injury_cooldown_ms);
                self.death.cancel(self.now, self.config.death_cooldown_ms);
                self.refresh();
                self.note("重生：清空临时增量，按当前模式重新计算；独立冷却继续计时");
            }
            GameEvent::ArmorDecreased { before, after } => {
                self.note(format!("护甲减少：{before} → {after}（暂不参与规则）"));
            }
        }
    }
    pub fn snapshot(&self) -> ControllerSnapshot {
        ControllerSnapshot {
            config: self.config.clone(),
            preview_enabled: self.enabled,
            player: self.player,
            max_health: self.max_health,
            targets: self.targets,
            injury_remaining_ms: self.injury.remaining(self.now),
            death_remaining_ms: self.death.remaining(self.now),
            injury_cooldown_remaining_ms: self.injury.cooldown_until.saturating_sub(self.now),
            death_cooldown_remaining_ms: self.death.cooldown_until.saturating_sub(self.now),
            logs: self.logs.iter().cloned().collect(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gmod_dglab_common::tracker::PlayerSample;
    fn state(max_health: Option<i32>) -> GameEvent {
        GameEvent::PlayerState {
            sample: Some(PlayerSample {
                entity_key: 1,
                health: 100,
                armor: None,
                life_state: 0,
            }),
            max_health,
            thread_id: 1,
        }
    }
    fn core() -> ControllerCore {
        let mut config = RuleConfig::default();
        config.channels[0].enabled = true;
        config.channels[0].base_strength = 10;
        config.channels[0].max_strength = 50;
        let mut core = ControllerCore::new(config).unwrap();
        core.handle(state(Some(100)), 0);
        core.set_preview(true, 0).unwrap();
        core
    }
    fn hurt(c: &mut ControllerCore, loss: i32, at: u64) {
        c.handle(
            GameEvent::HealthDecreased {
                before: 100,
                after: 100 - loss,
                death_priority: false,
            },
            at,
        );
    }
    #[test]
    fn damage_accumulates_refreshes_and_returns_to_base() {
        let mut c = core();
        hurt(&mut c, 5, 0);
        hurt(&mut c, 3, 200);
        assert_eq!(c.snapshot().targets, [18, 0]);
        c.tick(300);
        assert_eq!(c.snapshot().targets, [18, 0]);
        c.tick(500);
        assert_eq!(c.snapshot().targets, [10, 0]);
    }
    #[test]
    fn absolute_damage_does_not_require_maximum_health() {
        let mut c = core();
        c.handle(state(None), 0);
        hurt(&mut c, 5, 0);
        assert_eq!(c.snapshot().targets, [15, 0]);
    }
    #[test]
    fn death_adds_to_existing_injury_instead_of_replacing_it() {
        let mut c = core();
        let mut config = c.snapshot().config;
        config.death_enabled = true;
        config.death_strength = 30;
        c.configure(config).unwrap();
        hurt(&mut c, 5, 0);
        c.handle(GameEvent::Died, 100);
        assert_eq!(c.snapshot().targets, [45, 0]);
        c.tick(300);
        assert_eq!(c.snapshot().targets, [40, 0]);
    }
    #[test]
    fn no_legacy_five_second_cutoff() {
        let mut c = core();
        for now in (0..7000).step_by(100) {
            hurt(&mut c, 1, now);
        }
        assert_eq!(c.snapshot().targets, [50, 0]);
    }
    #[test]
    fn cap_disable_and_stop_are_immediate() {
        let mut c = core();
        hurt(&mut c, i32::MAX, 0);
        assert_eq!(c.snapshot().targets, [50, 0]);
        let mut cfg = c.snapshot().config;
        cfg.channels[0].max_strength = 20;
        c.configure(cfg.clone()).unwrap();
        assert_eq!(c.snapshot().targets, [20, 0]);
        cfg.channels[0].enabled = false;
        c.configure(cfg).unwrap();
        assert_eq!(c.snapshot().targets, [0, 0]);
        c.stop();
        assert!(!c.snapshot().preview_enabled);
    }
    #[test]
    fn respawn_resets_to_base_and_disconnect_requires_manual_enable() {
        let mut c = core();
        hurt(&mut c, 5, 0);
        c.handle(GameEvent::Respawned, 1);
        assert_eq!(c.snapshot().targets, [10, 0]);
        assert!(c.snapshot().preview_enabled);
        c.disconnect();
        assert_eq!(c.snapshot().targets, [0, 0]);
        c.handle(state(Some(100)), 2);
        assert!(!c.snapshot().preview_enabled);
    }

    #[test]
    fn fractional_rates_are_accumulated_before_rounding() {
        let mut c = core();
        let mut cfg = c.snapshot().config;
        cfg.channels[0].mapping_rate = 0.25;
        cfg.channels[1].enabled = true;
        cfg.channels[1].max_strength = 100;
        cfg.channels[1].mapping_rate = 2.0;
        c.configure(cfg).unwrap();
        for at in 0..4 {
            hurt(&mut c, 1, at);
        }
        assert_eq!(c.snapshot().targets, [11, 8]);
    }
    #[test]
    fn independent_durations_cooldowns_and_channel_selection() {
        let mut c = core();
        let mut cfg = c.snapshot().config;
        cfg.channels[1].enabled = true;
        cfg.channels[1].base_strength = 5;
        cfg.channels[1].max_strength = 100;
        cfg.duration_ms = 1000;
        cfg.injury_cooldown_ms = 2000;
        cfg.death_enabled = true;
        cfg.death_strength = 20;
        cfg.death_duration_ms = 200;
        cfg.death_cooldown_ms = 500;
        cfg.death_channels = DeathChannels::B;
        c.configure(cfg).unwrap();
        hurt(&mut c, 5, 0);
        c.handle(GameEvent::Died, 100);
        assert_eq!(c.snapshot().targets, [15, 30]);
        c.tick(300);
        assert_eq!(c.snapshot().targets, [15, 10]);
        assert_eq!(c.snapshot().death_cooldown_remaining_ms, 500);
        hurt(&mut c, 2, 400);
        assert_eq!(c.snapshot().targets, [17, 12]);
        assert_eq!(c.snapshot().injury_remaining_ms, 1000);
        c.handle(GameEvent::Died, 600);
        assert_eq!(c.snapshot().targets, [17, 12]);
        c.handle(GameEvent::Died, 800);
        assert_eq!(c.snapshot().targets, [17, 32]);
        c.tick(1400);
        assert_eq!(c.snapshot().targets, [10, 5]);
        hurt(&mut c, 9, 1500);
        assert_eq!(c.snapshot().targets, [10, 5]);
        c.handle(GameEvent::Died, 1500);
        assert_eq!(c.snapshot().targets, [10, 25]);
        c.tick(3400);
        hurt(&mut c, 1, 3400);
        assert_eq!(c.snapshot().targets, [11, 6]);
    }
    #[test]
    fn late_ticks_do_not_extend_cooldown_and_zero_disables_it() {
        let mut c = core();
        hurt(&mut c, 5, 0);
        c.tick(2000);
        assert_eq!(c.snapshot().injury_cooldown_remaining_ms, 0);
        hurt(&mut c, 2, 2000);
        assert_eq!(c.snapshot().targets, [12, 0]);
        let mut cfg = c.snapshot().config;
        cfg.injury_cooldown_ms = 0;
        c.configure(cfg).unwrap();
        c.tick(2300);
        hurt(&mut c, 2, 2300);
        assert_eq!(c.snapshot().targets, [12, 0]);
    }
    #[test]
    fn respawn_does_not_bypass_death_cooldown() {
        let mut c = core();
        let mut cfg = c.snapshot().config;
        cfg.death_enabled = true;
        cfg.death_strength = 20;
        cfg.death_cooldown_ms = 1000;
        c.configure(cfg).unwrap();
        c.handle(GameEvent::Died, 0);
        c.tick(300);
        c.handle(GameEvent::Respawned, 400);
        c.handle(GameEvent::Died, 500);
        assert_eq!(c.snapshot().targets, [10, 0]);
        assert_eq!(c.snapshot().death_cooldown_remaining_ms, 800);
    }
    #[test]
    fn old_settings_migrate_without_losing_limits_or_waveforms() {
        let cfg:RuleConfig=serde_json::from_str(r#"{"channels":[{"enabled":true,"min_strength":10,"max_strength":50,"waveform":"steady"},{"enabled":false,"min_strength":0,"max_strength":100,"waveform":"fade"}],"duration_ms":2000,"death_enabled":true,"death_strength":20}"#).unwrap();
        assert_eq!(cfg.channels[0].base_strength, 10);
        assert_eq!(cfg.channels[0].mapping_rate, 1.0);
        assert_eq!(cfg.death_channels, DeathChannels::All);
        cfg.validate().unwrap();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("min_strength"));
        assert!(json.contains("base_strength"));
    }
    #[test]
    fn boundaries_validation_clock_rollback_and_event_burst() {
        let mut cfg = RuleConfig {
            duration_ms: 10000,
            death_duration_ms: 10000,
            ..RuleConfig::default()
        };
        cfg.validate().unwrap();
        cfg.duration_ms = 10001;
        assert!(cfg.validate().is_err());
        cfg.duration_ms = 100;
        cfg.death_duration_ms = 99;
        assert!(cfg.validate().is_err());
        cfg.death_duration_ms = 100;
        cfg.injury_cooldown_ms = 60001;
        assert!(cfg.validate().is_err());
        cfg.injury_cooldown_ms = 0;
        for rate in [-1.0, f64::INFINITY, f64::NAN, 201.0] {
            cfg.channels[0].mapping_rate = rate;
            assert!(cfg.validate().is_err());
        }
        let mut c = core();
        for _ in 0..10000 {
            hurt(&mut c, 1, 100);
        }
        assert_eq!(c.snapshot().logs.len(), 200);
        c.tick(99);
        assert!(!c.snapshot().preview_enabled);
        assert_eq!(c.snapshot().targets, [0, 0]);
    }

    fn with_new_options(c: &mut ControllerCore, mode: &str, unit: &str) {
        let mut value = serde_json::to_value(c.snapshot().config).unwrap();
        value["injury_mode"] = mode.into();
        value["channels"][0]["mapping_unit"] = unit.into();
        value["channels"][0]["mapping_rate"] = 2.0.into();
        value["channels"][0]["max_strength"] = 200.into();
        let config = serde_json::from_value(value).expect("new mode and unit deserialize");
        c.configure(config).unwrap();
    }
    fn sample(
        c: &mut ControllerCore,
        health: i32,
        max_health: Option<i32>,
        life_state: u8,
        at: u64,
    ) {
        c.handle(
            GameEvent::PlayerState {
                sample: Some(PlayerSample {
                    entity_key: 1,
                    health,
                    armor: None,
                    life_state,
                }),
                max_health,
                thread_id: 1,
            },
            at,
        );
    }
    #[test]
    fn deficit_percent_tracks_healing_without_duration_or_cooldown() {
        let mut c = core();
        with_new_options(&mut c, "health_deficit", "percent");
        sample(&mut c, 150, Some(200), 0, 0);
        assert_eq!(c.snapshot().targets, [60, 0]);
        c.tick(10000);
        assert_eq!(c.snapshot().targets, [60, 0]);
        assert_eq!(c.snapshot().injury_remaining_ms, 0);
        assert_eq!(c.snapshot().injury_cooldown_remaining_ms, 0);
        sample(&mut c, 180, Some(200), 0, 10001);
        assert_eq!(c.snapshot().targets, [30, 0]);
        sample(&mut c, 220, Some(200), 0, 10002);
        assert_eq!(c.snapshot().targets, [10, 0]);
    }
    #[test]
    fn deficit_points_skip_invalid_max_and_dead_players() {
        let mut c = core();
        with_new_options(&mut c, "health_deficit", "points");
        sample(&mut c, 75, Some(100), 0, 0);
        assert_eq!(c.snapshot().targets, [60, 0]);
        sample(&mut c, 75, None, 0, 1);
        assert_eq!(c.snapshot().targets, [10, 0]);
        sample(&mut c, 0, Some(100), 1, 2);
        assert_eq!(c.snapshot().targets, [10, 0]);
        c.stop();
        sample(&mut c, 75, Some(100), 0, 3);
        assert_eq!(c.snapshot().targets, [0, 0]);
    }
    #[test]
    fn accumulated_percent_uses_maximum_and_keeps_fractional_losses() {
        let mut c = core();
        with_new_options(&mut c, "accumulate", "percent");
        sample(&mut c, 200, Some(200), 0, 0);
        hurt(&mut c, 10, 0);
        assert_eq!(c.snapshot().targets, [20, 0]);
        hurt(&mut c, 5, 100);
        assert_eq!(c.snapshot().targets, [25, 0]);
        sample(&mut c, 100, None, 0, 101);
        hurt(&mut c, 50, 102);
        assert_eq!(c.snapshot().targets, [25, 0]);
        assert_eq!(c.snapshot().injury_remaining_ms, 298);
    }

    #[test]
    fn mode_switch_clears_old_injury_and_death_uses_its_own_effect() {
        let mut c = core();
        hurt(&mut c, 10, 0);
        with_new_options(&mut c, "health_deficit", "percent");
        sample(&mut c, 150, Some(200), 0, 1);
        assert_eq!(c.snapshot().targets, [60, 0]);
        assert_eq!(c.snapshot().injury_remaining_ms, 0);
        let mut cfg = c.snapshot().config;
        cfg.death_enabled = true;
        cfg.death_strength = 20;
        c.configure(cfg).unwrap();
        sample(&mut c, 0, Some(200), 1, 2);
        c.handle(GameEvent::Died, 2);
        assert_eq!(c.snapshot().targets, [30, 0]);
        c.tick(302);
        assert_eq!(c.snapshot().targets, [10, 0]);
        with_new_options(&mut c, "accumulate", "points");
        sample(&mut c, 100, Some(200), 0, 303);
        assert_eq!(c.snapshot().targets, [10, 0]);
    }
    #[test]
    fn removed_presets_migrate_and_real_official_ids_are_preserved() {
        let mut value = serde_json::to_value(RuleConfig::default()).unwrap();
        value["channels"][0]["waveform"] = "double".into();
        value["channels"][1]["waveform"] = "official:tide".into();
        let cfg: RuleConfig = serde_json::from_value(value).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.channels[0].waveform, "official:breathing");
        assert_eq!(cfg.channels[1].waveform, "official:tide");
    }
}

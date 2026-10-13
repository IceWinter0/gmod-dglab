use super::{DeviceCommand, DeviceMode, DevicePhase, DeviceSnapshot};
use crate::rules::ChannelConfig;
use std::sync::Arc;
pub struct OutputInput {
    pub game_connected: bool,
    pub enabled: bool,
    pub strengths: [u16; 2],
    pub channels: [ChannelConfig; 2],
    pub frames: [Arc<[String]>; 2],
}
#[derive(Default)]
struct Channel {
    active: bool,
    wave: String,
    cursor: usize,
    next_ms: u64,
    ready_ms: u64,
    last_strength: Option<u16>,
    epoch_ms: u64,
}
pub struct OutputScheduler {
    mode: DeviceMode,
    channels: [Channel; 2],
    last_ms: Option<u64>,
}
impl OutputScheduler {
    pub fn new(mode: DeviceMode) -> Self {
        Self {
            mode,
            channels: Default::default(),
            last_ms: None,
        }
    }
    pub fn reset(&mut self) -> Vec<DeviceCommand> {
        self.channels = Default::default();
        self.last_ms = None;
        vec![DeviceCommand::ResetAll]
    }
    pub fn tick(
        &mut self,
        now: u64,
        input: &OutputInput,
        device: &DeviceSnapshot,
    ) -> Result<Vec<DeviceCommand>, String> {
        let was_active = self.channels.iter().any(|c| c.active);
        if !input.enabled
            || !input.game_connected
            || device.phase != DevicePhase::Ready
            || !input.channels.iter().any(|channel| channel.enabled)
        {
            return Ok(if was_active { self.reset() } else { vec![] });
        }
        if self
            .last_ms
            .is_some_and(|last| now < last || now.saturating_sub(last) > 1000)
        {
            self.reset();
            return Err("输出调度延迟，已停用输出".into());
        }
        self.last_ms = Some(now);
        for (i, channel) in input.channels.iter().enumerate() {
            if channel.enabled && device.limits[i].is_none() {
                return Err("尚未取得 APP 强度上限".into());
            }
        }
        let mut commands = if !was_active {
            vec![DeviceCommand::ResetAll]
        } else {
            vec![]
        };
        for (i, c) in self.channels.iter_mut().enumerate() {
            let config = &input.channels[i];
            if !config.enabled {
                if c.active {
                    commands.push(DeviceCommand::Clear { channel: i });
                    commands.push(DeviceCommand::SetStrength {
                        channel: i,
                        value: 0,
                    });
                    *c = Channel::default();
                }
                continue;
            }
            let limit = device.limits[i].ok_or("尚未取得 APP 强度上限")?;
            let strength = input.strengths[i]
                .min(config.max_strength)
                .min(limit)
                .min(200);
            let new_wave = !c.active || c.wave != config.waveform;
            if new_wave {
                if c.active {
                    commands.push(DeviceCommand::Clear { channel: i });
                }
                *c = Channel {
                    active: true,
                    wave: config.waveform.clone(),
                    ready_ms: now + 100,
                    next_ms: now + 100,
                    epoch_ms: now + 100,
                    ..Default::default()
                };
            }
            if c.last_strength != Some(strength)
                || (self.mode == DeviceMode::App4 && now >= c.next_ms)
            {
                commands.push(DeviceCommand::SetStrength {
                    channel: i,
                    value: strength,
                });
                c.last_strength = Some(strength);
            }
            if now >= c.ready_ms && now >= c.next_ms {
                let frames = &input.frames[i];
                if frames.is_empty() {
                    return Err("所选波形尚未加载".into());
                }
                let count = if self.mode == DeviceMode::App4 || c.cursor == 0 {
                    20
                } else {
                    10
                };
                if self.mode == DeviceMode::App4 {
                    c.cursor = ((now - c.epoch_ms) / 100) as usize;
                }
                let batch = (0..count)
                    .map(|offset| frames[(c.cursor + offset) % frames.len()].clone())
                    .collect();
                commands.push(DeviceCommand::Pulse {
                    channel: i,
                    frames: batch,
                    duration_ms: 2000,
                });
                c.cursor = c.cursor.saturating_add(count);
                c.next_ms = now + 1000;
            }
        }
        Ok(commands)
    }
}
#[cfg(test)]
mod tests {
    use super::super::{DeviceCommand, DeviceMode, DevicePhase, DeviceSnapshot};
    use super::*;
    use crate::rules::ChannelConfig;
    use std::sync::Arc;
    fn input() -> OutputInput {
        OutputInput {
            game_connected: true,
            enabled: true,
            strengths: [100, 0],
            channels: [
                ChannelConfig {
                    enabled: true,
                    max_strength: 80,
                    ..Default::default()
                },
                ChannelConfig::default(),
            ],
            frames: [
                Arc::from(vec!["0A0A0A0A64646464".into()]),
                Arc::from(vec!["0A0A0A0A64646464".into()]),
            ],
        }
    }
    #[test]
    fn unknown_limit_blocks_and_limit_drop_applies_immediately() {
        let mut scheduler = OutputScheduler::new(DeviceMode::App3);
        let input = input();
        let mut device = DeviceSnapshot {
            phase: DevicePhase::Ready,
            ..Default::default()
        };
        assert!(scheduler.tick(0, &input, &device).is_err());
        device.limits = [Some(35), Some(100)];
        assert!(scheduler
            .tick(0, &input, &device)
            .unwrap()
            .iter()
            .any(|c| matches!(
                c,
                DeviceCommand::SetStrength {
                    channel: 0,
                    value: 35
                }
            )));
        device.limits[0] = Some(20);
        assert!(scheduler
            .tick(20, &input, &device)
            .unwrap()
            .iter()
            .any(|c| matches!(
                c,
                DeviceCommand::SetStrength {
                    channel: 0,
                    value: 20
                }
            )));
    }
    #[test]
    fn stop_switch_and_late_tick_never_replay() {
        let mut scheduler = OutputScheduler::new(DeviceMode::App3);
        let mut input = input();
        let device = DeviceSnapshot {
            phase: DevicePhase::Ready,
            limits: [Some(100); 2],
            ..Default::default()
        };
        scheduler.tick(0, &input, &device).unwrap();
        input.enabled = false;
        assert!(matches!(
            scheduler.tick(20, &input, &device).unwrap()[0],
            DeviceCommand::ResetAll
        ));
        assert!(scheduler.tick(40, &input, &device).unwrap().is_empty());
        input.enabled = true;
        scheduler.tick(60, &input, &device).unwrap();
        assert!(scheduler.tick(3000, &input, &device).is_err());
    }
    #[test]
    fn long_wave_wraps_without_truncation() {
        let mut scheduler = OutputScheduler::new(DeviceMode::App3);
        let mut input = input();
        input.frames[0] = Arc::from(
            (0..4096)
                .map(|i| format!("{:02X}0A0A0A{:02X}000000", 10 + i % 231, i / 231))
                .collect::<Vec<_>>(),
        );
        let device = DeviceSnapshot {
            phase: DevicePhase::Ready,
            limits: [Some(100); 2],
            ..Default::default()
        };
        scheduler.tick(0, &input, &device).unwrap();
        let mut expected = 0;
        for i in 0..412 {
            let commands = scheduler.tick(100 + i * 1000, &input, &device).unwrap();
            for command in commands {
                if let DeviceCommand::Pulse {
                    channel: 0, frames, ..
                } = command
                {
                    assert_eq!(frames.len(), if i == 0 { 20 } else { 10 });
                    for frame in frames {
                        assert_eq!(frame, input.frames[0][expected % 4096]);
                        expected += 1;
                    }
                }
            }
        }
        assert!(expected > 4096);
    }
}

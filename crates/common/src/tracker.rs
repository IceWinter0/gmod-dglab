use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSample {
    pub entity_key: u64,
    pub health: i32,
    pub armor: i32,
    pub life_state: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GameEvent {
    HealthDecreased {
        before: i32,
        after: i32,
        death_priority: bool,
    },
    ArmorDecreased {
        before: i32,
        after: i32,
    },
    Died,
    Respawned,
}

#[derive(Default)]
pub struct Tracker {
    previous: Option<PlayerSample>,
}

impl Tracker {
    pub fn reset(&mut self) {
        self.previous = None;
    }
    pub fn observe(&mut self, sample: Option<PlayerSample>) -> Vec<GameEvent> {
        let previous = self.previous;
        self.previous = sample;
        let (Some(before), Some(after)) = (previous, sample) else {
            return vec![];
        };
        if before.entity_key != after.entity_key {
            return vec![];
        }
        let was_alive = before.life_state == 0;
        let alive = after.life_state == 0;
        if !was_alive && alive {
            return vec![GameEvent::Respawned];
        }
        if !was_alive {
            return vec![];
        }
        let mut events = vec![];
        if after.health < before.health {
            events.push(GameEvent::HealthDecreased {
                before: before.health,
                after: after.health,
                death_priority: !alive,
            });
        }
        if after.armor < before.armor {
            events.push(GameEvent::ArmorDecreased {
                before: before.armor,
                after: after.armor,
            });
        }
        if !alive {
            events.push(GameEvent::Died);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn player(key: u64, health: i32, armor: i32, life_state: u8) -> Option<PlayerSample> {
        Some(PlayerSample {
            entity_key: key,
            health,
            armor,
            life_state,
        })
    }
    #[test]
    fn first_sample_is_only_a_baseline() {
        assert!(Tracker::default().observe(player(1, 100, 0, 0)).is_empty());
    }
    #[test]
    fn drop_emits_amount_and_armor_separately() {
        let mut t = Tracker::default();
        t.observe(player(1, 100, 50, 0));
        assert_eq!(
            t.observe(player(1, 80, 40, 0)),
            vec![
                GameEvent::HealthDecreased {
                    before: 100,
                    after: 80,
                    death_priority: false
                },
                GameEvent::ArmorDecreased {
                    before: 50,
                    after: 40
                },
            ]
        );
        assert!(t.observe(player(1, 90, 60, 0)).is_empty());
    }
    #[test]
    fn replacement_and_disconnect_reset_baseline() {
        let mut t = Tracker::default();
        t.observe(player(1, 100, 0, 0));
        assert!(t.observe(player(2, 20, 0, 0)).is_empty());
        assert!(t.observe(None).is_empty());
        assert!(t.observe(player(2, 10, 0, 0)).is_empty());
        t.reset();
        assert!(t.observe(player(2, 5, 0, 0)).is_empty());
    }
    #[test]
    fn death_has_priority_and_respawn_reestablishes_baseline() {
        let mut t = Tracker::default();
        t.observe(player(1, 100, 0, 0));
        assert_eq!(
            t.observe(player(1, 0, 0, 1)),
            vec![
                GameEvent::HealthDecreased {
                    before: 100,
                    after: 0,
                    death_priority: true
                },
                GameEvent::Died,
            ]
        );
        assert!(t.observe(player(1, 0, 0, 2)).is_empty());
        assert_eq!(t.observe(player(1, 100, 0, 0)), vec![GameEvent::Respawned]);
        assert!(t.observe(player(1, 100, 0, 0)).is_empty());
    }
}

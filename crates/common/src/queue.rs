use crate::tracker::GameEvent;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};

pub struct QueuedEvent {
    pub data: GameEvent,
    pub at: Instant,
    pub generation: u64,
}
#[derive(Default)]
pub struct EventQueue {
    items: Mutex<VecDeque<QueuedEvent>>,
    dropped: AtomicU64,
}
impl EventQueue {
    pub fn push(&self, event: GameEvent, at: Instant) {
        self.push_generation(event, at, 0);
    }
    pub fn push_generation(&self, event: GameEvent, at: Instant, generation: u64) {
        let Ok(mut items) = self.items.try_lock() else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if items.len() == 256 {
            items.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        items.push_back(QueuedEvent {
            data: event,
            at,
            generation,
        });
    }
    pub fn pop_fresh(&self, now: Instant) -> Option<QueuedEvent> {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(event) = items.pop_front() {
            if now
                .checked_duration_since(event.at)
                .is_some_and(|age| age.as_millis() <= 1000)
            {
                return Some(event);
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        None
    }
    pub fn clear(&self) {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn pop_generation(&self, now: Instant, generation: u64) -> Option<QueuedEvent> {
        while let Some(event) = self.pop_fresh(now) {
            if event.generation == generation {
                return Some(event);
            }
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn new_connection_does_not_replay_previous_generation() {
        let q = EventQueue::default();
        let now = Instant::now();
        q.push_generation(GameEvent::Died, now, 1);
        q.push_generation(GameEvent::Respawned, now, 2);
        assert_eq!(q.pop_generation(now, 2).unwrap().data, GameEvent::Respawned);
        assert!(q.pop_generation(now, 2).is_none());
    }
    #[test]
    fn overflow_preserves_latest_and_stale_events_are_dropped() {
        let q = EventQueue::default();
        let now = Instant::now();
        for i in 1..=257 {
            q.push(
                GameEvent::HealthDecreased {
                    before: i + 1,
                    after: i,
                    death_priority: false,
                },
                now,
            );
        }
        assert_eq!(q.dropped(), 1);
        assert_eq!(
            q.pop_fresh(now).unwrap().data,
            GameEvent::HealthDecreased {
                before: 3,
                after: 2,
                death_priority: false
            }
        );
        q.clear();
        q.push(GameEvent::Died, now);
        assert!(q.pop_fresh(now + Duration::from_millis(1001)).is_none());
        q.push(GameEvent::Respawned, now);
        q.clear();
        assert!(q.pop_fresh(now).is_none());
    }
    #[test]
    fn contended_callback_does_not_wait() {
        let q = EventQueue::default();
        let _held = q.items.lock().unwrap();
        q.push(GameEvent::Died, Instant::now());
        assert_eq!(q.dropped(), 1);
    }
}

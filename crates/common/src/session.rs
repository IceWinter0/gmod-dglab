use crate::protocol::{Message, VERSION};
#[derive(Debug, PartialEq, Eq)]
pub enum RejectReason {
    Unauthenticated,
    WrongSession,
    Duplicate,
    Expired,
    Version,
    RepeatedHello,
}
pub struct SessionGate {
    token: String,
    session: Option<String>,
    last_seq: u64,
    heartbeat_at: u64,
}
impl SessionGate {
    pub fn new(token: String) -> Self {
        Self {
            token,
            session: None,
            last_seq: 0,
            heartbeat_at: 0,
        }
    }
    pub fn accept(&mut self, message: &Message, now_ms: u64) -> Result<(), RejectReason> {
        if let Message::Hello {
            version,
            session_id,
            token,
            ..
        } = message
        {
            if *version != VERSION {
                return Err(RejectReason::Version);
            }
            if self.session.is_some() {
                return Err(RejectReason::RepeatedHello);
            }
            let difference = token
                .bytes()
                .zip(self.token.bytes())
                .fold(0u8, |d, (a, b)| d | (a ^ b));
            if token.len() != self.token.len() || difference != 0 {
                return Err(RejectReason::Unauthenticated);
            }
            self.session = Some(session_id.clone());
            self.heartbeat_at = now_ms;
            return Ok(());
        }
        let (version, session, seq, age) = match message {
            Message::Heartbeat {
                version,
                session_id,
                seq,
            } => (*version, session_id, *seq, 0),
            Message::Event {
                version,
                session_id,
                seq,
                age_ms,
                ..
            } => (*version, session_id, *seq, *age_ms),
            Message::Hello { .. } => unreachable!(),
        };
        if version != VERSION {
            return Err(RejectReason::Version);
        }
        let Some(current) = &self.session else {
            return Err(RejectReason::Unauthenticated);
        };
        if current != session {
            return Err(RejectReason::WrongSession);
        }
        if !self.is_alive(now_ms) || age > 1000 {
            return Err(RejectReason::Expired);
        }
        if seq <= self.last_seq {
            return Err(RejectReason::Duplicate);
        }
        self.last_seq = seq;
        if matches!(message, Message::Heartbeat { .. }) {
            self.heartbeat_at = now_ms;
        }
        Ok(())
    }
    pub fn is_alive(&self, now_ms: u64) -> bool {
        self.session.is_some() && now_ms.saturating_sub(self.heartbeat_at) < 3000
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn hello(token: &str) -> Message {
        Message::Hello {
            version: VERSION,
            session_id: "s".into(),
            token: token.into(),
            module_id: "diagnostic".into(),
            process_id: None,
        }
    }
    fn beat(seq: u64) -> Message {
        Message::Heartbeat {
            version: VERSION,
            session_id: "s".into(),
            seq,
        }
    }
    #[test]
    fn authentication_sequence_and_heartbeat_expiration() {
        let mut gate = SessionGate::new("secret".into());
        assert_eq!(gate.accept(&beat(1), 0), Err(RejectReason::Unauthenticated));
        assert_eq!(
            gate.accept(&hello("wrong"), 0),
            Err(RejectReason::Unauthenticated)
        );
        assert_eq!(gate.accept(&hello("secret"), 10), Ok(()));
        assert_eq!(
            gate.accept(&hello("secret"), 10),
            Err(RejectReason::RepeatedHello)
        );
        assert_eq!(gate.accept(&beat(2), 100), Ok(()));
        assert_eq!(gate.accept(&beat(2), 101), Err(RejectReason::Duplicate));
        assert_eq!(gate.accept(&beat(1), 101), Err(RejectReason::Duplicate));
        assert!(gate.is_alive(3099));
        assert!(!gate.is_alive(3100));
        assert_eq!(gate.accept(&beat(3), 3100), Err(RejectReason::Expired));
    }
    #[test]
    fn old_event_and_different_session_rejected() {
        let mut gate = SessionGate::new("secret".into());
        gate.accept(&hello("secret"), 0).unwrap();
        assert_eq!(
            gate.accept(
                &Message::Event {
                    version: VERSION,
                    session_id: "s".into(),
                    seq: 1,
                    age_ms: 1001,
                    data: crate::tracker::GameEvent::Died
                },
                1
            ),
            Err(RejectReason::Expired)
        );
        assert_eq!(
            gate.accept(
                &Message::Heartbeat {
                    version: VERSION,
                    session_id: "other".into(),
                    seq: 1
                },
                1
            ),
            Err(RejectReason::WrongSession)
        );
    }
}

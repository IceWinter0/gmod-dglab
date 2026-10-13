use crate::tracker::GameEvent;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Hello {
        version: u32,
        session_id: String,
        token: String,
        module_id: String,
    },
    Heartbeat {
        version: u32,
        session_id: String,
        seq: u64,
    },
    Event {
        version: u32,
        session_id: String,
        seq: u64,
        age_ms: u64,
        data: GameEvent,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    TooLarge,
    InvalidJson,
    UnsupportedVersion,
    InvalidField,
}

pub fn decode_message(bytes: &[u8]) -> Result<Message, ProtocolError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let message: Message = serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
    let (version, session_id, valid) = match &message {
        Message::Hello {
            version,
            session_id,
            token,
            module_id,
        } => (
            *version,
            session_id,
            token.len() >= 32
                && token.len() <= 256
                && !module_id.is_empty()
                && module_id.len() <= 256,
        ),
        Message::Heartbeat {
            version,
            session_id,
            seq,
        } => (*version, session_id, *seq > 0),
        Message::Event {
            version,
            session_id,
            seq,
            data,
            ..
        } => {
            let data_valid = match data {
                GameEvent::HealthDecreased { before, after, .. }
                | GameEvent::ArmorDecreased { before, after } => {
                    before > after && *before >= 0 && *after >= 0
                }
                GameEvent::Died | GameEvent::Respawned => true,
            };
            (*version, session_id, *seq > 0 && data_valid)
        }
    };
    if version != VERSION {
        return Err(ProtocolError::UnsupportedVersion);
    }
    if session_id.is_empty() || session_id.len() > 128 || !valid {
        return Err(ProtocolError::InvalidField);
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_event_decodes() {
        let bytes = br#"{"type":"event","version":1,"session_id":"session-1","seq":1,"age_ms":50,"data":{"kind":"health_decreased","before":100,"after":80,"death_priority":false}}"#;
        assert_eq!(
            decode_message(bytes),
            Ok(Message::Event {
                version: 1,
                session_id: "session-1".into(),
                seq: 1,
                age_ms: 50,
                data: GameEvent::HealthDecreased {
                    before: 100,
                    after: 80,
                    death_priority: false
                },
            })
        );
    }
    #[test]
    fn refuses_unknown_version_and_oversize() {
        assert_eq!(
            decode_message(br#"{"type":"heartbeat","version":2,"session_id":"s","seq":1}"#),
            Err(ProtocolError::UnsupportedVersion)
        );
        assert_eq!(
            decode_message(&vec![b' '; 16385]),
            Err(ProtocolError::TooLarge)
        );
    }
    #[test]
    fn refuses_invalid_fields_and_nonfinite_values() {
        for value in [
            r#"{"type":"heartbeat","version":1,"session_id":"","seq":1}"#,
            r#"{"type":"heartbeat","version":1,"session_id":"s","seq":0}"#,
            r#"{"type":"event","version":1,"session_id":"s","seq":1,"age_ms":0,"data":{"kind":"health_decreased","before":10,"after":20,"death_priority":false}}"#,
        ] {
            assert_eq!(
                decode_message(value.as_bytes()),
                Err(ProtocolError::InvalidField)
            );
        }
        assert_eq!(
            decode_message(br#"{"type":"heartbeat","version":1,"session_id":"s","seq":NaN}"#),
            Err(ProtocolError::InvalidJson)
        );
    }
}

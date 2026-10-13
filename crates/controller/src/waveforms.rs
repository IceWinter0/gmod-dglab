use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

pub const DEFAULT_WAVEFORM: &str = "official:breathing";
pub const MAX_IMPORT_BYTES: usize = 1024 * 1024;
#[derive(Clone, Serialize)]
pub struct Waveform {
    pub id: String,
    pub name: String,
    pub points: Vec<f64>,
    pub frequency_codes: Vec<u8>,
    pub frames: Vec<String>,
    pub source: String,
}
#[derive(Deserialize)]
struct OfficialWave {
    id: String,
    name: String,
    frames: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportWave {
    name: String,
    #[serde(alias = "raw")]
    frames: Vec<String>,
}
static OFFICIAL: LazyLock<Vec<Waveform>> = LazyLock::new(|| {
    let data: Vec<OfficialWave> =
        serde_json::from_str(include_str!("../data/official-coyote.json")).expect("official JSON");
    data.into_iter()
        .map(|w| decode(w.id, w.name, w.frames, "DG-Lab 官方").expect("validated official frames"))
        .collect()
});
pub fn known_builtin_id(id: &str) -> bool {
    OFFICIAL.iter().any(|w| w.id == id)
}
pub fn valid_user_id(id: &str) -> bool {
    id.strip_prefix("user:")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn decode(id: String, name: String, frames: Vec<String>, source: &str) -> Result<Waveform, String> {
    let name = name.trim().to_string();
    if name.is_empty()
        || name.len() > 80
        || name.chars().any(char::is_control)
        || frames.is_empty()
        || frames.len() > 4096
    {
        return Err("波形名称须为 1..80 字节，不含控制字符；帧数须为 1..4096".into());
    }
    let mut power = Vec::with_capacity(frames.len() * 4);
    let mut frequency = Vec::with_capacity(frames.len() * 4);
    let mut normalized = Vec::with_capacity(frames.len());
    for frame in frames {
        if frame.len() != 16 || !frame.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("每帧须为 16 个十六进制字符（8 字节）".into());
        }
        let bytes: Vec<u8> = frame
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                ((b[0] as char).to_digit(16).unwrap() * 16 + (b[1] as char).to_digit(16).unwrap())
                    as u8
            })
            .collect();
        for i in 0..4 {
            if bytes[i] > 240
                || (bytes[i] < 10 && !(bytes[i] == 0 && bytes[i + 4] == 0))
                || bytes[i + 4] > 100
            {
                return Err("频率编码须为 10..240，幅度须为 0..100；仅允许全零补位".into());
            }
            frequency.push(bytes[i]);
            power.push(f64::from(bytes[i + 4]) / 100.0);
        }
        normalized.push(frame.to_ascii_uppercase());
    }
    // 图形最多1024点，完整帧始终保留；避免大文件拖慢 SVG 重绘。
    let step = power.len().div_ceil(1024);
    let points = power.into_iter().step_by(step).collect();
    let frequency_codes = frequency.into_iter().step_by(step).collect();
    Ok(Waveform {
        id,
        name,
        points,
        frequency_codes,
        frames: normalized,
        source: source.into(),
    })
}
pub fn presets() -> Vec<Waveform> {
    OFFICIAL.clone()
}
pub fn parse_import(json: &str) -> Result<Vec<Waveform>, String> {
    if json.len() > MAX_IMPORT_BYTES {
        return Err("波形文件不能超过 1 MiB".into());
    }
    let json = json.strip_prefix('\u{feff}').unwrap_or(json);
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "波形 JSON 无效".to_string())?;
    let records: Vec<ImportWave> = if value.is_array() {
        serde_json::from_value(value)
    } else {
        serde_json::from_value(value).map(|w| vec![w])
    }
    .map_err(|_| "格式须为 {name, frames} 或其数组；raw 可作为 frames 的别名".to_string())?;
    if records.is_empty() || records.len() > 128 {
        return Err("一次可导入 1..128 种波形".into());
    }
    records
        .into_iter()
        .map(|w| {
            let mut wave = decode(String::new(), w.name, w.frames, "用户导入")?;
            let mut hash = Sha256::new();
            hash.update(wave.name.as_bytes());
            hash.update([0]);
            for frame in &wave.frames {
                hash.update(frame.as_bytes());
            }
            wave.id = format!("user:{:x}", hash.finalize());
            Ok(wave)
        })
        .collect()
}
pub fn encode_imports(waves: &[Waveform]) -> Result<String, String> {
    if waves.len() > 128 {
        return Err("最多保存 128 种导入波形".into());
    }
    let records: Vec<ImportWave> = waves
        .iter()
        .map(|w| ImportWave {
            name: w.name.clone(),
            frames: w.frames.clone(),
        })
        .collect();
    let json = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
    if json.len() > MAX_IMPORT_BYTES {
        return Err("导入波形库总大小不能超过 1 MiB".into());
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_data_is_decoded_without_replacing_it_with_a_sine() {
        let waves = presets();
        let breathing = waves.iter().find(|w| w.id == "official:breathing").unwrap();
        assert_eq!(breathing.name, "呼吸");
        assert_eq!(breathing.frames[0], "0A0A0A0A00000000");
        assert_eq!(&breathing.points[..4], &[0.0, 0.0, 0.0, 0.0]);
        assert!(
            waves
                .iter()
                .filter(|w| w.id.starts_with("official:"))
                .count()
                == 24
        );
    }
    #[test]
    fn imports_validate_frames_and_have_stable_distinct_ids() {
        let json = r#"{"name":"我的波形","frames":["0a0a0a0a00643200"]}"#;
        let a = parse_import(json).unwrap();
        let b = parse_import(json).unwrap();
        assert_eq!(a[0].id, b[0].id);
        assert!(a[0].id.starts_with("user:"));
        assert_eq!(a[0].points, vec![0.0, 1.0, 0.5, 0.0]);
        for json in [
            r#"{"name":"","frames":["0A0A0A0A00000000"]}"#,
            r#"{"name":"坏","frames":["0A"]}"#,
            r#"{"name":"坏","frames":["FFFF0A0A00000000"]}"#,
            r#"{"name":"坏","frames":["0A0A0A0AFFFFFFFF"]}"#,
            r#"{"name":"坏","frames":[]}"#,
        ] {
            assert!(parse_import(json).is_err());
        }
    }
    #[test]
    fn batch_and_large_imports_are_bounded() {
        assert!(parse_import("[]").is_err());
        assert!(parse_import(&" ".repeat(1_048_577)).is_err());
        let waves=parse_import(r#"[{"name":"一","raw":["0A0A0A0A64646464"]},{"name":"二","frames":["2D2D2D2D00000000"]}]"#).unwrap();
        assert_eq!(waves.len(), 2);
        assert_ne!(waves[0].id, waves[1].id);
    }
    #[test]
    fn saved_imports_roundtrip_with_identical_ids_and_utf8_bom() {
        let waves = parse_import("\u{feff}{\"name\":\"自定义\",\"frames\":[\"0A0A0A0A00646400\"]}")
            .unwrap();
        let saved = encode_imports(&waves).unwrap();
        let restored = parse_import(&saved).unwrap();
        assert_eq!(restored[0].id, waves[0].id);
        assert_eq!(restored[0].frames, waves[0].frames);
    }

    #[test]
    fn only_official_waveforms_are_built_in() {
        let waves = presets();
        assert_eq!(waves.len(), 24);
        assert!(waves.iter().all(|w| !w.frames.is_empty()));
        assert!(!known_builtin_id("steady"));
    }
}

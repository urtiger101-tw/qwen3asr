use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const LANGUAGES: &[(&str, &str)] = &[
    ("zh", "Chinese"),
    ("en", "English"),
    ("yue", "Cantonese"),
    ("ar", "Arabic"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ko", "Korean"),
    ("ru", "Russian"),
    ("th", "Thai"),
    ("vi", "Vietnamese"),
    ("ja", "Japanese"),
    ("tr", "Turkish"),
    ("hi", "Hindi"),
    ("ms", "Malay"),
    ("nl", "Dutch"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("fil", "Filipino"),
    ("fa", "Persian"),
    ("el", "Greek"),
    ("hu", "Hungarian"),
    ("mk", "Macedonian"),
    ("ro", "Romanian"),
];

pub fn home() -> PathBuf {
    std::env::var_os("QWEN3ASR_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Qwen3ASR")
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub model: String,
    pub device: String,
    pub cache_dir: PathBuf,
    pub chunk_seconds: f64,
    pub cpu_threads: usize,
    pub max_new_tokens: usize,
    pub language: String,
    pub prompt: String,
    pub timestamps: String,
    pub offline: bool,
    pub max_chars: usize,
    pub max_cue_seconds: f64,
    pub traditional: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "0.6b-q8".into(),
            device: "auto".into(),
            cache_dir: home().join("models"),
            chunk_seconds: 25.0,
            cpu_threads: 4,
            max_new_tokens: 512,
            language: "auto".into(),
            prompt: String::new(),
            timestamps: "align".into(),
            offline: false,
            max_chars: 42,
            max_cue_seconds: 6.0,
            traditional: false,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = home().join("config.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let result: Self = serde_json::from_slice(&std::fs::read(&path)?).with_context(|| {
            format!(
                "Invalid configuration {}. Save a backup then use config reset.",
                path.display()
            )
        })?;
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<()> {
        if !["auto", "cuda", "cpu"].contains(&self.device.as_str()) {
            bail!("device must be auto, cuda or cpu");
        }
        if !["align", "segment", "none"].contains(&self.timestamps.as_str()) {
            bail!("timestamps must be align, segment or none");
        }
        if !(5.0..=60.0).contains(&self.chunk_seconds) {
            bail!("chunk_seconds must be 5..60");
        }
        if !(1..=64).contains(&self.cpu_threads) {
            bail!("cpu_threads must be 1..64");
        }
        if !(64..=2048).contains(&self.max_new_tokens) {
            bail!("max_new_tokens must be 64..2048");
        }
        if !(10..=120).contains(&self.max_chars) {
            bail!("max_chars must be 10..120");
        }
        if !(1.0..=30.0).contains(&self.max_cue_seconds) {
            bail!("max_cue_seconds must be 1..30");
        }
        if self.model.trim().is_empty() || self.cache_dir.as_os_str().is_empty() {
            bail!("model and cache_dir cannot be empty");
        }
        if self.prompt.chars().count() > 8192 {
            bail!("prompt is limited to 8192 characters");
        }
        normalize_language(&self.language)?;
        Ok(())
    }

    pub fn save(&self) -> Result<PathBuf> {
        self.validate()?;
        let path = home().join("config.json");
        std::fs::create_dir_all(home())?;
        let mut tmp = tempfile::NamedTempFile::new_in(home())?;
        use std::io::Write;
        tmp.write_all(&serde_json::to_vec_pretty(self)?)?;
        tmp.persist(&path)?;
        Ok(path)
    }

    pub fn set(key: &str, value: &str) -> Result<PathBuf> {
        let mut object = serde_json::to_value(Self::load()?)?;
        let old = object.get(key).context("Unknown configuration key")?;
        object[key] = if old.is_string() {
            value.into()
        } else {
            serde_json::from_str(value).context("Value must be a JSON number or boolean")?
        };
        serde_json::from_value::<Self>(object)?.save()
    }
}

pub fn normalize_language(value: &str) -> Result<String> {
    if value.eq_ignore_ascii_case("auto") {
        return Ok("auto".into());
    }
    LANGUAGES
        .iter()
        .find(|(code, name)| value.eq_ignore_ascii_case(code) || value.eq_ignore_ascii_case(name))
        .map(|(_, name)| name.to_string())
        .context("Unsupported language. Use qwen3asr languages.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_config_rejected() {
        let mut c = Config {
            chunk_seconds: f64::NAN,
            ..Default::default()
        };
        assert!(c.validate().is_err());
        c.chunk_seconds = 25.0;
        c.device = "gpu".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn language_codes_normalized() {
        assert_eq!(normalize_language("ZH").unwrap(), "Chinese");
        assert!(normalize_language("xx").is_err());
    }
    #[test]
    fn unknown_keys_rejected() {
        assert!(serde_json::from_str::<Config>(r#"{"typo":true}"#).is_err());
    }
}

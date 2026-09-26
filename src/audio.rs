//! Decode bounded, mono 16 kHz WAV slices with native FFmpeg.
use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

pub const SAMPLE_RATE: usize = 16_000;
pub const MAX_SECONDS: f64 = 60.0;

pub fn ffmpeg(root: &Path) -> PathBuf {
    let bundled = root.join(if cfg!(windows) {
        "native/ffmpeg.exe"
    } else {
        "native/ffmpeg"
    });
    if bundled.is_file() {
        return bundled;
    }
    let development = root.join(if cfg!(windows) {
        "native/bin/ffmpeg.exe"
    } else {
        "native/bin/ffmpeg"
    });
    if development.is_file() {
        development
    } else {
        PathBuf::from("ffmpeg")
    }
}

pub fn duration(root: &Path, input: &Path, cancel: &Arc<AtomicBool>) -> Result<f64> {
    let binary = ffmpeg(root);
    let probe = binary.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let output = crate::process::capture(
        Command::new(probe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=duration:format=duration",
                "-of",
                "json",
            ])
            .arg(input),
        cancel,
        65536,
    );
    if let Ok(output) = output
        && output.status.success()
        && !output.stdout_truncated
    {
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let seconds = value["streams"][0]["duration"]
            .as_str()
            .or_else(|| value["format"]["duration"].as_str())
            .and_then(|text| text.parse::<f64>().ok());
        if let Some(seconds) = seconds
            && seconds.is_finite()
            && seconds > 0.0
        {
            return Ok(seconds);
        }
    }
    let output = crate::process::capture(
        Command::new(binary).args(["-hide_banner", "-i"]).arg(input),
        cancel,
        65536,
    )?;
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    parse_duration(&diagnostic).context(
        "Could not determine audio duration. Check the media and native FFmpeg installation.",
    )
}

fn parse_duration(text: &str) -> Option<f64> {
    let value = text.split("Duration:").nth(1)?.trim().split(',').next()?;
    let mut parts = value.split(':');
    let hours: f64 = parts.next()?.trim().parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    let duration = hours * 3600.0 + minutes * 60.0 + seconds;
    (duration.is_finite() && duration > 0.0).then_some(duration)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Chunk {
    pub start: f64,
    pub length: f64,
    pub core_start: f64,
    pub core_end: f64,
}

pub fn chunks(duration: f64, seconds: f64, overlap: bool) -> Result<Vec<Chunk>> {
    if !duration.is_finite()
        || duration <= 0.0
        || !seconds.is_finite()
        || !(5.0..=MAX_SECONDS).contains(&seconds)
    {
        bail!("Invalid audio duration or chunk size");
    }
    let context = if overlap { 1.0 } else { 0.0 };
    let step = seconds.min(MAX_SECONDS - 2.0 * context);
    if duration / step > 100_000.0 {
        bail!("Recording exceeds 100,000 chunks; split the input into smaller recordings");
    }
    Ok((0..(duration / step).ceil() as usize)
        .map(|index| {
            let core_start = index as f64 * step;
            let core_end = (core_start + step).min(duration);
            let start = (core_start - context).max(0.0);
            let end = (core_end + context).min(duration);
            Chunk {
                start,
                length: end - start,
                core_start,
                core_end,
            }
        })
        .collect())
}

pub fn decode(
    root: &Path,
    input: &Path,
    chunk: &Chunk,
    output: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<()> {
    if chunk.start < 0.0
        || !chunk.start.is_finite()
        || !(0.0..=MAX_SECONDS).contains(&chunk.length)
        || chunk.length == 0.0
    {
        bail!("Invalid bounded audio slice");
    }
    let result = crate::process::capture(
        Command::new(ffmpeg(root))
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-ss",
            ])
            .arg(format!("{:.6}", chunk.start))
            .arg("-i")
            .arg(input)
            .arg("-t")
            .arg(format!("{:.6}", chunk.length))
            .args([
                "-map",
                "0:a:0",
                "-vn",
                "-ac",
                "1",
                "-ar",
                "16000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
            ])
            .arg(output),
        cancel,
        4096,
    )?;
    if !result.status.success() {
        bail!(
            "FFmpeg failed at {:.2}s: {}",
            chunk.start,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let bytes = output.metadata()?.len();
    if bytes <= 44 || bytes > ((chunk.length + 1.0) * SAMPLE_RATE as f64 * 2.0) as u64 + 4096 {
        bail!("FFmpeg returned an empty or unexpectedly large audio slice");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_audio_stays_bounded_with_complete_nonoverlapping_cores() {
        let chunks = chunks(145.0, 60.0, true).unwrap();
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.length <= 60.0));
        assert_eq!(chunks[0].core_start, 0.0);
        assert_eq!(chunks.last().unwrap().core_end, 145.0);
        for pair in chunks.windows(2) {
            assert_eq!(pair[0].core_end, pair[1].core_start);
        }
        assert!(super::chunks(f64::INFINITY, 25.0, true).is_err());
    }
    #[test]
    fn ffmpeg_duration_handles_hours_and_rejects_missing_values() {
        assert_eq!(
            parse_duration("x Duration: 01:02:03.50, start: 0"),
            Some(3723.5)
        );
        assert_eq!(parse_duration("Duration: N/A, start: 0"), None);
    }
}

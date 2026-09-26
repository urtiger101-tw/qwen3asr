//! Native transcription: bounded audio, one resident model at a time, atomic artifacts.
use crate::{
    audio,
    config::Config,
    subtitles::{self, Cue, Word},
    worker::Worker,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Serialize, Deserialize)]
struct Record {
    chunk: audio::Chunk,
    text: String,
    language: String,
}

pub fn infer(
    root: &Path,
    executable: &Path,
    request: &Value,
    chosen_device: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Value> {
    let input =
        PathBuf::from(request["input"].as_str().context("Missing input")?).canonicalize()?;
    let mut config_value = request["config"].clone();
    let aligner = config_value
        .as_object_mut()
        .context("Invalid configuration")?
        .remove("aligner")
        .and_then(|v| v.as_str().map(PathBuf::from));
    let cfg: Config = serde_json::from_value(config_value)?;
    cfg.validate()?;
    let destination = request["output_dir"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| input.parent().unwrap().to_path_buf());
    fs::create_dir_all(&destination)?;
    let destination = destination.canonicalize()?;
    let stem = input
        .file_stem()
        .context("Input filename is missing")?
        .to_string_lossy()
        .to_string();
    let formats: Vec<String> = serde_json::from_value(request["formats"].clone())?;
    let overwrite = request["overwrite"].as_bool().unwrap_or(false);
    let paths: BTreeMap<String, PathBuf> = formats
        .iter()
        .map(|format| (format.clone(), destination.join(format!("{stem}.{format}"))))
        .collect();
    if paths
        .values()
        .any(|path| path.canonicalize().ok().as_ref() == Some(&input))
    {
        bail!("Output would overwrite the input");
    }
    if !overwrite && let Some(path) = paths.values().find(|path| path.exists()) {
        bail!("Output already exists: {}", path.display());
    }
    let started = Instant::now();
    let duration = audio::duration(root, &input, &cancel)?;
    let bounds = audio::chunks(duration, cfg.chunk_seconds, cfg.timestamps == "align")?;
    fs::create_dir_all(crate::config::home())?;
    let work = tempfile::tempdir_in(crate::config::home())?;
    let records_path = work.path().join("transcript.jsonl");
    let wav = work.path().join("chunk.wav");
    let mut warnings = Vec::<String>::new();
    let mut device = chosen_device.to_owned();
    let mut executable = executable.to_path_buf();
    let mut peak_rss = match asr_pass(
        root,
        &executable,
        &input,
        &wav,
        &bounds,
        &records_path,
        &cfg,
        &device,
        cancel.clone(),
    ) {
        Ok(peak) => peak,
        Err(error)
            if device == "cuda" && cfg.device == "auto" && !cancel.load(Ordering::Relaxed) =>
        {
            let warning = format!("CUDA inference failed; retrying ASR on CPU: {error:#}");
            eprintln!("{warning}");
            warnings.push(warning);
            device = "cpu".into();
            executable = crate::runtime::worker_executable(root, "cpu")?;
            asr_pass(
                root,
                &executable,
                &input,
                &wav,
                &bounds,
                &records_path,
                &cfg,
                &device,
                cancel.clone(),
            )?
        }
        Err(error) => return Err(error),
    };
    // ASR process has exited before a forced-alignment model is loaded.
    let postprocess = (|| -> Result<Value> {
        let mut words = Vec::<Word>::new();
        if cfg.timestamps == "align" {
            let aligner = aligner
                .as_deref()
                .context("ForcedAligner model path is missing")?;
            match align_pass(
                root,
                &executable,
                &input,
                &wav,
                &records_path,
                aligner,
                &cfg,
                &device,
                cancel.clone(),
            ) {
                Ok((aligned, peak)) => {
                    words = aligned;
                    peak_rss = peak_rss.max(peak);
                }
                Err(error)
                    if device == "cuda"
                        && cfg.device == "auto"
                        && !cancel.load(Ordering::Relaxed) =>
                {
                    let warning =
                        format!("CUDA alignment failed; retrying alignment on CPU: {error:#}");
                    eprintln!("{warning}");
                    warnings.push(warning);
                    device = "cpu".into();
                    executable = crate::runtime::worker_executable(root, "cpu")?;
                    let (aligned, peak) = align_pass(
                        root,
                        &executable,
                        &input,
                        &wav,
                        &records_path,
                        aligner,
                        &cfg,
                        &device,
                        cancel.clone(),
                    )?;
                    words = aligned;
                    peak_rss = peak_rss.max(peak);
                }
                Err(error) => return Err(error),
            }
        }
        let converter = cfg.traditional.then(opencc_fmmseg::OpenCC::new);
        if let Some(converter) = &converter {
            for word in &mut words {
                word.text = converter.convert(&word.text, "s2t", false);
            }
        }
        let mut texts = Vec::new();
        let mut segments = Vec::<Cue>::new();
        let mut languages = Vec::<String>::new();
        for record in records(&records_path)? {
            let record = record?;
            if !record.language.is_empty() && !languages.contains(&record.language) {
                languages.push(record.language);
            }
            let text = match &converter {
                Some(converter) => converter.convert(&record.text, "s2t", false),
                None => record.text,
            };
            if !text.is_empty() {
                texts.push(text.clone());
                if cfg.timestamps == "segment" {
                    segments.push(Cue {
                        start: record.chunk.start,
                        end: record.chunk.start + record.chunk.length,
                        text,
                    });
                }
            }
        }
        let text = if cfg.timestamps == "align" {
            if words.is_empty() && texts.iter().any(|text| !text.is_empty()) {
                bail!("Forced alignment returned no words for a nonempty transcript");
            }
            segments = subtitles::cues_from_words(&words, cfg.max_chars, cfg.max_cue_seconds)?;
            subtitles::join_tokens(
                &words
                    .iter()
                    .map(|word| word.text.clone())
                    .collect::<Vec<_>>(),
            )
        } else {
            if cfg.timestamps == "segment" {
                warnings.push(
                    "Segment timestamps are approximate chunk boundaries, not forced alignment."
                        .into(),
                );
            }
            if bounds.len() > 1 {
                warnings.push("Fixed chunk boundaries may split speech in segment/none mode; use align mode for overlapping context.".into());
            }
            subtitles::join_tokens(&texts)
        };
        let language = match languages.len() {
            0 => Value::Null,
            1 => json!(languages[0]),
            _ => json!(languages),
        };
        let mut result = json!({"text":text,"language":language,"duration":duration,"segments":segments,
            "device":device,"model":cfg.model,"backend":"audio.cpp / GGML (native C++)","python_required":false,
            "artifacts":paths,"warnings":warnings,"metrics":{"elapsed_seconds":started.elapsed().as_secs_f64(),
            "peak_process_rss_bytes":if peak_rss>0 {Some(peak_rss)} else {None},"process_peak_source":"native_worker_peak_working_set"}});
        if cfg.timestamps == "align" {
            result["words"] = serde_json::to_value(words)?;
        }
        write_artifacts(&paths, &result, overwrite)?;
        Ok(result)
    })();
    match postprocess {
        Ok(result) => Ok(result),
        Err(error) => {
            let recovery = preserve_transcript(&records_path, &destination, &stem)?;
            Err(error)
                .with_context(|| format!("ASR transcript preserved at {}", recovery.display()))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn asr_pass(
    root: &Path,
    exe: &Path,
    input: &Path,
    wav: &Path,
    bounds: &[audio::Chunk],
    records_path: &Path,
    cfg: &Config,
    device: &str,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    let mut records = File::create(records_path)?;
    let mut worker = Worker::start(
        exe,
        Path::new(&cfg.model),
        "asr",
        device,
        cfg,
        cancel.clone(),
    )?;
    for (index, chunk) in bounds.iter().enumerate() {
        eprintln!(
            "ASR {}/{} · {:.1}s · {device}",
            index + 1,
            bounds.len(),
            chunk.core_start
        );
        audio::decode(root, input, chunk, wav, &cancel)?;
        let response = worker.run(wav, "", &cfg.language)?;
        let text = response["text"]
            .as_str()
            .context("Native ASR response is missing text")?
            .to_owned();
        let language = response["language"].as_str().unwrap_or("").to_owned();
        let language = if language.is_empty() && cfg.language != "auto" {
            cfg.language.clone()
        } else {
            language
        };
        serde_json::to_writer(
            &mut records,
            &Record {
                chunk: chunk.clone(),
                text,
                language,
            },
        )?;
        records.write_all(b"\n")?;
        records.flush()?;
    }
    worker.finish()
}

fn records(path: &Path) -> Result<impl Iterator<Item = Result<Record>>> {
    Ok(BufReader::new(File::open(path)?)
        .lines()
        .map(|line| Ok(serde_json::from_str(&line?)?)))
}

#[allow(clippy::too_many_arguments)]
fn align_pass(
    root: &Path,
    exe: &Path,
    input: &Path,
    wav: &Path,
    records_path: &Path,
    aligner: &Path,
    cfg: &Config,
    device: &str,
    cancel: Arc<AtomicBool>,
) -> Result<(Vec<Word>, u64)> {
    let mut worker = None;
    let mut words = Vec::<Word>::new();
    for record in records(records_path)? {
        let record = record?;
        if record.text.trim().is_empty() {
            continue;
        }
        if ![
            "Chinese",
            "English",
            "Cantonese",
            "French",
            "German",
            "Italian",
            "Japanese",
            "Korean",
            "Portuguese",
            "Russian",
            "Spanish",
        ]
        .contains(&record.language.as_str())
        {
            bail!(
                "ForcedAligner does not support detected language {:?}. Select --timestamps none or segment explicitly.",
                record.language
            );
        }
        if worker.is_none() {
            worker = Some(Worker::start(
                exe,
                aligner,
                "align",
                device,
                cfg,
                cancel.clone(),
            )?);
        }
        eprintln!("Align · {:.1}s · {device}", record.chunk.core_start);
        audio::decode(root, input, &record.chunk, wav, &cancel)?;
        let reply = worker
            .as_mut()
            .unwrap()
            .run(wav, &record.text, &record.language)?;
        let aligned: Vec<Word> = serde_json::from_value(reply["words"].clone())
            .context("Native aligner response is missing word timestamps")?;
        let aligned = subtitles::restore_word_punctuation(&aligned, &record.text)?;
        for mut word in aligned {
            if !word.start.is_finite()
                || !word.end.is_finite()
                || word.end < word.start
                || word.start < -0.1
                || word.end > record.chunk.length + 0.1
            {
                bail!("Native aligner returned an invalid or out-of-range word span");
            }
            word.start = word.start.clamp(0.0, record.chunk.length) + record.chunk.start;
            word.end = word
                .end
                .clamp(0.0, record.chunk.length)
                .max(word.start - record.chunk.start)
                + record.chunk.start;
            let midpoint = (word.start + word.end) / 2.0;
            let final_core =
                (record.chunk.start + record.chunk.length - record.chunk.core_end).abs() < 0.000001;
            if midpoint >= record.chunk.core_start
                && (midpoint < record.chunk.core_end
                    || final_core && midpoint <= record.chunk.core_end)
            {
                // Cross-chunk model jitter must not create overlapping display spans.
                if let Some(previous) = words.last() {
                    word.start = word.start.max(previous.end);
                    word.end = word.end.max(word.start);
                }
                words.push(word);
            }
        }
    }
    let peak = match worker {
        Some(worker) => worker.finish()?,
        None => 0,
    };
    Ok((words, peak))
}

fn preserve_transcript(source: &Path, destination: &Path, stem: &str) -> Result<PathBuf> {
    for index in 0..1000 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!(".{index}")
        };
        let target = destination.join(format!("{stem}.asr-recovery{suffix}.jsonl"));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
        {
            Ok(mut output) => {
                std::io::copy(&mut File::open(source)?, &mut output)?;
                return Ok(target);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!(
        "Too many recovery transcripts; original retained at {}",
        source.display()
    )
}

fn write_artifacts(
    paths: &BTreeMap<String, PathBuf>,
    result: &Value,
    overwrite: bool,
) -> Result<()> {
    let cues: Vec<Cue> = serde_json::from_value(result["segments"].clone())?;
    let mut prepared = Vec::new();
    for (format, path) in paths {
        let text = match format.as_str() {
            "txt" => format!("{}\n", result["text"].as_str().unwrap_or("")),
            "json" => format!("{}\n", serde_json::to_string_pretty(result)?),
            "srt" => subtitles::render_srt(&cues),
            "vtt" => subtitles::render_vtt(&cues),
            _ => bail!("Unsupported output format: {format}"),
        };
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        temporary.write_all(text.as_bytes())?;
        temporary.as_file().sync_all()?;
        prepared.push((path.clone(), temporary));
    }
    let mut prior = BTreeMap::new();
    // Prepare every backup before modifying any output.
    for (path, _) in &prepared {
        if overwrite && path.exists() {
            let backup = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            fs::copy(path, backup.path())?;
            prior.insert(path.clone(), backup);
        }
    }
    let mut committed = HashSet::new();
    for (path, temporary) in prepared {
        let stored = if overwrite {
            temporary.persist(&path)
        } else {
            temporary.persist_noclobber(&path)
        };
        match stored {
            Ok(_) => {
                committed.insert(path);
            }
            Err(error) => {
                let mut failures = Vec::new();
                for path in &committed {
                    if let Some(backup) = prior.remove(path) {
                        if let Err(restore) = backup.persist(path) {
                            let reason = restore.error.to_string();
                            let retained = restore
                                .file
                                .keep()
                                .map(|(_, p)| p.display().to_string())
                                .unwrap_or_else(|e| format!("backup retention failed: {e}"));
                            failures
                                .push(format!("{}: {reason}; backup: {retained}", path.display()));
                        }
                    } else if let Err(remove) = fs::remove_file(path) {
                        failures.push(format!("{}: {remove}", path.display()));
                    }
                }
                let status = if failures.is_empty() {
                    "completed outputs were rolled back".into()
                } else {
                    format!("rollback incomplete: {}", failures.join("; "))
                };
                return Err(error.error)
                    .with_context(|| format!("Could not commit output artifacts; {status}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_traditional_conversion_uses_embedded_dictionaries() {
        assert_eq!(
            opencc_fmmseg::OpenCC::new().convert("甚至出现交易几乎停滞的情况。", "s2t", false),
            "甚至出現交易幾乎停滯的情況。"
        );
    }
    #[test]
    fn atomic_outputs_protect_existing_files_and_hardlink_sources() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("original.wav");
        let output = dir.path().join("original.txt");
        fs::write(&source, b"source media bytes").unwrap();
        fs::hard_link(&source, &output).unwrap();
        let paths = BTreeMap::from([("txt".into(), output.clone())]);
        let result = json!({"text":"recognized","segments":[]});
        assert!(write_artifacts(&paths, &result, false).is_err());
        write_artifacts(&paths, &result, true).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"source media bytes");
        assert_eq!(fs::read_to_string(output).unwrap(), "recognized\n");
    }
}

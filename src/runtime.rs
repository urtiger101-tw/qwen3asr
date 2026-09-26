//! Bundled native engine discovery. No Python interpreter or pip installation.
use crate::config;
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
};

pub fn install_dir() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("QWEN3ASR_INSTALL_DIR") {
        return Ok(PathBuf::from(root));
    }
    let executable = std::env::current_exe()?;
    for root in executable.ancestors().skip(1).take(4) {
        if root.join("native").is_dir() {
            return Ok(root.to_path_buf());
        }
    }
    bail!(
        "Native engine files are missing. Reinstall Qwen3ASR or run scripts/build-native.ps1 for a source checkout."
    )
}

pub fn worker_executable(root: &Path, backend: &str) -> Result<PathBuf> {
    let filename = if cfg!(windows) {
        "qwen3asr-worker.exe"
    } else {
        "qwen3asr-worker"
    };
    for candidate in [
        root.join("native").join(backend).join(filename),
        root.join("native/bin").join(backend).join(filename),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!(
        "Native {backend} engine is missing. Repair/reinstall Qwen3ASR or build the native backend."
    )
}

fn probe(executable: &Path) -> Result<Value> {
    let output = crate::process::capture(
        Command::new(executable).arg("--probe"),
        &Arc::new(AtomicBool::new(false)),
        65536,
    )?;
    if !output.status.success() || output.stdout_truncated {
        bail!(
            "Native engine probe failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let response: Value =
        serde_json::from_slice(&output.stdout).context("Invalid native engine probe response")?;
    if response["ok"] != true {
        bail!("Native engine probe reported an error: {response}");
    }
    Ok(response)
}

fn choose(root: &Path, device: &str) -> Result<(PathBuf, String)> {
    if !["auto", "cuda", "cpu"].contains(&device) {
        bail!("device must be auto, cuda, or cpu");
    }
    if device != "cpu" {
        let gpu = worker_executable(root, "cuda").and_then(|path| Ok((probe(&path)?, path)));
        match gpu {
            Ok((status, path)) if status["cuda_available"] == true => {
                return Ok((path, "cuda".into()));
            }
            Ok((status, _)) if device == "cuda" => bail!(
                "NVIDIA CUDA is unavailable: {status}. Update the NVIDIA driver or use --device cpu."
            ),
            Err(error) if device == "cuda" => return Err(error),
            _ => {}
        }
    }
    let cpu = worker_executable(root, "cpu")?;
    let status = probe(&cpu)?;
    if status["cpu_available"] != true {
        bail!("Native CPU engine is not available");
    }
    Ok((cpu, "cpu".into()))
}

pub fn ensure(device: &str, _offline: bool) -> Result<PathBuf> {
    Ok(choose(&install_dir()?, device)?.0)
}

pub fn setup(device: &str) -> Result<PathBuf> {
    let root = install_dir()?;
    let path = choose(&root, device)?.0;
    let decoder = crate::process::capture(
        Command::new(crate::audio::ffmpeg(&root)).arg("-version"),
        &Arc::new(AtomicBool::new(false)),
        16384,
    )?;
    if !decoder.status.success() {
        bail!("Native FFmpeg is missing or damaged. Repair the installation.");
    }
    Ok(path)
}

pub fn doctor() -> Value {
    let mut result = json!({"version":env!("CARGO_PKG_VERSION"),"native_cli":"Rust","backend":"audio.cpp / GGML (native C++)",
        "python_required":false,"home":config::home(),"runtime_ready":false});
    let root = match install_dir() {
        Ok(root) => root,
        Err(error) => {
            result["runtime_error"] = json!(format!("{error:#}"));
            return result;
        }
    };
    let mut available = serde_json::Map::new();
    for backend in ["cpu", "cuda"] {
        let status = match worker_executable(&root, backend) {
            Ok(path) => match probe(&path) {
                Ok(mut value) => {
                    value["executable"] = json!(path);
                    value
                }
                Err(error) => json!({"error":format!("{error:#}")}),
            },
            Err(error) => json!({"error":format!("{error:#}")}),
        };
        available.insert(backend.into(), status);
    }
    result["runtime_ready"] = json!(available["cpu"]["ok"] == true);
    result["nvidia_detected"] = json!(available["cuda"]["cuda_available"] == true);
    result["engines"] = json!(available);
    result["ffmpeg"] = json!(crate::audio::ffmpeg(&root));
    result
}

pub fn infer(
    request: &Value,
    device: &str,
    _offline: bool,
    cancelled: Arc<AtomicBool>,
) -> Result<Value> {
    let root = install_dir()?;
    let (executable, selected) = choose(&root, device)?;
    fs::create_dir_all(config::home())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config::home().join("inference.lock"))?;
    lock.try_lock_exclusive().context(
        "Another Qwen3ASR recognition job is running. Jobs are serialized to protect GPU/RAM.",
    )?;
    crate::native::infer(&root, &executable, request, &selected, cancelled)
}

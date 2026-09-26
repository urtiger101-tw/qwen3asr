//! A single resident native model behind a cancellable JSON-lines process boundary.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 1024 * 1024;
pub struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    responses: Receiver<std::result::Result<Value, String>>,
    log: Arc<Mutex<Vec<u8>>>,
    cancelled: Arc<AtomicBool>,
    next_id: u64,
    pub peak_rss: u64,
}

impl Worker {
    pub fn start(
        executable: &Path,
        model: &Path,
        task: &str,
        device: &str,
        cfg: &crate::config::Config,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .args(["--task", task, "--backend", device, "--model"])
            .arg(model)
            .args([
                "--threads",
                &cfg.cpu_threads.to_string(),
                "--max-tokens",
                &cfg.max_new_tokens.to_string(),
                "--language",
                &cfg.language,
                "--prompt",
                &cfg.prompt,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("Cannot start native engine {}", executable.display()))?;
        let input = child.stdin.take().context("Native engine stdin missing")?;
        let output = child
            .stdout
            .take()
            .context("Native engine stdout missing")?;
        let stderr = child
            .stderr
            .take()
            .context("Native engine stderr missing")?;
        let (sender, responses) = mpsc::sync_channel(2);
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let result = reader
                    .by_ref()
                    .take(MAX_FRAME as u64 + 1)
                    .read_until(b'\n', &mut bytes);
                let message = match result {
                    Ok(0) => break,
                    Ok(n) if n > MAX_FRAME => Err("Native engine response exceeded 1 MiB".into()),
                    Ok(_) => serde_json::from_slice(&bytes)
                        .map_err(|error| format!("Invalid native response: {error}")),
                    Err(error) => Err(error.to_string()),
                };
                let fatal = message.is_err();
                if sender.send(message).is_err() || fatal {
                    break;
                }
            }
        });
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_reader = log.clone();
        thread::spawn(move || {
            let mut reader = stderr;
            let mut buffer = [0; 8192];
            loop {
                let n = match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if let Ok(mut bytes) = log_reader.lock() {
                    bytes.extend_from_slice(&buffer[..n]);
                    let excess = bytes.len().saturating_sub(65536);
                    bytes.drain(..excess);
                } else {
                    break;
                }
            }
        });
        let mut worker = Self {
            child,
            input: Some(input),
            responses,
            log,
            cancelled,
            next_id: 0,
            peak_rss: 0,
        };
        let ready = worker.receive()?;
        if ready["ready"] != true {
            bail!("Native engine did not report readiness: {ready}");
        }
        if let Some(actual) = ready["backend"].as_str()
            && actual != device
        {
            bail!("Native engine selected {actual} although {device} was requested");
        }
        Ok(worker)
    }

    fn receive(&mut self) -> Result<Value> {
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                bail!("Recognition cancelled");
            }
            match self.responses.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(value)) => {
                    if let Some(error) = value.get("error") {
                        bail!("Native inference failed: {error}\n{}", self.diagnostics());
                    }
                    self.peak_rss = self.peak_rss.max(
                        value["metrics"]["peak_process_rss_bytes"]
                            .as_u64()
                            .unwrap_or(0),
                    );
                    return Ok(value);
                }
                Ok(Err(error)) => bail!("{error}\n{}", self.diagnostics()),
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self.child.try_wait()?;
                    bail!(
                        "Native engine exited unexpectedly ({status:?}): {}",
                        self.diagnostics()
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn diagnostics(&self) -> String {
        self.log
            .lock()
            .map(|v| String::from_utf8_lossy(&v).to_string())
            .unwrap_or_else(|_| "Diagnostic buffer unavailable".into())
    }

    pub fn run(&mut self, audio: &Path, text: &str, language: &str) -> Result<Value> {
        self.next_id += 1;
        let request = json!({"id":self.next_id,"audio":audio,"text":text,"language":language});
        let stdin = self.input.as_mut().context("Native worker is closed")?;
        serde_json::to_writer(&mut *stdin, &request)?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        let reply = self.receive()?;
        if reply["id"] != self.next_id {
            bail!("Native response ID mismatch");
        }
        if reply["truncated"] == true {
            bail!(
                "Recognition reached the token limit. Shorten --chunk-seconds or increase --max-new-tokens."
            );
        }
        Ok(reply)
    }

    pub fn finish(mut self) -> Result<u64> {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                bail!("Recognition cancelled");
            }
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    bail!(
                        "Native engine failed during shutdown: {}",
                        self.diagnostics()
                    );
                }
                return Ok(self.peak_rss);
            }
            if Instant::now() > deadline {
                bail!("Native engine did not release its model in time");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.input.take();
        crate::process::terminate(&mut self.child);
    }
}

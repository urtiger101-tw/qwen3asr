use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::{
    io::{self, BufRead, BufReader, BufWriter, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_MCP_FRAME_BYTES: usize = 1024 * 1024;
const MAX_JOBS: usize = 16;
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    Queued,
    Running,
    CancellationUnconfirmed,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::CancellationUnconfirmed => "cancellation_unconfirmed",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
    tail: bool,
    truncated: bool,
}
impl BoundedBytes {
    fn new(limit: usize, tail: bool) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            tail,
            truncated: false,
        }
    }
    fn push(&mut self, incoming: &[u8]) {
        if self.tail {
            self.bytes.extend_from_slice(incoming);
            if self.bytes.len() > self.limit {
                let drop_count = self.bytes.len() - self.limit;
                self.bytes.drain(..drop_count);
                self.truncated = true;
            }
        } else if self.bytes.len() < self.limit {
            let keep = (self.limit - self.bytes.len()).min(incoming.len());
            self.bytes.extend_from_slice(&incoming[..keep]);
            self.truncated |= keep < incoming.len();
        } else if !incoming.is_empty() {
            self.truncated = true;
        }
    }
}

fn drain_pipe<R: Read + Send + 'static>(
    mut reader: R,
    shared: Arc<Mutex<BoundedBytes>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut block = [0u8; 8192];
        loop {
            match reader.read(&mut block) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if let Ok(mut output) = shared.lock() {
                        output.push(&block[..count]);
                    }
                }
            }
        }
    })
}

struct Job {
    id: String,
    args: Vec<String>,
    inputs: Vec<String>,
    output_dir: Option<String>,
    state: JobState,
    child: Option<Child>,
    stdout: Arc<Mutex<BoundedBytes>>,
    stderr: Arc<Mutex<BoundedBytes>>,
    readers: Vec<JoinHandle<()>>,
    exit_code: Option<i32>,
    result: Option<Value>,
}

#[derive(Default)]
struct JobStore {
    jobs: Vec<Job>,
    executable: Option<PathBuf>,
}

impl JobStore {
    fn prune(&mut self) {
        while self.jobs.len() >= MAX_JOBS {
            if let Some(index) = self.jobs.iter().position(|job| job.state.terminal()) {
                self.jobs.remove(index);
            } else {
                break;
            }
        }
    }

    fn enqueue(
        &mut self,
        inputs: Vec<String>,
        args: Vec<String>,
        output_dir: Option<String>,
    ) -> Result<Value> {
        self.prune();
        if self.jobs.len() >= MAX_JOBS {
            bail!("MCP transcription queue is full (limit {MAX_JOBS})");
        }
        let seq = NEXT_JOB.fetch_add(1, Ordering::Relaxed);
        let id = format!(
            "qwen3asr-{}-{seq}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        self.jobs.push(Job {
            id: id.clone(),
            args,
            inputs,
            output_dir,
            state: JobState::Queued,
            child: None,
            stdout: Arc::new(Mutex::new(BoundedBytes::new(MAX_RESULT_BYTES, false))),
            stderr: Arc::new(Mutex::new(BoundedBytes::new(MAX_LOG_BYTES, true))),
            readers: Vec::new(),
            exit_code: None,
            result: None,
        });
        self.pump();
        let state = self
            .jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.state.as_str())
            .unwrap_or("queued");
        Ok(json!({"job_id":id,"status":state}))
    }

    fn pump(&mut self) {
        if self.jobs.iter().any(|job| {
            matches!(
                job.state,
                JobState::Running | JobState::CancellationUnconfirmed
            )
        }) {
            return;
        }
        loop {
            let Some(index) = self
                .jobs
                .iter()
                .position(|job| job.state == JobState::Queued)
            else {
                return;
            };
            let exe = match self
                .executable
                .clone()
                .map(Ok)
                .unwrap_or_else(std::env::current_exe)
            {
                Ok(exe) => exe,
                Err(error) => {
                    self.jobs[index].state = JobState::Failed;
                    self.jobs[index].result =
                        Some(json!({"error":format!("Cannot locate CLI executable: {error}")}));
                    continue;
                }
            };
            let stdout = self.jobs[index].stdout.clone();
            let stderr = self.jobs[index].stderr.clone();
            let spawn = Command::new(exe)
                .args(&self.jobs[index].args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            match spawn {
                Ok(mut child) => {
                    if let Some(pipe) = child.stdout.take() {
                        self.jobs[index].readers.push(drain_pipe(pipe, stdout));
                    }
                    if let Some(pipe) = child.stderr.take() {
                        self.jobs[index].readers.push(drain_pipe(pipe, stderr));
                    }
                    self.jobs[index].child = Some(child);
                    self.jobs[index].state = JobState::Running;
                    return;
                }
                Err(error) => {
                    self.jobs[index].state = JobState::Failed;
                    self.jobs[index].result =
                        Some(json!({"error":format!("Could not start qwen3asr CLI: {error}")}));
                }
            }
        }
    }

    fn refresh(&mut self) -> Result<()> {
        let active = self
            .jobs
            .iter()
            .position(|job| job.state == JobState::Running);
        if let Some(index) = active {
            let status = self.jobs[index]
                .child
                .as_mut()
                .context("Running job lost its child process")?
                .try_wait()?;
            if let Some(status) = status {
                self.jobs[index].child.take();
                self.jobs[index].exit_code = status.code();
                for reader in self.jobs[index].readers.drain(..) {
                    let _ = reader.join();
                }
                let output_dir = self.jobs[index].output_dir.clone();
                let result = {
                    let captured = self.jobs[index]
                        .stdout
                        .lock()
                        .map_err(|_| anyhow::anyhow!("stdout capture lock poisoned"))?;
                    if captured.truncated {
                        Some(json!({"result_truncated":true,"output_dir":output_dir}))
                    } else {
                        match serde_json::from_slice::<Value>(&captured.bytes) {
                            Ok(value) => Some(result_summary(value)),
                            Err(error) => Some(
                                json!({"output":String::from_utf8_lossy(&captured.bytes).to_string(),"parse_error":error.to_string()}),
                            ),
                        }
                    }
                };
                self.jobs[index].result = result;
                self.jobs[index].state = if status.success() {
                    JobState::Succeeded
                } else {
                    JobState::Failed
                };
            }
        }
        self.pump();
        Ok(())
    }

    fn status(&mut self, id: &str) -> Result<Value> {
        self.refresh()?;
        let job = self
            .jobs
            .iter()
            .find(|job| job.id == id)
            .context("Unknown transcription job_id")?;
        let stderr = job
            .stderr
            .lock()
            .map_err(|_| anyhow::anyhow!("stderr capture lock poisoned"))?;
        Ok(json!({
            "job_id":job.id,"status":job.state.as_str(),"inputs":job.inputs,
            "output_dir":job.output_dir,"exit_code":job.exit_code,"result":job.result,
            "stderr_tail":String::from_utf8_lossy(&stderr.bytes),"stderr_truncated":stderr.truncated,
        }))
    }

    fn cancel(&mut self, id: &str) -> Result<Value> {
        self.cancel_with(id, kill_process_tree)
    }

    fn cancel_with(
        &mut self,
        id: &str,
        terminate: impl FnOnce(&mut Child) -> Result<bool>,
    ) -> Result<Value> {
        let index = self
            .jobs
            .iter()
            .position(|job| job.id == id)
            .context("Unknown transcription job_id")?;
        match self.jobs[index].state {
            JobState::Queued => self.jobs[index].state = JobState::Cancelled,
            JobState::Running => {
                let mut child = self.jobs[index]
                    .child
                    .take()
                    .context("Running job lost its child process")?;
                match terminate(&mut child) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(self.mark_cancellation_unconfirmed(
                            index,
                            child,
                            "the process-tree termination command did not confirm success".into(),
                        ));
                    }
                    Err(error) => {
                        return Err(self.mark_cancellation_unconfirmed(
                            index,
                            child,
                            format!("process-tree termination failed: {error:#}"),
                        ));
                    }
                }
                for reader in self.jobs[index].readers.drain(..) {
                    let _ = reader.join();
                }
                self.jobs[index].state = JobState::Cancelled;
                self.jobs[index].result = Some(json!({
                    "cancelled":true,
                    "process_tree_terminated":true
                }));
            }
            JobState::CancellationUnconfirmed => {
                bail!(
                    "Cancellation remains unconfirmed for job {id}; the queue is paused. Inspect and stop any orphaned qwen3asr-worker/ffmpeg processes. Restarting the MCP server discards its in-memory queue; resubmit queued jobs afterward."
                );
            }
            _ => {
                return Ok(
                    json!({"job_id":id,"status":self.jobs[index].state.as_str(),"cancelled":false}),
                );
            }
        }
        self.pump();
        Ok(json!({"job_id":id,"status":"cancelled","cancelled":true}))
    }

    fn mark_cancellation_unconfirmed(
        &mut self,
        index: usize,
        child: Child,
        cause: String,
    ) -> anyhow::Error {
        let id = self.jobs[index].id.clone();
        let message = format!(
            "Could not confirm termination of the process tree for job {id}: {cause}. The job is recorded as cancellation_unconfirmed and the queue remains paused. Inspect and stop any orphaned qwen3asr-worker/ffmpeg processes. Restarting the MCP server discards its in-memory queue; resubmit queued jobs afterward."
        );
        self.jobs[index].child = Some(child);
        self.jobs[index].state = JobState::CancellationUnconfirmed;
        self.jobs[index].result = Some(json!({
            "cancelled":false,
            "process_tree_terminated":false,
            "error":message
        }));
        anyhow::anyhow!(message)
    }
}

impl Drop for JobStore {
    fn drop(&mut self) {
        for job in &mut self.jobs {
            if let Some(mut child) = job.child.take() {
                let _ = kill_process_tree(&mut child);
            }
        }
    }
}

/// Kill the native CLI and any runtime subprocesses it owns. The Windows
/// CLI waits on native inference/decoder children, so `Child::kill` alone is not
/// enough to stop an inference job or release its captured pipes.
fn kill_process_tree(child: &mut Child) -> Result<bool> {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let status = Command::new("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            let _ = child.wait();
            return Ok(true);
        }
        if child.try_wait()?.is_none() {
            child
                .kill()
                .context("Could not terminate transcription process after taskkill failed")?;
            let _ = child.wait();
        }
        // The CLI may be gone already while one of its descendants remains.
        // Without a successful tree kill, the queue must remain blocked.
        Ok(false)
    }
    #[cfg(not(windows))]
    {
        child
            .kill()
            .context("Could not terminate transcription process")?;
        let _ = child.wait();
        Ok(true)
    }
}

fn result_summary(value: Value) -> Value {
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        let summaries = results
            .iter()
            .cloned()
            .map(result_summary)
            .collect::<Vec<_>>();
        let errors = value
            .get("errors")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        let mut entry = serde_json::Map::new();
                        if let Some(input) = item.get("input") {
                            entry.insert("input".into(), input.clone());
                        }
                        if let Some(error) = item.get("error").and_then(Value::as_str) {
                            let truncated = error.chars().count() > 2000;
                            entry.insert(
                                "error".into(),
                                json!(error.chars().take(2000).collect::<String>()),
                            );
                            if truncated {
                                entry.insert("error_truncated".into(), json!(true));
                            }
                        }
                        Value::Object(entry)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return json!({"results":summaries,"errors":errors});
    }
    let mut summary = serde_json::Map::new();
    for key in [
        "language",
        "duration",
        "device",
        "model",
        "artifacts",
        "warnings",
    ] {
        if let Some(value) = value.get(key) {
            summary.insert(key.into(), value.clone());
        }
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        let mut chars = text.chars();
        let preview: String = chars.by_ref().take(2000).collect();
        summary.insert("text_preview".into(), json!(preview));
        if chars.next().is_some() {
            summary.insert("text_truncated".into(), json!(true));
        }
    }
    Value::Object(summary)
}

struct CliRunOutput {
    success: bool,
    stdout: Vec<u8>,
    stdout_truncated: bool,
    stderr: Vec<u8>,
    stderr_truncated: bool,
}

fn run_cli(args: &[&str]) -> Result<CliRunOutput> {
    let exe = std::env::current_exe().context("Could not locate qwen3asr executable")?;
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = Arc::new(Mutex::new(BoundedBytes::new(MAX_RESULT_BYTES, false)));
    let stderr = Arc::new(Mutex::new(BoundedBytes::new(MAX_LOG_BYTES, true)));
    let out_thread = child
        .stdout
        .take()
        .map(|pipe| drain_pipe(pipe, stdout.clone()));
    let err_thread = child
        .stderr
        .take()
        .map(|pipe| drain_pipe(pipe, stderr.clone()));
    let status = child.wait()?;
    if let Some(thread) = out_thread {
        let _ = thread.join();
    }
    if let Some(thread) = err_thread {
        let _ = thread.join();
    }
    let stdout = stdout
        .lock()
        .map_err(|_| anyhow::anyhow!("stdout capture lock poisoned"))?;
    let stderr = stderr
        .lock()
        .map_err(|_| anyhow::anyhow!("stderr capture lock poisoned"))?;
    Ok(CliRunOutput {
        success: status.success(),
        stdout: stdout.bytes.clone(),
        stdout_truncated: stdout.truncated,
        stderr: stderr.bytes.clone(),
        stderr_truncated: stderr.truncated,
    })
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.into()}})
}
fn tool_result(value: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({"content":[{"type":"text","text":text}],"isError":is_error})
}

enum InputFrame {
    Data(Vec<u8>),
    TooLarge,
    Error(String),
}

fn read_bounded_frame<R: BufRead>(reader: &mut R) -> io::Result<Option<InputFrame>> {
    let mut frame = Vec::new();
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if frame.is_empty() && !oversized {
                Ok(None)
            } else if oversized {
                Ok(Some(InputFrame::TooLarge))
            } else {
                Ok(Some(InputFrame::Data(frame)))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content_len = newline.unwrap_or(available.len());
        let consumed = content_len + usize::from(newline.is_some());
        if !oversized {
            let remaining = MAX_MCP_FRAME_BYTES.saturating_sub(frame.len());
            if content_len > remaining {
                frame.extend_from_slice(&available[..remaining]);
                oversized = true;
            } else {
                frame.extend_from_slice(&available[..content_len]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(if oversized {
                InputFrame::TooLarge
            } else {
                InputFrame::Data(frame)
            }));
        }
    }
}

fn tools() -> Value {
    json!([
            {"name":"info","description":"Run qwen3asr doctor --json and return local runtime status.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
            {"name":"models_list","description":"List available Qwen3-ASR models and local model state.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
            {"name":"models_status","description":"Report installed Qwen3-ASR model files and verification state.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"transcription_start","description":"Queue an asynchronous transcription using the same qwen3asr CLI. GPU work is serialized; output is written by the CLI.","inputSchema":{"type":"object","properties":{
            "inputs":{"type":"array","items":{"type":"string"},"minItems":1,"description":"One or more existing audio/video file paths."},
            "output_dir":{"type":"string"},"formats":{"type":"array","items":{"type":"string","enum":["txt","srt","vtt","json"]}},
            "model":{"type":"string"},"device":{"type":"string","enum":["auto","cuda","cpu"]},"language":{"type":"string"},"timestamps":{"type":"string","enum":["align","segment","none"]},
            "offline":{"type":"boolean"},"traditional":{"type":"boolean"},"overwrite":{"type":"boolean"},"chunk_seconds":{"type":"number"},"cpu_threads":{"type":"integer"},"prompt":{"type":"string"}
                },"required":["inputs"],"additionalProperties":false}},
            {"name":"transcription_status","description":"Read the state and bounded output summary of an asynchronous transcription job.","inputSchema":{"type":"object","properties":{"job_id":{"type":"string"}},"required":["job_id"],"additionalProperties":false}},
            {"name":"transcription_cancel","description":"Cancel a queued or running transcription job.","inputSchema":{"type":"object","properties":{"job_id":{"type":"string"}},"required":["job_id"],"additionalProperties":false}}
    ])
}

fn bounded_result(args: &[&str]) -> Value {
    match run_cli(args) {
        Ok(output) => {
            let parsed = serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(
                |_| json!({"output":String::from_utf8_lossy(&output.stdout).to_string()}),
            );
            tool_result(
                json!({"result":parsed,"output_truncated":output.stdout_truncated,"stderr_tail":String::from_utf8_lossy(&output.stderr),"stderr_truncated":output.stderr_truncated}),
                !output.success,
            )
        }
        Err(error) => tool_result(json!({"error":format!("{error:#}")}), true),
    }
}

fn make_transcribe_args(arguments: &Value) -> Result<(Vec<String>, Vec<String>, Option<String>)> {
    const ALLOWED: &[&str] = &[
        "inputs",
        "output_dir",
        "formats",
        "model",
        "device",
        "language",
        "timestamps",
        "offline",
        "traditional",
        "overwrite",
        "chunk_seconds",
        "cpu_threads",
        "prompt",
    ];
    let object = arguments
        .as_object()
        .context("Transcription arguments must be an object")?;
    if let Some(unknown) = object.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        bail!("Unknown transcription option: {unknown}");
    }
    let inputs = arguments
        .get("inputs")
        .and_then(Value::as_array)
        .context("inputs must be a non-empty array of file paths")?;
    if inputs.is_empty() {
        bail!("inputs must contain at least one path");
    }
    let mut input_paths = Vec::with_capacity(inputs.len());
    for input in inputs {
        let value = input
            .as_str()
            .context("Each inputs entry must be a string path")?;
        let path = PathBuf::from(value)
            .canonicalize()
            .with_context(|| format!("Input file does not exist: {value}"))?;
        if !path.is_file() {
            bail!("Input is not a file: {}", path.display());
        }
        input_paths.push(path.to_string_lossy().into_owned());
    }
    let mut args = vec!["transcribe".to_string()];
    args.extend(input_paths.iter().cloned());
    let output_dir = arguments
        .get("output_dir")
        .map(|value| {
            let value = value.as_str().context("output_dir must be a string")?;
            Ok::<_, anyhow::Error>(
                std::path::absolute(PathBuf::from(value))?
                    .to_string_lossy()
                    .into_owned(),
            )
        })
        .transpose()?;
    if let Some(output_dir) = &output_dir {
        args.push(format!("--output-dir={output_dir}"));
    }
    if let Some(formats) = arguments.get("formats") {
        let formats = formats.as_array().context("formats must be an array")?;
        for format in formats {
            let format = format.as_str().context("formats entries must be strings")?;
            if !["txt", "srt", "vtt", "json"].contains(&format) {
                bail!("Unsupported format: {format}");
            }
            args.push(format!("--format={format}"));
        }
    }
    for (key, flag) in [
        ("model", "--model"),
        ("device", "--device"),
        ("language", "--language"),
        ("timestamps", "--timestamps"),
        ("prompt", "--prompt"),
    ] {
        if let Some(value) = arguments.get(key) {
            let value = value
                .as_str()
                .with_context(|| format!("{key} must be a string"))?;
            args.push(format!("{flag}={value}"));
        }
    }
    for (key, flag) in [
        ("chunk_seconds", "--chunk-seconds"),
        ("cpu_threads", "--cpu-threads"),
    ] {
        if let Some(value) = arguments.get(key) {
            let number = value
                .as_number()
                .with_context(|| format!("{key} must be numeric"))?;
            if key == "cpu_threads" && number.as_u64().is_none_or(|value| value == 0) {
                bail!("cpu_threads must be a positive integer");
            }
            if key == "chunk_seconds" && number.as_f64().is_none_or(|value| value <= 0.0) {
                bail!("chunk_seconds must be positive");
            }
            let value = number.to_string();
            args.push(format!("{flag}={value}"));
        }
    }
    for (key, flag) in [
        ("offline", "--offline"),
        ("traditional", "--traditional"),
        ("overwrite", "--overwrite"),
    ] {
        if let Some(value) = arguments.get(key)
            && value
                .as_bool()
                .with_context(|| format!("{key} must be boolean"))?
        {
            args.push(flag.into());
        }
    }
    if input_paths.len() > 1 {
        args.push("--continue-on-error".into());
    }
    args.push("--json".into());
    Ok((input_paths, args, output_dir))
}

fn validate_tool_arguments(arguments: &Value, allowed: &[&str], tool_name: &str) -> Result<()> {
    let object = arguments
        .as_object()
        .with_context(|| format!("{tool_name} arguments must be an object"))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        bail!("Unknown {tool_name} argument: {unknown}");
    }
    Ok(())
}

fn dispatch_tool(name: &str, args: &Value, jobs: &mut JobStore) -> Result<Value> {
    match name {
        "info" => {
            validate_tool_arguments(args, &[], "info")?;
            Ok(bounded_result(&["doctor", "--json"]))
        }
        "models_list" => {
            validate_tool_arguments(args, &[], "models_list")?;
            Ok(bounded_result(&["models", "list"]))
        }
        "models_status" => {
            validate_tool_arguments(args, &[], "models_status")?;
            Ok(bounded_result(&["models", "status"]))
        }
        "transcription_start" => {
            let (inputs, args, output_dir) = make_transcribe_args(args)?;
            jobs.enqueue(inputs, args, output_dir)
        }
        "transcription_status" => {
            validate_tool_arguments(args, &["job_id"], "transcription_status")?;
            jobs.status(
                args.get("job_id")
                    .and_then(Value::as_str)
                    .context("job_id is required")?,
            )
        }
        "transcription_cancel" => {
            validate_tool_arguments(args, &["job_id"], "transcription_cancel")?;
            jobs.cancel(
                args.get("job_id")
                    .and_then(Value::as_str)
                    .context("job_id is required")?,
            )
        }
        _ => bail!("Unknown qwen3asr MCP tool: {name}"),
    }
}

fn handle_message(message: &Value, jobs: &mut JobStore) -> Option<Value> {
    let id = message.get("id").cloned();
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request: jsonrpc must be 2.0",
        ));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    };
    let id = id?;
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2025-03-26");
            let supported = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
            let selected = if supported.contains(&version) {
                version
            } else {
                "2025-11-25"
            };
            Some(response(
                id,
                json!({"protocolVersion":selected,"capabilities":{"tools":{}},"serverInfo":{"name":"qwen3asr","version":env!("CARGO_PKG_VERSION")}}),
            ))
        }
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => Some(response(id, json!({}))),
        "shutdown" => Some(response(id, json!({}))),
        "tools/list" => Some(response(id, json!({"tools":tools()}))),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str);
            let arguments = params.get("arguments").unwrap_or(&Value::Null);
            let result = name
                .context("tools/call requires params.name")
                .and_then(|name| dispatch_tool(name, arguments, jobs));
            Some(response(
                id,
                match result {
                    Ok(value) if value.get("content").is_some() => value,
                    Ok(value) => tool_result(value, false),
                    Err(error) => tool_result(json!({"error":format!("{error:#}")}), true),
                },
            ))
        }
        _ => Some(rpc_error(id, -32601, format!("Method not found: {method}"))),
    }
}

pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut jobs = JobStore::default();
    let (sender, receiver) = mpsc::sync_channel::<InputFrame>(16);
    thread::spawn(move || {
        let mut reader = BufReader::new(stdin.lock());
        loop {
            match read_bounded_frame(&mut reader) {
                Ok(Some(frame)) => {
                    if sender.send(frame).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.send(InputFrame::Error(error.to_string()));
                    break;
                }
            }
        }
    });
    loop {
        if crate::config::CANCELLED.load(Ordering::Relaxed) {
            break;
        }
        let frame = match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => {
                jobs.refresh()?;
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if crate::config::CANCELLED.load(Ordering::Relaxed) {
            break;
        }
        let response = match frame {
            InputFrame::Error(error) => bail!("Could not read MCP request from stdin: {error}"),
            InputFrame::TooLarge => Some(rpc_error(
                Value::Null,
                -32600,
                format!("Request frame exceeds {MAX_MCP_FRAME_BYTES} byte limit"),
            )),
            InputFrame::Data(bytes) if bytes.iter().all(|byte| byte.is_ascii_whitespace()) => None,
            InputFrame::Data(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(request) => handle_message(&request, &mut jobs),
                Err(error) => Some(rpc_error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {error}"),
                )),
            },
        };
        jobs.refresh()?;
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_and_tools_list_are_json_rpc_responses() {
        let mut jobs = JobStore::default();
        let initialized = handle_message(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),&mut jobs).unwrap();
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        let listed = handle_message(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            &mut jobs,
        )
        .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn transcribe_arguments_match_cli_and_reject_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("recording.wav");
        std::fs::write(&input, b"fixture").unwrap();
        let request = json!({"inputs":[input.to_string_lossy()],"formats":["srt","json"],"device":"cpu","timestamps":"segment","offline":true});
        let (_, args, _) = make_transcribe_args(&request).unwrap();
        assert!(args.contains(&"--format=srt".to_string()));
        assert!(args.contains(&"--json".to_string()));
        assert!(make_transcribe_args(&json!({"inputs":["missing.wav"]})).is_err());
    }

    #[test]
    fn output_and_log_buffers_stay_bounded() {
        let mut buffer = BoundedBytes::new(4, true);
        buffer.push(b"abcdef");
        assert_eq!(buffer.bytes, b"cdef");
        assert!(buffer.truncated);
    }

    #[test]
    fn stdin_frames_are_bounded_and_oversized_lines_are_drained() {
        let mut bytes = vec![b'x'; MAX_MCP_FRAME_BYTES + 32];
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let mut reader = io::Cursor::new(bytes);
        assert!(matches!(
            read_bounded_frame(&mut reader).unwrap(),
            Some(InputFrame::TooLarge)
        ));
        match read_bounded_frame(&mut reader).unwrap().unwrap() {
            InputFrame::Data(frame) => assert_eq!(
                serde_json::from_slice::<Value>(&frame).unwrap()["method"],
                "ping"
            ),
            _ => panic!("expected the frame after the oversized request"),
        }
        assert!(read_bounded_frame(&mut reader).unwrap().is_none());
    }

    #[test]
    fn batch_summary_preserves_successes_artifacts_and_bounded_errors() {
        let long_error = "x".repeat(3000);
        let value = result_summary(json!({
            "results":[{"text":"recognized","language":"Chinese","artifacts":{"srt":"C:/out/a.srt"}}],
            "errors":[{"input":"bad.wav","error":long_error}]
        }));
        assert_eq!(value["results"][0]["text_preview"], "recognized");
        assert_eq!(value["results"][0]["artifacts"]["srt"], "C:/out/a.srt");
        assert_eq!(value["errors"][0]["input"], "bad.wav");
        assert_eq!(
            value["errors"][0]["error"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            2000
        );
        assert_eq!(value["errors"][0]["error_truncated"], true);
    }

    #[test]
    fn multi_file_arguments_enable_continue_on_error_and_validate_integer_options() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.wav");
        let second = dir.path().join("second.wav");
        std::fs::write(&first, b"a").unwrap();
        std::fs::write(&second, b"b").unwrap();
        let request = json!({"inputs":[first,second]});
        let (_, args, _) = make_transcribe_args(&request).unwrap();
        assert!(args.contains(&"--continue-on-error".to_string()));
        assert!(
            make_transcribe_args(
                &json!({"inputs":[dir.path().join("first.wav")],"cpu_threads":2.5})
            )
            .is_err()
        );
        assert!(
            make_transcribe_args(&json!({"inputs":[dir.path().join("first.wav")],"cpu_threads":0}))
                .is_err()
        );
        assert!(
            make_transcribe_args(&json!({"inputs":[dir.path().join("first.wav")],"unknown":true}))
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn sleeping_process_test_helper() {
        let arguments = std::env::args().collect::<Vec<_>>();
        let Some(separator) = arguments.iter().position(|argument| argument == "--") else {
            return;
        };
        let mode = arguments
            .get(separator + 1)
            .map(String::as_str)
            .unwrap_or("");
        if mode == "spawn-child" {
            let executable = std::env::current_exe().unwrap();
            let mut child = Command::new(executable)
                .args([
                    "--exact",
                    "mcp::tests::sleeping_process_test_helper",
                    "--nocapture",
                    "--",
                    "sleep",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            eprintln!("child_ready");
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        } else if mode == "sleep" {
            thread::sleep(Duration::from_secs(60));
        }
    }

    #[cfg(windows)]
    #[test]
    fn cancel_serial_queue_kills_cli_and_descendant_processes() {
        let mut jobs = JobStore::default();
        jobs.executable = Some(std::env::current_exe().unwrap());
        let args = vec![
            "--exact".into(),
            "mcp::tests::sleeping_process_test_helper".into(),
            "--nocapture".into(),
            "--".into(),
            "spawn-child".into(),
        ];
        let first = jobs
            .enqueue(vec!["first.wav".into()], args.clone(), None)
            .unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let second = jobs.enqueue(vec!["second.wav".into()], args, None).unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(jobs.status(&first).unwrap()["status"], "running");
        assert_eq!(jobs.status(&second).unwrap()["status"], "queued");
        assert_eq!(jobs.cancel(&second).unwrap()["cancelled"], true);

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let index = jobs.jobs.iter().position(|job| job.id == first).unwrap();
            let ready = jobs.jobs[index]
                .stderr
                .lock()
                .unwrap()
                .bytes
                .windows(11)
                .any(|bytes| bytes == b"child_ready");
            if ready {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "test helper did not launch its descendant"
            );
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(jobs.cancel(&first).unwrap()["cancelled"], true);
        assert_eq!(jobs.status(&first).unwrap()["status"], "cancelled");
        assert_eq!(jobs.status(&second).unwrap()["status"], "cancelled");
    }

    #[cfg(windows)]
    #[test]
    fn cancel_failure_keeps_slot_blocked_and_reports_unconfirmed_state() {
        let mut jobs = JobStore::default();
        jobs.executable = Some(std::env::current_exe().unwrap());
        let args = vec![
            "--exact".into(),
            "mcp::tests::sleeping_process_test_helper".into(),
            "--nocapture".into(),
            "--".into(),
            "sleep".into(),
        ];
        let first = jobs
            .enqueue(vec!["first.wav".into()], args.clone(), None)
            .unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let second = jobs.enqueue(vec!["second.wav".into()], args, None).unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();

        let error = jobs
            .cancel_with(&first, |child| {
                child.kill()?;
                child.wait()?;
                // Killing the worker process alone does not confirm that all descendants exited.
                Ok(false)
            })
            .unwrap_err();
        assert!(error.to_string().contains("queue remains paused"));
        assert!(error.to_string().contains("resubmit queued jobs"));
        assert_eq!(
            jobs.status(&first).unwrap()["status"],
            "cancellation_unconfirmed"
        );
        assert_eq!(jobs.status(&first).unwrap()["result"]["cancelled"], false);
        assert_eq!(jobs.status(&second).unwrap()["status"], "queued");
        let first_job = jobs.jobs.iter_mut().find(|job| job.id == first).unwrap();
        assert!(
            first_job
                .child
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
        );
        assert!(
            jobs.jobs
                .iter()
                .find(|job| job.id == second)
                .unwrap()
                .child
                .is_none()
        );
    }
}

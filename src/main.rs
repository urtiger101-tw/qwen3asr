mod audio;
mod config;
mod integrations;
mod mcp;
mod models;
mod native;
mod process;
mod runtime;
mod subtitles;
mod worker;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(
    name = "qwen3asr",
    version,
    about = "本機語音辨識 · Rust CLI · NVIDIA GPU 優先 · 精準字幕時間軸",
    after_help = "快速開始：qwen3asr transcribe recording.mp3\n亦可直接使用：qwen3asr recording.mp3\n原生 Rust/C++ 推論，首次辨識自動下載模型；預設輸出 TXT、SRT、JSON。"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 辨識音訊或影片，可一次指定多個檔案（依序處理）
    Transcribe(Box<Transcribe>),
    /// 下載、查驗與管理官方模型
    Models {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// 查看或保存預設設定；命令列選項優先
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// 檢查原生推論引擎，可預先下載模型；不需 Python
    Setup {
        #[arg(long, default_value="auto", value_parser=["auto","cuda","cpu"])]
        device: String,
        #[arg(long)]
        download_model: bool,
    },
    /// 檢查 NVIDIA GPU、推論環境與 FFmpeg
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// 列出辨識與精準時間對齊支援的語言
    Languages,
    /// 啟動 stdio MCP server，供 agent 使用
    Mcp,
    /// 安裝／移除 Codex、AGY、Claude 的 MCP 與 Skill
    Agents {
        #[command(subcommand)]
        command: AgentCommand,
    },
}

#[derive(Args)]
struct Transcribe {
    #[arg(required = true)]
    input: Vec<PathBuf>,
    #[arg(short = 'o', long)]
    output_dir: Option<PathBuf>,
    #[arg(short='f', long="format", value_parser=["txt","srt","vtt","json"])]
    formats: Vec<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, value_parser=["auto","cuda","cpu"])]
    device: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long, value_parser=["align","segment","none"])]
    timestamps: Option<String>,
    #[arg(long)]
    chunk_seconds: Option<f64>,
    #[arg(long)]
    cpu_threads: Option<usize>,
    #[arg(long)]
    max_new_tokens: Option<usize>,
    #[arg(long)]
    max_chars: Option<usize>,
    #[arg(long)]
    max_cue_seconds: Option<f64>,
    /// 僅使用已下載的環境與模型
    #[arg(long)]
    offline: bool,
    /// 將文字轉為繁體中文；辨識與對齊後執行
    #[arg(long)]
    traditional: bool,
    /// 允許覆寫既有輸出（不覆寫原始輸入）
    #[arg(long)]
    overwrite: bool,
    /// stdout 僅輸出 JSON；進度與錯誤寫入 stderr
    #[arg(long)]
    json: bool,
    /// 批次模式：單檔失敗後繼續；仍以非零退出碼回報
    #[arg(long)]
    continue_on_error: bool,
}

#[derive(Subcommand)]
enum ModelCommand {
    List,
    Status,
    Download {
        #[arg(default_value = "0.6b-q8")]
        model: String,
        #[arg(long)]
        with_aligner: bool,
    },
    Verify {
        #[arg(default_value = "0.6b-q8")]
        model: String,
    },
}
#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Path,
    Set { key: String, value: String },
    Reset,
}
#[derive(Subcommand)]
enum AgentCommand {
    Install {
        #[arg(long,default_value="codex",value_parser=["codex","agy","claude","all"])]
        target: String,
        #[arg(long)]
        dry_run: bool,
    },
    Uninstall {
        #[arg(long,default_value="codex",value_parser=["codex","agy","claude","all"])]
        target: String,
        #[arg(long)]
        dry_run: bool,
    },
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn path_key(path: &std::path::Path) -> String {
    let text = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn preflight(
    inputs: &[PathBuf],
    output_dir: Option<&PathBuf>,
    formats: &[String],
    overwrite: bool,
    continue_on_error: bool,
) -> Result<(Vec<PathBuf>, Vec<Value>)> {
    let source_keys: std::collections::HashSet<_> = inputs
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .map(|p| path_key(&p))
        .collect();
    let mut outputs = std::collections::HashSet::new();
    let mut ready = Vec::new();
    let mut errors = Vec::new();
    for original in inputs {
        let prepared = (|| -> Result<PathBuf> {
            let input = original
                .canonicalize()
                .with_context(|| format!("Input not found: {}", original.display()))?;
            if !input.is_file() {
                bail!("Input is not a regular file: {}", input.display());
            }
            let parent = std::path::absolute(
                output_dir
                    .cloned()
                    .unwrap_or_else(|| input.parent().unwrap().to_path_buf()),
            )?;
            let stem = input
                .file_stem()
                .context("Input has no filename")?
                .to_string_lossy();
            let mut keys = Vec::new();
            for format in formats {
                let output = parent.join(format!("{stem}.{format}"));
                let key = path_key(&output);
                if source_keys.contains(&key) {
                    bail!("Output would overwrite a batch input: {}", output.display());
                }
                if outputs.contains(&key) {
                    bail!(
                        "Batch output filename collision: {}. Rename inputs or process separately.",
                        output.display()
                    );
                }
                if !overwrite && output.exists() {
                    bail!(
                        "Output already exists: {}. Choose another output directory or use --overwrite.",
                        output.display()
                    );
                }
                keys.push(key);
            }
            outputs.extend(keys);
            Ok(input)
        })();
        match prepared {
            Ok(input) => ready.push(input),
            Err(error) if continue_on_error => {
                eprintln!("ERROR: {error:#}");
                errors.push(json!({"input":original,"error":format!("{error:#}")}));
            }
            Err(error) => return Err(error),
        }
    }
    Ok((ready, errors))
}

fn transcribe(args: Transcribe, cancelled: Arc<AtomicBool>) -> Result<()> {
    let mut cfg = config::Config::load()?;
    macro_rules! apply { ($($field:ident),*) => { $(if let Some(value) = args.$field { cfg.$field = value; })* }; }
    apply!(
        model,
        device,
        language,
        prompt,
        timestamps,
        chunk_seconds,
        cpu_threads,
        max_new_tokens,
        max_chars,
        max_cue_seconds
    );
    cfg.offline |= args.offline;
    cfg.traditional |= args.traditional;
    cfg.language = config::normalize_language(&cfg.language)?;
    cfg.validate()?;
    let mut formats = if args.formats.is_empty() {
        if cfg.timestamps == "none" {
            vec!["txt".into(), "json".into()]
        } else {
            vec!["txt".into(), "srt".into(), "json".into()]
        }
    } else {
        args.formats
    };
    let mut seen = std::collections::HashSet::new();
    formats.retain(|format| seen.insert(format.clone()));
    if cfg.timestamps == "none" && formats.iter().any(|f| f == "srt" || f == "vtt") {
        bail!("SRT/VTT requires --timestamps align or segment");
    }
    let need_alignment = cfg.timestamps == "align";
    if need_alignment
        && cfg.language != "auto"
        && ![
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
        .contains(&cfg.language.as_str())
    {
        bail!(
            "The official ForcedAligner does not support {}. Use --timestamps segment for explicitly approximate timing, or --timestamps none for text only.",
            cfg.language
        );
    }
    let (inputs, mut errors) = preflight(
        &args.input,
        args.output_dir.as_ref(),
        &formats,
        args.overwrite,
        args.continue_on_error,
    )?;
    if inputs.is_empty() {
        if args.json {
            print_json(&json!({"results":[],"errors":errors}))?;
        }
        bail!("No valid inputs to recognize");
    }
    models::validate_selector(&cfg.model)?;
    let _ = runtime::ensure(&cfg.device, cfg.offline)?;
    let model = models::resolve(&cfg.model, &cfg.cache_dir, cfg.offline)?;
    let aligner = if need_alignment {
        Some(models::resolve("aligner-q8", &cfg.cache_dir, cfg.offline)?)
    } else {
        None
    };
    let mut options = serde_json::to_value(&cfg)?;
    options["model"] = json!(model);
    if let Some(aligner) = aligner {
        options["aligner"] = json!(aligner);
    }
    let output_dir = args.output_dir.map(std::path::absolute).transpose()?;
    let mut results = Vec::new();
    for input in inputs {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Recognition cancelled");
        }
        eprintln!("Recognizing {}", input.display());
        let request = json!({"input":input,"config":options,"output_dir":output_dir,"formats":formats,"overwrite":args.overwrite});
        match runtime::infer(&request, &cfg.device, cfg.offline, cancelled.clone()) {
            Ok(mut value) => {
                value["requested_model"] = json!(cfg.model);
                if !args.json {
                    println!("{}", value["text"].as_str().unwrap_or(""));
                    println!("{}", serde_json::to_string_pretty(&value["artifacts"])?);
                }
                results.push(value);
            }
            Err(error) if args.continue_on_error => {
                eprintln!("ERROR: {error:#}");
                errors.push(json!({"input":input,"error":format!("{error:#}")}));
            }
            Err(error) => return Err(error),
        }
    }
    if args.json {
        if results.len() == 1 && errors.is_empty() {
            print_json(&results[0])?;
        } else {
            print_json(&json!({"results":results,"errors":errors}))?;
        }
    }
    if !errors.is_empty() {
        bail!("{} batch input(s) failed", errors.len());
    }
    Ok(())
}

fn execute(cli: Cli, cancelled: Arc<AtomicBool>) -> Result<()> {
    match cli.command {
        None => { use clap::CommandFactory; Cli::command().print_help()?; println!(); Ok(()) }
        Some(Commands::Transcribe(args)) => transcribe(*args, cancelled),
        Some(Commands::Models { command }) => {
            let cfg = config::Config::load()?;
            match command {
                ModelCommand::List | ModelCommand::Status => print_json(&models::list(&cfg.cache_dir)),
                ModelCommand::Download { model, with_aligner } => {
                    let mut result = vec![models::download(&model, &cfg.cache_dir)?];
                    if with_aligner && model != "aligner" && model != "aligner-q8" { result.push(models::download("aligner-q8", &cfg.cache_dir)?); }
                    print_json(&json!(result))
                }
                ModelCommand::Verify { model } => print_json(&models::verify(&model, &cfg.cache_dir)?),
            }
        }
        Some(Commands::Config { command }) => match command {
            ConfigCommand::Show => print_json(&serde_json::to_value(config::Config::load()?)?),
            ConfigCommand::Path => { println!("{}",config::home().join("config.json").display()); Ok(()) }
            ConfigCommand::Set { key, value } => { let path = config::Config::set(&key,&value)?; print_json(&json!({"saved":path})) }
            ConfigCommand::Reset => { let path = config::Config::default().save()?; print_json(&json!({"saved":path})) }
        },
        Some(Commands::Setup { device, download_model }) => {
            let prepare = if download_model {
                let cfg = config::Config::load()?;
                models::validate_selector(&cfg.model)?;
                Some(cfg)
            } else { None };
            let path = runtime::setup(&device)?;
            if let Some(cfg) = prepare {
                models::resolve(&cfg.model, &cfg.cache_dir, false)?;
                if cfg.timestamps == "align" { models::resolve("aligner-q8", &cfg.cache_dir, false)?; }
            }
            print_json(&json!({"ready":true,"engine":path,"python_required":false}))
        }
        Some(Commands::Doctor { json: as_json }) => {
            let status = runtime::doctor();
            if !as_json { println!("Qwen3ASR 環境診斷"); }
            print_json(&status)
        }
        Some(Commands::Languages) => print_json(&json!(config::LANGUAGES.iter().map(|(code,name)| json!({"code":code,"name":name,"forced_alignment":(["zh","en","yue","fr","de","it","ja","ko","pt","ru","es"].contains(code))})).collect::<Vec<_>>())),
        Some(Commands::Mcp) => mcp::run(),
        Some(Commands::Agents { command }) => match command {
            AgentCommand::Install { target,dry_run } => print_json(&integrations::install(&target,dry_run)?),
            AgentCommand::Uninstall { target,dry_run } => print_json(&integrations::uninstall(&target,dry_run)?),
        },
    }
}

fn main() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    if let Err(error) = ctrlc::set_handler(move || {
        signal.store(true, Ordering::Relaxed);
        config::CANCELLED.store(true, Ordering::Relaxed);
    }) {
        eprintln!(
            "WARNING: Cannot register Ctrl-C handler: {error}. Use the operating system to stop a job if needed."
        );
    }
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|a| {
        !a.to_string_lossy().starts_with('-')
            && ![
                "transcribe",
                "models",
                "config",
                "setup",
                "doctor",
                "languages",
                "mcp",
                "agents",
                "help",
            ]
            .contains(&a.to_string_lossy().as_ref())
    }) {
        args.insert(1, "transcribe".into());
    }
    let cli = Cli::parse_from(args);
    if let Err(error) = execute(cli, cancelled.clone()) {
        eprintln!("ERROR: {error:#}");
        std::process::exit(if cancelled.load(Ordering::Relaxed) {
            130
        } else {
            1
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_preserves_all_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("input.wav");
        let txt = dir.path().join("input.txt");
        std::fs::write(&wav, b"audio").unwrap();
        std::fs::write(&txt, b"other audio").unwrap();
        assert!(
            preflight(&[wav, txt], None, &["txt".into()], true, false)
                .unwrap_err()
                .to_string()
                .contains("batch input")
        );
    }
    #[test]
    fn dotted_filename_and_continue_preflight() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("clip.part.wav");
        std::fs::write(&wav, b"audio").unwrap();
        std::fs::write(dir.path().join("clip.part.srt"), b"existing").unwrap();
        assert!(
            preflight(
                std::slice::from_ref(&wav),
                None,
                &["srt".into()],
                false,
                false
            )
            .is_err()
        );
        let (inputs, errors) = preflight(
            &[dir.path().join("missing.wav"), wav],
            None,
            &["txt".into()],
            false,
            true,
        )
        .unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(errors.len(), 1);
    }
}

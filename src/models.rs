use anyhow::{Context, Result, bail};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

pub const CATALOG: &[(&str, &str)] = &[
    ("0.6b", "Qwen/Qwen3-ASR-0.6B-hf"),
    ("1.7b", "Qwen/Qwen3-ASR-1.7B-hf"),
    ("aligner", "Qwen/Qwen3-ForcedAligner-0.6B-hf"),
];

const GGUF_REPOSITORY: &str = "audio-cpp/audio.cpp-gguf";
const GGUF_REVISION: &str = "0a104324546d2622985e3c676a4b5550cc772127";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NativeRole {
    Asr,
    Aligner,
}

#[derive(Clone, Copy, Debug)]
struct NativeModel {
    alias: &'static str,
    role: NativeRole,
    group: &'static str,
    filename: &'static str,
    source_file: &'static str,
    size: u64,
    sha256: &'static str,
}

const NATIVE_CATALOG: &[NativeModel] = &[
    NativeModel {
        alias: "0.6b-q8",
        role: NativeRole::Asr,
        group: "Qwen3-ASR-0.6B-GGUF",
        filename: "qwen3-asr-0.6b-q8_0.gguf",
        source_file: "Qwen3-ASR-0.6B-GGUF/qwen3-asr-0.6b-q8_0.gguf",
        size: 1_151_272_416,
        sha256: "6c44ec2fb4cee513892d7863c1fcc3ea6b699ffa4d899b0ef4ab19956d9544f7",
    },
    NativeModel {
        alias: "aligner-q8",
        role: NativeRole::Aligner,
        group: "Qwen3-ForcedAligner-0.6B-GGUF",
        filename: "qwen3-forced-aligner-0.6b-q8_0.gguf",
        source_file: "Qwen3-ForcedAligner-0.6B-GGUF/qwen3-forced-aligner-0.6b-q8_0.gguf",
        size: 1_129_966_496,
        sha256: "75209490b11cec2b0db749ca5f4ff92266f58efd30f7fd04d9eb2a3ac9cc929f",
    },
];

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ModelFormat {
    #[default]
    Safetensors,
    Gguf,
}

#[derive(Clone, Copy)]
enum CatalogModel {
    Hf {
        alias: &'static str,
        repository: &'static str,
    },
    Native(&'static NativeModel),
}

#[derive(Serialize, Deserialize)]
struct ModelFile {
    name: String,
    size: u64,
    sha256: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    repository: String,
    revision: String,
    files: Vec<ModelFile>,
    #[serde(default)]
    format: ModelFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_file: Option<String>,
}

fn catalog_model(name: &str) -> Option<CatalogModel> {
    CATALOG
        .iter()
        .find(|(alias, repo)| name.eq_ignore_ascii_case(alias) || name == *repo)
        .map(|(alias, repository)| CatalogModel::Hf { alias, repository })
        .or_else(|| {
            NATIVE_CATALOG
                .iter()
                .find(|model| name.eq_ignore_ascii_case(model.alias))
                .map(CatalogModel::Native)
        })
}

fn native_directory(model: &NativeModel, cache: &Path) -> PathBuf {
    cache
        .join(GGUF_REPOSITORY.replace('/', "--"))
        .join(model.group)
}

fn native_file(model: &NativeModel, cache: &Path) -> PathBuf {
    native_directory(model, cache).join(model.filename)
}

fn native_provenance(model: &NativeModel) -> Value {
    json!({
        "source_repository": GGUF_REPOSITORY,
        "source_revision": GGUF_REVISION,
        "source_file": model.source_file,
        "format": "gguf",
    })
}

fn hf_provenance(repository: &str) -> Value {
    json!({
        "source_repository": repository,
        "format": "safetensors",
    })
}

fn has_gguf_magic(path: &Path) -> Result<bool> {
    let mut file =
        File::open(path).with_context(|| format!("Cannot open model file {}", path.display()))?;
    let mut magic = [0; 4];
    Ok(file.read_exact(&mut magic).is_ok() && &magic == b"GGUF")
}

fn validate_native_manifest(model: &NativeModel, dir: &Path, verify: bool) -> Result<()> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("qwen3asr-model.json"))?)?;
    if manifest.repository != GGUF_REPOSITORY
        || manifest.revision != GGUF_REVISION
        || manifest.format != ModelFormat::Gguf
        || manifest.source_file.as_deref() != Some(model.source_file)
        || manifest.files.len() != 1
    {
        bail!("Native model manifest provenance does not match the pinned GGUF catalog entry");
    }
    let entry = &manifest.files[0];
    if entry.name != model.filename
        || entry.size != model.size
        || entry.sha256.as_deref() != Some(model.sha256)
    {
        bail!("Native model manifest metadata does not match the pinned GGUF catalog entry");
    }
    validate_cached(dir, verify)
}

pub fn validate_selector(name: &str) -> Result<()> {
    if let Some(model) = catalog_model(name) {
        match model {
            CatalogModel::Hf {
                alias: "aligner", ..
            } => {
                bail!(
                    "The aligner model only predicts timestamps. Select 0.6b, 1.7b or 0.6b-q8 for speech recognition."
                );
            }
            CatalogModel::Native(model) if model.role == NativeRole::Aligner => {
                bail!(
                    "The aligner model only predicts timestamps. Select 0.6b, 1.7b or 0.6b-q8 for speech recognition."
                );
            }
            _ => return Ok(()),
        }
    }
    let path = Path::new(name);
    if path.is_file() {
        if has_gguf_magic(path)? {
            return Ok(());
        }
        bail!("Local model file is not GGUF (expected the GGUF magic header): {name}");
    }
    for filename in ["config.json", "processor_config.json", "tokenizer.json"] {
        if !path.join(filename).is_file() {
            bail!(
                "Local ASR model is missing {filename}: {name}. Use 0.6b, 1.7b, 0.6b-q8 or a complete official *-hf snapshot."
            );
        }
    }
    let cfg: Value = serde_json::from_slice(&fs::read(path.join("config.json"))?)
        .context("Invalid local model config.json")?;
    if !cfg["architectures"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|v| v == "Qwen3ASRForConditionalGeneration")
    }) {
        bail!("Local model is not a native Transformers Qwen3-ASR recognition checkpoint");
    }
    if !path.join("model.safetensors").is_file() {
        let index: Value = serde_json::from_slice(
            &fs::read(path.join("model.safetensors.index.json"))
                .context("Local ASR model has no safetensors weights")?,
        )?;
        let files = index["weight_map"]
            .as_object()
            .context("Invalid safetensors shard index")?;
        if files.is_empty() {
            bail!("Empty safetensors shard index");
        }
        for file in files.values() {
            let filename = file.as_str().context("Invalid shard filename")?;
            if !safe_relative(filename) || !path.join(filename).is_file() {
                bail!("Missing or invalid model shard: {filename}");
            }
        }
    }
    Ok(())
}

fn directory(repo: &str, cache: &Path) -> PathBuf {
    cache.join(repo.replace('/', "--"))
}

fn safe_relative(name: &str) -> bool {
    !name.contains('\\')
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && !name.is_empty()
        && !name.contains(':')
}

fn wanted(name: &str) -> bool {
    safe_relative(name)
        && [
            ".json",
            ".safetensors",
            ".txt",
            ".model",
            ".tiktoken",
            ".jinja",
        ]
        .iter()
        .any(|ext| name.ends_with(ext))
}

fn validate_cached(dir: &Path, verify: bool) -> Result<()> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("qwen3asr-model.json"))?)?;
    if manifest.files.is_empty() {
        bail!("Empty model manifest");
    }
    for entry in manifest.files {
        if !safe_relative(&entry.name) {
            bail!("Unsafe path in model manifest");
        }
        let path = dir.join(&entry.name);
        if fs::metadata(&path)?.len() != entry.size {
            bail!("Model file incomplete: {}", path.display());
        }
        if verify {
            let expected=entry.sha256.context("Legacy model manifest lacks a checksum; run models download again online to refresh its file checksums")?;
            if sha256(&path)? != expected {
                bail!("Model checksum mismatch: {}", path.display());
            }
        }
    }
    Ok(())
}

fn validate_hf_manifest(repository: &str, dir: &Path, verify: bool) -> Result<()> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("qwen3asr-model.json"))?)?;
    if manifest.repository != repository
        || manifest.format != ModelFormat::Safetensors
        || manifest.revision.len() != 40
        || !manifest.revision.chars().all(|c| c.is_ascii_hexdigit())
    {
        bail!("Model manifest provenance does not match the requested Hugging Face repository");
    }
    validate_cached(dir, verify)
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if crate::config::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Checksum verification cancelled");
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub fn list(cache: &Path) -> Value {
    let mut entries = CATALOG
        .iter()
        .map(|(name, repository)| {
            let dir = directory(repository, cache);
            json!({
                "name":name,
                "repository":repository,
                "path":dir,
                "downloaded":validate_hf_manifest(repository,&dir, false).is_ok(),
                "format":"safetensors",
                "format_provenance":hf_provenance(repository),
            })
        })
        .collect::<Vec<_>>();
    entries.extend(NATIVE_CATALOG.iter().map(|model| {
        let dir = native_directory(model, cache);
        json!({
            "name":model.alias,
            "repository":GGUF_REPOSITORY,
            "revision":GGUF_REVISION,
            "path":native_file(model, cache),
            "downloaded":validate_native_manifest(model, &dir, false).is_ok(),
            "format":"gguf",
            "format_provenance":native_provenance(model),
        })
    }));
    json!(entries)
}

pub fn resolve(name: &str, cache: &Path, offline: bool) -> Result<PathBuf> {
    let Some(model) = catalog_model(name) else {
        let path = PathBuf::from(name);
        if path.is_file() {
            if has_gguf_magic(&path)? {
                return path
                    .canonicalize()
                    .context("Cannot resolve local GGUF model file");
            }
            bail!("Local model file is not GGUF: {name}");
        }
        if path.join("config.json").is_file() {
            return path
                .canonicalize()
                .context("Cannot resolve local model directory");
        }
        bail!(
            "Unknown model {name}. Use 0.6b, 1.7b, 0.6b-q8, aligner, aligner-q8 or a local model path."
        );
    };
    match model {
        CatalogModel::Hf { repository, .. } => {
            let dir = directory(repository, cache);
            if validate_hf_manifest(repository, &dir, false).is_ok() {
                return Ok(dir.canonicalize()?);
            }
            if offline {
                bail!(
                    "Model {repository} is not fully cached. Run qwen3asr models download {name} while online."
                );
            }
            download(name, cache)?;
            Ok(dir.canonicalize()?)
        }
        CatalogModel::Native(model) => {
            let dir = native_directory(model, cache);
            let file = native_file(model, cache);
            if validate_native_manifest(model, &dir, false).is_ok() {
                return Ok(file.canonicalize()?);
            }
            if offline {
                bail!(
                    "Native GGUF model {} is not fully cached. Run qwen3asr models download {} while online.",
                    model.alias,
                    model.alias
                );
            }
            download(name, cache)?;
            Ok(file.canonicalize()?)
        }
    }
}

pub fn verify(name: &str, cache: &Path) -> Result<Value> {
    match catalog_model(name).context("verify supports catalog models only")? {
        CatalogModel::Hf { repository, .. } => {
            let dir = directory(repository, cache);
            validate_hf_manifest(repository, &dir, true)?;
            Ok(json!({
                "model":name,
                "verified":true,
                "repository":repository,
                "path":dir,
                "format":"safetensors",
                "format_provenance":hf_provenance(repository),
            }))
        }
        CatalogModel::Native(model) => {
            let dir = native_directory(model, cache);
            let file = native_file(model, cache);
            validate_native_manifest(model, &dir, true)?;
            Ok(json!({
                "model":name,
                "verified":true,
                "repository":GGUF_REPOSITORY,
                "revision":GGUF_REVISION,
                "path":file,
                "sha256":model.sha256,
                "format":"gguf",
                "format_provenance":native_provenance(model),
            }))
        }
    }
}

pub fn download(name: &str, cache: &Path) -> Result<Value> {
    match catalog_model(name)
        .context("Download model must be 0.6b, 1.7b, aligner, 0.6b-q8 or aligner-q8")?
    {
        CatalogModel::Hf { repository, .. } => download_hf(name, repository, cache),
        CatalogModel::Native(model) => download_native(model, cache),
    }
}

fn download_hf(name: &str, repo: &str, cache: &Path) -> Result<Value> {
    let dir = directory(repo, cache);
    fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".download.lock"))?;
    lock.try_lock_exclusive()
        .context("Another process is downloading this model. Wait for it to finish.")?;
    if validate_hf_manifest(repo, &dir, true).is_ok() {
        return Ok(json!({"model":name,"path":dir,"cached":true}));
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(7200))
        .user_agent(concat!("qwen3asr-cli/", env!("CARGO_PKG_VERSION")))
        .build()?;
    eprintln!("Downloading official model {repo}");
    let pinned = ["qwen3asr-model.json", "qwen3asr-download.json"]
        .iter()
        .find_map(|file| {
            fs::read(dir.join(file))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
                .filter(|manifest| {
                    manifest.repository == repo
                        && manifest.revision.len() == 40
                        && manifest.revision.chars().all(|c| c.is_ascii_hexdigit())
                })
        });
    let endpoint = match pinned {
        Some(manifest) => format!(
            "https://huggingface.co/api/models/{repo}/revision/{}?blobs=true",
            manifest.revision
        ),
        None => format!("https://huggingface.co/api/models/{repo}?blobs=true"),
    };
    let info: Value = client.get(endpoint).send()?.error_for_status()?.json()?;
    let revision = info["sha"]
        .as_str()
        .context("Hub response missing model revision")?
        .to_owned();
    if revision.len() != 40 || !revision.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("Invalid Hub revision");
    }
    let mut files: Vec<ModelFile> = info["siblings"]
        .as_array()
        .context("Hub response missing files")?
        .iter()
        .filter_map(|item| {
            let name = item["rfilename"].as_str()?;
            if !wanted(name) {
                return None;
            }
            Some(ModelFile {
                name: name.into(),
                size: item["size"].as_u64().unwrap_or(0),
                sha256: item["lfs"]["sha256"].as_str().map(str::to_owned),
            })
        })
        .collect();
    if !files.iter().any(|f| f.name.ends_with(".safetensors"))
        || !files.iter().any(|f| f.name == "config.json")
    {
        bail!("Hub returned no model weights/configuration");
    }
    // Pin interrupted downloads before fetching weights so a later retry cannot mix revisions.
    let mut pending = tempfile::NamedTempFile::new_in(&dir)?;
    pending.write_all(&serde_json::to_vec(
        &json!({"repository":repo,"revision":revision,"files":files}),
    )?)?;
    pending.persist(dir.join("qwen3asr-download.json"))?;
    for entry in &mut files {
        if crate::config::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Download cancelled; partial files retained");
        }
        if entry.size == 0 {
            bail!("Hub returned no size for {}", entry.name);
        }
        let destination = dir.join(&entry.name);
        if destination.is_file()
            && destination.metadata()?.len() == entry.size
            && entry
                .sha256
                .as_ref()
                .is_some_and(|expected| sha256(&destination).is_ok_and(|hash| hash == *expected))
        {
            continue;
        }
        let part = destination.with_file_name(format!(
            "{}.part",
            destination.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir_all(destination.parent().unwrap())?;
        if part.exists() && part.metadata()?.len() > entry.size {
            fs::remove_file(&part)?;
        }
        let remaining = entry
            .size
            .saturating_sub(part.metadata().map(|v| v.len()).unwrap_or(0));
        if fs2::available_space(&dir)? < remaining + 256 * 1024 * 1024 {
            bail!(
                "Insufficient free disk space for {} (need {} MiB)",
                entry.name,
                remaining / 1024 / 1024 + 256
            );
        }
        let url = format!(
            "https://huggingface.co/{repo}/resolve/{revision}/{}",
            entry.name
        );
        let mut success = false;
        for attempt in 0..3 {
            match download_file(&client, &url, &part, entry) {
                Ok(()) => {
                    success = true;
                    break;
                }
                Err(error) => {
                    if crate::config::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err(error);
                    }
                    if attempt == 2 {
                        return Err(error).with_context(|| format!("Download failed: {}. Partial data retained; rerun models download.", entry.name));
                    }
                    eprintln!(
                        "Download interrupted: {error}. Retrying ({}/2).",
                        attempt + 1
                    );
                    std::thread::sleep(Duration::from_secs(2 << attempt));
                }
            }
        }
        if success {
            if destination.exists() {
                fs::remove_file(&destination)?;
            }
            fs::rename(&part, &destination)?;
        }
        // Hub LFS supplies authoritative weight hashes. Non-LFS files arrive
        // over HTTPS from the same pinned revision; persist their local SHA-256
        // too, so future offline verify detects same-size corruption.
        if entry.sha256.is_none() {
            entry.sha256 = Some(sha256(&destination)?);
        }
    }
    let manifest = Manifest {
        repository: repo.into(),
        revision,
        files,
        format: ModelFormat::Safetensors,
        source_file: None,
    };
    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
    temp.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    temp.persist(dir.join("qwen3asr-model.json"))?;
    fs::remove_file(dir.join("qwen3asr-download.json"))?;
    Ok(json!({"model":name,"path":dir,"cached":false,"revision":manifest.revision}))
}

fn download_native(model: &NativeModel, cache: &Path) -> Result<Value> {
    let dir = native_directory(model, cache);
    fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".download.lock"))?;
    lock.try_lock_exclusive()
        .context("Another process is downloading this model. Wait for it to finish.")?;

    let destination = native_file(model, cache);
    if validate_native_manifest(model, &dir, true).is_ok() {
        return Ok(json!({
            "model":model.alias,
            "path":destination,
            "cached":true,
            "revision":GGUF_REVISION,
            "sha256":model.sha256,
            "format":"gguf",
            "format_provenance":native_provenance(model),
        }));
    }

    // A verified GGUF already present in the cache can be adopted without a second download.
    if destination.is_file()
        && destination.metadata()?.len() == model.size
        && sha256(&destination)? == model.sha256
    {
        write_native_manifest(model, &dir)?;
        return Ok(json!({
            "model":model.alias,
            "path":destination,
            "cached":true,
            "revision":GGUF_REVISION,
            "sha256":model.sha256,
            "format":"gguf",
            "format_provenance":native_provenance(model),
        }));
    }

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(7200))
        .user_agent(concat!("qwen3asr-cli/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let file = ModelFile {
        name: model.filename.to_owned(),
        size: model.size,
        sha256: Some(model.sha256.to_owned()),
    };
    let part = destination.with_file_name(format!("{}.part", model.filename));
    if part.exists() && part.metadata()?.len() > model.size {
        fs::remove_file(&part)?;
    }
    let remaining = model
        .size
        .saturating_sub(part.metadata().map(|v| v.len()).unwrap_or(0));
    if fs2::available_space(&dir)? < remaining + 256 * 1024 * 1024 {
        bail!(
            "Insufficient free disk space for {} (need {} MiB)",
            model.filename,
            remaining / 1024 / 1024 + 256
        );
    }
    let url = format!(
        "https://huggingface.co/{GGUF_REPOSITORY}/resolve/{GGUF_REVISION}/{}",
        model.source_file
    );
    eprintln!("Downloading pinned native GGUF model {}", model.alias);
    for attempt in 0..3 {
        match download_file(&client, &url, &part, &file) {
            Ok(()) => break,
            Err(error) => {
                if crate::config::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(error);
                }
                if attempt == 2 {
                    return Err(error).with_context(|| {
                        format!(
                            "Download failed: {}. Partial data retained; rerun models download.",
                            model.filename
                        )
                    });
                }
                if part.metadata().is_ok_and(|v| v.len() > model.size) {
                    fs::remove_file(&part)?;
                }
                eprintln!(
                    "Download interrupted: {error}. Retrying ({}/2).",
                    attempt + 1
                );
                std::thread::sleep(Duration::from_secs(2 << attempt));
            }
        }
    }
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    fs::rename(&part, &destination)?;
    write_native_manifest(model, &dir)?;
    Ok(json!({
        "model":model.alias,
        "path":destination,
        "cached":false,
        "revision":GGUF_REVISION,
        "sha256":model.sha256,
        "format":"gguf",
        "format_provenance":native_provenance(model),
    }))
}

fn write_native_manifest(model: &NativeModel, dir: &Path) -> Result<()> {
    let manifest = Manifest {
        repository: GGUF_REPOSITORY.into(),
        revision: GGUF_REVISION.into(),
        files: vec![ModelFile {
            name: model.filename.to_owned(),
            size: model.size,
            sha256: Some(model.sha256.to_owned()),
        }],
        format: ModelFormat::Gguf,
        source_file: Some(model.source_file.to_owned()),
    };
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    temp.persist(dir.join("qwen3asr-model.json"))?;
    Ok(())
}

fn download_file(client: &Client, url: &str, part: &Path, entry: &ModelFile) -> Result<()> {
    let offset = part.metadata().map(|v| v.len()).unwrap_or(0);
    if offset < entry.size {
        let mut request = client.get(url);
        if offset > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
        }
        let mut response = request.send()?.error_for_status()?;
        let append = offset > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if append {
            let range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !range.starts_with(&format!("bytes {offset}-")) {
                bail!("Server returned unexpected resume range");
            }
        }
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(part)?;
        eprintln!(
            "  {} ({:.1} MiB, resume {:.1} MiB)",
            entry.name,
            entry.size as f64 / 1048576.0,
            if append { offset } else { 0 } as f64 / 1048576.0
        );
        let mut buffer = vec![0; 1024 * 1024];
        let mut last_report = std::time::Instant::now();
        let mut downloaded = if append { offset } else { 0 };
        loop {
            if crate::config::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                bail!("Download cancelled; partial files retained");
            }
            let bytes = response.read(&mut buffer)?;
            if bytes == 0 {
                break;
            }
            out.write_all(&buffer[..bytes])?;
            downloaded += bytes as u64;
            if downloaded > entry.size {
                bail!("Server returned more data than the declared model size");
            }
            if last_report.elapsed() > Duration::from_secs(5) {
                eprintln!(
                    "    {:.1}% ({:.0}/{:.0} MiB)",
                    downloaded as f64 * 100.0 / entry.size as f64,
                    downloaded as f64 / 1048576.0,
                    entry.size as f64 / 1048576.0
                );
                last_report = std::time::Instant::now();
            }
        }
        out.sync_all()?;
    }
    if part.metadata()?.len() != entry.size {
        bail!("Downloaded size mismatch");
    }
    if let Some(expected) = &entry.sha256
        && sha256(part)? != *expected
    {
        fs::remove_file(part)?;
        bail!("SHA256 mismatch; corrupt partial file removed for retry");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal() {
        for value in ["../a", "/a", "C:/a", "x\\a", ""] {
            assert!(!safe_relative(value));
        }
        assert!(safe_relative("dir/config.json"));
    }
    #[test]
    fn no_remote_executable_code() {
        assert!(!wanted("model.py"));
        assert!(!wanted("model.bin"));
        assert!(wanted("model.safetensors"));
    }
    #[test]
    fn missing_offline_has_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            resolve("0.6b", dir.path(), true)
                .unwrap_err()
                .to_string()
                .contains("models download")
        );
    }
    #[test]
    fn hf_verify_binds_repository_and_requires_every_file_checksum() {
        let cache = tempfile::tempdir().unwrap();
        let repo = CATALOG[0].1;
        let dir = directory(repo, cache.path());
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json");
        fs::write(&config, b"{}").unwrap();
        let mut manifest = Manifest {
            repository: CATALOG[1].1.into(),
            revision: "a".repeat(40),
            files: vec![ModelFile {
                name: "config.json".into(),
                size: 2,
                sha256: Some(sha256(&config).unwrap()),
            }],
            format: ModelFormat::Safetensors,
            source_file: None,
        };
        let save = |value: &Manifest| {
            fs::write(
                dir.join("qwen3asr-model.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap()
        };
        save(&manifest);
        assert!(verify("0.6b", cache.path()).is_err());
        manifest.repository = repo.into();
        manifest.files[0].sha256 = None;
        save(&manifest);
        assert!(verify("0.6b", cache.path()).is_err());
        manifest.files[0].sha256 = Some(sha256(&config).unwrap());
        save(&manifest);
        assert!(verify("0.6b", cache.path()).is_ok());
        fs::write(&config, b"[]").unwrap();
        assert!(verify("0.6b", cache.path()).is_err());
    }
    #[test]
    fn validates_model_selection_without_downloading() {
        assert!(validate_selector("no-such-model").is_err());
        assert!(validate_selector("0.6b-q8").is_ok());
        assert!(
            validate_selector("aligner-q8")
                .unwrap_err()
                .to_string()
                .contains("aligner model")
        );
        assert!(validate_selector("1.7b").is_ok());
        assert!(validate_selector("aligner").is_err());
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), b"{}").unwrap();
        assert!(validate_selector(dir.path().to_str().unwrap()).is_err());
    }
    #[test]
    fn accepts_local_gguf_magic_and_resolves_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("local.gguf");
        fs::write(&model, b"GGUF\x03\x00\x00\x00").unwrap();

        assert!(validate_selector(model.to_str().unwrap()).is_ok());
        assert_eq!(
            resolve(model.to_str().unwrap(), dir.path(), true).unwrap(),
            model.canonicalize().unwrap()
        );
    }

    #[test]
    fn rejects_local_model_file_without_gguf_magic() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("not-a-model.bin");
        fs::write(&model, b"NOPE\x00\x00\x00\x00").unwrap();

        assert!(validate_selector(model.to_str().unwrap()).is_err());
        assert!(resolve(model.to_str().unwrap(), dir.path(), true).is_err());
    }

    #[test]
    fn native_catalog_pins_hub_source_and_gguf_checksums() {
        assert_eq!(GGUF_REVISION.len(), 40);
        let asr = catalog_model("0.6B-Q8").unwrap();
        let CatalogModel::Native(asr) = asr else {
            panic!("expected native ASR catalog entry");
        };
        assert_eq!(asr.role, NativeRole::Asr);
        assert_eq!(asr.size, 1_151_272_416);
        assert_eq!(
            asr.sha256,
            "6c44ec2fb4cee513892d7863c1fcc3ea6b699ffa4d899b0ef4ab19956d9544f7"
        );

        let aligner = catalog_model("aligner-q8").unwrap();
        let CatalogModel::Native(aligner) = aligner else {
            panic!("expected native aligner catalog entry");
        };
        assert_eq!(aligner.role, NativeRole::Aligner);
        assert_eq!(aligner.size, 1_129_966_496);
        assert_eq!(
            aligner.sha256,
            "75209490b11cec2b0db749ca5f4ff92266f58efd30f7fd04d9eb2a3ac9cc929f"
        );

        let entries = list(Path::new("cache"));
        let entries = entries.as_array().unwrap();
        assert!(entries.iter().any(|item| {
            item["name"] == "0.6b-q8"
                && item["format"] == "gguf"
                && item["revision"] == GGUF_REVISION
                && item["format_provenance"]["source_file"] == asr.source_file
        }));
        assert!(
            entries
                .iter()
                .any(|item| item["name"] == "0.6b" && item["format"] == "safetensors")
        );
    }

    #[test]
    fn native_cache_requires_matching_manifest_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = native_directory(&NATIVE_CATALOG[0], dir.path());
        fs::create_dir_all(&model_dir).unwrap();
        let wrong_manifest = json!({
            "repository": GGUF_REPOSITORY,
            "revision": "0".repeat(40),
            "format": "safetensors",
            "source_file": NATIVE_CATALOG[0].source_file,
            "files": [{
                "name": NATIVE_CATALOG[0].filename,
                "size": NATIVE_CATALOG[0].size,
                "sha256": NATIVE_CATALOG[0].sha256,
            }],
        });
        fs::write(
            model_dir.join("qwen3asr-model.json"),
            serde_json::to_vec(&wrong_manifest).unwrap(),
        )
        .unwrap();

        assert!(validate_native_manifest(&NATIVE_CATALOG[0], &model_dir, false).is_err());
        assert!(!list(dir.path())[3]["downloaded"].as_bool().unwrap());
    }

    #[test]
    fn full_check_detects_same_size_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let weights = dir.path().join("model.safetensors");
        fs::write(&weights, b"correct").unwrap();
        let manifest = Manifest {
            repository: CATALOG[0].1.into(),
            revision: "0".repeat(40),
            files: vec![ModelFile {
                name: "model.safetensors".into(),
                size: 7,
                sha256: Some(sha256(&weights).unwrap()),
            }],
            format: ModelFormat::Safetensors,
            source_file: None,
        };
        fs::write(
            dir.path().join("qwen3asr-model.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(&weights, b"corrupt").unwrap();
        assert!(validate_cached(dir.path(), false).is_ok());
        assert!(validate_cached(dir.path(), true).is_err());
    }
}

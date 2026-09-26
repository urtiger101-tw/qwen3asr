use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tempfile::NamedTempFile;
use toml_edit::{DocumentMut, Item, Table, value};

const SERVER_NAME: &str = "qwen3asr";
const SKILL_MARKER: &str = "<!-- qwen3asr-managed-skill -->";
const OWNERSHIP_MARKER: &str = "qwen3asr-agent-integration-v1";
const SKILL: &str = include_str!("../skills/qwen3asr/SKILL.md");
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not locate the current user's home directory")
}

fn codex_home() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(path));
    }
    Ok(home_dir()?.join(".codex"))
}

fn claude_config_dir(home: &Path, override_dir: Option<&Path>) -> PathBuf {
    override_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".claude"))
}

fn targets(target: &str) -> Result<Vec<&'static str>> {
    match target {
        "codex" => Ok(vec!["codex"]),
        "agy" => Ok(vec!["agy"]),
        "claude" => Ok(vec!["claude"]),
        "all" => Ok(vec!["codex", "agy", "claude"]),
        _ => bail!("target must be codex, agy, claude, or all"),
    }
}

fn ensure_safe_destination(path: &Path) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    #[cfg(windows)]
    let is_reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let is_reparse = metadata.file_type().is_symlink();
    if is_reparse {
        bail!(
            "Refusing to modify symbolic link or reparse-point destination {}; replace it with a regular file or manage its target directly",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!(
            "Refusing to modify non-file destination {}; expected a regular file",
            path.display()
        );
    }
    Ok(true)
}

fn backup_path(path: &Path) -> PathBuf {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        "{}.bak-{stamp}-{}-{sequence}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ))
}

fn create_backup(path: &Path) -> Result<PathBuf> {
    let contents = fs::read(path)?;
    loop {
        let backup = backup_path(path);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                file.write_all(&contents)?;
                file.sync_all()?;
                return Ok(backup);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Could not create backup {}", backup.display()));
            }
        }
    }
}

fn save_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_safe_destination(path)?;
    let parent = path.parent().context("Configuration path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.persist(path)
        .with_context(|| format!("Could not replace {}", path.display()))?;
    Ok(())
}

fn codex_entry(executable: &Path) -> Value {
    json!({"command":executable.to_string_lossy(),"args":["mcp"]})
}

fn claude_entry(executable: &Path) -> Value {
    json!({"type":"stdio","command":executable.to_string_lossy(),"args":["mcp"],"env":{}})
}

fn agy_entry(executable: &Path) -> Value {
    json!({"args":["mcp"],"command":executable.to_string_lossy(),"disabled":false})
}

fn integration_entry(client: &str, executable: &Path) -> Result<Value> {
    match client {
        "codex" => Ok(codex_entry(executable)),
        "agy" => Ok(agy_entry(executable)),
        "claude" => Ok(claude_entry(executable)),
        _ => bail!("Unknown agent client: {client}"),
    }
}

fn ownership_path(client: &str, home: &Path) -> PathBuf {
    let base = if client == "codex" {
        codex_home().unwrap_or_else(|_| home.join(".codex"))
    } else {
        home.join(".qwen3asr")
    };
    base.join("agent-integrations")
        .join(format!("{client}.json"))
}

fn client_config_path(
    client: &str,
    home: &Path,
    claude_config_override: Option<&Path>,
) -> Result<PathBuf> {
    match client {
        "codex" => Ok(codex_home()?.join("config.toml")),
        "agy" => Ok(home.join(".gemini").join("config").join("mcp_config.json")),
        "claude" => Ok(claude_config_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.to_path_buf())
            .join(".claude.json")),
        _ => bail!("Unknown agent client: {client}"),
    }
}

fn install_client_config(
    client: &str,
    path: &Path,
    executable: &Path,
    dry_run: bool,
) -> Result<Value> {
    if client == "codex" {
        install_toml_mcp(path, executable, dry_run)
    } else {
        install_json_mcp(
            path,
            client,
            integration_entry(client, executable)?,
            dry_run,
        )
    }
}

fn uninstall_client_config(
    client: &str,
    path: &Path,
    owned_entry: Option<&Value>,
    dry_run: bool,
) -> Result<Value> {
    if client == "codex" {
        uninstall_toml_mcp(path, owned_entry, dry_run)
    } else {
        uninstall_json_mcp(path, client, owned_entry, dry_run)
    }
}

fn manifest_value(client: &str, config: &Path, entry: Value) -> Result<Value> {
    let config = std::path::absolute(config)?;
    Ok(json!({
        "managed_by":OWNERSHIP_MARKER,
        "client":client,
        "config":config.to_string_lossy(),
        "entry":entry,
    }))
}

fn validate_manifest(path: &Path, client: &str, config: &Path) -> Result<Option<Value>> {
    if !ensure_safe_destination(path)? {
        return Ok(None);
    }
    let manifest: Value = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("Invalid qwen3asr ownership manifest {}", path.display()))?;
    if manifest["managed_by"].as_str() != Some(OWNERSHIP_MARKER)
        || manifest["client"].as_str() != Some(client)
        || manifest["config"].as_str()
            != Some(std::path::absolute(config)?.to_string_lossy().as_ref())
    {
        bail!(
            "Ownership manifest {} does not match this {client} configuration; preserving it",
            path.display()
        );
    }
    let entry = manifest
        .get("entry")
        .context("Ownership manifest is missing its entry")?
        .clone();
    let args = entry.get("args").and_then(Value::as_array);
    let command = entry.get("command").and_then(Value::as_str);
    let valid_shape = command.is_some_and(|command| {
        Path::new(command)
            .file_stem()
            .is_some_and(|stem| stem.eq_ignore_ascii_case("qwen3asr"))
    }) && args.is_some_and(|args| args.as_slice() == [json!("mcp")])
        && match client {
            "codex" => entry.as_object().is_some_and(|object| object.len() == 2),
            "agy" => {
                entry.as_object().is_some_and(|object| object.len() == 3)
                    && entry.get("disabled").and_then(Value::as_bool) == Some(false)
            }
            "claude" => {
                entry.as_object().is_some_and(|object| object.len() == 4)
                    && entry.get("type").and_then(Value::as_str) == Some("stdio")
                    && entry.get("env").is_some_and(Value::is_object)
            }
            _ => false,
        };
    if !valid_shape {
        bail!(
            "Ownership manifest {} contains an invalid {client} server entry; preserving it",
            path.display()
        );
    }
    Ok(Some(entry))
}

fn install_manifest(
    path: &Path,
    client: &str,
    config: &Path,
    entry: Value,
    dry_run: bool,
) -> Result<Value> {
    let exists = ensure_safe_destination(path)?;
    let manifest = manifest_value(client, config, entry)?;
    if exists {
        let previous: Value = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("Invalid qwen3asr ownership manifest {}", path.display()))?;
        if previous["managed_by"].as_str() != Some(OWNERSHIP_MARKER) {
            bail!(
                "Unrecognized ownership file at {}; preserving it",
                path.display()
            );
        }
        if previous == manifest {
            return Ok(json!({"client":client,"action":"unchanged","ownership":path}));
        }
    }
    let backup = if exists {
        Some(backup_path(path))
    } else {
        None
    };
    if dry_run {
        return Ok(
            json!({"client":client,"action":"would_install","ownership":path,"backup":backup}),
        );
    }
    let backup = if exists {
        Some(create_backup(path)?)
    } else {
        None
    };
    save_atomic(path, serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    Ok(json!({"client":client,"action":"installed","ownership":path,"backup":backup}))
}

fn uninstall_manifest(path: &Path, client: &str, config: &Path, dry_run: bool) -> Result<Value> {
    let Some(entry) = validate_manifest(path, client, config)? else {
        return Ok(json!({"client":client,"action":"absent","ownership":path}));
    };
    let backup = backup_path(path);
    if dry_run {
        return Ok(
            json!({"client":client,"action":"would_uninstall","ownership":path,"entry":entry,"backup":backup}),
        );
    }
    let backup = create_backup(path)?;
    fs::remove_file(path)?;
    Ok(json!({"client":client,"action":"uninstalled","ownership":path,"backup":backup}))
}

fn install_toml_mcp(path: &Path, executable: &Path, dry_run: bool) -> Result<Value> {
    let exists = ensure_safe_destination(path)?;
    let original = if exists { fs::read(path)? } else { Vec::new() };
    let mut doc = if original.is_empty() {
        DocumentMut::new()
    } else {
        std::str::from_utf8(&original)?
            .parse::<DocumentMut>()
            .with_context(|| format!("Invalid TOML in {}", path.display()))?
    };

    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    let servers = servers
        .as_table_mut()
        .context("mcp_servers must be a TOML table")?;
    let executable = executable.to_string_lossy().to_string();
    let mut server = Table::new();
    server["command"] = value(executable.clone());
    let mut args = toml_edit::Array::new();
    args.push("mcp");
    server["args"] = Item::Value(args.into());

    if let Some(existing) = servers.get(SERVER_NAME) {
        let matches = existing.as_table().is_some_and(|table| {
            table.iter().count() == 2
                && table.get("command").and_then(Item::as_str) == Some(executable.as_str())
                && table
                    .get("args")
                    .and_then(Item::as_array)
                    .is_some_and(|args| {
                        args.len() == 1
                            && args.get(0).and_then(toml_edit::Value::as_str) == Some("mcp")
                    })
        });
        if matches {
            return Ok(json!({"client":"codex","action":"unchanged","config":path}));
        }
        bail!(
            "A different `{SERVER_NAME}` MCP entry already exists in {}",
            path.display()
        );
    }

    servers.insert(SERVER_NAME, Item::Table(server));
    let backup = if exists {
        Some(backup_path(path))
    } else {
        None
    };
    if dry_run {
        return Ok(
            json!({"client":"codex","action":"would_install","config":path,"backup":backup}),
        );
    }
    let backup = if exists {
        Some(create_backup(path)?)
    } else {
        None
    };
    save_atomic(path, doc.to_string().as_bytes())?;
    Ok(json!({"client":"codex","action":"installed","config":path,"backup":backup}))
}

fn uninstall_toml_mcp(path: &Path, owned_entry: Option<&Value>, dry_run: bool) -> Result<Value> {
    let exists = ensure_safe_destination(path)?;
    if !exists {
        return Ok(json!({"client":"codex","action":"absent","config":path}));
    }
    let original = fs::read(path)?;
    let mut doc = std::str::from_utf8(&original)?
        .parse::<DocumentMut>()
        .with_context(|| format!("Invalid TOML in {}", path.display()))?;
    let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return Ok(json!({"client":"codex","action":"absent","config":path}));
    };
    let Some(existing) = servers.get(SERVER_NAME) else {
        return Ok(json!({"client":"codex","action":"absent","config":path}));
    };
    let observed = existing.as_table().and_then(|table| {
        if table.iter().count() != 2 {
            return None;
        }
        let command = table.get("command").and_then(Item::as_str)?;
        let args = table.get("args").and_then(Item::as_array)?;
        (args.len() == 1 && args.get(0).and_then(toml_edit::Value::as_str) == Some("mcp"))
            .then(|| json!({"command":command,"args":["mcp"]}))
    });
    let is_owned = owned_entry.is_some() && observed.as_ref() == owned_entry;
    if !is_owned {
        bail!(
            "The named `{SERVER_NAME}` MCP entry in {} was changed; preserving it",
            path.display()
        );
    }
    let backup = backup_path(path);
    if dry_run {
        return Ok(
            json!({"client":"codex","action":"would_uninstall","config":path,"backup":backup}),
        );
    }
    let backup = create_backup(path)?;
    servers.remove(SERVER_NAME);
    save_atomic(path, doc.to_string().as_bytes())?;
    Ok(json!({"client":"codex","action":"uninstalled","config":path,"backup":backup}))
}

fn install_json_mcp(path: &Path, client: &str, entry: Value, dry_run: bool) -> Result<Value> {
    let exists = ensure_safe_destination(path)?;
    let original = if exists {
        fs::read(path)?
    } else {
        b"{}".to_vec()
    };
    let mut root: Value = serde_json::from_slice(&original)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    let object = root
        .as_object_mut()
        .context("MCP config root must be an object")?;
    let servers = object.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .context("mcpServers must be an object")?;
    if let Some(existing) = servers.get(SERVER_NAME) {
        if existing == &entry {
            return Ok(json!({"client":client,"action":"unchanged","config":path}));
        }
        bail!(
            "A different `{SERVER_NAME}` MCP entry already exists in {}",
            path.display()
        );
    }
    servers.insert(SERVER_NAME.into(), entry);
    let backup = if exists {
        Some(backup_path(path))
    } else {
        None
    };
    if dry_run {
        return Ok(json!({"client":client,"action":"would_install","config":path,"backup":backup}));
    }
    let backup = if exists {
        Some(create_backup(path)?)
    } else {
        None
    };
    save_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(json!({"client":client,"action":"installed","config":path,"backup":backup}))
}

fn uninstall_json_mcp(
    path: &Path,
    client: &str,
    owned_entry: Option<&Value>,
    dry_run: bool,
) -> Result<Value> {
    let exists = ensure_safe_destination(path)?;
    if !exists {
        return Ok(json!({"client":client,"action":"absent","config":path}));
    }
    let original = fs::read(path)?;
    let mut root: Value = serde_json::from_slice(&original)
        .with_context(|| format!("Invalid JSON in {}", path.display()))?;
    let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return Ok(json!({"client":client,"action":"absent","config":path}));
    };
    let Some(existing) = servers.get(SERVER_NAME) else {
        return Ok(json!({"client":client,"action":"absent","config":path}));
    };
    if owned_entry.is_none_or(|owned| owned != existing) {
        bail!(
            "The named `{SERVER_NAME}` MCP entry in {} was changed; preserving it",
            path.display()
        );
    }
    let backup = backup_path(path);
    if dry_run {
        return Ok(
            json!({"client":client,"action":"would_uninstall","config":path,"backup":backup}),
        );
    }
    let backup = create_backup(path)?;
    servers.remove(SERVER_NAME);
    save_atomic(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(json!({"client":client,"action":"uninstalled","config":path,"backup":backup}))
}

fn skill_path(client: &str, home: &Path, claude_config_override: Option<&Path>) -> PathBuf {
    match client {
        "codex" => codex_home()
            .unwrap_or_else(|_| home.join(".codex"))
            .join("skills")
            .join(SERVER_NAME)
            .join("SKILL.md"),
        "claude" => claude_config_dir(home, claude_config_override)
            .join("skills")
            .join(SERVER_NAME)
            .join("SKILL.md"),
        "agy" => home
            .join(".agents")
            .join("skills")
            .join(SERVER_NAME)
            .join("SKILL.md"),
        _ => unreachable!(),
    }
}

fn install_skill(path: &Path, dry_run: bool) -> Result<Value> {
    let body = skill_body();
    let exists = ensure_safe_destination(path)?;
    if exists {
        let previous = fs::read_to_string(path)?;
        if previous == body {
            return Ok(json!({"action":"unchanged","skill":path}));
        }
        if !previous.contains(SKILL_MARKER) {
            bail!(
                "A non-qwen3asr Skill already exists at {}; preserving it",
                path.display()
            );
        }
    }
    let backup = if exists {
        Some(backup_path(path))
    } else {
        None
    };
    if dry_run {
        return Ok(json!({"action":"would_install","skill":path,"backup":backup}));
    }
    let backup = if exists {
        Some(create_backup(path)?)
    } else {
        None
    };
    save_atomic(path, body.as_bytes())?;
    Ok(json!({"action":"installed","skill":path,"backup":backup}))
}

fn skill_body() -> String {
    if SKILL.contains(SKILL_MARKER) {
        SKILL.to_string()
    } else {
        format!("{SKILL_MARKER}\n{SKILL}")
    }
}

fn uninstall_skill(path: &Path, dry_run: bool) -> Result<Value> {
    if !ensure_safe_destination(path)? {
        return Ok(json!({"action":"absent","skill":path}));
    }
    let body = fs::read_to_string(path)?;
    if body != skill_body() {
        bail!(
            "Skill at {} was modified or is not managed by qwen3asr; preserving it",
            path.display()
        );
    }
    let backup = backup_path(path);
    if dry_run {
        return Ok(json!({"action":"would_uninstall","skill":path,"backup":backup}));
    }
    let backup = create_backup(path)?;
    fs::remove_file(path)?;
    Ok(json!({"action":"uninstalled","skill":path,"backup":backup}))
}

pub fn install(target: &str, dry_run: bool) -> Result<Value> {
    let clients = targets(target)?;
    let home = home_dir()?;
    let claude_config_override = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let executable = std::env::current_exe().context("Could not locate qwen3asr executable")?;
    if !dry_run {
        for client in &clients {
            let config = client_config_path(client, &home, claude_config_override.as_deref())?;
            let entry = integration_entry(client, &executable)?;
            install_client_config(client, &config, &executable, true)?;
            install_manifest(&ownership_path(client, &home), client, &config, entry, true)?;
            install_skill(
                &skill_path(client, &home, claude_config_override.as_deref()),
                true,
            )?;
        }
    }
    let mut changes = Vec::new();
    for client in clients {
        let config = client_config_path(client, &home, claude_config_override.as_deref())?;
        let entry = integration_entry(client, &executable)?;
        changes.push(install_manifest(
            &ownership_path(client, &home),
            client,
            &config,
            entry,
            dry_run,
        )?);
        changes.push(install_client_config(
            client,
            &config,
            &executable,
            dry_run,
        )?);
        changes.push(install_skill(
            &skill_path(client, &home, claude_config_override.as_deref()),
            dry_run,
        )?);
    }
    Ok(json!({"target":target,"dry_run":dry_run,"changes":changes}))
}

pub fn uninstall(target: &str, dry_run: bool) -> Result<Value> {
    let clients = targets(target)?;
    let home = home_dir()?;
    let claude_config_override = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    if !dry_run {
        for client in &clients {
            let config = client_config_path(client, &home, claude_config_override.as_deref())?;
            let owner = ownership_path(client, &home);
            let entry = validate_manifest(&owner, client, &config)?;
            uninstall_client_config(client, &config, entry.as_ref(), true)?;
            uninstall_skill(
                &skill_path(client, &home, claude_config_override.as_deref()),
                true,
            )?;
            uninstall_manifest(&owner, client, &config, true)?;
        }
    }
    let mut changes = Vec::new();
    for client in clients {
        let config = client_config_path(client, &home, claude_config_override.as_deref())?;
        let owner = ownership_path(client, &home);
        let entry = validate_manifest(&owner, client, &config)?;
        changes.push(uninstall_client_config(
            client,
            &config,
            entry.as_ref(),
            dry_run,
        )?);
        changes.push(uninstall_skill(
            &skill_path(client, &home, claude_config_override.as_deref()),
            dry_run,
        )?);
        changes.push(uninstall_manifest(&owner, client, &config, dry_run)?);
    }
    Ok(json!({"target":target,"dry_run":dry_run,"changes":changes}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn codex_install_and_uninstall_only_touch_named_table() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("config.toml");
        fs::write(
            &config,
            "model = \"test\"\n[mcp_servers.keep]\ncommand = \"other\"\n",
        )
        .unwrap();
        let original = fs::read(&config).unwrap();
        let exe = PathBuf::from("C:/tools/qwen3asr.exe");
        let installed = install_toml_mcp(&config, &exe, false).unwrap();
        let backup = PathBuf::from(installed["backup"].as_str().unwrap());
        assert_eq!(fs::read(backup).unwrap(), original);
        let doc = fs::read_to_string(&config)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(doc["model"].as_str(), Some("test"));
        assert_eq!(
            doc["mcp_servers"]["keep"]["command"].as_str(),
            Some("other")
        );
        assert_eq!(
            doc["mcp_servers"][SERVER_NAME]["args"][0].as_str(),
            Some("mcp")
        );
        uninstall_toml_mcp(&config, Some(&codex_entry(&exe)), false).unwrap();
        let after = fs::read_to_string(&config)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert!(
            after["mcp_servers"]
                .as_table()
                .unwrap()
                .get(SERVER_NAME)
                .is_none()
        );
        assert_eq!(
            after["mcp_servers"]["keep"]["command"].as_str(),
            Some("other")
        );
    }

    #[test]
    fn claude_conflict_is_preserved() {
        let dir = tempdir().unwrap();
        let config = dir.path().join(".claude.json");
        fs::write(
            &config,
            r#"{"mcpServers":{"qwen3asr":{"command":"other","args":["mcp"]}}}"#,
        )
        .unwrap();
        assert!(
            install_json_mcp(
                &config,
                "claude",
                json!({"type":"stdio","command":"qwen3asr.exe","args":["mcp"]}),
                false
            )
            .is_err()
        );
        let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(value["mcpServers"][SERVER_NAME]["command"], "other");
    }

    #[test]
    fn agy_uses_verified_gemini_config_shape_and_preserves_named_neighbors() {
        let dir = tempdir().unwrap();
        let config = dir.path().join(".gemini/config/mcp_config.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(
            &config,
            r#"{"mcpServers":{"other":{"command":"keep","args":[]}}}"#,
        )
        .unwrap();
        let exe = PathBuf::from(r"D:\tools\qwen3asr.exe");
        let entry = agy_entry(&exe);
        let owner = dir.path().join(".qwen3asr/agent-integrations/agy.json");
        install_manifest(&owner, "agy", &config, entry.clone(), false).unwrap();
        let installed = install_json_mcp(&config, "agy", entry, false).unwrap();
        assert_eq!(installed["action"], "installed");
        let backup = PathBuf::from(installed["backup"].as_str().unwrap());
        assert!(backup.exists());
        let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            value["mcpServers"][SERVER_NAME]["command"],
            exe.to_string_lossy().as_ref()
        );
        assert_eq!(value["mcpServers"][SERVER_NAME]["args"], json!(["mcp"]));
        assert_eq!(value["mcpServers"][SERVER_NAME]["disabled"], false);
        assert_eq!(value["mcpServers"]["other"]["command"], "keep");
        let owned = validate_manifest(&owner, "agy", &config).unwrap();
        let removed = uninstall_json_mcp(&config, "agy", owned.as_ref(), false).unwrap();
        assert_eq!(removed["action"], "uninstalled");
        uninstall_manifest(&owner, "agy", &config, false).unwrap();
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert!(after["mcpServers"].get(SERVER_NAME).is_none());
        assert_eq!(after["mcpServers"]["other"]["command"], "keep");
    }

    #[test]
    fn agy_dry_run_and_skill_location_are_isolated() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("mcp_config.json");
        let plan =
            install_json_mcp(&config, "agy", agy_entry(Path::new("qwen3asr.exe")), true).unwrap();
        assert_eq!(plan["action"], "would_install");
        assert!(!config.exists());
        assert!(skill_path("agy", dir.path(), None).ends_with(".agents/skills/qwen3asr/SKILL.md"));
        assert_eq!(targets("all").unwrap(), vec!["codex", "agy", "claude"]);
    }

    #[test]
    fn claude_user_mcp_and_skill_paths_follow_official_config_directory() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let config_dir = dir.path().join("claude-profile");

        assert_eq!(
            client_config_path("claude", &home, None).unwrap(),
            home.join(".claude.json")
        );
        assert_eq!(
            skill_path("claude", &home, None),
            home.join(".claude/skills/qwen3asr/SKILL.md")
        );
        assert_eq!(
            client_config_path("claude", &home, Some(&config_dir)).unwrap(),
            config_dir.join(".claude.json")
        );
        assert_eq!(
            skill_path("claude", &home, Some(&config_dir)),
            config_dir.join("skills/qwen3asr/SKILL.md")
        );
    }

    #[test]
    fn uninstall_requires_the_exact_recorded_entry_and_preserves_user_changes() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("mcp_config.json");
        let foreign = json!({"args":["mcp"],"command":"C:/other/qwen3asr.exe","disabled":false});
        fs::write(
            &config,
            json!({"mcpServers":{"qwen3asr":foreign.clone()}}).to_string(),
        )
        .unwrap();
        assert!(uninstall_json_mcp(&config, "agy", None, false).is_err());
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(after["mcpServers"][SERVER_NAME], foreign);

        let own = agy_entry(Path::new("C:/tools/qwen3asr.exe"));
        let owner = dir.path().join("owner.json");
        install_manifest(&owner, "agy", &config, own.clone(), false).unwrap();
        let mut user_modified = own.clone();
        user_modified["user_env"] = json!({"KEEP":"me"});
        let root = json!({"mcpServers":{"qwen3asr":user_modified.clone()}});
        fs::write(&config, root.to_string()).unwrap();
        let recorded = validate_manifest(&owner, "agy", &config).unwrap();
        assert!(uninstall_json_mcp(&config, "agy", recorded.as_ref(), false).is_err());
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(after["mcpServers"][SERVER_NAME], user_modified);
    }

    #[cfg(unix)]
    #[test]
    fn file_symlink_destinations_are_rejected_without_replacing_the_link() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        let target = dir.path().join("real.json");
        let link = dir.path().join("linked.json");
        fs::write(&target, "{}\n").unwrap();
        symlink(&target, &link).unwrap();
        assert!(
            install_json_mcp(&link, "agy", agy_entry(Path::new("qwen3asr.exe")), false).is_err()
        );
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "{}\n");
    }
}

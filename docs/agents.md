# MCP and agent integrations

## MCP server

Start the stdio JSON-RPC server with `qwen3asr mcp`. It writes protocol messages to stdout; progress and errors from the CLI subprocess are captured from stderr. Keep this process attached to the agent client rather than redirecting unrelated output into the protocol stream.

The server exposes these tools:

| Tool | Arguments | Result |
|---|---|---|
| `info` | none | `doctor --json` runtime and device status |
| `models_list` | none | model catalog and local availability |
| `models_status` | none | downloaded model verification state |
| `transcription_start` | required `inputs: string[]`; optional `output_dir`, `formats`, `model`, `device`, `language`, `timestamps`, `offline`, `traditional`, `overwrite`, `chunk_seconds`, `cpu_threads`, `prompt` | `job_id` and `queued` or `running` state |
| `transcription_status` | required `job_id` | job state, exit code, bounded result summary and stderr tail |
| `transcription_cancel` | required `job_id` | cancellation result |

`inputs` must name existing local files. The server canonicalizes them to absolute paths before invoking the same CLI with `transcribe --json`; multi-file requests continue after an individual failure and preserve successful artifact paths plus a bounded error list. The CLI writes artifacts to `output_dir`, or beside each source file when no output directory is supplied.

One transcription subprocess runs at a time, so queued requests do not compete for the GPU. Poll `transcription_status` until a terminal state. Output capture keeps at most 1 MiB of CLI stdout and the last 64 KiB of stderr; transcript previews and individual batch error strings are bounded. Windows cancellation uses `taskkill /T /F` to stop the CLI and its native inference descendants. If process-tree termination cannot be confirmed, cancellation returns an error, the job becomes `cancellation_unconfirmed`, and the queue remains paused even if the CLI parent has exited. Its result exposes `cancelled: false` and `process_tree_terminated: false`. Inspect and stop orphaned worker/FFmpeg processes before restarting the MCP server; restart discards the in-memory queue, so submit those jobs again.

## Install or remove integrations

Use `qwen3asr agents install --target codex|agy|claude|all --dry-run` to preview, then omit `--dry-run` to install. `qwen3asr agents uninstall --target ... --dry-run` previews removal. Install and uninstall back up existing files before changing them. Operations preflight the named configuration, Skill and ownership-manifest paths before applying changes. A conflicting or unrecognized `qwen3asr` entry is preserved and reported as an error. Config or Skill files that are symlinks or Windows reparse points are rejected with an actionable error; replace the link with a regular file or manage its target directly.

The CLI updates only the named `qwen3asr` server entry and installs the Skill at these paths:

| Target | MCP config | Skill |
|---|---|---|
| Codex | `$CODEX_HOME/config.toml`, or `~/.codex/config.toml`, under `[mcp_servers.qwen3asr]` | `$CODEX_HOME/skills/qwen3asr/SKILL.md`, or `~/.codex/skills/qwen3asr/SKILL.md` |
| AGY | `~/.gemini/config/mcp_config.json`, under `mcpServers.qwen3asr` | `~/.agents/skills/qwen3asr/SKILL.md` |
| Claude Code | `~/.claude.json`, under top-level `mcpServers.qwen3asr` | `~/.claude/skills/qwen3asr/SKILL.md` |

Claude Code's `CLAUDE_CONFIG_DIR` overrides its personal configuration directory. When set, the MCP config is `<CLAUDE_CONFIG_DIR>/.claude.json` and the Skill is `<CLAUDE_CONFIG_DIR>/skills/qwen3asr/SKILL.md`. These paths follow the [official MCP scope documentation](https://code.claude.com/docs/en/mcp#user-scope) and [Claude directory reference](https://code.claude.com/docs/en/claude-directory), and the MCP file location was checked against the installed Claude CLI in an isolated profile.

Each installation also stores an ownership manifest. Codex uses `$CODEX_HOME/agent-integrations/codex.json` (or `~/.codex/agent-integrations/codex.json`); AGY and Claude Code use `~/.qwen3asr/agent-integrations/<client>.json`. Uninstall requires that manifest and an exact match with the installed entry. User-added fields or a changed command make the entry unrecognized, so the CLI preserves it. If an exact matching entry was configured manually, explicitly running `agents install` records ownership for future uninstall.

AGY's config path and entry shape were checked against the installed AGY CLI in an isolated user profile. Its MCP entry is:

```json
{
  "args": ["mcp"],
  "command": "<absolute path to qwen3asr executable>",
  "disabled": false
}
```

Codex, AGY and Claude Skill discovery is independent of config installation. The AGY skill location uses the shared `~/.agents/skills` convention; whether every AGY build discovers that Skill path has not been verified. A successful install command only confirms files were updated, not that a client loaded the Skill or connected to the MCP server.

Each backup is placed beside its original with a `.bak-<UTC timestamp>-<process>-<sequence>` suffix. To restore manually, close the client and copy the desired backup over its corresponding original config or Skill file. Uninstall also creates a backup before removing the managed entry, manifest or unchanged Skill file.

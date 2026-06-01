# SeekForge

SeekForge is a DeepSeek-native fork of OpenAI Codex. The fork keeps Codex's
agent harness strengths while replacing the default OpenAI/ChatGPT product path
with a DeepSeek-first runtime.

## Direction

Keep:

- MCP, plugins, skills, memory, sandbox, approvals, and TUI/exec flows.
- Upstream-compatible Codex project layout.
- TOML-based provider and model configuration.

Change:

- Default provider is `deepseek`.
- Default executor model is `deepseek-v4-flash`.
- Default review/planner model is `deepseek-v4-pro`.
- Native OpenAI/ChatGPT login is not a SeekForge product capability.

## Quickstart

Build and run from the repository root using the existing Codex workflow. During
the transition the binary may still be named `codex`.

```powershell
$env:DEEPSEEK_API_KEY = "sk-..."
codex
```

or configure the same value in your shell profile before starting SeekForge.

`codex login` is intentionally only a guidance command in this fork. It does not
create `auth.json`; use `DEEPSEEK_API_KEY` or provider-specific TOML settings.

## DeepSeek Config

SeekForge ships with DeepSeek defaults, so most users only need
`DEEPSEEK_API_KEY`. To pin the same behavior explicitly in `config.toml`:

```toml
model = "deepseek-v4-flash"
model_provider = "deepseek"
review_model = "deepseek-v4-pro"

[deepseek_native]
planner_model = "deepseek-v4-pro"
planner_enabled = false

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat_completions"
requires_openai_auth = false
request_max_retries = 4
stream_max_retries = 5
stream_idle_timeout_ms = 300000
```

Run `codex doctor` to confirm the active provider, `DEEPSEEK_API_KEY`, and
provider endpoint reachability.

## Repository Notes

- Codex source lives at the repository root.
- `reasonix/` is ignored and used only as local reference material.
- The full DeepSeek-native fork plan is in
  [docs/seekforge_deepseek_native_plan.md](docs/seekforge_deepseek_native_plan.md).
- The upstream project remains [openai/codex](https://github.com/openai/codex).

This repository keeps the upstream [Apache-2.0 License](LICENSE).

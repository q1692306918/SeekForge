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

## Repository Notes

- Codex source lives at the repository root.
- `reasonix/` is ignored and used only as local reference material.
- The full DeepSeek-native fork plan is in
  [docs/seekforge_deepseek_native_plan.md](docs/seekforge_deepseek_native_plan.md).
- The upstream project remains [openai/codex](https://github.com/openai/codex).

This repository keeps the upstream [Apache-2.0 License](LICENSE).

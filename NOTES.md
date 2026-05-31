# Codex Reasonix Notes

## Source Checkouts

- `codex/`: local Codex source checkout.
- `reasonix/`: cloned from `https://github.com/esengine/DeepSeek-Reasonix.git`.
  - Branch at clone time: `main-v2`.
  - Commit at clone time: `fa0a75f3`.

## Initial Reasonix Ideas To Evaluate

- DeepSeek prefix-cache-first session design: keep system prompt, tools, and project memory byte-stable across turns.
- Separate planner and executor sessions for two-model collaboration, avoiding shared-session model switching.
- Low-frequency context compaction with archive of dropped history.
- Config-driven OpenAI-compatible providers so DeepSeek/MiMo-style endpoints are config, not hardcoded code paths.
- MCP-compatible plugin loading, including `.mcp.json` compatibility, MCP prompts as slash commands, and resources via `@server:uri`.
- Project memory hierarchy with durable `REASONIX.md` / `AGENTS.md` loading and quick memory additions.
- CodeGraph-style symbol and call-graph tools as a cheaper alternative to embedding search.
- Per-call permission policy and workspace write confinement as separate layers.

## Discussion Needed

- Which Codex capabilities must stay first-class.
- Which DeepSeek-specific optimizations are worth carrying into Codex.
- Whether this fork should be a minimal DeepSeek provider adaptation or a broader agent-harness redesign.

## Confirmed Product Direction

- Product: DeepSeek-native Codex, not a generic multi-provider Codex variant.
- Rationale: Codex is the stronger harness engineering base compared with CodeWhale and Reasonix; prior CodeWhale customization improved adherence to project rules, but Codex already has the core agent infrastructure worth preserving.
- Do not preserve native OpenAI/ChatGPT login as a first-class product capability.
- Preserve Codex's existing MCP, plugins, skills, memory, sandbox, and approval systems completely.
- Preserve upstream Codex compatibility and keep the fork rebase-friendly.
- Default target models: `deepseek-v4-flash` and `deepseek-v4-pro`.
- Model endpoints and defaults should remain TOML-maintainable.
- Scope: medium change, not a full Reasonix-style harness rewrite.

## Proposed Medium-Scope Track

- Change product defaults to DeepSeek provider + DeepSeek model defaults.
- Use env-key auth such as `DEEPSEEK_API_KEY`; avoid first-party OpenAI/ChatGPT login flows in the default product surface.
- Keep Codex provider/config abstractions, because they already support custom OpenAI-compatible providers via TOML.
- Add DeepSeek-aware cache discipline: stable prefix, stable tool ordering/schema, and careful handling of project memory in the prompt prefix.
- Add optional planner/executor separation using independent sessions so model collaboration does not break prefix cache locality.
- Adapt compaction behavior for DeepSeek long-context cost control while preserving Codex rollout/history traceability.
- Evaluate CodeGraph-style local symbol/call-graph search as an optional tool layer after provider and cache behavior are stable.

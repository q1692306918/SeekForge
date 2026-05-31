# SeekForge DeepSeek-Native Fork Plan

## 1. Product Positioning

SeekForge is a DeepSeek-native fork of Codex. The product bet is that Codex has
the stronger agent harness: MCP, plugins, skills, memory, sandboxing, approvals,
TUI/exec flows, rollout persistence, and upstream engineering discipline. The
fork should keep those strengths and replace the OpenAI-first product surface
with a DeepSeek-first runtime.

Non-goals:

- Do not become a generic multi-provider agent rewrite.
- Do not rewrite the harness in Reasonix's Go architecture.
- Do not remove Codex subsystems just because the OpenAI product surface is not
  used.
- Do not vendor Reasonix source into this repository; keep it as ignored local
  reference material.

Compatibility goal:

- Keep the fork easy to rebase onto upstream Codex. Prefer isolated provider,
  model, auth-surface, and prompt/cache changes over broad renames or deletion
  of upstream crates.

## 2. Repository Shape

- Codex source lives at the repository root.
- `reasonix/` is ignored and remains a local reference checkout only.
- `origin` should point to `https://github.com/q1692306918/SeekForge.git`.
- Add `upstream` for `https://github.com/openai/codex.git` after pushing, so
  upstream syncs are explicit:

```sh
git remote add upstream https://github.com/openai/codex.git
git fetch upstream
```

## 3. Capability Decisions

Keep first-class:

- MCP client and MCP server support.
- Plugins and plugin marketplace plumbing unless a path hard-requires OpenAI
  account features.
- Skills and local skill discovery.
- Memory, AGENTS.md loading, thread persistence, rollout traceability, and
  compaction records.
- Sandbox, approval, guardian, and execution policy layers.
- TUI and non-interactive `exec` flows.
- Existing TOML configuration model.

Remove or hide as product surface:

- ChatGPT browser login.
- OpenAI API-key login as the default setup path.
- ChatGPT account/plan prompts.
- OpenAI-specific onboarding copy and remediation text.

Keep as implementation detail where useful:

- `codex-login` crate and auth-manager code can stay initially to reduce rebase
  conflicts and because MCP OAuth / remote workflows may still share plumbing.
- OpenAI provider code can stay as upstream-compatible dead-end or optional
  compatibility path, but SeekForge defaults must not require it.

## 4. Core Technical Finding

Current Codex has moved provider execution toward the OpenAI Responses API. In
`codex-rs/model-provider-info/src/lib.rs`, `WireApi` only accepts `responses`,
and `wire_api = "chat"` is rejected.

DeepSeek's OpenAI-compatible API should be treated as a Chat Completions style
backend for this fork. Therefore, the core change is not just:

```toml
model = "deepseek-v4-flash"
model_provider = "deepseek"
```

The fork needs a DeepSeek runtime adapter that maps Codex's internal request,
tool, stream, usage, and history model onto DeepSeek-compatible chat completion
requests.

## 5. Target Configuration

Default behavior:

```toml
model = "deepseek-v4-flash"
model_provider = "deepseek"
review_model = "deepseek-v4-pro"

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

Model intent:

- `deepseek-v4-flash`: default executor model.
- `deepseek-v4-pro`: default review/planner model.
- Both remain overrideable through TOML, profile, and CLI model flags.

Configuration design:

- Add a DeepSeek built-in provider ID, likely `deepseek`.
- Either extend `WireApi` with `ChatCompletions` or implement a provider-owned
  runtime path that bypasses the Responses-only assumption.
- Prefer explicit `wire_api = "chat_completions"` over reviving ambiguous
  `wire_api = "chat"`.
- Keep `model_providers` as the user extension point; do not hardcode endpoint
  lists outside provider defaults and model catalog presets.

## 6. Provider Adapter Design

Create a DeepSeek/OpenAI-chat-compatible adapter in the Rust provider/client
layer. Candidate homes:

- `codex-rs/model-provider`: provider-specific capability and model-manager
  behavior.
- `codex-rs/core/src/client.rs` or adjacent client module: request execution
  path that currently speaks Responses.
- A new small crate if the mapping becomes large enough to keep `codex-core`
  from growing further.

Responsibilities:

- Convert Codex prompt items to chat messages:
  - system/developer instructions into stable prefix messages.
  - user content into `role = "user"`.
  - assistant text and tool calls into `role = "assistant"`.
  - tool results into `role = "tool"` with stable `tool_call_id`.
- Convert Codex tool schemas to chat-completions `tools`.
- Parse streaming text deltas.
- Parse streaming reasoning deltas if DeepSeek emits a `reasoning_content`-style
  field; display dimly or route through existing reasoning events.
- Accumulate streamed tool-call deltas by index and emit complete tool calls.
- Preserve tool-call ordering and IDs so approvals, sandbox, and MCP execution
  remain untouched.
- Normalize usage:
  - input tokens.
  - output tokens.
  - cache-hit/cache-miss prompt tokens when DeepSeek exposes them.
- Map provider errors into existing Codex error types with actionable messages.
- Keep retries and idle timeout behavior consistent with current Codex settings.

Provider capabilities:

- Disable OpenAI-native namespace tools that DeepSeek cannot support directly.
- Keep function/tool calling available.
- Disable image generation and OpenAI web-search by default unless backed by
  local tools/MCP.
- Keep MCP tools, shell tools, file tools, and skills exactly as Codex exposes
  them today.

## 7. Model Catalog

Add a static DeepSeek model catalog so SeekForge does not depend on an OpenAI
models endpoint shape.

Model entries should include:

- slug.
- display name.
- context window.
- default reasoning/verbosity compatibility.
- supported service tiers, likely none initially.
- auto-compact token limit.

Initial defaults:

- `deepseek-v4-flash`
- `deepseek-v4-pro`

Keep catalog overrideable through existing model configuration paths. Tests
should verify that unknown DeepSeek model strings can still be passed through
when users configure custom endpoints.

## 8. Auth and Onboarding

Desired user path:

```sh
set DEEPSEEK_API_KEY=sk-...
seekforge
```

or, while the binary is still named `codex`:

```sh
set DEEPSEEK_API_KEY=sk-...
codex
```

Implementation:

- `requires_openai_auth = false` for the DeepSeek provider.
- TUI bootstrap should not show ChatGPT sign-in when the selected provider does
  not require OpenAI auth.
- Missing key errors should mention `DEEPSEEK_API_KEY`, not `codex login`.
- `doctor` should report DeepSeek env-key status and endpoint reachability.
- `codex login` can remain temporarily for upstream compatibility, but it should
  not be part of the SeekForge happy path.

Later product cleanup:

- Decide whether to remove `login` from top-level help, repurpose it to an env
  setup helper, or leave it as an explicitly unsupported compatibility command.
- Rebrand command/package names only after the runtime changes are stable.

## 9. Prefix Cache Discipline

Reasonix's strongest idea for this fork is cache-first prompt construction.
DeepSeek automatic prefix caching rewards byte-stable prefixes, so SeekForge
should make request serialization deterministic.

Rules:

- Stable system/developer instruction ordering.
- Stable tool schema ordering.
- Stable MCP tool names and schema serialization.
- Stable project memory/AGENTS.md prefix content within a session.
- Put volatile turn data in the tail, not in the prefix.
- Avoid changing model/provider metadata inside the prompt prefix mid-session.

Engineering tasks:

- Audit prompt construction in `codex-rs/core`.
- Add request serialization snapshot tests for the DeepSeek adapter.
- Add tests that two adjacent turns share identical serialized bytes up to the
  conversation-tail boundary.
- Track cache-hit/cache-miss usage from DeepSeek responses and surface it in
  debug logs or telemetry.
- Avoid changing existing Codex memory semantics; only stabilize where content
  lands in the model request.

## 10. Planner / Executor Collaboration

Medium-scope target:

- Optional planner model, defaulting to `deepseek-v4-pro`.
- Executor model defaults to `deepseek-v4-flash`.
- Planner and executor use separate sessions to avoid breaking DeepSeek prefix
  cache locality.

Config sketch:

```toml
[deepseek_native]
planner_model = "deepseek-v4-pro"
planner_enabled = false
```

Execution:

- Planner receives user request and stable project context, but no write tools.
- Planner produces concise structured guidance.
- Executor receives the plan as a tail message and performs normal Codex tool
  execution.
- Planner session and executor session never mix histories.
- Approvals and sandbox remain executor-side.

Rollout:

- Phase 1: config and no-op plumbing.
- Phase 2: non-interactive planner before complex tasks.
- Phase 3: TUI visibility and user controls.

## 11. Compaction

Current Codex has local and remote compaction paths. Remote compaction is
OpenAI-specific and should not be used for DeepSeek unless deliberately adapted.

DeepSeek behavior:

- Use local inline compaction through the DeepSeek adapter.
- Keep Codex compaction records and rollout traceability.
- Preserve recent tool-call boundaries so the model never sees orphaned tool
  results.
- Tune default auto-compact limit around DeepSeek context windows and observed
  latency/cost.
- Keep manual `/compact`.

Tests:

- Compact with DeepSeek provider uses local adapter, not OpenAI remote compact.
- Compacted history preserves tool-call/result integrity.
- Compaction summary prompt remains overrideable with existing config.

## 12. Code Intelligence

Reasonix's CodeGraph idea is valuable but should come after the provider path is
stable.

Approach:

- Keep existing Codex file search and `rg`-first behavior.
- Add optional CodeGraph-style local symbol/call-graph tools later.
- Prefer MCP or plugin integration first, so the core does not absorb a large
  indexer.
- Enable per-project opt-in before default-on.

## 13. Implementation Milestones

### Milestone A: Repository and Defaults

- Root repository layout completed.
- `reasonix/` ignored.
- Push baseline to SeekForge.
- Add upstream remote.
- Add DeepSeek provider default and static model presets.
- Make default model/provider DeepSeek.

Verification:

- `git status --short` excludes `reasonix/`.
- Config-loading tests cover DeepSeek provider.
- Model default tests expect `deepseek-v4-flash`.

### Milestone B: DeepSeek Transport

- Add Chat Completions wire path or DeepSeek-specific adapter.
- Implement streaming text/tool-call parsing.
- Implement auth via `DEEPSEEK_API_KEY`.
- Add mock-server integration tests.

Verification:

- A mock DeepSeek stream can produce assistant text.
- A mock DeepSeek stream can request a shell/file/MCP tool.
- Tool results loop back into the next model request.

### Milestone C: Product Surface

- Remove ChatGPT sign-in from default onboarding.
- Update missing-key and doctor messages.
- Keep MCP OAuth intact.
- Add sample config docs for DeepSeek.

Verification:

- Starting with no auth but with `DEEPSEEK_API_KEY` does not show login.
- Starting with no `DEEPSEEK_API_KEY` shows a DeepSeek-specific fix.
- MCP OAuth login still works.

### Milestone D: Cache-First Behavior

- Stabilize DeepSeek request serialization.
- Add prefix-stability tests.
- Surface cache-hit usage where available.
- Tune tool schema ordering.

Verification:

- Snapshot tests show deterministic prefix bytes.
- Cache metrics are parsed from mock usage payloads.

### Milestone E: Planner / Executor

- Add optional planner config.
- Keep separate planner/executor model sessions.
- Add traces/events to explain planner activity.

Verification:

- Planner can be enabled/disabled.
- Executor still owns tools, approvals, and sandbox behavior.
- Switching planner model does not mutate executor prefix.

### Milestone F: Compaction Tuning

- Ensure DeepSeek uses local compaction.
- Tune default token limits.
- Keep rollout traceability.

Verification:

- Auto compact works with mock DeepSeek provider.
- Manual `/compact` works in TUI.
- Rollout history records replacement history.

### Milestone G: Optional CodeGraph

- Prototype as MCP/plugin.
- Compare against existing search tools.
- Make opt-in, then decide whether to bundle.

## 14. Risk Register

- DeepSeek may not match OpenAI Responses semantics. Mitigation: dedicated chat
  adapter and mock protocol tests.
- Tool-call streaming details may differ. Mitigation: accumulator tests for
  partial deltas and multi-tool calls.
- Prefix stability can be broken by dynamic instructions. Mitigation: snapshot
  serialized requests and keep volatile context in tail.
- Removing login too aggressively can break app-server/plugin code. Mitigation:
  hide from product path first, remove later only with focused tests.
- Broad branding rename can create constant upstream conflicts. Mitigation:
  postpone binary/package rename until runtime is stable.
- Model names and context limits may change. Mitigation: TOML overrides and
  static catalog kept small.

## 15. First Code Change Recommendation

Start with the smallest runtime-relevant slice:

1. Add a built-in DeepSeek provider and static model catalog.
2. Add the `chat_completions` wire enum and route DeepSeek to a stub adapter.
3. Add mock tests for one text-only streaming response.
4. Change default provider/model only after the adapter can answer a simple
   prompt.

This keeps the fork honest: defaults become DeepSeek-native only when the
transport can actually run DeepSeek-shaped traffic.

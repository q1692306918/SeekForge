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

Codex baseline audit:

- Sub-agents/context isolation: Codex already does this well. Multi-agent v2
  creates separate child threads, returns child metadata/status to the parent,
  uses explicit inter-agent communication tools, and only forks parent history
  into a child through sanitized fork modes. SeekForge should preserve and
  snapshot-test this behavior under DeepSeek; it should not port Reasonix's
  sub-agent architecture as a replacement.
- Token usage: Codex already has canonical `TokenUsage` fields for input,
  cached input, output, reasoning output, totals, app-server notifications, and
  TUI/exec display. DeepSeek cache fields should feed this existing model
  instead of introducing parallel accounting.
- Reasoning: Codex already has reasoning items, reasoning summary/raw events,
  rollout mapping, and display controls. DeepSeek `reasoning_content` should map
  into those existing events/items and must be filtered from outgoing chat
  history.
- Skills: Codex's skill system is stronger than Reasonix's simple prompt
  injection. It already renders ordered skill metadata, aliases long paths,
  applies a context budget, and lazy-loads full skill bodies only when selected.
  The DeepSeek work is prefix stability, not replacing skill discovery.
- Compaction: Codex already has inline compaction, remote OpenAI compaction,
  hooks, rollout traceability, tool-call integrity handling, and token-window
  logic. DeepSeek should use the existing non-remote path and provider-specific
  limits.
- Retry/auth/provider config: Codex already has env-key provider auth,
  request/stream retry knobs, stream idle timeout, and provider capability
  plumbing. The DeepSeek adapter should reuse these surfaces.
- New work remains: Chat Completions transport, chat-message serialization,
  DeepSeek streaming tool-call accumulation, DeepSeek-specific usage parsing,
  live cache probes, DeepSeek model defaults/catalog, and hiding the
  OpenAI/ChatGPT login product path.

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
  field; route them through Codex's existing reasoning summary/raw events and
  rollout item model instead of adding a parallel reasoning UI path.
- Never send `reasoning_content` back in later requests. Treat it as
  response-only data for display, logs, and rollout archives. Re-uploading it
  turns hidden reasoning into paid prompt input and can break DeepSeek request
  validation/cache behavior.
- Accumulate streamed tool-call deltas by index and emit complete tool calls.
- Emit a tool-call-start event as soon as the streamed function name is known,
  then emit the complete tool call after arguments finish streaming.
- Preserve tool-call ordering and IDs so approvals, sandbox, and MCP execution
  remain untouched.
- Always serialize a `content` field for chat messages, even when empty. DeepSeek
  rejects some assistant/tool-call messages when `content` is omitted.
- Normalize usage:
  - input tokens.
  - output tokens.
  - DeepSeek top-level `prompt_cache_hit_tokens` and
    `prompt_cache_miss_tokens`, mapped into Codex's existing
    `cached_input_tokens` and non-cached input display.
  - OpenAI-compatible nested `prompt_tokens_details.cached_tokens`, when using
    other compatible endpoints.
  - reasoning tokens when the backend exposes them separately.
- Map provider errors into existing Codex error types with actionable messages.
- Keep retries and idle timeout behavior consistent with current Codex settings:
  reuse `request_max_retries`, `stream_max_retries`, and
  `stream_idle_timeout_ms`; retry transient network failures plus 408, 429, and
  5xx with bounded backoff; surface 401/403 as `DEEPSEEK_API_KEY`
  configuration problems.

Provider capabilities:

- Disable OpenAI-native namespace tools that DeepSeek cannot support directly.
- Keep function/tool calling available.
- Disable image generation and OpenAI web-search by default unless backed by
  local tools/MCP.
- Keep MCP tools, shell tools, file tools, and skills exactly as Codex exposes
  them today.

Cost and observability:

- Add cache-aware pricing fields to the provider/model metadata where practical:
  cached input, fresh input, output, and reasoning output if DeepSeek bills it
  separately.
- Show per-turn cache usage as absolute numbers, e.g. `N cached / M new`, not
  only a percentage. Percentages can look worse on long fresh turns even when the
  stable prefix is still hitting.
- Show session-level cache hit/miss totals in debug/TUI surfaces when available.
- Preserve raw DeepSeek usage fields in debug traces so pricing assumptions can
  be audited later.

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
- Plan-mode markers must ride in the user turn tail, not mutate system
  instructions or the active tool list.
- Mid-session memory writes must ride the next user turn as a memory update; fold
  them into the stable prefix only on a new session/resume boundary.
- Background job completion notices must ride the next user turn, not mutate the
  prefix.
- Preserve Codex's lazy skill body model. A stable skill index
  (name/description/path alias) can live in the prefix, but full playbooks
  should load only when the skill is selected and should not be permanently
  injected.

Engineering tasks:

- Audit prompt construction in `codex-rs/core`.
- Add request serialization snapshot tests for the DeepSeek adapter.
- Add tests that two adjacent turns share identical serialized bytes up to the
  conversation-tail boundary.
- Track cache-hit/cache-miss usage from DeepSeek responses and surface it in
  debug logs or telemetry.
- Avoid changing existing Codex memory semantics; only stabilize where content
  lands in the model request.
- Add a live, env-gated DeepSeek cache probe test that can be run manually with
  `DEEPSEEK_API_KEY` to confirm real cache-hit behavior, `reasoning_content`
  behavior, and prompt-token deltas.
- Add a regression test proving `reasoning_content` is never serialized into an
  outgoing DeepSeek chat request.

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
- Planner output should not be appended to the executor's stable prefix. It is
  turn-specific guidance and belongs in the executor tail.

Rollout:

- Phase 1: config and no-op plumbing.
- Phase 2: non-interactive planner before complex tasks.
- Phase 3: TUI visibility and user controls.

## 11. Compaction

Current Codex has local and remote compaction paths. Remote compaction is
OpenAI/Azure-specific and is already gated by provider capability, so DeepSeek
should naturally use the inline path unless a DeepSeek remote compaction API is
deliberately added later.

DeepSeek behavior:

- Use local inline compaction through the DeepSeek adapter.
- Keep Codex compaction records and rollout traceability.
- Preserve recent tool-call boundaries so the model never sees orphaned tool
  results.
- Tune default auto-compact limit around DeepSeek context windows and observed
  latency/cost.
- Keep manual `/compact`.
- Treat compaction as an intentional cache-reset point. Between compactions,
  preserve append-only session growth so DeepSeek cache hit rates can climb.
- Archive or preserve dropped originals through the existing Codex rollout
  history so summaries remain auditable.

Tests:

- Compact with DeepSeek provider uses local adapter, not OpenAI remote compact.
- Compacted history preserves tool-call/result integrity.
- Compaction summary prompt remains overrideable with existing config.
- Cache-hit tests should show the expected hit-rate drop after compaction and
  recovery as the new prefix stabilizes.

## 12. Sub-Agents And Context Isolation

Reasonix uses sub-agents to keep broad exploration from polluting the parent
context, but Codex already has a stronger version of this pattern. SeekForge
should preserve Codex's multi-agent/sub-agent machinery and make its DeepSeek
behavior cache-aware.

Existing Codex behavior to preserve:

- Child agents run as separate threads/sessions.
- `spawn_agent` returns metadata such as agent ID, task name, and nickname, not
  a full child transcript.
- `wait_agent` reports status; detailed parent/child communication goes through
  explicit mailbox-style tools such as `send_message` and `assign_task`.
- Parent history may be forked into a child only through explicit fork modes,
  and that fork is sanitized to remove non-final assistant/tool chatter.
- Child tool calls, file reads, and reasoning do not automatically become parent
  prompt history.

Guidelines:

- Sub-agents should run in their own model sessions.
- Parent context should receive only the distilled answer unless the user asks
  for full details.
- Sub-agent tool calls, file reads, and reasoning should not automatically enter
  the parent prompt.
- Sub-agent model selection should remain configurable; a cheap executor model
  can handle focused exploration, while `deepseek-v4-pro` can be reserved for
  planning/review roles.

Tests:

- A sub-agent run does not inject its intermediate tool traffic into the parent
  DeepSeek request.
- Sub-agent final answers preserve useful file/line citations.

## 13. Explicitly Out Of Scope: CodeGraph

CodeGraph is not part of this SeekForge fork plan. The current product goal is a
DeepSeek-native Codex harness, not a new code-indexing architecture.

Rules:

- Keep existing Codex file search, `rg`-first behavior, MCP tools, and plugin
  extension points.
- Do not add CodeGraph config, background indexing, symbol graph storage, or
  planner dependencies in this phase.
- If code intelligence is revisited later, treat it as a separate opt-in
  project with its own design, tests, and upstream-compatibility review.

## 14. Implementation Milestones

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
- Drop `reasoning_content` from outgoing request history while retaining it for
  display/archive.
- Always serialize message `content`, including empty assistant messages with
  tool calls.
- Parse DeepSeek cache-hit/cache-miss usage fields.
- Add cache-aware provider pricing metadata.
- Add mock-server integration tests.

Verification:

- A mock DeepSeek stream can produce assistant text.
- A mock DeepSeek stream can request a shell/file/MCP tool.
- Tool results loop back into the next model request.
- A mock DeepSeek stream can emit partial tool-call deltas that are accumulated
  into one complete tool call.
- Outgoing request snapshots never contain `reasoning_content`.
- Usage fixtures parse cached/new/reasoning token counts.

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
- Keep plan-mode, mid-session memory updates, and background job notices in the
  user-turn tail.
- Preserve Codex's lazy skill bodies while exposing a stable skill index.
- Add optional live DeepSeek cache probe.

Verification:

- Snapshot tests show deterministic prefix bytes.
- Cache metrics are parsed from mock usage payloads.
- Toggling plan mode does not change system instructions or tool schema bytes.
- Adding memory mid-session does not mutate the prefix for the active session.
- Live cache probe can be run manually and reports hit/miss/reasoning behavior.

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
- Treat compaction as the only routine cache-reset point.

Verification:

- Auto compact works with mock DeepSeek provider.
- Manual `/compact` works in TUI.
- Rollout history records replacement history.
- Cache tests show hit-rate collapse/recovery around compaction.

### Milestone G: Sub-Agent Isolation Verification

- Verify Codex sub-agent flows remain isolated with the DeepSeek adapter.
- Keep parent context compact by returning distilled answers instead of full
  sub-agent transcripts.

Verification:

- Parent DeepSeek request snapshots do not include sub-agent intermediate tool
  traffic.
- Sub-agent final answers keep enough file/line evidence for follow-up work.

## 15. Reasonix-Inspired Gaps After Codex Baseline

Port only the DeepSeek-specific ideas as Codex-native implementation details,
not as a Reasonix architecture rewrite.

New DeepSeek adapter work:

- Add Chat Completions transport or a DeepSeek-specific adapter.
- Map Codex internal prompt/tool/history items to DeepSeek chat messages.
- Do not resend `reasoning_content`; display/archive only.
- Parse DeepSeek `prompt_cache_hit_tokens` and `prompt_cache_miss_tokens`.
- Accumulate streamed tool-call deltas by index and expose early tool-call-start
  events.
- Always serialize chat message `content`.
- Add real DeepSeek cache probes guarded by `DEEPSEEK_API_KEY`.

Adapt existing Codex systems:

- Map DeepSeek cache/reasoning usage into existing `TokenUsage`, app-server, and
  TUI/exec displays.
- Track cache-aware cost with separate cached-input pricing, preferably in
  provider/model metadata rather than a new accounting subsystem.
- Keep plan-mode and volatile runtime notices in the user-turn tail.
- Add prefix-stability tests for normal turns, plan mode, memory updates, skill
  indexes, and compaction boundaries.
- Reuse Codex retry/auth/provider configuration; only add DeepSeek-specific
  error messages where needed.
- Align compaction boundaries to avoid orphan tool results.

Already handled better by Codex; verify rather than port:

- Sub-agent context isolation and thread separation.
- Lazy skill bodies plus budgeted skill metadata/index rendering.
- Token usage propagation through protocol, app-server, TUI, and exec.
- Reasoning event/archive/display plumbing.
- Inline compaction, rollout traceability, hooks, and tool-call integrity.
- Env-key provider auth and retry/idle-timeout knobs.

## 16. Risk Register

- DeepSeek may not match OpenAI Responses semantics. Mitigation: dedicated chat
  adapter and mock protocol tests.
- Tool-call streaming details may differ. Mitigation: accumulator tests for
  partial deltas and multi-tool calls.
- Reasoning content may leak back into prompt history and inflate cost.
  Mitigation: outgoing request snapshots must reject `reasoning_content`.
- Prefix stability can be broken by dynamic instructions. Mitigation: snapshot
  serialized requests and keep volatile context in tail.
- Cache metrics may be misread if only percentages are shown. Mitigation: show
  absolute cached/new counts and preserve raw usage fields.
- Reimplementing Codex-native systems from Reasonix can regress harness quality.
  Mitigation: classify every borrowed idea as preserve, adapt, or add before
  coding, and snapshot-test DeepSeek adapter behavior against existing Codex
  contracts.
- Removing login too aggressively can break app-server/plugin code. Mitigation:
  hide from product path first, remove later only with focused tests.
- Broad branding rename can create constant upstream conflicts. Mitigation:
  postpone binary/package rename until runtime is stable.
- Model names and context limits may change. Mitigation: TOML overrides and
  static catalog kept small.

## 17. First Code Change Recommendation

Start with the smallest runtime-relevant slice:

1. Add a built-in DeepSeek provider and static model catalog.
2. Add the `chat_completions` wire enum and route DeepSeek to a stub adapter.
3. Add mock tests for one text-only streaming response.
4. Change default provider/model only after the adapter can answer a simple
   prompt.

This keeps the fork honest: defaults become DeepSeek-native only when the
transport can actually run DeepSeek-shaped traffic.

## 18. Current Implementation Snapshot

As of 2026-06-01, the active branch has moved beyond the initial recommendation
and contains the first SeekForge runtime slice:

Completed or partially completed:

- Repository shape is Codex-at-root, with `reasonix/` ignored as local reference
  material only.
- `origin` points at `https://github.com/q1692306918/SeekForge.git`, with
  `upstream` retained for `https://github.com/openai/codex.git`.
- Built-in provider `deepseek` exists with `DEEPSEEK_API_KEY`,
  `https://api.deepseek.com`, `wire_api = "chat_completions"`, no OpenAI auth
  requirement, and no websocket requirement.
- Default model selection uses `deepseek-v4-flash`; review/planner defaults use
  `deepseek-v4-pro`.
- Static DeepSeek catalog entries exist, including 128k context and a 115,200
  token auto-compact limit.
- Chat Completions request serialization maps Codex messages/tools into
  DeepSeek-compatible chat payloads, always serializes `content`, filters
  outgoing `reasoning_content`, and sorts tool schemas for a stable prefix.
- Chat Completions streaming parses text deltas, `reasoning_content`, streamed
  tool-call deltas, cache hit/miss usage, nested cached-token usage, and
  reasoning-token usage.
- Chat Completions providers keep function/MCP-style tools enabled while
  disabling OpenAI-hosted web search and image generation capabilities.
- `codex login` is no longer a native OpenAI/ChatGPT login path in this fork; it
  prints DeepSeek-native `DEEPSEEK_API_KEY` guidance and does not write
  `auth.json`.
- README and first-run welcome copy now present SeekForge as a DeepSeek-native
  Codex harness.

Still planned:

- Planner/executor execution is still config-first; separate planner sessions
  need an implementation pass before enabling it by default.
- DeepSeek-specific TUI/exec cache usage surfacing should be audited end to
  end, even though raw usage now maps into existing token usage structures.
- Sub-agent isolation needs DeepSeek request snapshot tests rather than a new
  architecture.
- Compaction needs mock DeepSeek auto-compact coverage beyond the existing
  provider gate and catalog limit.
- Full local CLI/TUI test verification on Windows currently depends on a usable
  `rusty_v8` artifact or a warmed cache; otherwise builds may fail before tests
  run while downloading or preparing `v8`.

# BlastGuard + Claude Code Integration

Drop-in hooks + skill that bias Claude Code toward BlastGuard's three MCP
tools (`search`, `apply_change`, `run_tests`) without resorting to hard
forcing (which the [CodeCompass paper](https://arxiv.org/abs/2602.20048)
measured as actively harmful — forced tools are ignored ~58% of the time).

## What's in this directory

| File | Purpose |
|---|---|
| `hooks/bg-pretool-grep.sh` | PreToolUse hook for `Grep`. Nudges toward `blastguard__search` when the pattern looks structural (`fn `, `impl `, `class `, `import`, etc.). Silent on plain-text patterns. |
| `hooks/bg-pretool-bash.sh` | PreToolUse hook for `Bash`. Catches `rg/grep/ack` with structural patterns and direct test-runner invocations (`pytest`, `cargo test`, etc.). |
| `hooks/bg-pretool-edit.sh` | PreToolUse hook for `Edit` / `Write`. Nudges toward `blastguard__apply_change` when the target is a source file. Silent on docs/config edits. |
| `settings.sample.json` | Template `.claude/settings.json` wiring the three hooks. |
| `skills/blastguard/SKILL.md` | Claude Code skill file. Ship this inside your project's `.claude/skills/blastguard/` for auto-invocation on trigger phrases. |

All three hooks always emit `permissionDecision: "allow"` — the user's
chosen tool still runs. Only `additionalContext` is injected, which
nudges the agent on the next turn. **No hook denies a tool.** Denying
native tools breaks Claude Code's own orchestration.

## Install

### 1. Build BlastGuard

```bash
git clone https://github.com/A-Hamilton/blastguard.git
cd blastguard
cargo build --release
```

### 2. Register the MCP server for your project

In your project root:

```bash
claude mcp add blastguard --scope project -- \
    /absolute/path/to/blastguard/target/release/blastguard \
    "$(pwd)"
```

`--scope project` commits the registration to `.claude/` so collaborators
get it automatically.

### 3. Install the skill (optional but recommended)

```bash
cd /path/to/your/project
mkdir -p .claude/skills/blastguard
cp /absolute/path/to/blastguard/integrations/claude-code/skills/blastguard/SKILL.md \
   .claude/skills/blastguard/SKILL.md
```

The skill auto-activates when the agent sees trigger phrases ("callers
of", "who calls", "what imports", etc.). Survives context compaction
better than plain CLAUDE.md prose.

### 4. Install the hooks (the enforcement layer)

Hooks are the strongest reinforcement signal because they fire on every
matching tool call. They require `jq` on PATH.

```bash
# Make the hook scripts executable.
chmod +x /absolute/path/to/blastguard/integrations/claude-code/hooks/*.sh

# Copy the sample settings and edit the absolute paths.
cp /absolute/path/to/blastguard/integrations/claude-code/settings.sample.json \
   .claude/settings.json
# Then edit .claude/settings.json and replace /ABSOLUTE/PATH/TO/blastguard
# with the real path to your BlastGuard clone.
```

Restart Claude Code to pick up the new settings.

### 5. Verify

In a Claude Code session, run:

- `/mcp` — BlastGuard should appear with three tools (`search`,
  `apply_change`, `run_tests`) plus the `blastguard://status` resource.
- Ask: "who calls processRequest in this repo?" — the agent should pick
  `blastguard__search` over native Grep. If it doesn't, the skill file
  wasn't loaded or the trigger phrase didn't match; see debugging below.

## Debugging

### Hooks aren't firing

- Check `jq` is installed: `which jq`.
- Check hook scripts are executable: `ls -la integrations/claude-code/hooks/`.
- Check `.claude/settings.json` paths are absolute, not relative.
- Test a hook manually:

  ```bash
  echo '{"tool_name":"Grep","tool_input":{"pattern":"fn processRequest"}}' | \
      /absolute/path/to/integrations/claude-code/hooks/bg-pretool-grep.sh
  ```

  Expected: a JSON object with `additionalContext` mentioning BlastGuard.

- Check Claude Code's hook traces: settings → "Developer" → "Show hook debug output".

### Skill isn't auto-invoking

- The skill file must be at `.claude/skills/blastguard/SKILL.md` (not just `blastguard.md`).
- Restart Claude Code after adding the skill.
- Try restating the query with a structural keyword: "callers of X", "imports of FILE", "find SYMBOL".

### BlastGuard MCP server isn't listed

- `claude mcp list` should show `blastguard`.
- If not, re-run the `claude mcp add` command from step 2.
- Check the binary is executable and the project path is absolute.

## Why hooks + skill + server instructions + tool descriptions all together?

Each layer nudges independently:

1. **Tool descriptions** (built into the BlastGuard binary) — the single
   highest-leverage signal per [Anthropic's tool-use docs](https://platform.claude.com/docs/en/docs/agents-and-tools/tool-use/define-tools).
2. **Server `instructions` field** (built in) — auto-injected into
   Claude's context on every session start.
3. **Skill `description` + `when_to_use`** — survives context compaction
   better than CLAUDE.md, gives a second routing signal.
4. **PreToolUse hooks** — fire on every matching Grep / Bash / Edit call,
   reinforcing the routing *during* the task rather than only up-front.

Hooks are the last-mile reinforcement. Tool descriptions bias the
initial choice; hooks correct drift during long tasks.

## Honesty contract

Even with all four layers enabled, expect ~40-60% of routine queries to
still reach for native tools unless the user's phrasing makes structural
intent obvious. That's the Navigation Paradox documented in CodeCompass
— agents only reach for the novel tool when they sense task difficulty.
BlastGuard's durable value is strongest on tasks that are hard enough
for Claude to want help: multi-file refactors, cascade analysis, test
attribution after a batch of edits.

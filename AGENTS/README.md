# Agent Workspace Policy

This directory separates reusable public Agent guidance from local task state.

| Location | Tracked | Purpose |
|---|---:|---|
| `AGENTS.md` | Yes | Minimal discovery entry point for tools that recognize it |
| `AGENTS/README.md` | Yes | Public workspace and publication rules |
| `AGENTS/templates/` | Yes | Reusable, data-free task and handoff templates |
| `AGENTS/state/` | No | Current task state, local notes, and temporary evidence |
| `AGENTS/handoffs/` | No | Private handoff notes and resumable context |
| `AGENTS/local/` | No | Machine-specific instructions or local tool settings |

## Public Rules

- Treat every tracked file, commit message, issue, pull request, and CI log as
  public information.
- Never move content from `private/` or an ignored `AGENTS/` subdirectory into
  a tracked file without explicit maintainer approval.
- Do not include credentials, tokens, cookies, private keys, production hosts,
  network topology, user data, or unpatched vulnerability details in public
  content.
- UI hosts consume the shared Core facade; they do not create independent
  business reducers, queues, or playback engines.
- Providers do not receive Core database handles, local IDs, raw credentials,
  or realtime audio access.
- Local catalog and playback must not depend on Cloud or Provider availability.

## Design Boundaries

An Agent may not independently change identity scope, Sync merge or recovery
semantics, cryptography, token custody, Provider trust, FFI lifetime rules,
audio realtime rules, or database migration policy. Propose a public-safe
summary and wait for maintainer direction before changing such behavior.

The complete working architecture baseline is maintainer-only. If it is present
in the ignored `private/` directory on a maintainer machine, it informs local
work but must not be copied into public issues, commits, or documentation
without an explicit publication decision.

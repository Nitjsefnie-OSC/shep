# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`docs/terminology.md`**: the lexicon (sheep, flock, shepherd, dogs, lambs…). It is this repo's glossary, and `GLOSSARY.md` at the repo root is the signpost that points at it.
- **`docs/decisions.md`**: why shep is built this way, including rejected paths. It serves as this repo's ADR log.
- **`GLOSSARY.md`** at the repo root, or
- **`GLOSSARY-MAP.md`** at the repo root if it exists: it points at one `GLOSSARY.md` per context. Read each one relevant to the topic. shep has a single context, so it has no map.
- **`docs/adr/`**: read ADRs that touch the area you're about to work in. In multi-context repos, also check `src/<context>/docs/adr/` for context-scoped decisions.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo, which is what this one is:

```
/
├── GLOSSARY.md               ← signpost to docs/terminology.md
├── docs/
│   ├── terminology.md        ← the lexicon itself
│   └── decisions.md          ← the decision log, in place of docs/adr/
└── crates/
```

A repo with several contexts would have a `GLOSSARY-MAP.md` at the root in
place of `GLOSSARY.md`, pointing at one `GLOSSARY.md` per context.

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `docs/terminology.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal: either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_

# Glossary

shep's vocabulary lives in [docs/terminology.md](docs/terminology.md): the
lexicon table, the usage rules, and the ruling each term carries. Read it
before naming a domain concept, and write a term there the moment one gets
resolved, in that file's table form. This file is a signpost, so a skill that
looks for `GLOSSARY.md` at the repo root finds the vocabulary instead of
skipping it.

The glossary is the only place a term is decided, and it is a design lexicon:
keyed on the conventional word, carrying the ruling behind each choice. The
published Terminology page reads a different table, the operator's
`web/src/data/lexicon-table.md`, parsed row by row by
`web/src/data/docs-lexicon.ts`. A term added here reaches that page only when
it also has a row there. `web/src/data/lexicon.ts` and
`web/src/data/docs-rules.ts` read `docs/terminology.md` only as a guard: each
fails the site build when a term or a section heading it was curated from is
gone.

shep has a single context, so there is no `GLOSSARY-MAP.md`. The decision log
that sits beside the glossary is [docs/decisions.md](docs/decisions.md).

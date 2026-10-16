# Glossary

shep's vocabulary lives in [docs/terminology.md](docs/terminology.md): the
lexicon table, the usage rules, and the ruling each term carries. Read it
before naming a domain concept, and write a term there the moment one gets
resolved, in that file's table form. This file is a signpost, so a skill that
looks for `GLOSSARY.md` at the repo root finds the vocabulary instead of
skipping it.

The glossary is the only place a term is decided. `docs/terminology.md` is
also the file the docs site parses for its published lexicon
(`web/src/data/lexicon.ts`), so a term added there reaches the site while a
term added anywhere else reaches nothing.

shep has a single context, so there is no `GLOSSARY-MAP.md`. The decision log
that sits beside the glossary is [docs/decisions.md](docs/decisions.md).

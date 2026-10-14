// Prose ceiling for the docs pages.
//
// Counts prose only: Astro frontmatter, <style> blocks, <CodeBlock> contents
// and terminal transcripts are excluded, because none of them is text a
// reader wades through and none should be cut to hit a number. A page over
// its budget fails the build.
//
// BUDGETS is a ratchet in the same shape as verify-heading-anchors.ts's
// ENFORCED. A page joins it in the phase that rewrites it. A page not listed
// is reported by the summary and does not fail.
//
// This exists because a rewrite brief carrying no numeric target reliably
// comes back having kept most of the original text. The number is the brief.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const pagesDir = fileURLToPath(new URL("../src/pages/docs", import.meta.url));

/**
 * Maximum prose words per page, by slug, at what each measures plus about
 * five percent.
 *
 * A number here only ever comes down. It came down across the board on
 * 2026-09-14, when the "Where to go next" cards were cut from all 25 pages
 * that carried them: 539 lines and the two-card pitch at the foot of each
 * chapter. The chapter bar already says what comes next, and the sidebar
 * lists every chapter on every page.
 *
 * Two pages had no entry at all until that pass. `lookout-config` and
 * `pm2-verbs` shipped after the phase that seeded this map and nothing
 * noticed, which is what the "every page has a budget" test below is for.
 * `lookout` is the other tell: it sat at 5750 against a page measuring
 * 2629, a ceiling so far above the text that it could not have failed.
 *
 * Raising one is allowed and has happened twice, both on 2026-09-14, both
 * for a correction rather than for prose: `getting-started` for the note
 * saying `./server` is the reader's own binary, and `from-pm2` for the
 * clause saying which of the three binaries 17.94 MiB is. Say why in a
 * comment when you do it.
 */
export const BUDGETS: Record<string, number> = {
  // Raised from 4150 on 2026-09-27 for the per-sheep dog settings section:
  // the page had already drifted to 4131 against a ceiling recorded at
  // 4042, so almost none of the stated headroom was real, and the new
  // section (DogSheepSettings, parse_sheep_settings, config.sheep.<dog>)
  // is required content, not prose to trim back out.
  //
  // Raised again from 4200 the same day for "Publishing a per-sheep
  // schema": probe_with_sheep is the write side of that same feature, and
  // it needed its own section rather than a sentence borrowed from
  // somebody else's.
  //
  // Raised from 4340 on 2026-09-30 for "Asking for the shepherd channel":
  // `shep-channel: true` is a new line a dog author has to be told about,
  // and the page had 15 words of headroom.
  //
  // Raised from 4450 on 2026-10-13 for `DogConfigResolved`: a dog author
  // has to be told which request resolves a `{{secret:...}}` and which
  // one still gives the text as written.
  "writing-a-dog": 4470, // 4467
  overrides: 3380, // 3250
  // Raised from 3240 on 2026-09-27 for "A sheep's dogs row": the sub-screen
  // and the per-sheep table pane are new operator-facing surface, not
  // prose to trim back out.
  //
  // Raised from 3290 on 2026-10-01 for the dogs tab: a ninth group with its
  // own key is new operator-facing surface, not prose to trim back out.
  //
  // Raised from 3310 on 2026-10-12 for the tree of tables: `←`, `→` and
  // `Enter` open and close a dog's nested tables, which is new
  // operator-facing surface, not prose to trim back out.
  //
  // Raised again for the list sub-screen refusing an element: an item
  // schema's minLength, pattern, enum and integer bounds are checked before
  // the element is filed, and the page has to name those four rather than
  // say the editor refuses whatever the item schema rejects, which is what
  // it said and is not true of maxLength.
  //
  // Raised again for the dog config pane's arrays and maps of tables, which
  // read as a count and open onto entries: a new row shape the page has to
  // name, with how its entries are labelled and that they are read-only.
  //
  // Raised from 3465 for editing a field inside an entry: the page said
  // everything under one was read-only, and now has to say what edits, what
  // stays read-only, and that Esc writes the section once.
  // Raised from 3530 on 2026-10-13 for `d` on a dog pane's entry row: it
  // removes an operator's data from dogs.toml, so the page has to say when
  // it writes, what `u` does and where the cursor goes. The page had 9
  // words of headroom.
  // Raised from 3785 on 2026-10-13 for a dog setting that is a tagged union:
  // it reads as its tag, opens to that tag's fields and is picked with space,
  // which is new operator-facing surface. The page had no headroom.
  "lookout-config": 3880, // 3871
  // Raised from 2930 on 2026-09-14 for the host line, which `shep flock`
  // began printing on a one-shot listing and not only under `--follow`. It
  // shows above every table on this page, so four transcripts gained a line
  // and the page gained a section saying what the four figures are and why
  // a rate can read `-`.
  output: 3130, // 3111
  // Raised from 2980 on 2026-10-13 for the `log_match` rule: a new `on`
  // value, its `stream` key, and the loss and secrets caveats are
  // operator-facing surface, not prose to trim back out.
  // Raised from 3100 on 2026-10-13 for the `secrets.json` rotation: a
  // `shep secret set` now reaches a running dog, which the live-updates
  // passage has to list.
  // Raised from 3130 on 2026-10-13 for dog names: a sheep cannot take a
  // dog's name, and one that already holds a built-in's keeps it.
  //
  // Raised from 3200 for `shep delete` on a dog: one verb now carries both
  // removals, and the page has to say what survives it and that a built-in
  // is refused. Required content, not prose to trim back out.
  //
  // Raised from 3254 on 2026-10-14 for what `shep daemon reload` does with a
  // dog: which ones cross, which one the successor registers afresh, and which
  // one it stops. An operator has to know a hand-edited `enabled_dogs` reaches
  // a running dog through the reload and nowhere else. Raised again the same
  // day for the review's correction to that rule: what a successor leaves
  // behind is a parked dog, and a marked one still holding a process crosses.
  //
  // Raised again for the metrics dog's OTLP export: four config keys an
  // operator has to be told the defaults of, the `--features otel` install
  // line, what a build without the feature does with `otlp_endpoint`, and
  // the measured binary size #292 asked for. All of it is operator-facing
  // surface for a shipped feature, not prose to trim back out.
  //
  // Raised again from 3650 for the "Channel samples" section: the grammar and
  // caps a sheep's own samples are held to, and the refusal counter. New
  // content rather than a correction.
  //
  // Raised from 3900 on 2026-10-14 for #737: the page said the `otel`
  // feature was "off in the shipped binary", and release artifacts now ship
  // it on. Naming which channels carry it and which take `--features otel`
  // is a correction, not prose. The merged page measures 3900, so the 10
  // words of headroom are for drift, not for this correction.
  // Raised from 3910 on 2026-10-14 for the channel samples joining the OTLP
  // export: the kind mapping, `help` becoming `description`, the refusal
  // counter going out as an integer sum, and the two values `/metrics` can
  // render that OTLP/JSON has no number for. Operator-facing surface for a
  // shipped feature, in the same shape as the raises above, and the page had
  // no headroom to pay for it.
  dogs: 3960, // 3947
  lookout: 2770, // 2629
  "first-flockfile": 2360, // 2245
  // Raised from 2080 on 2026-10-13 for the rotation paragraph: a provider's
  // push now reaches a running dog, which an operator rotating a secret has
  // to be told. The page had drifted to 2067 against a recorded 1977.
  // Raised again from 2150 on 2026-10-13 for the store poll: a `shep secret
  // set` reaches a running dog, and the page says how soon and which keys.
  secrets: 2200, // 2184
  // Raised from 2020 on 2026-10-13 for `disable` and `enable`: two verbs
  // with a refusal an operator has to be told about (an id or `name:slot`
  // exits 2, and `start`, `restart` and `stock` refuse a disabled app) are
  // operator-facing surface, not prose to trim back out.
  //
  // Raised from 2190 on 2026-10-13 for the handover half of the same pair of
  // verbs: the mark now survives `shep daemon reload`, and the recorded `2088`
  // was 95 words stale before that sentence went in.
  //
  // Raised from 2200 for `shep delete` reaching a dog: the verb an operator
  // already reads on this page now has a second path, and the page has to say
  // which names take it. The dog's own detail stays on the dogs page.
  //
  // Raised again for the same verb refusing a name it cannot judge, which is
  // an exit an operator hits. Paid for by tightening the sentence above it.
  lifecycle: 2245, // 2235
  logs: 1900, // 1805
  // Raised from 1690 on 2026-09-28 for "Naming your lambs": `lamb-label`
  // is a new message an app author has to be told about, not prose to trim
  // back out.
  // Raised from 1890 for the "Metrics" section, which says where a `metric`
  // line turns up and what the dog refuses. New content rather than a
  // correction.
  "shepherd-channel": 2060, // 2051
  "from-pm2": 1720, // 1659
  "boot-order": 1670, // 1586
  "talking-to-a-sheep": 1620, // 1534
  // Raised from 1330 on the same day and for the same feature. `host` is a
  // fourth key beside `data`, and its three spellings are three different
  // answers a script has to tell apart: an object, `null`, and no key at
  // all.
  //
  // Raised from 1410 on 2026-10-13 for `disabled`: a key that is absent
  // unless true, beside a `status` that still reads `stopped`, is a shape
  // a script has to be told about.
  "json-output": 1470, // 1413
  startup: 1250, // 1192
  examples: 1200, // 1156
  "not-built": 1080, // 1020
  folds: 900, // 852
  whistle: 710, // 675
  containers: 560, // 526
  // Raised from 560 on 2026-10-13 for dog names: an upgrade must say a sheep
  // named after a built-in keeps its name.
  //
  // Raised from 610 on 2026-10-13 for "Going back to an older version": an
  // operator has to be told a downgrade loses the disable marks and that
  // `shep disable` is what to re-run. Nothing on the page was cut for it.
  //
  // Raised again the same day, to 675, when review named the wrong downgrade
  // behaviour in that section. A parked sheep stays down and loses its refusals
  // rather than coming straight back up, and saying that correctly costs 28
  // more words than saying it wrongly.
  // Raised from 675 on 2026-10-14 for the `enabled_dogs` half of the reload
  // the page already describes: a successor stops a dog the new file dropped
  // and registers a parked one at rest. Required content, not prose.
  upgrading: 710, // 704
  kv: 550, // 519
  // Raised from 440 on 2026-09-21 for the prebuilt-binary download list,
  // which is new content rather than a correction. The page was sitting at
  // exactly 440, one word from failing the build, and the `// 413` beside
  // the old number was 27 words stale: the figure in this comment is only
  // as good as the run that wrote it.
  "getting-started": 500, // 471
  serve: 480, // 448
  cli: 380, // 356
  "community-dogs": 300, // 277
  terminology: 210, // 199
  "pm2-verbs": 40, // 36
};


/**
 * Remove every `<div>` opening with `open`, balancing nested div tags.
 *
 * An unterminated tag ends the walk rather than continuing it. `indexOf`
 * answers -1 there, so the old `+ 1` restarted `j` at 0 and the scan found
 * the same tag forever. This script runs first in `npm run build`, so the
 * hang arrived instead of Astro's report of the malformed page.
 */
function stripBlock(source: string, open: string): string {
  let out = "";
  let i = 0;
  for (;;) {
    const start = source.indexOf(`<${open}`, i);
    if (start === -1) return out + source.slice(i);
    out += source.slice(i, start);
    let depth = 0;
    let j = start;
    for (;;) {
      const next = source.slice(j).search(/<\/?div\b/);
      if (next === -1) return out;
      j += next;
      const closer = source.startsWith("</div", j);
      const end = source.indexOf(">", j);
      if (end === -1) return out + source.slice(start);
      depth += closer ? -1 : 1;
      j = end + 1;
      if (closer && depth === 0) break;
    }
    i = j;
  }
}

/** Words a reader actually reads: no frontmatter, styles, code or transcripts. */
export function proseWords(source: string): number {
  let s = source.replace(/^---[\s\S]*?^---/m, "");
  s = s.replace(/<style>[\s\S]*?<\/style>/g, "");
  s = s.replace(/<CodeBlock[^>]*>[\s\S]*?<\/CodeBlock>/g, "");
  // Terminal transcripts and file panels nest <div>s, so a lazy match ends
  // at the FIRST inner </div> and leaves the rest of the block counted as
  // prose. Found by an agent rewriting the examples page, which has two of
  // them: `$ shep --help` and a column header row were being counted as
  // words a reader wades through. Balance the tags instead.
  s = stripBlock(s, 'div class="terminal');
  s = stripBlock(s, 'div class="file-panel"');
  // A tag may carry a quoted attribute that itself contains "<" or ">":
  // folds.astro's description says `fold:<name> reaches it from any verb`.
  // A plain /<[^>]*>/ stops at the first ">" it meets, which is the one
  // inside those quotes, so the tail of the tag was counted as prose. This
  // walks quoted runs as single units instead.
  s = s.replace(/<[a-zA-Z/!][^>"']*(?:(?:"[^"]*"|'[^']*')[^>"']*)*>/g, " ");
  s = s.replace(/<[^>]*>/g, " ");
  s = s.replace(/\{[^{}]*\}/g, " ");
  return s.split(/\s+/).filter(Boolean).length;
}

test("proseWords counts prose and ignores frontmatter, styles and code", () => {
  const page = [
    "---",
    'import DocsLayout from "../../layouts/DocsLayout.astro";',
    "---",
    "<p>One two three four five</p>",
    "<CodeBlock>{`this whole block is not prose at all`}</CodeBlock>",
    "<style>.x { color: red; }</style>",
  ].join("\n");
  assert.equal(proseWords(page), 5);
});

test("proseWords does not count a terminal transcript", () => {
  const page = `<p>Two words</p><div class="terminal"><div>shep start Flockfile.toml</div></div><p>here</p>`;
  assert.equal(proseWords(page), 3);
});

test("an unterminated div ends the walk instead of spinning forever", () => {
  // This used to hang. `indexOf(">")` answers -1 on a tag with no `>`, the
  // old `+ 1` sent the cursor back to 0, and the scan found the same tag
  // again. `npm run build` runs this script before astro, so the malformed
  // page was never reported: the build simply stopped.
  assert.equal(proseWords(`<p>ok here</p><div class="terminal"><div>a</div`), 3);
});

test("every budgeted page is at or under its ceiling", async () => {
  for (const [slug, budget] of Object.entries(BUDGETS)) {
    const source = await readFile(`${pagesDir}/${slug}.astro`, "utf8");
    const actual = proseWords(source);
    assert.ok(
      actual <= budget,
      `${slug}.astro has ${actual} prose words against a budget of ${budget}; cut ${actual - budget} or move them to the page they belong on`,
    );
  }
});

test("every budgeted slug is a page that exists", async () => {
  const files = await readdir(pagesDir);
  for (const slug of Object.keys(BUDGETS)) {
    assert.ok(
      files.includes(`${slug}.astro`),
      `BUDGETS names "${slug}" but src/pages/docs/${slug}.astro does not exist`,
    );
  }
});

test("every page that exists has a budget", async () => {
  // The other direction, and the one that was missing. `lookout-config` and
  // `pm2-verbs` shipped after the phase that seeded BUDGETS, so they had no
  // ceiling at all: the report below printed them without one and nothing
  // failed. A page with no ceiling is a page that can grow back.
  const files = await readdir(pagesDir);
  const missing = files
    .filter((f) => f.endsWith(".astro"))
    .map((f) => f.replace(/\.astro$/, ""))
    // `index` is the redirect to chapter 1, not a chapter. Same exclusion
    // verify-docs-nav.ts makes, for the same reason.
    .filter((slug) => slug !== "index" && !(slug in BUDGETS));
  assert.deepEqual(
    missing,
    [],
    `no prose budget for ${missing.join(", ")}; add an entry at the page's current count plus a little headroom`,
  );
});

test("report every page's prose count, so the unbudgeted ones stay visible", async () => {
  const files = (await readdir(pagesDir)).filter((f) => f.endsWith(".astro") && f !== "index.astro");
  const rows: [string, number][] = [];
  for (const file of files) {
    const slug = file.replace(/\.astro$/, "");
    rows.push([slug, proseWords(await readFile(`${pagesDir}/${file}`, "utf8"))]);
  }
  rows.sort((a, b) => b[1] - a[1]);
  const total = rows.reduce((sum, [, n]) => sum + n, 0);
  for (const [slug, n] of rows) {
    const budget = BUDGETS[slug];
    console.log(`  ${String(n).padStart(5)}  ${slug}${budget === undefined ? "" : `  (budget ${budget})`}`);
  }
  console.log(`  ${String(total).padStart(5)}  TOTAL across ${rows.length} pages`);
  assert.ok(rows.length > 0, "found no docs pages to count");
});

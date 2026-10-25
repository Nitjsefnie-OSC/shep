/*
 * The landing pasture's animals: the authored config for the resident
 * grazers and for the roamers that visit and leave. PastureAnimals.astro
 * renders the markup (and the watch dog, which has a fixed spot and no
 * config of its own). The `walk`, `roam`, `gambol` and `trot` keyframes
 * these numbers feed are in src/styles/motion.css; SkyStage.astro's THE
 * SPEED MODEL comment is the one explanation of the parallax the pasture
 * sits inside.
 */

/*
 * Roaming animals visit the pasture and leave the way they came (the
 * `roam` keyframes in motion.css: enter from --roam-from, graze at
 * --roam-visit, one small --roam-shift step, graze, exit — negative
 * delays start each one mid-visit so the pasture is never suddenly empty
 * at load). Visit spots are chosen OFF the resident grazers' lanes so no
 * sheep ever walks through another — the fix for the old edge-to-edge
 * crossings reading as z-order bugs. Each visit spot has a grass tuft
 * planted at it (the tufts config in PastureScenery.astro) so the stops
 * read as eating.
 * Fleece colors are illustration literals like the marks' own: a
 * moorit-brown ewe trailing a white lamb up front, a black sheep and a
 * white ewe with a brown lamb far back. Lambs gambol instead of bobbing.
 * The far groups take the same desaturate/brighten treatment and
 * behind-the-fence z-index as the far grazer; widths are clamp()ed in
 * PastureAnimals.astro's styles, per depth and per adult/lamb, for the
 * same footprint-shrinks-with-viewport reason.
 */
export interface RoamerSpec {
  depth: "mid" | "far";
  from: string;
  visit: string;
  shift: string;
  facingRight: boolean;
  duration: string;
  delay: string;
  bottom: string;
  width: number;
  fleece?: string;
  bobDuration: string;
  bobDelay: string;
  trotDuration: string;
  lamb?: { width: number; fleece?: string; gambolDuration: string };
}

export const roaming: RoamerSpec[] = [
  {
    depth: "mid",
    from: "-28vw",
    visit: "14vw",
    shift: "4vw",
    facingRight: true,
    duration: "88s",
    delay: "-34s",
    bottom: "4.5vh",
    width: 64,
    fleece: "#b5895f",
    bobDuration: "5.1s",
    bobDelay: ".6s",
    trotDuration: "0.9s",
    lamb: { width: 36, gambolDuration: "1.15s" },
  },
  {
    depth: "far",
    from: "108vw",
    visit: "58vw",
    shift: "-4vw",
    facingRight: false,
    duration: "120s",
    delay: "-58s",
    bottom: "17vh",
    width: 48,
    fleece: "#4b463d",
    bobDuration: "6.2s",
    bobDelay: ".2s",
    trotDuration: "1.05s",
  },
  {
    depth: "far",
    from: "-28vw",
    visit: "27vw",
    shift: "3vw",
    facingRight: true,
    duration: "140s",
    delay: "-118s",
    /* 14.5vh, in front of the resident far grazer's 16vh lane: its entry
       crosses that sheep's x, and lower-on-screen must paint nearer for
       the pass to read as depth instead of a glitch. */
    bottom: "14.5vh",
    width: 44,
    bobDuration: "6.9s",
    bobDelay: ".9s",
    trotDuration: "1.1s",
    /* width >= 32 so Sheep.astro's build-time legs rule keeps the lamb's
       legs — 28 produced a legless blob bouncing across the far hill. */
    lamb: { width: 34, fleece: "#b5895f", gambolDuration: "0.95s" },
  },
];

/*
 * Depth for the grazing sheep: far/mid/near controls width (a clamp() in
 * PastureAnimals.astro's stylesheet, not a fixed px, so the footprint
 * itself shrinks with the viewport instead of only its position), walk
 * speed, height against the hill, and z-index (the far sheep paints behind
 * .fence — see that rule in PastureScenery.astro). left/bottom are home
 * positions; --walk-amp is how far each one wanders from home before
 * turning back (see the `walk` keyframes in motion.css).
 * These lanes plus the dog's fixed spot were checked against each other at
 * 320px — the tightest width, since every sheep's clamp() is pinned to its
 * min there — with a positive clearance margin at every boundary; wider
 * viewports only add more room. See the .depth-* / .watch-dog rules in
 * PastureAnimals.astro.
 */
export const grazing = [
  {
    depth: "far",
    left: "10%",
    bottom: "16vh",
    width: 52,
    bobDuration: "6.8s",
    bobDelay: ".3s",
    chewing: false,
    trotDuration: "1.1s",
    walkAmp: "2.5vw",
    walkDuration: "34s",
    walkDelay: "0s",
  },
  {
    depth: "mid",
    left: "30%",
    // 3vh, not the original 8vh: at 8vh the mid sheep's feet landed inside
    // .fence's own band (bottom:7vh/54px tall, or bottom:8vh below 600px) at
    // ordinary viewport heights, painting its legs directly over a rail
    // since depth-mid is in front (z-index 3 > the fence's 2) — it read as
    // standing on the fence. 3vh keeps its feet below the fence's bottom
    // edge at both breakpoints (matching how the near sheep already clears
    // it: its feet sit well past the fence's bottom rather than at the same
    // height as a rail), while its back still rises in front of the fence
    // as it walks past — a clean occlusion, not a coincidental balance.
    bottom: "3vh",
    width: 72,
    bobDuration: "5.6s",
    bobDelay: ".8s",
    chewing: true,
    trotDuration: "0.95s",
    walkAmp: "2vw",
    walkDuration: "24s",
    walkDelay: "1.4s",
  },
  {
    depth: "near",
    left: "52%",
    bottom: "2vh",
    width: 92,
    bobDuration: "4.8s",
    bobDelay: "0s",
    chewing: false,
    trotDuration: "0.85s",
    walkAmp: "2.5vw",
    walkDuration: "17s",
    walkDelay: "2.6s",
  },
];

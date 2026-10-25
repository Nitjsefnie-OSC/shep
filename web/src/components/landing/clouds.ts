/*
 * The landing sky's drifting puffs: the authored config (where each one
 * sits, how fast it scrolls, how it sways) and the one function that turns
 * that config into the custom properties the drift animations consume.
 * Clouds.astro renders the markup; SkyStage.astro's THE SPEED MODEL comment
 * is the one explanation of the parallax these numbers feed.
 */

/*
 * Clouds used to sit at raw viewport percentages, so at ordinary widths
 * (below ~1240px, where .inner's max-width isn't yet centering the page)
 * they landed inside the scene column's own horizontal span and hid behind
 * the lede. left/right below are no longer viewport-relative: `.clouds`
 * (see its rule in Clouds.astro) is inset from the LEFT by --col-right, the
 * widest scene column's actual right edge at the current width, computed
 * with the same max-width/columnWidth math SceneSection.astro uses. So these
 * percentages are of the safe zone to the right of every scene's text, and
 * a cloud can never land under copy at any width.
 *
 * `speed` is the fraction of content speed the cloud scrolls at while on
 * screen (THE SPEED MODEL, in SkyStage.astro's stylesheet): near, big clouds
 * run fast (0.75), far, small ones slow (0.4) — depth comes from the SPREAD
 * of those rates, not from any one of them. `wind` is the time-based
 * sideways sway of the puff itself; amplitude and period also scale with
 * depth (near = bigger and faster), and the negative delays start each cloud
 * mid-cycle so no two ever sway in sync (motion.css's lockstep rule).
 *
 * Clouds visible before any scrolling happens can't run on a view()
 * timeline (`atTop`, see `.cloud.at-top` in Clouds.astro). Their `spanVh` is
 * the document top-to-bottom-edge extent — top position plus own height —
 * the figure the scroll(root) range is derived from.
 *
 * Variety comes from three knobs so every puff stays the one handoff
 * silhouette rather than new artwork: `squash` scales the height only
 * (the svg renders with preserveAspectRatio="none", so a 0.8 is a long
 * flat drifter and a 1.12 a taller cumulus — at 3.4px the stroke's
 * non-uniform scaling is imperceptible), `mirror` flips the path inside
 * the svg (an attribute transform, because CSS transforms on the svg
 * element belong to the wind sway), and the two farthest clouds drop a
 * little opacity to lose contrast the way the far sheep do.
 */
export interface CloudSpec {
  left?: string;
  right?: string;
  top: string;
  width: string;
  speed: number;
  atTop?: { spanVh: number };
  squash?: number;
  mirror?: boolean;
  opacity?: number;
  wind: { amp: string; duration: string; delay: string };
}

export const clouds: CloudSpec[] = [
  {
    left: "6%",
    top: "52vh",
    width: "150px",
    speed: 0.6,
    atTop: { spanVh: 63 },
    wind: { amp: "10px", duration: "23s", delay: "-7s" },
  },
  {
    right: "24%",
    top: "30vh",
    width: "88px",
    speed: 0.45,
    atTop: { spanVh: 36 },
    squash: 0.82,
    mirror: true,
    opacity: 0.9,
    wind: { amp: "7px", duration: "29s", delay: "-13s" },
  },
  {
    right: "18%",
    top: "128vh",
    width: "112px",
    speed: 0.5,
    squash: 0.85,
    wind: { amp: "9px", duration: "26s", delay: "-3s" },
  },
  {
    left: "38%",
    top: "262vh",
    width: "205px",
    speed: 0.78,
    mirror: true,
    wind: { amp: "17px", duration: "16s", delay: "-11s" },
  },
  {
    right: "10%",
    top: "332vh",
    width: "132px",
    speed: 0.6,
    squash: 1.12,
    wind: { amp: "12px", duration: "21s", delay: "-15s" },
  },
  {
    left: "20%",
    top: "392vh",
    width: "84px",
    speed: 0.4,
    squash: 0.8,
    opacity: 0.88,
    wind: { amp: "7px", duration: "31s", delay: "-5s" },
  },
];

/*
 * Turn a cloud's authored `speed` into the custom properties the drift
 * animations consume — the two formulas from THE SPEED MODEL, computed here
 * so the tuning knob stays the single speed number above.
 */
export const cloudMotion = (c: CloudSpec) => {
  const wind = `--wind-amp:${c.wind.amp};--wind-duration:${c.wind.duration};--wind-delay:${c.wind.delay};`;
  if (c.atTop) {
    const range = c.atTop.spanVh / c.speed;
    return `--parallax-drift:${(range - c.atTop.spanVh).toFixed(1)}vh;--parallax-range:${range.toFixed(1)}vh;${wind}`;
  }
  const drift = (100 * (1 - c.speed)) / c.speed;
  return `--parallax-drift:${drift.toFixed(1)}vh;--parallax-range-end:${(100 / c.speed).toFixed(1)}%;${wind}`;
};

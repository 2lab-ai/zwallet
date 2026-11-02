import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import "./check-site.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const output = resolve(root, "_site");
await mkdir(resolve(output, "assets"), { recursive: true });
// Explicit public files only: no repository metadata, wallet data or internal docs.
const pages = ["index.html", "animation.html", "film.html", ".nojekyll"];
const assets = [
  "site.css",
  "site.js",
  "animation-player.js",
  "favicon.svg",
  "agent-story.mp4",
  "agent-story-ko.mp4",
  "agent-story-poster.png",
  "agent-story-ko-poster.png",
];
for (const file of [...pages, ...assets.map((x) => "assets/" + x)])
  await cp(resolve(root, "docs", file), resolve(output, file));
const base = process.env.SITE_URL || "https://2lab-ai.github.io/zwallet/";
if (!/^https:\/\/[a-zA-Z0-9.-]+(?:\/[a-zA-Z0-9._/-]*)?\/$/.test(base))
  throw new Error("SITE_URL must be an HTTPS URL ending in /");
const index = await readFile(resolve(output, "index.html"), "utf8");
await writeFile(
  resolve(output, "index.html"),
  index.replaceAll("https://2lab-ai.github.io/zwallet/", base),
);
console.log(
  `Built ${pages.length + assets.length} public files in _site/ for ${base}`,
);

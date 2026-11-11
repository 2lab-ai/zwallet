import { readFile, stat } from "node:fs/promises";
import { resolve, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { Script } from "node:vm";

const root = fileURLToPath(new URL("../", import.meta.url));
const docs = resolve(root, "docs");
const pages = ["index.html", "animation.html", "film.html"];
let links = 0;
for (const name of pages) {
  const file = resolve(docs, name),
    html = await readFile(file, "utf8");
  for (const match of html.matchAll(
    /<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g,
  ))
    new Script(match[1], { filename: name });
  for (const [, url] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if (/^(https?:|data:|mailto:)/.test(url)) continue;
    const clean = url.split("?")[0].split("#")[0];
    if (clean) {
      const dest = resolve(dirname(file), decodeURIComponent(clean));
      if (relative(docs, dest).startsWith(".."))
        throw new Error(`${name}: asset escapes public site: ${url}`);
      await stat(dest);
      links++;
    } else if (url.startsWith("#") && !html.includes(`id="${url.slice(1)}"`))
      throw new Error(`${name}: missing anchor ${url}`);
  }
}
for (const name of ["site.js", "animation-player.js"])
  new Script(await readFile(resolve(docs, "assets", name), "utf8"), {
    filename: name,
  });
console.log(
  `Site checks passed: ${pages.length} HTML pages, ${links} local references, JavaScript syntax.`,
);

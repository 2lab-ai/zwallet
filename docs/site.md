# Landing page and GitHub Pages

The official target is **https://2lab-ai.github.io/zwallet/**. The static site is
owned by this repository. No hosting service account or frontend package install
is needed. The site describes the wallet; it never connects a wallet or sends funds.

## Enable the official site once

A repository administrator must choose **Settings → Pages → Build and deployment →
Source: GitHub Actions**. This is a one-time repository setting; merging a workflow
cannot enable a missing Pages site with the ordinary `GITHUB_TOKEN`.

After setup, merging changes to `docs/`, the site scripts or the Pages workflow into
`main` runs **Landing page** and publishes the site. If the first run happened before
setup, rerun it or use **Actions → Landing page → Run workflow** on `main`.
The job summary/environment provides the actual deployed URL. A successful PR build
alone does not mean the official site has been deployed.

The `github-pages` environment should allow `main` deployments. Pull requests only
validate the site and receive read-only permissions. The deploy job alone receives
`pages: write` and `id-token: write`; no PAT, cloud secret or wallet key is used.

## Edit and preview

| File | Purpose |
|---|---|
| `docs/index.html` | English content with Korean `data-ko` translations; ordinary links remain usable without JavaScript |
| `docs/assets/site.css` | Responsive layout, foil card and reduced-motion styles |
| `docs/assets/site.js` | Language, copy command, chapter controls and viewport-aware playback |
| `docs/animation.html` | The original deterministic SVG film timeline, also used by the MP4 renderer |
| `docs/assets/animation-player.js` | Native playback, same-origin player messages and mobile graph reflow |
| `docs/film.html` | Optional native MP4 player and downloads |
| `.github/workflows/pages.yml` | PR build checks and main-branch Pages deployment |

With Node 24 and Python 3:

```sh
node --test scripts/test-site-player.mjs
node scripts/build-site.mjs
python3 -m http.server 8765 --directory _site
```

Open `http://localhost:8765/`, then `?lang=ko`. Check the page at desktop and narrow
phone widths. Play, pause, replay, seek and choose each chapter; check keyboard focus,
the copy button, reduced motion and the separate film page. Player tests cover the
final-frame notification, paused/reduced-motion startup, the message boundary and
chapter selections made before the lazy frame finishes loading.
Site checks validate local links and JavaScript syntax, not visual design or the
wallet's security.

The build writes only an explicit set of public pages/assets to ignored `_site/`.
Set `SITE_URL` to an HTTPS project URL ending in `/` to build canonical/social links
for a fork. Relative asset paths work under GitHub's `/zwallet/` project prefix.
No external JavaScript, fonts or analytics are requested by the landing page.

The compatibility entry `examples/agent-story/motion.html` redirects to the canonical
animation. The [media instructions](../examples/agent-story/README.md) reproduce MP4s.
Changing player controls or responsive layout does not require re-encoding the film;
changing the deterministic story does.

For copy changes, use the [research and rationale](marketing/landing-research.md),
[feature status](features.md) and [security model](security.md). Keep future features
visibly WIP and distinguish agent-made reports from wallet-native functionality.

# Inline login and setup help: screen evidence

These screens use **synthetic test fixtures**, not a real account, sign-in, device or playback session. Each screenshot carries a visible fixture banner. Typed test tokens are masked; the test import is deliberately refused rather than presented as a successful live sign-in.

The browser suite exercises all 48 guides and all 16 runtime source pages at 1440, 820 and 390 pixels. It checks numbered steps, prerequisites and recovery, method-specific placement, advanced disclosures, loading/error recovery, masked paste/reveal, escaped redirect text, heading order and horizontal overflow. Network requests are intercepted at the fixture origin.

| State | Desktop | Tablet | Phone |
| :-- | :-- | :-- | :-- |
| Recommended TIDAL browser sign-in | [1440 px](shots/login-guides/tidal-recommended-1440.png) | [820 px](shots/login-guides/tidal-recommended-820.png) | [Advanced method](shots/login-guides/tidal-advanced-390.png) |
| Missing librespot: native and Docker instructions | [1440 px](shots/login-guides/spotify-librespot-missing-1440.png) | [820 px](shots/login-guides/spotify-librespot-missing-820.png) | [390 px](shots/login-guides/spotify-librespot-missing-390.png) |
| Service-app setup | — | [Advanced setup](shots/login-guides/service-apps-820.png) | — |
| Optional SoundCloud setup | — | [Client prerequisites](shots/login-guides/soundcloud-client-820.png) | — |
| Existing-token import | — | — | [Masked paste](shots/login-guides/paste-masked-390.png) |
| Server-side Plex setup | — | — | [390 px](shots/login-guides/plex-390.png) |
| Source with no sign-in | [Account-free guide](shots/login-guides/open-source-1440.png) | — | — |
| Node login and token alternatives | — | — | [390 px](shots/login-guides/login-390.png) |
| Loading failure and retry | — | — | [Error state](shots/login-guides/youtube-error-390.png) |

## Reproduce

From a checkout with Playwright and Chromium installed:

```sh
node tools/login-guides-check.js
node tools/login-guides-check.js --self-test
node tools/login-guides-content-check.js .
python3 -I tools/login-guides-recipes-check.py .
node tools/ui-login-guides.js "$PWD" /path/to/screenshots
```

The registry self-test adds an unclassified source, removes a render hook, renames a login method adds a Service app, changes an account auth route or credential field, and adds a podcast server kind in memory; each must fail the coverage check. Content regressions cover writable executable directories, actual Docker container discovery (including the bundle), current quota wording and advanced own-app guidance.

The native/Docker dependency recipes and external developer-account registration steps were checked against runtime code and primary documentation, not executed on a real installation. Synthetic UI checks do not validate a subscription, live account, token renewal, container installation or physical output.

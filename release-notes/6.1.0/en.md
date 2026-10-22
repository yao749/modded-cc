This release adds release notes that pop up after each update, so you can see what changed right inside the app. The tray menu now shows proxy status and common shortcuts, the Live routing page gets the sketchbook look, provider information follows the UI language, and there is a new Requesty provider plus Zhipu's international Z.ai endpoints. Existing behaviour is unchanged by default.
Release notes are new in this version, so the last section recaps the main changes in 6.0.0.

## Features
- **Release notes after updates**: the first time you open the window after an update, the notes for that version pop up once; if you skipped versions, the ones you missed are listed too. Tabs at the top switch between 中文, English and 日本語, defaulting to the UI language. After closing it, click the party popper ("What's new") at the bottom of the sidebar to reopen it and browse earlier versions. The web UI does not pop it up automatically; it only has the sidebar entry.
- **Tray menu with status and shortcuts**: the tray menu now shows the version, proxy status and listen address, and how many subscriptions are available (click to open Subscriptions), plus "Live Routing" and "Request Logs" shortcuts, "Copy Claude Code Env Vars" (PowerShell syntax on Windows), "Check for Updates…" (showing the version number when an update is available) and a "Launch at Startup" checkbox. The menu updates in place when the status changes.
- **Sketchbook look for the Live routing page**: the page now uses cards; the routing diagram draws clients as sticky notes and upstream providers as luggage tags, and scales with the window width. A provider whose subscriptions are all disabled is shown in grey instead of being flagged as failing.
- **Provider information follows the UI language**: built-in providers' names, descriptions, compatibility notes and endpoint names appear in English or Japanese in those UIs, and provider names in the subscription list switch too (usage receipts still show the name from when the subscription was created).
- **Requesty provider** (#48): built-in Anthropic-compatible endpoint for Requesty, plus an EU endpoint; both use the same key.
- **Zhipu's international Z.ai endpoints**: two new endpoints for Z.ai, a Coding Plan subscription and pay-as-you-go. They need a Z.ai API key; keys from open.bigmodel.cn do not work there.
- **Regions in the subscription list**: the Subscriptions page shows a region tag after the provider name (China / Global / EU); local and custom subscriptions have none. Xiaomi's and Kiro's European endpoints are tagged EU.

## Fixes
- **Refreshing models after switching endpoints queried the wrong address**: after a built-in subscription switched to an endpoint in another region, refreshing its model list still queried the old endpoint's domain, which failed with the new region's API key.

## Other
- Removed the 神马中转 API (whatai) provider, which has shut down. Existing subscriptions are unaffected; it is no longer offered when creating a subscription.
- The OpenRouter icon uses its monochrome version on light backgrounds so it stays readable.

## 6.0.0 recap
- **Sketchbook look**: the colours match the website, and the sidebar, cards, buttons, dialogs and more have hand-drawn shapes, in light and dark palettes.
- **Terminal UI cc-router-tui** (off by default): manage cc-router from your terminal without opening a window. Turn it on under Settings → Security & Access → Terminal UI.
- **Tool-call stats**: request logs record which tools the model called, and the Stats page gains tool-call KPI cards and a Top 10 tools card. Only tool names are stored, never their arguments or results.
- **New request detail layout**: new route and usage sections show which subscription and real model each request went through, plus latency, input, output, cache read and cache write.
- **Custom providers discover models automatically** (#44): cc-router can fetch the upstream model list before you save.
- **The database size limit now works**: past the limit (500 MB by default), the oldest request logs and events are deleted first.
- **Backend text follows the UI language**: test-connection results, a subscription's last error and the balance card no longer show Chinese in the English and Japanese UIs.

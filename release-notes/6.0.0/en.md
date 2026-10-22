This is a major release. The desktop app gets a "sketchbook" look that matches the website, and a new terminal UI, cc-router-tui, lets you manage cc-router without opening a window. Request logs and stats now record tool calls, and the database size limit is now a safety net that actually works. Existing behaviour is unchanged by default, and the terminal UI ships disabled.

## Features
- **Sketchbook look**: ivory paper, ink and terracotta, with light and dark palettes that use the same values as ccrouter.app. The sidebar has 11 hand-drawn icons and a hand-drawn wavy right edge. The logo has been redrawn by hand and every app icon regenerated. Cards, buttons, tags, inputs, switches, segmented controls, tables, dialogs and tabs all have hand-drawn shapes, and page titles get a double-stroke terracotta underline. The app bundles Latin subsets of Instrument Sans, Newsreader, JetBrains Mono and Caveat; Chinese and Japanese text still use the system fonts.
- **Terminal UI cc-router-tui (off by default)**: manage a running cc-router from your terminal. Turn it on under Settings → Security & Access → Terminal UI.
  - Five tabs: Overview, Subscriptions, Virtual Models, Live Routing and Request Logs.
  - Subscriptions: enable or disable, test the connection, refresh models, refresh the balance, and change each slot's model and thinking effort. You can create subscriptions (built-in providers, or custom providers over 5 protocols, with model discovery for both) and delete them; the confirmation lists every virtual model that references the subscription.
  - Virtual Models: reorder subscriptions, add or remove them, and switch the scheduling mode.
  - Live Routing: a stream of every attempt plus the last 60 seconds, with pause and filter. Press ⏎ on a row to jump to that subscription's request logs.
  - Request Logs: paginated, filterable, with a detail view for each request.
  - The interface is available in Chinese, English and Japanese and follows the desktop app's language setting. When that setting is "System", it picks the language the same way the tray menu does.
  - It accepts local connections only, and the web UI does not need to be on.
  - The same settings card has an "Add to PATH" row: on macOS it creates a symlink, on Windows it adds the install folder to your user PATH, and with AppImage it copies the binary to ~/.local/bin. After that, typing cc-router-tui in any terminal opens it.
  - --check runs a self-test; --no-fx or CCR_TUI_NO_FX=1 turns off animations; colours use true colour, 16 colours or none (NO_COLOR), depending on the terminal.
  - The terminal must be at least 80×24. Creating OAuth subscriptions and changing API keys still happen in the desktop app.
- **Tool-call stats**: request logs now record the stop_reason, how many tools the request offered, how many tool_results it sent, how many tool_use blocks came back, and the names of the tools called. Only tool names are stored, never their arguments or results. Every upstream protocol is covered. The Stats page gains two tool-call KPI cards and a Top 10 tools card (a tool with the same name is listed separately for each client), the subscription table gets a tools column, and the request list shows a tool marker.
- The request detail dialog has a new layout:
  - The header shows the status, HTTP code and whether the request was streamed, plus the time and request ID. "Copy all" is always visible.
  - A new route section: client → virtual model → subscription → real model (including the model name the upstream reported).
  - A new usage section: latency, input, output, cache read and cache write. Token counts were not shown in the dialog before.
  - Tool calls, the error message and the raw upstream text each have their own section.
- **The "Add subscription" dialog** on the Virtual Models page shows more detail. Each row has the provider logo, the subscription name and status, the provider and endpoint (the account email for OAuth subscriptions), the model the subscription will actually use in this virtual model, and which virtual models already reference it. A search box appears when there are more than 6 candidates, and the footer shows how many are selected.
- **Custom providers discover models automatically (#44)**: cc-router can fetch the upstream model list before you save, and the lookup writes nothing to the database. This also works for custom Anthropic endpoints. Slots use the same picker as built-in providers, including per-slot thinking effort.
- Settings → Advanced shows the database size and the row count of each table.
- Launching cc-router while it is already running brings the existing window back instead of starting a second copy.

## Fixes
- **Text that comes from the backend now follows the UI language**: test-connection results, a subscription's last error and the balance card used to show Chinese even in the English and Japanese UIs. The comments in the recommended Codex config are now translated too.
- **The database size limit now works**. The setting used to do nothing. Now, when the database grows past the limit (500 MB by default), the oldest request logs and events are deleted first and a warning event is emitted. The database file is compacted on every startup, and a limit of 0 turns the check off. The events table is now cleaned up with the same "log retention days" setting as request logs.
- The macOS Dock icon now has the padding Apple's icon grid expects, so it is no longer a size bigger than other apps in the Dock and in Cmd+Tab.

## Other
- Removed the request-type events: no screen used them and they duplicated the request logs. App log files now rotate daily and the last 14 are kept.
- Built-in provider definitions are now compiled into the app, so the install folder no longer contains yaml or sql files.
- The README has a new Terminal UI section in all three languages.

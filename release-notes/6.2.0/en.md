This release adds switchable themes (Sketch, Classic, Win2000) and config export / import. After changing the proxy port, protocol and similar settings, you can now restart the proxy from the Settings page instead of restarting the whole app. Default behavior is unchanged after upgrading: the interface still uses the Sketch theme.

## Features
- **Themes**: Three themes to choose from: Sketch (default, the hand-drawn notebook style of the website), Classic (the plain look from earlier versions) and Win2000 (a retro Windows 2000 look). Switch in Settings → General → Appearance, or cycle through them with the theme button at the bottom of the sidebar.
  - A new color mode button at the bottom of the sidebar cycles through System → Light → Dark.
  - With the Classic theme, the Live routing page returns to its earlier full-width layout and routing diagram.
  - On Windows / Linux the Win2000 theme also draws the window title bar in Windows 2000 style; macOS keeps the system traffic-light buttons.
- **Config export / import**: Export your subscriptions and virtual model settings in Settings → Backup & migration, then import them on another machine or after a reinstall. Request logs and statistics are not included.
  - You can choose to include API keys and the proxy access token in the export; this part is encrypted with a password you set, and the password cannot be recovered. ChatGPT / Kiro sign-in credentials are never exported; sign in again on the new machine.
  - Before importing, a preview shows whether each subscription will be added or skipped: subscriptions that already exist on this machine, ones that need a fresh sign-in and ones that fail validation are skipped with the reason shown.
  - Subscriptions imported without keys start disabled and are marked "API key needed" in the subscription list.
  - On first launch, the page for adding your first subscription also offers "Import from backup".
  - The Web UI can only export configs without keys.
- **Restart proxy service**: After changing the protocol, port, listen address, request body limit or HTTP/2, click "Restart proxy service" in Settings → Proxy → Proxy service to apply it without restarting the app. Requests in progress are not interrupted.
  - If the new port is taken, the next free port is used and you are shown the new address for your clients; if the new config fails to start, the previous config is restored and keeps running.
  - Regenerating the HTTPS certificate or changing its extra IPs / domains now takes effect immediately, with no restart needed.

## Fixes
- **Error messages shown as `[object Object]`**: When some actions failed, the message only said `[object Object]`. It now shows the actual reason.
- **Kanji rendered with Chinese glyphs in the Japanese UI**: The page language used to be fixed to Chinese, so kanji in the Japanese UI were rendered with Chinese glyphs and browsers offered to translate the Web UI. It now follows the interface language.

## Other
- The Settings page has two new tabs, "Web & TUI" and "Backup & migration"; the Web UI and Terminal UI settings moved from "Security & Access" to "Web & TUI".
- The disclaimer on the About page and the first-launch page now states that cc-router is an independent third-party open-source project, not affiliated with Anthropic, and not an official Claude / Claude Code app.
- Upgraded Tauri to 2.12 and updated rustls to fix a security advisory; launch at login, if already enabled, is unaffected. Building from source now requires Rust 1.90 or later.

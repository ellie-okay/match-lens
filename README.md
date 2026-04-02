# Match Lens

An open source, local-only replay recorder for League of Legends. No account, no uploads, no subscription. Recordings stay on your machine.

> A free alternative to Outplayed, Medal, and similar tools.

**Windows 10 / 11 only.**

## Install

1. Download **Match Lens_1.0.0_x64-setup.exe** from the [latest release](../../releases/latest)
2. Run the installer
3. Match Lens will appear in your system tray

That's it. The next time you play a game, it records automatically.

## How it works

Match Lens watches the League client in the background. When a game starts it begins recording your screen and audio. When the game ends the recording is saved and the review window opens.

The review window shows:
- **Video playback** of the recording
- **Event markers** on the timeline: click any kill, death, Dragon, Baron, or other objective to jump to 10 seconds before it happened
- **Network graph** showing your ping and packet loss throughout the game

Nothing is uploaded anywhere. Everything is stored locally in your Videos folder.

## No setup required

Match Lens uses the League client's own API to detect games. It doesn't need you to configure anything or point it at a folder. Just install and play.

The first time you run it, you'll find it in the system tray (bottom-right of your taskbar). The review window opens automatically after each game, or you can open it any time by double-clicking the tray icon.

## Storage

Recordings can add up. By default Match Lens will delete your oldest recordings when your total usage exceeds 50 GB. You can change this limit (or the recording resolution) in Settings, accessible from the tray icon.

## Privacy

- No account or login required
- No data leaves your machine
- No telemetry or analytics of any kind
- Source code is publicly available and MIT licensed

## Building from source

Requirements: [Rust](https://rustup.rs/) stable, [Node.js](https://nodejs.org/) 18+, Windows 10/11.

```powershell
git clone https://github.com/ellie-okay/match-lens
cd match-lens
./setup.ps1
npm run tauri build
```

The installer will be in `src-tauri/target/release/bundle/nsis/`.

To cut a release, push a `v*` tag. GitHub Actions will build and publish the installer automatically.

## License

MIT. See [LICENSE](LICENSE).

> Match Lens is not affiliated with or endorsed by Riot Games.

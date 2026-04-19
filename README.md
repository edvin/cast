# Cast

A local network media server, Apple TV client, and desktop management app for streaming your video library. A Rust server indexes your media folders, fetches metadata and artwork from TMDB, and streams video with automatic MKV→MP4 remuxing. A tvOS app discovers the server via Bonjour and provides a cinematic browsing and playback experience. A Tauri desktop app provides a system tray server with library management, drag-and-drop import, and real-time monitoring.

## Architecture

```
┌─────────────┐        HTTP (LAN)        ┌──────────────┐
│  Apple TV   │ ◄──────────────────────► │  Cast Server  │
│  tvOS App   │   Browse, stream, track  │  (Rust/axum)  │
│  (SwiftUI)  │   Bonjour discovery      │  Your PC/Mac  │
└─────────────┘                          └──────────────┘
```

## Media Library Setup

Organize your media directory with one subfolder per show:

```
media/
├── Planet Earth/
│   ├── S01E01 From Pole to Pole.mp4
│   ├── S01E02 Mountains.mkv
│   └── ...
├── Cosmos/
│   ├── 01x01 - Standing Up in the Milky Way.mp4
│   └── ...
└── The Joy of Painting/
    ├── Episode 01 - A Walk in the Woods.mp4
    └── ...
```

**Episode naming** — these patterns are auto-parsed for season/episode numbers:
- `S01E03 - Episode Title.mp4` (also works with scene-release names like `show.name.s01e03.720p.web.mkv`)
- `01x03 - Title.mp4`
- `Episode 03 - Title.mp4`
- `03 - Title.mp4`
- `03.mp4`

**Video formats** — MP4/MOV files play natively. MKV, AVI, and WebM files are automatically remuxed to MP4 in the background when detected (requires ffmpeg). The original file is deleted once remuxing completes. HEVC 10-bit video is transcoded to H.264 for Apple TV compatibility. If a file hasn't been remuxed yet when you hit play, it streams on-the-fly.

**Subtitles** — external `.srt` files are detected alongside video files:
- `S01E01.srt` — matches `S01E01.mp4`, defaults to English
- `S01E01.en.srt` — English subtitles
- `S01E01.sv.srt` — Swedish subtitles
- Multiple languages supported per episode

Embedded subtitles in the video container (MKV/MP4) are preserved during remux and appear in the tvOS player's subtitle picker.

**Artwork** — place any of these in a show folder and they'll be detected automatically:
- Poster: `.poster.jpg`, `.poster.png` (also accepts legacy: `poster.jpg`, `folder.jpg`, `cover.jpg`)
- Backdrop: `.backdrop.jpg`, `.backdrop.png` (also accepts: `backdrop.jpg`, `fanart.jpg`)

Or let TMDB fetch artwork automatically (see below).

## Quick Start

### 1. Build the server

```bash
cd server
cargo build --release
```

Requires [Rust](https://rustup.rs/). The binary is at `server/target/release/cast-server`.

### 2. Configure

Create `server/.env`:

```env
CAST_MEDIA_PATH=/path/to/your/media
TMDB_API_KEY=your-tmdb-api-key
CAST_SERVER_NAME=Living Room
```

Get a free TMDB API key at [themoviedb.org/settings/api](https://www.themoviedb.org/settings/api) (optional but recommended — provides descriptions, episode info, cast, and artwork).

### 3. Run

```bash
cd server
cargo run --release
```

The server starts on port 3456 and advertises itself via Bonjour.

### 4. Desktop app (recommended for Windows/macOS)

Download the installer from [Releases](../../releases):
- **Windows**: `Cast Server Setup.exe`
- **macOS**: `Cast Server.dmg`

The desktop app includes:
- System tray icon — server runs in the background
- Library browser with artwork, episode details, and file status
- Drag-and-drop import — drop video files to auto-organize into series folders
- Remux management — convert MKV files to MP4 with one click
- Delete watched episodes — bulk cleanup with file size info
- Settings UI — configure media path, TMDB key, server name
- Real-time log viewer

### 5. Install the tvOS app

Open `Cast/Cast.xcodeproj` in Xcode, select your Apple TV as the run destination, and deploy. The app auto-discovers the server on the local network.

## Server Configuration

All options can be set via CLI flags, environment variables, or `.env` file:

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--media` | `CAST_MEDIA_PATH` | *(required)* | Path to media directory |
| `--port` | — | `3456` | HTTP port |
| `--name` | `CAST_SERVER_NAME` | `Cast Server` | Bonjour display name |
| `--tmdb-key` | `TMDB_API_KEY` | *(none)* | TMDB API key for metadata |
| `--log-file` | — | `false` | Log to files instead of stdout |

## TMDB Metadata

When a TMDB API key is configured, the server automatically:

- Searches TMDB by folder name (cleans common tags like year, resolution, encoding)
- Downloads poster and backdrop artwork
- Fetches show info: title, overview, genres, rating, year
- Fetches episode info: title, overview, air date, runtime, cast & guest stars

**If auto-matching fails**, create a `tmdb.txt` file in the show's folder containing just the TMDB ID:

```
# Find the ID at https://www.themoviedb.org/tv/
echo 12345 > "/path/to/media/My Show/tmdb.txt"
```

You can also trigger a metadata refresh from the app (Refresh button) or via the API:

```bash
curl -X POST http://localhost:3456/api/metadata/fetch
```

## Dependencies

- **ffmpeg** (recommended) — **required** for MKV/AVI/WebM files. The server automatically converts these to MP4 in the background and deletes the originals. Without ffmpeg, only native MP4/MOV files will play.
- **ffprobe** — enables video duration detection (bundled with ffmpeg)

Install via your package manager:

| Platform | Command |
|----------|---------|
| macOS | `brew install ffmpeg` |
| Windows | `winget install ffmpeg` or download from [gyan.dev/ffmpeg/builds](https://www.gyan.dev/ffmpeg/builds/) and add to PATH |
| Linux | `apt install ffmpeg` / `dnf install ffmpeg` |

## Windows Deployment

1. Download `cast-server-windows-amd64.exe` from [Releases](../../releases)
2. Place it in a folder (e.g. `C:\Cast\`)
3. Copy `scripts\install-windows.ps1` and `scripts\uninstall-windows.ps1` into the same folder
4. Create a `.env` file next to the binary:
```
CAST_MEDIA_PATH=D:\Media
TMDB_API_KEY=your-key
CAST_SERVER_NAME=Living Room
```
5. Run from PowerShell:
```powershell
.\install-windows.ps1
```

This creates a Task Scheduler entry that auto-starts at login with file logging. Logs go to `<media-dir>\logs\`.

```powershell
# Management
Stop-ScheduledTask -TaskName CastServer
Start-ScheduledTask -TaskName CastServer
.\uninstall-windows.ps1
```

### Wake-on-LAN

The tvOS app stores the server's MAC address after a successful connect and
shows a **Wake Server** button on the "Server Unreachable" screen. Tap it to
broadcast a magic packet on the local subnet. For this to actually wake a
sleeping Windows machine you need:

1. **BIOS / UEFI** — enable "Wake on LAN" / "Power on by PCI-E" / "Resume by PME"
   (wording varies by vendor).
2. **Network adapter** in Windows Device Manager → *Properties* → *Power
   Management*: enable **Allow this device to wake the computer** and
   **Only allow a magic packet to wake the computer**. On the *Advanced* tab,
   set **Wake on Magic Packet** = Enabled. If present, also enable
   **Wake on pattern match**.
3. **Windows power plan** — disable Fast Startup
   (Control Panel → Power Options → *Choose what the power button does* →
   uncheck *Turn on fast startup*). Fast Startup leaves the NIC in an
   unpowered state where WoL doesn't work on most adapters.
4. **Router/switch** — WoL is a local-network feature. The magic packet has
   to reach the server's subnet; cross-subnet WoL is out of scope here.

You can verify WoL works from another machine on the LAN with `wakeonlan`
(macOS: `brew install wakeonlan`) or any Windows WoL utility, independent of
the Cast app.

## API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/continue-watching` | In-progress shows, sorted by recency |
| GET | `/api/series` | All shows with metadata and watch summary |
| GET | `/api/series/{id}` | Show detail with episodes |
| GET | `/api/series/{id}/next` | Smart next episode recommendation |
| GET | `/api/series/{id}/art` | Poster image |
| GET | `/api/series/{id}/backdrop` | Backdrop image |
| DELETE | `/api/series/{id}` | Delete series and all files |
| DELETE | `/api/series/{id}/progress` | Reset all watch progress for a show |
| POST | `/api/series/{id}/remux` | Trigger MKV→MP4 remux for all episodes |
| GET | `/api/episodes/{id}/stream` | Video stream (byte-range, auto-remux) |
| GET | `/api/episodes/{id}/thumbnail` | Episode thumbnail |
| GET | `/api/episodes/{id}/progress` | Watch progress |
| POST | `/api/episodes/{id}/progress` | Update watch progress |
| DELETE | `/api/episodes/{id}/progress` | Mark episode as unwatched |
| GET | `/api/episodes/{id}/credits` | Cast & guest stars (from TMDB, cached) |
| GET | `/api/episodes/{id}/subtitles` | List available subtitle languages |
| GET | `/api/episodes/{id}/subtitles/{lang}` | Subtitle file (SRT converted to WebVTT) |
| DELETE | `/api/episodes/{id}` | Delete episode and related files |
| POST | `/api/episodes/{id}/prepare` | Trigger on-demand remux, returns progress |
| GET | `/api/episodes/watched` | All watched episodes with file info |
| POST | `/api/metadata/fetch` | Trigger TMDB metadata refresh |

All error responses return JSON: `{"error": "...", "code": 404, "detail": null}`

## Development

```bash
# Run server in development
cd server
echo 'CAST_MEDIA_PATH=/path/to/media' > .env
echo 'TMDB_API_KEY=your-key' >> .env
cargo run

# Run tests
cargo test
```

The tvOS app requires Xcode and an Apple Developer account. Build and deploy to your Apple TV over the same local network as the server.

## Project Structure

```
cast/
├── server/              # Rust media server (library crate + CLI binary)
│   ├── src/
│   │   ├── lib.rs       # Server library (start_server, AppState, background tasks)
│   │   ├── main.rs      # CLI binary wrapper
│   │   ├── library.rs   # Media directory scanner, filename parsing
│   │   ├── db.rs        # SQLite (watch progress + metadata + credits cache)
│   │   ├── routes.rs    # HTTP API endpoints
│   │   ├── tmdb.rs      # TMDB API client + metadata fetching
│   │   ├── media.rs     # ffprobe/ffmpeg integration
│   │   └── mdns.rs      # Bonjour/mDNS advertisement
│   └── Cargo.toml
├── desktop/             # Tauri 2 desktop app
│   ├── src-tauri/       # Rust backend (tray, commands, file ops)
│   └── gui/             # HTML/CSS/JS frontend
├── Cast/                # tvOS SwiftUI app (Xcode project)
├── scripts/             # Windows install/uninstall scripts
└── Cargo.toml           # Workspace root
```

## License

MIT License. See [LICENSE](LICENSE) for details.

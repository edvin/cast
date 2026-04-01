# Cast

A local network media server and Apple TV client for streaming your video library. A Rust server indexes your media folders, fetches metadata and artwork from TMDB, and streams video with on-the-fly remuxing. A tvOS app discovers the server via Bonjour and provides a cinematic browsing and playback experience.

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

### 4. Install the tvOS app

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

- **ffmpeg** (recommended) — **required** for MKV/AVI/WebM files. The server automatically converts these to MP4 in the background and deletes the originals. Without ffmpeg, only native MP4/MOV files will play. Also enables episode thumbnail generation.
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

## API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/continue-watching` | In-progress shows, sorted by recency |
| GET | `/api/series` | All shows with metadata and watch summary |
| GET | `/api/series/{id}` | Show detail with episodes |
| GET | `/api/series/{id}/next` | Smart next episode recommendation |
| GET | `/api/series/{id}/art` | Poster image |
| GET | `/api/series/{id}/backdrop` | Backdrop image |
| DELETE | `/api/series/{id}/progress` | Reset all watch progress for a show |
| GET | `/api/episodes/{id}/stream` | Video stream (byte-range, auto-remux) |
| GET | `/api/episodes/{id}/thumbnail` | Episode thumbnail |
| GET | `/api/episodes/{id}/progress` | Watch progress |
| POST | `/api/episodes/{id}/progress` | Update watch progress |
| DELETE | `/api/episodes/{id}/progress` | Mark episode as unwatched |
| GET | `/api/episodes/{id}/credits` | Cast & guest stars (from TMDB, cached) |
| GET | `/api/episodes/{id}/subtitles` | List available subtitle languages |
| GET | `/api/episodes/{id}/subtitles/{lang}` | Subtitle file (SRT converted to WebVTT) |
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
├── server/              # Rust media server
│   ├── src/
│   │   ├── main.rs      # CLI, startup, periodic rescan
│   │   ├── library.rs   # Media directory scanner, filename parsing
│   │   ├── db.rs        # SQLite (watch progress + metadata)
│   │   ├── routes.rs    # HTTP API endpoints
│   │   ├── tmdb.rs      # TMDB API client + metadata fetching
│   │   ├── media.rs     # ffprobe/ffmpeg integration
│   │   └── mdns.rs      # Bonjour/mDNS advertisement
│   └── Cargo.toml
├── Cast/                # tvOS SwiftUI app (Xcode project)
└── scripts/             # Windows install/uninstall scripts
```

## License

MIT License. See [LICENSE](LICENSE) for details.

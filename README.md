# Cast

Stream your local TV series to Apple TV over the local network. A Rust media server indexes your library, fetches metadata from TMDB, and serves video with byte-range streaming. A tvOS app discovers the server via Bonjour, presents a premium browsing experience, and plays episodes with full transport controls and progress tracking.

## Architecture

```
┌─────────────┐        HTTP (LAN)        ┌──────────────┐
│  Apple TV   │ ◄──────────────────────► │  Cast Server  │
│  tvOS App   │   Browse, stream, track  │  (Rust/axum)  │
│  (SwiftUI)  │   Bonjour discovery      │  Your PC/Mac  │
└─────────────┘                          └──────────────┘
```

## Media Library Setup

Organize your media directory with one subfolder per series:

```
media/
├── Breaking Bad/
│   ├── S01E01 Pilot.mp4
│   ├── S01E02 Cat's in the Bag.mp4
│   └── ...
├── The Wire/
│   ├── 01x01 - The Target.mp4
│   └── ...
```

**Episode naming** — these patterns are auto-parsed for season/episode numbers:
- `S01E03 - Episode Title.mp4`
- `01x03 - Title.mp4`
- `Episode 03 - Title.mp4`
- `03 - Title.mp4`
- `03.mp4`

**Subtitles** — external `.srt` files are detected alongside video files:
- `S01E01.srt` — matches `S01E01.mp4`, defaults to English
- `S01E01.en.srt` — English subtitles
- `S01E01.sv.srt` — Swedish subtitles
- Multiple languages supported per episode

Embedded subtitles in the video container also work automatically.

**Artwork** — place any of these in a series folder and they'll be detected automatically:
- Poster: `poster.jpg`, `poster.png`, `folder.jpg`, `cover.jpg`
- Backdrop: `backdrop.jpg`, `fanart.jpg`

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
CAST_MEDIA_PATH=/path/to/your/shows
TMDB_API_KEY=your-tmdb-api-key
```

Get a free TMDB API key at [themoviedb.org/settings/api](https://www.themoviedb.org/settings/api) (optional but recommended — provides series descriptions, episode info, and artwork).

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
| `--name` | — | `Cast Server` | Bonjour display name |
| `--tmdb-key` | `TMDB_API_KEY` | *(none)* | TMDB API key for metadata |
| `--log-file` | — | `false` | Log to files instead of stdout |

## TMDB Metadata

When a TMDB API key is configured, the server automatically:

- Searches TMDB by series folder name (cleans common tags like year, resolution, encoding)
- Downloads poster and backdrop artwork
- Fetches series info: title, overview, genres, rating, year
- Fetches episode info: title, overview, air date, runtime

**If auto-matching fails**, create a `tmdb.txt` file in the series folder containing just the TMDB ID:

```
# Find the ID at https://www.themoviedb.org/tv/
# e.g. Breaking Bad is https://www.themoviedb.org/tv/1396
echo 1396 > "/path/to/media/Breaking Bad/tmdb.txt"
```

You can also trigger a metadata refresh anytime:

```bash
curl -X POST http://localhost:3456/api/metadata/fetch
```

## Optional Dependencies

- **ffmpeg** — enables episode thumbnail generation (on-demand)
- **ffprobe** — enables video duration detection

Install via your package manager (`brew install ffmpeg`, `winget install ffmpeg`, etc.).

## Windows Deployment

For running as a background service on Windows:

```powershell
.\scripts\install-windows.ps1 -MediaPath "D:\Shows" -TmdbKey "your-key"
```

This creates a Task Scheduler entry that auto-starts at login with file logging. Logs go to `<media-dir>\logs\`.

```powershell
# Management
Stop-ScheduledTask -TaskName CastServer
Start-ScheduledTask -TaskName CastServer
.\scripts\uninstall-windows.ps1
```

## API

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/continue-watching` | In-progress series, sorted by recency |
| GET | `/api/series` | All series with metadata and watch summary |
| GET | `/api/series/{id}` | Series detail with episodes |
| GET | `/api/series/{id}/next` | Smart next episode recommendation |
| GET | `/api/series/{id}/art` | Series poster image |
| GET | `/api/series/{id}/backdrop` | Series backdrop image |
| DELETE | `/api/series/{id}/progress` | Reset all watch progress for a series |
| GET | `/api/episodes/{id}/stream` | Video stream (byte-range support) |
| GET | `/api/episodes/{id}/thumbnail` | Episode thumbnail |
| GET | `/api/episodes/{id}/progress` | Watch progress |
| POST | `/api/episodes/{id}/progress` | Update watch progress |
| DELETE | `/api/episodes/{id}/progress` | Mark episode as unwatched |
| GET | `/api/episodes/{id}/subtitles` | List available subtitle languages |
| GET | `/api/episodes/{id}/subtitles/{lang}` | Subtitle file (SRT converted to WebVTT) |
| POST | `/api/metadata/fetch` | Trigger TMDB metadata refresh |

All error responses return JSON: `{"error": "...", "code": 404, "detail": null}`

## Development

```bash
# Run server in development
cd server
echo 'CAST_MEDIA_PATH=/path/to/shows' > .env
echo 'TMDB_API_KEY=your-key' >> .env
cargo run

# Run tests
cargo test

# Lint
cargo clippy -- -D warnings
cargo fmt --all -- --check
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
├── Cast/                 # tvOS SwiftUI app (Xcode project)
├── scripts/             # Windows install/uninstall scripts
├── APP_PLAN.md          # Detailed tvOS app implementation plan
├── PLAN.md              # Architecture overview
└── CLAUDE.md            # AI assistant context
```

## License

Private project.

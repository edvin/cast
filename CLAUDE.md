# Cast

Local network series streamer — Rust server + tvOS Apple TV app.

## Project Structure
- `server/` — Rust (axum) HTTP media server
- `app/` — tvOS SwiftUI app (Xcode project)

## Media Model
- Each subdirectory in the media folder = a series
- Video files in a series folder = episodes (sorted by filename)
- Art auto-detected: poster.jpg/png, folder.jpg/png, cover.jpg/png, banner.jpg/png
- Backdrop auto-detected: backdrop.jpg/png, fanart.jpg/png
- Watch progress in SQLite `cast.db` in media directory
- Single user, no auth

## Server API
- `GET /api/series` — list series with watch summary
- `GET /api/series/{id}` — series detail + episodes + progress
- `GET /api/series/{id}/next` — smart next episode (resume/next/first)
- `GET /api/series/{id}/art` — series poster artwork
- `GET /api/series/{id}/backdrop` — series backdrop/fanart image
- `GET /api/episodes/{id}/stream` — video stream (byte-range)
- `GET/POST /api/episodes/{id}/progress` — watch progress
- `POST /api/metadata/fetch` — trigger TMDB art download for series missing artwork
- Bonjour service type: `_cast-media._tcp`

## TMDB Integration
- Optional: pass `--tmdb-key <key>` or set `TMDB_API_KEY` env var
- On startup, auto-fetches posters + backdrops for series without art
- Can also trigger manually via `POST /api/metadata/fetch`

## App UX Flow
1. Auto-discover server via Bonjour
2. Series grid with poster art
3. Select series → smart next episode (resume or next unwatched)
4. Full-screen AVPlayer with periodic progress reporting

## Running Server
```bash
cargo run -- --media /path/to/shows --port 3456
# With TMDB artwork fetching:
cargo run -- --media /path/to/shows --tmdb-key YOUR_API_KEY
```

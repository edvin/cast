# Cast

Local network series streamer — Rust server + tvOS Apple TV app.

## Project Structure
- `server/` — Rust (axum) HTTP media server
- `app/` — tvOS SwiftUI app (Xcode project)
- `scripts/` — Windows install/uninstall PowerShell scripts

## Media Model
- Each subdirectory in the media folder = a series
- Video files in a series folder = episodes (sorted by filename)
- Filename parsing: S01E03, 01x03, Episode 03, bare numbers — extracts season/episode numbers and titles
- Art auto-detected: poster.jpg/png, folder.jpg/png, cover.jpg/png, banner.jpg/png
- Backdrop auto-detected: backdrop.jpg/png, fanart.jpg/png
- Thumbnails generated on-demand via ffmpeg, stored in `.thumbnails/`
- Watch progress + metadata in SQLite `cast.db` in media directory
- TMDB override: place `tmdb.txt` with TMDB ID in series folder for manual matching
- Single user, no auth

## Server API
- `GET /api/continue-watching` — series with in-progress/next-up episodes, sorted by recency
- `GET /api/series` — list series with metadata (overview, genres, rating, year, watch summary, has_metadata)
- `GET /api/series/{id}` — series detail + episodes with metadata + progress
- `GET /api/series/{id}/next` — smart next episode (resume/next/first)
- `GET /api/series/{id}/art` — series poster artwork
- `GET /api/series/{id}/backdrop` — series backdrop/fanart image
- `DELETE /api/series/{id}/progress` — reset all watch progress for a series
- `GET /api/episodes/{id}/stream` — video stream (byte-range)
- `GET /api/episodes/{id}/thumbnail` — episode thumbnail (generated on-demand via ffmpeg)
- `GET/POST /api/episodes/{id}/progress` — get/update watch progress
- `DELETE /api/episodes/{id}/progress` — reset episode watch progress (mark unwatched)
- `GET /api/progress` — all watch progress entries
- `POST /api/metadata/fetch` — trigger TMDB metadata + art fetch
- All error responses are JSON: `{"error": "...", "code": 404, "detail": "..."}`
- Bonjour service type: `_cast-media._tcp`

## TMDB Integration
- Optional: pass `--tmdb-key <key>` or set `TMDB_API_KEY` env var
- Folder name cleaning: strips year, resolution, source, encoding tags before searching
- Override: `tmdb.txt` file in series folder with TMDB ID for manual matching
- Fetches: series info (title, overview, genres, rating, year), episode info (title, overview, air date, runtime, still images), posters, backdrops
- On startup, auto-fetches for series missing metadata/art
- Can also trigger manually via `POST /api/metadata/fetch`
- All metadata stored in SQLite (series_metadata + episode_metadata tables)

## App UX Flow
1. Auto-discover server via Bonjour
2. "Continue Watching" row at top (from /api/continue-watching)
3. Series grid with poster art, overview, genres
4. Select series → smart next episode (resume or next unwatched)
5. Episode list with thumbnails, titles, overviews, watch status
6. Full-screen AVPlayer with periodic progress reporting
7. English subtitles auto-selected, user can switch via tvOS player menu

## Running Server
```bash
cargo run -- --media /path/to/shows --port 3456
# With TMDB:
cargo run -- --media /path/to/shows --tmdb-key YOUR_API_KEY
# With file logging (for Windows/background):
cargo run -- --media /path/to/shows --tmdb-key YOUR_API_KEY --log-file
```

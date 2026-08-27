# Aurorix Product Overview

Aurorix Music is a local-first music center for Windows and Android. It combines
local music management, high-quality playback, optional external music
Providers, extensibility, personal statistics, and optional multi-device Sync.
It is not a clone of one streaming service and does not require a Cloud account
for local cataloging or local playback.

## Product Surfaces

| Surface | Purpose | Planned owner | Stage |
|---|---|---|---|
| Home | Recently played, recently added, favorites, continue listening, and later recommendations | Core projections plus native UI | Gate 3 and later |
| Library | Songs, albums, artists, genres, folders, metadata, sorting, filtering, and batch actions | Rust library Core plus native UI | Gate 1 Core, Gate 3 UI |
| Search | Local-first search across songs, albums, artists, playlists, and later Providers | Application/Core facade | Gate 1 local Core, later Provider merge |
| Player | Current track, progress, transport controls, volume, playback policy, and quality state | Playback/audio Core plus native host | Gate 2 Core, Gate 3/6 output |
| Queue | Ordered upcoming items, shuffle, repeat, history, and save-as-playlist | Playback Core | Gate 2 Core, Gate 3 UI |
| Playlist | User-created playlists, favorites, ordering, and later sharing/Sync | Core replicated state plus native UI | Gate 1 Sync foundation, Gate 3/4 |
| Music details | Recording, release, artist, credits, metadata, artwork, audio facts, and lyrics | Library/Core projections plus UI | Gate 3 and Provider stages |
| Lyrics | Local and Provider lyric documents, translations, transliteration, and timing | Clock consumer plus Provider/UI layers | Clock foundation in Gate 2, content later |
| Audio Center | Output status, format, latency, DSP policy, EQ, and future visualizers | Audio Core plus platform adapter | Gate 2 contracts, later DSP/platform work |
| Statistics | Listening time, top songs/artists/albums, history, trends, and distributions | Play facts plus local projections | Gate 1 Core, later UI |
| Provider Center | External search, catalog, streaming, lyrics, artwork, playlists, and account state | Provider Host and SDK | Gate 5 |
| Extension Center | Provider, theme, DSP, visualizer, and tool packages | Extension Host and SDK | Gate 5 and later |
| Theme Center | Light/dark/system themes, materials, accent, motion, and visual effects | Native UI and Theme SDK | Later release |
| Sync Center | Device state, account-scoped replicated data, conflicts, and recovery status | Sync Core plus Cloud | Gate 1 local foundation, Gate 4 transport |
| Account | Profile, devices, sessions, security, and account lifecycle | Cloud server plus native clients | Gate 4 |
| Settings | Appearance, playback, audio, library, Provider, extensions, Sync, account, and advanced settings | Core contracts plus native clients | Gate 3-6 by setting ownership |
| Web control surface | Administrative and remote-control views | Web client over approved APIs | Later release |

## Local-first Boundary

The following remain usable without an account, Provider, or Cloud service:

- Local catalog discovery and search.
- Local metadata and local asset availability tracking.
- Local queue and playback state.
- Local playback history and derived statistics.

Device-local files, OS URI permissions, Provider credentials, output-device
configuration, themes, and runtime leases are not replicated as portable user
state. Playlists, favorites, and finalized play facts may be replicated through
the versioned Sync boundary.

## Playback Scope

The first playback Core targets local WAV PCM, FLAC, MP3 CBR/VBR, AAC-LC
M4A/ADTS, and Opus/Ogg input within bounded format limits. It uses a shared
Rust playback/audio boundary so Windows and Android hosts can later provide
native output without owning a second queue or clock.

Gate 2 proves the offline control/data path and deterministic realtime behavior.
Windows output, Android service integration, EQ, visualizers, advanced DSP,
and strict bit-perfect claims require later platform or contract evidence.

## Product Principles

- Local capability is not made dependent on remote service availability.
- Native clients render Core state and send Core commands.
- Provider capabilities are replaceable and permission-scoped.
- Playback facts are derived from the presentation clock, not UI timers.
- Requested quality is distinct from actual delivered format and processing.
- Missing local assets do not erase their catalog or replicated identity.
- Accessibility, reduced motion, battery, and resource limits override visual
  effects.

## Current Status

- Gate 0 is complete.
- Gate 1 local Core foundations are complete.
- Gate 2 offline playback Core is planned and not yet implemented.
- Windows, Android, Cloud, Provider, FFI, and Web runtime implementations are
  not yet complete.

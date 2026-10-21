# Falcon RGB

Rust replacement for the Windows configuration software of the **MAX Falcon-20 RGB** (20-key RGB macro
keypad, USB `195D:6008`). Target: macOS first, portable to Windows/Linux via hidapi + egui.

- Protocol description: `docs/protocol.md` (cited in code as "spec §n"). Mark anything not verified on hardware
  as **[HW-VERIFY]** in code comments.
- Every device action must also work against the simulated device (`--simulate`), which records the exact bytes
  sent, so most work needs no keypad.
- Never cite addresses, symbol names or code of the original Windows software; describe observable behaviour
  only ("Windows sends …", "Windows waits …").

## Layout

- `crates/falcon-core` — protocol (packet encoding), data model, transport trait, hidapi transport (feature `hid`),
  simulated device. No UI code.
- `crates/falcon-cli` — `falcon` binary: list/query devices, apply settings, `--dry-run` hex dumps.
- `crates/falcon-gui` — egui/eframe app; the same crate builds for the desktop and for the browser
  (`wasm32-unknown-unknown`, trunk, WebHID). `web/spike/` is a plain-JS WebHID test page for protocol debugging.

## Browser build (WebHID)

- `just web` (trunk serve, http://127.0.0.1:8080, `?simulate`), `just web-build` → `dist/web/`; CI deploy in
  `.github/workflows/pages.yml`. Needs rustup's toolchain (`PATH=$HOME/.cargo/bin:$PATH`; Homebrew rustc has no wasm
  std) and `trunk`. `just check`/`just lint` also check the wasm target.
- `falcon-core` stays sync-first; `asynch.rs` adds `AsyncTransport` + `AsyncFalcon` (+ `AsyncRecorder`) that run the
  same `encode` plans. A test asserts byte-identical packet logs against the sync `Falcon`. The workspace dep has
  `default-features = false`; the CLI and native GUI enable `hid`.
- GUI split by `cfg(target_arch = "wasm32")`: `worker/native.rs` (thread, hidapi) vs `worker/web.rs` (one
  `spawn_local` task: std mpsc polled every 15 ms, `Dev::Sim` sync simulator or `Dev::Hid` = `AsyncFalcon<WebHid>`),
  `worker/webhid.rs` (web-sys HID, needs `--cfg=web_sys_unstable_apis` from `.cargo/config.toml`). Pairing:
  `Command::Pair` → `requestDevice` (Chrome's transient user activation still holds when the task runs);
  `Connection::NeedsPairing`/`Unsupported` are web-only states. Hot-plug: `navigator.hid` connect/disconnect + 2 s
  `getDevices` re-scan.
- Other web gates: config in `localStorage` (`app/persist.rs`, keys `falcon-rgb/config` and `…/config.sim`), file
  dialogs via `files.rs` (download link / async picker into an `Inbox`), `web-time` instead of `std::time::Instant`
  (panics on wasm), no launch keys (`host::CAN_LAUNCH`), no auto layer switch, no Input Monitoring banner.

## Build

- `just` lists recipes: `check` (fastest loop), `build`, `gui`, `cli`, `test` (nextest), `lint`, `release`,
  `bundle` (macOS .app → `dist/`), `dist-windows` (cargo-xwin cross build), `cache-stats`.
- Fast builds come from a user-level `~/.cargo/config.toml` (not part of the repo): sccache wrapper, lld,
  `debug = "line-tables-only"`, incremental, deps at opt-level 1. With it: cold 73 s → after `cargo clean`
  17.5 s (100 % sccache hits) → incremental 0.4 s.
- **Never set `[build] rustflags` in `.cargo/config.toml`**: rustflags sources don't merge, and the global
  `target.<triple>.rustflags` of a user config wins — a `build.rustflags` would be silently ignored on macOS.
  Add flags per `[target.<triple>]` only.
- Profiles: `release` = thin LTO (fast link); `dist` = fat LTO + `panic = "abort"` for shipping. Fat LTO is
  single-threaded and slow — use it only for `bundle`/`dist-windows`.
- All dependency versions live in `[workspace.dependencies]`; crates reference them with `workspace = true`.
- Tables (key names, function codes) are committed Rust source — no `build.rs` codegen, so incremental builds stay
  valid.
- Windows: `.cargo/config.toml` links the MSVC CRT statically (`+crt-static`); cross builds need rustup's
  toolchain (Homebrew rustc has no Windows std), `cargo-xwin` and Homebrew `llvm`.
- `MACOSX_DEPLOYMENT_TARGET = 11.0` is set in `.cargo/config.toml` `[env]`.

## HID on macOS

- hidapi is built with `macos-shared-device`: open devices non-exclusively, otherwise the keypad stops typing while
  the app is connected.
- Opening the keypad needs **Input Monitoring** (TCC) even though only vendor collections are opened (they share
  If1 with the mouse collection). Without it `hid_open_path` fails with `0xE00002E2` (not permitted); the hid
  transport maps that to `Error::PermissionDenied` (friendly message). The CLI works when the terminal has the
  permission; the bundled `Falcon RGB.app` needs its own entry, and ad-hoc signed builds may need re-allowing after
  each rebuild (then restart the app).
- GUI on `PermissionDenied`: status bar + banner with the instructions and an "Open Input Monitoring settings" button
  (`open "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"`,
  `hid::INPUT_MONITORING_SETTINGS_URL`); the worker logs it once and retries quietly every 5 s. No
  `IOHIDRequestAccess` call (would need FFI; `unsafe_code` is denied).

## Core design

- `falcon-core` modules: `protocol` (all constants, delays and [HW-VERIFY] items in one place, packet builders,
  reply/event parsers), `layout` (k ↔ hw ↔ LED), `codes` (key names, function → device byte), `model` (serde
  `Config`), `encode` (pure plans of `Step`s), `transport` (`Transport` trait, `SimulatedDevice`, `Recorder` for
  logging/dry run, `hid`), `device` (`Falcon<T>`), `store` (JSON config).
- Behaviour matching the Windows software: P2 default colour is blue, P3 red;
  a full download writes the whole 25-byte factory template before patching keys (slots 20..24 reset),
  lighting-only keeps the read-back key map; input reports are zero-padded and filtered like Windows
  (profile raw 7 → 1, 6 ignored; `kk` > 19 → `HostCode`); polling rate/key mode only as 8 bytes; `0x82` replies
  must be ≥ 208 bytes with `reply[0] = 07` before any write-back; dry run forwards only byte-exact standard
  queries, `raw --dry-run` opens nothing.
- Decisions from hardware (spec §11), deviating from Windows: combos and Show Desktop/Zoom are **`F8` macros** with
  the full chord (presses in order, releases in reverse, delay 1 = 10 ms, repeat 1) — Windows' `E8` + presses-only
  upload types nothing; `0x81` is read 1-based (1..5, anything else is an error); wave direction byte 127 is
  0 right→left, 1 left→right, 2 top→bottom, 3 bottom→top (`WaveDirection`; old config values `"left"`/`"right"`/
  `{"other": n}` still load with their old bytes); brightness stays 1..=5 = byte + 1, but 1 means LEDs **off**; `04 1F cmd 00 val`
  echoes → `DeviceEvent::CommandAck`, report ID 1 → `MouseReport` (both informational); `0x80` with 264 bytes
  is not implemented (the Get returns the stale feature buffer with byte 1 cleared, e.g. `07 00 01` after a `0x81`)
  → a long reply counts as a version only if `reply[1] = 0x80`, else "not available".
- Non-264-byte feature reports are refused on macOS by the hid transport (`SHORT_REPORTS_BLOCKED`): IOKit rejects
  them and the keypad stops answering until replugged. The CLI refuses `version --len 8`, `raw --len ≠ 264` and
  `experimental …` up front (except with `--simulate`); the GUI only sends 264 bytes.
- Every `0x02` lands in flash: a lighting-only write (no `0x06`) survives a power cycle. Writing a **non-active**
  profile makes the keypad display that profile's lighting at once (preview) while `0x81` still reports the old
  layer; a layer switch or replug shows the active layer again. Full downloads still end with `0x06` (harmless).
- CLI write target without `--profile` is the keypad's active layer (`0x81`); with `--simulate` (a fresh simulator
  on layer 1 every run) or when the query fails, the stored `active_profile`.

## Usage (`just cli <args>` or `target/debug/falcon`)

- Global: `--simulate` (in-memory keypad, config `config.sim.json`), `--dry-run` (writes are logged, not sent;
  0x80/0x81/0x82 queries still reach the keypad), `-v`/`-vv` packet log, `--config FILE`.
- Read: `list`, `doctor`, `version [--len auto|264|8]` (8 refused on macOS), `profile get`, `dump N [--out pN.bin]`,
  `show-config`, `listen [--seconds N]` (prints `ack 0x81 → 2` for echoes, `mouse report (ignored)`), `keys`.
- Backup: `backup [--out FILE]` (JSON, `falcon_core::backup`: the raw `0x82` block of every layer plus config
  macros of `F8` keys), `restore FILE [--layer N [--to M]] [--yes]` (also takes `dump --out` .bin files; asks unless
  `--yes`/`--simulate`/`--dry-run`; afterwards the config follows the restored layers via
  `LayerBackup::apply_to_profile`). GUI: Layer tab → Back up keypad… / Restore backup… (worker `Command::Backup` /
  `Restore`, confirmation modal).
- Write (config is updated first, then sent): `profile set N`,
  `effect wave --direction to-left|to-right|down|up|0..3 --brightness 1..5` (1 = off; bare `left`/`right` refused),
  `key-color 5|bar:lu RRGGBB`, `remap K kp_7|0x2c|ctrl+c|volume-up|disable|default|launch:PATH`,
  `macro K --events "down:lctrl tap:c,50ms 1s up:lctrl" [--repeat N|held|toggle]`, `apply [--all]`.
- Expert: `raw "07 80" --get --yes-i-know` (264 bytes; other lengths refused on macOS),
  `experimental polling-rate|key-mode … --yes-i-know` (8-byte only, refused on macOS).

GUI (`just gui --simulate` or `target/debug/falcon-gui [--simulate] [--config FILE]`, bundle `dist/Falcon RGB.app`):

- Same config files as the CLI (`config.sim.json` with `--simulate`). The "Simulated" toolbar toggle switches the
  transport at runtime only; the config path is fixed at launch.
- A worker thread (`worker/`) owns the transport: channel commands, hot-plug re-scan every 2 s, presence check every
  3 s, `CONNECT_SETTLE` only after a hot-plug arrival, event polling between commands. The UI never blocks.
- Save (⌘S) = validate → config file → full `write_profile` of the edited layer; "Write all layers" writes all 5.
  "Apply to keypad"/Auto-apply (1000 ms debounce, one in flight, skipped when the keypad already holds the value —
  each write costs flash) = `apply_lighting` (no `0x06`, still persistent).
  Orange dot on a layer = keys/lighting differ from what the keypad is believed to hold (sim boots factory defaults;
  hardware: keys are assumed to match the file, lighting is read from the keypad — `0x82` ×5 at connect and Refresh,
  `Update::KeypadLighting` replaces the configured lighting; macros cannot be read back, so keys are not).
- The edited layer follows the keypad: `0x81` at connect and Refresh, our own selects and `ProfileChanged` events.
  `HostCode` events trigger a refresh. `HostKey` runs the key's `LaunchProgram` (macOS `open`/`open -a`).
  `CommandAck`/`MouseReport` only show in the packet log. Firmware shows "n/a" (the 264-byte `0x80` is not implemented).
- Auto layer switch (Layer tab): macOS polls `lsappinfo` every 2 s; other OSes: TODO (no frontmost-app source).
- Macro recording captures key events in the app window (`raw_input_hook`, physical keys → HID); numpad records as
  digit row, media/Fn keys are skipped, OS shortcuts (⌘Q, ⌘Tab) never reach the app.
- Lighting preview is approximate (`preview.rs`); the packet log panel shows every SET/GET/IN packet as hex.
- Screenshot check without hardware: run with `--simulate`, `screencapture -l <window id>`.

Hardware sessions: the first one (2026-10-20) is summarised in spec §11; its raw factory blocks are in
`crates/falcon-core/tests/data/` (`tests/hw_dumps.rs`). `falcon doctor` is
read-only and prints the checklist (`HW_VERIFY`: open items first, then the confirmed ones). Still open: effects
8–10, polling rate, key mode (both 8-byte, not testable on macOS), macro mouse-move encoding, the
effect/brightness/direction event filters (no on-keypad keys found), what `0x06` actually does.

## Conventions

- `cargo fmt`, clippy with `-D warnings`, `unsafe_code = "deny"`.
- Documentation and code comments in English.

# Falcon RGB

An unofficial, open-source configuration tool for the **MAX Falcon-20 RGB** macro keypad
(USB `195D:6008`), written in Rust. The keypad only ships with Windows software; this project
runs on macOS first and should work on Windows and Linux through hidapi and egui.

> **Not affiliated with, endorsed by or supported by the keypad's manufacturer.** "MAX",
> "Falcon" and all other product names are trademarks of their respective owners and are used
> here only to identify the hardware this tool works with.

## Features

- Per-layer key assignments for all 20 keys and 5 layers: single keys, key combos, macros
  (recorded or edited, with delays and repeat modes), mouse buttons, scrolling, media keys and
  layer switching.
- "Launch program" keys: the keypad reports the key press and the app opens a program on the
  computer.
- Lighting: static, wave and custom per-key colours, the four light bars, brightness and wave
  direction.
- Automatic layer switching based on the frontmost app (macOS).
- The desktop app follows layer changes made on the keypad itself.
- A command-line tool (`falcon`) for scripting, plus hex packet logs, dry runs and a simulated
  keypad (`--simulate`) for working without hardware.

## Status

The protocol was worked out for interoperability and tested on a real keypad on macOS. Reading
and writing layers, lighting, the LED map, key remaps, macros, combos, persistence and layer
events are confirmed. Polling rate, key mode and effects 8–10 are still untested; items not
confirmed on hardware are marked `[HW-VERIFY]` in the code. `falcon doctor` prints the current
list. See [docs/protocol.md](docs/protocol.md) for the protocol description.

## Building

Requires a recent stable Rust toolchain (edition 2024) and optionally
[just](https://github.com/casey/just) and [cargo-nextest](https://nexte.st).

```sh
just build                 # or: cargo build --workspace
just gui                   # desktop app
just gui --simulate        # desktop app against a simulated keypad
just cli list              # command-line tool
just test
just bundle                # macOS: dist/Falcon RGB.app (ad-hoc signed)
```

Run `just` to list all recipes.

## macOS permission

macOS only lets apps open the keypad with the **Input Monitoring** permission (System Settings
→ Privacy & Security → Input Monitoring), even though the app only uses the keypad's vendor
configuration interface. Without the permission the app shows a banner with a button that opens
the settings page. Ad-hoc signed builds may need the permission again after every rebuild:
remove the old entry, add the app again and restart it.

The CLI uses the permission of the terminal it runs in.

## Command-line examples

```sh
falcon list                                   # find the keypad
falcon doctor                                 # read-only self-test
falcon profile set 2                          # switch to layer 2
falcon effect static --color ff8800 --brightness 5
falcon effect wave --direction to-right
falcon remap 1 ctrl+c
falcon macro 2 --events "down:lshift tap:h up:lshift tap:i"
falcon listen --seconds 30                    # print events from the keypad
falcon --simulate --dry-run -v remap 3 kp_7   # show the packets without hardware
```

`falcon --help` and `falcon <command> --help` list all options.

## Caution

- Every write is stored in the keypad's flash memory. The app avoids unnecessary writes, but
  don't script thousands of them.
- On macOS, 8-byte feature reports make the keypad stop responding until it is replugged, so
  the tool never sends them there. The `raw` and `experimental` commands are for protocol work
  only.
- Back up your layers before experimenting: `falcon dump 1 --out p1.bin` (repeat for 2–5).

## License

[MIT](LICENSE) © 2026 Raphmatt. This software is provided "as is", without warranty of any kind;
use it at your own risk.

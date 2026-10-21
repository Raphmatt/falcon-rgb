# MAX Falcon-20 RGB: USB HID protocol

This document describes the USB HID interface of the MAX Falcon-20 RGB keypad (USB `195D:6008`) as observed, for the
purpose of interoperability: it is what `falcon-rgb` implements to configure the keypad without the official Windows
software. It covers only the interface (packets, byte layouts, codes and timing). Facts checked on real hardware are
marked **(hardware-confirmed)**; everything marked **(unverified)** is expected behaviour that has not been tested on a
keypad yet. "MAX", "Falcon" and all other product and company names are trademarks of their respective owners and are
used here only to identify the hardware.

Section numbers are stable: the source code refers to them as "spec §n".

## 1. Device

| Item | Value |
|---|---|
| VID / PID | `0x195D` / `0x6008` (vendor: Itron Technology / iONE). Confirmed by USB enumeration |
| Interfaces | If0 boot keyboard, If1 mouse + consumer + vendor collections, If2 NKRO keyboard. IN endpoints only |
| Config collection | **Usage page `0xFF01`, usage `0x0001`**. Select it by usage page/usage, never by interface number |
| Event collections | Report ID 4 arrives on the vendor collections (hardware-confirmed). On macOS the vendor path is shared with the mouse collection, so report ID 1 mouse reports arrive there too. The `02 19 5D 60 08 kk` host-key report (§7) has not been observed yet |
| Transport | **Feature reports only** (hidapi `send_feature_report` / `get_feature_report`) |

macOS notes:

- Open devices non-exclusively (hidapi feature `macos-shared-device`), or the keypad stops typing.
- Open only the vendor collections, never the keyboard or keypad collections. macOS still requires the Input
  Monitoring permission, because the vendor collections share interface 1 with the mouse collection: without it,
  opening fails with `0xE00002E2` (not permitted).
- **Never send a feature report that is not 264 bytes** (hardware-confirmed). `IOHIDDeviceSetReport` with 8 bytes fails
  (`0xE0005000`), and afterwards the keypad answers no request at all (I/O timeout) until it is replugged. This rules out
  the 8-byte forms of `0x80`, polling rate and key mode on macOS.

WebHID notes (Chrome on macOS, hardware-confirmed):

- `navigator.hid.requestDevice` with the filter `{vendorId: 0x195D, productId: 0x6008, usagePage: 0xFF01, usage: 1}`
  grants five collections: `01:02` (mouse, no reports exposed), `0C:01` (input 3), `FF00:FF00` (input 4, the events and
  `04 1F` echoes), `FF00:01` (input 6) and `FF01:01` (feature 7). The keyboard collections are not exposed to pages.
- `sendFeatureReport(7, data)` takes the 263 bytes after the report ID. `receiveFeatureReport(7)` returns all 264 bytes
  **including** the report ID at `[0]`, the same layout as hidapi.
- 264-byte `0x81`, `0x82` and `0x01` work exactly as with hidapi. No macOS permission prompt appeared in the test
  (unverified whether Chrome already held Input Monitoring). The 8-byte rule above is assumed to apply too (same IOKit
  path), so only 264-byte reports are sent.

## 2. Packet format

```
byte:  0     1     2    3    4    5..7   8 .............. 263
      [07] [cmd] [a0] [a1] [a2] [00..]  payload, zero-filled
```

- Report ID is always `0x07`. Length is **264** bytes including the report ID (hardware-confirmed). Zero the whole
  buffer before filling it.
- No checksum, sequence number or framing.
- Read-back: SetFeature (request), wait, GetFeature (report 7, same length). The reply has the same layout.
- Delays used by the official Windows software: 20 ms after ordinary commands, 200 ms around `0x82`/`0x02`/`0x06`,
  50 ms between macro chunks and 200 ms after the last one. Also 3 s after connect before `0x80`, 100 ms before the
  `0x82` of a full download, and for a lighting-only update 100 ms between `0x82` and `0x02` plus 500 ms after.
  Implementations should keep these as configurable constants.
- `0x01` polling rate and `0x11` key mode exist only in an 8-byte form (unverified). A 264-byte `07 01 08` would mean
  "select profile 8". The 8-byte form cannot be sent on macOS (§1).
- The keypad echoes every command as input report `04 1F <cmd> 00 <value>` (§7, hardware-confirmed).

| Cmd | Len | Kind | Args | Meaning |
|---|---|---|---|---|
| `0x80` | 8 | Set + Get | none | Firmware version in `reply[2]`, `reply[3]` (the official software shows them as two concatenated decimal numbers) (unverified). The firmware does **not** implement a 264-byte form (hardware-confirmed): the Get returns the previous feature buffer with byte 1 cleared (`07 00 00 …` right after connect, `07 00 01 …` after a `0x81` that answered `07 81 01`). Treat a long reply as a version only if `reply[1] = 0x80`, otherwise "not available". The 8-byte form hangs the keypad on macOS |
| `0x81` | 264 | Set + Get | none | Current profile in `reply[2]`, **1-based**, 1..5 (hardware-confirmed: layer 1 → 1, layer 2 → 2) |
| `0x01` | 264 | Set | `a0 = profile` (1..5) | Select the active profile (layer) |
| `0x01` | 8 | Set | `a0 = 1 << n`, n = 0..3 → `01`/`02`/`04`/`08` | Polling rate (unverified). Overloads cmd `0x01`; experimental only |
| `0x11` | 8 | Set | `a0 = 0/1` | Key mode, probably 6KRO/NKRO (unverified). Experimental only |
| `0x82` | 264 | Set + Get | `a0 = profile` (1..5) | Read the profile block. Data = `reply[8..208]` (200 bytes used) |
| `0x02` | 264 | Set | `a0 = profile`, `buf[8..208] = block` | Write the profile block (§3) |
| `0x05` | 264 ×3 | Set | `a0 = profile`, `a1 = hw key index`, `a2 = chunk 1..3`, `buf[8..264] = 256 bytes` | Write one key's macro (§5) |
| `0x06` | 264 | Set | none | Sent last in a full download by the official software. Not needed for persistence (§9); its meaning is unknown (unverified) |

Full download for profile `p` (what the official software does on "Save"):

1. `0x82(p)`, patch the block, `0x02(p)`
2. `0x05(p, hw, 1..3)` for each key of macro type
3. `0x06`

A lighting-only update skips steps 2 and 3. Any profile can be written directly, because every command carries the
profile number.

## 3. Profile block (cmd 0x02 / 0x82, 200 bytes, offsets from `buf[8]`)

**Always read-modify-write**: start from the bytes returned by `0x82` and patch only the fields below. Unknown bytes
must round-trip unchanged.

| Offset | Size | Field |
|---|---|---|
| 0..24 | 25 | Key map: device code per **hw index** (§4). A full download writes all 25 bytes from the factory map first (slots 20..24 become `BB 00 BC B9 00`), then patches the 20 keys. A lighting-only update leaves 0..24 as read back. The factory blocks hold exactly the factory map (hardware-confirmed) |
| 25..49 | 25 | A second copy of the factory key map in all five factory blocks (hardware-confirmed); meaning unknown, preserve |
| 50..74 | 25 | Red plane, by **LED index** (hardware-confirmed) |
| 75..99 | 25 | Green plane (hardware-confirmed) |
| 100..124 | 25 | Blue plane (hardware-confirmed) |
| 125 | 1 | Lighting effect (§6) |
| 126 | 1 | Brightness: **0 = LEDs off**, 1..4 = dim to full (hardware-confirmed). The official software writes its UI level 1..5 minus one. Factory: 4, except profile 2 = 0 (off) |
| 127 | 1 | Wave direction: **0 = right→left, 1 = left→right, 2 = top→bottom, 3 = bottom→top** (hardware-confirmed). The official software only writes 1 ("left") and 2 ("right"). Factory: 1, except profile 3 = 0 |
| 128..130 | 3 | Effect colour R, G, B. Factory: P1 `FFFFFF`, P2 `0000FF`, P3 `FF0000`, P4 `00FF00`, P5 `FF00FF` |
| 131..133 | 3 | Constant `01 00 05`; always written, present in all factory blocks (hardware-confirmed) |
| 134..151 | 18 | Zeros in the factory blocks; preserve |
| 152..199 | 48 | Firmware RAM leaking into the reply (pointer-like values such as `54 09 00 10`); preserve |

## 4. Key and LED indexing

Button index **k = row*4 + col** (row 0..4 top to bottom, col 0..3 left to right). The official software's UI uses the
same order.

- **hw index** (key map, macro `a1`, events `kk`) = `col*5 + row`.
  k→hw: `0,5,10,15,1,6,11,16,2,7,12,17,3,8,13,18,4,9,14,19`
- **LED index** (colour planes) = `row*5 + col`.
  k→LED: `0,1,2,3,5,6,7,8,10,11,12,13,15,16,17,18,20,21,22,23`
- Light bars (colour slots 20..23): slot 20 left-upper → LED 9, slot 21 right-upper → LED 4, slot 22 left-lower → LED
  24, slot 23 right-lower → LED 19. LED 14 is never written.
- The LED index map, the light-bar LEDs and the R/G/B planes at 50/75/100 are confirmed visually on hardware.

Factory key map (block bytes 0..24, by hw index; hardware-confirmed):

```
hw:  0  1  2  3  4 | 5  6  7  8  9 |10 11 12 13 14 |15 16 17 18 19 |20 21 22 23 24
     53 5F 5C 59 62| B4 60 5D 5A BA| B5 61 5E 5B 63| 54 55 56 57 58| BB 00 BC B9 00
```

The same layout by button k (row-major):

| row | col 0 | col 1 | col 2 | col 3 |
|---|---|---|---|---|
| 0 | NumLock 53 | Layer1 B4 | Layer2 B5 | KP/ 54 |
| 1 | KP7 5F | KP8 60 | KP9 61 | KP* 55 |
| 2 | KP4 5C | KP5 5D | KP6 5E | KP− 56 |
| 3 | KP1 59 | KP2 5A | KP3 5B | KP+ 57 |
| 4 | KP0 62 | "00" BA | KP. 63 | KPEnter 58 |

## 5. Key assignments → device bytes

Kinds of key assignment and their key map byte:

- `Default`: the factory byte.
- `Disabled`: `0x00`.
- `Key(usage)`: a single HID usage, written as is, including the vendor codes below. `04` makes the key type `a`, and
  the remap persists across a replug (hardware-confirmed).
- `Macro { repeat, events }`: key map byte `0xF8` plus a macro upload (hardware-confirmed).
- `Combo(keys)`: a key plus modifiers. The official software writes key map byte `0xE8` plus a macro upload, but **`E8`
  does nothing on this firmware** (hardware-confirmed): pressing such a key only sends an empty mouse report
  `01 00 …`. Write `0xF8` and the full chord as an ordinary macro instead (below).
- `Function(fn)`: mapped through the table below. The meaning of most of these device bytes is unknown; use the table as
  observed.
- Host-only functions: launch a program (function `0x180`, device byte `0xF7`, see §8), plus the Windows shortcuts
  `0x1E1` (Show desktop), `0x1E3` (Zoom in) and `0x1E4` (Zoom out). The official software writes the shortcuts as `0xE8`
  with a synthesized chord; write `0xF8` with the same chord.

Function codes are the identifiers the official software uses for its key functions; `falcon-rgb` keeps the same
numbering. Function code → device byte, as written by the official software:

```
105→F7 106→F8 104→F9 121/122/123→C7 141→C4 142→C5 143→C6 161→C4 162→C5 163→C6 180→F7 DF→DF DB→DB
1A1→CE 1A2→CF 1A4→D5 1A3→D0 1A7→D2 1A8→D1 1A5→D3 1A6→D4
1C4→A7 1C7→A5 1C5→A6 1C6→A8 1C1→AB 1C2→AA 1C3→A9 1E2→DD
```

Precedence: apply the table first (first match); then `0x1D1` (single key) → the raw usage if non-zero (0 keeps the
factory byte); `0x200..0xFFFF` (recorded macro) → `F8`; `0x1D2` (combo), `0x1E1`, `0x1E3`, `0x1E4` → `E8` in the
official software, **`F8` in falcon-rgb** (see above); `0x1D3` (disable) → `00`; anything else keeps the factory byte.

Function code names:

- Profile: `0x104` Cycle, `0x105` Up, `0x106` Down.
- Mouse: `0x1A1`–`0x1AA` Left, Right, Middle, Double click, Back, Forward, Scroll up, Scroll down, Scroll left,
  Scroll right. `0x1A9`/`0x1AA` are not in the table.
- Media: `0x1C1`–`0x1C6` Vol up, Vol down, Mute, Play/Pause, Prev, Next.
- Windows: `0x1E1`–`0x1E4` Show desktop, Calculator (`1E2→DD`), Zoom in, Zoom out.

Vendor key codes usable as a single key (`0x1D1`): Layer 1–5 `B4–B8`, Layer cycle `B9`, "00" `BA`, LED brightness
`BB`, LED mode cycle `BC`, Lock all keys `FA`; media Stop `A5`, Prev `A6`, Play `A7`, Next `A8`, Mute `A9`, Vol− `AA`,
Vol+ `AB`, Player `A4`; mouse Left `BE`, Right `BF`, Middle `C0`, Back `C3`, Forward `C4`. Everything else is a standard
HID keyboard usage (`0x04`..`0xE7`).

### Macro block (722 bytes, sent as 3 × 256 bytes; chunk 3 is zero-padded)

| Offset | Content |
|---|---|
| 0 | Repeat, high byte |
| 1 | Repeat, low byte. `repeat` is a big-endian u16: N = play N times, `0xFFFF` = while held, `0` = until pressed again. Combos and shortcuts use `00 01` |
| 2.. | Up to **240** events × 3 bytes; the list ends at the first event with code 0 |

Event = `[b0, b1, code]`:

- `b0` bit 7 = key down (1) or up (0).
- `delay = ((b0 & 0x7F) << 8 | b1)` × 10 ms, range 1..0x7FFF. Never 0 (the official software replaces 0 with 1).
- Hardware-confirmed: repeat `00 01` with `down:lshift tap:b up:lshift` types `B`, and `tap:h tap:i` types `hi`
  (3 chunks of 256 bytes, events `{80|delay_hi, delay_lo, code}`).

Inside macros, `B7`/`B8`/`B9` = mouse left/right/middle, and `BC`/`BD`/`BE`/`BF` = mouse move Y+/Y−/X−/X+ (magnitude
encoding unknown, unverified; not offered in the editor yet).

Synthesized shortcuts (keys of the chord):

- `0x1E1` Show desktop: Win (`E3`) + `07` (D)
- `0x1E3` Zoom in: Ctrl (`E0`) + `C0` (unverified: zoom codes)
- `0x1E4` Zoom out: Ctrl (`E0`) + `C1` (unverified)

What the official software uploads for a combo: only the first two events, i.e. the presses of the first two keys with
delay 5 and no releases, with key map byte `E8`. Example Ctrl+C: `00 01 | 80 05 E0 | 80 05 06`. On hardware this types
nothing, and neither does `E8` with the full chord `80 05 E1 80 05 05 00 05 05 00 05 E1` (hardware-confirmed). It also
does not clear the 722-byte block between keys, so stale bytes can leak into later combos.

What falcon-rgb sends for combos and the three synthesized shortcuts: key map byte **`F8`** and an ordinary macro with
repeat `00 01` and the full chord, every event 10 ms apart (delay 1, like a recorded macro): all presses in order, then
the releases in reverse order. Ctrl+C = `00 01 | 80 01 E0 | 80 01 06 | 00 01 06 | 00 01 E0`. Any number of keys works
(up to 120, i.e. 240 events). A one-key combo is written as a single key (`0x1D1`). The macro block is zeroed for every
key.

## 6. Lighting

| Effect | Name | Uses |
|---|---|---|
| 0 | Normal (static) | colour, brightness (hardware-confirmed) |
| 1 | Breathing | colour, brightness (unverified) |
| 2 | Fade-in | colour, brightness (unverified) |
| 3 | Fade-out | colour, brightness (unverified) |
| 4 | Last keystroke stays lit | colour, brightness (unverified) |
| 5 | RGB wave | direction, brightness. A rainbow wave; the effect colour is ignored (hardware-confirmed) |
| 6 | RGB random single key | brightness (unverified) |
| 7 | Customize (per-key) | 20 key colours + 4 light-bar colours, brightness (hardware-confirmed) |

Effect values 8–10 may exist in firmware (unverified). The per-key colour planes are always written (the official
software always sends them).

Factory effect colours per profile: P1 white, P2 blue, P3 red, P4 green, P5 magenta (hardware-confirmed). Factory
per-key colours (Customize): k5 red, k8 and k10 blue, k9 yellow, k13 green, light bars black (hardware-confirmed).

## 7. Input reports from the device

Non-blocking reads on the vendor collections.

| Bytes | Meaning |
|---|---|
| `04 01 00 p 00 00` (`buf[5] == 0`) | Profile switched, `p` 1-based (hardware-confirmed: `04 01 00 01` after Layer 1, `04 01 00 02` after Layer 2). Only 1..5 have been observed; the official software treats 7 as profile 1 and ignores other values (unverified) |
| `04 01 01 m` | Lighting effect changed on the device to `m`; valid range 0..9 (unverified: not observed, no on-keypad key found) |
| `04 02 0C d` | Wave direction changed to `d` (unverified: not observed; the official software only accepts 0..1) |
| `04 01 07 s` | Brightness changed: brightness = s + 1, valid `s` 0..4 (unverified: not observed) |
| `04 1F c 00 v 00 00 00 00` | Echo of feature command `c` with value `v` (hardware-confirmed: `04 1F 81 00 02` after `0x81` on layer 2, `04 1F 81 00 01` on layer 1, `04 1F 01 00 00` after `0x01`). For `0x81` and `0x82` the value is the **active** layer, not the one read (`0x82(3)` on layer 1 → `04 1F 82 00 01`); for `0x01` it is always 0. Seen identically through hidapi and Chrome WebHID on macOS. Informational; the official software ignores it |
| `03 uu 00` (consumer collection `0C:01`) | A key mapped to a media function sends the consumer usage on press and `03 00` on release (hardware-confirmed: device byte `A7` Play/Pause → `03 CD`, `A8` Next → `03 B5`, `A6` Previous → `03 B6`). This is ordinary media-key input for the OS, not a configuration event; WebHID exposes it to pages, hidapi readers of the vendor collections never see it |
| `01 00 00 00 00 00 00 00` | Report ID 1 from the mouse collection (e.g. when a key mapped to `E8` is pressed). Ignore |
| `xx 02 19 5D 60 08 kk` | A host-function key was pressed (unverified: not observed yet). `kk` 0..19 = hw index; the host runs that key's function (launch program). Other `kk` values are device codes; after profile functions the official software re-reads `0x81` |

Input buffers have the collection's full input report length, zero-padded; pad short hidapi reads with zeros before
matching. Reading only the vendor collections (not the keyboard ones) is enough to receive the report-4 events
(hardware-confirmed).

The host application should update its UI state from these events.

## 8. Host-side features (no device support)

- Launch a program on a key press (device byte `F7`, then the host-key event `kk`). On macOS: `open -a` / `NSWorkspace`.
- Automatic profile switch by foreground application (5 applications per profile). On macOS: poll the frontmost
  application every 2 s.
- On-screen display and tray icon: optional. A menu-bar item is the macOS equivalent.

## 9. Persistence

On the device (hardware-confirmed):

- Every `0x02` is stored immediately. A lighting-only write without `0x06` survived a USB power cycle, so each write
  costs a flash cycle: debounce live edits and skip writes when nothing changed.
- Key remaps and macros persist across a replug.
- Writing a profile that is **not** the active one makes the keypad display that profile's lighting at once, while
  `0x81` still reports the old active layer. After a layer switch or replug the active layer's lighting is shown again.
  This can be used as a live preview.

On the host, falcon-rgb stores its own JSON (5 profiles plus global settings) in
`~/Library/Application Support/falcon-rgb/` on macOS, and in the platform's config directory elsewhere. Importing
profile files of the official Windows software is a possible future feature; their format is not documented here.

## 10. Open questions

Not yet confirmed on hardware:

- Effects 1–4 and 6, and whether effects 8–10 exist.
- Polling rate (`0x01`, 8-byte) and key mode (`0x11`, 8-byte). They cannot be tested on macOS, because 8-byte reports
  hang the keypad (§1).
- The firmware version query (`0x80`) in its 8-byte form.
- The meaning of `0x06`.
- The second key map copy at block bytes 25..49.
- The mouse-move encoding inside macros.
- The zoom shortcut codes (`C0`/`C1`).
- The effect, brightness and direction input events (no on-keypad keys for them were found) and the host-key report
  `02 19 5D 60 08 kk`.

Checklist used for the first hardware session, useful for re-testing other units or firmware versions:

1. `falcon list`: the collections of `195D:6008` with usage page/usage, and the feature report length.
2. `falcon version` with the 264-byte form only.
3. `falcon dump 1..5 --out pN.bin` (`0x82`): save the raw blocks and check the factory map and unknown bytes.
4. `falcon effect static --color ff0000`, then `brightness` and `wave --direction`: check bytes 126/127.
5. Per-key colours: light one key at a time to check the LED index map and the light bars.
6. `falcon listen`: press the Layer/LED keys and check that report 4 events arrive.
7. Remap one key (`0x1D1`), upload one macro, power-cycle and check that both persist.

## 11. Hardware findings

First hardware session: macOS, keypad on USB, `falcon` CLI. The raw `0x82` replies of the five factory blocks (264 bytes
each, `07 82 0N 00 …`) are in `crates/falcon-core/tests/data/`, checked by `crates/falcon-core/tests/hw_dumps.rs`.

| Topic | Result |
|---|---|
| Report length | 264 bytes work. An 8-byte SetFeature fails on macOS (`0xE0005000`), and the keypad then times out on every request until replugged |
| `0x80` version | The 264-byte form is not implemented: the Get returns the previous feature buffer with byte 1 cleared (`07 00 00 …` after connect, `07 00 01 …` after a profile query, which can be mistaken for firmware 1.0). No firmware version is available |
| `0x81` current profile | `reply[2]` is 1-based: 1 on layer 1 (also right after `0x01` select 1), 2 on layer 2 |
| Block layout | Key map 0..24 = factory map; 25..49 = a second copy of it (meaning unknown); constant `01 00 05` at 131..133; 152..199 = firmware RAM (`54 09 00 10` …) |
| Factory lighting | Effect 0 (static) in all profiles; colours P1 `FFFFFF`, P2 `0000FF`, P3 `FF0000`, P4 `00FF00`, P5 `FF00FF`; brightness byte 4, P2 0 (off); direction byte 1, P3 0; default per-key pattern (k5 red, k8/k10 blue, k9 yellow, k13 green), bars black |
| Brightness (126) | 0 = LEDs off, 1..4 = increasing, 4 = full |
| Direction (127) | 0 right→left, 1 left→right, 2 top→bottom, 3 bottom→top (watched on the keypad) |
| Effects | 0 static, 5 rainbow wave (colour ignored), 7 custom per-key colours |
| LEDs | Index `row*5+col`; light bars left-upper/right-upper/left-lower/right-lower at LEDs 9/4/24/19; planes R/G/B at 50/75/100 |
| Single-key remap | Key map byte = HID usage (`04` types `a`); persists across a replug |
| Macros (`F8`) | Upload format confirmed (repeat big-endian `00 01`, events `{80\|delay_hi, delay_lo, code}`, 3 chunks of 256). `down:lshift tap:b up:lshift` types `B`; `tap:h tap:i` types `hi` |
| Combos (`E8`) | Type nothing, both with the official software's presses-only upload `00 01 80 05 E1 80 05 05` and with the full chord. Pressing the key only sends mouse report `01 00 00 00 00 00 00 00`. Combos and shortcuts are therefore uploaded as `F8` macros (§5) |
| Persistence | A lighting-only `0x02` (no `0x06`) survived a USB power cycle |
| Preview | `0x02` to a non-active profile shows that profile's lighting at once; `0x81` still reports the old layer; after a replug the active layer is shown again |
| Events | `04 01 00 01` / `04 01 00 02` after pressing Layer 1 / Layer 2. Each command is echoed as `04 1F <cmd> 00 <value>` |

Consequences for the implementation: `0x81` is parsed 1-based; wave direction has four values; brightness byte 0 is
shown as "off"; combos and shortcuts are uploaded as `F8` macros with delay 1; command echoes and mouse reports are
decoded as their own input events; the transport refuses non-264-byte reports on macOS; GUI auto-apply is debounced and
skipped when nothing changed.

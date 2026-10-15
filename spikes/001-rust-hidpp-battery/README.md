# Spike 001 — read Logitech battery state directly from hardware over HID++ (raw hidraw)

**Question this spike answers:** can a Rust program read battery level and charging state
from Logitech devices with no Python, no Solaar, no `hidapi`, no HID++ crate — just
`open(2)`, `write(2)`, `poll(2)`, `read(2)` on `/dev/hidrawN`?

**Answer: yes.** See Verdict below.

```
battery/            # cargo project (binary name: battery)
  Cargo.toml        # dependencies: libc only
  src/main.rs       # CLI, device/route orchestration, table
  src/sysfs.rs      # hidraw enumeration from /sys/class/hidraw (no udev)
  src/hidpp.rs      # HID++ transport + protocol (framing, match loop, retry)
  src/battery.rs    # battery decoders + Solaar's mV -> % estimation table
  evidence/         # raw run outputs and Solaar's own wire trace
run.sh              # build + run
```

## Build / run

```bash
export PATH=$HOME/.cargo/bin:$PATH
cd spikes/001-rust-hidpp-battery
./run.sh                       # or: cargo build --target-dir target && ./target/debug/battery
./target/debug/battery --raw   # dump every request/reply frame to stderr
./target/debug/battery --help
```

Options: `--raw`, `--short` (7-byte reports), `--no-receiver`, `--no-device`, `--no-extra`,
`--timeout MS` (device/BT calls, default 4000), `--timeout-rx MS` (receiver calls, default 900),
`--attempts N` (default 2), `--max-index N` (receiver slot sweep, default 6).

---

## Verdict: VALIDATED

A ~1,100-line Rust program with **one dependency (`libc`, for `poll(2)`)** reads battery
percentage, millivolts and charging state straight off `/dev/hidraw*` over HID++, on
**both** access routes, and reproduces `solaar show` exactly:

| device | mine — receiver route | mine — device route | `solaar show` (1.1.20) |
|---|---|---|---|
| G703 LIGHTSPEED mouse | `/dev/hidraw6` idx `0x01`: **88 % / 4050 mV / DISCHARGING** | `/dev/hidraw8` idx `0x01`: **88 % / 4050 mV / DISCHARGING** | 88 % 4050 mV DISCHARGING |
| G915 TKL keyboard | `/dev/hidraw3` idx `0x01`: **95 % / 4122 mV / DISCHARGING** | `/dev/hidraw7` idx `0x01`: **95 % / 4122 mV / DISCHARGING** | 95 % 4122 mV DISCHARGING |
| G733 headset | (direct USB, no receiver) | no usable battery path — reported honestly, no invented numbers | `Battery: unknown (device is offline)` |

Exit status 0, whole run ~4–5 s (it sweeps 6 receiver slots on both receivers and tries a
HID++ 1.0 register fallback on the headset). A successful query costs 20–50 ms when the
device is awake, and up to ~1.5 s over the receiver when the wireless link has to wake a
sleeping device (the G915 TKL's receiver-route row measured 28 ms in one run, 1415 ms in
another — the numbers never varied, only the latency).

Raw evidence, verbatim:

* `evidence/final.out` / `evidence/final.err` — default run (table + first-attempt frames)
* `evidence/raw-proof.out` / `evidence/raw-proof.err` — `--raw --max-index 1`: every frame
* `evidence/solaar-show.txt` — `solaar show` for comparison
* `evidence/solaar-show-debug.log` — Solaar `-ddd show`, its own `w[…]`/`r[…]` wire trace
* `evidence/run1.*` — first working-ish attempt (register framing still wrong)
* `evidence/dev-run-first-working.*`, `evidence/dev-run-pre-cleanup.*` — after the transport fix
* `evidence/*.STALE-BINARY-MISBUILD` — kept for the record: a leaked `CARGO_TARGET_DIR`
  sent a rebuild into a sibling spike's target dir, so these two runs came from an old binary

---

## The table my program prints (verbatim `evidence/final.out`)

```
--- BATTERY TABLE ---
node            route     dev   name                              %     mV status               ms  source / notes
------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
/dev/hidraw3    receiver  0x01  G913 TKL                         95   4122 DISCHARGING          28  BATTERY_VOLTAGE {1001} idx 7 fn0 | BATTERY_STATUS {1000}: not supported | BATTERY_VOLTAGE {1001} idx 7 type 0x00 ver 3 | serial F99A783D | raw[10 1A 00] level-flag=exact
/dev/hidraw6    receiver  0x01  G703 LS                          88   4050 DISCHARGING          32  BATTERY_VOLTAGE {1001} idx 6 fn0 | BATTERY_STATUS {1000}: not supported | BATTERY_VOLTAGE {1001} idx 6 type 0x00 ver 2 | serial 348181A8 | raw[0F D2 00] level-flag=exact
/dev/hidraw0    device    0xff  Logitech G733 Gaming Headset      -      - -                  3624  no battery feature supported | BATTERY_STATUS {1000}: not supported | BATTERY_VOLTAGE {1001}: not supported | UNIFIED_BATTERY {1004}: not supported | ADC_MEASUREMENT {1F20} idx 8 type 0x00 ver 4 | ADC_MEASUREMENT {1F20} fn0 failed: HID++ 2.0 error 0x05 = NOT_ALLOWED (feature 0x08 fn 0x0) | register 0x07: no reply after 2 attempt(s) / 1803ms | register 0x07: HID++ 2.0 error 0x06 = UNSUPPORTED (feature 0x81 fn 0x0) | register 0x0d: no reply after 2 attempt(s) / 1802ms | register 0x0d: HID++ 2.0 error 0x06 = UNSUPPORTED (feature 0x81 fn 0x0)
/dev/hidraw7    device    0x01  Logitech G913 TKL                95   4122 DISCHARGING          29  BATTERY_VOLTAGE {1001} idx 7 fn0 | BATTERY_STATUS {1000}: not supported | BATTERY_VOLTAGE {1001} idx 7 type 0x00 ver 3 | sysfs parent node /dev/hidraw3 | receiver slot 1 from register 0x2B5 | raw[10 1A 00] level-flag=exact
/dev/hidraw8    device    0x01  Logitech G703 LS                 88   4050 DISCHARGING         984  BATTERY_VOLTAGE {1001} idx 6 fn0 | BATTERY_STATUS {1000}: not supported | BATTERY_VOLTAGE {1001} idx 6 type 0x00 ver 2 | sysfs parent node /dev/hidraw6 | receiver slot 1 from register 0x2B5 | raw[0F D2 00] level-flag=exact
```

(raw[10 1A 00] = 0x101A = 4122 mV, flags 0x00 = discharging; raw[0F D2 00] = 0x0FD2 = 4050 mV.
`level-flag=exact` = the flags byte has bit5 clear, i.e. the level is not an approximation.)

The raw frames behind those two rows (`evidence/raw-proof.err`, first attempt of each route):

```
# receiver route, G703 on /dev/hidraw6 with device index 0x01
[/dev/hidraw6] -> 11 01 00 0B 10 00 ...        ROOT.getFeature(0x1000 BATTERY_STATUS)
[/dev/hidraw6] <- 11 01 00 0B 00 00 ...        -> feature index 0 = not supported
[/dev/hidraw6] -> 11 01 00 0A 10 01 ...        ROOT.getFeature(0x1001 BATTERY_VOLTAGE)
[/dev/hidraw6] <- 11 01 00 0A 06 00 02 ...     -> feature index 6, type 0, version 2
[/dev/hidraw6] -> 11 01 06 09 00 00 ...        feature 6 function 0 = getBatteryVoltage
[/dev/hidraw6] <- 11 01 06 09 0F D2 00 ...     -> 0x0FD2 = 4050 mV, flags 0x00 = discharging

# receiver pairing register gave the name and serial for free
[/dev/hidraw6] -> 10 FF 83 B5 20 00 00        reg 0x2B5 sub 0x20 = slot 1 pairing info
[/dev/hidraw6] <- 11 FF 83 B5 20 07 08 40 86 0C 01 02 B3 ...   -> WPID 4086
[/dev/hidraw6] -> 10 FF 83 B5 40 00 00        sub 0x40 = device name
[/dev/hidraw6] <- 11 FF 83 B5 40 07 47 37 30 33 20 4C 53 ...   -> len 7, "G703 LS"
[/dev/hidraw6] -> 10 FF 83 B5 30 00 00        sub 0x30 = serial
[/dev/hidraw6] <- 11 FF 83 B5 30 34 81 81 A8 ...               -> 348181A8

# device route, G915 TKL on its own node /dev/hidraw7, index 0x01
[/dev/hidraw7] -> 11 01 00 0B 10 00 ...        ROOT.getFeature(0x1000)  -> index 0 (unsupported)
[/dev/hidraw7] -> 11 01 00 0A 10 01 ...        ROOT.getFeature(0x1001)  -> index 7, version 3
[/dev/hidraw7] -> 11 01 07 09 00 00 ...        feature 7 function 0
[/dev/hidraw7] <- 11 01 07 09 10 1A 00 ...     -> 0x101A = 4122 mV, flags 0x00
```

Solaar's own frames for the same device are identical in shape
(`evidence/solaar-show-debug.log`: `w[11 01 060B …]` -> `r[11 01 060B 0FD200…]`).

---

## What worked

* **sysfs enumeration, no udev.** `/sys/class/hidraw/*/device/uevent` gives `HID_ID`
  (bus:vid:pid) and `HID_NAME`; resolving the sysfs realpath identifies the USB interface
  and links a device node to the receiver that spawned it. Zero dependencies.
* **Interface selection.** Logitech receivers expose three hidraw nodes for the same USB
  device; the HID++ channel is the one on interface `:1.2` (`/dev/hidraw3`, `/dev/hidraw6`
  here). Verified by probing, not guessed — writes to the `:1.0`/`:1.1` nodes fail with
  `EPIPE`, which is a clean, unambiguous signal.
* **Both access routes work and agree.**
  * receiver route: open the receiver's control node, address the paired device by its
    receiver slot index (`0x01`) — 28–32 ms per query;
  * device route: open the device's own node (`/dev/hidraw8`, `/dev/hidraw7`), same index.
* **The HID++ 2.0 flow really is tiny**: `ROOT.getFeature(0x1001)` (2-byte big-endian
  feature id) returns the feature index; two more bytes of framing then answer
  `getBatteryVoltage` with `u16 BE mV + flags`.
* **Receiver register `0x2B5` is free metadata**: device name ("G703 LS"), serial and WPID
  per slot, no HID++ 2.0 needed. Auto-detecting the register style (Unifying/Lightspeed
  `0x2N` vs Bolt `0x5N`) is ~10 lines and made the code work on both receivers here.
* **Error paths are decodable, not opaque**: HID++ 2.0 error frames
  (`0xFF feature fn error_code`) and HID++ 1.0 error frames (`0x8F req_hi req_lo code`)
  both map onto meaningful names (`NOT_ALLOWED`, `UNSUPPORTED`, …).
* **Offline devices are handled honestly** — the G733 headset row prints no numbers and the
  reasons why each path failed; exit status stays 0. No invented values anywhere.

## What didn't work / what bit me (the important part)

Each of these cost real debugging time; all were resolved by diffing against Solaar's own
wire trace (`solaar -ddd show`) rather than by guessing.

1. **Register request id: `0x83B5`, not `0x81B5`.** A "2" in front of a register number marks
   it a *long* register, so `request_id = 0x8100 | (reg & 0x2FF)` turns `0x2B5` into `0x83B5`.
   Sending `0x81B5` draws HID++ 1.0 error `0x01 INVALID_SUB_ID_COMMAND`. Symptom before the
   fix: pairing info simply never arrived.
2. **Receiver register reads must use SHORT (7-byte `0x10`) reports.** Solaar only raises the
   long-report flag for HID++ 2.0 devices, so its receiver register reads go out as short
   reports. Both receivers here answer only that form; long register reads time out.
3. **Device nodes answer under their receiver slot index, not `0xFF`.** Requesting on
   `/dev/hidraw7` with index `0xFF` gets a reply stamped `dev = 0x01`, and Solaar likewise
   always sends a paired device's own number even when it talks to the device node. A matcher
   that accepts only `index` or `index ^ 0xFF` (Solaar's rule) silently discards every reply —
   literally every row read "-" until this was fixed. (Direct-USB devices such as the G733
   *do* answer `0xFF` on their own node, so both conventions must be tolerated.)
4. **Timeouts and software-id discipline are the real protocol risk, not the byte layout.**
   With a 300 ms window the receiver→wireless round trip (~300–600 ms for the sleeping G915)
   blew the budget, and the late reply was then matched to the *next* request: the
   `getFeature(0x1000)` answer was credited to `getFeature(0x1001)`, so the keyboard was
   reported as "BATTERY_VOLTAGE not supported" while its voltage was read through a
   misattributed `ADC_MEASUREMENT` row. The 2-byte request/reply match (`feature | fn|swid`)
   cannot tell two `getFeature` calls apart, so this fails silently and *plausibly*.
   Fixes, both adopted here:
   * Solaar's timeouts — 4 s for device calls, 0.9 s for calls to the receiver itself;
   * a **rotating software id** per feature call (0x0B, 0x0A, 0x09, …), so a stale reply from
     a previous request can never match (this is a deliberate improvement over Solaar's fixed
     `0x0B`; it costs ~5 lines).
   Symptom to remember: numbers that are right but attributed to the wrong feature.
5. **Devices report millivolts, not a percentage.** Neither the G703 nor the G915 has
   `BATTERY_STATUS {1000}` (`getFeature` returns index 0). The percentage Solaar prints comes
   from a piecewise-linear mV→% table (`estimate_battery_level_percentage`, 13 points,
   4186 mV = 100 % … 3500 mV = 0 %). A rewrite must port that table or it will show a
   different percentage than Solaar for the same device (4050 mV → 88 %, 4122 mV → 95 %; both
   verified here).
6. **The G733 headset has no readable battery on this firmware**: `{1000}`, `{1001}`,
   `{1004}` all unsupported, `ADC_MEASUREMENT {1F20}` exists at index 8 but returns HID++ 2.0
   error `0x05 NOT_ALLOWED`, and registers `0x07`/`0x0D` are unsupported. This matches
   Solaar's `Battery: unknown (device is offline)`. Correct behaviour is to say so.
7. Open files are closed immediately after each query (`Hidpp` is dropped per device/route) —
   devices are never held open, which matters because a second HID++ owner can conflict with
   Solaar. Solaar was not running during these runs.
8. **A sleeping device answers nothing — and both tools agree about it.** In a later run the
   G915 TKL had gone to sleep, and every call (both routes) came back with HID++ 1.0 error
   `0x08 UNKNOWN_DEVICE` in ~20 ms — the *receiver* was awake and answering, the keyboard was
   not. My program printed no numbers for it, and `solaar show` at the same moment also no
   longer listed the keyboard at all. The 95 % / 4122 mV row above therefore needs the device
   to be awake and paired; the numbers never disagreed between the two tools, only
   `UNKNOWN_DEVICE` vs. a value. Treat `0x08` as "device asleep/unreachable", not as a bug.

## How much protocol code does a rewrite actually need?

Measured on this spike (non-comment source lines):

| piece | lines | notes |
|---|---|---|
| HID++ transport + protocol (`src/hidpp.rs`) | 300 | framing/padding, `poll`+read loop, reply matching, retry, error decoding, `getFeature`, register reads |
| battery decode + mV→% table (`src/battery.rs`) | 228 | 2.0 status/voltage, 1.0 registers, unified battery, ADC, the 13-point table |
| sysfs/udev-free enumeration (`src/sysfs.rs`) | 123 | uevent parsing, interface detection, device↔receiver linking |
| CLI + orchestration + table (`src/main.rs`) | 461 | both routes, receiver slot sweep, pairing register parsing, reporting |
| **total** | **~1,100** | one dependency: `libc` |

Practical reading for the rewrite:

* **The protocol itself is ~300 lines and is not the risk.** Byte layouts are small and were
  reproduced from scratch. Owning them (rather than depending on a HID++ crate) is clearly
  feasible; ~500 lines covers enumeration + both routes + battery + voltage/status + naming.
* **Budget the effort for the operational details, not the wire format**: the timeout and
  software-id discipline (finding 4), the short/long register framing (1, 2), the device-index
  convention (3) and the mV→% table (5). Those four are where correctness silently diverges.
* Recommended split in a real rewrite: a `hidpp` transport module (framing + synchronous
  request/reply with rotating sw-id and per-target timeouts) plus a thin per-feature layer;
  the receiver-register and HID++ 1.0 paths can stay small and best-effort, since every device
  tested only needs the 2.0 feature flow.
* `hidapi`-style abstractions are unnecessary on Linux: `File` + `libc::poll` is enough, and
  the sysfs path already tells you which node is the HID++ channel.
* Not covered by this spike (untouched surface, still to prove): notifications/async events,
  pairing, settings writes (DPI, RGB, onboard profiles), Bolt encryption/host switching,
  Centurion devices, and non-Linux backends.

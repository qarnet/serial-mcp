# UF2 Bootloader + 1200-Baud Touch Plan

## Background: How the 1200-Baud Touch Works

From the Adafruit TinyUSB source (`Adafruit_USBD_CDC.cpp`):

```c
void tud_cdc_line_state_cb(uint8_t instance, bool dtr, bool rts) {
    if (!dtr) {
        if (instance == 0) {
            cdc_line_coding_t coding;
            tud_cdc_get_line_coding(&coding);
            if (coding.bit_rate == 1200) {
                TinyUSB_Port_EnterDFU();  // writes GPREGRET=0x57, resets
            }
        }
    }
}
```

Host-side sequence:

1. Open USB CDC port at **1200 baud**
2. Assert **DTR** (high)
3. De-assert **DTR** (low) — this is the "touch"
4. Firmware writes `0x57` to `NRF_POWER->GPREGRET` and resets
5. Bootloader reads GPREGRET on boot → enters **UF2 mode**
6. A USB mass storage drive (`XIAO BLE`) appears for drag-and-drop flashing

## Key Constraint

The 1200-baud touch is handled by the **application firmware**, not the
bootloader. The bootloader only checks GPREGRET on startup. Both are needed:

- Adafruit UF2 bootloader (+ SoftDevice s140)
- Application firmware with **USB CDC** implementing the touch handler

## Bootloader Files

From `0hotpotman0/BLE_52840_Core/bootloader/Seeed_XIAO_nRF52840/`:

| File | Purpose |
|------|---------|
| `Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex` | Full bootloader + SoftDevice for SWD flash |
| `Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.zip` | DFU package |
| `update-Seeed_XIAO_nRF52840_bootloader-0.6.1_nosd.uf2` | Bootloader update via existing UF2 bootloader |

## What Changes from Current Setup

| | Current | With UF2 Bootloader |
|---|---|---|
| Bootloader | None (bare metal at 0x0) | Adafruit UF2 + SoftDevice s140 |
| App start addr | `0x0` | `0x27000` (after SoftDevice) |
| USB | Disabled | USB CDC-ACM (native USB) |
| Host port | `/dev/ttyACM0` (PicoProbe) | `/dev/ttyACMx` (native USB) + PicoProbe |
| Flashing | `pyocd` at 0x0 | Drag-drop UF2 or `pyocd` |
| `pm_static.yml` | Forces addr 0x0 | Needs addr 0x27000 or removed |
| `CONFIG_BUILD_OUTPUT_UF2` | `n` | `y` |

## Step-by-Step Plan

### Phase 1: Flash the Adafruit UF2 Bootloader

1. Download `Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex`
2. Full-chip erase + flash via pyocd:
   ```bash
   pyocd erase -t nrf52840 --chip
   pyocd flash -t nrf52840 Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex
   ```
3. Verify: double-tap reset button → USB mass storage drive should appear

### Phase 2: Modify Firmware for USB CDC + 1200-Baud Touch

The firmware needs a **second** serial interface (USB CDC) alongside the
existing physical UART.

**Option A — Zephyr USB CDC-ACM (recommended)**

- Add `CONFIG_USB_DEVICE_STACK=y`, `CONFIG_UART_LINE_CTRL=y`,
  `CONFIG_USB_CDC_ACM=y`
- Add a devicetree overlay enabling `zephyr_cdc_acm_uart`
- Implement a custom handler watching for `LINE_CTRL_BAUD_RATE=1200` + DTR
  toggle
- On trigger: write `NRF_POWER->GPREGRET = 0x57; NVIC_SystemReset();`
- Physical UART (`uart0`) stays as-is for test commands via PicoProbe
- USB CDC provides the 1200-baud touch entry point only

**Option B — Arduino sketch (simpler but separate)**

- Write a minimal Arduino sketch using Adafruit nRF52 BSP
- Gets the touch handler for free via TinyUSB
- Loses Zephyr test firmware commands
- Only useful for testing bootloader entry, not full validation

**Recommendation: Option A** — keeps everything in one firmware, tests both
UART commands and bootloader entry.

### Phase 3: Build Configuration Changes

- Update `firmware/pm_static.yml`: set app start at `0x27000` (or remove and
  let partition manager handle it)
- Set `CONFIG_BUILD_OUTPUT_UF2=y` to produce `.uf2` output
- Set `CONFIG_USE_DT_CODE_PARTITION=y` so linker respects bootloader partition
- Set `CONFIG_BOOTLOADER_MCUBOOT=n` (using Adafruit, not MCUboot)

### Phase 4: serial-mcp Test for Bootloader Entry

New test using serial-mcp tools:

```
1. list_ports            → find XIAO's native USB CDC port
2. open port at 1200 baud
3. set_dtr_rts dtr=true  rts=false
4. set_dtr_rts dtr=false rts=false   ← triggers bootloader
5. Verify: port disappears, USB mass storage appears
6. (Optional) copy UF2 file to mass storage → firmware re-flashes
```

This exercises `serial-mcp`'s `open` (at non-standard baud) and `set_dtr_rts`
in a real-world bootloader-entry workflow.

### Phase 5: Recovery Path

If anything breaks:

- PicoProbe + pyocd can always recover (SWD access is independent of
  USB/bootloader state)
- Re-flash bootloader hex via `pyocd flash`
- Or erase chip and go back to bare-metal firmware at 0x0

## Risks / Open Questions

1. **Two USB CDC ports on host** — XIAO native USB + PicoProbe both present as
   `/dev/ttyACMx`. Need to identify which is which (by USB VID/PID or serial
   number).

2. **SoftDevice + Zephyr compatibility** — The Adafruit bootloader bundles
   SoftDevice s140 7.3.0, but NCS/Zephyr uses its own BLE stack (SoftDevice
   Controller or Zephyr BLE). The SoftDevice in the bootloader is only used by
   the bootloader for BLE DFU; the application can use Zephyr's stack. Need to
   confirm no conflict.

3. **Flash partitioning** — Getting `pm_static.yml` right for the new layout is
   critical. Wrong offsets = bricked app (recoverable via pyocd).

4. **AGENTS.md constraint** — Current rules say "Do NOT add USB CDC-ACM." This
   plan intentionally changes that for the bootloader-entry feature. Physical
   UART for test commands stays unchanged.

## Deliverables

1. Downloaded bootloader hex, flashed to XIAO
2. Modified `firmware/prj.conf` + overlay for USB CDC
3. New GPREGRET reset handler in firmware (`command.c` or new file)
4. Updated `firmware/pm_static.yml` for bootloader-aware partitioning
5. `cargo test` for the 1200-baud touch sequence in `xiao_ble_validation.rs`
6. Updated `firmware/AGENTS.md` documenting dual-boot setup

## GPREGRET Magic Values (Reference)

From Adafruit bootloader `main.c`:

| Magic | Value | Meaning |
|-------|-------|---------|
| `DFU_MAGIC_OTA_APPJUM` | `0xB1` | BLE DFU, SD already inited |
| `DFU_MAGIC_OTA_RESET` | `0xA8` | BLE DFU via soft reset |
| `DFU_MAGIC_SERIAL_ONLY_RESET` | `0x4E` | CDC only (no MSC) |
| `DFU_MAGIC_UF2_RESET` | `0x57` | CDC + MSC (UF2 mode) |
| `DFU_MAGIC_SKIP` | `0x6D` | Skip DFU entirely |

The 1200-baud touch writes `0x57` (UF2 mode with mass storage).

## Status: PAUSED

**User paused execution. Resumable from here.**

### Completed

- [x] Plan written (`firmware/UF2_BOOTLOADER_PLAN.md`)
- [x] Bootloader hex downloaded:
  `firmware/bootloader/Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex`
  (520956 bytes)
- [x] `firmware/AGENTS.md` updated to reflect the new UF2-bootloader
  approach (old "Do Not Drift" USB CDC / 0x27000 rules removed per
  explicit user permission)

### Blocked — Hardware not ready

**Blocker 1: SWD connection dead**

`pyocd` times out on every SWD connect attempt. Reproduced with:

```bash
pyocd commander -t nrf52840 -c "show"
# 0010413 C Error: [Errno 110] Operation timed out [__main__]

pyocd commander -t nrf52840 -O "connect_mode=under-reset" -c "show"
# 0010410 C Error: [Errno 110] Operation timed out [__main__]

pyocd commander -t nrf52840 -O "frequency=1000000" -c "show"
# 0010413 C Error: [Errno 110] Operation timed out [__main__]
```

Same for `pyocd reset`, `pyocd load`, `pyocd erase`. Probe itself
enumerates fine — issue is between PicoProbe SWD and the XIAO.

**Blocker 2: XIAO native USB not connected to host**

```text
$ lsusb
Bus 001 Device 006: ID 0b05:1bef Realtek Bluetooth Controller
Bus 001 Device 007: ID 2e8a:000c Raspberry Pi Debugprobe on Pico (CMSIS-DAP)
```

No Seeed (`2886:*`) or Adafruit (`239a:*`) device. Only the PicoProbe
shows up. The 1200-baud touch is a USB CDC mechanism, so the XIAO's
USB-C must be physically wired to the host.

### Recovery steps before resuming Phase 1

1. **Connect XIAO native USB to host** — plug XIAO's USB-C into the
   host. After that, `lsusb` should show `2886:002a` (UF2 bootloader
   PID) or `2886:000a` (serial-only). The 1200-baud touch requires
   this.
2. **Recover SWD** — the XIAO needs a reset / power cycle. Try one of:
   - Unplug PicoProbe USB from host, wait 5s, plug back in
   - Press the XIAO's reset button while pyocd is attempting attach
   - Pull XIAO's RESET pin low briefly (if a header pin is available)
3. **Confirm pyocd connect** — before any erase, verify:
   ```bash
   pyocd commander -t nrf52840 -c "show"
   # Should show: 0010XXX I nRF52840 ...
   # No "Operation timed out"
   ```
4. **User must confirm `pyocd erase --chip`** — the current app
   firmware at 0x0 will be destroyed. The user already said the
   current AGENTS.md "Do Not Drift" rules can be discarded, but the
   destructive erase itself should be a separate confirmation.

### Not started

- [ ] Phase 1: Flash bootloader hex (blocked on SWD)
- [ ] Phase 2: Add USB CDC + 1200-baud touch to firmware
- [ ] Phase 3: Update build config (prj.conf, overlay, pm_static.yml)
- [ ] Phase 4: Add serial-mcp test for 1200-baud touch
- [ ] Phase 5: Build firmware + verify

### Decision points deferred

- **Option A vs B** (Zephyr CDC-ACM vs Arduino sketch) — recommendation
  is Option A. Not implemented yet; can be made once the user is ready
  to resume.
- **pm_static.yml layout** — committed to `0x27000` in the plan, not
  yet applied. The board's stock `zephyr/boards/seeed/xiao_ble/pm_static.yml`
  uses `0x27000` for the app, so the override may be redundant once
  `CONFIG_USE_DT_CODE_PARTITION=y` is set. Needs a clean build to
  confirm.

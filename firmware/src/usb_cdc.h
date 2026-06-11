/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * USB CDC-ACM support for the 1200-baud touch → bootloader entry flow
 * on the native_sim POSIX emulator.
 *
 * The CDC-ACM device is enabled by setting `CONFIG_USB_DEVICE_STACK=y`
 * in `boards/native_sim_usb.conf`. The companion CDC-ACM node is
 * declared in `boards/native_sim_usb.overlay`.
 */
#ifndef USB_CDC_H_
#define USB_CDC_H_

#include <zephyr/kernel.h>

/*
 * Initialize the USB device stack and register a CDC-ACM UART.
 *
 * The driver samples DTR every 50 ms and watches for the 1200-baud
 * touch sequence:
 *   1. Host opens CDC port at 1200 baud
 *   2. Host sets DTR (asserted)
 *   3. Host clears DTR (de-asserted) — the "touch"
 * When detected, the firmware writes `sim_gpregret = 0x57` and
 * calls `exit(42)` so the test process can verify the magic exit code.
 *
 * Returns 0 on success, negative errno on failure.
 */
int usb_cdc_init(void);

#endif /* USB_CDC_H_ */

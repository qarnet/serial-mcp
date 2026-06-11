/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * USB CDC-ACM support for the 1200-baud touch → bootloader entry flow.
 *
 * Compiled only when CONFIG_USB_DEVICE_STACK_NEXT=y (set by board
 * fragment boards/<board>_usb.conf). Without it, this header provides
 * stub functions returning -ENODEV.
 */
#ifndef USB_CDC_H_
#define USB_CDC_H_

#include <zephyr/kernel.h>

/*
 * Initialize the USB device stack and register a callback for CDC
 * ACM line state + line coding events.
 *
 * The callback watches for the 1200-baud touch sequence:
 *   1. Host opens CDC port at 1200 baud
 *   2. Host sets DTR (asserted)
 *   3. Host clears DTR (de-asserted) — the "touch"
 * When detected, calls do_bootloader_entry() which:
 *   - On xiao_ble: writes NRF_POWER->GPREGRET = 0x57, then NVIC_SystemReset()
 *   - On native_sim: writes sim_gpregret = 0x57, then exit(42)
 *
 * Returns 0 on success, negative errno on failure.
 */
int usb_cdc_init(void);

#endif /* USB_CDC_H_ */

/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * USB CDC-ACM device stack initialization + 1200-baud touch handler.
 *
 * Built for the native_sim POSIX emulator. Uses the legacy Zephyr
 * USB device stack because that is the only stack with
 * USB_NATIVE_POSIX / USB/IP support on native_sim.
 *
 * When the host opens the CDC-ACM port at 1200 baud and pulses DTR
 * (assert → de-assert), this module triggers bootloader entry:
 *   - writes 0x57 to a global, then exit(42) so the test
 *     process can verify the magic exit code.
 */

#include "usb_cdc.h"

#include <errno.h>
#include <stdlib.h>
#include <zephyr/logging/log.h>
#include <zephyr/devicetree.h>

LOG_MODULE_REGISTER(usb_cdc, LOG_LEVEL_INF);

#define GPREGRET_BOOTLOADER_MAGIC 0x57
#define TOUCH_BAUD_RATE 1200

/* Visible to tests: a global that the native_sim bootloader-entry
 * handler writes to. Tests can verify the magic value.
 *
 * Always defined so `usb_cdc_init()` callers (and any test code that
 * peeks at it) can link cleanly against the plain `native_sim` build.
 */
volatile uint8_t sim_gpregret;

#if defined(CONFIG_USB_DEVICE_STACK) && defined(CONFIG_USB_CDC_ACM) && DT_NODE_EXISTS(DT_NODELABEL(cdc_acm_0))

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/drivers/uart/cdc_acm.h>
#include <zephyr/usb/usb_device.h>

static inline void do_bootloader_entry(void)
{
	sim_gpregret = GPREGRET_BOOTLOADER_MAGIC;
	LOG_INF("Bootloader entry: sim_gpregret=0x57, exit(42)");
	exit(42);
}

/* CDC-ACM UART node from our overlay (cdc_acm_0 on zephyr_udc0). */
#define CDC_DEV DEVICE_DT_GET(DT_NODELABEL(cdc_acm_0))

static uint32_t current_baud = 115200;
static bool dtr_prev_state;
static struct k_work_delayable dtr_poll_work;

/* Periodic DTR poll — the legacy CDC-ACM driver does not fire a callback
 * on DTR changes, so we sample uart_line_ctrl_get(UART_LINE_CTRL_DTR)
 * every 50 ms.
 */
static void dtr_poll_fn(struct k_work *work)
{
	uint32_t dtr = 0;

	if (uart_line_ctrl_get(CDC_DEV, UART_LINE_CTRL_DTR, &dtr) == 0) {
		bool dtr_high = (dtr != 0);

		if (dtr_prev_state && !dtr_high &&
		    current_baud == TOUCH_BAUD_RATE) {
			LOG_INF("1200-baud touch detected");
			do_bootloader_entry();
		}
		dtr_prev_state = dtr_high;
	}

	k_work_schedule(&dtr_poll_work, K_MSEC(50));
}

/* Baud rate change callback (CDC_ACM_DTE_RATE_CALLBACK_SUPPORT). */
static void baud_rate_cb(const struct device *dev, uint32_t rate)
{
	LOG_INF("CDC baud rate changed to %u", rate);
	current_baud = rate;
}

int usb_cdc_init(void)
{
	int err;

	if (!device_is_ready(CDC_DEV)) {
		LOG_ERR("CDC ACM device not ready");
		return -ENODEV;
	}

	cdc_acm_dte_rate_callback_set(CDC_DEV, baud_rate_cb);

	k_work_init_delayable(&dtr_poll_work, dtr_poll_fn);
	k_work_schedule(&dtr_poll_work, K_MSEC(100));

	/* Enable the USB device stack.  This triggers usb_dc_attach() on
	 * the native_posix controller, which starts the USB/IP server
	 * on port 3240.
	 */
	err = usb_enable(NULL);
	if (err) {
		LOG_ERR("Failed to enable USB device support: %d", err);
		return err;
	}

	LOG_INF("USB CDC-ACM ready");
	return 0;
}

#else /* !(CONFIG_USB_DEVICE_STACK && CONFIG_USB_CDC_ACM && cdc_acm_0 DT node) */

/* Plain `native_sim` build: no USB CDC support compiled in.
 * `usb_cdc_init()` returns -ENODEV so callers (see `main.c`) can
 * treat "USB not built" as a non-fatal condition.
 */
int usb_cdc_init(void)
{
	return -ENODEV;
}

#endif /* CONFIG_USB_DEVICE_STACK && CONFIG_USB_CDC_ACM && DT_NODE_EXISTS(cdc_acm_0) */

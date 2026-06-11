/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * USB CDC-ACM device stack initialization + 1200-baud touch handler.
 *
 * Supports two USB stacks depending on Kconfig:
 *   - CONFIG_USB_DEVICE_STACK_NEXT  → device-next API (xiao_ble)
 *   - CONFIG_USB_DEVICE_STACK       → legacy API (native_sim; the only
 *     stack that has USB_NATIVE_POSIX / USB/IP support on native_sim)
 *
 * When the host opens the CDC-ACM port at 1200 baud and pulses DTR
 * (assert → de-assert), this module triggers bootloader entry:
 *   - xiao_ble: writes NRF_POWER->GPREGRET = 0x57, then NVIC_SystemReset()
 *   - native_sim: writes 0x57 to a global, then exit(42) so the test
 *                process can verify the magic exit code.
 */

#include "usb_cdc.h"

#include <zephyr/logging/log.h>
#include <stdlib.h>
#include <errno.h>

LOG_MODULE_REGISTER(usb_cdc, LOG_LEVEL_INF);

#define GPREGRET_BOOTLOADER_MAGIC 0x57
#define TOUCH_BAUD_RATE 1200

/* Visible to tests: a global that the native_sim bootloader-entry
 * handler writes to. Tests can verify the magic value.
 */
volatile uint8_t sim_gpregret;

/* ── Shared: bootloader entry (target-specific) ─────────────────────── */

static inline void do_bootloader_entry(void)
{
#if defined(CONFIG_BOARD_NATIVE_SIM) || defined(CONFIG_BOARD_NATIVE_SIM_64) || \
	defined(CONFIG_SOC_NATIVE_SIM)
	sim_gpregret = GPREGRET_BOOTLOADER_MAGIC;
	LOG_INF("Bootloader entry: sim_gpregret=0x57, exit(42)");
	exit(42);
#else
	/* xiao_ble / nRF52840: write GPREGRET and reset. */
	volatile uint32_t *gpregret = (volatile uint32_t *)0x4000051CUL;
	*gpregret = GPREGRET_BOOTLOADER_MAGIC;
	LOG_INF("Bootloader entry: GPREGRET=0x57, NVIC reset");
	volatile uint32_t *aircr = (volatile uint32_t *)0xE000ED0CUL;
	*aircr = 0x05FA0004;
	for (;;) {
		__asm__ volatile("wfi");
	}
#endif
}

/* ── Device-next stack (CONFIG_USB_DEVICE_STACK_NEXT) ──────────────── */

#ifdef CONFIG_USB_DEVICE_STACK_NEXT

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/usb/usbd.h>
#include <zephyr/usb/usbd_msg.h>

static struct usbd_context *usbd;

static void usb_msg_cb(struct usbd_context *const ctx, const struct usbd_msg *const msg)
{
	if (msg->type == USBD_MSG_CDC_ACM_LINE_CODING) {
		uint32_t baud = 0;
		int ret = uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_BAUD_RATE, &baud);
		if (ret == 0) {
			LOG_INF("CDC line coding: baud=%u", baud);
		}
	}

	if (msg->type == USBD_MSG_CDC_ACM_CONTROL_LINE_STATE) {
		uint32_t dtr = 0;
		uint32_t baud = 0;

		uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_DTR, &dtr);
		uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_BAUD_RATE, &baud);

		LOG_INF("CDC control line state: DTR=%u baud=%u", dtr, baud);

		static bool prev_dtr;
		if (prev_dtr && !dtr && baud == TOUCH_BAUD_RATE) {
			LOG_INF("1200-baud touch detected");
			do_bootloader_entry();
		}
		prev_dtr = (bool)dtr;
	}
}

USBD_DEVICE_DEFINE(usb_cdc_usbd,
		   DEVICE_DT_GET(DT_NODELABEL(zephyr_udc0)),
		   0x2fe3, 0x0001);

USBD_DESC_LANG_DEFINE(usb_cdc_lang);
USBD_DESC_MANUFACTURER_DEFINE(usb_cdc_mfr, "serial-mcp");
USBD_DESC_PRODUCT_DEFINE(usb_cdc_product, "serial-mcp test FW");

USBD_DESC_CONFIG_DEFINE(usb_cdc_fs_cfg_desc, "FS Config");

static const uint8_t attributes = USB_SCD_SELF_POWERED;

USBD_CONFIGURATION_DEFINE(usb_cdc_fs_config,
			  attributes,
			  100, &usb_cdc_fs_cfg_desc);

static const char *const blocklist[] = {
	NULL,
};

int usb_cdc_init(void)
{
	int err;

	err = usbd_add_configuration(&usb_cdc_usbd, USBD_SPEED_FS,
				     &usb_cdc_fs_config);
	if (err) {
		LOG_ERR("Failed to add FS configuration: %d", err);
		return err;
	}

	if (IS_ENABLED(CONFIG_USBD_CDC_ACM_CLASS)) {
		err = usbd_register_all_classes(&usb_cdc_usbd, USBD_SPEED_FS,
						1, blocklist);
		if (err) {
			LOG_ERR("Failed to register CDC ACM class: %d", err);
			return err;
		}
	}

	err = usbd_msg_register_cb(&usb_cdc_usbd, usb_msg_cb);
	if (err) {
		LOG_ERR("Failed to register message callback: %d", err);
		return err;
	}

	err = usbd_init(&usb_cdc_usbd);
	if (err) {
		LOG_ERR("Failed to initialize USB device support: %d", err);
		return err;
	}

	if (!usbd_can_detect_vbus(&usb_cdc_usbd)) {
		err = usbd_enable(&usb_cdc_usbd);
		if (err) {
			LOG_ERR("Failed to enable USB device support: %d", err);
			return err;
		}
	}

	usbd = &usb_cdc_usbd;
	LOG_INF("USB CDC-ACM device initialized (device-next)");
	return 0;
}

/* ── Legacy stack (CONFIG_USB_DEVICE_STACK) ────────────────────────── */

#elif defined(CONFIG_USB_DEVICE_STACK)

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/drivers/uart/cdc_acm.h>
#include <zephyr/usb/usb_device.h>

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
			LOG_INF("1200-baud touch detected (legacy)");
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

	LOG_INF("USB CDC-ACM ready (legacy stack)");
	return 0;
}

/* ── No USB configured ─────────────────────────────────────────────── */

#else

int usb_cdc_init(void)
{
	return -ENODEV;
}

#endif /* CONFIG_USB_DEVICE_STACK_NEXT / CONFIG_USB_DEVICE_STACK */

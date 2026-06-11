/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * USB CDC-ACM device stack initialization + 1200-baud touch handler.
 *
 * When the host opens the CDC-ACM port at 1200 baud and pulses DTR
 * (assert → de-assert), this module triggers bootloader entry:
 *   - xiao_ble: writes NRF_POWER->GPREGRET = 0x57, then NVIC_SystemReset()
 *   - native_sim: writes 0x57 to a global, then exit(42) so the test
 *                process can verify the magic exit code.
 *
 * The whole file is conditional on CONFIG_USB_DEVICE_STACK_NEXT which
 * is set by boards/<board>_usb.conf. Without that, a stub
 * usb_cdc_init() returns -ENODEV.
 */

#include "usb_cdc.h"

#include <zephyr/logging/log.h>
#include <stdlib.h>
#include <errno.h>

#ifdef CONFIG_USB_DEVICE_STACK_NEXT

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/usb/usbd.h>
#include <zephyr/usb/usbd_msg.h>

LOG_MODULE_REGISTER(usb_cdc, LOG_LEVEL_INF);

#define GPREGRET_BOOTLOADER_MAGIC 0x57
#define TOUCH_BAUD_RATE 1200

/* Visible to tests: a global that the native_sim bootloader-entry
 * handler writes to. Tests can read it via /proc/<pid>/maps or by
 * reading the firmware's stdout at shutdown. For now, it is only
 * inspected via the process exit code.
 */
volatile uint8_t sim_gpregret;

static struct usbd_context *usbd;

static inline void do_bootloader_entry(void)
{
#if defined(CONFIG_BOARD_NATIVE_SIM) || defined(CONFIG_BOARD_NATIVE_SIM_64) || \
	defined(CONFIG_SOC_NATIVE_SIM)
	/* native_sim: write the magic into a volatile global, then exit
	 * with a recognizable code so the test harness can verify
	 * bootloader entry without needing GPREGRET hardware.
	 */
	sim_gpregret = GPREGRET_BOOTLOADER_MAGIC;
	LOG_INF("Bootloader entry: sim_gpregret=0x57, exit(42)");
	exit(42);
#else
	/* xiao_ble / nRF52840: write GPREGRET and reset. The Adafruit UF2
	 * bootloader checks GPREGRET at boot and enters UF2 mode if it
	 * sees 0x57.
	 */
	extern void NRF_POWER_REG_RESET(void);
	/* Direct register write — GPREGRET is at offset 0x51C in NRF_POWER. */
	volatile uint32_t *gpregret = (volatile uint32_t *)0x4000051CUL;
	*gpregret = GPREGRET_BOOTLOADER_MAGIC;
	LOG_INF("Bootloader entry: GPREGRET=0x57, NVIC reset");
	/* System reset request via ARM AIRCR */
	volatile uint32_t *aircr = (volatile uint32_t *)0xE000ED0CUL;
	*aircr = 0x05FA0004;
	/* Should not reach here */
	for (;;) {
		__asm__ volatile("wfi");
	}
#endif
}

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

		/* 1200-baud touch: DTR de-asserted while baud is 1200.
		 * Only trigger on the falling edge of DTR (was set,
		 * now cleared).
		 */
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
		/* Register all CDC-ACM class instances from devicetree.
		 * (Both the board's board_cdc_acm_uart and any
		 * user-defined nodes will be picked up.)
		 */
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
	LOG_INF("USB CDC-ACM device initialized");
	return 0;
}

#else /* !CONFIG_USB_DEVICE_STACK_NEXT */

int usb_cdc_init(void)
{
	return -ENODEV;
}

#endif /* CONFIG_USB_DEVICE_STACK_NEXT */

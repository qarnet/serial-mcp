/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * Main entry point for serial-mcp test firmware.
 *
 * Runs on the Zephyr `native_sim` POSIX emulator. The command channel
 * rides on the PTY-backed uart0 device that `native_sim` exposes. The
 * PTY path is printed to stdout at boot so the host can `open` it via
 * serial-mcp's `open` tool.
 *
 * USB CDC-ACM is OPTIONAL. When `boards/native_sim_usb.conf` is
 * applied, `CONFIG_USB_DEVICE_STACK=y` and `usb_cdc_init()` brings up
 * a CDC-ACM UART exposed via the native_posix USB/IP controller
 * (default port 3240, overridden to 3241 via the `USBIP_PORT` macro).
 * The CDC-ACM port is used to validate the 1200-baud touch →
 * `exit(42)` flow in software.
 */

#include "command.h"
#include "uart_drv.h"
#include "usb_cdc.h"

#include <errno.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

LOG_MODULE_REGISTER(main, LOG_LEVEL_INF);

static struct uart_drv uart0;
static struct app_state app;

int main(void)
{
	int ret;

	ret = uart_drv_init(&uart0);
	if (ret != 0) {
		LOG_ERR("UART driver init failed: %d", ret);
		return 0;
	}

	/* Try to bring up USB CDC-ACM. Returns -ENODEV if not configured
	 * (no `boards/native_sim_usb.conf` applied). That's expected on
	 * the no-USB build; log at debug level only.
	 */
	ret = usb_cdc_init();
	if (ret == 0) {
		LOG_INF("USB CDC-ACM initialized");
	} else if (ret != -ENODEV) {
		LOG_WRN("USB CDC-ACM init failed: %d", ret);
	}

	command_init(&app, &uart0);

	uart_drv_send_str(&uart0, "serial-mcp test firmware ready\r\n");

	char cmd_buf[UART_CMD_BUF_SIZE];

	while (1) {
		command_poll(&app);

		int len = uart_drv_get_cmd(&uart0, cmd_buf, sizeof(cmd_buf));
		if (len >= 0) {
			command_process(&app, cmd_buf);
		} else {
			k_msleep(1);
		}
	}

	return 0;
}

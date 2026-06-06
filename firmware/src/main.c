/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * Main entry point for serial-mcp test firmware on XIAO BLE nRF52840.
 * Provides UART CLI over physical UART0 for serial communication testing.
 */

#include "command.h"
#include "uart_drv.h"

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

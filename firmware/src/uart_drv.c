/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * UART0 driver with line-buffered RX and ringbuf TX.
 */

#include "uart_drv.h"
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <stdio.h>
#include <string.h>
#include <stdarg.h>

LOG_MODULE_REGISTER(uart_drv, LOG_LEVEL_INF);

static void uart_isr(const struct device *dev, void *user_data)
{
	struct uart_drv *drv = (struct uart_drv *)user_data;

	while (uart_irq_update(dev) && uart_irq_is_pending(dev)) {
		if (!drv->rx_throttled && uart_irq_rx_ready(dev)) {
			uint8_t byte;
			while (uart_fifo_read(dev, &byte, 1) == 1) {
				if (drv->trace_on) {
					uart_drv_printf(drv, "RX[%u]=0x%02x\r\n",
							drv->rx_seq, byte);
					drv->rx_seq++;
				}

				if (drv->cmd_len < UART_CMD_BUF_SIZE - 1) {
					if (byte == '\r' || byte == '\n') {
						if (drv->cmd_len > 0) {
							drv->cmd_buf[drv->cmd_len] = '\0';
							drv->cmd_ready = true;
							if (drv->framing_on) {
								uart_drv_printf(drv,
									"LINE len=%u data=\"%s\"\r\n",
									drv->cmd_len,
									drv->cmd_buf);
							}
						}
					} else {
						drv->cmd_buf[drv->cmd_len++] = byte;
					}
				} else {
					drv->cmd_buf[drv->cmd_len] = '\0';
					drv->cmd_ready = true;
				}
			}
		}

		if (uart_irq_tx_ready(dev)) {
			if (drv->tx_hold) {
				uart_irq_tx_disable(dev);
				drv->tx_busy = false;
				continue;
			}
			uint8_t tmp[64];
			int rb_len = (int)ring_buf_get(&drv->tx_ringbuf, tmp, sizeof(tmp));

			if (rb_len == 0) {
				uart_irq_tx_disable(dev);
				drv->tx_busy = false;
				continue;
			}

			int sent = uart_fifo_fill(dev, tmp, rb_len);
			if (sent < rb_len) {
				ring_buf_put(&drv->tx_ringbuf, tmp + sent,
					     rb_len - sent);
			}
		}
	}
}

int uart_drv_init(struct uart_drv *drv)
{
	memset(drv, 0, sizeof(*drv));
	drv->dev = DEVICE_DT_GET(DT_CHOSEN(zephyr_console));

	if (!device_is_ready(drv->dev)) {
		LOG_ERR("Console UART device not ready");
		return -ENODEV;
	}

	ring_buf_init(&drv->tx_ringbuf, sizeof(drv->tx_buf), drv->tx_buf);

	uart_irq_callback_user_data_set(drv->dev, uart_isr, drv);
	uart_irq_rx_enable(drv->dev);

	return 0;
}

void uart_drv_send(struct uart_drv *drv, const void *data, size_t len)
{
	if (len == 0) {
		return;
	}

	unsigned int key = irq_lock();
	size_t space = ring_buf_space_get(&drv->tx_ringbuf);
	size_t put_len = (len < space) ? len : space;
	ring_buf_put(&drv->tx_ringbuf, data, put_len);
	bool was_idle = !drv->tx_busy;
	drv->tx_busy = true;
	irq_unlock(key);

	if (put_len < len) {
		LOG_WRN("TX drop: %zu of %zu bytes", len - put_len, len);
	}

	if (was_idle) {
		uart_irq_tx_enable(drv->dev);
	}
}

void uart_drv_send_str(struct uart_drv *drv, const char *str)
{
	uart_drv_send(drv, str, strlen(str));
}

void uart_drv_printf(struct uart_drv *drv, const char *fmt, ...)
{
	char tmp[128];
	va_list ap;
	va_start(ap, fmt);
	int n = vsnprintk(tmp, sizeof(tmp), fmt, ap);
	va_end(ap);
	if (n > 0) {
		uart_drv_send(drv, tmp, (size_t)n);
	}
}

void uart_drv_tx_clear(struct uart_drv *drv)
{
	unsigned int key = irq_lock();
	ring_buf_reset(&drv->tx_ringbuf);
	drv->tx_busy = false;
	irq_unlock(key);
	uart_irq_tx_disable(drv->dev);
}

int uart_drv_get_cmd(struct uart_drv *drv, char *buf, size_t buf_size)
{
	if (!drv->cmd_ready) {
		return -1;
	}

	unsigned int key = irq_lock();
	size_t len = drv->cmd_len;
	if (len >= buf_size) {
		len = buf_size - 1;
	}
	memcpy(buf, drv->cmd_buf, len);
	buf[len] = '\0';
	drv->cmd_len = 0;
	drv->cmd_ready = false;
	irq_unlock(key);
	return (int)len;
}

const struct device *uart_drv_device(struct uart_drv *drv)
{
	return drv->dev;
}

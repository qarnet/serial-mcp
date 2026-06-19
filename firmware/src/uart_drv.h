#ifndef UART_DRV_H_
#define UART_DRV_H_

#include <zephyr/device.h>
#include <zephyr/drivers/uart.h>
#include <zephyr/sys/ring_buffer.h>
#include <zephyr/kernel.h>

#define UART_RX_BUF_SIZE 256
#define UART_TX_BUF_SIZE 4096
#define UART_CMD_BUF_SIZE 256

struct uart_drv {
	const struct device *dev;
	struct ring_buf tx_ringbuf;
	uint8_t tx_buf[UART_TX_BUF_SIZE];
	volatile bool tx_busy;
	volatile bool rx_throttled;

	uint8_t cmd_buf[UART_CMD_BUF_SIZE];
	uint16_t cmd_len;
	volatile bool cmd_ready;

	uint8_t rx_seq;
	bool trace_on;
	bool framing_on;
	bool tx_hold;
};

int uart_drv_init(struct uart_drv *drv);
void uart_drv_send(struct uart_drv *drv, const void *data, size_t len);
void uart_drv_send_str(struct uart_drv *drv, const char *str);
void uart_drv_printf(struct uart_drv *drv, const char *fmt, ...);
void uart_drv_tx_clear(struct uart_drv *drv);
int uart_drv_get_cmd(struct uart_drv *drv, char *buf, size_t buf_size);
const struct device *uart_drv_device(struct uart_drv *drv);

#endif

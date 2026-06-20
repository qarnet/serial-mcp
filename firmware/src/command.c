/*
 * Copyright (c) 2025 serial-mcp contributors
 * SPDX-License-Identifier: MIT
 *
 * Command parser and executor for serial-mcp test firmware.
 *
 * Commands:
 *   ping                                  → pong
 *   spam <count> hex [last_data=".."] [delay=<ms>]
 *   spam stop                             → stop transmission, report bytes
 *   info                                  → board/build info
 *   rxbuf status                          → partial-line buffer contents
 *   rxbuf clear                           → clear partial-line buffer
 *   arm_cmd <delay_ms>                    → delay before next cmd execution
 *   trace on|off                          → echo each RX byte with seq number
 *   framing on|off                        → report line boundaries on parse
 *   slow on [<us>]                        → slow consumer mode (delay reads)
 *   slow off                             → normal consumer mode
 *   write cmd <id> <rest...>             → execute <rest> tagged with <id>
 *   binary on|off                        → binary trace mode
 *   txbuf status                         → TX ring buffer occupancy
 *   ack on|off                           → pre-execution ack per command
 *   hold on|off                          → stall firmware TX drain
 *   touch                                 → exit(42) — bootloader entry trigger
 */

#include "command.h"
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

LOG_MODULE_REGISTER(command, LOG_LEVEL_INF);

#define HEX_CHARS "0123456789abcdef"

static uint32_t xorshift32(uint32_t *state)
{
	uint32_t x = *state;
	x ^= x << 13;
	x ^= x >> 17;
	x ^= x << 5;
	*state = x;
	return x;
}

static void spam_timer_cb(struct k_timer *timer)
{
	struct spam_ctx *spam = CONTAINER_OF(timer, struct spam_ctx, timer);

	if (!spam->active || spam->stopping) {
		k_timer_stop(timer);
		return;
	}

	if (spam->remaining == 0) {
		spam->stopping = true;
		k_timer_stop(timer);
		uart_drv_printf(spam->drv, "Spam complete: %u bytes sent\r\n",
				 spam->total_bytes);
		spam->active = false;
		return;
	}

	uint8_t pkt[256];
	uint32_t pkt_len = (spam->remaining < sizeof(pkt)) ?
			   spam->remaining : sizeof(pkt);

	for (uint32_t i = 0; i < pkt_len; i++) {
		uint32_t v = xorshift32(&spam->xorshift_state);
		pkt[i] = HEX_CHARS[v & 0x0f];
	}

	uart_drv_send(spam->drv, pkt, pkt_len);
	spam->total_bytes += pkt_len;
	spam->remaining -= pkt_len;

	if (spam->remaining == 0) {
		if (spam->last_data_len > 0) {
			uart_drv_send(spam->drv, spam->last_data,
				      spam->last_data_len);
			spam->total_bytes += spam->last_data_len;
		}
		spam->stopping = true;
		k_timer_stop(timer);
		uart_drv_printf(spam->drv, "Spam complete: %u bytes sent\r\n",
				 spam->total_bytes);
		spam->active = false;
	}
}

static void cmd_sendraw(struct app_state *state, char *args)
{
	char *saveptr = NULL;
	char *mode = strtok_r(args, " ", &saveptr);
	char *data;

	if (!mode || *mode == '\0') {
		uart_drv_send_str(state->uart,
				  "ERR usage: sendraw hex|text <data>\r\n");
		return;
	}

	if (strcmp(mode, "hex") == 0) {
		data = strtok_r(NULL, "", &saveptr);
		if (!data) {
			uart_drv_send_str(state->uart,
					  "ERR usage: sendraw hex <hexdata>\r\n");
			return;
		}
		uint8_t byte;
		char pair[3] = {0};
		while (*data) {
			/* skip whitespace */
			while (*data == ' ')
				data++;
			if (!*data || !data[1])
				break;
			pair[0] = data[0];
			pair[1] = data[1];
			byte = (uint8_t)strtoul(pair, NULL, 16);
			uart_drv_send(state->uart, &byte, 1);
			data += 2;
		}
	} else if (strcmp(mode, "text") == 0) {
		data = strtok_r(NULL, "", &saveptr);
		if (!data) {
			data = "";
		}
		uart_drv_send_str(state->uart, data);
		/* No \r\n appended */
	} else {
		uart_drv_send_str(state->uart,
				  "ERR usage: sendraw hex|text <data>\r\n");
	}
}

static void cmd_ping(struct app_state *state)
{
	uart_drv_send_str(state->uart, "pong\r\n");
}

static void cmd_info(struct app_state *state)
{
	uart_drv_printf(state->uart,
		"board=native_sim build=0.1.0 " __DATE__ " " __TIME__ "\r\n");
}

static void cmd_spam_start(struct app_state *state, char *count_str, char *rest)
{
	if (state->spam.active) {
		uart_drv_send_str(state->uart, "ERR spam already active\r\n");
		return;
	}

	uint32_t count = (uint32_t)strtoul(count_str, NULL, 10);

	char *tok = strtok(rest, " ");
	if (!tok || strcmp(tok, "hex") != 0) {
		uart_drv_send_str(state->uart, "ERR only hex mode supported\r\n");
		return;
	}

	state->spam.total_bytes = 0;
	state->spam.remaining = count;
	state->spam.delay_ms = 10;
	state->spam.last_data_len = 0;
	state->spam.stopping = false;
	state->spam.xorshift_state = 0x12345678;

	while ((tok = strtok(NULL, " ")) != NULL) {
		if (strncmp(tok, "delay=", 6) == 0) {
			state->spam.delay_ms =
				(uint16_t)strtoul(tok + 6, NULL, 10);
		} else if (strncmp(tok, "last_data=", 10) == 0) {
			char *val = tok + 10;
			size_t vlen = strlen(val);
			if (vlen > 0 && val[0] == '"') {
				val++;
				vlen--;
			}
			if (vlen > 0 && val[vlen - 1] == '"') {
				vlen--;
			}
			if (vlen > sizeof(state->spam.last_data)) {
				vlen = sizeof(state->spam.last_data);
			}
			memcpy(state->spam.last_data, val, vlen);
			state->spam.last_data_len = (uint8_t)vlen;
		}
	}

	state->spam.active = true;
	state->spam.drv = state->uart;
	k_timer_init(&state->spam.timer, spam_timer_cb, NULL);
	k_timer_start(&state->spam.timer, K_MSEC(state->spam.delay_ms),
		       K_MSEC(state->spam.delay_ms));

	uart_drv_printf(state->uart, "spam start count=%u delay=%u\r\n",
			 count, state->spam.delay_ms);
}

static void cmd_spam_stop(struct app_state *state)
{
	if (!state->spam.active) {
		uart_drv_send_str(state->uart, "ERR no spam active\r\n");
		return;
	}

	state->spam.stopping = true;
	k_timer_stop(&state->spam.timer);
	state->spam.active = false;
	uart_drv_tx_clear(state->uart);
	uart_drv_printf(state->uart, "Spam stopped: %u bytes sent\r\n",
			 state->spam.total_bytes);
}

static void cmd_rxbuf_status(struct app_state *state)
{
	struct uart_drv *drv = state->uart;
	unsigned int key = irq_lock();
	uint16_t len = drv->cmd_len;
	char tmp[UART_CMD_BUF_SIZE];
	memcpy(tmp, drv->cmd_buf, len);
	irq_unlock(key);

	tmp[len] = '\0';
	uart_drv_printf(state->uart, "rxbuf len=%u data=\"", len);
	uart_drv_send(state->uart, tmp, len);
	uart_drv_send_str(state->uart, "\"\r\n");
}

static void cmd_rxbuf_clear(struct app_state *state)
{
	struct uart_drv *drv = state->uart;
	unsigned int key = irq_lock();
	uint16_t old_len = drv->cmd_len;
	drv->cmd_len = 0;
	drv->cmd_ready = false;
	irq_unlock(key);

	uart_drv_printf(state->uart, "rxbuf clear was_len=%u\r\n", old_len);
}

static void cmd_arm(struct app_state *state, char *delay_str)
{
	if (!delay_str || *delay_str == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: arm_cmd <delay_ms>\r\n");
		return;
	}

	state->arm_delay_ms = (uint32_t)strtoul(delay_str, NULL, 10);
	state->arm_active = true;
	uart_drv_printf(state->uart, "arm_cmd delay=%u\r\n",
			 state->arm_delay_ms);
}

static void cmd_trace(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: trace on|off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		state->uart->trace_on = true;
		state->uart->rx_seq = 0;
		uart_drv_send_str(state->uart, "trace on\r\n");
	} else if (strcmp(arg, "off") == 0) {
		state->uart->trace_on = false;
		uart_drv_send_str(state->uart, "trace off\r\n");
	} else {
		uart_drv_send_str(state->uart, "ERR usage: trace on|off\r\n");
	}
}

static void cmd_framing(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: framing on|off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		state->uart->framing_on = true;
		uart_drv_send_str(state->uart, "framing on\r\n");
	} else if (strcmp(arg, "off") == 0) {
		state->uart->framing_on = false;
		uart_drv_send_str(state->uart, "framing off\r\n");
	} else {
		uart_drv_send_str(state->uart, "ERR usage: framing on|off\r\n");
	}
}

static void cmd_slow(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: slow on [<us>] | slow off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		state->slow_delay_us = 50000;
		state->slow_mode = true;
		uart_drv_printf(state->uart, "slow on delay=%u\r\n",
				 state->slow_delay_us);
	} else if (strncmp(arg, "on ", 3) == 0) {
		state->slow_delay_us = (uint32_t)strtoul(arg + 3, NULL, 10);
		state->slow_mode = true;
		uart_drv_printf(state->uart, "slow on delay=%u\r\n",
				 state->slow_delay_us);
	} else if (strcmp(arg, "off") == 0) {
		state->slow_mode = false;
		uart_drv_send_str(state->uart, "slow off\r\n");
	} else {
		uart_drv_send_str(state->uart, "ERR usage: slow on [<us>] | slow off\r\n");
	}
}

static void cmd_write(struct app_state *state, char *args)
{
	char *saveptr2 = NULL;
	char *sub = strtok_r(args, " ", &saveptr2);
	if (!sub || strcmp(sub, "cmd") != 0) {
		uart_drv_send_str(state->uart, "ERR usage: write cmd <id> <command...>\r\n");
		return;
	}

	char *id_str = strtok_r(NULL, " ", &saveptr2);
	if (!id_str) {
		uart_drv_send_str(state->uart, "ERR missing command id\r\n");
		return;
	}

	uint32_t cmd_id = (uint32_t)strtoul(id_str, NULL, 10);
	char *rest = strtok_r(NULL, "", &saveptr2);
	if (!rest) {
		uart_drv_send_str(state->uart, "ERR missing command body\r\n");
		return;
	}

	uart_drv_printf(state->uart, "ack %u exec>%s\r\n", cmd_id, rest);

	command_process(state, rest);
}

static void cmd_binary(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: binary on|off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		state->binary_mode = true;
		state->uart->trace_on = true;
		state->uart->rx_seq = 0;
		uart_drv_send_str(state->uart, "binary on\r\n");
	} else if (strcmp(arg, "off") == 0) {
		state->binary_mode = false;
		uart_drv_send_str(state->uart, "binary off\r\n");
	} else {
		uart_drv_send_str(state->uart, "ERR usage: binary on|off\r\n");
	}
}

static void cmd_txbuf_status(struct app_state *state)
{
	struct uart_drv *drv = state->uart;
	unsigned int key = irq_lock();
	uint32_t len = ring_buf_size_get(&drv->tx_ringbuf);
	bool busy = drv->tx_busy;
	irq_unlock(key);

	uart_drv_printf(state->uart, "txbuf len=%u busy=%u\r\n", len, busy);
}

static void cmd_ack(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: ack on|off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		state->ack_enabled = true;
		state->ack_seq = 0;
		uart_drv_send_str(state->uart, "ack on\r\n");
	} else if (strcmp(arg, "off") == 0) {
		state->ack_enabled = false;
		uart_drv_send_str(state->uart, "ack off\r\n");
	} else {
		uart_drv_send_str(state->uart, "ERR usage: ack on|off\r\n");
	}
}

static void cmd_hold(struct app_state *state, char *arg)
{
	if (!arg || *arg == '\0') {
		uart_drv_send_str(state->uart, "ERR usage: hold on|off\r\n");
		return;
	}

	if (strcmp(arg, "on") == 0) {
		uart_drv_send_str(state->uart, "hold on\r\n");
		state->uart->tx_hold = true;
	} else if (strcmp(arg, "off") == 0) {
		uart_drv_send_str(state->uart, "hold off\r\n");
		state->uart->tx_hold = false;
		/* Re-enable TX IRQ so pending data can drain. */
		uart_irq_tx_enable(state->uart->dev);
	} else {
		uart_drv_send_str(state->uart, "ERR usage: hold on|off\r\n");
	}
}

static void cmd_jsonout(struct app_state *state)
{
	/* Emit JSON lines for parser testing.
	   Each line is a complete JSON object terminated by \r\n. */
	uart_drv_printf(state->uart,
			"{\"sensor\":\"temp\",\"value\":25.5,\"unit\":\"C\"}\r\n");
	uart_drv_printf(state->uart,
			"{\"sensor\":\"humidity\",\"value\":60,\"unit\":\"%%\"}\r\n");
	uart_drv_printf(state->uart,
			"{\"sensor\":\"pressure\",\"value\":1013.25,\"unit\":\"hPa\"}\r\n");
}

void command_init(struct app_state *state, struct uart_drv *drv)
{
	memset(state, 0, sizeof(*state));
	state->uart = drv;
	state->slow_delay_us = 50000;
	state->spam.drv = drv;
}

void command_process(struct app_state *state, char *line)
{
	char *saveptr = NULL;
	char *cmd = strtok_r(line, " ", &saveptr);

	if (!cmd) {
		return;
	}

	if (state->arm_active) {
		uint32_t delay = state->arm_delay_ms;
		state->arm_active = false;
		k_msleep(delay);
	}

	if (state->slow_mode) {
		k_usleep(state->slow_delay_us);
	}

	if (state->ack_enabled) {
		uart_drv_printf(state->uart, "ack %u\r\n", state->ack_seq++);
	}

	if (strcmp(cmd, "ping") == 0) {
		cmd_ping(state);
	} else if (strcmp(cmd, "spam") == 0) {
		char *sub = strtok_r(NULL, " ", &saveptr);
		if (sub && strcmp(sub, "stop") == 0) {
			cmd_spam_stop(state);
		} else if (sub) {
			cmd_spam_start(state, sub, saveptr);
		} else {
			uart_drv_send_str(state->uart, "ERR usage: spam <count> hex [last_data=\"..\"] [delay=<ms>]\r\n");
		}
	} else if (strcmp(cmd, "info") == 0) {
		cmd_info(state);
	} else if (strcmp(cmd, "rxbuf") == 0) {
		char *sub = strtok_r(NULL, " ", &saveptr);
		if (sub && strcmp(sub, "status") == 0) {
			cmd_rxbuf_status(state);
		} else if (sub && strcmp(sub, "clear") == 0) {
			cmd_rxbuf_clear(state);
		} else {
			uart_drv_send_str(state->uart, "ERR usage: rxbuf status|clear\r\n");
		}
	} else if (strcmp(cmd, "arm_cmd") == 0) {
		cmd_arm(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "trace") == 0) {
		cmd_trace(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "framing") == 0) {
		cmd_framing(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "slow") == 0) {
		cmd_slow(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "write") == 0) {
		cmd_write(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "binary") == 0) {
		cmd_binary(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "txbuf") == 0) {
		char *sub = strtok_r(NULL, " ", &saveptr);
		if (sub && strcmp(sub, "status") == 0) {
			cmd_txbuf_status(state);
		} else {
			uart_drv_send_str(state->uart, "ERR usage: txbuf status\r\n");
		}
	} else if (strcmp(cmd, "ack") == 0) {
		cmd_ack(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "hold") == 0) {
		cmd_hold(state, saveptr ? saveptr : "");
	} else if (strcmp(cmd, "touch") == 0) {
		uart_drv_send_str(state->uart, "touch exit(42)\r\n");
		exit(42);
	} else if (strcmp(cmd, "jsonout") == 0) {
		cmd_jsonout(state);
	} else if (strcmp(cmd, "sendraw") == 0) {
		cmd_sendraw(state, saveptr ? saveptr : "");
	} else {
		uart_drv_printf(state->uart, "ERR unknown command: %s\r\n", cmd);
	}
}

void command_poll(struct app_state *state)
{
	if (state->spam.active && state->spam.stopping) {
		state->spam.active = false;
	}
}

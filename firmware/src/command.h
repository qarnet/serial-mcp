#ifndef COMMAND_H_
#define COMMAND_H_

#include "uart_drv.h"

struct spam_ctx {
	bool active;
	bool stopping;
	uint32_t total_bytes;
	uint32_t remaining;
	uint16_t delay_ms;
	uint8_t last_data[4];
	uint8_t last_data_len;
	uint32_t xorshift_state;
	struct k_timer timer;
	struct uart_drv *drv;
};

struct app_state {
	struct uart_drv *uart;
	struct spam_ctx spam;

	bool arm_active;
	uint32_t arm_delay_ms;

	bool slow_mode;
	uint32_t slow_delay_us;

	bool cmd_id_enabled;
	uint32_t next_cmd_id;

	bool binary_mode;

	bool ack_enabled;
	uint32_t ack_seq;
};

void command_init(struct app_state *state, struct uart_drv *drv);
void command_process(struct app_state *state, char *line);
void command_poll(struct app_state *state);

#endif
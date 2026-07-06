Before we continue with phase 3, could you give me a review on the serial-mcp server? I was seeing you use it in a maybe confusing way for example. You open the serial port, flash the device and the start reading. The tool only starts buffering bytes, once you start the read command. You would have to use the subscribe tool to get all output from a certain point on. Is that unexpected behaviour to you? Give me your thoughts on using the tool
+ Thought: 10.4s
Honest review of my serial-mcp usage — you're right, I used it wrong.
What I did wrong
Pattern I used: open → flash/reset → read
Problem: read only returns bytes received after the call starts. By the time I called read, the device had already booted and the banner was sitting in the OS UART buffer (or already gone). On the E83 I got lucky — the boot was slow enough that bytes were still in the buffer. On the Xiao I got nothing and had to reset a second time.
Flush made it worse: I called flush(target=input) before read in some cases — that discards any buffered boot bytes, guaranteeing I'd miss the banner. Then read times out because no new bytes arrive (device already booted).
What I should have done
Correct pattern for boot-log capture:
1. open the port
2. subscribe (with timeout or match) — starts a background notification stream that captures everything from this moment on
3. Then flash/reset the device (via bash/openocd)
4. The boot bytes arrive as MCP notifications while I'm running the flash command
5. unsubscribe or let the subscription timeout
OR if not using subscribe:
1. open the port
2. Start a read with a generous timeout before triggering reset (the read is blocking, so I'd need it in parallel with the reset — which means the bash call and the read can't be sequential. Subscribe is the better tool here.)
For interactive monitoring (e.g., watching logs while a test runs): subscribe is the right tool — it streams continuously via notifications. read is for one-shot "give me what's there now" or "wait for a specific pattern."
Why I got confused
The read tool description does say "Returns only future bytes — data received after the call starts, not previously buffered data." I read that but didn't internalize the implication: I need the read active BEFORE the event I want to capture. Instead I treated it like cat on a file — open the port, do stuff, then read what accumulated. That mental model is wrong for this tool.
The subscribe tool is the one that matches my mental model of "capture everything from now on" — it runs in the background and forwards bytes as notifications. I should have used it for all the boot-capture scenarios.
Concrete workflow I'd use next time
1. serial-mcp open → connection_id
2. serial-mcp subscribe (timeout_ms=15000, optional match="Advertising")
3. [bash] openocd ... -c "reset run"   ← device boots, bytes stream as notifications
4. [notifications arrive with boot log]
5. serial-mcp unsubscribe (or auto-stops on timeout/match)
For flash + boot capture:
1. serial-mcp open
2. serial-mcp subscribe (timeout_ms=30000)
3. [bash] fw-flash-5340  ← flash resets the device, boot bytes stream
4. [notifications arrive]
5. serial-mcp unsubscribe
Tool UX observations
1. read vs subscribe distinction isn't obvious at first glance. Both "read serial data." The key difference (one-shot vs background stream) is buried in the description. A naming hint or a clearer one-liner would help — e.g., read = "one-shot read" vs subscribe = "continuous stream."
2. read returning zero bytes on timeout looks like an error. I saw bytes_returned: 0 and assumed something was wrong with the port, when actually it just meant "no new data arrived during the window." The stop_reason: "timeout" field clarifies this, but I had to look for it.
3. No way to read previously-buffered data. The OS UART driver buffers incoming bytes, but read skips that buffer and only returns new arrivals. For boot-log capture (where the event already happened), this forces a second reset. subscribe has the same limitation — it only captures from the subscribe call onward. This is correct behavior (predictable, no stale data), but it means the tool requires you to think about timing: start the reader before the event.
4. The flush tool is dangerous for boot capture. It silently discards buffered bytes. I used it to "clean up" before a read, not realizing I was throwing away the boot log I wanted. Maybe a warning in the description that flush discards unread data would help.
5. Framing would help for log parsing. I got raw byte streams with ANSI escape codes mixed in. Using rx_framing: {type: "line"} would have given me one notification per line, much easier to analyze. I didn't use framing at all — raw stream made the boot log harder to read.
Summary
My usage was wrong. subscribe before reset/flash is the correct pattern for boot capture. read is for one-shot/pattern-match queries. flush before capture is destructive. Framing by line would have made output cleaner. I'll use these patterns correctly going forward.
▣  Thinker · GLM-5.2 · 28.3s
Okay thank you for the summary. So something like a circle buffer which lets you do operations similar to cat would be helpful you think?
Yes, a ring buffer mode would fix my core mistake. Here's what I'd find most useful:
The feature
A per-connection circular buffer that always captures incoming bytes in the background, regardless of whether a read or subscribe is active. Then:
- read returns from the buffer (like cat) — gets everything since the last read, not just bytes after the call
- flush still works to clear it
- subscribe still streams live notifications for the "watch continuously" use case
Why it helps
My repeated failure pattern was: open port → flash/reset → read. The device already booted during the flash. With a ring buffer, the boot bytes are sitting in it when I call read — I get the log immediately without a second reset.
This is exactly what /dev/ttyUSB0 already does at the OS level (the kernel UART driver buffers incoming bytes). serial-mcp currently skips that buffer and only returns post-call arrivals. Matching the OS behavior would make the tool intuitive — cat /dev/ttyUSB0 works because the kernel buffers; serial-mcp read doesn't because it ignores the buffer.
Suggested shape
- Always-on ring buffer per connection (configurable size, default e.g. 64 KB — enough for several boot cycles at 115200)
- read returns from buffer head (oldest unread bytes), advances head. Optional timeout_ms waits for new data if buffer empty.
- flush clears the buffer (as today)
- subscribe still does live notifications — for the "watch in real time" case where you don't want to poll
- get_log already does something similar (returns the event log) — the ring buffer is the same idea for raw RX data
Tradeoff
Memory: 64 KB per connection × N connections. Trivial on a dev machine.
Complexity: low — it's a ring buffer with a read pointer. The hard part (serial I/O, framing, matching) is already implemented.
One caveat
Ring buffer size matters. At 115200 baud, one byte ≈ 87 µs. 64 KB ≈ 5.6 seconds of continuous data. A boot log (1-2 KB) fits easily. A long streaming session would wrap the buffer and lose old data — but for that case subscribe is the right tool anyway. The ring buffer is for "I just did something and want to see what happened" — exactly my boot-capture use case.
Short answer: yes, a ring buffer with read draining it would have saved me several failed capture attempts and a lot of confusion.

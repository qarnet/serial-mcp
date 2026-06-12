# Simulation Matrix — serial-mcp Test Suitability

## Goal

- Compare simulation and virtualization approaches for `serial-mcp` testing.
- Focus on what each option gives, what it lacks, and how well it fits CI.
- Ignore implementation effort here. Focus on testing value.

## Summary

- Best current fit for `serial-mcp`: **Zephyr `native_sim` + PTY**
- Best simple supplement: **plain PTY pair** or **custom UART simulator**
- Best truth layer: **real hardware HIL**
- Most relevant when testing embedded Linux images: **QEMU**
- Most relevant when deeper MCU/peripheral simulation matters: **Renode**

## Comparison Table

| Approach | What runs | UART realism | Device/board realism | Control-line realism | Speed | Determinism | CI fit | Best for serial-mcp | Main gain | Main lack |
|---|---|---:|---:|---:|---:|---:|---:|---|---|---|
| **Zephyr `native_sim` + PTY** | Real Zephyr app as host executable | High | Low-medium | Low | Very high | High | Excellent | End-to-end MCP serial tests against Zephyr firmware logic | Real firmware behavior with real PTY endpoint | Not true hardware, incomplete line-control APIs |
| **QEMU** | Full guest machine / OS image | Medium-high | Medium-high | Low-medium | Medium | Medium-high | Good | Linux guest serial console, boot/log automation | Whole system behavior, kernel/userspace interaction | Heavy, slower, more moving parts, weaker “real serial device” feel |
| **Renode** | Simulated MCU/board/peripherals | High | High | Medium | Medium | High | Good | Rich MCU/peripheral protocol tests | Better board/peripheral simulation than native_sim | More abstract than real OS tty path, still simulator limits |
| **Plain PTY pair (`socat`, tty0tty, com0com)** | No firmware unless custom responder added | Medium | None | None | Very high | High | Excellent | Tool plumbing, framing, buffering, open/close behavior | Simplest fake serial endpoint | No real firmware/application behavior |
| **Custom UART simulator/responder** | Scripted fake device | Medium | None-low | None | High | High | Excellent | Deterministic protocol-response tests | Exact protocol scenarios, fault injection, golden behavior | Easy to overfit, not real firmware |
| **Real hardware HIL** | Actual board + actual USB/UART | Very high | Very high | High | Low | Low-medium | Fair-poor | Final truth, hardware regressions | Real world confidence | Flaky, lab deps, slower, harder CI |

## Thoughts on Each

### Zephyr `native_sim` + PTY

**Overall:** best core CI solution for this repo.

What it gives:

- real Zephyr firmware logic
- real serial-like host endpoint
- software-only execution
- fast and reproducible CI
- strong lifecycle and command-path validation

What it lacks:

- not real hardware
- line control support incomplete
- USB behavior not fully real
- hardware timing not faithful

CI view:

- excellent sweet spot
- likely best long-term default foundation for `serial-mcp`

### QEMU

**Overall:** useful when testing full guest systems, especially embedded Linux.

What it gives:

- whole guest OS boot path
- serial console testing at system level
- kernel/userspace interaction
- good fit for Yocto-style console workflows

What it lacks:

- heavier than needed for most current `serial-mcp` tests
- slower than `native_sim`
- less natural for “open a serial-like PTY and talk to firmware” workflows

CI view:

- good, but larger hammer
- strongest when repo focus shifts toward Linux images or whole-system serial consoles

### Renode

**Overall:** strong future option when deeper peripheral or board simulation matters.

What it gives:

- richer MCU/peripheral simulation
- more board realism than `native_sim`
- useful when UART behavior is tied to other simulated peripherals

What it lacks:

- still simulator, not real tty hardware
- may need extra bridging to expose exactly the host-side serial shape desired
- solves broader simulation problems than current repo needs

CI view:

- good candidate for advanced future tests
- more complementary than replacement for current PTY-native_sim approach

### Plain PTY Pair (`socat`, `tty0tty`, `com0com`)

**Overall:** excellent low-level supplement, not enough alone.

What it gives:

- very fast setup
- clean serial-like endpoints
- great for open/read/write/flush/close semantics
- ideal for transport and plumbing tests

What it lacks:

- no firmware behavior
- no embedded command parser unless added externally
- no realistic device state machine by itself

CI view:

- excellent for narrow tool-path tests
- weak as sole source of end-to-end confidence

### Custom UART Simulator / Responder

**Overall:** great for deterministic protocol testing and fault injection.

What it gives:

- exact scripted behavior
- strong determinism
- easy error-path and corner-case injection
- useful for future decoder/parser tests

What it lacks:

- fake behavior can drift from reality
- confidence limited by simulator quality
- easy to overfit tests to expected responses

CI view:

- excellent companion layer
- especially good for protocol-level and parser-level tests
- should supplement, not replace, real firmware-backed validation

### Real Hardware HIL

**Overall:** truth layer, not ideal primary PR gate.

What it gives:

- real USB/UART behavior
- real line control and reset behavior
- board-specific quirks
- highest external validity

What it lacks:

- slower
- more flaky
- harder to parallelize
- lab/device dependency

CI view:

- best as final confidence or scheduled validation layer
- not ideal as main always-on merge gate unless lab setup is very stable

## Recommended View for serial-mcp

### Best base layer

- **Zephyr `native_sim` + PTY**

### Best supporting layers

- **Plain PTY/custom responder** for focused deterministic edge cases
- **Real hardware HIL** for occasional truth checks
- **Renode** later if tests grow beyond simple UART shell behavior

### Less central right now

- **QEMU**, unless `serial-mcp` begins focusing more on embedded Linux serial-console workflows

## Bottom Line

For `serial-mcp`, the current `native_sim` approach is a strong choice:

- more realistic than pure mocks
- much simpler than full system emulation
- far better CI fit than hardware-only validation

It should remain the primary software-only integration layer unless the project’s center of gravity shifts significantly toward Linux images, deep peripheral simulation, or hardware-accurate line-control validation.

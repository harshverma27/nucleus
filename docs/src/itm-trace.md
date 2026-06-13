# Enabling ITM Trace

`nucleus trace` decodes ARM CoreSight ITM/SWO packets and streams them to the
trace dashboard. Getting data flowing end-to-end needs two pieces wired
together: a few lines of C in your firmware that configure the ITM/TPIU
peripherals and write to stimulus ports, and an OpenOCD session that captures
the resulting SWO byte stream and forwards it to `nucleus trace`.

```text
Firmware (ITM) --SWO--> ST-Link --USB--> OpenOCD --TCP--> nucleus trace --WebSocket--> dashboard
```

## Firmware side: the CoreSight register setup

CMSIS headers (pulled in by `stm32f4xx_hal.h`) define `CoreDebug`, `TPI`, and
`ITM` as memory-mapped structs. Enabling SWO output is six register writes:

```c
#include "stm32f4xx_hal.h"

/* Configure SWO output. `core_hz` is [device].clock_hz; `swo_hz` must match
 * [trace].swo_freq in stm32.toml (and the value passed to
 * `nucleus trace --openocd`). */
void itm_init(uint32_t core_hz, uint32_t swo_hz)
{
    CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk; /* 1. enable tracing */

    TPI->SPPR = 2;                       /* 2. SWO protocol = NRZ/UART */
    TPI->ACPR = (core_hz / swo_hz) - 1;  /* 3. SWO baud rate divisor */

    ITM->LAR = 0xC5ACCE55;               /* 4. unlock the ITM registers */
    ITM->TCR |= ITM_TCR_ITMENA_Msk;      /* 5. enable the ITM */
    ITM->TER |= 1UL;                     /* 6. enable stimulus port 0 (log text) */
}
```

Call `itm_init(...)` once, after `SystemClock_Config()`, alongside the
generated `Nucleus_Init()`.

### Logging text (port 0)

`nucleus-trace` reassembles single-byte writes to stimulus port 0 into UTF-8
log lines, splitting on `\n`. Write one byte at a time, blocking while the
port's FIFO is full:

```c
void itm_log(const char *s)
{
    while (*s) {
        while (ITM->PORT[0].u32 == 0) { } /* wait for FIFO space */
        ITM->PORT[0].u8 = (uint8_t)*s;
        s++;
    }
}
```

A trailing `\n` flushes the line to the dashboard's log panel:

```c
itm_log("system init complete\n");
```

### Tracing typed variables (ports 1-7)

Each `[[trace.variables]]` entry in `stm32.toml` names a port and a type
(`f32`, `u16`, `u32`, or `i32`). The *access width* of the write to
`ITM->PORT[n]` determines the packet size `nucleus-itm` reports, so match the
width to the type: 4 bytes for `f32`/`u32`/`i32`, 2 bytes for `u16`.

```c
static inline void itm_write32(uint8_t port, uint32_t value)
{
    if (!(ITM->TCR & ITM_TCR_ITMENA_Msk) || !(ITM->TER & (1UL << port)))
        return;
    while (ITM->PORT[port].u32 == 0) { }
    ITM->PORT[port].u32 = value;
}

static inline void itm_write16(uint8_t port, uint16_t value)
{
    if (!(ITM->TCR & ITM_TCR_ITMENA_Msk) || !(ITM->TER & (1UL << port)))
        return;
    while (ITM->PORT[port].u16 == 0) { }
    ITM->PORT[port].u16 = value;
}
```

To trace an `f32` on port 1, write its bits as `u32`:

```c
float temperature = read_temperature();
uint32_t bits;
memcpy(&bits, &temperature, sizeof(bits));
itm_write32(1, bits);
```

Remember to enable each port you use — add it to `itm_init()`:

```c
ITM->TER |= (1UL << 1); /* enable port 1 for the temperature trace */
```

## OpenOCD side

OpenOCD captures SWO from the ST-Link and forwards it to a TCP port via
`tpiu config internal`. `nucleus trace --openocd <telnet_addr>` sends this
sequence for you over OpenOCD's telnet console (default port 4444):

```
tpiu config internal :<trace_port> uart off <core_hz> <swo_hz>
itm ports on
```

`<trace_port>` is the TCP port `nucleus trace --trace-tcp` connects to, and
`<core_hz>`/`<swo_hz>` must match the values passed to `itm_init()` in
firmware. If you'd rather configure OpenOCD by hand (e.g. to debug a version
mismatch), connect with `telnet localhost 4444` and run the same two commands.

## Putting it together

```toml
# stm32.toml
[trace]
enabled  = true
swo_freq = 2_000_000

[[trace.variables]]
name = "temperature"
port = 1
type = "f32"
```

```sh
nucleus trace --trace-tcp 127.0.0.1:3344 --openocd 127.0.0.1:4444 \
              --config stm32.toml
```

See [CLI Usage](cli.md#nucleus-trace) for the full flag reference, and open
the dashboard via the VS Code command **Nucleus: Open Trace Dashboard** (or
`extension/dist/index.html` standalone) to see the log lines and `temperature`
chart update live.

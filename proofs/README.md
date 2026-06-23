# Nucleus Proofs

Step-by-step terminal PDFs demonstrating nucleus capabilities. Each PDF shows real command execution, output, and results for a specific feature or workflow.

## PDFs

### 1. Build_Sample.pdf
**Basic firmware build workflow**

- Project initialization: `nucleus init --board NUCLEO-F411RE`
- Configuration: stm32.toml device setup
- Firmware: Simple LED blink code in C
- Validation: `nucleus check` (constraint solver)
- Build: `nucleus build` (generate + compile)
- Output: firmware.bin, firmware.elf, firmware.hex ready for flashing

**Use case:** Get started with a first nucleus project. Understand project structure and build pipeline.

---

### 2. ITM_Setup.pdf
**Real-time debug output via Instrumentation Trace Macrocell (ITM)**

- Configure trace in stm32.toml (SWO frequency, variables)
- Firmware setup: itm_init(), itm_log(), itm_write32()
- Hardware connection: ST-Link USB
- OpenOCD: Capture SWO packets
- nucleus trace: Decode ITM → WebSocket → Dashboard
- Dashboard: Live log panel + variable charts

**Use case:** Stream debug output and variable traces from firmware to browser dashboard in real-time. No UART debugging needed.

---

### 3. Hardware_Testing.pdf
**Dual-backend test execution: Hardware, QEMU, Both**

Shows three test configurations:

1. **Hardware Only** (`nucleus test --backend hardware`)
   - Connect to real NUCLEO-F411RE via ST-Link
   - Run GPIO/UART test on silicon
   
2. **QEMU Only** (`nucleus test --backend qemu`)
   - Emulate STM32F411 in QEMU (netduinoplus2 machine)
   - Run same test in simulation (1.89s vs 2.34s hardware)
   
3. **Both** (`nucleus test` default)
   - Sequential hardware + QEMU
   - Detect sim/silicon divergence

Test code: Rust with nucleus-test-sdk AgentClient. Controls GPIO, UART via RAM mailbox protocol. No special debug infrastructure.

**Use case:** Validate firmware on both simulated and real hardware. Fast QEMU iteration, confidence from hardware testing. Detect emulator bugs.

---

### 4. Test_Writing_Guide.pdf
**Complete reference for writing tests in Rust**

7 sections:

1. **Setup** — Project structure, stm32.toml [[test]], Cargo.toml deps
2. **Basic Structure** — Hello world → backend connection
3. **Common Patterns** — GPIO, UART, registers, stateful sequences
4. **Error Handling** — SdkError types, recovery
5. **Environment** — Logging, backend detection, timing/delays
6. **Best Practices** — 8 rules (always connect first, assertions, timing, etc)
7. **Complete Example** — Realistic multi-device test (GPIO + UART combined)

**Use case:** Write your own tests. Learn AgentClient API, RAM mailbox protocol, test patterns.

---

### 5. Test_History.pdf
**Test results tracking, dashboard visualization, CI integration**

8 sections:

1. **History File Format** — tests/test_history.json structure (append-only)
2. **Querying History** — `nucleus history`, `nucleus history --graph`, `nucleus show` commands
3. **Dashboard Bar Chart** — Real-time visualization of pass/fail/skip trends
4. **CI Integration** — GitHub Actions workflow, auto-posted PR comments
5. **Complete CI Workflow** — Push → CI runs → PR comment → Merge → Hardware validation
6. **Persisting History** — Git commit vs gitignore trade-offs
7. **Troubleshooting** — No history file, skipped tests, CI comment issues
8. **Commands Reference** — All nucleus history/show/test commands

**Use case:** Track test results over time. Integrate tests into CI. Visualize trends. Auto-comment PRs with test summaries.

---

## Quick Start

1. **Just build?** Start with **Build_Sample.pdf**
2. **Debug output?** Read **ITM_Setup.pdf** next
3. **Test hardware?** Follow **Hardware_Testing.pdf**
4. **Write tests?** Reference **Test_Writing_Guide.pdf**
5. **Track results?** See **Test_History.pdf** for CI + dashboard

---

## Typical Workflow

```
1. nucleus init --board NUCLEO-F411RE           (Build_Sample)
2. Edit stm32.toml, src/main.c
3. nucleus check                                 (Build_Sample)
4. nucleus build                                 (Build_Sample)
5. nucleus test --backend qemu                  (Hardware_Testing)
6. nucleus history (view test results)          (Test_History)
7. nucleus flash                                 (Hardware_Testing)
8. nucleus test --backend hardware              (Hardware_Testing)
9. nucleus trace --openocd 127.0.0.1:4444      (ITM_Setup)
10. Open dashboard at http://localhost:5678    (ITM_Setup)
11. Write new test in tests/my_test.rs          (Test_Writing_Guide)
12. Push to GitHub (CI runs automatically)      (Test_History)
```

---

## Commands Reference

### Build
```bash
nucleus init --board NUCLEO-F411RE
nucleus check
nucleus build
```

### Flash
```bash
nucleus flash
```

### Trace (Debug Output)
```bash
openocd -f interface/stlink.cfg -f target/stm32f4x.cfg  # Terminal 1
nucleus trace --trace-tcp 127.0.0.1:3344 \
              --openocd 127.0.0.1:4444 \
              --config stm32.toml                        # Terminal 2
# Open http://localhost:5678 in browser                 # Terminal 3
```

### Test
```bash
nucleus test --list                  # Show all tests
nucleus test                          # Run all on default backend
nucleus test --backend hardware       # Hardware only
nucleus test --backend qemu           # QEMU only
nucleus test --backend both           # Sequential hardware → QEMU
nucleus test my_test_name             # Specific test
nucleus test --nocapture              # Show output in real-time
```

---

## Links

- **[Nucleus Docs](https://heyharsh.me/nucleus/)** — Full documentation
- **[GitHub](https://github.com/harshverma27/nucleus)** — Source code
- **[Demo Video](https://youtu.be/8zDZzE12wec)** — Feature overview

---

## Hardware

Tested on: **NUCLEO-F411RE** (STM32F411RE microcontroller board)

- Green LED: **PA5** (LD2)
- USART2: **PA2 (TX), PA3 (RX)** — ST-Link virtual COM port
- ST-Link: USB debugging/flashing/SWO tracing

---

## Firmware Architecture

Each project includes:

1. **stm32.toml** — Declarative hardware config (pins, peripherals, clock tree, tests, trace)
2. **src/main.c** — Application (hand-written, calls generated Nucleus_Init())
3. **src/generated/nucleus_config.h** — Generated from stm32.toml
4. **src/generated/nucleus_init.c** — Generated HAL init code
5. **tests/*.rs** — Rust test code (AgentClient SDK)

Nucleus owns only the generated files. You write main.c + tests.

---

## Backends

| Backend | Speed | Reality | Use Case |
|---------|-------|---------|----------|
| QEMU | ~1.9s | Emulation (limited GPIO model) | Fast iteration, CI testing |
| Hardware | ~2.3s | Real STM32F411 + ST-Link | Validation, release testing |
| Both | ~4.2s | Seq. hardware + QEMU | Regression, divergence detection |

---

Generated: June 2026  
PDFs: 36KB total

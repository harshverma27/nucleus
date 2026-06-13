# Vendored pin data

Source: [STMicroelectronics/STM32_open_pin_data](https://github.com/STMicroelectronics/STM32_open_pin_data)
(`master` branch, fetched 2026-06-12). Copyright STMicroelectronics; see the
upstream repository's LICENSE for terms.

| File | Upstream path | Purpose |
|---|---|---|
| `STM32F446RETx.xml` | `mcu/STM32F446R(C-E)Tx.xml` | The LQFP64 package pinout — defines which pins physically exist on the F446RE |
| `GPIO-STM32F446_gpio_v1_0_Modes.xml` | `mcu/IP/GPIO-STM32F446_gpio_v1_0_Modes.xml` | The pin ↔ AF ↔ peripheral-signal mux tables for the F446 family |
| `STM32F411RETx.xml` | `mcu/STM32F411R(C-E)Tx.xml` | The LQFP64 package pinout — defines which pins physically exist on the F411RE |
| `GPIO-STM32F411_gpio_v1_0_Modes.xml` | `mcu/IP/GPIO-STM32F411_gpio_v1_0_Modes.xml` | The pin ↔ AF ↔ peripheral-signal mux tables for the F411 family |

These files are **read-only upstream data**. Do not hand-edit them — known
anomalies are corrected by the patch table in `src/pack.rs` so every deviation
from upstream is recorded and traceable (see the Phase 1 exit criteria).

`build.rs` parses these at build time into a sorted, byte-deterministic table
embedded in the crate (see `$OUT_DIR/f446re_gen.rs` and `$OUT_DIR/f411re_gen.rs`).

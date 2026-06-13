# Quickstart: Blink an LED

This walks through setting up everything from scratch — toolchain, STM32
HAL sources, the `nucleus` CLI — and ends with a blinking LED (`LD2`, pin
`PA5`) on a **NUCLEO-F446RE**.

---

## 1. Install the ARM cross toolchain

`nucleus build` cross-compiles with `arm-none-eabi-gcc`.

**Arch Linux:**

```sh
sudo pacman -S arm-none-eabi-gcc arm-none-eabi-newlib arm-none-eabi-binutils arm-none-eabi-gdb
```

`arm-none-eabi-newlib` is required too — it provides `nano.specs`/`nosys.specs`,
which the generated build links against.

**Other platforms:** install the [ARM GNU Toolchain](https://developer.arm.com/downloads/-/arm-gnu-toolchain-downloads)
release and add its `bin/` directory to `PATH`.

Verify:

```sh
arm-none-eabi-gcc --version
```

You'll also need `cmake` and `st-flash` (from `stlink-tools`):

```sh
sudo pacman -S cmake stlink
```

---

## 2. Get the STM32CubeF4 HAL/CMSIS sources

This is **not** STM32CubeIDE/CubeMX (no GUI needed) — just a source checkout
of ST's HAL driver and CMSIS device headers.

```sh
git clone https://github.com/STMicroelectronics/STM32CubeF4 ~/STM32CubeF4
cd ~/STM32CubeF4
git submodule update --init Drivers/STM32F4xx_HAL_Driver Drivers/CMSIS/Device/ST/STM32F4xx
```

(Only those two submodules are needed — the full set includes BSPs and
middleware for boards/features this demo doesn't use.)

Verify the key headers exist:

```sh
ls Drivers/STM32F4xx_HAL_Driver/Inc/stm32f4xx_hal.h
ls Drivers/CMSIS/Device/ST/STM32F4xx/Include/stm32f446xx.h
```

Export the path (add this to your `~/.bashrc`/`~/.zshrc` so it persists
across shells):

```sh
export STM32CUBE_PATH=~/STM32CubeF4
```

---

## 3. Install `nucleus`

From a clone of this repo:

```sh
git clone https://github.com/harshverma27/nucleus
cd nucleus
cargo install --path crates/nucleus-cli --locked
```

Verify:

```sh
nucleus --version
nucleus --help
```

---

## 4. Scaffold a new project

```sh
mkdir ~/my-blink && cd ~/my-blink
nucleus init .
```

This creates `stm32.toml`, `CMakeLists.txt`, a linker script, HAL config
header, interrupt-handler stubs, and a starter `src/main.c` that calls the
generated `Nucleus_Init()`. The default `stm32.toml` configures `USART2`
(the ST-Link virtual COM port) — nothing else is required for this demo.

Validate the config:

```sh
nucleus check
```

---

## 5. Add the LED blink code

`LD2` (the on-board green LED) is wired to **PA5** on the NUCLEO-F446RE.
Plain GPIO toggling isn't part of `stm32.toml`'s declarative peripheral
model (that's reserved for pin-muxed peripherals like USART/SPI/I2C/TIM),
so it's hand-written in `main.c` — same as any bare-metal HAL project.

Edit `src/main.c`:

```c
/* Application entry point. Hand-written — Nucleus only owns nucleus_init.c. */
#include "stm32f4xx_hal.h"
#include "generated/nucleus_config.h"

void SystemClock_Config(void);

int main(void)
{
    HAL_Init();
    SystemClock_Config();
    Nucleus_Init();          /* generated from stm32.toml */

    __HAL_RCC_GPIOA_CLK_ENABLE();

    GPIO_InitTypeDef led = {0};
    led.Pin   = GPIO_PIN_5;
    led.Mode  = GPIO_MODE_OUTPUT_PP;
    led.Pull  = GPIO_NOPULL;
    led.Speed = GPIO_SPEED_FREQ_LOW;
    HAL_GPIO_Init(GPIOA, &led);

    while (1) {
        HAL_GPIO_TogglePin(GPIOA, GPIO_PIN_5);
        HAL_Delay(500);
    }
}

/* Replace with a clock setup matching [device].clock_hz in stm32.toml.
 * A full clock-tree solver is intentionally out of Nucleus's scope. */
__attribute__((weak)) void SystemClock_Config(void) {}
```

---

## 6. Build

```sh
nucleus build
```

This validates `stm32.toml`, generates `src/generated/nucleus_config.h` and
`nucleus_init.c`, then drives CMake + `arm-none-eabi-gcc`. On success you'll
see:

```
Generating firmware.bin / firmware.hex
[100%] Built target firmware
build OK — firmware in ./build
```

(A few `_close`/`_read`/`_write is not implemented` linker warnings are
expected and harmless — they come from `-specs=nosys.specs`.)

---

## 7. Flash

Plug the NUCLEO board in via the **ST-Link USB port** (the one nearer the
edge of the board, not the "USB User" port). Then:

```sh
nucleus flash
```

This programs `build/firmware.bin` at `0x08000000` via `st-flash` and resets
the board. `LD2` should start blinking at ~1 Hz.

---

## Troubleshooting

- **`st-flash: Failed to enter SWD mode` / `Failed to connect to target`**
  but `st-info --probe` finds the programmer: the ST-Link itself is detected,
  but the SWD link to the target MCU isn't up. Unplug/replug the USB cable,
  or press the board's black **RESET** button, then retry.

- **`st-info --probe` reports an unexpected `chipid`/`dev-type`**: confirm
  your board is actually a NUCLEO-**F446RE** (chip ID `0x421`). Nucleus is
  built and validated against the F446RE only — other F4 boards (e.g. an
  F411RE, chip ID `0x431`) often *happen* to work for simple GPIO demos since
  the F4 family shares register layouts, but pin/AF validation (`nucleus
  check`) and any clock-tree setup are F446-specific.

- **Missing `stm32f4xx_hal_conf.h` / undefined HAL functions at link time**:
  make sure you're on a `nucleus` build that includes the scaffold fix
  (`nucleus init` should create `src/stm32f4xx_hal_conf.h`, `src/stm32f4xx_it.c`,
  and `STM32F446RETx_FLASH.ld`). Re-run `cargo install --path crates/nucleus-cli
  --locked --force` from an updated checkout if these files are missing.

- **`STM32CUBE_PATH` errors** (`stm32f4xx_hal.h: No such file or directory`):
  confirm `echo $STM32CUBE_PATH` points at your STM32CubeF4 checkout and that
  the submodules from step 2 are initialized (their directories aren't empty).

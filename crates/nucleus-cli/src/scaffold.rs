//! Project scaffolding for `nucleus init`.
//!
//! Writes a minimal but complete STM32 project: a starter `stm32.toml`, a CMake
//! build that cross-compiles with `arm-none-eabi-gcc`, a `main.c` that calls the
//! generated `Nucleus_Init()`, and a CI workflow that runs `nucleus check`.
//!
//! Existing files are never overwritten — `init` is safe to re-run in a
//! populated directory and only fills in what's missing.

use std::path::Path;

/// One file the scaffolder can emit: a path relative to the project root and
/// its contents.
struct Template {
    path: &'static str,
    contents: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        path: "stm32.toml",
        contents: STM32_TOML,
    },
    Template {
        path: "CMakeLists.txt",
        contents: CMAKELISTS,
    },
    Template {
        path: "cmake/arm-none-eabi-gcc.cmake",
        contents: TOOLCHAIN_CMAKE,
    },
    Template {
        path: "STM32F446RETx_FLASH.ld",
        contents: LINKER_SCRIPT,
    },
    Template {
        path: "src/main.c",
        contents: MAIN_C,
    },
    Template {
        path: "src/stm32f4xx_hal_conf.h",
        contents: HAL_CONF_H,
    },
    Template {
        path: "src/stm32f4xx_it.h",
        contents: STM32F4XX_IT_H,
    },
    Template {
        path: "src/stm32f4xx_it.c",
        contents: STM32F4XX_IT_C,
    },
    Template {
        path: ".github/workflows/ci.yml",
        contents: CI_YML,
    },
    Template {
        path: ".gitignore",
        contents: GITIGNORE,
    },
];

/// Outcome of scaffolding one file.
pub enum Written {
    Created(String),
    Skipped(String),
}

/// Scaffold a project under `root`, creating missing files and skipping any
/// that already exist. Returns one [`Written`] per template.
pub fn scaffold(root: &Path) -> std::io::Result<Vec<Written>> {
    let mut results = Vec::with_capacity(TEMPLATES.len());
    for tpl in TEMPLATES {
        let dest = root.join(tpl.path);
        if dest.exists() {
            results.push(Written::Skipped(tpl.path.to_string()));
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, tpl.contents)?;
        results.push(Written::Created(tpl.path.to_string()));
    }
    Ok(results)
}

const STM32_TOML: &str = r#"# Nucleus project configuration. Validate with `nucleus check`.
[device]
family   = "STM32F446RE"
board    = "NUCLEO-F446RE"
clock_hz = 180_000_000

[build]
toolchain    = "arm-none-eabi-gcc"
optimization = "Os"

[peripherals.usart2]   # ST-Link virtual COM port on the NUCLEO-F446RE
tx   = "PA2"
rx   = "PA3"
baud = 115200
"#;

const MAIN_C: &str = r#"/* Application entry point. Hand-written — Nucleus only owns nucleus_init.c. */
#include "stm32f4xx_hal.h"
#include "generated/nucleus_config.h"

void SystemClock_Config(void);

int main(void)
{
    HAL_Init();
    SystemClock_Config();
    Nucleus_Init();          /* generated from stm32.toml */

    while (1) {
        /* your application code */
    }
}

/* Replace with a clock setup matching [device].clock_hz in stm32.toml.
 * A full clock-tree solver is intentionally out of Nucleus's scope. */
__attribute__((weak)) void SystemClock_Config(void) {}
"#;

const CMAKELISTS: &str = r#"cmake_minimum_required(VERSION 3.20)

# Cross-compilation toolchain (must be set before project()).
set(CMAKE_TOOLCHAIN_FILE ${CMAKE_CURRENT_SOURCE_DIR}/cmake/arm-none-eabi-gcc.cmake)

project(firmware C ASM)

# Point this at a checkout of STMicroelectronics/STM32CubeF4 (HAL + CMSIS).
set(STM32CUBE_PATH "$ENV{STM32CUBE_PATH}" CACHE PATH "Path to STM32CubeF4")

set(HAL_DRIVER_SRC ${STM32CUBE_PATH}/Drivers/STM32F4xx_HAL_Driver/Src)
set(CMSIS_TEMPLATES ${STM32CUBE_PATH}/Drivers/CMSIS/Device/ST/STM32F4xx/Source/Templates)

set(MCU_FLAGS -mcpu=cortex-m4 -mfpu=fpv4-sp-d16 -mfloat-abi=hard -mthumb)
add_compile_options(${MCU_FLAGS} -ffunction-sections -fdata-sections -Wall)
add_link_options(${MCU_FLAGS} -Wl,--gc-sections -specs=nano.specs -specs=nosys.specs
    -T${CMAKE_CURRENT_SOURCE_DIR}/STM32F446RETx_FLASH.ld
    -Wl,-Map=firmware.map
)

add_compile_definitions(STM32F446xx USE_HAL_DRIVER)

add_executable(firmware
    src/main.c
    src/stm32f4xx_it.c
    src/generated/nucleus_init.c

    # CMSIS startup + system init
    ${CMSIS_TEMPLATES}/gcc/startup_stm32f446xx.s
    ${CMSIS_TEMPLATES}/system_stm32f4xx.c

    # HAL driver core + the peripheral families Nucleus supports
    # (USART, SPI, I2C, TIM) — unused code is stripped by --gc-sections.
    ${HAL_DRIVER_SRC}/stm32f4xx_hal.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_rcc.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_rcc_ex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_gpio.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_exti.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_cortex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_pwr.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_pwr_ex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_flash.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_flash_ex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_dma.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_dma_ex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_uart.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_spi.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_i2c.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_i2c_ex.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_tim.c
    ${HAL_DRIVER_SRC}/stm32f4xx_hal_tim_ex.c
)

target_include_directories(firmware PRIVATE
    src
    ${STM32CUBE_PATH}/Drivers/STM32F4xx_HAL_Driver/Inc
    ${STM32CUBE_PATH}/Drivers/CMSIS/Device/ST/STM32F4xx/Include
    ${STM32CUBE_PATH}/Drivers/CMSIS/Include
)

# Emit a raw binary alongside the .elf for flashing.
add_custom_command(TARGET firmware POST_BUILD
    COMMAND ${CMAKE_OBJCOPY} -O binary $<TARGET_FILE:firmware> firmware.bin
    COMMAND ${CMAKE_OBJCOPY} -O ihex   $<TARGET_FILE:firmware> firmware.hex
    COMMENT "Generating firmware.bin / firmware.hex"
)
"#;

const LINKER_SCRIPT: &str = r#"/* Linker script for STM32F446RETx: 512K flash, 128K RAM. */
ENTRY(Reset_Handler)

_estack = ORIGIN(RAM) + LENGTH(RAM);

_Min_Heap_Size  = 0x200;
_Min_Stack_Size = 0x400;

MEMORY
{
  RAM   (xrw) : ORIGIN = 0x20000000, LENGTH = 128K
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 512K
}

SECTIONS
{
  .isr_vector :
  {
    . = ALIGN(4);
    KEEP(*(.isr_vector))
    . = ALIGN(4);
  } >FLASH

  .text :
  {
    . = ALIGN(4);
    *(.text)
    *(.text*)
    *(.glue_7)
    *(.glue_7t)
    *(.eh_frame)
    KEEP (*(.init))
    KEEP (*(.fini))
    . = ALIGN(4);
    _etext = .;
  } >FLASH

  .rodata :
  {
    . = ALIGN(4);
    *(.rodata)
    *(.rodata*)
    . = ALIGN(4);
  } >FLASH

  .ARM.extab : { *(.ARM.extab* .gnu.linkonce.armextab.*) } >FLASH
  .ARM :
  {
    __exidx_start = .;
    *(.ARM.exidx*)
    __exidx_end = .;
  } >FLASH

  .preinit_array :
  {
    PROVIDE_HIDDEN (__preinit_array_start = .);
    KEEP (*(.preinit_array*))
    PROVIDE_HIDDEN (__preinit_array_end = .);
  } >FLASH

  .init_array :
  {
    PROVIDE_HIDDEN (__init_array_start = .);
    KEEP (*(SORT(.init_array.*)))
    KEEP (*(.init_array*))
    PROVIDE_HIDDEN (__init_array_end = .);
  } >FLASH

  .fini_array :
  {
    PROVIDE_HIDDEN (__fini_array_start = .);
    KEEP (*(SORT(.fini_array.*)))
    KEEP (*(.fini_array*))
    PROVIDE_HIDDEN (__fini_array_end = .);
  } >FLASH

  _sidata = LOADADDR(.data);

  .data :
  {
    . = ALIGN(4);
    _sdata = .;
    *(.data)
    *(.data*)
    . = ALIGN(4);
    _edata = .;
  } >RAM AT> FLASH

  .bss :
  {
    . = ALIGN(4);
    _sbss = .;
    *(.bss)
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    _ebss = .;
  } >RAM

  ._user_heap_stack :
  {
    . = ALIGN(8);
    PROVIDE ( end = . );
    PROVIDE ( _end = . );
    . = . + _Min_Heap_Size;
    . = . + _Min_Stack_Size;
    . = ALIGN(8);
  } >RAM

  /DISCARD/ :
  {
    libc.a ( * )
    libm.a ( * )
    libgcc.a ( * )
  }

  .ARM.attributes 0 : { *(.ARM.attributes) }
}
"#;

const HAL_CONF_H: &str = r#"/* HAL configuration. Hand-written — selects the HAL modules Nucleus uses
 * and the board's oscillator values (NUCLEO-F446RE: 8 MHz HSE from ST-Link). */
#ifndef NUCLEUS_HAL_CONF_H
#define NUCLEUS_HAL_CONF_H

#ifdef __cplusplus
extern "C" {
#endif

#define HAL_MODULE_ENABLED
#define HAL_RCC_MODULE_ENABLED
#define HAL_GPIO_MODULE_ENABLED
#define HAL_EXTI_MODULE_ENABLED
#define HAL_DMA_MODULE_ENABLED
#define HAL_CORTEX_MODULE_ENABLED
#define HAL_PWR_MODULE_ENABLED
#define HAL_FLASH_MODULE_ENABLED
#define HAL_UART_MODULE_ENABLED
#define HAL_SPI_MODULE_ENABLED
#define HAL_I2C_MODULE_ENABLED
#define HAL_TIM_MODULE_ENABLED

#if !defined(HSE_VALUE)
#define HSE_VALUE    8000000U
#endif

#if !defined(HSE_STARTUP_TIMEOUT)
#define HSE_STARTUP_TIMEOUT 100U
#endif

#if !defined(LSE_STARTUP_TIMEOUT)
#define LSE_STARTUP_TIMEOUT 5000U
#endif

#if !defined(HSI_VALUE)
#define HSI_VALUE    16000000U
#endif

#if !defined(LSI_VALUE)
#define LSI_VALUE    32000U
#endif

#if !defined(LSE_VALUE)
#define LSE_VALUE    32768U
#endif

#if !defined(EXTERNAL_CLOCK_VALUE)
#define EXTERNAL_CLOCK_VALUE 12288000U
#endif

#define VDD_VALUE             3300U
#define TICK_INT_PRIORITY     0x0FU
#define USE_RTOS              0U
#define PREFETCH_ENABLE       1U
#define INSTRUCTION_CACHE_ENABLE 1U
#define DATA_CACHE_ENABLE     1U

#ifdef USE_FULL_ASSERT
#define assert_param(expr) ((expr) ? (void)0U : assert_failed((uint8_t *)__FILE__, __LINE__))
void assert_failed(uint8_t *file, uint32_t line);
#else
#define assert_param(expr) ((void)0U)
#endif

#include "stm32f4xx_hal_rcc.h"
#include "stm32f4xx_hal_gpio.h"
#include "stm32f4xx_hal_exti.h"
#include "stm32f4xx_hal_dma.h"
#include "stm32f4xx_hal_cortex.h"
#include "stm32f4xx_hal_flash.h"
#include "stm32f4xx_hal_pwr.h"
#include "stm32f4xx_hal_uart.h"
#include "stm32f4xx_hal_spi.h"
#include "stm32f4xx_hal_i2c.h"
#include "stm32f4xx_hal_tim.h"

#ifdef __cplusplus
}
#endif

#endif /* NUCLEUS_HAL_CONF_H */
"#;

const STM32F4XX_IT_H: &str = r#"/* Interrupt handler declarations. Hand-written. */
#ifndef STM32F4XX_IT_H
#define STM32F4XX_IT_H

#ifdef __cplusplus
extern "C" {
#endif

void NMI_Handler(void);
void HardFault_Handler(void);
void MemManage_Handler(void);
void BusFault_Handler(void);
void UsageFault_Handler(void);
void SVC_Handler(void);
void DebugMon_Handler(void);
void PendSV_Handler(void);
void SysTick_Handler(void);

#ifdef __cplusplus
}
#endif

#endif /* STM32F4XX_IT_H */
"#;

const STM32F4XX_IT_C: &str = r#"/* Core interrupt handlers. Hand-written — add peripheral IRQ handlers here
 * as your application needs them. */
#include "stm32f4xx_hal.h"
#include "stm32f4xx_it.h"

void NMI_Handler(void)
{
    while (1) { }
}

void HardFault_Handler(void)
{
    while (1) { }
}

void MemManage_Handler(void)
{
    while (1) { }
}

void BusFault_Handler(void)
{
    while (1) { }
}

void UsageFault_Handler(void)
{
    while (1) { }
}

void SVC_Handler(void)
{
}

void DebugMon_Handler(void)
{
}

void PendSV_Handler(void)
{
}

void SysTick_Handler(void)
{
    HAL_IncTick();
}
"#;

const TOOLCHAIN_CMAKE: &str = r#"# arm-none-eabi-gcc cross toolchain for STM32.
set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR arm)

set(TOOLCHAIN_PREFIX arm-none-eabi-)
set(CMAKE_C_COMPILER   ${TOOLCHAIN_PREFIX}gcc)
set(CMAKE_ASM_COMPILER ${TOOLCHAIN_PREFIX}gcc)
set(CMAKE_OBJCOPY      ${TOOLCHAIN_PREFIX}objcopy)
set(CMAKE_SIZE         ${TOOLCHAIN_PREFIX}size)

# Don't try to run target binaries on the host during compiler checks.
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
"#;

const CI_YML: &str = r#"name: ci
on: [push, pull_request]

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Nucleus
        run: cargo install nucleus-cli
      - name: Validate stm32.toml
        run: nucleus check
"#;

const GITIGNORE: &str = "/build/\n/src/generated/\n*.elf\n*.bin\n*.hex\n*.map\n";

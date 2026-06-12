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
        path: "src/main.c",
        contents: MAIN_C,
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

set(MCU_FLAGS -mcpu=cortex-m4 -mfpu=fpv4-sp-d16 -mfloat-abi=hard -mthumb)
add_compile_options(${MCU_FLAGS} -ffunction-sections -fdata-sections -Wall)
add_link_options(${MCU_FLAGS} -Wl,--gc-sections -specs=nano.specs -specs=nosys.specs)

add_compile_definitions(STM32F446xx USE_HAL_DRIVER)

add_executable(firmware
    src/main.c
    src/generated/nucleus_init.c
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

const GITIGNORE: &str = "/build/\n/src/generated/\n*.elf\n*.bin\n*.hex\n";

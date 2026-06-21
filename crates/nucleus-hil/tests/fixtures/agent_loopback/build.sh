#!/bin/sh
# Rebuilds the checked-in agent_loopback_{qemu,hw}.{elf,bin} fixtures from
# source. Run by hand when the fixture needs to change — CI never invokes this
# (it has no arm-none-eabi-gcc), it just uses the committed binaries.
set -e
cd "$(dirname "$0")"

CC=arm-none-eabi-gcc
CFLAGS="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -nostartfiles -ffreestanding -O2 -Wall -Wextra"

$CC $CFLAGS -T link.ld startup.s agent.c -o agent_loopback_hw.elf
$CC $CFLAGS -DHIL_QEMU_SEMIHOSTING_ITM -T link.ld startup.s agent.c -o agent_loopback_qemu.elf

arm-none-eabi-objcopy -O binary agent_loopback_hw.elf agent_loopback_hw.bin
arm-none-eabi-objcopy -O binary agent_loopback_qemu.elf agent_loopback_qemu.bin

echo "built agent_loopback_hw.{elf,bin} and agent_loopback_qemu.{elf,bin}"

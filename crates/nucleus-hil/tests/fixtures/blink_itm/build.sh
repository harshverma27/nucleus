#!/bin/sh
# Rebuilds the checked-in blink_itm_{qemu,hw}.{elf,bin} fixtures from source.
# Run by hand when the fixture needs to change — CI never invokes this (it
# has no arm-none-eabi-gcc), it just uses the committed binaries.
set -e
cd "$(dirname "$0")"

CC=arm-none-eabi-gcc
CFLAGS="-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -nostartfiles -ffreestanding -O2 -Wall -Wextra"

$CC $CFLAGS -T link.ld startup.s blink_itm.c -o blink_itm_hw.elf
$CC $CFLAGS -DHIL_QEMU_SEMIHOSTING_ITM -T link.ld startup.s blink_itm.c -o blink_itm_qemu.elf

arm-none-eabi-objcopy -O binary blink_itm_hw.elf blink_itm_hw.bin
arm-none-eabi-objcopy -O binary blink_itm_qemu.elf blink_itm_qemu.bin

echo "built blink_itm_hw.{elf,bin} and blink_itm_qemu.{elf,bin}"

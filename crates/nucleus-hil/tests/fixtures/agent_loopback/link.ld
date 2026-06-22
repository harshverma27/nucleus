/* Linker script for the agent_loopback fixture: STM32F4-family memory map
 * (flash at 0x08000000, SRAM at 0x20000000), no CMSIS startup files.
 *
 * The `.nucleus_agent` section is placed FIRST in RAM so the mailbox struct
 * lands exactly at 0x20000000 — the host SDK's MAILBOX_BASE constant
 * (nucleus-test-sdk/src/protocol.rs) finds it with no ELF symbol lookup.
 * NOLOAD: it's runtime scratch, not initialized from flash. */
MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 128K
}

_estack = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .isr_vector :
  {
    KEEP(*(.isr_vector))
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /* Mailbox lives at the very start of SRAM (0x20000000). */
  .nucleus_agent (NOLOAD) : ALIGN(4)
  {
    KEEP(*(.nucleus_agent))
  } > RAM

  .data :
  {
    *(.data*)
  } > RAM AT> FLASH

  .bss :
  {
    *(.bss*)
  } > RAM
}

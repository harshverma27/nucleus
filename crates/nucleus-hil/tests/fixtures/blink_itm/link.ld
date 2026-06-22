/* Minimal linker script for the blink_itm fixture: STM32F4-family memory
 * map (flash at 0x08000000, SRAM at 0x20000000), no CMSIS startup files. */
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

  .data :
  {
    *(.data*)
  } > RAM AT> FLASH

  .bss :
  {
    *(.bss*)
  } > RAM
}

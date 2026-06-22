@ Minimal Cortex-M4 vector table + reset entry for the blink_itm fixture.
@ No CMSIS, no HAL — this firmware exists purely to prove the M5 backends'
@ observation API against real silicon behavior (PA5 toggle + one ITM log).
.syntax unified
.cpu cortex-m4
.thumb

.section .isr_vector, "a"
.word _estack
.word Reset_Handler
.word Default_Handler   @ NMI
.word Default_Handler   @ HardFault
.word Default_Handler   @ MemManage
.word Default_Handler   @ BusFault
.word Default_Handler   @ UsageFault
.word 0
.word 0
.word 0
.word 0
.word Default_Handler   @ SVCall
.word Default_Handler   @ DebugMon
.word 0
.word Default_Handler   @ PendSV
.word Default_Handler   @ SysTick

.section .text

.thumb_func
Default_Handler:
    b .

.thumb_func
.global Reset_Handler
Reset_Handler:
    bl main
    b .

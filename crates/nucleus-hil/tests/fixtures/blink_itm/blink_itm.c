/* The M5 exit-criterion fixture: toggle PA5 and emit one ITM log, nothing
 * else. No HAL, no CMSIS — direct register pokes only, since this firmware
 * exists to exercise the HIL backends, not to be a real application.
 *
 * Built twice from this one source (see build.sh): `HIL_QEMU_SEMIHOSTING_ITM`
 * defined picks the QEMU variant (no TPIU/ITM block in QEMU's STM32F4 model,
 * so the "ITM log" goes out over semihosting, pre-encoded as the exact bytes
 * a real ITM stimulus-port write would have produced — see
 * src/qemu/itm.rs's doc comment for why); undefined picks the real-hardware
 * variant (writes the real ITM stimulus port register).
 */
#include <stdint.h>

#define RCC_AHB1ENR (*(volatile uint32_t *)0x40023830)
#define RCC_APB1ENR (*(volatile uint32_t *)0x40023840)
#define GPIOA_MODER (*(volatile uint32_t *)0x40020000)
#define GPIOA_ODR (*(volatile uint32_t *)0x40020014)
#define TIM2_CR1 (*(volatile uint32_t *)0x40000000)

static void delay(volatile uint32_t n) {
    while (n--) {
        __asm volatile("nop");
    }
}

#ifdef HIL_QEMU_SEMIHOSTING_ITM

static void semihost_writec(uint8_t c) {
    /* Explicit hardware-register bindings, not just storage-class hints —
     * without `asm("r0")`/`asm("r1")` the compiler doesn't actually pin
     * these to the registers the semihosting ABI reads, and silently
     * miscompiles which value reaches the trap. */
    register int r0 asm("r0") = 0x03; /* SYS_WRITEC */
    register int r1 asm("r1") = (int)&c;
    __asm volatile("bkpt 0xAB" : "+r"(r0), "+r"(r1)::"memory");
}

/* Pre-encoded ITM SWIT packets for stimulus port 0, payload "OK" — the exact
 * wire bytes nucleus_itm::Decoder expects (header (port<<3)|0b001, then the
 * 1-byte payload, per ARMv7-M DDI 0403E §C1.10). */
static void emit_itm_log(void) {
    uint8_t bytes[4] = {0x01, 'O', 0x01, 'K'};
    for (int i = 0; i < 4; i++) {
        semihost_writec(bytes[i]);
    }
}

#else

/* Packet size on the wire follows the bus-access width of the store to the
 * stimulus port (ARMv7-M DDI 0403E §C1.10) — a uint32_t* store here would
 * emit 4-byte SWIT packets even for a 1-byte char, confirmed empirically
 * against real hardware (the QEMU semihosting-encoded variant above only
 * ever produces the 1-byte form, so this divergence is invisible there).
 * uint8_t* keeps both backends' wire format identical. */
#define ITM_STIM0 (*(volatile uint8_t *)0xE0000000)
#define ITM_TER0 (*(volatile uint32_t *)0xE0000E00)
#define ITM_TCR (*(volatile uint32_t *)0xE0000E80)
#define DEMCR (*(volatile uint32_t *)0xE000EDFC)
#define DBGMCU_CR (*(volatile uint32_t *)0xE0042004)
#define GPIOB_MODER (*(volatile uint32_t *)0x40020400)
#define GPIOB_AFRL (*(volatile uint32_t *)0x40020420)

/* SWO only reaches the ST-Link on real hardware once PB3 is muxed to its
 * TRACESWO alternate function (AF0) and DBGMCU.CR.TRACE_IOEN is set — both
 * default off after reset (shared as a plain GPIO otherwise). QEMU's STM32F4
 * model has no such pin-mux/DBGMCU gating, so this requirement only ever
 * surfaces against real silicon: confirmed empirically — without it the
 * OpenOCD SWO TCP stream stays bytes-zero even with ITM_TER0/TCR set. */
static void enable_swo_pin(void) {
    RCC_AHB1ENR |= (1u << 1); /* GPIOBEN */
    GPIOB_MODER &= ~(0x3u << 6);
    GPIOB_MODER |= (0x2u << 6); /* PB3 = alternate function (10) */
    GPIOB_AFRL &= ~(0xFu << 12); /* PB3 AF0 = TRACESWO */
    DBGMCU_CR |= (1u << 5);      /* TRACE_IOEN */
}

static void emit_itm_log(void) {
    enable_swo_pin();
    DEMCR |= (1u << 24);  /* TRCENA */
    ITM_TCR |= 1u;        /* ITMENA */
    ITM_TER0 |= 1u;       /* enable stimulus port 0 */
    ITM_STIM0 = (uint32_t)'O';
    ITM_STIM0 = (uint32_t)'K';
}

#endif

int main(void) {
    RCC_AHB1ENR |= (1u << 0);  /* GPIOAEN */
    GPIOA_MODER |= (1u << 10); /* PA5 = general-purpose output (01) */
    GPIOA_MODER &= ~(1u << 11);

    /* TIM2 is a real QEMU model on netduinoplus2 (GPIO is not — see
     * src/qemu/mod.rs's doc comment); free-run it so the QEMU backend has a
     * genuinely changing register to observe over a sample window. */
    RCC_APB1ENR |= (1u << 0); /* TIM2EN */
    TIM2_CR1 |= 1u;           /* CEN: counter enable */

    emit_itm_log();

    while (1) {
        GPIOA_ODR ^= (1u << 5);
        delay(200000);
    }
}

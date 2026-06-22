/* The M7 device test-agent firmware: the *device half* of the RAM-mailbox
 * loopback protocol. The host SDK (nucleus-test-sdk) writes a command into a
 * mailbox struct pinned at 0x20000000 and sets status=BUSY; this firmware
 * polls that flag, executes the command (GPIO/UART/register access), writes a
 * response, and sets status=DONE/ERR. No HAL, no CMSIS — direct register
 * pokes only.
 *
 * The mailbox layout, magic, status values, command ids, and RX sentinel
 * mirror nucleus-test-sdk/src/protocol.rs BYTE-FOR-BYTE. Mismatched offsets or
 * ids silently break the loopback, so keep these in lockstep with that file.
 *
 * Built twice from this one source (see build.sh): `HIL_QEMU_SEMIHOSTING_ITM`
 * defined picks the QEMU variant (no TPIU/ITM block in QEMU's STM32F4 model,
 * so the "agent_ready" ITM marker goes out over semihosting, pre-encoded as
 * the exact bytes a real ITM stimulus-port write would have produced);
 * undefined picks the real-hardware variant (real ITM stimulus port + SWO pin
 * setup).
 */
#include <stdint.h>

/* --- protocol constants (mirror protocol.rs exactly) --------------------- */
#define MAGIC 0x4E544167u
#define VERSION 1u

#define ST_IDLE 0u
#define ST_BUSY 1u
#define ST_DONE 2u
#define ST_ERR 3u

#define CMD_PING 0u
#define CMD_SET_GPIO 1u
#define CMD_READ_GPIO 2u
#define CMD_READ_REG 3u
#define CMD_UART_TX 4u
#define CMD_UART_RX_POLL 5u

#define RX_NONE 0xFFFFFFFFu

/* --- mailbox struct, pinned at 0x20000000 by the linker ------------------ */
/* Field order must match protocol.rs OFF_* offsets:
 * magic@0x00 version@0x04 seq@0x08 cmd@0x0C arg0@0x10 arg1@0x14
 * status@0x18 resp@0x1C. */
typedef struct {
    volatile uint32_t magic;
    volatile uint32_t version;
    volatile uint32_t seq;
    volatile uint32_t cmd;
    volatile uint32_t arg0;
    volatile uint32_t arg1;
    volatile uint32_t status;
    volatile uint32_t resp;
} mailbox_t;

__attribute__((section(".nucleus_agent"), used)) mailbox_t g_mbox;

/* --- STM32F411RE register definitions ----------------------------------- */
#define RCC_AHB1ENR (*(volatile uint32_t *)0x40023830)
#define RCC_APB1ENR (*(volatile uint32_t *)0x40023840)

#define GPIOA_BASE 0x40020000u
#define GPIO_STRIDE 0x400u
/* GPIO register offsets */
#define GPIO_MODER 0x00u
#define GPIO_OTYPER 0x04u
#define GPIO_OSPEEDR 0x08u
#define GPIO_PUPDR 0x0Cu
#define GPIO_IDR 0x10u
#define GPIO_ODR 0x14u
#define GPIO_BSRR 0x18u
#define GPIO_AFRL 0x20u

/* USART2 (APB1) */
#define USART2_BASE 0x40004400u
#define USART2_SR (*(volatile uint32_t *)(USART2_BASE + 0x00u))
#define USART2_DR (*(volatile uint32_t *)(USART2_BASE + 0x04u))
#define USART2_BRR (*(volatile uint32_t *)(USART2_BASE + 0x08u))
#define USART2_CR1 (*(volatile uint32_t *)(USART2_BASE + 0x0Cu))

#define USART_SR_RXNE (1u << 5)
#define USART_SR_TXE (1u << 7)
#define USART_CR1_RE (1u << 2)
#define USART_CR1_TE (1u << 3)
#define USART_CR1_UE (1u << 13)

static inline volatile uint32_t *gpio_reg(uint32_t port_index, uint32_t off) {
    return (volatile uint32_t *)(GPIOA_BASE + port_index * GPIO_STRIDE + off);
}

/* --- ITM "agent_ready" marker (reused verbatim from blink_itm.c) --------- */
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

/* ITM SWIT packets for stimulus port 0, one header+payload byte pair per
 * character (header (port<<3)|0b001 = 0x01, then the 1-byte payload), per
 * ARMv7-M DDI 0403E §C1.10 — the exact wire bytes nucleus_itm::Decoder
 * expects. */
static void itm_emit(const char *s) {
    while (*s) {
        semihost_writec(0x01);
        semihost_writec((uint8_t)*s);
        s++;
    }
}

#else

/* Packet size on the wire follows the bus-access width of the store to the
 * stimulus port (ARMv7-M DDI 0403E §C1.10) — a uint32_t* store would emit
 * 4-byte SWIT packets even for a 1-byte char, confirmed empirically against
 * real hardware. uint8_t* keeps both backends' wire format identical. */
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
 * surfaces against real silicon. */
static void enable_swo_pin(void) {
    RCC_AHB1ENR |= (1u << 1); /* GPIOBEN */
    GPIOB_MODER &= ~(0x3u << 6);
    GPIOB_MODER |= (0x2u << 6);  /* PB3 = alternate function (10) */
    GPIOB_AFRL &= ~(0xFu << 12); /* PB3 AF0 = TRACESWO */
    DBGMCU_CR |= (1u << 5);      /* TRACE_IOEN */
}

static void itm_init(void) {
    enable_swo_pin();
    DEMCR |= (1u << 24); /* TRCENA */
    ITM_TCR |= 1u;       /* ITMENA */
    ITM_TER0 |= 1u;      /* enable stimulus port 0 */
}

static void itm_emit(const char *s) {
    while (*s) {
        ITM_STIM0 = (uint8_t)*s;
        s++;
    }
}

#endif

/* --- USART2 ------------------------------------------------------------- */
/* PA2 = TX (AF7), PA3 = RX (AF7), 115200 8N1.
 * After reset SYSCLK = HSI = 16MHz, APB1 PCLK = 16MHz, so
 * USARTDIV = 16000000 / 115200 ≈ 139 = 0x8B (oversampling by 16). */
static void usart2_init(void) {
    RCC_AHB1ENR |= (1u << 0); /* GPIOAEN */
    RCC_APB1ENR |= (1u << 17); /* USART2EN */

    /* PA2, PA3 -> alternate function mode (10) */
    volatile uint32_t *moder = gpio_reg(0, GPIO_MODER);
    *moder &= ~((0x3u << (2 * 2)) | (0x3u << (3 * 2)));
    *moder |= (0x2u << (2 * 2)) | (0x2u << (3 * 2));

    /* PA2, PA3 -> AF7 (USART2) in AFRL (pins 0-7, 4 bits each) */
    volatile uint32_t *afrl = gpio_reg(0, GPIO_AFRL);
    *afrl &= ~((0xFu << (2 * 4)) | (0xFu << (3 * 4)));
    *afrl |= (0x7u << (2 * 4)) | (0x7u << (3 * 4));

    USART2_BRR = 0x8Bu; /* 115200 @ 16MHz PCLK */
    USART2_CR1 = USART_CR1_TE | USART_CR1_RE | USART_CR1_UE;
}

static void usart2_tx_byte(uint8_t b) {
    while (!(USART2_SR & USART_SR_TXE)) {
    }
    USART2_DR = b;
}

static int usart2_rx_ready(void) { return (USART2_SR & USART_SR_RXNE) != 0; }

static uint8_t usart2_rx_byte(void) { return (uint8_t)(USART2_DR & 0xFFu); }

/* --- GPIO command helpers ----------------------------------------------- */
/* enc = (port_index << 8) | pin, port A=0,B=1,...,H=7. */
static void gpio_write(uint32_t enc, uint32_t level) {
    uint32_t port = (enc >> 8) & 0xFFu;
    uint32_t pin = enc & 0xFFu;

    /* Configure pin as general-purpose output (MODER = 01) before driving, so
     * the loopback can set a pin it has not separately initialized. */
    volatile uint32_t *moder = gpio_reg(port, GPIO_MODER);
    *moder &= ~(0x3u << (pin * 2));
    *moder |= (0x1u << (pin * 2));

    volatile uint32_t *bsrr = gpio_reg(port, GPIO_BSRR);
    if (level) {
        *bsrr = (1u << pin); /* set */
    } else {
        *bsrr = (1u << (pin + 16)); /* reset */
    }
}

static uint32_t gpio_read(uint32_t enc) {
    uint32_t port = (enc >> 8) & 0xFFu;
    uint32_t pin = enc & 0xFFu;
    volatile uint32_t *idr = gpio_reg(port, GPIO_IDR);
    return (*idr >> pin) & 0x1u;
}

int main(void) {
#ifndef HIL_QEMU_SEMIHOSTING_ITM
    itm_init();
#endif
    usart2_init();

    /* Publish the mailbox in a host-safe order: clear/init everything else,
     * set status IDLE, then write magic LAST — the host waits on magic before
     * trusting any other field. */
    g_mbox.version = VERSION;
    g_mbox.seq = 0;
    g_mbox.cmd = 0;
    g_mbox.arg0 = 0;
    g_mbox.arg1 = 0;
    g_mbox.resp = 0;
    g_mbox.status = ST_IDLE;
    g_mbox.magic = MAGIC;

    itm_emit("agent_ready");

    for (;;) {
        if (g_mbox.status == ST_BUSY) {
            uint32_t resp = 0;
            uint32_t err = 0;
            switch (g_mbox.cmd) {
            case CMD_PING:
                resp = VERSION;
                break;
            case CMD_SET_GPIO:
                gpio_write(g_mbox.arg0, g_mbox.arg1);
                break;
            case CMD_READ_GPIO:
                resp = gpio_read(g_mbox.arg0);
                break;
            case CMD_READ_REG:
                resp = *(volatile uint32_t *)g_mbox.arg0;
                break;
            case CMD_UART_TX:
                usart2_tx_byte((uint8_t)g_mbox.arg0);
                break;
            case CMD_UART_RX_POLL:
                resp = usart2_rx_ready() ? (uint32_t)usart2_rx_byte() : RX_NONE;
                break;
            default:
                err = 1;
                break;
            }
            g_mbox.resp = resp;
            g_mbox.status = err ? ST_ERR : ST_DONE;
        }
    }
}

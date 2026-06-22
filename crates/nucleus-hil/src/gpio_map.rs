//! STM32F4 GPIO peripheral memory map: base addresses + the register offsets
//! [`crate::Backend::pin`] needs. Identical across F446RE, F411RE, and QEMU's
//! `netduinoplus2` (F405) machine model — same AHB1 GPIO block layout per
//! RM0390/RM0383 — so one table serves both backends.

use nucleus_db::Port;

/// Offset of the Input Data Register from a GPIOx base address.
pub const IDR_OFFSET: u32 = 0x10;
/// Offset of the Output Data Register from a GPIOx base address.
pub const ODR_OFFSET: u32 = 0x14;

/// The AHB1 base address of `port`'s GPIO peripheral block.
pub const fn gpio_base(port: Port) -> u32 {
    match port {
        Port::A => 0x4002_0000,
        Port::B => 0x4002_0400,
        Port::C => 0x4002_0800,
        Port::D => 0x4002_0C00,
        Port::E => 0x4002_1000,
        Port::F => 0x4002_1400,
        Port::G => 0x4002_1800,
        Port::H => 0x4002_1C00,
    }
}

/// The address of `port`'s IDR, the register `pin()` reads to get a pin's
/// current input level.
pub const fn idr_address(port: Port) -> u32 {
    gpio_base(port) + IDR_OFFSET
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases_are_0x400_apart_starting_at_gpioa() {
        let ports = [
            Port::A,
            Port::B,
            Port::C,
            Port::D,
            Port::E,
            Port::F,
            Port::G,
            Port::H,
        ];
        assert_eq!(gpio_base(Port::A), 0x4002_0000);
        for (a, b) in ports.iter().zip(ports.iter().skip(1)) {
            assert_eq!(gpio_base(*b) - gpio_base(*a), 0x400);
        }
    }

    #[test]
    fn idr_address_is_base_plus_0x10() {
        assert_eq!(idr_address(Port::A), 0x4002_0010);
        assert_eq!(idr_address(Port::H), gpio_base(Port::H) + 0x10);
    }
}

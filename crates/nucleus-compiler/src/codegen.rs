//! HAL code generation.
//!
//! Turns a validated [`Config`] into two C files:
//!
//! - `nucleus_config.h` — a typed config struct per peripheral plus `extern`
//!   HAL handle declarations and the `Nucleus_Init()` prototype.
//! - `nucleus_init.c` — the resolved config struct instances, the handle
//!   definitions, and a single `Nucleus_Init()` that enables GPIO clocks,
//!   configures the alternate-function muxing (using AF numbers resolved from
//!   [`nucleus_db`]), and calls the stock ST HAL `HAL_*_Init` functions.
//!
//! **Architectural rule (README):** the generated code never reimplements the
//! HAL. It only calls `Init` functions with resolved parameters, so a HAL
//! point-release that changes internals does not break Nucleus output. The
//! tested HAL family is STM32F4 (`stm32f4xx_hal.h`).
//!
//! Codegen assumes the config already passed [`crate::solver::solve`]; it skips
//! unmodelled peripheral kinds and pins it cannot resolve rather than failing.

use std::fmt::Write;
use std::str::FromStr;

use nucleus_db::dma::{Direction, Slot};
use nucleus_db::irq::IrqMap;
use nucleus_db::{Database, Pin};

use crate::config::{Config, Peripheral};
use crate::model;

/// The generated C sources for a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generated {
    pub config_h: String,
    pub init_c: String,
}

/// One peripheral lowered to everything codegen needs.
struct Lowered {
    /// HAL instance name, e.g. `USART2`.
    instance: String,
    /// Handle variable, e.g. `huart2`.
    handle: String,
    /// HAL handle type, e.g. `UART_HandleTypeDef`.
    handle_type: &'static str,
    /// Per-instance config struct type, e.g. `Nucleus_USART2_Config`.
    config_type: String,
    kind: Kind,
    /// Resolved pin uses: `(pin, af, signal)`.
    pins: Vec<(Pin, u8, &'static str)>,
    /// NVIC vector name(s) + preempt priority, when `irq = true` and the
    /// family models a vector for this peripheral.
    irq: Option<IrqInit>,
    /// Resolved DMA stream(s), when `dma` is set and the solver assigned a
    /// slot for that direction. Empty for `Kind::Tim` — TIM's DMA handle
    /// field is the `hdma[]` array, not the simple `hdmatx`/`hdmarx` pair the
    /// other kinds use, so it's out of scope here.
    dma: Vec<DmaInit>,
}

struct IrqInit {
    vectors: &'static [&'static str],
    priority: i64,
}

struct DmaInit {
    direction: Direction,
    slot: Slot,
    priority: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Usart,
    Spi,
    I2c,
    Tim,
}

/// Generate `nucleus_config.h` and `nucleus_init.c` for `config`.
pub fn generate(config: &Config, db: &Database) -> Generated {
    let irq_map = crate::irq_map_for(&config.device.family);
    let dma_map = crate::dma_map_for(&config.device.family);
    // Codegen only runs on a config the solver already validated clean (see
    // `nucleus-cli`'s `generate_sources`), so re-running the same greedy
    // assignment here lands on the exact slots `dma::validate` would have
    // approved — there is nothing left to collide.
    let (dma_assigned, _) = crate::dma::resolve(config, &dma_map);

    let lowered: Vec<Lowered> = config
        .peripherals
        .iter()
        .filter_map(|(instance, table)| lower(instance, table, db, &irq_map, &dma_assigned))
        .collect();

    Generated {
        config_h: config_header(&lowered),
        init_c: init_source(config, &lowered),
    }
}

fn kind_of(instance: &str) -> Option<Kind> {
    let prefix = instance.trim_end_matches(|c: char| c.is_ascii_digit());
    match prefix {
        "usart" | "uart" => Some(Kind::Usart),
        "spi" => Some(Kind::Spi),
        "i2c" | "fmpi2c" => Some(Kind::I2c),
        "tim" => Some(Kind::Tim),
        _ => None,
    }
}

fn lower(
    instance: &str,
    table: &Peripheral,
    db: &Database,
    irq_map: &IrqMap,
    dma_assigned: &std::collections::BTreeMap<(String, Direction), Slot>,
) -> Option<Lowered> {
    let kind = kind_of(instance)?;
    let roles = model::roles_for(instance)?;
    let name = model::peripheral_name(instance);
    // The instance index is the *trailing* digit run, so `i2c1` -> "1" (not
    // "21" from the embedded "2" in "i2c").
    let digits: String = {
        let rev: String = instance
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect();
        rev.chars().rev().collect()
    };

    let (handle_prefix, handle_type) = match kind {
        Kind::Usart => ("huart", "UART_HandleTypeDef"),
        Kind::Spi => ("hspi", "SPI_HandleTypeDef"),
        Kind::I2c => ("hi2c", "I2C_HandleTypeDef"),
        Kind::Tim => ("htim", "TIM_HandleTypeDef"),
    };

    let mut pins = Vec::new();
    for role in roles {
        if let Some(value) = table.pin_str(role.key) {
            if let Ok(pin) = Pin::from_str(value) {
                if let Some(af) = db.find_af(pin, &name, role.signal) {
                    pins.push((pin, af, role.signal));
                }
            }
        }
    }

    let irq = match table.0.get("irq").and_then(toml::Value::as_bool) {
        Some(true) => {
            let vectors = irq_map.vectors(&name);
            if vectors.is_empty() {
                None
            } else {
                let priority = table
                    .0
                    .get("irq_priority")
                    .and_then(toml::Value::as_integer)
                    .unwrap_or(0);
                Some(IrqInit { vectors, priority })
            }
        }
        _ => None,
    };

    let dma_priority = table
        .0
        .get("dma_priority")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    let dma = if kind == Kind::Tim {
        Vec::new()
    } else {
        [Direction::Tx, Direction::Rx]
            .into_iter()
            .filter_map(|direction| {
                dma_assigned
                    .get(&(name.clone(), direction))
                    .map(|&slot| DmaInit {
                        direction,
                        slot,
                        priority: dma_priority,
                    })
            })
            .collect()
    };

    Some(Lowered {
        config_type: format!("Nucleus_{name}_Config"),
        handle: format!("{handle_prefix}{digits}"),
        handle_type,
        instance: name,
        kind,
        pins,
        irq,
        dma,
    })
}

fn config_header(lowered: &[Lowered]) -> String {
    let mut s = String::new();
    s.push_str(GENERATED_BANNER);
    s.push_str(
        "#ifndef NUCLEUS_CONFIG_H\n\
         #define NUCLEUS_CONFIG_H\n\n\
         #include \"stm32f4xx_hal.h\"\n\n\
         #ifdef __cplusplus\n\
         extern \"C\" {\n\
         #endif\n\n",
    );

    for p in lowered {
        let _ = writeln!(s, "/* {} — resolved configuration */", p.instance);
        let _ = writeln!(s, "typedef struct {{");
        for field in p.kind.config_fields() {
            let _ = writeln!(s, "    uint32_t {field};");
        }
        let _ = writeln!(s, "}} {};", p.config_type);
        let _ = writeln!(s, "extern {} {};", p.handle_type, p.handle);
        for d in &p.dma {
            let _ = writeln!(
                s,
                "extern DMA_HandleTypeDef {};",
                dma_handle_name(p, d.direction)
            );
        }
        s.push('\n');
    }

    s.push_str(
        "/* Initializes every peripheral declared in stm32.toml. Call once after\n\
         \x20  HAL_Init() and the system clock configuration. */\n\
         void Nucleus_Init(void);\n\n\
         #ifdef __cplusplus\n\
         }\n\
         #endif\n\n\
         #endif /* NUCLEUS_CONFIG_H */\n",
    );
    s
}

fn init_source(config: &Config, lowered: &[Lowered]) -> String {
    let mut s = String::new();
    s.push_str(GENERATED_BANNER);
    s.push_str("#include \"nucleus_config.h\"\n\n");

    // Handle definitions.
    for p in lowered {
        let _ = writeln!(s, "{} {};", p.handle_type, p.handle);
        for d in &p.dma {
            let _ = writeln!(s, "DMA_HandleTypeDef {};", dma_handle_name(p, d.direction));
        }
    }
    s.push('\n');

    // Resolved config struct instances (the "typed config" the HAL init reads).
    for p in lowered {
        emit_config_instance(&mut s, config, p);
    }

    s.push_str("void Nucleus_Init(void)\n{\n");
    s.push_str("    GPIO_InitTypeDef GPIO_InitStruct = {0};\n\n");

    emit_gpio_clock_enables(&mut s, lowered);
    emit_dma_clock_enables(&mut s, lowered);

    for p in lowered {
        let _ = writeln!(s, "    /* ---- {} ---- */", p.instance);
        emit_gpio_config(&mut s, p);
        emit_peripheral_init(&mut s, p);
        emit_dma_init(&mut s, p);
        emit_irq_init(&mut s, p);
        s.push('\n');
    }

    s.push_str("}\n");
    s
}

fn emit_config_instance(s: &mut String, config: &Config, p: &Lowered) {
    let var = format!("{}_config", p.instance.to_ascii_lowercase());
    let table = &config.peripherals[&p.instance.to_ascii_lowercase()];
    let _ = writeln!(s, "static const {} {} = {{", p.config_type, var);
    match p.kind {
        Kind::Usart => {
            let baud = table
                .0
                .get("baud")
                .and_then(toml::Value::as_integer)
                .unwrap_or(115_200);
            let _ = writeln!(s, "    .BaudRate = {baud}u,");
        }
        Kind::Spi => {
            let mode = table
                .0
                .get("mode")
                .and_then(toml::Value::as_integer)
                .unwrap_or(0);
            let (cpol, cpha) = spi_mode(mode);
            let _ = writeln!(s, "    .CLKPolarity = {cpol},");
            let _ = writeln!(s, "    .CLKPhase = {cpha},");
        }
        Kind::I2c => {
            let speed = table
                .0
                .get("speed")
                .and_then(toml::Value::as_str)
                .unwrap_or("standard");
            let hz = if speed.eq_ignore_ascii_case("fast") {
                400_000
            } else {
                100_000
            };
            let _ = writeln!(s, "    .ClockSpeed = {hz}u,");
        }
        Kind::Tim => {
            let (psc, arr) = tim_timing(config, table);
            let _ = writeln!(s, "    .Prescaler = {psc}u,");
            let _ = writeln!(s, "    .Period = {arr}u,");
        }
    }
    let _ = writeln!(s, "}};\n");
}

fn emit_gpio_clock_enables(s: &mut String, lowered: &[Lowered]) {
    let mut ports: Vec<char> = lowered
        .iter()
        .flat_map(|p| p.pins.iter().map(|(pin, _, _)| pin.port.letter()))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    if ports.is_empty() {
        return;
    }
    s.push_str("    /* GPIO port clocks */\n");
    for port in ports {
        let _ = writeln!(s, "    __HAL_RCC_GPIO{port}_CLK_ENABLE();");
    }
    s.push('\n');
}

fn emit_gpio_config(s: &mut String, p: &Lowered) {
    for (pin, af, _signal) in &p.pins {
        let port = pin.port.letter();
        let pull = if p.kind == Kind::I2c {
            "GPIO_PULLUP"
        } else {
            "GPIO_NOPULL"
        };
        let mode = if p.kind == Kind::I2c {
            "GPIO_MODE_AF_OD"
        } else {
            "GPIO_MODE_AF_PP"
        };
        let _ = writeln!(s, "    GPIO_InitStruct.Pin = GPIO_PIN_{};", pin.number);
        let _ = writeln!(s, "    GPIO_InitStruct.Mode = {mode};");
        let _ = writeln!(s, "    GPIO_InitStruct.Pull = {pull};");
        let _ = writeln!(s, "    GPIO_InitStruct.Speed = GPIO_SPEED_FREQ_VERY_HIGH;");
        let _ = writeln!(
            s,
            "    GPIO_InitStruct.Alternate = GPIO_AF{af}_{};",
            p.instance
        );
        let _ = writeln!(s, "    HAL_GPIO_Init(GPIO{port}, &GPIO_InitStruct);");
    }
}

/// DMA handle variable name, e.g. `hdma_usart2_rx`.
fn dma_handle_name(p: &Lowered, direction: Direction) -> String {
    format!(
        "hdma_{}_{}",
        p.instance.to_ascii_lowercase(),
        direction.name().to_ascii_lowercase()
    )
}

fn dma_priority_macro(priority: i64) -> &'static str {
    match priority {
        0 => "DMA_PRIORITY_LOW",
        1 => "DMA_PRIORITY_MEDIUM",
        2 => "DMA_PRIORITY_HIGH",
        _ => "DMA_PRIORITY_VERY_HIGH",
    }
}

fn emit_dma_clock_enables(s: &mut String, lowered: &[Lowered]) {
    let mut controllers: Vec<&'static str> = lowered
        .iter()
        .flat_map(|p| p.dma.iter().map(|d| d.slot.controller.name()))
        .collect();
    controllers.sort_unstable();
    controllers.dedup();
    if controllers.is_empty() {
        return;
    }
    s.push_str("    /* DMA controller clocks */\n");
    for controller in controllers {
        let _ = writeln!(s, "    __HAL_RCC_{controller}_CLK_ENABLE();");
    }
    s.push('\n');
}

/// Emits `HAL_DMA_Init` + `__HAL_LINKDMA` for every resolved DMA stream on
/// `p`. Field name on the parent handle (`hdmatx`/`hdmarx`) is the same
/// across UART/SPI/I2C `_HandleTypeDef`s, which is why `Kind::Tim` (whose
/// `TIM_HandleTypeDef` instead exposes an `hdma[]` array) is excluded in
/// [`lower`].
fn emit_dma_init(s: &mut String, p: &Lowered) {
    for d in &p.dma {
        let handle = dma_handle_name(p, d.direction);
        let stream = format!("{}_Stream{}", d.slot.controller.name(), d.slot.stream);
        let channel = format!("DMA_CHANNEL_{}", d.slot.channel);
        let direction_macro = match d.direction {
            Direction::Tx => "DMA_MEMORY_TO_PERIPH",
            Direction::Rx => "DMA_PERIPH_TO_MEMORY",
        };
        let field = match d.direction {
            Direction::Tx => "hdmatx",
            Direction::Rx => "hdmarx",
        };

        let _ = writeln!(s, "    {handle}.Instance = {stream};");
        let _ = writeln!(s, "    {handle}.Init.Channel = {channel};");
        let _ = writeln!(s, "    {handle}.Init.Direction = {direction_macro};");
        for (field_name, val) in [
            ("PeriphInc", "DMA_PINC_DISABLE"),
            ("MemInc", "DMA_MINC_ENABLE"),
            ("PeriphDataAlignment", "DMA_PDATAALIGN_BYTE"),
            ("MemDataAlignment", "DMA_MDATAALIGN_BYTE"),
            ("Mode", "DMA_NORMAL"),
            ("Priority", dma_priority_macro(d.priority)),
        ] {
            let _ = writeln!(s, "    {handle}.Init.{field_name} = {val};");
        }
        let _ = writeln!(s, "    HAL_DMA_Init(&{handle});");
        let _ = writeln!(s, "    __HAL_LINKDMA(&{}, {field}, {handle});", p.handle);
    }
}

/// Emits `HAL_NVIC_SetPriority` + `HAL_NVIC_EnableIRQ` for every vector the
/// family models for `p`'s peripheral. I2Cx's two vectors (`_EV`/`_ER`) both
/// get the peripheral's single configured `irq_priority`.
fn emit_irq_init(s: &mut String, p: &Lowered) {
    let Some(irq) = &p.irq else { return };
    for vector in irq.vectors {
        let _ = writeln!(
            s,
            "    HAL_NVIC_SetPriority({vector}_IRQn, {}, 0);",
            irq.priority
        );
        let _ = writeln!(s, "    HAL_NVIC_EnableIRQ({vector}_IRQn);");
    }
}

fn emit_peripheral_init(s: &mut String, p: &Lowered) {
    let h = &p.handle;
    let cfg = format!("{}_config", p.instance.to_ascii_lowercase());
    let _ = writeln!(s, "    __HAL_RCC_{}_CLK_ENABLE();", p.instance);
    let _ = writeln!(s, "    {h}.Instance = {};", p.instance);
    match p.kind {
        Kind::Usart => {
            let _ = writeln!(s, "    {h}.Init.BaudRate = {cfg}.BaudRate;");
            for (field, val) in [
                ("WordLength", "UART_WORDLENGTH_8B"),
                ("StopBits", "UART_STOPBITS_1"),
                ("Parity", "UART_PARITY_NONE"),
                ("Mode", "UART_MODE_TX_RX"),
                ("HwFlowCtl", "UART_HWCONTROL_NONE"),
                ("OverSampling", "UART_OVERSAMPLING_16"),
            ] {
                let _ = writeln!(s, "    {h}.Init.{field} = {val};");
            }
            let _ = writeln!(s, "    HAL_UART_Init(&{h});");
        }
        Kind::Spi => {
            let _ = writeln!(s, "    {h}.Init.CLKPolarity = {cfg}.CLKPolarity;");
            let _ = writeln!(s, "    {h}.Init.CLKPhase = {cfg}.CLKPhase;");
            for (field, val) in [
                ("Mode", "SPI_MODE_MASTER"),
                ("Direction", "SPI_DIRECTION_2LINES"),
                ("DataSize", "SPI_DATASIZE_8BIT"),
                ("NSS", "SPI_NSS_SOFT"),
                ("BaudRatePrescaler", "SPI_BAUDRATEPRESCALER_16"),
                ("FirstBit", "SPI_FIRSTBIT_MSB"),
                ("TIMode", "SPI_TIMODE_DISABLE"),
                ("CRCCalculation", "SPI_CRCCALCULATION_DISABLE"),
            ] {
                let _ = writeln!(s, "    {h}.Init.{field} = {val};");
            }
            let _ = writeln!(s, "    HAL_SPI_Init(&{h});");
        }
        Kind::I2c => {
            let _ = writeln!(s, "    {h}.Init.ClockSpeed = {cfg}.ClockSpeed;");
            for (field, val) in [
                ("DutyCycle", "I2C_DUTYCYCLE_2"),
                ("OwnAddress1", "0"),
                ("AddressingMode", "I2C_ADDRESSINGMODE_7BIT"),
                ("DualAddressMode", "I2C_DUALADDRESS_DISABLE"),
                ("OwnAddress2", "0"),
                ("GeneralCallMode", "I2C_GENERALCALL_DISABLE"),
                ("NoStretchMode", "I2C_NOSTRETCH_DISABLE"),
            ] {
                let _ = writeln!(s, "    {h}.Init.{field} = {val};");
            }
            let _ = writeln!(s, "    HAL_I2C_Init(&{h});");
        }
        Kind::Tim => {
            let _ = writeln!(s, "    {h}.Init.Prescaler = {cfg}.Prescaler;");
            let _ = writeln!(s, "    {h}.Init.Period = {cfg}.Period;");
            for (field, val) in [
                ("CounterMode", "TIM_COUNTERMODE_UP"),
                ("ClockDivision", "TIM_CLOCKDIVISION_DIV1"),
                ("AutoReloadPreload", "TIM_AUTORELOAD_PRELOAD_ENABLE"),
            ] {
                let _ = writeln!(s, "    {h}.Init.{field} = {val};");
            }
            let _ = writeln!(s, "    HAL_TIM_PWM_Init(&{h});");
        }
    }
}

impl Kind {
    fn config_fields(self) -> &'static [&'static str] {
        match self {
            Kind::Usart => &["BaudRate"],
            Kind::Spi => &["CLKPolarity", "CLKPhase"],
            Kind::I2c => &["ClockSpeed"],
            Kind::Tim => &["Prescaler", "Period"],
        }
    }
}

/// SPI mode (0–3) → `(CLKPolarity, CLKPhase)` HAL macros.
fn spi_mode(mode: i64) -> (&'static str, &'static str) {
    match mode {
        1 => ("SPI_POLARITY_LOW", "SPI_PHASE_2EDGE"),
        2 => ("SPI_POLARITY_HIGH", "SPI_PHASE_1EDGE"),
        3 => ("SPI_POLARITY_HIGH", "SPI_PHASE_2EDGE"),
        _ => ("SPI_POLARITY_LOW", "SPI_PHASE_1EDGE"),
    }
}

/// Resolve a PWM timer's `(Prescaler, Period)` from the requested frequency and
/// duty resolution. Uses the device `clock_hz` as the timer clock estimate
/// (an approximation — full clock-tree solving is explicitly out of scope).
///
/// `crate::clocks::validate` runs this same `timer_clk` approximation against
/// `frequency_hz`/`duty_resolution_bits` before codegen ever runs, raising a
/// `Conflict::ClockConstraint` for any combination that would underflow PSC
/// here — so by the time this function runs, `divisor <= timer_clk` always
/// holds and the `saturating_sub` below never actually saturates.
fn tim_timing(config: &Config, table: &Peripheral) -> (u32, u32) {
    let bits = table
        .0
        .get("duty_resolution_bits")
        .and_then(toml::Value::as_integer)
        .unwrap_or(16)
        .clamp(1, 31) as u32;
    let arr: u32 = (1u32 << bits) - 1;

    let freq = table
        .0
        .get("frequency_hz")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1000)
        .max(1) as u64;
    let timer_clk = config.device.clock_hz.unwrap_or(180_000_000).max(1);

    // freq = timer_clk / ((PSC + 1) * (ARR + 1))  =>  PSC = timer_clk/(freq*(ARR+1)) - 1
    let divisor = freq * (arr as u64 + 1);
    let psc = (timer_clk / divisor).saturating_sub(1);
    (psc.min(u32::MAX as u64) as u32, arr)
}

const GENERATED_BANNER: &str = "\
/* Generated by Nucleus — do not edit by hand.\n\
\x20* Regenerate with `nucleus build`. Source of truth: stm32.toml. */\n\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn gen(text: &str) -> Generated {
        let cfg = config::parse(text).unwrap();
        generate(&cfg, &Database::f446re())
    }

    const EXAMPLE: &str = r#"
[device]
family = "STM32F446RE"
clock_hz = 180_000_000

[peripherals.usart2]
tx = "PA2"
rx = "PA3"
baud = 115200

[peripherals.spi1]
mosi = "PA7"
miso = "PA6"
sck = "PA5"
nss = "PA4"
mode = 0

[peripherals.i2c1]
sda = "PB9"
scl = "PB8"
speed = "fast"

[peripherals.tim2]
channel1 = "PA0"
frequency_hz = 1000
duty_resolution_bits = 16
"#;

    #[test]
    fn header_declares_handles_and_prototype() {
        let g = gen(EXAMPLE);
        assert!(g.config_h.contains("extern UART_HandleTypeDef huart2;"));
        assert!(g.config_h.contains("typedef struct {"));
        assert!(g.config_h.contains("void Nucleus_Init(void);"));
        assert!(g.config_h.contains("#ifndef NUCLEUS_CONFIG_H"));
    }

    #[test]
    fn init_calls_stock_hal_init_functions() {
        let g = gen(EXAMPLE);
        for call in [
            "HAL_UART_Init(&huart2);",
            "HAL_SPI_Init(&hspi1);",
            "HAL_I2C_Init(&hi2c1);",
            "HAL_TIM_PWM_Init(&htim2);",
        ] {
            assert!(g.init_c.contains(call), "missing {call}\n{}", g.init_c);
        }
        // Exactly one init entry point.
        assert_eq!(g.init_c.matches("void Nucleus_Init(void)").count(), 1);
    }

    #[test]
    fn gpio_uses_af_numbers_from_database() {
        let g = gen(EXAMPLE);
        // PA2 = USART2_TX is AF7; PA7 = SPI1_MOSI is AF5; PB9 = I2C1_SDA is AF4.
        assert!(g.init_c.contains("GPIO_InitStruct.Pin = GPIO_PIN_2;"));
        assert!(g.init_c.contains("GPIO_AF7_USART2;"));
        assert!(g.init_c.contains("GPIO_AF5_SPI1;"));
        assert!(g.init_c.contains("GPIO_AF4_I2C1;"));
    }

    #[test]
    fn enables_each_gpio_port_clock_once() {
        let g = gen(EXAMPLE);
        assert_eq!(g.init_c.matches("__HAL_RCC_GPIOA_CLK_ENABLE();").count(), 1);
        assert_eq!(g.init_c.matches("__HAL_RCC_GPIOB_CLK_ENABLE();").count(), 1);
    }

    #[test]
    fn i2c_pins_are_open_drain_with_pullups() {
        let g = gen(EXAMPLE);
        assert!(g.init_c.contains("GPIO_MODE_AF_OD"));
        assert!(g.init_c.contains("GPIO_PULLUP"));
    }

    #[test]
    fn resolved_params_land_in_config_structs() {
        let g = gen(EXAMPLE);
        assert!(g.init_c.contains(".BaudRate = 115200u,"));
        assert!(g.init_c.contains(".ClockSpeed = 400000u,")); // fast
        assert!(g.init_c.contains(".CLKPolarity = SPI_POLARITY_LOW,")); // mode 0
    }

    #[test]
    fn output_is_deterministic() {
        assert_eq!(gen(EXAMPLE), gen(EXAMPLE));
    }

    #[test]
    fn empty_config_still_emits_valid_init() {
        let g = gen("[device]\nfamily = \"STM32F446RE\"\n");
        assert!(g.init_c.contains("void Nucleus_Init(void)"));
        assert!(g.config_h.contains("void Nucleus_Init(void);"));
    }

    #[test]
    fn irq_true_emits_nvic_enable_and_priority() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=true\nirq_priority=5\n";
        let g = gen(text);
        assert!(g.init_c.contains("HAL_NVIC_SetPriority(USART2_IRQn, 5, 0);"));
        assert!(g.init_c.contains("HAL_NVIC_EnableIRQ(USART2_IRQn);"));
    }

    #[test]
    fn irq_false_emits_no_nvic_calls() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\nirq=false\n";
        let g = gen(text);
        assert!(!g.init_c.contains("HAL_NVIC"));
    }

    #[test]
    fn i2c_irq_emits_both_event_and_error_vectors() {
        let text = "[peripherals.i2c1]\nsda=\"PB9\"\nscl=\"PB8\"\nirq=true\n";
        let g = gen(text);
        assert!(g.init_c.contains("HAL_NVIC_EnableIRQ(I2C1_EV_IRQn);"));
        assert!(g.init_c.contains("HAL_NVIC_EnableIRQ(I2C1_ER_IRQn);"));
    }

    #[test]
    fn dma_true_emits_hal_dma_init_and_link_for_both_directions() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=true\n";
        let g = gen(text);
        assert!(g.init_c.contains("HAL_DMA_Init(&hdma_usart2_tx);"));
        assert!(g.init_c.contains("HAL_DMA_Init(&hdma_usart2_rx);"));
        assert!(g.init_c.contains("__HAL_LINKDMA(&huart2, hdmatx, hdma_usart2_tx);"));
        assert!(g.init_c.contains("__HAL_LINKDMA(&huart2, hdmarx, hdma_usart2_rx);"));
        assert!(g.init_c.contains("hdma_usart2_tx.Init.Direction = DMA_MEMORY_TO_PERIPH;"));
        assert!(g.init_c.contains("hdma_usart2_rx.Init.Direction = DMA_PERIPH_TO_MEMORY;"));
        assert!(g.config_h.contains("extern DMA_HandleTypeDef hdma_usart2_tx;"));
    }

    #[test]
    fn dma_rx_only_emits_single_stream() {
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=[\"rx\"]\n";
        let g = gen(text);
        assert!(g.init_c.contains("HAL_DMA_Init(&hdma_usart2_rx);"));
        assert!(!g.init_c.contains("hdma_usart2_tx"));
    }

    #[test]
    fn dma_assigns_resolved_stream_and_channel() {
        // USART2_RX is DMA1 stream 5 channel 4 on the F446 (matches dma.rs's
        // model row), so codegen's own greedy resolve must land here too.
        let text = "[peripherals.usart2]\ntx=\"PA2\"\nrx=\"PA3\"\ndma=[\"rx\"]\n";
        let g = gen(text);
        assert!(g.init_c.contains("hdma_usart2_rx.Instance = DMA1_Stream5;"));
        assert!(g.init_c.contains("hdma_usart2_rx.Init.Channel = DMA_CHANNEL_4;"));
        assert!(g.init_c.contains("__HAL_RCC_DMA1_CLK_ENABLE();"));
    }

    #[test]
    fn tim_does_not_emit_dma_init() {
        // TIM's DMA handle field is the `hdma[]` array, out of scope here —
        // opting in must not crash or emit a bogus hdmatx/hdmarx link.
        let text = "[peripherals.tim2]\nchannel1=\"PA0\"\ndma=true\n";
        let g = gen(text);
        assert!(!g.init_c.contains("HAL_DMA_Init"));
    }

    #[test]
    fn no_irq_or_dma_opt_in_emits_neither() {
        let g = gen(EXAMPLE);
        assert!(!g.init_c.contains("HAL_NVIC"));
        assert!(!g.init_c.contains("HAL_DMA_Init"));
    }
}

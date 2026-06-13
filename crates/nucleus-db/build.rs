//! Generates the compiled-in STM32F446RE AF table from the vendored ST pin
//! data in `packdata/` (see `packdata/README.md`). Output is sorted and
//! byte-deterministic; `data.rs` includes it from `$OUT_DIR`.

use std::{env, fs, path::Path};

// The lib compiles this module too; in the build-script copy some items
// (e.g. the `Patch` variants while PATCHES is empty) go unused.
#[allow(dead_code)]
#[path = "src/pack.rs"]
mod pack;

fn main() {
    println!("cargo::rerun-if-changed=packdata");
    println!("cargo::rerun-if-changed=src/pack.rs");

    generate(
        "packdata/GPIO-STM32F446_gpio_v1_0_Modes.xml",
        "packdata/STM32F446RETx.xml",
        "F446RE",
        "f446re_gen.rs",
    );
    generate(
        "packdata/GPIO-STM32F411_gpio_v1_0_Modes.xml",
        "packdata/STM32F411RETx.xml",
        "F411RE",
        "f411re_gen.rs",
    );
}

/// Parse one family's vendored pin data and write its generated AF table.
fn generate(gpio_path: &str, mcu_path: &str, const_name: &str, out_name: &str) {
    let gpio_xml = fs::read_to_string(gpio_path).expect("read GPIO modes xml");
    let mcu_xml = fs::read_to_string(mcu_path).expect("read MCU package xml");

    let mut mappings = pack::parse_gpio_modes(&gpio_xml).expect("parse GPIO modes xml");
    let package_pins = pack::parse_package_pins(&mcu_xml).expect("parse MCU package xml");
    pack::apply_patches(&mut mappings, pack::PATCHES);

    let table = pack::generate_table(&mappings, &package_pins, const_name);

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join(out_name);
    fs::write(out, table).expect("write generated table");
}

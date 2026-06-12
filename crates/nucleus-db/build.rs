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

    let gpio_xml = fs::read_to_string("packdata/GPIO-STM32F446_gpio_v1_0_Modes.xml")
        .expect("read GPIO modes xml");
    let mcu_xml = fs::read_to_string("packdata/STM32F446RETx.xml").expect("read MCU package xml");

    let mut mappings = pack::parse_gpio_modes(&gpio_xml).expect("parse GPIO modes xml");
    let package_pins = pack::parse_package_pins(&mcu_xml).expect("parse MCU package xml");
    pack::apply_patches(&mut mappings, pack::PATCHES);

    let table = pack::generate_table(&mappings, &package_pins);

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("f446re_gen.rs");
    fs::write(out, table).expect("write generated table");
}

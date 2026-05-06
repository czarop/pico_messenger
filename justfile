build:
    cargo build --release

pico:
    elf2uf2-rs target/thumbv8m.main-none-eabihf/release/pico_messenger out.uf2

run:
    cargo run --release
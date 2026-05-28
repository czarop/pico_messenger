build:
    cargo build --release

pico:
    elf2uf2-rs target/thumbv6m-none-eabi/release/pico_messenger out.uf2

pico2:
    elf2uf2-rs target/thumbv8m.main-none-eabihf/release/pico_messenger out.uf2


challenger:
    DEV_BOARD=challenger picotool uf2 convert target/thumbv8m.main-none-eabihf/release/pico_messenger -t elf out.uf2 --family rp2350-arm-s

transfer:
    picotool load out.uf2 --verify

reboot:
    picotool reboot


runc:
    DEV_BOARD=challenger cargo run --release
run:
    cargo run --release
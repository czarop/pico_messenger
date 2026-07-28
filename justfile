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


runcm:
    DEV_BOARD=challenger cargo run --release --bin pico_messenger --features mock_gnss,mock_host_sleep
runcg:
    DEV_BOARD=challenger cargo run --release --bin pico_messenger --features mock_gnss
runc:
    DEV_BOARD=challenger cargo run --release --bin pico_messenger

runm:
    cargo run --release --bin pico_messenger --features mock_modem,mock_host_sleep

provision:
    cargo run --release --bin provision

tcptest:
    cargo run --release --bin tcptest

attach:
    probe-rs attach --chip RP2350 target/thumbv8m.main-none-eabihf/release/pico_messenger
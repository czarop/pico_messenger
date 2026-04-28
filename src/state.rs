use embassy_rp::flash::{ERASE_SIZE, Flash};
use embassy_rp::peripherals::FLASH;
use littlefs2::driver::Storage;
use littlefs2::fs::Filesystem;
use littlefs2::io::Write;
use littlefs2::path::Path;
use serde::{Deserialize, Serialize};
// We use the last 1MB of a 2MB Flash chip
const FS_OFFSET: u32 = 2 * 1024 * 1024 - 64 * 1024; // last 64KB of chip
const FS_SIZE: usize = 64 * 1024;

pub struct EmbassyStorage {
    // Note: The '2097152' is 2MB in bytes
    pub flash: Flash<'static, FLASH, embassy_rp::flash::Blocking, 2097152>,
}

impl Storage for EmbassyStorage {
    const READ_SIZE: usize = 1;
    const WRITE_SIZE: usize = 256;
    const BLOCK_SIZE: usize = ERASE_SIZE; // 4096
    const BLOCK_COUNT: usize = FS_SIZE / ERASE_SIZE; // 256 blocks
    const BLOCK_CYCLES: isize = 100;

    fn read(&mut self, offset: usize, buffer: &mut [u8]) -> littlefs2::io::Result<usize> {
        self.flash
            .blocking_read(FS_OFFSET + offset as u32, buffer)
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(buffer.len())
    }

    fn write(&mut self, offset: usize, data: &[u8]) -> littlefs2::io::Result<usize> {
        self.flash
            .blocking_write(FS_OFFSET + offset as u32, data)
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(data.len())
    }

    fn erase(&mut self, offset: usize, len: usize) -> littlefs2::io::Result<usize> {
        self.flash
            .blocking_erase(
                FS_OFFSET + offset as u32,
                FS_OFFSET + offset as u32 + len as u32,
            )
            .map_err(|_| littlefs2::io::Error::IO)?;
        Ok(len)
    }

    type CACHE_SIZE = littlefs2::consts::U512;
    type LOOKAHEAD_SIZE = littlefs2::consts::U16;
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct AppState {
    pub version: u8,
    pub msg_count: u32,
}

const CONFIG_PATH: &Path = littlefs2::path!("config.bin");

pub fn save_state(fs: &mut Filesystem<'static, EmbassyStorage>, state: &AppState) {
    let mut buffer = [0u8; 128]; // this only changes if AppState grows significantly
    let encoded = postcard::to_slice(state, &mut buffer).unwrap();

    fs.open_file_with_options_and_then(
        |o| o.write(true).create(true).truncate(true),
        CONFIG_PATH,
        |file| {
            file.write_all(encoded)?;
            Ok(())
        },
    )
    .expect("Flash write failed");
}

pub fn load_state(fs: &mut Filesystem<'static, EmbassyStorage>) -> AppState {
    let mut buffer = [0u8; 128];

    let result = fs.open_file_with_options_and_then(
        |o| o.read(true),
        CONFIG_PATH,
        |file| {
            let n = file.read(&mut buffer)?;
            Ok(n)
        },
    );

    match result {
        Ok(n) => {
            // postcard::from_bytes(&buffer[..n]).map_err(|_| littlefs2::io::Error::NO_SUCH_ENTRY)
            match postcard::from_bytes::<AppState>(&buffer[..n]) {
                Ok(state) => state,
                Err(_) => {
                    // Corruption or Version Mismatch detected!
                    // Wipe the file and reset to defaults
                    let _ = fs.remove(CONFIG_PATH); // Delete the bad file
                    AppState::default() // Return defaults
                }
            }
        }
        Err(_) => {
            defmt::warn!("State file doesn't exist, starting with default");
            AppState::default()
        }
    }
}

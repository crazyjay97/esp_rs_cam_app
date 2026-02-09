#![no_std]
#![no_main]

use core::sync::atomic::{AtomicUsize, Ordering};

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_hal::clock::CpuClock;
use {esp_backtrace as _, esp_println as _};
extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

static COUNT: AtomicUsize = AtomicUsize::new(0);
defmt::timestamp!("{=usize}", COUNT.fetch_add(1, Ordering::Relaxed));

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let config = config.with_psram(esp_hal::psram::PsramConfig {
        size: esp_hal::psram::PsramSize::Size(8),
        core_clock: Some(esp_hal::psram::SpiTimingConfigCoreClock::default()),
        flash_frequency: esp_hal::psram::FlashFreq::FlashFreq80m,
        ram_frequency: esp_hal::psram::SpiRamFreq::Freq80m,
    });
    let peripherals = esp_hal::init(config);
    // RTOS Timer
    let timg0_p = unsafe { peripherals.TIMG0.clone_unchecked() };
    let timg0 = esp_hal::timer::timg::TimerGroup::new(timg0_p);
    esp_rtos::start(timg0.timer0);
    let psram_size = esp_hal::psram::psram_raw_parts(&peripherals.PSRAM);
    defmt::info!("PSRAM size: {}", psram_size);
    // Clone for camera before moving fields
    esp_alloc::heap_allocator!(size: 128 * 1024); // 128KB 内部 RAM
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    info!("Embassy initialized!");

    // Init Wifi
    let rng = esp_hal::rng::Rng::new();
    //Init Camera
    let wifi = unsafe { peripherals.WIFI.clone_unchecked() };
    let camera = app::cam::init_cam(peripherals).await.unwrap();

    match app::wifi::init(rng, wifi, &spawner, camera).await {
        Ok(stack) => {
            info!("Waiting to get IP address...");
            loop {
                if let Some(config) = stack.config_v4() {
                    info!("Got IP: {}", config.address);
                    break;
                }
                Timer::after(Duration::from_millis(500)).await;
            }
        }
        Err(e) => {
            defmt::error!("Wifi init failed: {:?}", e)
        }
    }

    loop {
        info!("Running...");
        Timer::after(Duration::from_secs(100)).await;
    }
}

#![no_std]
#![no_main]
#![feature(allocator_api)]
use core::sync::atomic::{AtomicUsize, Ordering};

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_alloc::{self as _};
use esp_hal::clock::CpuClock;
use {esp_backtrace as _, esp_println as _};
extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

static COUNT: AtomicUsize = AtomicUsize::new(0);
defmt::timestamp!("{=usize}", COUNT.fetch_add(1, Ordering::Relaxed));

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let config = config.with_psram({
        let mut config = esp_hal::psram::PsramConfig::default();
        config.size = esp_hal::psram::PsramSize::Size(16 * 1024 * 1024);
        config.flash_frequency = esp_hal::psram::FlashFreq::FlashFreq120m;
        config.core_clock =
            Some(esp_hal::psram::SpiTimingConfigCoreClock::SpiTimingConfigCoreClock80m);
        config
    });
    let peripherals = esp_hal::init(config);
    // RTOS Timer
    let timg0_p = unsafe { peripherals.TIMG0.clone_unchecked() };
    let timg0 = esp_hal::timer::timg::TimerGroup::new(timg0_p);
    esp_alloc::heap_allocator!(size: 128 * 1024); // 128KB 内部 RAM
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);
    esp_rtos::start(timg0.timer0);
    // Clone for camera before moving fields
    info!("Embassy initialized!");
    defmt::info!("RAM: {}", esp_alloc::HEAP.stats());
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
        Timer::after(Duration::from_secs(100000000)).await;
    }
}

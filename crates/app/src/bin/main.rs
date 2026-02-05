#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_hal::clock::CpuClock;
use {esp_backtrace as _, esp_println as _};
extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Clone for camera before moving fields

    esp_alloc::heap_allocator!(size: 72 * 1024);

    // RTOS Timer
    let timg0_p = unsafe { peripherals.TIMG0.clone_unchecked() };
    let timg0 = esp_hal::timer::timg::TimerGroup::new(timg0_p);
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

    // Init Wifi
    let rng = esp_hal::rng::Rng::new();

    if let Err(e) =
        app::wifi::init(rng, unsafe { peripherals.WIFI.clone_unchecked() }, &spawner).await
    {
        defmt::error!("Wifi init failed: {:?}", e);
    } else {
        info!("Wifi initialized!");
    }

    //Init Camera
    app::cam::init_cam(peripherals, spawner).await;

    loop {
        info!("Running...");
        Timer::after(Duration::from_secs(1)).await;
    }
}

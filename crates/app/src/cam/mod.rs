use defmt::info;
use embassy_executor::Spawner;
// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
// use embassy_sync::mutex::Mutex;
use embassy_time::Timer;
use esp_hal::{
    delay::Delay,
    dma::DmaRxStreamBuf,
    dma_rx_stream_buffer,
    gpio::{Level, Output, OutputConfig},
    i2c,
    lcd_cam::{
        cam::{Camera, CameraTransfer, Config},
        LcdCam,
    },
    peripherals::Peripherals,
    time::Rate,
    Async,
};

//pub static JPEG_DATA: Mutex<CriticalSectionRawMutex, Option<Vec<u8, 65535>>> = Mutex::new(None);

/// GND
/// SCL    ->   14
/// SDA    ->   13
/// D0     ->   12
/// D2     ->   11
/// D4     ->   10
/// D6     ->   9
/// PCLK   ->   3
/// PWDN   ->   8
/// 3.3V
/// SYNC   ->   36
/// HREF   ->   37
/// RST    ->   38
/// D1     ->   39
/// D3     ->   40
/// D5     ->   41
/// D7     ->   42
/// FLASH  ->   2
pub async fn init_cam(peripherals: Peripherals, spawner: Spawner) {
    let mut delay = Delay::new();

    let _pwdn = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let mut rst = Output::new(peripherals.GPIO38, Level::Low, OutputConfig::default());

    delay.delay_millis(10);
    rst.set_high();
    delay.delay_millis(10);

    let i2c_config = i2c::master::Config::default();
    let i2c = i2c::master::I2c::new(peripherals.I2C0, i2c_config)
        .unwrap()
        .with_scl(peripherals.GPIO14)
        .with_sda(peripherals.GPIO13);

    let dma_buf = dma_rx_stream_buffer!(20 * 1000, 1000);
    let vsync_pin = peripherals.GPIO36;
    let href_pin = peripherals.GPIO37;
    let pclk_pin = peripherals.GPIO3;

    let config = Config::default().with_frequency(Rate::from_mhz(20));

    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    let camera = Camera::new(lcd_cam.cam, peripherals.DMA_CH0, config)
        .unwrap()
        .with_data0(peripherals.GPIO12)
        .with_data1(peripherals.GPIO39)
        .with_data2(peripherals.GPIO11)
        .with_data3(peripherals.GPIO40)
        .with_data4(peripherals.GPIO10)
        .with_data5(peripherals.GPIO41)
        .with_data6(peripherals.GPIO9)
        .with_data7(peripherals.GPIO42)
        .with_pixel_clock(pclk_pin)
        .with_vsync(vsync_pin)
        .with_h_enable(href_pin);

    let mut ov = ov2640::OV2640::new(i2c);
    match ov.init(&mut delay) {
        Ok(_) => defmt::info!("init ov2640 ok"),
        Err(e) => defmt::warn!("init ov2640 failed {:?}", e),
    }
    //let dma_buf = dma_rx_stream_buffer!(60 * 1024, 2048); // 60KB buffer
    spawner.spawn(cam_task_handle(camera, dma_buf)).ok();
}

#[embassy_executor::task]
async fn cam_task_handle(mut camera: Camera<'static>, mut dma_buf: DmaRxStreamBuf) {
    defmt::info!("cam_task_handle>>>>>>>>>>>>>>>");

    loop {
        let mut transfer = match camera.receive(dma_buf) {
            Ok(t) => t,
            Err((e, cam, buf)) => {
                defmt::error!("Camera receive error: {:?}", e);
                return;
            }
        };
        for _ in 0..2 {
            loop {
                let mut len = 0;
                let mut eof = false;
                {
                    let (data, e) = transfer.peek_until_eof();
                    len = data.len();
                    eof = e
                }
                transfer.consume(len);
                if eof {
                    break;
                }
            }
        }
        loop {
            let mut len = 0;
            let mut eof = false;
            {
                let (data, e) = transfer.peek_until_eof();
                info!("{:02X}", data);
                len = data.len();
                eof = e
            }
            transfer.consume(len);
            if eof {
                break;
            }
        }
        (camera, dma_buf) = transfer.stop();
    }
}

fn extract_jpeg(data: &[u8]) -> Option<heapless::Vec<u8, 65535>> {
    let mut out = heapless::Vec::<u8, 65535>::new();
    let mut found_start = false;
    let mut i = 0;

    while i + 1 < data.len() {
        if !found_start {
            if data[i] == 0xFF && data[i + 1] == 0xD8 {
                found_start = true;
                out.extend_from_slice(&[0xFF, 0xD8]).ok()?;
                i += 2;
                continue;
            }
        } else {
            if data[i] == 0xFF && data[i + 1] == 0xD9 {
                out.extend_from_slice(&[0xFF, 0xD9]).ok()?;
                return Some(out);
            }
            out.push(data[i]).ok()?;
        }
        i += 1;
    }

    None
}

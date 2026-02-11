use alloc::vec::Vec;
use defmt::{error, info, warn};
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{
    delay::Delay,
    dma::DmaRxStreamBuf,
    gpio::{Level, Output, OutputConfig},
    i2c,
    lcd_cam::{
        cam::{Camera, Config, EofMode},
        LcdCam,
    },
    peripherals::Peripherals,
    time::Rate,
};

use crate::wifi::write_all;

pub enum CamEvent {
    FrameStart,
    Data(Vec<u8>),
    FrameEnd,
}

pub static CAM_CHANNEL: Channel<CriticalSectionRawMutex, CamEvent, 5> = Channel::new();

/// GND
/// SCL    ->   13
/// SDA    ->   12
/// D0     ->   11
/// D2     ->   10
/// D4     ->   9
/// D6     ->   46
/// PCLK   ->   3
/// PWDN   ->   8
/// 3.3V
/// VSYNC   ->  4
/// HREF   ->   5
/// RST    ->   6
/// D1     ->   7
/// D3     ->   15
/// D5     ->   16
/// D7     ->   17
/// FLASH  ->   18
pub async fn init_cam(peripherals: Peripherals) -> Result<Camera<'static>, ()> {
    let mut delay = Delay::new();

    let _pwdn = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let mut rst = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());

    delay.delay_millis(10);
    rst.set_high();
    delay.delay_millis(10);

    let i2c_config = i2c::master::Config::default();
    let i2c = i2c::master::I2c::new(peripherals.I2C0, i2c_config)
        .unwrap()
        .with_scl(peripherals.GPIO13)
        .with_sda(peripherals.GPIO12);

    let vsync_pin = peripherals.GPIO4;
    let href_pin = peripherals.GPIO5;
    let pclk_pin = peripherals.GPIO3;

    let config = Config::default()
        .with_frequency(Rate::from_mhz(20))
        .with_eof_mode(EofMode::VsyncSignal)
        .with_invert_vsync(false)
        .with_invert_h_enable(false);

    let lcd_cam = LcdCam::new(peripherals.LCD_CAM);
    let camera = Camera::new(lcd_cam.cam, peripherals.DMA_CH0, config)
        .unwrap()
        .with_data0(peripherals.GPIO11)
        .with_data1(peripherals.GPIO7)
        .with_data2(peripherals.GPIO10)
        .with_data3(peripherals.GPIO15)
        .with_data4(peripherals.GPIO9)
        .with_data5(peripherals.GPIO16)
        .with_data6(peripherals.GPIO46)
        .with_data7(peripherals.GPIO17)
        .with_pixel_clock(pclk_pin)
        .with_vsync(vsync_pin)
        .with_h_enable(href_pin);

    let mut ov = ov2640::OV2640::new(i2c);
    match ov.init(&mut delay) {
        Ok(_) => defmt::info!("init ov2640 ok"),
        Err(e) => defmt::warn!("init ov2640 failed {:?}", e),
    }
    match ov.set_image_format(ov2640::ImageFormat::JPEG, &mut delay) {
        Ok(_) => defmt::info!("ov2640 set_image_format ok"),
        Err(e) => defmt::warn!("ov2640 set_image_format failed {:?}", e),
    }
    match ov.set_resolution(ov2640::Resolution::R320x240) {
        Ok(_) => defmt::info!("ov2640 set_resolution ok"),
        Err(e) => defmt::warn!("ov2640 set_resolution failed {:?}", e),
    }
    match ov.set_saturation(ov2640::Saturation::Saturation1) {
        Ok(_) => defmt::info!("ov2640 set_saturation ok"),
        Err(e) => defmt::warn!("ov2640 set_saturation failed {:?}", e),
    }
    match ov.set_brightness(ov2640::Brightness::Brightness1) {
        Ok(_) => defmt::info!("ov2640 set_brightness ok"),
        Err(e) => defmt::warn!("ov2640 set_brightness failed {:?}", e),
    }
    match ov.set_contrast(ov2640::Contrast::Contrast1) {
        Ok(_) => defmt::info!("ov2640 set_contrast ok"),
        Err(e) => defmt::warn!("ov2640 set_contrast failed {:?}", e),
    }
    match ov.set_special_effect(ov2640::SpecialEffect::Normal) {
        Ok(_) => defmt::info!("ov2640 set_special_effect ok"),
        Err(e) => defmt::warn!("ov2640 set_special_effect failed {:?}", e),
    };
    Ok(camera)
}

pub async fn stream_camera(
    mut camera: Camera<'static>,
    mut dma_buf: DmaRxStreamBuf,
    socket: &mut TcpSocket<'_>,
) -> (Camera<'static>, DmaRxStreamBuf) {
    if let Err(e) = write_all(
        socket,
        b"HTTP/1.1 200 OK\r\n\
          Content-Type: multipart/x-mixed-replace; boundary=frame\r\n\
          Cache-Control: no-cache\r\n\
          Connection: keep-alive\r\n\r\n",
    )
    .await
    {
        warn!("Failed to send HTTP headers: {}", e);
        return (camera, dma_buf);
    }
    let mut jpeg_buffer = unsafe {
        let p = esp_alloc::HEAP.alloc_caps(
            esp_alloc::MemoryCapability::External.into(),
            core::alloc::Layout::from_size_align(40960 * 10, 4).unwrap(),
        );
        alloc::vec::Vec::from_raw_parts(p, 40960 * 10, 40960 * 10)
    };
    let mut jpeg_len = 0;
    let mut frame_count = 0;
    let mut in_frame = false;
    let mut fps_count = 0;
    let mut last_fps_instant = Instant::now();

    loop {
        let mut transfer = match camera.receive(dma_buf) {
            Ok(t) => t,
            Err((e, cam, buf)) => {
                error!("Camera receive error: {:?}", e);
                return (cam, buf);
            }
        };
        loop {
            let (data, eof) = transfer.peek_until_eof();
            let len = data.len();
            if data.is_empty() {
                if transfer.is_done() {
                    warn!("Too slow!");
                    break;
                }
                if eof {
                    break;
                }
                Timer::after_micros(100).await;
                continue;
            }
            if let Err(_) = process_jpeg_data(
                data,
                &mut jpeg_buffer,
                &mut jpeg_len,
                &mut in_frame,
                &mut frame_count,
                socket,
            )
            .await
            {
                warn!("process error");
                return transfer.stop();
            }
            transfer.consume(len);
            if eof {
                let now = Instant::now();
                if now - last_fps_instant >= Duration::from_secs(1) {
                    let frames_sent = frame_count - fps_count;
                    info!("FPS: {}", frames_sent);
                    fps_count = frame_count;
                    last_fps_instant = now;
                    defmt::info!("HEAP: {:?}", esp_alloc::HEAP.stats());
                }
            }
        }
        (camera, dma_buf) = transfer.stop();
    }
}

async fn process_jpeg_data(
    data: &[u8],
    jpeg_buffer: &mut [u8],
    jpeg_len: &mut usize,
    in_frame: &mut bool,
    frame_count: &mut u32,
    socket: &mut TcpSocket<'_>,
) -> Result<(), ()> {
    let mut i = 0;
    while i < data.len() {
        if !*in_frame {
            if let Some(pos) = data[i..].windows(2).position(|w| w == [0xFF, 0xD8]) {
                *in_frame = true;
                *jpeg_len = 0;
                i += pos;

                if jpeg_buffer.len() >= 2 {
                    jpeg_buffer[..2].copy_from_slice(&[0xFF, 0xD8]);
                    *jpeg_len = 2;
                    i += 2;
                } else {
                    warn!("buf to small, buf size: {}", jpeg_buffer.len());
                    return Err(());
                }
            } else {
                break;
            }
            continue;
        }

        let remaining = &data[i..];
        if let Some(eoi_pos) = remaining.windows(2).position(|w| w == [0xFF, 0xD9]) {
            let bytes_to_copy = eoi_pos + 2;
            if *jpeg_len + bytes_to_copy <= jpeg_buffer.len() {
                jpeg_buffer[*jpeg_len..*jpeg_len + bytes_to_copy]
                    .copy_from_slice(&remaining[..bytes_to_copy]);
                *jpeg_len += bytes_to_copy;
                if let Err(e) =
                    send_jpeg_frame(socket, &jpeg_buffer[..*jpeg_len], *frame_count).await
                {
                    warn!("Failed to send frame: {}", e);
                    return Err(());
                }
                *frame_count += 1;
            } else {
                warn!(
                    "JPEG buffer overflow, dropping frame (size: {})",
                    *jpeg_len + bytes_to_copy
                );
            }
            *in_frame = false;
            *jpeg_len = 0;
            i += bytes_to_copy;
        } else {
            let bytes_to_copy = remaining.len();
            let available_space = jpeg_buffer.len().saturating_sub(*jpeg_len);

            if bytes_to_copy <= available_space {
                jpeg_buffer[*jpeg_len..*jpeg_len + bytes_to_copy].copy_from_slice(remaining);
                *jpeg_len += bytes_to_copy;
            } else {
                warn!("JPEG buffer full at {} bytes, dropping frame", *jpeg_len);
                *in_frame = false;
                *jpeg_len = 0;
            }
            break;
        }
    }

    Ok(())
}

async fn send_jpeg_frame(
    socket: &mut TcpSocket<'_>,
    jpeg_data: &[u8],
    _frame_count: u32,
) -> Result<(), ()> {
    let mut header = heapless::String::<256>::new();
    use core::fmt::Write;

    let _ = write!(
        &mut header,
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg_data.len()
    );

    if let Err(e) = write_all(socket, header.as_bytes()).await {
        warn!("Failed to send frame header: {}", e);
        return Err(());
    }

    if let Err(e) = write_all(socket, jpeg_data).await {
        warn!("Failed to send JPEG data: {}", e);
        return Err(());
    }

    if let Err(e) = write_all(socket, b"\r\n").await {
        warn!("Failed to send frame end: {}", e);
        return Err(());
    }

    Ok(())
}

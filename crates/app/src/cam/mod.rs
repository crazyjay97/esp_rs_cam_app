use alloc::vec::Vec;
use defmt::info;
use embassy_executor::Spawner;
use embassy_net::{dns::Socket, tcp::TcpSocket};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::{
    delay::Delay,
    dma_rx_stream_buffer,
    gpio::{Level, Output, OutputConfig},
    i2c,
    lcd_cam::{
        cam::{Camera, Config, EofMode},
        LcdCam,
    },
    peripherals::Peripherals,
    time::Rate,
};

// Define event types for the camera stream
pub enum CamEvent {
    FrameStart,
    Data(Vec<u8>),
    FrameEnd,
}

// Channel for streaming data from cam_task to http_handle
// Capacity 10 allows some buffering if HTTP task is slightly slower
pub static CAM_CHANNEL: Channel<CriticalSectionRawMutex, CamEvent, 5> = Channel::new();

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
/// VSYNC   ->  36
/// HREF   ->   37
/// RST    ->   38
/// D1     ->   39
/// D3     ->   40
/// D5     ->   41
/// D7     ->   42
/// FLASH  ->   2
pub async fn init_cam(peripherals: Peripherals) -> Result<Camera<'static>, ()> {
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

    let vsync_pin = peripherals.GPIO36;
    let href_pin = peripherals.GPIO37;
    let pclk_pin = peripherals.GPIO3;

    let config = Config::default()
        .with_frequency(Rate::from_mhz(10))
        .with_eof_mode(EofMode::VsyncSignal)
        .with_invert_vsync(false)
        .with_invert_h_enable(false);

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
    match ov.set_image_format(ov2640::ImageFormat::JPEG, &mut delay) {
        Ok(_) => defmt::info!("ov2640 set_image_format ok"),
        Err(e) => defmt::warn!("ov2640 set_image_format failed {:?}", e),
    }
    match ov.set_resolution(ov2640::Resolution::R800x600) {
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
    match ov.set_contrast(ov2640::Contrast::Contrast2) {
        Ok(_) => defmt::info!("ov2640 set_contrast ok"),
        Err(e) => defmt::warn!("ov2640 set_contrast failed {:?}", e),
    }
    match ov.set_special_effect(ov2640::SpecialEffect::Normal) {
        Ok(_) => defmt::info!("ov2640 set_special_effect ok"),
        Err(e) => defmt::warn!("ov2640 set_special_effect failed {:?}", e),
    };
    //spawner.spawn(cam_task(camera, dma_buf)).ok();
    //let dma_buf = dma_rx_stream_buffer!(20 * 1024, 1000);
    Ok(camera)
}

use esp_hal::dma::DmaRxStreamBuf;

#[embassy_executor::task]
async fn cam_task(mut camera: Camera<'static>, mut dma_buf: DmaRxStreamBuf) {
    // JPEG 一帧通常 20~60KB，给大一点避免频繁 realloc
    let mut frame_buffer: Vec<u8> = Vec::with_capacity(16 * 1024);
    let mut found_start = false;

    info!("cam task started >>>>>>>>>>>>>>>>>>");

    loop {
        let mut transfer = match camera.receive(dma_buf) {
            Ok(t) => t,
            Err((e, _cam, _buf)) => {
                defmt::error!("Camera receive error: {:?}", e);
                return;
            }
        };

        // 跳过前 2 个 dummy transfer
        for _ in 0..2 {
            loop {
                let (data, eof) = transfer.peek_until_eof();
                let len = data.len();
                transfer.consume(len);
                if eof {
                    break;
                }
            }
        }

        loop {
            let (data, eof) = transfer.peek_until_eof();
            let len = data.len();

            if len > 0 {
                let mut i = 0;
                while i < len {
                    if !found_start {
                        // 找 FF D8
                        if i + 1 < len && data[i] == 0xFF && data[i + 1] == 0xD8 {
                            found_start = true;
                            frame_buffer.clear();
                            CAM_CHANNEL.send(CamEvent::FrameStart).await;

                            frame_buffer.extend_from_slice(&[0xFF, 0xD8]);
                            i += 2;
                        } else {
                            i += 1;
                        }
                    } else {
                        // 找 FF D9
                        if i + 1 < len && data[i] == 0xFF && data[i + 1] == 0xD9 {
                            frame_buffer.push(0xFF);
                            frame_buffer.push(0xD9);
                            if !frame_buffer.is_empty() {
                                let chunk = core::mem::take(&mut frame_buffer);
                                CAM_CHANNEL.send(CamEvent::Data(chunk)).await;
                            }

                            CAM_CHANNEL.send(CamEvent::FrameEnd).await;

                            found_start = false;
                            i += 2;
                        } else {
                            frame_buffer.push(data[i]);
                            i += 1;
                            // 达到 chunk 大小就发
                            if frame_buffer.len() >= 2048 {
                                let chunk = core::mem::take(&mut frame_buffer);
                                CAM_CHANNEL.send(CamEvent::Data(chunk)).await;
                            }
                        }
                    }
                }
            }

            transfer.consume(len);
            if eof {
                break;
            }
        }
        // 结束 DMA
        (camera, dma_buf) = transfer.stop();
        // 帧间隔
        Timer::after(Duration::from_millis(1000)).await;
    }
}

/// TODO WORK
pub async fn stream_camera(
    mut camera: Camera<'static>,
    mut dma_buf: DmaRxStreamBuf,
    socket: &mut TcpSocket<'_>,
) -> (Camera<'static>, DmaRxStreamBuf) {
    let mut buf_len = 0;
    let mut found_start = false;

    loop {
        let mut transfer = match camera.receive(dma_buf) {
            Ok(t) => t,
            Err((e, _cam, _buf)) => {
                defmt::error!("Camera receive error: {:?}", e);
                return (_cam, _buf);
            }
        };

        // 跳过前 2 个 dummy transfer
        for _ in 0..2 {
            loop {
                let (data, eof) = transfer.peek_until_eof();
                let len = data.len();
                transfer.consume(len);
                if eof {
                    break;
                }
            }
        }

        loop {
            let (data, eof) = transfer.peek_until_eof();
            let len = data.len();

            if len > 0 {
                let mut i = 0;
                while i < len {
                    if !found_start {
                        // 找 FF D8
                        if i + 1 < len && data[i] == 0xFF && data[i + 1] == 0xD8 {
                            found_start = true;
                            buf_len = 0;
                            //let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=boundarystring\r\nConnection: keep-alive\r\n\r\n").await;

                            // 直接把 FF D8 写进 buffer
                            frame_buffer[buf_len] = 0xFF;
                            frame_buffer[buf_len + 1] = 0xD8;
                            buf_len += 2;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    } else {
                        // 找 FF D9
                        if i + 1 < len && data[i] == 0xFF && data[i + 1] == 0xD9 {
                            frame_buffer[buf_len] = 0xFF;
                            frame_buffer[buf_len + 1] = 0xD9;
                            buf_len += 2;

                            if on_chunk(&frame_buffer[..buf_len]).await.is_err() {
                                defmt::warn!("Chunk send failed");
                                return transfer.stop();
                            }

                            found_start = false;
                            buf_len = 0;
                            i += 2;
                        } else {
                            // 写入 buffer
                            if buf_len < frame_buffer.len() {
                                frame_buffer[buf_len] = data[i];
                                buf_len += 1;
                                i += 1;
                            } else {
                                defmt::warn!("Frame buffer overflow, dropping data");
                                i += 1;
                            }

                            // 达到 2KB chunk，提前发送
                            if buf_len >= 2048 {
                                if on_chunk(&frame_buffer[..buf_len]).await.is_err() {
                                    defmt::warn!("Chunk send failed");
                                    return transfer.stop();
                                }
                                buf_len = 0;
                            }
                        }
                    }
                }
            }

            transfer.consume(len);
            if eof {
                break;
            }
        }
        (camera, dma_buf) = transfer.stop();
        Timer::after(Duration::from_millis(10)).await;
    }
}

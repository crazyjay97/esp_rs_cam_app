use alloc::{boxed::Box, vec::Vec};
use defmt::{error, info, warn};
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Timer;
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
    match ov.set_contrast(ov2640::Contrast::Contrast2) {
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
    // 发送 HTTP 头
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

    let mut jpeg_buffer: Box<[u8; 40960]> = Box::new([0u8; 40960]);
    let mut jpeg_len = 0;
    let mut in_frame = false;
    let mut frame_count = 0;

    loop {
        info!("Starting camera receive, frame: {}", frame_count);

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
                transfer.consume(0);
                if eof {
                    break;
                }
                continue;
            }

            if let Err(_) = process_jpeg_data(
                data,
                &mut *jpeg_buffer, // 解引用 Box
                &mut jpeg_len,
                &mut in_frame,
                &mut frame_count,
                socket,
            )
            .await
            {
                return transfer.stop();
            }

            transfer.consume(len);
            if eof {
                break;
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
        // 查找 JPEG 起始标记 (SOI: 0xFF 0xD8)
        if !*in_frame {
            if i + 1 < data.len() && data[i] == 0xFF && data[i + 1] == 0xD8 {
                *in_frame = true;
                *jpeg_len = 0;

                // 写入 SOI 到缓冲区
                if *jpeg_len + 2 <= jpeg_buffer.len() {
                    jpeg_buffer[*jpeg_len..*jpeg_len + 2].copy_from_slice(&[0xFF, 0xD8]);
                    *jpeg_len += 2;
                }
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        // 在帧内,查找结束标记 (EOI: 0xFF 0xD9)
        if i + 1 < data.len() && data[i] == 0xFF && data[i + 1] == 0xD9 {
            // 写入 EOI
            if *jpeg_len + 2 <= jpeg_buffer.len() {
                jpeg_buffer[*jpeg_len..*jpeg_len + 2].copy_from_slice(&[0xFF, 0xD9]);
                *jpeg_len += 2;

                // 发送完整的 multipart 帧
                if let Err(e) =
                    send_jpeg_frame(socket, &jpeg_buffer[..*jpeg_len], *frame_count).await
                {
                    warn!("Failed to send frame: {}", e);
                    return Err(());
                }

                info!("Frame {} sent, size: {} bytes", *frame_count, *jpeg_len);
                *frame_count += 1;
                *in_frame = false;
                *jpeg_len = 0;
                i += 2;
                continue;
            } else {
                // 缓冲区溢出，丢弃此帧
                warn!(
                    "JPEG buffer overflow, dropping frame (size would be: {})",
                    *jpeg_len + 2
                );
                *in_frame = false;
                *jpeg_len = 0;
                i += 2;
                continue;
            }
        }

        // 累积帧数据
        if *jpeg_len < jpeg_buffer.len() {
            jpeg_buffer[*jpeg_len] = data[i];
            *jpeg_len += 1;
        } else {
            // 缓冲区满，丢弃此帧
            warn!("JPEG buffer full at {} bytes, dropping frame", *jpeg_len);
            *in_frame = false;
            *jpeg_len = 0;
        }
        i += 1;
    }

    Ok(())
}

async fn send_jpeg_frame(
    socket: &mut TcpSocket<'_>,
    jpeg_data: &[u8],
    _frame_count: u32,
) -> Result<(), ()> {
    // 构造 multipart 边界和头部
    // 使用 heapless::String 避免堆分配
    let mut header = heapless::String::<256>::new();
    use core::fmt::Write;

    let _ = write!(
        &mut header,
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg_data.len()
    );

    // 发送头部
    if let Err(e) = write_all(socket, header.as_bytes()).await {
        warn!("Failed to send frame header: {}", e);
        return Err(());
    }

    // 发送 JPEG 数据
    if let Err(e) = write_all(socket, jpeg_data).await {
        warn!("Failed to send JPEG data: {}", e);
        return Err(());
    }

    // 发送帧结尾
    if let Err(e) = write_all(socket, b"\r\n").await {
        warn!("Failed to send frame end: {}", e);
        return Err(());
    }

    Ok(())
}

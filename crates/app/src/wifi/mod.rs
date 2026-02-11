use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, IpListenEndpoint, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::{dma_rx_stream_buffer, lcd_cam::cam::Camera, peripherals::WIFI, rng::Rng};
use esp_radio::{
    wifi::{self, ClientConfig, WifiController, WifiDevice, WifiEvent, WifiMode},
    Controller,
};
extern crate alloc;
use alloc::{boxed::Box, string::String};

use crate::{cam::stream_camera, errors::RuntimeError, mk_static};

pub async fn init(
    rng: Rng,
    wifi_peripheral: WIFI<'static>,
    spawner: &Spawner,
    camera: Camera<'static>,
) -> Result<Stack<'static>, RuntimeError> {
    let init = esp_radio::init()?;
    let init = mk_static!(Controller, init);
    let (control, interface) = wifi::new(init, wifi_peripheral, Default::default())?;
    let device = interface.sta;
    let config = embassy_net::Config::dhcpv4(Default::default());
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let stack_resources = mk_static!(StackResources::<6>, StackResources::<6>::new());
    let (stack, runner) = embassy_net::new(device, config, stack_resources, seed);
    spawner.spawn(connection(control)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(http_handle(stack, camera)).ok();
    Ok(stack)
}

pub async fn write_all(socket: &mut TcpSocket<'_>, buf: &[u8]) -> Result<(), ()> {
    // info!("{:02X}", buf);
    let mut offset = 0;
    while offset < buf.len() {
        match socket.write(&buf[offset..]).await {
            Ok(0) => return Err(()),
            Ok(n) => offset += n,
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

#[embassy_executor::task]
pub async fn http_handle(stack: Stack<'static>, mut camera: Camera<'static>) {
    let mut rx_buffer = Box::new([0u8; 4096]);
    //let mut tx_buffer = Box::new([0u8; 4096 * 14]);
    let mut tx_buffer = unsafe {
        let p = esp_alloc::HEAP.alloc_caps(
            esp_alloc::MemoryCapability::External.into(),
            core::alloc::Layout::from_size_align(40960 * 10, 4).unwrap(),
        );
        alloc::vec::Vec::from_raw_parts(p, 40960 * 10, 40960 * 10)
    };
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    stack
        .config_v4()
        .inspect(|c| defmt::info!("ipv4 config: {}", c));
    let mut dma_buf = dma_rx_stream_buffer!(65536, 1024);
    loop {
        let mut socket = TcpSocket::new(stack, &mut *rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        defmt::info!("Wait for connection...");
        let r = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: 80,
            })
            .await;
        defmt::info!("Connected...");
        if let Err(e) = r {
            defmt::info!("connect error: {:?}", e);
            continue;
        }
        let mut buffer = [0u8; 2048];
        let mut pos = 0;

        let mut attempts = 0;
        let max_attempts = 20;

        while attempts < max_attempts {
            match socket.read(&mut buffer[pos..]).await {
                Ok(0) => {
                    // 连接正常关闭
                    if pos > 0 {
                        defmt::info!("Client finished sending ({} bytes)", pos);
                        break;
                    } else {
                        defmt::info!("Client closed without data");
                        break;
                    }
                }
                Ok(len) => {
                    pos += len;
                    defmt::debug!("Read {} bytes, total {}", len, pos);

                    let request = unsafe { core::str::from_utf8_unchecked(&buffer[..pos]) };

                    // 检查是否收到完整请求
                    if request.contains("\r\n\r\n") {
                        defmt::info!("Complete request received");
                        break;
                    }

                    if pos >= buffer.len() {
                        defmt::warn!("Buffer full");
                        break;
                    }

                    attempts = 0; // 收到数据，重置尝试计数
                }
                Err(e) => {
                    defmt::warn!("Read error: {:?}, bytes read: {}", e, pos);
                    // 如果已经读到了一些数据，尝试处理
                    if pos > 0 {
                        break;
                    } else {
                        // 没读到数据，直接关闭
                        socket.close();
                        Timer::after(Duration::from_millis(10)).await;
                        continue;
                    }
                }
            }
            attempts += 1;
        }

        // 如果没有读到任何数据，跳过处理
        if pos == 0 {
            defmt::warn!("No data received, closing");
            socket.close();
            Timer::after(Duration::from_millis(10)).await;
            continue;
        }

        let request = unsafe { core::str::from_utf8_unchecked(&buffer[..pos]) };
        defmt::info!("Request: <{}>", request);
        if request.contains("GET /index") {
            let buf = include_bytes!("../../../../stream.html");
            let mut header = heapless::String::<256>::new();
            use core::fmt::Write;
            let _ = write!(
                &mut header,
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n",
                buf.len()
            );
            if write_all(&mut socket, header.as_bytes()).await.is_err() {
                continue;
            }
            if write_all(&mut socket, buf).await.is_err() {
                continue;
            }
        } else if request.contains("GET /stream") {
            (camera, dma_buf) = stream_camera(camera, dma_buf, &mut socket).await;
        } else if request.contains("GET /snapshot") {
            _ = socket
                .write(b"HTTP/1.1 404 Not Found\r\n\r\nNo Image")
                .await;
        } else {
            // Default to simple page
            let html = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html><body><h1>ESP32 Camera</h1><img src='/stream' /></body></html>";
            _ = socket.write(html.as_bytes()).await;
        }

        let r = socket.flush().await;
        if let Err(e) = r {
            defmt::info!("flush error: {:?}", e);
        }
        Timer::after(Duration::from_millis(10)).await;
        socket.close();
        defmt::info!("close >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>");
    }
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    loop {
        if !matches!(controller.is_started(), Ok(true)) {
            controller.set_mode(WifiMode::Sta).unwrap();
            let client_config = ClientConfig::default()
                .with_ssid(String::try_from("TP-LINK_7CBB").unwrap())
                .with_password(String::try_from("mt.123456").unwrap());
            controller
                .set_config(&wifi::ModeConfig::Client(client_config))
                .unwrap();
            controller.start_async().await.unwrap();
        }

        defmt::info!("Wifi connecting...");
        match controller.connect_async().await {
            Ok(_) => {
                defmt::info!("Wifi connected!");
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
            }
            Err(e) => {
                defmt::warn!("Failed to connect: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

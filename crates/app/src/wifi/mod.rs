use core::net::{Ipv4Addr, SocketAddrV4};

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket, IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_time::{Duration, Timer};
use esp_hal::{peripherals::WIFI, rng::Rng};
use esp_radio::{
    wifi::{self, AccessPointConfig, AuthMethod, WifiController, WifiDevice, WifiEvent, WifiMode},
    Controller,
};
extern crate alloc;

use crate::{errors::RuntimeError, mk_static};

const ADDR: (u8, u8, u8, u8) = (11, 0, 0, 1);

pub async fn init(
    rng: Rng,
    wifi_peripheral: WIFI<'static>,
    spawner: &Spawner,
) -> Result<(), RuntimeError> {
    let init = esp_radio::init()?;
    let init = mk_static!(Controller, init);
    let (control, interface) = wifi::new(init, wifi_peripheral, Default::default())?;
    let device = interface.ap;
    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Addr::new(ADDR.0, ADDR.1, ADDR.2, ADDR.3), 24),
        gateway: Some(Ipv4Addr::new(ADDR.0, ADDR.1, ADDR.2, ADDR.3)),
        dns_servers: Default::default(),
    });
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let stack_resources = mk_static!(StackResources::<6>, StackResources::<6>::new());
    let (stack, runner) = embassy_net::new(device, config, stack_resources, seed);
    spawner.spawn(connection(control)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(run_dhcp(stack)).ok();
    spawner.spawn(http_handle(stack)).ok();
    spawner.spawn(dns_task(stack)).ok();
    Ok(())
}

#[embassy_executor::task]
pub async fn http_handle(stack: Stack<'static>) {
    let mut rx_buffer = [0; 1536];
    let mut tx_buffer = [0; 1536];
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
    stack
        .config_v4()
        .inspect(|c| defmt::info!("ipv4 config: {}", c));
    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
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
        if request.contains("GET /hotspot-detect.html") {
            defmt::info!("iOS hotspot-detect.html - serving portal page directly");
            let html = build_simple_portal_page();
            _ = socket.write(html.as_bytes()).await;
        } else if request.contains("GET /snapshot") {
            let data = {
                //let lock = crate::cam::JPEG_DATA.lock().await;
                //lock.clone()
            };
            // if let Some(img) = data {
            //     let header =
            //         "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nConnection: close\r\n\r\n";
            //     _ = socket.write(header.as_bytes()).await;
            //     _ = socket.write(&img).await;
            // } else {
            //     _ = socket
            //         .write(b"HTTP/1.1 404 Not Found\r\n\r\nNo Image")
            //         .await;
            // }
        } else if request.contains("GET /portal")
            || request.contains("GET /index")
            || request.contains("GET / ")
        {
            // 返回 portal 页面
            defmt::info!("Serving portal page");
            let html = build_portal_page();
            _ = socket.write(html.as_bytes()).await;
        } else if is_captive_portal_check(request) {
            defmt::info!("is_captive_portal_check ******************");
            let response = b"HTTP/1.1 302 Found\r\n\
Location: http://11.0.0.1/portal\r\n\
Cache-Control: no-cache\r\n\
Connection: close\r\n\
Content-Length: 0\r\n\
\r\n";
            _ = socket.write(response).await;
        } else {
            let response = b"HTTP/1.1 302 Found\r\n\
Location: http://11.0.0.1/portal\r\n\
Cache-Control: no-cache\r\n\
Connection: close\r\n\
Content-Length: 0\r\n\
\r\n";
            _ = socket.write(response).await;
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

fn is_captive_portal_check(request: &str) -> bool {
    // 各操作系统的检测 URL
    request.contains("generate_204") ||      // Android
    request.contains("hotspot-detect") ||    // iOS/macOS
    request.contains("connecttest") ||       // Windows
    request.contains("success.txt") // 一些 Linux 发行版
}

fn build_portal_page() -> heapless::String<2048> {
    let body = "\
<!DOCTYPE html>\
<html>\
<head>\
    <meta charset='utf-8'>\
    <meta name='viewport' content='width=device-width, initial-scale=1'>\
    <title>欢迎使用 ESP WiFi</title>\
    <style>\
        body { font-family: Arial; text-align: center; padding: 50px; }\
        h1 { color: #333; }\
        .btn { \
            background: #4CAF50; \
            color: white; \
            padding: 15px 30px; \
            border: none; \
            border-radius: 5px; \
            font-size: 16px; \
        }\
    </style>\
</head>\
<body>\
    <h1>🎉 欢迎连接 ESP WiFi</h1>\
    <p>你已成功连接到设备</p>\
    <button class='btn' onclick='alert(\"已连接!\")'>确认</button>\
</body>\
</html>";

    let mut page = heapless::String::<2048>::new();
    use core::fmt::Write;

    let _ = write!(
        page,
        "HTTP/1.1 200 OK\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Cache-Control: no-cache\r\n\
Connection: close\r\n\
Content-Length: {}\r\n\
\r\n\
{}",
        body.as_bytes().len(),
        body
    );

    page
}

fn build_simple_portal_page() -> heapless::String<1024> {
    // 所有样式内联，不依赖外部资源，极简设计
    let body = "<!DOCTYPE html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width,initial-scale=1'><title>WiFi</title></head><body style='margin:0;padding:60px 20px;font-family:sans-serif;text-align:center;background:#f5f5f5'><div style='background:#fff;padding:40px 20px;border-radius:12px;max-width:320px;margin:0 auto;box-shadow:0 2px 8px rgba(0,0,0,0.1)'><h1 style='margin:0 0 20px;color:#333;font-size:26px'>Connected</h1><p style='margin:0;color:#34C759;font-size:20px;font-weight:bold'>✓ Success</p><p style='margin:16px 0 0;color:#666;font-size:15px'>You are connected to ESP WiFi</p></div></body></html>";

    let mut page = heapless::String::<1024>::new();
    use core::fmt::Write;

    let _ = write!(
        page,
        "HTTP/1.1 200 OK\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Content-Length: {}\r\n\
Cache-Control: no-cache\r\n\
Connection: close\r\n\
\r\n\
{}",
        body.len(),
        body
    );

    page
}

fn build_camera_page() -> heapless::String<2048> {
    let body = "<!DOCTYPE html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'><title>ESP Camera</title><style>body{text-align:center;padding:20px}img{max-width:100%}</style><script>setInterval(function(){document.getElementById('c').src='/snapshot?'+new Date().getTime()},200)</script></head><body><h1>Camera</h1><img id='c' src='/snapshot'></body></html>";

    let mut page = heapless::String::<2048>::new();
    use core::fmt::Write;

    let _ = write!(
        page,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    page
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    loop {
        if !matches!(controller.is_started(), Ok(true)) {
            if start_ap(&mut controller).await.is_err() {
                defmt::warn!("ap start failed !!!!!");
            }
        } else {
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
    }
}

pub async fn start_ap(controller: &mut WifiController<'static>) -> Result<(), RuntimeError> {
    controller.set_mode(WifiMode::Ap)?;
    let ap_config = AccessPointConfig::default()
        .with_ssid(alloc::string::String::try_from("ESP-Camera").unwrap())
        .with_auth_method(AuthMethod::None);
    controller.set_config(&wifi::ModeConfig::AccessPoint(ap_config))?;
    controller.start_async().await?;
    Ok(())
}

#[embassy_executor::task]
async fn run_dhcp(stack: Stack<'static>) {
    _ = dhcp_task(stack).await;
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

async fn dhcp_task(stack: Stack<'static>) -> Result<(), RuntimeError> {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let ip = Ipv4Addr::new(ADDR.0, ADDR.1, ADDR.2, ADDR.3);

    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    let mut opts = ServerOptions::new(ip, Some(&mut gw_buf));
    let dns = &[ip];
    opts.dns = dns;
    loop {
        _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(ip),
            &opts,
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|_| defmt::warn!("DHCP server error"));
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
async fn dns_task(stack: Stack<'static>) {
    use edge_nal::{UdpBind, UdpReceive, UdpSend};
    use edge_nal_embassy::{Udp, UdpBuffers};
    let mut buf = [0u8; 512];
    let buffers = UdpBuffers::<3, 512, 512, 5>::new();
    let unbound_socket = Udp::new(stack, &buffers);

    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            53, // DNS 端口
        )))
        .await
        .unwrap();

    loop {
        if let Ok((len, remote)) = bound_socket.receive(&mut buf).await {
            // 解析 DNS 请求
            if len > 12 {
                // 构建响应：将所有域名解析到 AP 的 IP
                let response = build_dns_response(&buf[..len], ADDR);

                if let Some(resp) = response {
                    _ = bound_socket.send(remote, &resp).await;
                }
            }
        }
    }
}

fn build_dns_response(query: &[u8], addr: (u8, u8, u8, u8)) -> Option<heapless::Vec<u8, 512>> {
    use heapless::Vec;
    if query.len() < 12 {
        return None;
    }
    let mut response = Vec::<u8, 512>::new();
    // Transaction ID
    response.extend_from_slice(&query[0..2]).ok()?;
    // Flags: Standard query response, No error
    response.extend_from_slice(&[0x85, 0x80]).ok()?;
    // Questions count
    response.extend_from_slice(&query[4..6]).ok()?;
    // Answer RRs
    response.extend_from_slice(&query[4..6]).ok()?;
    // Authority RRs
    response.extend_from_slice(&[0x00, 0x00]).ok()?;
    // Additional RRs
    response.extend_from_slice(&[0x00, 0x00]).ok()?;
    let question_end = find_question_end(&query[12..])?;
    response
        .extend_from_slice(&query[12..12 + question_end])
        .ok()?;
    // Ack
    // Name: Domain (0xC00C)
    response.extend_from_slice(&[0xC0, 0x0C]).ok()?;
    // Type: A (0x0001)
    response.extend_from_slice(&[0x00, 0x01]).ok()?;
    // Class: IN (0x0001)
    response.extend_from_slice(&[0x00, 0x01]).ok()?;
    // TTL: 60 seconds
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]).ok()?;
    // Data length: 4 bytes
    response.extend_from_slice(&[0x00, 0x04]).ok()?;
    // IP Address
    response
        .extend_from_slice(&[addr.0, addr.1, addr.2, addr.3])
        .ok()?;
    Some(response)
}

fn find_question_end(data: &[u8]) -> Option<usize> {
    let mut pos = 0;
    loop {
        if pos >= data.len() {
            return None;
        }
        let len = data[pos] as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            pos += 2;
            break;
        }
        pos += len + 1;
        if pos > data.len() {
            return None;
        }
    }
    pos += 4;
    if pos <= data.len() {
        Some(pos)
    } else {
        None
    }
}

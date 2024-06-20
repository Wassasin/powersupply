use core::marker::PhantomData;

use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Config, Ipv4Address, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
    WifiState,
};

use crate::bsp::Wifi;

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASSWORD");

macro_rules! mk_static {
    ($t:path,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

pub struct Net(PhantomData<()>);

impl Net {
    pub async fn init(wifi: Wifi, spawner: &Spawner) -> Net {
        let config = Config::dhcpv4(Default::default());

        // Init network stack
        let stack = &*mk_static!(
            Stack<WifiDevice<'_, WifiStaDevice>>,
            Stack::new(
                wifi.device,
                config,
                mk_static!(StackResources<3>, StackResources::<3>::new()),
                wifi.seed
            )
        );

        spawner.spawn(connection_task(wifi.controller)).ok();
        spawner.spawn(stack_task(&stack)).ok();

        spawner.spawn(net_task(&stack)).ok();

        Net(PhantomData::default())
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(50)).await;
    }

    log::info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            log::info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(50)).await;
    }

    loop {
        let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let remote_endpoint = (Ipv4Address::new(142, 250, 185, 115), 80);
        log::info!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            log::info!("connect error: {:?}", e);
            continue;
        }
        log::info!("connected!");
        let mut buf = [0; 1024];
        loop {
            use embedded_io_async::Write;
            let r = socket
                .write_all(b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n")
                .await;
            if let Err(e) = r {
                log::info!("write error: {:?}", e);
                break;
            }
            let n = match socket.read(&mut buf).await {
                Ok(0) => {
                    log::info!("read EOF");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    log::info!("read error: {:?}", e);
                    break;
                }
            };
            log::info!("{}", core::str::from_utf8(&buf[..n]).unwrap());
        }
        Timer::after(Duration::from_millis(3000)).await;
    }
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    log::info!("start connection task");
    log::info!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            log::info!("Starting wifi");
            controller.start().await.unwrap();
            log::info!("Wifi started!");
        }
        log::info!("About to connect...");

        match controller.connect().await {
            Ok(_) => log::info!("Wifi connected!"),
            Err(e) => {
                log::info!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn stack_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}

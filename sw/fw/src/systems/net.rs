use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, Config, IpEndpoint, Ipv4Address, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
    WifiState,
};
use heapless::{String, Vec};
use rust_mqtt::{
    client::{client::MqttClient, client_config::ClientConfig},
    packet::v5::{publish_packet::QualityOfService, reason_codes::ReasonCode},
    utils::rng_generator::CountingRng,
};
use serde::Serialize;

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

type DataChannel<T> = Channel<NoopRawMutex, T, 1>;

const TOPIC_SIZE: usize = 64;
const CONTENT_SIZE: usize = 80;

pub struct Message {
    topic: String<TOPIC_SIZE>,
    content: Vec<u8, CONTENT_SIZE>,
}

#[derive(Debug)]
pub enum Error {
    TopicTooLarge,
    ContentTooLarge,
}

pub enum Topic {
    Stats,
    Record,
    Config,
}

impl Topic {
    pub fn to_str(&self) -> Result<String<TOPIC_SIZE>, ()> {
        match self {
            Topic::Stats => String::try_from("slakkotron/stats").map_err(|_| ()),
            Topic::Record => String::try_from("slakkotron/record").map_err(|_| ()),
            Topic::Config => String::try_from("slakkotron/config").map_err(|_| ()),
        }
    }
}

impl Message {
    pub fn new(topic: &Topic, value: &impl Serialize) -> Result<Self, Error> {
        let topic = topic.to_str().map_err(|_| Error::TopicTooLarge)?;
        let mut content: Vec<u8, CONTENT_SIZE> = Vec::new();
        content.resize_default(CONTENT_SIZE).unwrap();
        let size =
            serde_json_core::to_slice(value, &mut content).map_err(|_| Error::ContentTooLarge)?;
        content.truncate(size);

        Ok(Self { topic, content })
    }
}

pub struct Net {
    channel: DataChannel<Message>,
}

impl Net {
    pub async fn init(wifi: Wifi, spawner: &Spawner) -> &'static Net {
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

        let net = &*mk_static!(
            Net,
            Net {
                channel: DataChannel::new()
            }
        );

        spawner.spawn(connection_task(wifi.controller)).unwrap();
        spawner.spawn(stack_task(&stack)).unwrap();
        spawner
            .spawn(net_task(&stack, &net.channel, wifi.seed))
            .unwrap();

        net
    }

    pub async fn send(&self, message: Message) {
        self.channel.send(message).await
    }
}

#[embassy_executor::task]
async fn net_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    channel: &'static DataChannel<Message>,
    seed: u64,
) {
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

        let endpoint = IpEndpoint::new(Ipv4Address::new(192, 168, 1, 2).into(), 1883);

        log::info!("Connecting...");
        let r = socket.connect(endpoint).await;
        if let Err(e) = r {
            log::info!("connect error: {:?}", e);
            continue;
        }
        log::info!("Connected!");

        let mut config: ClientConfig<'_, 20, CountingRng> = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(seed % (u16::MAX as u64)),
        );

        config.add_max_subscribe_qos(QualityOfService::QoS1);
        config.add_client_id("slakkotron");
        config.max_packet_size = 20;

        let mut recv_buffer = [0; 256];
        let mut write_buffer = [0; 256];

        let mut client = MqttClient::new(
            socket,
            &mut write_buffer,
            512,
            &mut recv_buffer,
            512,
            config,
        );

        log::info!("Connecting to broker...");
        client.connect_to_broker().await.unwrap();
        log::info!("Connected");

        loop {
            let message = channel.receive().await;
            match client
                .send_message(
                    &message.topic,
                    &message.content,
                    QualityOfService::QoS1,
                    false,
                )
                .await
            {
                Ok(()) => {}
                Err(ReasonCode::NoMatchingSubscribers) => {}
                Err(e) => {
                    log::error!("{:?}", e);
                    break;
                }
            }
        }
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

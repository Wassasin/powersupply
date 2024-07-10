//! Networking and MQTT client.

use embassy_executor::Spawner;
use embassy_net::{tcp::TcpSocket, IpEndpoint, Ipv4Address, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel, pubsub::PubSubBehavior};
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
use static_cell::StaticCell;

use crate::{
    bsp::Wifi,
    systems::{
        config::{Config, SettingsBuilder},
        watchdog::{Watchdog, WatchdogTicket},
    },
    util::{PubSub, Sub},
};

const SSID: &str = env!("WIFI_SSID");
const PASSWORD: &str = env!("WIFI_PASSWORD");

type MessageChannel<T> = Channel<NoopRawMutex, T, 1>;

const TOPIC_SIZE: usize = 64;
const CONTENT_SIZE: usize = 128;
const MAX_PACKET_SIZE: usize = 256;
const SOCKET_BUFFER_SIZE: usize = 1024;
const MAX_PROPERTIES: usize = 20;

pub struct Message {
    topic: String<TOPIC_SIZE>,
    content: Vec<u8, CONTENT_SIZE>,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Event {
    ConnectedWifi,
    ConnectedMQTT,
}

#[derive(Debug)]
pub enum Error {
    TopicTooLarge,
    ContentTooLarge,
}

#[derive(Debug)]
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

    pub fn try_parse(str: &str) -> Result<Topic, ()> {
        // TODO do properly
        if str == "slakkotron/config" {
            Ok(Topic::Config)
        } else {
            Err(())
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
    outgoing_channel: MessageChannel<Message>,
    event_channel: PubSub<Event>,
    config: &'static Config,
}

impl Net {
    pub async fn init(
        wifi: Wifi,
        config: &'static Config,
        watchdog: &'static Watchdog,
        spawner: &Spawner,
    ) -> &'static Net {
        let netconfig = embassy_net::Config::dhcpv4(Default::default());

        static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
        let resources = RESOURCES.init(StackResources::<3>::new());

        static STACK: StaticCell<Stack<WifiDevice<'_, WifiStaDevice>>> = StaticCell::new();
        let stack = STACK.init(Stack::new(wifi.device, netconfig, resources, wifi.seed));

        static SYSTEM: StaticCell<Net> = StaticCell::new();
        let system: &mut Net = SYSTEM.init(Net {
            outgoing_channel: MessageChannel::new(),
            event_channel: PubSub::new(),
            config,
        });

        spawner.spawn(connection_task(wifi.controller)).unwrap();
        spawner.spawn(stack_task(stack)).unwrap();
        spawner
            .spawn(net_task(stack, system, wifi.seed, watchdog.ticket().await))
            .unwrap();

        system
    }

    pub async fn send(&self, message: Message) {
        self.outgoing_channel.send(message).await
    }

    pub fn event_subscriber(&'static self) -> Sub<Event> {
        self.event_channel.subscriber().unwrap()
    }

    async fn process_message(&self, topic: &str, buf: &[u8]) {
        if let Ok(topic) = Topic::try_parse(topic) {
            log::info!("Received from {:?}", topic);
            #[allow(clippy::single_match)]
            match topic {
                Topic::Config => {
                    if let Ok((new_settings, _)) =
                        serde_json_core::from_slice::<SettingsBuilder>(buf)
                    {
                        self.config
                            .update(|mut settings| {
                                settings.integrate(new_settings);
                                settings
                            })
                            .await
                    } else {
                        log::warn!("Failed to parse settings");
                    }
                }
                _ => {}
            }
        } else {
            log::warn!("Received message from unknown topic \"{}\"", topic)
        }
    }
}

/// Try to send a message with when receiving an unrelated packet, retry until we get an Ack.
async fn send_message_qos1<'a>(
    client: &mut MqttClient<'a, TcpSocket<'a>, MAX_PROPERTIES, CountingRng>,
    topic: &str,
    content: &[u8],
    retain: bool,
) -> Result<(), ReasonCode> {
    // TODO fix the MQTT client to not be lossy.

    for _ in 0..5 {
        match client
            .send_message(topic, content, QualityOfService::QoS1, retain)
            .await
        {
            Ok(()) => return Ok(()),
            Err(ReasonCode::ImplementationSpecificError) => {
                continue;
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
    Err(ReasonCode::ImplementationSpecificError)
}

async fn mqtt_connected<'a>(
    system: &'static Net,
    watchdog_ticket: &WatchdogTicket,
    client: &mut MqttClient<'a, TcpSocket<'a>, MAX_PROPERTIES, CountingRng>,
) {
    loop {
        watchdog_ticket.feed().await;

        let outcoming_fut = system.outgoing_channel.receive();
        let incoming_fut = client.receive_message();

        match embassy_futures::select::select(outcoming_fut, incoming_fut).await {
            embassy_futures::select::Either::First(message) => {
                if let Err(e) =
                    send_message_qos1(client, &message.topic, &message.content, false).await
                {
                    log::error!("{:?}", e);
                }
            }
            embassy_futures::select::Either::Second(message) => match message {
                Ok((topic, buf)) => system.process_message(topic, buf).await,
                Err(ReasonCode::ImplementationSpecificError) => {}
                Err(ReasonCode::NetworkError) => {
                    log::error!("Network error");
                    return;
                }
                Err(e) => log::error!("{:?}", e),
            },
        }
    }
}

async fn link_up(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    system: &'static Net,
    seed: u64,
    watchdog_ticket: &WatchdogTicket,
) {
    system.event_channel.publish_immediate(Event::ConnectedWifi);
    watchdog_ticket.feed().await;

    let mut rx_buffer = [0; SOCKET_BUFFER_SIZE];
    let mut tx_buffer = [0; SOCKET_BUFFER_SIZE];

    loop {
        if !stack.is_link_up() {
            log::warn!("Link down, awaiting reconnect...");
            return;
        }

        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let endpoint = IpEndpoint::new(Ipv4Address::new(192, 168, 1, 2).into(), 1883);

        log::info!("Connecting to socket...");
        let r = socket.connect(endpoint).await;
        if let Err(e) = r {
            log::info!("connect error: {:?}", e);
            continue;
        }
        log::info!("Socket connected!");

        let mut config: ClientConfig<'_, MAX_PROPERTIES, CountingRng> = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(seed % (u16::MAX as u64)),
        );

        config.add_max_subscribe_qos(QualityOfService::QoS1);
        config.add_client_id("slakkotron");
        config.max_packet_size = MAX_PACKET_SIZE as u32;

        let mut recv_buffer = [0; MAX_PACKET_SIZE];
        let mut write_buffer = [0; MAX_PACKET_SIZE];

        let mut client = MqttClient::new(
            socket,
            &mut write_buffer,
            MAX_PACKET_SIZE,
            &mut recv_buffer,
            MAX_PACKET_SIZE,
            config,
        );

        log::info!("Connecting to broker...");
        client.connect_to_broker().await.unwrap();
        log::info!("Broker connected");

        system.event_channel.publish_immediate(Event::ConnectedMQTT);

        client
            .subscribe_to_topic(&Topic::Config.to_str().unwrap())
            .await
            .unwrap();

        mqtt_connected(system, watchdog_ticket, &mut client).await;

        log::warn!("MQTT connection broken down, reconnecting...");
    }
}

#[embassy_executor::task]
async fn net_task(
    stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>,
    system: &'static Net,
    seed: u64,
    watchdog_ticket: WatchdogTicket,
) {
    loop {
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

        watchdog_ticket.feed().await;

        link_up(stack, system, seed, &watchdog_ticket).await;
    }
}

#[embassy_executor::task]
async fn connection_task(mut controller: WifiController<'static>) {
    log::info!("start connection task");
    log::info!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        if let WifiState::StaConnected = esp_wifi::wifi::get_wifi_state() {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(1000)).await
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
    stack.run().await;
}

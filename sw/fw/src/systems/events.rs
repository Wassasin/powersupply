//! System to act on various events and engaging systems correspondingly.

use embassy_executor::Spawner;

use crate::systems::{
    config::Config,
    net::{self, Net},
    record::Record,
    stats::Stats,
};

pub struct Events;

impl Events {
    pub async fn init(
        stats: &'static Stats,
        record: &'static Record,
        config: &'static Config,
        net: &'static Net,
        spawner: &Spawner,
    ) {
        spawner.must_spawn(net_task(record, config, net));
        spawner.must_spawn(publish_task(stats, record, config, net));
    }
}

/// Task to act on Net events like connected and specific messages received.
#[embassy_executor::task]
async fn net_task(record: &'static Record, config: &'static Config, net: &'static Net) {
    let mut subscriber = net.event_subscriber();
    loop {
        use embassy_sync::pubsub::WaitResult;

        #[allow(clippy::single_match)]
        match subscriber.next_message().await {
            WaitResult::Message(event) => {
                log::info!("Net {:#?}", event);
                match event {
                    net::Event::ConnectedMQTT => {
                        record.publish_immediate().await;
                        config.publish_immediate().await;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Task to publish various reports from systems to Net::MQTT.
#[embassy_executor::task]
async fn publish_task(
    stats: &'static Stats,
    record: &'static Record,
    config: &'static Config,
    net: &'static Net,
) {
    let mut stats_subscriber = stats.subscriber();
    let mut record_subscriber = record.subscriber();
    let mut config_subscriber = config.subscriber();
    loop {
        use embassy_futures::select::Either3;
        use embassy_sync::pubsub::WaitResult;

        match embassy_futures::select::select3(
            stats_subscriber.next_message(),
            record_subscriber.next_message(),
            config_subscriber.next_message(),
        )
        .await
        {
            Either3::First(WaitResult::Message(message)) => {
                log::info!("Stats {:#?}", message);
                net.send(net::Message::new(&net::Topic::Stats, &message).unwrap())
                    .await;
            }
            Either3::Second(WaitResult::Message(message)) => {
                log::info!("Record {:#?}", message);
                net.send(net::Message::new(&net::Topic::Record, &message).unwrap())
                    .await;
            }
            Either3::Third(WaitResult::Message(message)) => {
                log::info!("Config {:#?}", message);
                net.send(net::Message::new(&net::Topic::Config, &message).unwrap())
                    .await;
            }
            _ => {}
        }
    }
}

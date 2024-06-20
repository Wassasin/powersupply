use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embedded_hal_async::i2c;
use esp_hal::{
    clock::{ClockControl, Clocks},
    delay::Delay,
    dma::{ChannelCreator, Dma, DmaPriority},
    dma_buffers,
    gpio::{GpioPin, Input, Io, Level, Output, Pull},
    i2c::I2C,
    i2s::{DataFormat, I2s, Standard},
    ledc::{timer::Timer, LSGlobalClkSource, Ledc, LowSpeed},
    peripherals::{Peripherals, I2C0},
    prelude::*,
    rmt::Rmt,
    rng::Rng,
    rtc_cntl::Rtc,
    system::SystemControl,
    timer::{systimer::SystemTimer, timg::TimerGroup},
    Async, Blocking,
};
use esp_wifi::wifi::{WifiController, WifiDevice, WifiStaDevice};
use static_cell::StaticCell;

pub type I2cInstance = I2C<'static, I2C0, Async>;
pub type ClocksInstance = Clocks<'static>;

pub type I2cBus = Mutex<NoopRawMutex, I2cInstance>;
pub type I2cBusDevice = I2cDevice<'static, NoopRawMutex, I2cInstance>;
pub type I2cError = <I2cBusDevice as i2c::ErrorType>::Error;

pub struct Wifi {
    pub device: WifiDevice<'static, WifiStaDevice>,
    pub controller: WifiController<'static>,
    pub seed: u64,
}

pub struct Bsp {
    pub i2c_bus: &'static mut I2cBus,

    pub clocks: &'static ClocksInstance,
    pub delay: Delay,
    pub rtc: Rtc<'static>,
    pub wifi: Wifi,
}

impl Bsp {
    pub fn init(peripherals: Peripherals) -> Self {
        let system = SystemControl::new(peripherals.SYSTEM);
        let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

        static CLOCKS: StaticCell<Clocks> = StaticCell::new();
        let clocks = CLOCKS.init(ClockControl::max(system.clock_control).freeze());

        let mut rtc = Rtc::new(peripherals.LPWR, None);

        let systimer = SystemTimer::new(peripherals.SYSTIMER).alarm0;
        let mut rng = Rng::new(peripherals.RNG);
        let wifi_seed = (rng.random() as u64) << 32 | rng.random() as u64;
        let wifi_init = esp_wifi::initialize(
            esp_wifi::EspWifiInitFor::Wifi,
            systimer,
            rng,
            peripherals.RADIO_CLK,
            &clocks,
        )
        .unwrap();

        let (wifi_device, wifi_controller) =
            esp_wifi::wifi::new_with_mode(&wifi_init, peripherals.WIFI, WifiStaDevice).unwrap();

        let wifi = Wifi {
            device: wifi_device,
            controller: wifi_controller,
            seed: wifi_seed,
        };

        let timg0 = TimerGroup::new_async(peripherals.TIMG0, &clocks);
        esp_hal_embassy::init(&clocks, timg0);

        let i2c: I2cInstance = I2C::new_with_timeout_async(
            peripherals.I2C0,
            io.pins.gpio5,
            io.pins.gpio4,
            400u32.kHz(),
            &clocks,
            Some(20),
        );

        static I2C_BUS: StaticCell<I2cBus> = StaticCell::new();
        let i2c_bus: &'static mut _ = I2C_BUS.init(Mutex::new(i2c));

        let mut delay = Delay::new(&clocks);

        log::info!("initialized");

        Bsp {
            i2c_bus,
            clocks,
            delay,
            rtc,
            wifi,
        }
    }
}

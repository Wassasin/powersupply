use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_executor::SendSpawner;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    mutex::Mutex,
};
use embedded_hal_async::i2c;
use esp_hal::{
    analog::adc::{Adc, AdcCalBasic, AdcCalCurve, AdcConfig, AdcPin},
    clock::{ClockControl, Clocks},
    delay::Delay,
    dma::{ChannelCreator, Dma, DmaPriority},
    dma_buffers,
    gpio::{GpioPin, Input, Io, Level, Output, Pull},
    i2c::I2C,
    i2s::{DataFormat, I2s, Standard},
    ledc::{timer::Timer, LSGlobalClkSource, Ledc, LowSpeed},
    peripherals::{Peripherals, ADC1, I2C0},
    prelude::*,
    rmt::Rmt,
    rng::Rng,
    rtc_cntl::Rtc,
    system::SystemControl,
    timer::{systimer::SystemTimer, timg::TimerGroup},
    Async, Blocking,
};
use esp_hal_embassy::InterruptExecutor;
use esp_storage::FlashStorage;
use esp_wifi::wifi::{WifiController, WifiDevice, WifiStaDevice};
use static_cell::StaticCell;

pub type I2cInstance = I2C<'static, I2C0, Async>;
pub type ClocksInstance = Clocks<'static>;

pub type I2cBus = Mutex<CriticalSectionRawMutex, I2cInstance>;
pub type I2cBusDevice = I2cDevice<'static, CriticalSectionRawMutex, I2cInstance>;
pub type I2cError = <I2cBusDevice as i2c::ErrorType>::Error;

pub struct Wifi {
    pub device: WifiDevice<'static, WifiStaDevice>,
    pub controller: WifiController<'static>,
    pub seed: u64,
}

pub type StatsADCInstance = ADC1;
pub type StatsAdcCal = AdcCalCurve<StatsADCInstance>;
pub type StatsVProgMeasurePin = AdcPin<GpioPin<1>, StatsADCInstance, StatsAdcCal>;
pub type StatsVSupplyMeasurePin = AdcPin<GpioPin<0>, StatsADCInstance, StatsAdcCal>;
pub type StatsVOutMeasurePin = AdcPin<GpioPin<3>, StatsADCInstance, StatsAdcCal>;

pub struct StatsPins {
    pub vprog: StatsVProgMeasurePin,
    pub vsupply: StatsVSupplyMeasurePin,
    pub vout: StatsVOutMeasurePin,
}

pub type StatsAdc = Adc<'static, StatsADCInstance>;

pub struct Stats {
    pub pins: StatsPins,
    pub adc: StatsAdc,
}

pub type PowerExtEnablePin = Output<'static, GpioPin<10>>;
pub type PowerExtNIntPin = Input<'static, GpioPin<20>>;

pub struct PowerExt {
    pub i2c: I2cBusDevice,
    pub enable_pin: PowerExtEnablePin,
    pub nint_pin: PowerExtNIntPin,
}

pub type USBPDResetPin = Output<'static, GpioPin<7>>;
pub type USBPDIntPin = Input<'static, GpioPin<21>>;

pub struct USBPD {
    pub reset_pin: USBPDResetPin,
    pub int_pin: USBPDIntPin,
    pub i2c: I2cBusDevice,
}

pub struct Storage {
    flash: FlashStorage,
}

pub struct Bsp {
    pub i2c_bus: &'static I2cBus,

    pub clocks: &'static ClocksInstance,
    pub delay: Delay,
    pub rtc: Rtc<'static>,
    pub wifi: Wifi,
    pub stats: Stats,
    pub power_ext: PowerExt,
    pub usb_pd: USBPD,

    pub high_prio_spawner: SendSpawner,
}

impl Bsp {
    pub fn init(peripherals: Peripherals) -> Self {
        let system = SystemControl::new(peripherals.SYSTEM);
        let io = Io::new(peripherals.GPIO, peripherals.IO_MUX);

        // Set device speed
        let clocks = {
            static CLOCKS: StaticCell<Clocks> = StaticCell::new();
            CLOCKS.init(ClockControl::max(system.clock_control).freeze())
        };

        // Initialize the RTC before the Wifi because of esp-wifi issues
        let mut rtc = Rtc::new(peripherals.LPWR, None);

        // Wifi
        let wifi = {
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

            Wifi {
                device: wifi_device,
                controller: wifi_controller,
                seed: wifi_seed,
            }
        };

        // Initialize embassy
        {
            let timg0 = TimerGroup::new_async(peripherals.TIMG0, &clocks);
            esp_hal_embassy::init(&clocks, timg0);
        }

        // I2C generic bus
        let i2c_bus: &'static _ = {
            let i2c: I2cInstance = I2C::new_with_timeout_async(
                peripherals.I2C0,
                io.pins.gpio5,
                io.pins.gpio4,
                400u32.kHz(),
                &clocks,
                Some(40),
            );

            static I2C_BUS: StaticCell<I2cBus> = StaticCell::new();
            I2C_BUS.init(Mutex::new(i2c))
        };

        let stats = {
            let vprog = io.pins.gpio1;
            let vsupply = io.pins.gpio0;
            let vout = io.pins.gpio3;

            let mut adc_config = AdcConfig::new();
            let vprog = adc_config.enable_pin_with_cal::<_, StatsAdcCal>(
                vprog,
                esp_hal::analog::adc::Attenuation::Attenuation11dB,
            );
            let vsupply = adc_config.enable_pin_with_cal::<_, StatsAdcCal>(
                vsupply,
                esp_hal::analog::adc::Attenuation::Attenuation11dB,
            );
            let vout = adc_config.enable_pin_with_cal::<_, StatsAdcCal>(
                vout,
                esp_hal::analog::adc::Attenuation::Attenuation11dB,
            );
            let adc = Adc::new(peripherals.ADC1, adc_config);

            Stats {
                adc,
                pins: StatsPins {
                    vprog,
                    vsupply,
                    vout,
                },
            }
        };

        let power_ext = PowerExt {
            i2c: I2cBusDevice::new(i2c_bus),
            enable_pin: Output::new(io.pins.gpio10, Level::Low),
            nint_pin: Input::new(io.pins.gpio20, Pull::None),
        };

        let usb_pd = USBPD {
            reset_pin: Output::new(io.pins.gpio7, Level::Low),
            int_pin: Input::new(io.pins.gpio21, Pull::None),
            i2c: I2cBusDevice::new(i2c_bus),
        };

        let mut delay = Delay::new(&clocks);

        static EXECUTOR: StaticCell<InterruptExecutor<2>> = StaticCell::new();
        let executor =
            InterruptExecutor::new(system.software_interrupt_control.software_interrupt2);
        let executor = EXECUTOR.init(executor);

        let high_prio_spawner = executor.start(esp_hal::interrupt::Priority::Priority3);

        log::info!("initialized");

        Bsp {
            i2c_bus,
            clocks,
            delay,
            rtc,
            wifi,
            stats,
            power_ext,
            usb_pd,
            high_prio_spawner,
        }
    }
}

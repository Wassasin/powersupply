use core::str::FromStr;

use embassy_time::Instant;
use log::LevelFilter;

use super::println;

const LOG_TARGETS: Option<&'static str> = option_env!("PSU_LOGTARGETS");

pub fn init_logger_from_env() {
    unsafe {
        log::set_logger_racy(&EspLogger).unwrap();
    }

    const LEVEL: Option<&'static str> = option_env!("PSU_LOGLEVEL");

    if let Some(lvl) = LEVEL {
        let level = LevelFilter::from_str(lvl).unwrap_or(LevelFilter::Off);
        unsafe { log::set_max_level_racy(level) };
    } else {
        unsafe { log::set_max_level_racy(LevelFilter::Info) };
    }
}

struct EspLogger;

impl log::Log for EspLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    #[allow(unused)]
    fn log(&self, record: &log::Record) {
        // check enabled log targets if any
        if let Some(targets) = LOG_TARGETS {
            if !targets.split(',').any(|v| record.target().starts_with(v)) {
                return;
            }
        }

        const RESET: &str = "\u{001B}[0m";
        const BOLD: &str = "\u{001B}[1m";
        const DIMMED: &str = "\u{001B}[2m";
        const RED: &str = "\u{001B}[31m";
        const GREEN: &str = "\u{001B}[32m";
        const YELLOW: &str = "\u{001B}[33m";
        const BLUE: &str = "\u{001B}[34m";
        const CYAN: &str = "\u{001B}[35m";

        let color = match record.level() {
            log::Level::Error => RED,
            log::Level::Warn => YELLOW,
            log::Level::Info => GREEN,
            log::Level::Debug => BLUE,
            log::Level::Trace => CYAN,
        };

        let target = record.target();

        let now = Instant::now();
        let now_ms = now.as_millis();
        let now_ms_sub = now_ms % 1000;
        let now_s = now_ms / 1000;

        println!(
            "{}{:3}.{:03} [{}{}{}{} {}{}{}{}]{} {}",
            DIMMED,
            now_s,
            now_ms_sub,
            RESET,
            color,
            record.level(),
            RESET,
            BOLD,
            target,
            RESET,
            DIMMED,
            RESET,
            record.args(),
        );
    }

    fn flush(&self) {}
}

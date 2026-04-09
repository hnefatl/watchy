#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;
use core::fmt::Write;

use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use embedded_hal_bus::spi::RefCellDevice;
use epd_waveshare::epd1in54_v2::{Display1in54, Epd1in54};
use epd_waveshare::prelude::*;
use esp_hal::gpio::{Input, InputConfig, Level, OutputConfig, Pull};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{clock::CpuClock, gpio::Output};

use defmt::info;
use esp_println as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant};

use esp_backtrace as _;

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73000);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!("Embassy initialized!");

    let pin_spi_edp_cs = Output::new(peripherals.GPIO33, Level::High, OutputConfig::default());
    let pin_edp_dc = Output::new(peripherals.GPIO34, Level::Low, OutputConfig::default());
    let pin_edp_reset = Output::new(peripherals.GPIO35, Level::Low, OutputConfig::default());
    let pin_edp_busy = Input::new(
        peripherals.GPIO36,
        InputConfig::default().with_pull(Pull::Up),
    );

    let spi = esp_hal::spi::master::Spi::new(
        peripherals.SPI2,
        esp_hal::spi::master::Config::default()
            .with_frequency(Rate::from_mhz(2))
            .with_mode(esp_hal::spi::Mode::_0),
    )
    .unwrap()
    .into_async()
    .with_sck(peripherals.GPIO47)
    .with_miso(peripherals.GPIO46)
    .with_mosi(peripherals.GPIO48);

    let r = RefCell::new(spi);
    let mut delay = embassy_time::Delay;
    let mut spi_device =
        RefCellDevice::new(&r, pin_spi_edp_cs, &mut delay).expect("failed to init SPI device");

    info!("SPI device initialised");

    let mut epd = Epd1in54::new(
        &mut spi_device,
        pin_edp_busy,
        pin_edp_dc,
        pin_edp_reset,
        &mut embassy_time::Delay,
        Some(1_000), // 1ms
    )
    .unwrap();
    let mut display = Display1in54::default();

    //let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    //let (mut _wifi_controller, _interfaces) =
    //    esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
    //       .expect("Failed to initialize Wi-Fi controller");

    // TODO: Spawn some tasks
    let _ = spawner;

    display.clear(Color::White);
    epd.set_background_color(Color::White);
    let style = MonoTextStyle::new(&FONT_10X20, Color::Black);

    let mut lut_loop = [Some(RefreshLut::Full), Some(RefreshLut::Quick), None, None]
        .iter()
        .cycle();

    let mut ticker = embassy_time::Ticker::every(Duration::from_secs(10));
    loop {
        let now_secs = Instant::now().as_secs();
        let mut s = heapless::String::<20>::new();
        let _ = write!(
            s,
            "{:02}:{:02}:{:02}",
            now_secs / 3600 % 24,
            now_secs / 60 % 60,
            now_secs % 60
        );
        info!("Time since boot: {}", &s);
        let text = Text::new(&s, Point::new(40, 40), style);

        display.clear(Color::White);
        text.draw(&mut display).unwrap();
        epd.wake_up(&mut spi_device, &mut embassy_time::Delay)
            .unwrap();
        if let Some(lut) = lut_loop.next().unwrap() {
            epd.set_lut(&mut spi_device, &mut embassy_time::Delay, Some(*lut))
                .unwrap();
        }
        epd.update_and_display_frame(&mut spi_device, &display.buffer(), &mut embassy_time::Delay)
            .unwrap();
        epd.wait_until_idle(&mut spi_device, &mut embassy_time::Delay)
            .unwrap();
        epd.sleep(&mut spi_device, &mut embassy_time::Delay)
            .unwrap();

        ticker.next().await;
    }
}

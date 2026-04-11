#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;

use button_driver::ButtonConfig;
use embassy_futures::select::Either5;
use embedded_hal_bus::spi::RefCellDevice;
use esp_hal::gpio::{Input, InputConfig, Level, OutputConfig};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{clock::CpuClock, gpio::Output};

use defmt::info;
use esp_println as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant};

use esp_backtrace as _;

extern crate alloc;

type Button = button_driver::Button<Input<'static>, Instant, Duration>;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

mod display;
use display::*;

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

    let mut button1 = Button::new(
        Input::new(peripherals.GPIO7, InputConfig::default()),
        ButtonConfig::default(),
    );
    let mut button2 = Button::new(
        Input::new(peripherals.GPIO6, InputConfig::default()),
        ButtonConfig::default(),
    );
    let mut button3 = Button::new(
        Input::new(peripherals.GPIO0, InputConfig::default()),
        ButtonConfig::default(),
    );
    let mut button4 = Button::new(
        Input::new(peripherals.GPIO8, InputConfig::default()),
        ButtonConfig::default(),
    );

    let pin_spi_edp_cs = Output::new(peripherals.GPIO33, Level::High, OutputConfig::default());

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
    let spi_device =
        RefCellDevice::new(&r, pin_spi_edp_cs, &mut delay).expect("failed to init SPI device");

    info!("SPI device initialised");

    let mut display = Display::new(
        spi_device,
        peripherals.GPIO36,
        peripherals.GPIO34,
        peripherals.GPIO35,
    );

    //let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    //let (mut wifi_controller, _interfaces) =
    //    esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
    //        .expect("Failed to initialize Wi-Fi controller");

    // TODO: Spawn some tasks
    let _ = spawner;

    let mut ticker = embassy_time::Ticker::every(Duration::from_secs(10));
    display.force_render().unwrap();
    loop {
        button1.tick();
        button2.tick();
        button3.tick();
        button4.tick();

        display
            .update_state(|state: &mut DisplayState| {
                match &mut state.watch_state {
                    WatchState::Main => {
                        if button1.is_clicked() {
                            button1.reset();
                            state.watch_state = WatchState::ManualTimeSet {
                                offset_hours: 0,
                                offset_mins: 0,
                            };
                            info!("Entered manual time set mode")
                        }
                    }
                    WatchState::ManualTimeSet {
                        offset_hours,
                        offset_mins,
                    } => {
                        if button2.is_clicked() {
                            button2.reset();
                            *offset_hours = (*offset_hours + 1) % 24
                        }
                        if button3.is_clicked() {
                            button3.reset();
                            *offset_mins = (*offset_mins + 1) % 60
                        }
                        state.time_offset =
                            Duration::from_secs(3600 * *offset_hours + 60 * *offset_mins);
                        info!("New time offset: {}", state.time_offset);

                        if button1.is_clicked() {
                            button1.reset();
                            // Exit
                            state.watch_state = WatchState::Main;
                            info!("Returned to main mode");
                        }
                    }
                }
            })
            .unwrap();

        // This is a busy polling loop, not an energy-efficient interrupt.
        let res = embassy_futures::select::select5(
            ticker.next(),
            // Wait for release, not press
            button1.pin.wait_for_any_edge(),
            button2.pin.wait_for_any_edge(),
            button3.pin.wait_for_any_edge(),
            button4.pin.wait_for_any_edge(),
        )
        .await;
        let cause = match res {
            Either5::First(_) => "timer",
            Either5::Second(_) => "button1",
            Either5::Third(_) => "button2",
            Either5::Fourth(_) => "button3",
            Either5::Fifth(_) => "button4",
        };
        info!("Woken up by: {}", cause);
    }
}

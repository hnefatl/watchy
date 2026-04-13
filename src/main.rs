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

use embassy_futures::select::{Either, select};
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use embedded_hal::spi::SpiDevice;
use embedded_hal_bus::spi::RefCellDevice;
use epd_waveshare::color::Color;
use epd_waveshare::epd1in54::Display1in54;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Io, Level, OutputConfig, RtcFunction, RtcPin};
use esp_hal::rtc_cntl::sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel};
use esp_hal::rtc_cntl::{Rtc, reset_reason, wakeup_cause};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{clock::CpuClock, gpio::Output};

use defmt::info;
use esp_println as _;

use embassy_executor::Spawner;
use embassy_time::Duration;

use esp_backtrace as _;

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

mod display;
use display::*;
mod buttons;
use buttons::*;

const INACTIVITY_DURATION: Duration = Duration::from_secs(5);

// TODO: split into separate structs for clarity.
#[derive(Clone, Debug, PartialEq, Eq, defmt::Format)]
pub enum MenuState {
    None,
    DebugView(heapless::String<100>),
    ManualTimeSet { offset_hours: u64, offset_mins: u64 },
}
// TODO: add update for each struct.
impl MenuItem for MenuState {
    fn render(&self, display: &mut Display1in54, time_provider: &impl TimeProvider) {
        const FONT: MonoFont = FONT_10X20;
        const STYLE: MonoTextStyle<'_, Color> = MonoTextStyle::new(&FONT, Color::Black);

        display.clear(Color::White);

        let current_time = time_provider.get_current_time();
        match &self {
            MenuState::None => {
                let mut s = heapless::String::<20>::new();
                let _ = write!(
                    s,
                    "{:02}:{:02}:{:02}",
                    current_time.as_secs() / 3600 % 24,
                    current_time.as_secs() / 60 % 60,
                    current_time.as_secs() % 60
                );
                let text = Text::new(&s, Point::new(40, 40), STYLE);
                text.draw(display).unwrap();
            }
            MenuState::DebugView(message) => {
                let text = Text::new(&message, Point::new(10, 10), STYLE);
                text.draw(display).unwrap();
            }
            MenuState::ManualTimeSet {
                offset_hours: _,
                offset_mins: _,
            } => {
                let text = Text::new("time set mode", Point::new(35, 100), STYLE);
                text.draw(display).unwrap();
            }
        }
    }
    fn update(&mut self, time_provider: &impl TimeProvider) {
        unimplemented!()
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    info!(
        "Reset due to: {:?} ({})",
        defmt::Debug2Format(&reset_reason(esp_hal::system::Cpu::ProCpu)),
        wakeup_cause(),
    );

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    //esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73000);
    esp_rtos::start(timg0.timer0);
    let mut rtc = Rtc::new(peripherals.LPWR);
    info!("Embassy initialized");

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

    let delay = Delay::new();
    let mut io = Io::new(peripherals.IO_MUX);
    let button_pins = (
        peripherals.GPIO7,
        peripherals.GPIO6,
        peripherals.GPIO0,
        peripherals.GPIO8,
    );
    // RtcioWakeupSource configures the pins to the RTC mux: but due to a bug(?),
    // it doesn't reset them back to the IO mux (that logic's in the drop
    // function, which isn't called when resuming from sleep).
    // Workaround by manually reconfiguring all the wakeup pins to digital IO.
    button_pins.0.rtc_set_config(true, false, RtcFunction::Rtc);
    button_pins.1.rtc_set_config(true, false, RtcFunction::Rtc);
    button_pins.2.rtc_set_config(true, false, RtcFunction::Rtc);
    button_pins.3.rtc_set_config(true, false, RtcFunction::Rtc);
    let mut buttons = Buttons::init(&mut io, button_pins);
    info!("Buttons initialized");

    let mut time_provider = RtcTimeProvider::new(&rtc);
    main_loop(&mut display, &mut buttons, &mut time_provider).await;

    let mut button_pins = buttons.reclaim();
    let button_rtc_pins: &mut [(&mut dyn RtcPin, WakeupLevel)] = &mut [
        (&mut button_pins.0, WakeupLevel::Low),
        (&mut button_pins.1, WakeupLevel::Low),
        // GPIO0 is low immediately after deep sleep reset, so waiting for it to go
        // low immediately wakes up. But because it's already low, if the button's
        // pressed there's no change, so wakeup doesn't work for this button.
        // TODO: fix somehow?
        (&mut button_pins.2, WakeupLevel::High),
        (&mut button_pins.3, WakeupLevel::Low),
    ];
    let buttons_wakeup = RtcioWakeupSource::new(button_rtc_pins);
    let timer_wakeup = TimerWakeupSource::new(core::time::Duration::from_secs(60));

    info!("Entering deep sleep");
    delay.delay_millis(100);
    rtc.sleep_deep(&[&buttons_wakeup, &timer_wakeup]);

    //let radio_init = esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller");
    //let (mut wifi_controller, _interfaces) =
    //    esp_radio::wifi::new(&radio_init, peripherals.WIFI, Default::default())
    //        .expect("Failed to initialize Wi-Fi controller");
}

async fn main_loop<SPI>(
    display: &mut Display<SPI>,
    buttons: &mut Buttons,
    time_provider: &mut impl TimeProvider,
) where
    SPI: SpiDevice,
{
    info!("Forced initial render");
    let mut state = MenuState::None;

    loop {
        display.render(&state, time_provider).unwrap();
        let inactivity_timer = embassy_time::Timer::after(INACTIVITY_DURATION);
        match select(inactivity_timer, buttons.wait_for_event_ready()).await {
            Either::First(()) => {
                info!(
                    "Inactive for {}s, going to sleep.",
                    INACTIVITY_DURATION.as_secs()
                );
                display.force_full_render(&state, time_provider).unwrap();
                break;
            }
            Either::Second(()) => {
                // If there are queued button interrupts (e.g. during a slow
                // display update), process them all at once before refreshing the screen.
                while let Some(e) = buttons.try_get_event() {
                    info!("Button press: {}", e);
                    match e {
                        (ButtonId::Button1, ButtonEvent::Pressed) => {
                            time_provider.shift_current_time(Duration::from_secs(60));
                        }
                        (ButtonId::Button2, ButtonEvent::Pressed) => {
                            time_provider.shift_current_time(Duration::from_secs(60 * 60));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

use core::fmt::Write;
use core::iter::{Cycle, Iterator};

use defmt::info;
use embassy_time::{Duration, Instant};
use embedded_graphics::mono_font::iso_8859_1::FONT_10X20;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use embedded_hal::spi::SpiDevice;
use epd_waveshare::epd1in54::Display1in54;
use epd_waveshare::epd1in54_v2::Epd1in54;
use epd_waveshare::prelude::*;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::peripherals::{GPIO34, GPIO35, GPIO36};

const LUT_CYCLE: [Option<RefreshLut>; 4] =
    [Some(RefreshLut::Full), Some(RefreshLut::Quick), None, None];

#[derive(Clone, Debug, PartialEq, Eq, defmt::Format)]
pub enum WatchState {
    Main,
    ManualTimeSet { offset_hours: u64, offset_mins: u64 },
}

#[derive(Clone, PartialEq, Eq, Debug, defmt::Format)]
pub struct DisplayState {
    pub watch_state: WatchState,
    pub time_since_boot: Instant,
    pub time_offset: Duration,
}
impl DisplayState {
    fn render(&self, display: &mut Display1in54) {
        const FONT: MonoFont = FONT_10X20;
        const STYLE: MonoTextStyle<'_, Color> = MonoTextStyle::new(&FONT, Color::Black);

        display.clear(Color::White);
        let mut s = heapless::String::<20>::new();
        let time = self.time_since_boot + self.time_offset;
        let _ = write!(
            s,
            "{:02}:{:02}:{:02}",
            time.as_secs() / 3600 % 24,
            time.as_secs() / 60 % 60,
            time.as_secs() % 60
        );
        let text = Text::new(&s, Point::new(40, 40), STYLE);
        text.draw(display).unwrap();
        info!("Time is now: {}", s);

        if let WatchState::ManualTimeSet {
            offset_mins: _,
            offset_hours: _,
        } = self.watch_state
        {
            let text = Text::new("time set mode", Point::new(35, 100), STYLE);
            text.draw(display).unwrap();
        }
    }
}
impl Default for DisplayState {
    fn default() -> Self {
        Self {
            watch_state: WatchState::Main,
            time_since_boot: Instant::now(),
            time_offset: Duration::from_secs(0),
        }
    }
}

pub struct Display<SPI> {
    state: DisplayState,
    lut_loop: Cycle<core::slice::Iter<'static, Option<RefreshLut>>>,
    epd: Epd1in54<SPI, Input<'static>, Output<'static>, Output<'static>, embassy_time::Delay>,
    spi_device: SPI,
    display: Display1in54,
}

impl<SPI> Display<SPI>
where
    SPI: SpiDevice,
{
    pub fn new(
        mut spi_device: SPI,
        pin_busy: GPIO36<'static>,
        pin_dc: GPIO34<'static>,
        pin_reset: GPIO35<'static>,
    ) -> Self {
        let mut epd = Epd1in54::new(
            &mut spi_device,
            Input::new(pin_busy, InputConfig::default().with_pull(Pull::Up)),
            Output::new(pin_dc, Level::Low, OutputConfig::default()),
            Output::new(pin_reset, Level::Low, OutputConfig::default()),
            &mut embassy_time::Delay,
            Some(1_000), // 1ms
        )
        .unwrap();
        let display = Display1in54::default();

        epd.set_background_color(Color::White);
        Self {
            state: DisplayState::default(),
            lut_loop: LUT_CYCLE.iter().cycle(),
            epd,
            spi_device,
            display,
        }
    }

    pub fn update_state<F>(&mut self, update_state: F) -> Result<(), SPI::Error>
    where
        F: FnOnce(&mut DisplayState),
    {
        let old_state = self.state.clone();
        update_state(&mut self.state);
        if self.state != old_state {
            info!("New state: {}", self.state);
            self.force_render()
        } else {
            Ok(())
        }
    }
    pub fn force_render(&mut self) -> Result<(), SPI::Error> {
        self.display.clear(Color::White);
        self.state.render(&mut self.display);

        self.epd
            .wake_up(&mut self.spi_device, &mut embassy_time::Delay)?;
        if let Some(lut) = self.lut_loop.next().unwrap() {
            self.epd
                .set_lut(&mut self.spi_device, &mut embassy_time::Delay, Some(*lut))?;
        }
        self.epd.update_and_display_frame(
            &mut self.spi_device,
            &self.display.buffer(),
            &mut embassy_time::Delay,
        )?;
        self.epd
            .wait_until_idle(&mut self.spi_device, &mut embassy_time::Delay)?;
        self.epd
            .sleep(&mut self.spi_device, &mut embassy_time::Delay)?;
        Ok(())
    }
}

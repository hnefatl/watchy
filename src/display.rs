use core::iter::{Cycle, Iterator};

use embassy_time::{Duration, Instant};
use embedded_graphics::prelude::*;
use embedded_hal::spi::SpiDevice;
use epd_waveshare::epd1in54::Display1in54;
use epd_waveshare::epd1in54_v2::Epd1in54;
use epd_waveshare::prelude::*;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::peripherals::{GPIO34, GPIO35, GPIO36};
use esp_hal::rtc_cntl::Rtc;

const LUT_CYCLE: [Option<RefreshLut>; 4] =
    [Some(RefreshLut::Full), Some(RefreshLut::Quick), None, None];

pub trait TimeProvider {
    fn get_current_time(&self) -> Instant;
    fn set_current_time(&self, time: Instant);
    fn shift_current_time(&self, shift: Duration);
}
pub struct RtcTimeProvider<'a> {
    rtc: &'a Rtc<'a>,
}
impl<'a> RtcTimeProvider<'a> {
    pub fn new(rtc: &'a Rtc<'a>) -> Self {
        RtcTimeProvider { rtc }
    }
}
impl TimeProvider for RtcTimeProvider<'_> {
    fn get_current_time(&self) -> Instant {
        Instant::from_micros(self.rtc.current_time_us())
    }
    fn set_current_time(&self, time: Instant) {
        self.rtc.set_current_time_us(time.as_micros());
    }
    fn shift_current_time(&self, shift: Duration) {
        self.set_current_time(self.get_current_time() + shift)
    }
}

pub trait MenuItem {
    fn render(&self, display: &mut Display1in54, time: &impl TimeProvider);
    fn update(&mut self, time_provider: &impl TimeProvider);
}

pub struct Display<SPI> {
    spi_device: SPI,
    display: Display1in54,
    lut_loop: Cycle<core::slice::Iter<'static, Option<RefreshLut>>>,
    epd: Epd1in54<SPI, Input<'static>, Output<'static>, Output<'static>, embassy_time::Delay>,
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
            spi_device,
            display,
            lut_loop: LUT_CYCLE.iter().cycle(),
            epd,
        }
    }

    pub fn render<M: MenuItem>(
        &mut self,
        renderable: &M,
        time_provider: &impl TimeProvider,
    ) -> Result<(), SPI::Error> {
        self.display.clear(Color::White);
        renderable.render(&mut self.display, time_provider);

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
    pub fn force_full_render<M: MenuItem>(
        &mut self,
        renderable: &M,
        time_provider: &impl TimeProvider,
    ) -> Result<(), SPI::Error> {
        self.lut_loop = LUT_CYCLE.iter().cycle();
        self.render(renderable, time_provider)
    }
}
